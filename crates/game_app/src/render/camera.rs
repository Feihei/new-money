//! 相机：正交等轴测 + rig 跟随棋子。对应 Godot `board.gd:168-188`（`_setup_camera`）
//! 与 `:462-473`（`_follow_token` 的 0.35s 补间）。
//!
//! 结构照 Godot 的 rig：父节点只管 XZ 平移，子 `Camera3d` 固定挂着 (8,16,8) 的偏移
//! 并看向父原点 —— 于是「转镜头」与「推镜头」是两件互不干扰的事。

use bevy::{camera::ScalingMode, prelude::*};
use game_core::prelude::{ActiveBoard, BoardDef, PlayerId, TokenPosition, TurnOrder};

use super::{Rendered, dim, geom::BoardGeom, palette, token::TokenVisual};

#[derive(Component)]
pub(crate) struct CameraRig;

/// 正交等轴测相机。`ClearColor` 对应 Godot 的 `Environment.background_color`。
pub(crate) fn spawn_camera_rig(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    order: Res<TurnOrder>,
    geom: Res<BoardGeom>,
    players: Query<&TokenPosition, With<PlayerId>>,
) {
    let b = &active.def;
    // 开局先看着手玩家，而不是棋盘中心；整盘取景必须钉在中心，否则棋子随机落位会把盘推出窗口
    let focus = if full_board_view() {
        board_center(b, &geom)
    } else if let Some(at) = cam_at().and_then(|i| geom.tile(i)).map(|f| f.at) {
        at
    } else {
        order
            .current()
            .and_then(|p| players.get(p).ok())
            .and_then(|tp| geom.tile(tp.tile.0 as usize).map(|f| f.at))
            .unwrap_or_else(|| board_center(b, &geom))
    };

    commands
        .spawn((
            CameraRig,
            // 子相机的 `InheritedVisibility` 要从父节点继承，父节点没有 `Visibility`
            // 就会报 Bevy 的 B0004 警告（可见性传播断在 rig 这一层）
            Visibility::default(),
            Transform::from_xyz(focus.x, 0.0, focus.z),
            // 相机是 rig 的子节点，`despawn()` 递归回收，标记只挂在父上
            Rendered,
        ))
        .with_children(|rig| {
            rig.spawn((
                Camera3d::default(),
                // 只有标了 `MeshPickingCamera` 的相机参与网格命中测试（见 main.rs 的 require_markers）
                MeshPickingCamera,
                Camera {
                    clear_color: ClearColorConfig::Custom(palette::SKY),
                    ..default()
                },
                Projection::from(OrthographicProjection {
                    near: 0.1,
                    far: 200.0,
                    scaling_mode: ScalingMode::FixedVertical {
                        viewport_height: view_height(b, &geom),
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
    if full_board_view() || cam_at().is_some() {
        return;
    }
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
fn cam_size_override() -> Option<f32> {
    std::env::var("NM_CAM_SIZE")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
}

fn full_board_view() -> bool {
    cam_size_override().is_some_and(|s| s <= f32::EPSILON)
}

/// `NM_CAM_AT=<主环格号>`：把镜头钉在那一格上，不跟随任何人。
///
/// 核几何（转角咬合、支线死胡同、某个特定街段）时要用 —— 否则只能赌开局骰子把镜头
/// 落在附近，而「拍到了但与你想看的差两格」比没拍到更费时间。
fn cam_at() -> Option<usize> {
    std::env::var("NM_CAM_AT").ok()?.parse().ok()
}

fn view_height(b: &BoardDef, geom: &BoardGeom) -> f32 {
    let size = cam_size_override().unwrap_or(dim::CAM_SIZE);
    if size > f32::EPSILON {
        return size;
    }
    // XZ 平面矩形在 45° 方位 + 54.7° 俯角下的屏幕高 ≈ (宽+深)·sin45°·sin54.7°，再留 25% 边距
    let (min, max) = board_aabb(b, geom);
    ((max.x - min.x) + (max.z - min.z)) * 0.707 * 0.816 * 1.25
}

/// 所有道路格（含分支）+ 土地足迹的世界 AABB。
///
/// 折线版里方块是转过的，半幅要按旋转角算：`side/2 · (|cos|+|sin|)`。网格版里 yaw 恒 0，
/// 这个式子正好退化成原来那个 `side/2` —— 两条路共用一个函数而不必开分支。
fn board_aabb(b: &BoardDef, geom: &BoardGeom) -> (Vec3, Vec3) {
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    let mut fold = |p: Vec3, side: f32, yaw: f32| {
        let half = side * 0.5 * (yaw.cos().abs() + yaw.sin().abs());
        min = min.min(Vec3::new(p.x - half, 0.0, p.z - half));
        max = max.max(Vec3::new(p.x + half, 0.0, p.z + half));
    };
    for t in &b.tiles {
        let f = geom.at(t.col, t.row);
        fold(f.at, b.cell_size, f.yaw);
        if let Some(br) = &t.branch {
            for bt in &br.tiles {
                let g = geom.at(bt.col, bt.row);
                fold(g.at, b.cell_size, g.yaw);
            }
        }
    }
    for l in &b.lands {
        if let Some(f) = geom.land(l.id) {
            fold(f.at, f.side, f.yaw);
        }
    }
    (min, max)
}

fn board_center(b: &BoardDef, geom: &BoardGeom) -> Vec3 {
    let (min, max) = board_aabb(b, geom);
    (min + max) * 0.5
}
