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
use game_core::prelude::{ActiveBoard, DataHandles, GameSeed, GameSession, TurnPhase};

mod autopilot;
mod hud;
mod render;

const DEV_FONT: &str = "font/dev_SourceHanSansSC-Regular.otf";

fn main() {
    let root = asset_root();
    info!("资产根：{}", root.display());
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "暴富 · B4 等轴测渲染切片".into(),
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
        .add_plugins(render::RenderPlugin)
        .add_plugins(autopilot::AutopilotPlugin)
        .add_systems(Startup, load_data)
        .add_systems(
            Update,
            (
                hud::spawn_hud.run_if(resource_added::<ActiveBoard>),
                // `GameSession`/`TurnOrder` 与 `ActiveBoard` 同批插入，但守门要写出来：
                // 少了它，第一帧的 `Res<GameSession>` 参数校验就会失败
                hud::refresh_hud.run_if(resource_exists::<GameSession>),
                check_assets.run_if(in_state(TurnPhase::Boot)),
                maybe_capture,
            ),
        )
        .run();
}

fn load_data(mut commands: Commands, server: Res<AssetServer>) {
    // 真实客户端每次开局换种子；测试与回放走 game_core::GameSeed::default() 的固定值
    commands.insert_resource(GameSeed(clock_seed()));
    commands.insert_resource(DataHandles::load(&server));
    commands.insert_resource(render::DevFont(server.load(DEV_FONT)));
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
fn asset_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let from_cwd = cwd.join("assets");
    if from_cwd.is_dir() {
        return from_cwd;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().map(|p| p.join("assets")).unwrap_or(from_cwd)
}
