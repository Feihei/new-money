//! 「哪些地块算一片」——范围解析（P3c 从 [`crate::damage`] 里独立出来）。
//!
//! 范围是地块的拓扑性质，不是一种动词，所以卡（涨价／查封）、损害（拆房／夺地）、
//! 攻击道具（定时炸弹／飞弹）共用这一份解析。
//!
//! **11-05 起按道路格数算，不按网格正方形算**：路是线性的，斜街与直街上的「相邻」
//! 是同一件事，而网格上的 3×3 在斜街上会漏掉正前方那一格、又扫到背街的两格。
//! 于是 [`PlotArea::Nearby`] 的中心是**中心地块绑着的那几格路**，范围是沿主环
//! 前后各 `span` 格；[`lands_around_tile`] 同一条尺子，只是中心换成爆心那一格。
//!
//! 全部是 `BoardDef` 上的**纯函数**：不碰实体、不查组件，因此「环尾怎么折回来」
//! 这类裁决只在这里说一次，损害、新闻与道具各自拿同一份结果。

use crate::board::{BoardDef, LandDef, TileIndex};
use crate::land::LandId;
use serde::Deserialize;

/// 一片地块的范围。
///
/// 带 `Deserialize` 是因为**数据要能写它**（新闻的「这一带」、卡片的「整街」）：
/// 范围是卡与损害共用的概念，那就只能有一个类型，而不是卡那侧再抄一份字符串自己解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlotArea {
    /// 只打中心这一块。
    One,
    /// 沿主环**前后各 `span` 格**内的地块（中心那块永远在内）。
    ///
    /// `span: 0` 与 [`PlotArea::One`] 同义。道具那边三档：定时炸弹 `0`、
    /// 飞弹 `1`（当格 + 前后各一格 = 3 格）、原子弹 `3`（= 7 格）。
    /// 环是闭环，两端取更近的那一边；中心地块绑了几格路就按那几格各算一次取最小。
    Nearby { span: u32 },
    /// 整条路段：与中心同 `group` 的全部地块（「整路段夷为平地」「一排房屋消失」，
    /// 也是涨价卡「整街加倍」的那条街）。
    /// 中心那块没有 `group` 时只剩它自己 —— 缺配置不该扩大破坏面。
    Street,
    /// 全图。
    Whole,
}

/// 卡面不带范围时的默认值。写成 `Default` 而不是给每一条效果都补一个 `area` 字段：
/// 拍卖卡（P9）的 JSON 里本来就没有这一格，而它的语义就是「只打挑中那一块」——
/// 出牌请求已经带了那一块是哪个。新闻（P11c）则在数据里显式写范围。
impl Default for PlotArea {
    fn default() -> Self {
        Self::One
    }
}

/// 这块地在不在 `area` 里。`hub` 是中心地块的定义。
pub fn is_in_area(board: &BoardDef, hub: &LandDef, other: &LandDef, area: PlotArea) -> bool {
    match area {
        PlotArea::One => hub.id == other.id,
        PlotArea::Street => match &hub.group {
            None => hub.id == other.id,
            Some(group) => other.group.as_deref() == Some(group.as_str()),
        },
        PlotArea::Nearby { span } => land_hops(board, hub, other).is_some_and(|hops| hops <= span),
        PlotArea::Whole => true,
    }
}

/// 环上两格之间**沿路**的最短步数。
///
/// 主环是一个闭环，所以只看下标：`min(|a-b|, n-|a-b|)`。支线格没有 `TileIndex`，
/// 今天也没有任何范围以它们为中心（炸弹只能放在主环格上），所以这里不建图。
pub(crate) fn ring_steps(n: usize, a: usize, b: usize) -> u32 {
    if n == 0 {
        return 0;
    }
    let d = a.abs_diff(b);
    d.min(n - d) as u32
}

/// 两块地之间隔几格路：各自绑着的那几格两两取最小；自己与自己为 0。
///
/// 没有任何道路格绑着的地（数据错误）读作「够不着」，而不是「在中心」——
/// 后者会让一块坏数据凭空落进每一片范围里。
fn land_hops(board: &BoardDef, a: &LandDef, b: &LandDef) -> Option<u32> {
    if a.id == b.id {
        return Some(0);
    }
    let n = board.tiles.len();
    let bound_a = bound_tiles(board, a.id);
    let bound_b = bound_tiles(board, b.id);
    if bound_a.is_empty() || bound_b.is_empty() {
        return None;
    }
    bound_a
        .iter()
        .flat_map(|&x| bound_b.iter().map(move |&y| ring_steps(n, x, y)))
        .min()
}

/// 这块地绑在哪几格主环上（下标）。地块与道路格的绑定只有 `TileDef.land_id` 一处真相。
fn bound_tiles(board: &BoardDef, land: u32) -> Vec<usize> {
    board
        .tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.land_id == Some(land))
        .map(|(i, _)| i)
        .collect()
}

/// 以**道路格**为爆心的范围解析（P5 的炸弹、将来的飞弹／原子弹）。
///
/// 为什么不能直接走 [`area_of`]：那条路线的中心是一块**地**，而炸弹的爆心是一格**路**。
/// 命中判据是「这块地绑着的某一格，离爆心不超过 `span` 步」—— 与 [`PlotArea::Nearby`]
/// 同一把尺子，只是锚从「地的绑定格」换成「爆心那一格」。
pub fn lands_around_tile(board: &BoardDef, tile: TileIndex, span: u32) -> Vec<LandId> {
    // 用下标而不是 `tile()`：`tile()` 会回绕，于是 `TileIndex(99)` 会悄悄变成环上某格，
    // 「越界格什么都没打到」这条就用一条假中心炸出一片地
    let center = tile.0 as usize;
    let n = board.tiles.len();
    if center >= n {
        return Vec::new();
    }
    board
        .lands
        .iter()
        .filter(|land| {
            bound_tiles(board, land.id)
                .iter()
                .any(|&i| ring_steps(n, center, i) <= span)
        })
        .map(|land| LandId(land.id))
        .collect()
}

/// 范围解析：中心 + 范围 → 哪些地块被圈进来（按棋盘声明顺序，稳定可断言）。
///
/// 走 `BoardDef` 而不是查实体：范围是**静态拓扑**事实（道路格归属都在定义里），
/// 复制一份「哪些地属于同一条街」到运行时就是第二份真相来源（§4.2 的 B2a 修正）。
/// 棋盘里没有中心那块地时返回空 —— 数据错误该表现为「什么都没打着」，而不是炸掉一盘整盘。
pub fn area_of(board: &BoardDef, center: LandId, area: PlotArea) -> Vec<LandId> {
    let Some(hub) = board.land(center.0) else {
        return Vec::new();
    };
    board
        .lands
        .iter()
        .filter(|l| is_in_area(board, hub, l, area))
        .map(|l| LandId(l.id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{LandType, TileDef, TileKind};

    /// 造一块地：`(编号, 列, 行, 占边长, 路段)`
    fn plot(id: u32, col: u32, row: u32, size: u8, group: Option<String>) -> LandDef {
        LandDef {
            id,
            col,
            row,
            size,
            name: format!("地{id}"),
            group,
            land_type: LandType::Residential,
            price: 1_500,
            rent: vec![10, 50, 150, 450, 1_200, 3_000],
            upgrade_cost: 1_000,
            building_kind: None,
            fee: None,
        }
    }

    fn board(lands: Vec<LandDef>, tiles: Vec<TileDef>) -> BoardDef {
        BoardDef {
            board_name: "测试盘".into(),
            start_cash: 20_000,
            cell_size: 2.0,
            grid_cols: 8,
            grid_rows: 8,
            tiles,
            lands,
            facility_fees: Default::default(),
        }
    }

    /// 一条 `n` 格的环，每格绑一块 1×1 的地（地块编号 = 格号）。
    /// 格心坐标故意按网格摆（`(i,0)`），但**范围只读下标** —— 这正是要钉的那件事。
    fn ring(n: u32) -> BoardDef {
        let tiles = (0..n)
            .map(|i| TileDef {
                id: i,
                col: i,
                row: 0,
                kind: TileKind::Road,
                name: format!("格{i}"),
                land_id: Some(i),
                branch: None,
                data: Default::default(),
            })
            .collect();
        let lands = (0..n)
            .map(|i| plot(i, i, 0, 1, Some("街".into())))
            .collect();
        board(lands, tiles)
    }

    #[test]
    fn one_hits_only_the_center() {
        let b = ring(10);
        assert_eq!(
            area_of(&b, LandId(5), PlotArea::One),
            vec![LandId(5)],
            "只打中心"
        );
    }

    #[test]
    fn nearby_one_is_the_centre_and_its_two_road_neighbours() {
        let b = ring(10);
        assert_eq!(
            area_of(&b, LandId(5), PlotArea::Nearby { span: 1 }),
            [4, 5, 6].map(LandId).to_vec(),
            "飞弹那一档：当格 + 前后各一格 = 3 格"
        );
    }

    #[test]
    fn nearby_wraps_around_the_end_of_the_ring() {
        let b = ring(10);
        // 按棋盘声明顺序给结果（0、1 在 9 前面），不按下标排序
        assert_eq!(
            area_of(&b, LandId(0), PlotArea::Nearby { span: 1 }),
            [0, 1, 9].map(LandId).to_vec(),
            "环是闭环：0 号的前一格是 9 号"
        );
        assert_eq!(
            area_of(&b, LandId(0), PlotArea::Nearby { span: 3 }),
            [0, 1, 2, 3, 7, 8, 9].map(LandId).to_vec(),
            "原子弹那一档：前后各三格 = 7 格"
        );
    }

    #[test]
    fn nearby_counts_road_steps_not_squares() {
        // 同一条环、同样的网格坐标，只有下标不同：这条把「环上一步」与
        // 「网格上九格远」这两件事分开（0 号与 9 号在网格上相距 9，在环上只隔一步）。
        let b = ring(10);
        assert_eq!(
            area_of(&b, LandId(9), PlotArea::Nearby { span: 1 }),
            [0, 8, 9].map(LandId).to_vec()
        );
    }

    #[test]
    fn nearby_zero_degenerates_to_one() {
        let b = ring(10);
        assert_eq!(
            area_of(&b, LandId(5), PlotArea::Nearby { span: 0 }),
            area_of(&b, LandId(5), PlotArea::One),
            "0 步就是当格（定时炸弹那一档）"
        );
    }

    #[test]
    fn a_two_by_two_lot_is_measured_by_both_of_its_road_cells() {
        // 2×2 的商业地绑两格路。从任何一格的「当格」范围都该命中它 —— 只认绑定的第一格，
        // 就会让站在它另一边的那颗炸弹凭空炸空。
        let mut b = ring(6);
        b.lands[0].size = 2;
        b.tiles[1].land_id = Some(0);
        assert_eq!(lands_around_tile(&b, TileIndex(0), 0), vec![LandId(0)]);
        assert_eq!(
            lands_around_tile(&b, TileIndex(1), 0),
            vec![LandId(0)],
            "第二格也是它的临街面"
        );
    }

    #[test]
    fn street_follows_the_group_and_a_missing_group_does_not_widen_the_blast() {
        let mut b = ring(10);
        b.lands[5].group = Some("另一条".into());
        assert_eq!(
            area_of(&b, LandId(4), PlotArea::Street),
            (0..10).filter(|i| *i != 5).map(LandId).collect::<Vec<_>>(),
            "「整路段」= 同一组的地块，与前后无关"
        );
        b.lands[4].group = None;
        assert_eq!(
            area_of(&b, LandId(4), PlotArea::Street),
            vec![LandId(4)],
            "中心没有路段键时只打自己 —— 缺配置不该把全图算成一条街"
        );
    }

    #[test]
    fn whole_hits_every_land_declared() {
        let b = ring(10);
        assert_eq!(area_of(&b, LandId(3), PlotArea::Whole).len(), 10);
    }

    #[test]
    fn a_missing_center_selects_nothing() {
        let b = ring(10);
        for area in [
            PlotArea::One,
            PlotArea::Nearby { span: 9 },
            PlotArea::Street,
            PlotArea::Whole,
        ] {
            assert!(
                area_of(&b, LandId(99), area).is_empty(),
                "棋盘里没有这块地还照着 {area:?} 圈一遍 = 凭空毁一盘整盘"
            );
        }
    }

    /// P5 的炸弹爆心是**道路格**，不是地块。
    #[test]
    fn a_tile_centered_blast_hits_the_lands_bound_to_the_neighbouring_tiles() {
        const PROTOTYPE: &str = include_str!("../../../assets/data/boards/prototype.json");
        let b = BoardDef::from_json(PROTOTYPE).expect("prototype.json 应能解析");
        // 1 号格绑南京路（地块 0）；2 号格绑汉口路（地块 1）
        assert_eq!(
            lands_around_tile(&b, TileIndex(1), 0),
            vec![LandId(0)],
            "定时炸弹那一档：只打当格那块地"
        );
        assert_eq!(
            lands_around_tile(&b, TileIndex(1), 1),
            vec![LandId(0), LandId(1)],
            "飞弹那一档：前后各一格，汉口路也进来"
        );
        // 外滩（2×2）绑 6、7 两格；从 6 号格起算 0 步就该命中
        assert_eq!(lands_around_tile(&b, TileIndex(6), 0), vec![LandId(2)]);
        assert!(
            lands_around_tile(&b, TileIndex(99), 3).is_empty(),
            "越界格不该凭空圈出一片地"
        );
    }
}
