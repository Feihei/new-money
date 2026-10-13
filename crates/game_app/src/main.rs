use std::fs;
use std::path::PathBuf;

use bevy::{
    asset::AssetPlugin,
    camera::ScalingMode,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PresentMode, Window},
};
use game_core::CoreGamePlugin;
use game_core::board::{BoardDef, TileKind};

mod board_view;

/// B0 阶段先用文件系统直读；B1 换成 `AssetServer` + 自定义 `AssetLoader`。
const BOARD_REL: &str = "data/boards/prototype.json";
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

#[derive(Resource)]
pub struct ActiveBoard(pub BoardDef);

#[derive(Resource)]
pub struct AssetRoot(pub PathBuf);

fn main() {
    let root = asset_root();
    info!("资产根：{}", root.display());
    App::new()
        .insert_resource(AssetRoot(root.clone()))
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "暴富 · B0 骨架".into(),
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
        .add_systems(
            Startup,
            (
                setup_board,
                setup_camera,
                board_view::spawn_board,
                setup_text,
            )
                .chain(),
        )
        .add_systems(Update, maybe_capture)
        .run();
}

/// 设了 `NM_SHOT=输出路径` 就截一张 PNG —— 让 agent 能自己核对画面，不必每次请人肉眼验收。
///
/// 触发条件必须是**墙钟时间**而不是帧数：`AutoNoVsync` 下前 40 帧只有一二十毫秒，
/// 3D 管线（着色器编译 + 网格上传）还没热完，截出来只剩 clear color 与 UI。
fn maybe_capture(time: Res<Time>, mut done: Local<bool>, mut commands: Commands) {
    let Ok(path) = std::env::var("NM_SHOT") else {
        return;
    };
    if *done || time.elapsed_secs() < 1.5 {
        return;
    }
    *done = true;
    info!("截图输出：{path}（t={:.2}s）", time.elapsed_secs());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

fn setup_board(mut commands: Commands, root: Res<AssetRoot>) {
    let path = root.0.join(BOARD_REL);
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到 {}（请在仓库根目录 cargo run）: {e}", path.display()));
    let board = BoardDef::from_json(&text).unwrap_or_else(|e| panic!("棋盘解析失败: {e}"));
    if let Err(errs) = board.validate() {
        panic!("棋盘校验失败:\n  {}", errs.join("\n  "));
    }
    info!(
        "棋盘 {} 载入：{} 主环格 / {} 块土地",
        board.board_name,
        board.loop_len(),
        board.lands.len()
    );
    commands.insert_resource(ActiveBoard(board));
}

fn setup_camera(mut commands: Commands, board: Res<ActiveBoard>) {
    let (min, max) = board_aabb(&board.0);
    let center = (min + max) * 0.5;
    // 正交投影下距离只影响裁剪，真正决定取景的是 viewport_height。
    // XZ 平面矩形在 45° 方位 + 54.7° 俯角下的屏幕高度约 (宽+深)·sin45°·sin54.7°。
    let span = (max.x - min.x) + (max.z - min.z);
    let viewport_height = span * 0.707 * 0.816 * 1.25;
    let eye = center + ISO_DIR.normalize() * ISO_DIST;
    info!("取景 center={center} eye={eye} viewport_height={viewport_height} span={span}");

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

fn setup_text(mut commands: Commands, asset_server: Res<AssetServer>, board: Res<ActiveBoard>) {
    let font: Handle<Font> = asset_server.load(DEV_FONT);
    let style = TextFont {
        font: font.clone().into(),
        font_size: FontSize::Px(26.0),
        ..default()
    };

    let names = board
        .0
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::Property)
        .map(|t| t.name.as_str())
        .collect::<Vec<_>>()
        .join("　");

    let small = TextFont {
        font_size: FontSize::Px(22.0),
        ..style.clone()
    };

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
                Text::new(format!("暴富 · B0 骨架 · 棋盘「{}」", board.0.board_name)),
                style,
                TextColor(Color::srgb(0.08, 0.08, 0.1)),
            ));
            panel.spawn((
                Text::new(names.clone()),
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
