//! 静态棋盘几何：地面密铺 + 路面（含对角段）+ 道路格地板 + 同色组标记条。
//!
//! 对应 Godot `board.gd:206-302`（`_build_board` / `_build_roads` / `_render_filler_tiles`）
//! 与 `tile_view.gd:83-152`（`_build_visual` 的地板与色条部分）。
//! 土地的底座与建筑不在这里 —— 它们挂在 Land 实体上（[`super::land`]），因为要随归属与等级变。

use std::collections::HashSet;

use bevy::prelude::*;
use game_core::board::{BoardDef, TileDef};
use game_core::prelude::ActiveBoard;

use super::{darkened, dim, flat, palette, yaw_to_face};

/// 一次性铺好不会变的几何。之后只有土地建筑与棋子会动。
pub(crate) fn spawn_board_geometry(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let b = &active.def;
    let roads = spawn_roads(&mut commands, &mut meshes, &mut mats, b);
    let floors = spawn_tile_floors(&mut commands, &mut meshes, &mut mats, b);
    let filler = spawn_ground(&mut commands, &mut meshes, &mut mats, b);
    info!(
        "棋盘几何：路面 {roads} 段 / 道路格地板 {floors} 块 / 地面密铺 {filler} 块（土地视觉挂在 Land 实体上）"
    );
}

/// 路面：主环相邻格之间（含末→首）+ 每条分支从接入点铺到死胡同。
/// 对应 `board.gd:219-234` —— 分支也是路，不铺的话死胡同看着像悬空的一块。
fn spawn_roads(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
) -> usize {
    let mut segments = 0;
    let next = |i: usize| (i + 1) % b.tiles.len();
    for i in 0..b.tiles.len() {
        segments += road_between(
            commands,
            meshes,
            mats,
            b,
            b.world_pos(b.tiles[i].col, b.tiles[i].row),
            b.world_pos(b.tiles[next(i)].col, b.tiles[next(i)].row),
        );
    }
    for t in &b.tiles {
        let Some(branch) = &t.branch else { continue };
        let mut prev = b.world_pos(t.col, t.row);
        for bt in &branch.tiles {
            let cur = b.world_pos(bt.col, bt.row);
            segments += road_between(commands, meshes, mats, b, prev, cur);
            prev = cur;
        }
    }
    segments
}

fn road_between(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    a: Vec3,
    c: Vec3,
) -> usize {
    let delta = c - a;
    let len = delta.xz().length();
    if len <= f32::EPSILON {
        return 0;
    }
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(len, dim::ROAD_H, b.cell_size * dim::ROAD_W))),
        MeshMaterial3d(flat(mats, palette::ROAD)),
        // 局部 +X 对齐到路段方向：正交段与对角段同一条公式（Godot `board.gd:248`）
        Transform::from_xyz((a.x + c.x) / 2.0, dim::ROAD_H / 2.0, (a.z + c.z) / 2.0)
            .with_rotation(Quat::from_rotation_y(yaw_to_face(delta))),
    ));
    1
}

/// 每个道路格（含分支死胡同格）一块整格地板，颜色按格子类型压暗（`tile_view.gd:92-96`）。
/// 同色组标记条只画在主环格上：分支格按 schema 不写 `id`（serde 默认成 0），
/// 拿它的 `id` 去查绑定会查到 0 号格的地，是假数据。
fn spawn_tile_floors(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
) -> usize {
    let mut blocks = 0;
    for t in &b.tiles {
        blocks += tile_floor(commands, meshes, mats, b, t);
        group_strip(commands, meshes, mats, b, t);
        if let Some(branch) = &t.branch {
            for bt in &branch.tiles {
                blocks += tile_floor(commands, meshes, mats, b, bt);
            }
        }
    }
    blocks
}

fn tile_floor(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    t: &TileDef,
) -> usize {
    let pos = b.world_pos(t.col, t.row);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(b.cell_size, dim::FLOOR_H, b.cell_size))),
        MeshMaterial3d(flat(mats, darkened(palette::tile(t.kind), 0.5))),
        Transform::from_xyz(pos.x, dim::FLOOR_Y, pos.z),
    ));
    1
}

/// 同色组标记条画在**道路格**上、朝向它绑定的那块地（ADR-001 §7 土地-道路绑定）。
/// Godot 端把它固定摆在 -Z（`tile_view.gd:114`）且颜色写死成 blue（`:157-160`），
/// 于是只有上边缘的格子看着对 —— 这里按真实朝向摆，见 §5.1 的差异记录。
fn group_strip(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    t: &TileDef,
) {
    let Some(land) = b.land_for_tile(t.id as i64) else {
        return;
    };
    let Some(center) = b.land_world_pos(land.id) else {
        return;
    };
    let pos = b.world_pos(t.col, t.row);
    let dir = Vec3::new(center.x - pos.x, 0.0, center.z - pos.z);
    if dir.length_squared() <= f32::EPSILON {
        return;
    }
    let dir = dir.normalize();
    // 长轴垂直于「格 → 地」方向，才贴着两者之间那条边
    let yaw = yaw_to_face(Vec3::new(-dir.z, 0.0, dir.x));
    let at = pos + dir * (b.cell_size * dim::STRIP_OFF);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(
            b.cell_size * land.size as f32 * 0.92,
            dim::STRIP_H,
            b.cell_size * dim::STRIP_DEPTH,
        ))),
        MeshMaterial3d(flat(
            mats,
            land.group
                .as_deref()
                .map(palette::group)
                .unwrap_or(Color::srgb(0.6, 0.6, 0.6)),
        )),
        Transform::from_xyz(at.x, dim::STRIP_Y, at.z).with_rotation(Quat::from_rotation_y(yaw)),
    ));
}

/// 网格里既不是道路格、也不是分支格、也不在土地足迹内的空格，用双色地面补齐
/// （`board.gd:264-302`）。少了这一步，棋盘外圈是空的背景色，等轴测下看着像纸片。
fn spawn_ground(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
) -> usize {
    let mut occupied: HashSet<(u32, u32)> = HashSet::new();
    for t in &b.tiles {
        occupied.insert((t.col, t.row));
        if let Some(branch) = &t.branch {
            for bt in &branch.tiles {
                occupied.insert((bt.col, bt.row));
            }
        }
    }
    // 土地跳过整块 size×size 足迹，否则 2x2 商业的下半会被地面块盖住（Godot 同注释）
    for l in &b.lands {
        for dc in 0..l.size as u32 {
            for dr in 0..l.size as u32 {
                occupied.insert((l.col + dc, l.row + dr));
            }
        }
    }

    let mut blocks = 0;
    for row in 0..b.grid_rows {
        for col in 0..b.grid_cols {
            if occupied.contains(&(col, row)) {
                continue;
            }
            let pos = b.world_pos(col, row);
            let checker = if (col + row) % 2 == 0 {
                palette::FILLER_A
            } else {
                palette::FILLER_B
            };
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(b.cell_size, dim::FLOOR_H, b.cell_size))),
                MeshMaterial3d(flat(mats, checker)),
                Transform::from_xyz(pos.x, dim::FLOOR_Y, pos.z),
            ));
            blocks += 1;
        }
    }
    blocks
}
