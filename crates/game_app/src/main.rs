use std::path::PathBuf;

use bevy::{
    asset::{AssetPlugin, LoadState},
    camera::ScalingMode,
    ecs::system::SystemParam,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PresentMode, Window},
};
use game_core::CoreGamePlugin;
use game_core::board::TileKind;
use game_core::prelude::{
    ActiveBoard, BoardDef, CardSet, DataHandles, GameRules, GameSeed, GameSetup, StockSet,
    begin_game_system,
};

mod board_view;

const DEV_FONT: &str = "font/dev_SourceHanSansSC-Regular.otf";

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

/// 等轴测视线方向：与 Godot 端 `CAM_OFFSET=(8,16,8)` 同一条 (1,2,1) 对角，俯角约 54.7°。
const ISO_DIR: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const ISO_DIST: f32 = 40.0;

fn main() {
    let root = asset_root();
    info!("资产根：{}", root.display());
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "暴富 · B3 经济核心（无 UI）".into(),
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
        .add_systems(Startup, load_data)
        // 表现层的建图流程读规则层产物 `ActiveBoard`，所以必须显式排在 `begin_game_system` 之后
        .add_systems(
            Update,
            (boot_world, maybe_capture).chain().after(begin_game_system),
        )
        .run();
}

fn load_data(mut commands: Commands, server: Res<AssetServer>) {
    // 真实客户端每次开局换种子；测试与回放走 game_core::GameSeed::default() 的固定值
    commands.insert_resource(GameSeed(clock_seed()));
    commands.insert_resource(DataHandles::load(&server));
}

fn clock_seed() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0)
}

/// 资产句柄与它们的存储打包成系统参数（Bevy 的 `SystemParam` derive），
/// 免得 `boot_world` 的签名长到得靠 `#[allow]` 过关。
#[derive(SystemParam)]
struct SceneAssets<'w> {
    card_sets: Res<'w, Assets<CardSet>>,
    stock_sets: Res<'w, Assets<StockSet>>,
}

/// 启动这一趟流程要读的 resource 们，同样是打包成系统参数。
#[derive(SystemParam)]
struct BootState<'w> {
    data: Res<'w, DataHandles>,
    server: Res<'w, AssetServer>,
    rules: Res<'w, GameRules>,
    setup: Res<'w, GameSetup>,
    /// 规则层的产物：`begin_game_system` 校验过棋盘、铺好实体之后才存在
    active: Option<Res<'w, ActiveBoard>>,
}

/// 数据资产与规则层世界都就位后，一次性建好相机、棋盘几何与 HUD。
fn boot_world(
    mut commands: Commands,
    state: BootState,
    assets: SceneAssets,
    mut booted: Local<bool>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    if *booted {
        return;
    }
    expect_ready(&state.server, &state.data.board, "棋盘");
    expect_ready(&state.server, &state.data.chance, "机会牌");
    expect_ready(&state.server, &state.data.fate, "命运牌");
    expect_ready(&state.server, &state.data.stocks, "股票");

    let (Some(chance), Some(fate), Some(stocks)) = (
        assets.card_sets.get(&state.data.chance),
        assets.card_sets.get(&state.data.fate),
        assets.stock_sets.get(&state.data.stocks),
    ) else {
        return;
    };
    // 数据校验只在规则层做一次（`begin_game_system` 里四类资产一起验，无头测试也走那条路）。
    // 这里只负责「加载失败要响」（expect_ready），不负责内容对错。

    let Some(active) = state.active.as_ref() else {
        return;
    };
    let board = &active.def;
    info!(
        "表现层就绪：机会 {} 张、命运 {} 张；股票 {} 只",
        chance.0.len(),
        fate.0.len(),
        stocks.0.len()
    );

    spawn_camera(&mut commands, board);
    board_view::spawn_board(&mut commands, &mut meshes, &mut mats, board);
    spawn_hud(
        &mut commands,
        &state.server,
        board,
        &state.setup,
        &state.rules,
    );

    *booted = true;
}

/// 加载失败要立刻炸出来：Bevy 对失败资产默认只打一条日志，静默返回空画面极难排查。
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
fn maybe_capture(time: Res<Time>, mut done: Local<bool>, mut commands: Commands) {
    let Ok(path) = std::env::var("NM_SHOT") else {
        return;
    };
    if *done {
        // 留出一点余量让截图回调把 PNG 落盘，然后退出：截图进程不该靠外部 kill
        if time.elapsed_secs() >= 2.5 {
            std::process::exit(0);
        }
        return;
    }
    if time.elapsed_secs() < 1.5 {
        return;
    }
    *done = true;
    info!("截图输出：{path}（t={:.2}s）", time.elapsed_secs());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

fn spawn_camera(commands: &mut Commands, board: &BoardDef) {
    let (min, max) = board_aabb(board);
    let center = (min + max) * 0.5;
    // 正交投影下距离只影响裁剪，真正决定取景的是 viewport_height。
    // XZ 平面矩形在 45° 方位 + 54.7° 俯角下的屏幕高度约 (宽+深)·sin45°·sin54.7°。
    let span = (max.x - min.x) + (max.z - min.z);
    let viewport_height = span * 0.707 * 0.816 * 1.25;
    let eye = center + ISO_DIR.normalize() * ISO_DIST;
    info!("取景 center={center} viewport_height={viewport_height}");

    commands.spawn((
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::srgb(0.6, 0.6, 0.7)),
            ..default()
        },
        Projection::from(OrthographicProjection {
            near: 0.1,
            far: 200.0,
            scaling_mode: ScalingMode::FixedVertical { viewport_height },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_translation(eye).looking_at(center, Vec3::Y),
    ));
}

/// 所有道路格 + 土地足迹的世界 AABB，用于取景。
fn board_aabb(b: &BoardDef) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut fold = |p: Vec3, half: f32| {
        min = min.min(Vec3::new(p.x - half, 0.0, p.z - half));
        max = max.max(Vec3::new(p.x + half, 0.0, p.z + half));
    };
    for t in &b.tiles {
        fold(b.world_pos(t.col, t.row), b.cell_size * 0.5);
        if let Some(br) = &t.branch {
            for bt in &br.tiles {
                fold(b.world_pos(bt.col, bt.row), b.cell_size * 0.5);
            }
        }
    }
    for l in &b.lands {
        if let Some(p) = b.land_world_pos(l.id) {
            fold(p, b.cell_size * l.size as f32 * 0.5);
        }
    }
    (min, max)
}

fn spawn_hud(
    commands: &mut Commands,
    server: &AssetServer,
    board: &BoardDef,
    setup: &GameSetup,
    rules: &GameRules,
) {
    let font: Handle<Font> = server.load(DEV_FONT);
    let style = TextFont {
        font: font.into(),
        font_size: FontSize::Px(26.0),
        ..default()
    };
    let small = TextFont {
        font_size: FontSize::Px(22.0),
        ..style.clone()
    };

    let names = board
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::Property)
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join("　");

    // 开局快照：阵容与现金取自规则层的配置，真实 HUD（现金随结算变动）在 B5
    let roster = setup
        .players
        .iter()
        .map(|p| format!("{} ${}", p.name, rules.start_cash))
        .collect::<Vec<_>>()
        .join("　");

    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                padding: UiRect::axes(px(12), px(8)),
                row_gap: px(6),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.78)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(format!("暴富 · B3 经济核心 · 棋盘「{}」", board.board_name)),
                style,
                TextColor(Color::srgb(0.08, 0.08, 0.1)),
            ));
            panel.spawn((
                Text::new(roster),
                small.clone(),
                TextColor(Color::srgb(0.45, 0.2, 0.05)),
            ));
            panel.spawn((
                Text::new(names),
                small.clone(),
                TextColor(Color::srgb(0.1, 0.25, 0.55)),
            ));
            panel.spawn((
                Text::new("监狱　医院　机会　命运　中央银行"),
                small,
                TextColor(Color::srgb(0.55, 0.15, 0.1)),
            ));
        });
}
