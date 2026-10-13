//! 棋盘几何：把 `BoardDef` 的网格坐标铺成等轴测可见物。
//! 对应 Godot 端 `board.gd:219-302`（路面/填充地面）与 `tile_view.gd:92-127`（踏板/组色条）。

use bevy::prelude::*;
use game_core::board::{LandDef, TileDef, TileKind};

use crate::ActiveBoard;

const ROAD_W: f32 = 0.9;
const PAD_H: f32 = 0.06;
const ROAD_H: f32 = 0.12;

#[derive(Component)]
struct BoardPiece;

/// 与 Godot 端一致：材质全部 unshaded，不依赖光照（`board.gd` 里 StandardMaterial3D 都是
/// `SHADING_MODE_UNSHADED`），否则单一光源会把色组颜色压成看不出区别的暗块。
fn flat(c: Color) -> StandardMaterial {
    StandardMaterial {
        base_color: c,
        unlit: true,
        ..default()
    }
}

pub fn spawn_board(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    board: Res<ActiveBoard>,
) {
    let b = &board.0;

    for i in 0..b.tiles.len() {
        let from = &b.tiles[i];
        let to = &b.tiles[(i + 1) % b.tiles.len()];
        spawn_road_segment(&mut commands, &mut meshes, &mut mats, b.cell_size, from, to);
    }

    for t in &b.tiles {
        let pos = b.world_pos(t.col, t.row);
        let color = tile_color(t, &b.lands);
        commands.spawn((
            BoardPiece,
            Mesh3d(meshes.add(Cuboid::new(b.cell_size * 0.86, PAD_H, b.cell_size * 0.86))),
            MeshMaterial3d(mats.add(flat(color))),
            Transform::from_xyz(pos.x, PAD_H / 2.0, pos.z),
        ));
        if let Some(branch) = &t.branch {
            for bt in &branch.tiles {
                let p = b.world_pos(bt.col, bt.row);
                commands.spawn((
                    BoardPiece,
                    Mesh3d(meshes.add(Cuboid::new(b.cell_size * 0.8, PAD_H, b.cell_size * 0.8))),
                    MeshMaterial3d(mats.add(flat(Color::srgb(0.45, 0.42, 0.5)))),
                    Transform::from_xyz(p.x, PAD_H / 2.0, p.z),
                ));
            }
        }
    }

    for l in &b.lands {
        let Some(center) = b.land_world_pos(l.id) else {
            continue;
        };
        let side = b.cell_size * l.size as f32 * 0.8;
        let rgb = group_rgb(&l.group);
        // 底座（建筑高度按 level 堆叠留给 B4）
        commands.spawn((
            BoardPiece,
            Mesh3d(meshes.add(Cuboid::new(side, 0.3, side))),
            MeshMaterial3d(mats.add(flat(Color::srgb(rgb.0 * 0.55, rgb.1 * 0.55, rgb.2 * 0.55)))),
            Transform::from_xyz(center.x, 0.15, center.z),
        ));
        // 同色组标记条
        commands.spawn((
            BoardPiece,
            Mesh3d(meshes.add(Cuboid::new(side, 0.5, side * 0.12))),
            MeshMaterial3d(mats.add(flat(group_color(&l.group)))),
            Transform::from_xyz(center.x, 0.4, center.z),
        ));
    }

    let branch_pads: usize = b
        .tiles
        .iter()
        .filter_map(|t| t.branch.as_ref())
        .map(|br| br.tiles.len())
        .sum();
    info!(
        "棋盘图元：路面 {} 段 / 道路格 {} 块 / 分支格 {branch_pads} 块 / 土地 {} 块",
        b.tiles.len(),
        b.tiles.len(),
        b.lands.len()
    );
}

fn spawn_road_segment(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    cell_size: f32,
    from: &TileDef,
    to: &TileDef,
) {
    let a = Vec3::new(
        from.col as f32 * cell_size,
        0.0,
        from.row as f32 * cell_size,
    );
    let c = Vec3::new(to.col as f32 * cell_size, 0.0, to.row as f32 * cell_size);
    let delta = c - a;
    let len = delta.x.hypot(delta.z);
    if len <= f32::EPSILON {
        return;
    }
    // 绕 +Y 旋转把局部 +X 对齐到 (dx,0,dz)：与 Godot 端 rotation.y=atan2(-dz,dx) 同式
    let yaw = delta.x.atan2(-delta.z);
    commands.spawn((
        BoardPiece,
        Mesh3d(meshes.add(Cuboid::new(len, ROAD_H, cell_size * ROAD_W))),
        MeshMaterial3d(mats.add(flat(Color::srgb(0.33, 0.34, 0.38)))),
        Transform::from_xyz((a.x + c.x) / 2.0, ROAD_H / 2.0, (a.z + c.z) / 2.0)
            .with_rotation(Quat::from_rotation_y(yaw)),
    ));
}

fn tile_color(t: &TileDef, lands: &[LandDef]) -> Color {
    match t.kind {
        TileKind::Start => Color::srgb(0.85, 0.72, 0.25),
        TileKind::Bank => Color::srgb(0.25, 0.45, 0.8),
        TileKind::Chance => Color::srgb(0.3, 0.7, 0.4),
        TileKind::Fate => Color::srgb(0.6, 0.3, 0.75),
        TileKind::Stock => Color::srgb(0.2, 0.65, 0.7),
        TileKind::Property => lands
            .iter()
            .find(|l| Some(l.id) == t.land_id)
            .map(|l| group_color(&l.group))
            .unwrap_or(Color::srgb(0.5, 0.5, 0.55)),
        TileKind::Jail | TileKind::Hospital => Color::srgb(0.55, 0.4, 0.35),
        TileKind::Neutral
        | TileKind::Road
        | TileKind::Transport
        | TileKind::Lottery
        | TileKind::GodDeity => Color::srgb(0.5, 0.5, 0.55),
    }
}

fn group_rgb(group: &Option<String>) -> (f32, f32, f32) {
    match group.as_deref() {
        Some("brown") => (0.55, 0.36, 0.2),
        Some("orange") => (0.9, 0.55, 0.2),
        Some("red") => (0.85, 0.25, 0.25),
        Some("yellow") => (0.9, 0.82, 0.3),
        Some("green") => (0.3, 0.75, 0.35),
        Some("blue") => (0.3, 0.5, 0.9),
        _ => (0.6, 0.6, 0.6),
    }
}

fn group_color(group: &Option<String>) -> Color {
    let (r, g, b) = group_rgb(group);
    Color::srgb(r, g, b)
}
