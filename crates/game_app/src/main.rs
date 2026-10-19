//! `game_app` 入口：窗口、资产根、各层 Plugin 的接线，以及 agent 自查画面用的截图钩子。
//!
//! 这里只做接线，不画东西 —— 几何在 `render/*`，占位面板在 `hud.rs`。

use std::path::PathBuf;

use bevy::{
    asset::{AssetPlugin, LoadState},
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PresentMode, Window},
};
use game_core::CoreGamePlugin;
use game_core::prelude::{DataHandles, GameSeed, TurnPhase};

mod autopilot;
mod dump;
mod render;
mod scene;
mod ui;

const DEV_FONT: &str = "font/dev_SourceHanSansSC-Regular.otf";

fn main() {
    let root = asset_root();
    info!("资产根：{}", root.display());
    let mut app = App::new();
    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: "暴富 · B5 UI 与交互".into(),
                    present_mode: PresentMode::AutoNoVsync,
                    ..default()
                }),
                ..default()
            })
            .set(AssetPlugin {
                file_path: root.display().to_string(),
                ..default()
            }),
    )
    // 规则层的 Plugin 在表现层注册：验证「无渲染 bevy 子集 + 完整 bevy」能共存于同一 App
    .add_plugins(CoreGamePlugin)
    // 网格命中测试：点格详情卡要能点到 3D 地板。默认会 raycast **所有**网格，
    // 建筑与棋子都会被点中；`require_markers = true` 把它改成显式登记 ——
    // 只有带 `Pickable` 的地板 + 标了 `MeshPickingCamera` 的相机会参与
    .add_plugins(MeshPickingPlugin)
    .insert_resource(MeshPickingSettings {
        require_markers: true,
        ..default()
    })
    .add_plugins(render::RenderPlugin)
    .add_plugins(ui::UiPlugin)
    .add_plugins(scene::ScenePlugin)
    .add_plugins(autopilot::AutopilotPlugin);
    // 调试面板排在最后：它只读别人的状态，而且默认不装（见 `dump` 模块头）
    dump::install_if_requested(&mut app);

    insert_boot_resources(app.world_mut());

    app.add_systems(
        Update,
        (
            check_assets.run_if(in_state(TurnPhase::Boot)),
            maybe_capture,
        ),
    )
    .run();
}

/// 开局资源直接写进 world，不走 `Commands`。
///
/// `MainSchedule::Startup` 末尾就跑一次 `StateTransition`，所以 `ScenePlugin` 那句
/// `set(Scene::Menu)` 会在**同一批**里触发 `OnEnter(Menu)` → `spawn_menu`。
/// 用 `Commands` 插资源要等到下一批 flush，那时菜单已经试着 spawn 过了，
/// 报的是「`Res<DevFont>` failed validation: Resource does not exist」。
fn insert_boot_resources(world: &mut World) {
    let server = world.resource::<AssetServer>().clone();
    world.insert_resource(GameSeed(boot_seed()));
    world.insert_resource(DataHandles::load(&server));
    world.insert_resource(render::DevFont(server.load(DEV_FONT)));
}

/// 种子默认取墙钟纳秒；`NM_SEED=<十进制>` 钉死它，让一局能原样重跑。
///
/// 长对局验收需要可复现：几百回合之后崩一次，换个种子可能就是另一种走法，查无可查。
fn boot_seed() -> u64 {
    std::env::var("NM_SEED")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or_else(clock_seed)
}

fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// 加载失败要立刻炸出来：Bevy 对失败资产默认只打一条日志然后永远等下去，
/// 静默的白屏比一句 panic 难排查得多（§六 坑 6）。
///
/// 挂在 `Boot` 态上：棋盘没装配好就一直等在这里，所以每帧都问一遍状态，
/// 直到 `AssetServer` 给出 `Failed` 为止。
fn check_assets(server: Res<AssetServer>, data: Res<DataHandles>) {
    expect_ready(&server, &data.board, "棋盘");
    expect_ready(&server, &data.chance, "机会牌");
    expect_ready(&server, &data.fate, "命运牌");
    // 道具牌也要查：装配要四份资产齐（`world::begin_game_system` 那个元组），
    // 少一份就是卡在 `Boot` 上不动 —— 而 Bevy 对失败资产只打一条日志然后永远等下去
    expect_ready(&server, &data.items, "道具牌");
    expect_ready(&server, &data.stocks, "股票");
}

fn expect_ready<T: Asset>(server: &AssetServer, handle: &Handle<T>, what: &str) {
    if let LoadState::Failed(err) = server.load_state(handle) {
        panic!(
            "{what}加载失败：{err}（路径 {:?}，检查资产根）",
            handle.path()
        );
    }
}

/// 设了 `NM_SHOT=输出路径` 就截一张 PNG 然后自行退出 —— 让 agent 能自己核对画面，
/// 不必每次请人肉眼验收，也不会留下一堆没人关的窗口进程。
///
/// 触发条件必须是**墙钟时间**而不是帧数：`AutoNoVsync` 下前 40 帧只有一二十毫秒，
/// 3D 管线（着色器编译 + 网格上传）还没热完，截出来只剩 clear color 与 UI。
///
/// `NM_SHOT_AT=<秒>` 推迟截图时刻（默认 1.5s）。B4 之后画面是**随对局长的** ——
/// 建筑要买地才有、相机要走过几格才看得出跟随，所以验收图需要等到那时候。
fn maybe_capture(time: Res<Time>, mut done: Local<bool>, mut commands: Commands) {
    let Ok(path) = std::env::var("NM_SHOT") else {
        return;
    };
    let at = std::env::var("NM_SHOT_AT")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(1.5);
    if *done {
        // 留出一点余量让截图回调把 PNG 落盘，然后退出：截图进程不该靠外部 kill
        if time.elapsed_secs() >= at + 1.0 {
            std::process::exit(0);
        }
        return;
    }
    if time.elapsed_secs() < at {
        return;
    }
    *done = true;
    info!("截图输出：{path}（t={:.2}s）", time.elapsed_secs());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

/// Bevy 0.20 的 `AssetPlugin::file_path` 是相对「可执行文件目录」的，而 `cargo run` 的 CWD 在仓库根，
/// 两者对不上会让所有资产静默 404，所以这里把根显式钉成绝对路径。
/// `pub(crate)` 是因为 `dump` 面板要在运行时找同一份开发字体（见它的注释）。
pub(crate) fn asset_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let from_cwd = cwd.join("assets");
    if from_cwd.is_dir() {
        return from_cwd;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().map(|p| p.join("assets")).unwrap_or(from_cwd)
}
