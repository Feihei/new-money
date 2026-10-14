//! 相机：正交等轴测 + rig 跟随棋子。对应 Godot `board.gd:168-188`（`_setup_camera`）
//! 与 `:462-473`（`_follow_token` 的 0.35s 补间）。
//!
//! 结构照 Godot 的 rig：父节点只管 XZ 平移，子 `Camera3d` 固定挂着 (8,16,8) 的偏移
//! 并看向父原点 —— 于是「转镜头」与「推镜头」是两件互不干扰的事。

use bevy::{camera::ScalingMode, prelude::*};
use game_core::prelude::{ActiveBoard, BoardDef, PlayerId, TokenPosition, TurnOrder};

use super::{dim, palette, token::TokenVisual};

#[derive(Component)]
pub(crate) struct CameraRig;

/// 正交等轴测相机。`ClearColor` 对应 Godot 的 `Environment.background_color`。
pub(crate) fn spawn_camera_rig(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    order: Res<TurnOrder>,
    players: Query<&TokenPosition, With<PlayerId>>,
) {
    let b = &active.def;
    // 开局先看着手玩家，而不是棋盘中心：整盘取景时两者一样，跟随取景时才看得出区别
    let focus = order
        .current()
        .and_then(|p| players.get(p).ok())
        .and_then(|tp| b.tile_world_pos(tp.tile.0 as i64))
        .unwrap_or_else(|| board_center(b));

    commands
        .spawn((
            CameraRig,
            // 子相机的 `InheritedVisibility` 要从父节点继承，父节点没有 `Visibility`
            // 就会报 Bevy 的 B0004 警告（可见性传播断在 rig 这一层）
            Visibility::default(),
            Transform::from_xyz(focus.x, 0.0, focus.z),
        ))
        .with_children(|rig| {
            rig.spawn((
                Camera3d::default(),
                Camera {
                    clear_color: ClearColorConfig::Custom(palette::SKY),
                    ..default()
                },
                Projection::from(OrthographicProjection {
                    near: 0.1,
                    far: 200.0,
                    scaling_mode: ScalingMode::FixedVertical {
                        viewport_height: view_height(b),
                    },
                    ..OrthographicProjection::default_3d()
                }),
                Transform::from_translation(dim::CAM_OFFSET.normalize() * dim::CAM_DIST)
                    .looking_at(Vec3::ZERO, Vec3::Y),
            ));
        });
}

/// rig 每帧向当前回合棋子的**视觉位置**靠一点。
///
/// 跟的是插值后的棋子而不是 `TokenPosition`：后者在掷骰那一帧就跳到终点，
/// 跟着它会把镜头甩在棋子前面（Godot 端 `board.gd:459` 也是每落一格才 tween 一次）。
pub(crate) fn follow_current_token(
    time: Res<Time>,
    order: Res<TurnOrder>,
    tokens: Query<(&TokenVisual, &GlobalTransform)>,
    mut rigs: Query<&mut Transform, With<CameraRig>>,
) {
    let (Some(player), Ok(mut rig)) = (order.current(), rigs.single_mut()) else {
        return;
    };
    let Some((_, at)) = tokens.iter().find(|(v, _)| v.player == player) else {
        return;
    };
    let target = at.translation();
    let k = 1.0 - (-time.delta_secs() / dim::CAM_FOLLOW_TAU).exp();
    rig.translation.x += (target.x - rig.translation.x) * k;
    rig.translation.z += (target.z - rig.translation.z) * k;
}

/// 取景高度（世界单位）。默认沿用 Godot `board.gd:31` 的 `CAM_SIZE=8.0` —— 约看得见棋子
/// 四周 4×4 格。`NM_CAM_SIZE=0` 退化成整盘取景（B0–B3 截图的形态）：§7.1 要求这两个值
/// 都做成可配置、肉眼定标一次，所以开关留着，默认值听代码的而不是听文档的。
fn view_height(b: &BoardDef) -> f32 {
    let size = std::env::var("NM_CAM_SIZE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(dim::CAM_SIZE);
    if size > f32::EPSILON {
        return size;
    }
    // XZ 平面矩形在 45° 方位 + 54.7° 俯角下的屏幕高 ≈ (宽+深)·sin45°·sin54.7°，再留 25% 边距
    let (min, max) = board_aabb(b);
    ((max.x - min.x) + (max.z - min.z)) * 0.707 * 0.816 * 1.25
}

/// 所有道路格（含分支）+ 土地足迹的世界 AABB。
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

fn board_center(b: &BoardDef) -> Vec3 {
    let (min, max) = board_aabb(b);
    (min + max) * 0.5
}
