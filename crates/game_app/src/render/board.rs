//! 静态棋盘几何：三层地面 + 支线尽端建筑。
//!
//! 第一层是一张铺满全盘的草地（每格 UV 各自 0..1，整块一次 draw）；第二层是街面，
//! 每格一张按街道朝向转过的直路图 —— 相邻两格中心距正好等于方图边长，所以它们边对边
//! 咬合成一条连续街带，与直街斜街无关；第三层是转角与三岔处的沥青圆盘，盖住方图
//! 盖不住的那道缝。
//!
//! 土地的底座与建筑不在这里 —— 它们挂在 Land 实体上（[`super::land`]），因为要随归属与等级变。
//! 位置与朝向全部来自 [`BoardGeom`]（等步长折线，见 [`super::geom`]），不读
//! `BoardDef::world_pos`：网格只剩「方向」这一件事。

use bevy::prelude::*;
use game_core::board::BoardDef;
use game_core::prelude::ActiveBoard;

use super::{GroundArt, Rendered, darkened, dim, flat, geom::BoardGeom, palette, photo};

/// 一次性铺好不会变的几何。之后只有土地建筑与棋子会动。
///
/// 每块都挂 [`Rendered`]：回主菜单要靠它一次性清场，漏一块就会在下一局棋盘下面
/// 留一块颜色不对的地板，而那种「偶发的脏格子」最难查回来源。
pub(crate) fn spawn_board_geometry(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    art: GroundArt<'_>,
    geom: Res<BoardGeom>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let b = &active.def;
    let odd = spawn_branch_buildings(&mut commands, &mut meshes, &mut mats, b, &geom);
    // 草地是**点击的唯一落点** —— 整盘只有这一块是可拾取的网格，于是「点在 UI 上」与
    // 「点在地面上」各有明确结果，也不必给每格铺一块不可见地皮
    // （见 `ui::tile_info::pick_tile`：射线打到这一层，再按 `geom.picks` 找最近的一格）。
    let grass = match &art.units.grass {
        Some(img) => photo(&mut mats, img, false),
        None => flat(&mut mats, palette::FILLER_A),
    };
    commands.spawn((
        Mesh3d(tiled_plate(
            &mut meshes,
            geom.plate.cols,
            geom.plate.rows,
            b.cell_size,
        )),
        MeshMaterial3d(grass),
        Transform::from_xyz(geom.plate.at.x, dim::TILE_Y - 0.02, geom.plate.at.z),
        Pickable::default(),
        Rendered,
    ));
    let tiles = spawn_street_ground(&mut commands, &mut meshes, &mut mats, b, &geom, &art);
    info!("棋盘几何：街面 {tiles} 格 / 支线尽端建筑 {odd} 座");
}

/// 街面二三层：一格一张按街道朝向转过的直路图，转角与三岔处再盖一块沥青圆盘。
fn spawn_street_ground(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    geom: &BoardGeom,
    art: &GroundArt<'_>,
) -> usize {
    let quad = meshes.add(Rectangle::new(b.cell_size, b.cell_size));
    // 素材不齐时（别的机器上是正常状态，AGENTS.md 硬约束 8）退回纯色，形状照样成立
    let road = match &art.units.straight {
        Some(img) => photo(mats, img, false),
        None => flat(mats, palette::ROAD),
    };
    // L1 街面：一格一张按街道朝向转过的直路图。咬合由几何保证（相邻两格中心距正好
    // 等于方图边长），所以不需要接边词汇表；转角格那一点溢出由 L2 的圆盘盖住。
    let mut spawned = 0;
    for (i, _) in b.tiles.iter().enumerate() {
        let Some(frame) = geom.tile(i) else {
            continue;
        };
        spawned += street_quad(commands, &quad, &road, frame, dim::TILE_Y);
    }
    for t in &b.tiles {
        let Some(branch) = &t.branch else { continue };
        for bt in &branch.tiles {
            let frame = geom.at(bt.col, bt.row);
            spawned += street_quad(commands, &quad, &road, frame, dim::TILE_Y);
        }
    }
    // L2 转角盘：转角格那张方图按角平分线转，沥青带与前后两段都**接不上**（两段的开口
    // 是两条斜线，而逐格贴图不能裁），贴图层面无解 —— 盖一块沥青圆盘。
    //
    // 半径按「两条带子外沿的交点」算：`(W/2) / sin(转角/2)`，W = 0.5 格。45° 转角要
    // 0.65 格、90° 要 0.71 格，取 0.75 格把两种转角都盖住；再大就盖到邻格的沥青上、
    // 读成一个广场（当前两张盘的转角都是 45°，90° 是给将来的盘留的）。
    // 三岔的接入点也铺：支线多半从侧面插进来，同一个接不上边的问题。
    let disc = meshes.add(Circle::new(b.cell_size * 0.75));
    let disc_mat = flat(mats, palette::ASPHALT);
    for (i, t) in b.tiles.iter().enumerate() {
        let Some(frame) = geom.tile(i) else {
            continue;
        };
        if !frame.bend && t.branch.is_none() {
            continue;
        }
        commands.spawn((
            Mesh3d(disc.clone()),
            MeshMaterial3d(disc_mat.clone()),
            Transform::from_xyz(frame.at.x, dim::TILE_Y + 0.02, frame.at.z)
                .with_rotation(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)),
            Rendered,
        ));
    }
    spawned
}

/// 一张铺满整盘的草地：`cols × rows` 个格子拼成的单块网格，每格 UV 各自 0..1。
///
/// 不这么做就得在材质上开 Repeat 采样器 —— `StandardMaterial` 没有采样器那一格，
/// 只能改共享的 `Image`。每格独立 UV 既不动素材，也只剩一次 draw。
fn tiled_plate(meshes: &mut Assets<Mesh>, cols: u32, rows: u32, cell: f32) -> Handle<Mesh> {
    let (w, h) = (cols as f32 * cell, rows as f32 * cell);
    let n = (cols * rows) as usize;
    let mut positions = Vec::with_capacity(n * 4);
    let mut uvs = Vec::with_capacity(n * 4);
    let mut indices = Vec::with_capacity(n * 6);
    for r in 0..rows {
        for c in 0..cols {
            let x0 = c as f32 * cell - w / 2.0;
            let z0 = r as f32 * cell - h / 2.0;
            let (x1, z1) = (x0 + cell, z0 + cell);
            let base = positions.len() as u32;
            // 绕序朝 +Y：从上方看是逆时针，法线才朝上（反面会被剔掉，地面就成了洞）
            positions.extend([[x0, 0.0, z0], [x0, 0.0, z1], [x1, 0.0, z1], [x1, 0.0, z0]]);
            uvs.extend([[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]]);
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    let normals = vec![[0.0, 1.0, 0.0]; positions.len()];
    let mut mesh = Mesh::new(
        bevy::render::mesh::PrimitiveTopology::TriangleList,
        bevy::asset::RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    mesh.insert_indices(bevy::mesh::Indices::U32(indices));
    meshes.add(mesh)
}

fn street_quad(
    commands: &mut Commands,
    quad: &Handle<Mesh>,
    mat: &Handle<StandardMaterial>,
    frame: super::geom::Frame,
    y: f32,
) -> usize {
    // 转角那格按角平分线转，方图与前后两格各重叠一小块：同层就是 z 打架，
    // 抬高一档让它稳定压在上面，缝的形状才读得出来
    let y = y + f32::from(frame.bend) * 0.01;
    commands.spawn((
        Mesh3d(quad.clone()),
        MeshMaterial3d(mat.clone()),
        Transform::from_xyz(frame.at.x, y, frame.at.z).with_rotation(
            Quat::from_rotation_y(frame.yaw) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
        ),
        Rendered,
    ));
    1
}

/// 死胡同尽端的建筑（P2c）：监狱与医院在支线尽头各有一座楼，棋子被关进去时停在这里。
///
/// 它不参与拾取：点格详情卡认的是主环接入点，死胡同的楼只是「人停在这里」的视觉。
/// 高度比路面高一截，等轴测下才读得出「这条路尽头有栋房子」，否则与一格地板没区别。
fn spawn_branch_buildings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    geom: &BoardGeom,
) -> usize {
    let mut built = 0;
    for t in &b.tiles {
        let Some(branch) = &t.branch else { continue };
        let Some(end) = branch.tiles.last() else {
            continue;
        };
        let pos = geom.at(end.col, end.row).at;
        let h = b.cell_size * 1.6;
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(b.cell_size * 0.78, h, b.cell_size * 0.78))),
            MeshMaterial3d(flat(mats, darkened(palette::tile(end.kind), 0.15))),
            Transform::from_xyz(pos.x, h * 0.5, pos.z),
            Rendered,
        ));
        built += 1;
    }
    built
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::geom;

    fn board(which: &str) -> BoardDef {
        let text = match which {
            "taiwan" => include_str!("../../../../assets/data/boards/taiwan.json"),
            "china" => include_str!("../../../../assets/data/boards/china.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{which} 解析失败：{e}"))
    }

    fn shipped() -> [(&'static str, BoardDef); 2] {
        [("taiwan", board("taiwan")), ("china", board("china"))]
    }

    #[test]
    fn both_shipped_boards_close_their_ring() {
        // 折线没有闭合约束：环尾再走一步回不到第一格时，那道缝会在棋盘上留一个缺口。
        // 真盘是对称的八角形，缝应当就是 0 —— 报非零说明数据里的环不是「等步长闭合」的。
        for (name, b) in shipped() {
            let g = geom::build(&b);
            assert_eq!(g.ring_len(), b.tiles.len(), "{name} 主环格数对不上");
            assert!(
                g.closure_gap < b.cell_size * 0.01,
                "{name} 的环没闭合：缝 {:.3}（一格 {:.3}）",
                g.closure_gap,
                b.cell_size
            );
        }
    }

    #[test]
    fn the_ring_advances_exactly_one_cell_per_step() {
        // 斜街两格的中心距必须与直街一样是 `cell_size`：差这一步，方块就永远咬不齐，
        // 而画面上的症状只是「斜街看着有点碎」。
        for (name, b) in shipped() {
            let g = geom::build(&b);
            for i in 1..g.ring_len() {
                let (here, prev) = (g.tile(i).unwrap(), g.tile(i - 1).unwrap());
                let d = here.at.distance(prev.at);
                assert!(
                    (d - b.cell_size).abs() < 1e-3,
                    "{name} 第 {i} 格与前一格相距 {d:.3}，应为 {:.3}",
                    b.cell_size
                );
            }
        }
    }

    #[test]
    fn neighbouring_plots_are_one_cell_apart_and_never_overlap() {
        // 咬合成立时，同一条街上相邻两块地的中心正好差一格；一旦「往外退多少」算错，
        // 它们就会压在一起，而画面上的症状只是「某条街少了一块地」。
        for (name, b) in shipped() {
            let g = geom::build(&b);
            assert_eq!(g.overlapping_lands, 0, "{name} 有地块压叠");
            assert_eq!(g.side_ambiguous, 0, "{name} 有地块读不出街的哪一侧");
            let mut compared = 0usize;
            for i in 0..b.tiles.len() {
                let (a, c) = (&b.tiles[i], &b.tiles[(i + 1) % b.tiles.len()]);
                let (Some(ia), Some(ic)) = (a.land_id, c.land_id) else {
                    continue;
                };
                if ia == ic {
                    continue;
                }
                let (Some(fa), Some(fc)) = (g.land(ia), g.land(ic)) else {
                    panic!("{name} 地块 {ia}/{ic} 没有几何");
                };
                // 只比同一条街的两个 1×1 邻块：转角两边的两块地分属两条街，而 2×2 的
                // 商业地绑两格、中心本来就退得更远
                let (Some(da), Some(dc)) = (b.land(ia), b.land(ic)) else {
                    panic!("{name} 地块 {ia}/{ic} 没有定义");
                };
                if da.group != dc.group || da.size != 1 || dc.size != 1 {
                    continue;
                }
                let d = fa.at.distance(fc.at);
                compared += 1;
                assert!(
                    (d - b.cell_size).abs() < 1e-2,
                    "{name} 第 {i} 格的两块地相距 {d:.3}，应为 {:.3}",
                    b.cell_size
                );
            }
            // 判据不能空跑：过滤条件写宽了会让这条测试一条都没比就绿
            assert!(compared >= 50, "{name} 只比了 {compared} 对邻块");
        }
    }

    #[test]
    fn every_ring_and_branch_cell_is_a_hit_point() {
        // 点击命中走这张坐标表：漏一格就是「那一格点不动」，多点一格就是两格争同一个点。
        for (name, b) in shipped() {
            let g = geom::build(&b);
            let branches: usize = b
                .tiles
                .iter()
                .map(|t| t.branch.as_ref().map_or(0, |br| br.tiles.len()))
                .sum();
            assert_eq!(
                g.picks.len(),
                b.tiles.len() + branches,
                "{name} 命中点数不对"
            );
            for (i, t) in b.tiles.iter().enumerate() {
                let here = g.tile(i).expect("环上每一格都该有姿态").at;
                let hit = g
                    .nearest_cell(here)
                    .unwrap_or_else(|| panic!("{name} 第 {i} 格自己点不中"));
                assert_eq!(hit, (t.col, t.row), "{name} 第 {i} 格点中的是别处");
            }
        }
    }

    #[test]
    fn the_street_stays_on_the_grass_plate() {
        // 街面若累积漂到草地之外，等轴测下就是一条悬在背景色上的路。
        for (name, b) in shipped() {
            let g = geom::build(&b);
            let half = Vec2::new(g.plate.cols as f32, g.plate.rows as f32) * b.cell_size * 0.5;
            for i in 0..g.ring_len() {
                let frame = g.tile(i).unwrap();
                let (dx, dz) = (frame.at.x - g.plate.at.x, frame.at.z - g.plate.at.z);
                assert!(
                    dx.abs() <= half.x + b.cell_size * 0.5
                        && dz.abs() <= half.y + b.cell_size * 0.5,
                    "{name} 第 {i} 格跑到草地外：( {dx:.1}, {dz:.1} )，草地半宽 {half:?}"
                );
            }
        }
    }
}
