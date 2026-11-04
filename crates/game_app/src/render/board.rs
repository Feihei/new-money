//! 静态棋盘几何：无缝地面密铺 + 路面（含对角段）+ 道路格地板（盒子版）。
//!
//! 对应 Godot `board.gd:206-302`（`_build_board` / `_build_roads` / `_render_filler_tiles`）。
//! 土地的底座与建筑不在这里 —— 它们挂在 Land 实体上（[`super::land`]），因为要随归属与等级变。
//!
//! 「同一条街的格子怎么认出来」靠的是**空间**：地块只出现在直线上的一段连续格子里，
//! 段与段之间隔着非地块格（银行／命运／公园…）。原先为此在道路格上画了一条同色组标记条
//! （Godot `tile_view.gd:114-160`），11-04 撤除 —— 数据核对过，两张真盘的每一条街
//! 本来就是一段连续格子（taiwan 16 街＝16 段、china 20 街＝20 段，无混段无拆段），
//! 色条不提供任何信息，只提供一层与无缝地面打架的几何。

use std::collections::{HashMap, HashSet};

use bevy::prelude::*;
use game_core::board::{BoardDef, TileDef};
use game_core::prelude::ActiveBoard;

use super::{
    BoardPaper, GroundArt, Rendered, TilePick, darkened, dim, flat, palette, photo, plated,
    yaw_to_face,
};

/// 一格贴哪个道路单元。词汇表就是 `assets/textures/road/units/` 那七张图，每张接哪几边
/// 由 `.tmp/road_compositor.cjs` 的 `UNITS` 折线定义 —— 是画之前就定好的，不是画完再猜的。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Unit {
    /// 草坪：道路不经过的格子（地块足迹、棋盘外圈）
    Grass,
    /// 直路 `straight_orth`
    Straight,
    /// 直角弯 `turn_90`
    Corner,
    /// 丁字 `junction_t`（支线从主环拐出去那一格）
    Junction,
    /// 断头路 `endpoint`（支线尽头）
    DeadEnd,
    /// 斜直路 `straight_diag`
    Diag,
    /// 45° 缓弯 `turn_45`
    Bend45,
    /// 135° 急折返 `turn_135`（11-04 补的那张：prototype 上有 7 处，原先路面在那儿断一截）
    Bend135,
}

impl Unit {
    /// 材质表下标。与 `RoadTiles::table` 的配对表同源，改这里要一起改那张表。
    pub(crate) fn index(self) -> usize {
        match self {
            Unit::Grass => 0,
            Unit::Straight => 1,
            Unit::Corner => 2,
            Unit::Junction => 3,
            Unit::DeadEnd => 4,
            Unit::Diag => 5,
            Unit::Bend45 => 6,
            Unit::Bend135 => 7,
        }
    }
}

/// 一格该贴的单元与档位。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct GroundTile {
    pub unit: Unit,
    /// 这张图转几档，每档 **45°、俯视顺时针**（`N→NE→E→…`）。正交类只会拿到偶数档。
    pub rot: u8,
}

/// 八个方向，**顺时针**编号。编号顺序就是「转一档」的方向，所以旋转 = 位左移。
///
/// `N = -row`、`E = +col` —— 由 `BoardDef::world_pos` 是 `(col, 0, row)`
/// （core `board.rs:460`）与「贴图 v=0 在图的顶边」推出来。符号反了不会报错，
/// 只会让所有路都拐向镜像方向。
mod dir {
    pub const N: u8 = 0;
    pub const NE: u8 = 1;
    pub const E: u8 = 2;
    pub const SE: u8 = 3;
    pub const S: u8 = 4;
    pub const SW: u8 = 5;
    pub const W: u8 = 6;
    pub const NW: u8 = 7;

    pub const fn bit(d: u8) -> u8 {
        1 << d
    }
}

/// 俯视顺时针转一档（八个方向 = 一个 `u8` 的八个位，转一档就是循环左移一位）。
fn turn_cw(mask: u8) -> u8 {
    mask.rotate_left(1)
}

fn rotate_n(mask: u8, n: u8) -> u8 {
    (0..n).fold(mask, |m, _| turn_cw(m))
}

/// 邻格在哪个方向（一位掩码）。同格与跨了两格以上的都不算邻接，返回 0 = 不接任何边。
///
/// 这道闸不是洁癖：`signum` 会把「同方向隔两格」折成同一个方向，数据写错时它不会
/// 报「认不出来」，而是安静地贴上一张接不上缝的图 —— 症状是路面断一截。
fn dir_toward(from: (i64, i64), to: (i64, i64)) -> u8 {
    let (dc, dr) = (to.0 - from.0, to.1 - from.1);
    if dc.abs().max(dr.abs()) != 1 {
        return 0;
    }
    let d = match (dc.signum(), dr.signum()) {
        (1, 0) => dir::E,
        (1, -1) => dir::NE,
        (0, -1) => dir::N,
        (-1, -1) => dir::NW,
        (-1, 0) => dir::W,
        (-1, 1) => dir::SW,
        (0, 1) => dir::S,
        (1, 1) => dir::SE,
        _ => return 0,
    };
    dir::bit(d)
}

/// 每格的接边集合。主环给两侧；支线在环格上多出第三侧（那就是丁字），
/// 支线尽头只剩一侧（断头）。只有道路格会进这张表。
pub(crate) fn road_sides(b: &BoardDef) -> HashMap<(u32, u32), u8> {
    let xy = |t: &TileDef| (t.col as i64, t.row as i64);
    let mut out: HashMap<(u32, u32), u8> = HashMap::new();
    let n = b.tiles.len();
    for i in 0..n {
        let t = &b.tiles[i];
        let mut mask = 0;
        for j in [if i == 0 { n - 1 } else { i - 1 }, (i + 1) % n] {
            mask |= dir_toward(xy(t), xy(&b.tiles[j]));
        }
        let branch = t.branch.as_ref();
        if let Some(first) = branch.and_then(|br| br.tiles.first()) {
            mask |= dir_toward(xy(t), xy(first));
        }
        *out.entry((t.col, t.row)).or_default() |= mask;
        let Some(br) = branch else { continue };
        let mut prev = xy(t);
        for (k, bt) in br.tiles.iter().enumerate() {
            // 支线自己那一串：朝来路一侧，若有后路再加一侧；尽头那一格就只剩一侧
            let mut m = dir_toward(xy(bt), prev);
            if let Some(next) = br.tiles.get(k + 1) {
                m |= dir_toward(xy(bt), xy(next));
            }
            prev = xy(bt);
            *out.entry((bt.col, bt.row)).or_default() |= m;
        }
    }
    out
}

/// 每个单元的原始接边（图上量的）与允许的档位。
///
/// 正交类只转 90° 的整数倍：转 45° 会画出一条不接边的斜带，而 `straight_diag` 转 45°
/// 恰好等于 `straight_orth` 转 90° —— 两个单元抢同一格时先表先得会选错图。
/// 各单元的两侧夹角互不相同（对顶 180°／90°／135°／45°、单边、三边），所以这张表没有歧义，
/// `tests::no_two_units_claim_the_same_edges` 钉着它。
const CATALOG: [(Unit, u8, &[u8]); 7] = [
    (Unit::DeadEnd, dir::bit(dir::W), &[0, 1, 2, 3, 4, 5, 6, 7]),
    (Unit::Straight, dir::bit(dir::W) | dir::bit(dir::E), &[0, 2]),
    (Unit::Diag, dir::bit(dir::NW) | dir::bit(dir::SE), &[0, 2]),
    (
        Unit::Corner,
        dir::bit(dir::N) | dir::bit(dir::W),
        &[0, 2, 4, 6],
    ),
    (
        Unit::Junction,
        dir::bit(dir::N) | dir::bit(dir::W) | dir::bit(dir::E),
        &[0, 2, 4, 6],
    ),
    (
        Unit::Bend45,
        dir::bit(dir::W) | dir::bit(dir::NE),
        &[0, 1, 2, 3, 4, 5, 6, 7],
    ),
    (
        Unit::Bend135,
        dir::bit(dir::W) | dir::bit(dir::NW),
        &[0, 1, 2, 3, 4, 5, 6, 7],
    ),
];

/// 接边集合 → 单元与档位。认不出来就是 `None`：那一格退回草地，比硬凑一张接不上缝的图
/// 诚实（四岔路口、跨了两格以上的邻接都是这种）。
pub(crate) fn tile_for(mask: u8) -> Option<(Unit, u8)> {
    if mask == 0 {
        return None;
    }
    CATALOG.into_iter().find_map(|(unit, canon, rots)| {
        rots.iter()
            .find(|&&r| rotate_n(canon, r) == mask)
            .map(|&r| (unit, r))
    })
}

/// 全盘每一格贴什么，下标 `row * grid_cols + col`。
pub(crate) fn ground_map(b: &BoardDef) -> Vec<GroundTile> {
    let sides = road_sides(b);
    let grass = GroundTile {
        unit: Unit::Grass,
        rot: 0,
    };
    let mut map = vec![grass; (b.grid_cols * b.grid_rows) as usize];
    for (row, line) in map.chunks_mut(b.grid_cols as usize).enumerate() {
        for (col, cell) in line.iter_mut().enumerate() {
            let Some(&mask) = sides.get(&(col as u32, row as u32)) else {
                continue;
            };
            if let Some((unit, rot)) = tile_for(mask) {
                *cell = GroundTile { unit, rot };
            }
        }
    }
    map
}

/// 把一张方形贴图平铺在地面上：先躺平（图的顶边朝 `-row`），再绕世界 Y 转 `steps` 档、
/// 每档 `step` 弧度。顺序换过来就是绕**已躺平的**轴转，整盘会歪着贴。
pub(crate) fn lay_flat(steps: f32, step: f32) -> Quat {
    Quat::from_rotation_y(-steps * step) * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

/// 无缝版地面：一格一张贴图，路面在图里，不再单独铺盒子。
///
/// 材质只有八份（草 + 七张单元），网格只有一份 —— 盒子版是每格一个 `Cuboid` mesh
/// 加一个自己的材质，784 格就是 784 份。
fn spawn_ground_tiles(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    table: &[(Unit, &Handle<Image>)],
) -> usize {
    // 单元图是不透明的，用 `plated()` 的 Mask 判据没意义（那里是给纸卡的透明角准备的）。
    // 表序就是 `Unit::index()` 序 —— 这一句是唯一的耦合点，配对表写错立刻红在这里，
    // 而不是「某一种路口永远贴成另一种样子」（那要盯着棋盘看半天才发现）。
    debug_assert!(
        table
            .iter()
            .enumerate()
            .all(|(i, (unit, _))| unit.index() == i),
        "道路单元表的下标与 `Unit::index()` 不同源：{table:?}"
    );
    let by_unit: Vec<Handle<StandardMaterial>> = table
        .iter()
        .map(|(_, img)| photo(mats, img, false))
        .collect();
    let quad = meshes.add(Rectangle::new(b.cell_size, b.cell_size));
    let sides = road_sides(b);
    let mut spawned = 0;
    for (index, cell) in ground_map(b).iter().enumerate() {
        let (c, r) = (
            (index % b.grid_cols as usize) as u32,
            (index / b.grid_cols as usize) as u32,
        );
        let handle = by_unit[cell.unit.index()].clone();
        let pos = b.world_pos(c, r);
        let rot = lay_flat(cell.rot as f32, std::f32::consts::FRAC_PI_4);
        let mut spawn = commands.spawn((
            Mesh3d(quad.clone()),
            MeshMaterial3d(handle),
            Transform::from_xyz(pos.x, dim::TILE_Y, pos.z).with_rotation(rot),
            Rendered,
        ));
        // 点格详情卡认的还是道路格那一格：`TilePick` + `Pickable` 成对出现，缺一个就是「点了没反应」
        if sides.contains_key(&(c, r)) {
            spawn.insert((TilePick { col: c, row: r }, Pickable::default()));
        }
        spawned += 1;
    }
    spawned
}

/// 一次性铺好不会变的几何。之后只有土地建筑与棋子会动。
///
/// 每块都挂 [`Rendered`]：回主菜单要靠它一次性清场，漏一块就会在下一局棋盘下面
/// 留一块颜色不对的地板，而那种「偶发的脏格子」最难查回来源。
pub(crate) fn spawn_board_geometry(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    art: GroundArt<'_>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let b = &active.def;
    let odd = spawn_branch_buildings(&mut commands, &mut meshes, &mut mats, b);
    match art.units.table() {
        Some(table) => {
            let tiles = spawn_ground_tiles(&mut commands, &mut meshes, &mut mats, b, &table);
            info!("棋盘几何（无缝）：地面 {tiles} 格 / 支线尽端建筑 {odd} 座");
        }
        None => {
            let roads = spawn_roads(&mut commands, &mut meshes, &mut mats, b);
            let floors = spawn_tile_floors(&mut commands, &mut meshes, &mut mats, b, &art.paper);
            let filler = spawn_ground(&mut commands, &mut meshes, &mut mats, b, &art.paper);
            info!(
                "棋盘几何（盒子）：路面 {roads} 段 / 道路格地板 {floors} 块 / 地面密铺 {filler} 块 \
                 / 支线尽端建筑 {odd} 座（土地视觉挂在 Land 实体上）"
            );
        }
    }
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
        Rendered,
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
    paper: &BoardPaper,
) -> usize {
    let mut blocks = 0;
    for t in &b.tiles {
        blocks += tile_floor(commands, meshes, mats, b, t, paper);
        if let Some(branch) = &t.branch {
            for bt in &branch.tiles {
                blocks += tile_floor(commands, meshes, mats, b, bt, paper);
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
    paper: &BoardPaper,
) -> usize {
    let pos = b.world_pos(t.col, t.row);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(b.cell_size, dim::FLOOR_H, b.cell_size))),
        MeshMaterial3d(plated(
            mats,
            darkened(palette::tile(t.kind), 0.5),
            &paper.plate,
        )),
        Transform::from_xyz(pos.x, dim::FLOOR_Y, pos.z),
        // 点格详情卡的命中目标：`TilePick` + `Pickable` 成对出现，缺一个就是「点了没反应」
        TilePick {
            col: t.col,
            row: t.row,
        },
        Pickable::default(),
        Rendered,
    ));
    1
}

/// 死胡同尽端的建筑（P2c）：监狱与医院在支线尽头各有一座楼，棋子被关进去时停在这里。
///
/// 它不挂 `TilePick`：点格详情卡认的是主环接入点，死胡同的楼只是「人停在这里」的视觉。
/// 高度比路面高一截，等轴测下才读得出「这条路尽头有栋房子」，否则与一格地板没区别。
fn spawn_branch_buildings(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
) -> usize {
    let mut built = 0;
    for t in &b.tiles {
        let Some(branch) = &t.branch else { continue };
        let Some(end) = branch.tiles.last() else {
            continue;
        };
        let pos = b.world_pos(end.col, end.row);
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

/// 网格里既不是道路格、也不是分支格、也不在土地足迹内的空格，用双色地面补齐
/// （`board.gd:264-302`）。少了这一步，棋盘外圈是空的背景色，等轴测下看着像纸片。
fn spawn_ground(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    b: &BoardDef,
    paper: &BoardPaper,
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
                MeshMaterial3d(plated(mats, checker, &paper.ground)),
                Transform::from_xyz(pos.x, dim::FLOOR_Y, pos.z),
                Rendered,
            ));
            blocks += 1;
        }
    }
    blocks
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::RoadTiles;

    fn board(which: &str) -> BoardDef {
        let text = match which {
            "taiwan" => include_str!("../../../../assets/data/boards/taiwan.json"),
            "china" => include_str!("../../../../assets/data/boards/china.json"),
            "prototype" => include_str!("../../../../assets/data/boards/prototype.json"),
            _ => unreachable!(),
        };
        serde_json::from_str(text).unwrap_or_else(|e| panic!("{which} 解析失败：{e}"))
    }

    /// `(草地, 直路, 直角, 丁字, 断头, 斜直, 45°弯, 135°折返)`
    fn census(b: &BoardDef) -> [usize; 8] {
        let mut c = [0usize; 8];
        for t in ground_map(b) {
            c[t.unit.index()] += 1;
        }
        c
    }

    // 下面几组数是 11-04 用一份独立实现（python，同一套接边表）在同一张盘上数出来的。
    // 改道路拓扑（加斜路、加支线）要连这几行一起改。八角形主环没有 90° 直角与 135°
    // 折返：四边是直路、四个角是 45° 斜街，街角与斜街接边都是 Bend45。
    #[test]
    fn taiwan_is_covered_by_straights_diagonals_and_45_bends() {
        let c = census(&board("taiwan"));
        assert_eq!(
            c,
            [1046, 86, 0, 2, 2, 12, 8, 0],
            "草地/直/直角/丁字/断头/斜/45/135"
        );
    }

    #[test]
    fn china_is_covered_by_straights_diagonals_and_45_bends() {
        let c = census(&board("china"));
        assert_eq!(c, [2059, 114, 0, 2, 2, 24, 8, 0]);
    }

    #[test]
    fn every_road_cell_on_the_two_shipped_boards_is_recognisable() {
        // 认不出接边的格子会静默退回草地，症状是「路面断一截」而不是报错。
        // 这条判据的牙在下面 `the_missing_vocabulary...` 里：同一张 prototype 上它就不为空。
        for which in ["taiwan", "china"] {
            let sides = road_sides(&board(which));
            assert!(
                !sides.is_empty(),
                "{which} 一个道路格都没分类出来，分类器根本没跑"
            );
            let unrecognised: Vec<_> = sides
                .iter()
                .filter(|(_, mask)| tile_for(**mask).is_none())
                .map(|(cell, mask)| (*cell, *mask))
                .collect();
            assert!(
                unrecognised.is_empty(),
                "{which} 有接边认不出来：{unrecognised:?}"
            );
        }
    }

    /// 演示盘（`prototype.json`，进不了窗口，只有测试读它）上还缺三种形状。
    /// 这条不是「已知会失败」，是把缺的词汇钉成一张清单：将来真盘加斜向岔口时，
    /// 要么补图、要么改这张表，总之不许让它静默变成一块草地。
    #[test]
    fn the_missing_vocabulary_is_a_known_list_not_a_surprise() {
        let sides = road_sides(&board("prototype"));
        let mut missing: Vec<u8> = sides
            .values()
            .filter(|m| tile_for(**m).is_none())
            .copied()
            .collect();
        missing.sort_unstable();
        let shown = missing
            .iter()
            .map(|m| format!("{m:08b}"))
            .collect::<Vec<_>>()
            .join(" ");
        // 两个「斜臂丁字」（两条正交臂 + 一条斜臂）与一个「斜对斜的直角弯」
        let want = [
            dir::bit(dir::NE) | dir::bit(dir::E) | dir::bit(dir::W),
            dir::bit(dir::N) | dir::bit(dir::SE) | dir::bit(dir::W),
            dir::bit(dir::SW) | dir::bit(dir::NW),
        ];
        assert_eq!(missing, want, "prototype 上认不出的接边集合变了：{shown}");
    }

    #[test]
    fn the_material_table_is_ordered_by_unit_index() {
        // `spawn_ground_tiles` 按下标取材质，而配对表在 `RoadTiles::table` 里手写顺序。
        // 两处不同源的症状是「某一种路口永远贴成另一种样子」——不会报错，只会看着不对
        let some = || Some(Handle::default());
        let tiles = RoadTiles {
            grass: some(),
            straight: some(),
            corner: some(),
            junction: some(),
            dead_end: some(),
            diag: some(),
            bend45: some(),
            bend135: some(),
        };
        let table = tiles.table().expect("八张齐全就该成套");
        assert_eq!(table.len(), 8);
        for (i, (unit, _)) in table.iter().enumerate() {
            assert_eq!(unit.index(), i, "第 {i} 位是 {unit:?}，与 `index()` 不同源");
        }
    }

    #[test]
    fn no_two_units_claim_the_same_edges() {
        // 词汇表有歧义时先表先得会静默选错图 —— 症状是某一种路口永远贴成另一种样子
        let mut claimed: HashMap<u8, Unit> = HashMap::new();
        for (unit, canon, rots) in CATALOG {
            for &r in rots {
                let mask = rotate_n(canon, r);
                if let Some(prev) = claimed.insert(mask, unit) {
                    assert_eq!(
                        prev, unit,
                        "接边 {mask:08b} 被 {prev:?} 与 {unit:?} 同时认领"
                    );
                }
            }
        }
    }

    #[test]
    fn each_rotation_lands_on_the_direction_the_ring_needs() {
        let b = dir::bit;
        let cases = [
            (b(dir::W) | b(dir::E), (Unit::Straight, 0)),
            (b(dir::N) | b(dir::S), (Unit::Straight, 2)),
            (b(dir::NW) | b(dir::SE), (Unit::Diag, 0)),
            (b(dir::NE) | b(dir::SW), (Unit::Diag, 2)),
            (b(dir::N) | b(dir::W), (Unit::Corner, 0)),
            (b(dir::E) | b(dir::N), (Unit::Corner, 2)),
            (b(dir::S) | b(dir::E), (Unit::Corner, 4)),
            (b(dir::W) | b(dir::S), (Unit::Corner, 6)),
            (b(dir::W), (Unit::DeadEnd, 0)),
            (b(dir::N), (Unit::DeadEnd, 2)),
            (b(dir::SE), (Unit::DeadEnd, 5)),
            (b(dir::N) | b(dir::W) | b(dir::E), (Unit::Junction, 0)),
            (b(dir::E) | b(dir::S) | b(dir::N), (Unit::Junction, 2)),
            // 45° 缓弯与 135° 急折返只差在两条臂的夹角上：135° 与 45°
            (b(dir::W) | b(dir::NE), (Unit::Bend45, 0)),
            (b(dir::W) | b(dir::NW), (Unit::Bend135, 0)),
            // 两臂夹角 135° 是缓弯那一族，夹角 45° 才是急折返 —— 差一个档位判据就反了
            (b(dir::NE) | b(dir::S), (Unit::Bend45, 3)),
            (b(dir::N) | b(dir::NE), (Unit::Bend135, 2)),
        ];
        for (mask, want) in cases {
            assert_eq!(
                tile_for(mask),
                Some(want),
                "接边 {mask:08b} 该转成 {want:?}"
            );
        }
    }

    #[test]
    fn a_four_way_cross_is_not_in_the_vocabulary() {
        // 四岔路口没有对应的图：宁可乐成草地，也不硬凑一张接不上缝的
        assert_eq!(
            tile_for(dir::bit(dir::N) | dir::bit(dir::E) | dir::bit(dir::S) | dir::bit(dir::W)),
            None
        );
        assert_eq!(tile_for(0), None);
    }

    #[test]
    fn the_ground_map_addresses_by_row_then_col() {
        // `ground_map` 的下标是 `row * grid_cols + col`，摆错一格就是整盘错位而不报错
        let b = board("taiwan");
        let map = ground_map(&b);
        assert_eq!(map.len(), (b.grid_cols * b.grid_rows) as usize);
        let jail = b
            .tiles
            .iter()
            .find(|t| t.branch.is_some())
            .expect("台湾盘应有监狱/医院两条支线");
        let index = (jail.row * b.grid_cols + jail.col) as usize;
        assert_eq!(map[index].unit, Unit::Junction);
    }

    #[test]
    fn a_diagonal_neighbour_is_a_direction_not_a_failure() {
        // 旧的四边模型把斜向邻格判成「认不出来」；八方向里它是合法的一条臂
        assert_eq!(dir_toward((0, 0), (1, -1)), dir::bit(dir::NE));
        assert_eq!(dir_toward((0, 0), (-1, 1)), dir::bit(dir::SW));
        // 同格与跨两格都不是邻接：不接任何边
        assert_eq!(dir_toward((0, 0), (0, 0)), 0);
        assert_eq!(dir_toward((0, 0), (2, 0)), 0);
    }
}
