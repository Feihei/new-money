//! 「哪些地块算一片」——范围解析（P3c 从 [`crate::damage`] 里独立出来）。
//!
//! 原来这个枚举叫 `DamageArea`，住在损害模块里，因为只有拆房要选范围。P3c 的涨价卡与
//! 查封卡要的是**同一个东西**（PLAN §三「抵消对」那行写的「整街 5 天」），而它们跟损害
//! 没有半点关系 —— 继续留在 `damage` 里只有两条路可走：要么卡去 `use damage::…`
//! （读起来像「查封是一种拆房」），要么复制一份范围解析（第二份真相来源，§4.2 明令禁止）。
//! 所以它搬到这里，改名 `PlotArea`：**范围是地块的拓扑性质，不是一种动词**。
//!
//! 棋盘是网格路网（ADR-001），所以「几×几」按网格
//! 距离算，不是世界坐标半径。全部是 `BoardDef` 上的**纯函数**：不碰实体、不查组件，
//! 因此「3×3 在角落该是 2×2」这类裁决只在这里说一次，损害与卡各自拿同一份结果。

use crate::board::{BoardDef, LandDef, TileIndex};
use crate::land::LandId;
use serde::Deserialize;

/// 一片地块的范围。
///
/// 带 `Deserialize` 是因为**卡的数据要能写它**（涨价卡的「整街」）：范围是卡与损害共用的
/// 概念，那就只能有一个类型，而不是卡那侧再抄一份字符串自己解析。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlotArea {
    /// 只打中心这一块。
    One,
    /// 以中心为正中、边长为 `side` 的正方形（含中心）。`Box(3)` = 周围一圈，
    /// `Box(9)` = 核子飞弹那一档。按切比雪夫距离算，因此偶数边长与「向下取奇」同值
    /// （`Box(4)` 的半径就是 1，与 `Box(3)` 相同）—— 数据里写偶数不是错误，只是没有意义。
    Box { side: u32 },
    /// 整条路段：与中心同 `group` 的全部地块（「整路段夷为平地」「一排房屋消失」，
    /// 也是涨价卡「整街加倍」的那条街）。
    /// 中心那块没有 `group` 时只剩它自己 —— 缺配置不该扩大破坏面。
    Street,
    /// 全图。
    Whole,
}

/// 这块地在不在 `area` 里。`hub` 是中心地块的定义。
pub fn is_in_area(hub: &LandDef, other: &LandDef, area: PlotArea) -> bool {
    match area {
        PlotArea::One => hub.id == other.id,
        PlotArea::Street => match &hub.group {
            None => hub.id == other.id,
            Some(group) => other.group.as_deref() == Some(group.as_str()),
        },
        PlotArea::Box { side } => {
            let radius = side.saturating_sub(1) / 2;
            rect_gap(hub, other) <= radius
        }
        PlotArea::Whole => true,
    }
}

/// 以**道路格**为爆心的范围解析（P5 的定时炸弹）。
///
/// 为什么不能直接走 [`area_of`]：那条路线的中心是一块**地**，而炸弹的爆心是一格**路**。
/// 3×3 是网格距离，不是「这块地周围一圈地块」—— `Books::damage` 仍只吃地块集合，
/// 所以这里把「以格为中心命中哪些地块」算完再交给它（ADR-002 §8 明确要求这一句不要
/// 在炸弹模块里另写一份）。
///
/// 地块按**足迹**算间隙：size=2 的商业地占两格，贴着爆心边缘就该算命中。
pub fn lands_around_tile(board: &BoardDef, tile: TileIndex, side: u32) -> Vec<LandId> {
    // 用下标而不是 `tile()`：`tile()` 会回绕，于是 `TileIndex(99)` 会悄悄变成环上某格，
    // 「越界格什么都没打到」这条就用一条假中心炸出一片地
    let Some(center) = board.tiles.get(tile.0 as usize) else {
        return Vec::new();
    };
    let radius = side.saturating_sub(1) / 2;
    board
        .lands
        .iter()
        .filter(|land| point_gap(center.col, center.row, land) <= radius)
        .map(|land| LandId(land.id))
        .collect()
}

/// 一个网格点到一块地足迹的切比雪夫间隙（落在足迹上或内部 = 0）。
fn point_gap(col: u32, row: u32, land: &LandDef) -> u32 {
    let (col_hi, row_hi) = footprint_max(land);
    let gap_on =
        |p: u32, lo: u32, hi: u32| -> u32 { if p < lo { lo - p } else { p.saturating_sub(hi) } };
    gap_on(col, land.col, col_hi).max(gap_on(row, land.row, row_hi))
}

/// 范围解析：中心 + 范围 → 哪些地块被圈进来（按棋盘声明顺序，稳定可断言）。
///
/// 走 `BoardDef` 而不是查实体：范围是**静态拓扑**事实（网格坐标、路段归属都在定义里），
/// 复制一份「哪些地属于同一条街」到运行时就是第二份真相来源（§4.2 的 B2a 修正）。
/// 棋盘里没有中心那块地时返回空 —— 数据错误该表现为「什么都没打着」，而不是炸掉一盘整盘。
pub fn area_of(board: &BoardDef, center: LandId, area: PlotArea) -> Vec<LandId> {
    let Some(hub) = board.land(center.0) else {
        return Vec::new();
    };
    board
        .lands
        .iter()
        .filter(|l| is_in_area(hub, l, area))
        .map(|l| LandId(l.id))
        .collect()
}

/// 两个地块**足迹**之间的切比雪夫间隙（重叠 = 0，不相交 = 行列间隙的较大者）。
///
/// 足迹是 `col..col+size`：商业用地 size=2 占两格，所以它与「旁边」那块地的间隙
/// 是 0 而不是 1 —— 不按足迹算就会让 3×3 在商业地产边上少拆一排，那种错只在画面上看得见。
fn rect_gap(a: &LandDef, b: &LandDef) -> u32 {
    // 一根轴上的区间间隙：重叠时为 0，两个 `saturating_sub` 里只有一个能非零
    let gap_on = |a_lo: u32, a_hi: u32, b_lo: u32, b_hi: u32| -> u32 {
        b_lo.saturating_sub(a_hi).max(a_lo.saturating_sub(b_hi))
    };
    let (a_col_hi, a_row_hi) = footprint_max(a);
    let (b_col_hi, b_row_hi) = footprint_max(b);
    gap_on(a.col, a_col_hi, b.col, b_col_hi).max(gap_on(a.row, a_row_hi, b.row, b_row_hi))
}

/// 足迹的右下界（含）。`size` 至少为 1，写 0 时按 1 理解而不是回到左上角。
fn footprint_max(land: &LandDef) -> (u32, u32) {
    let span = u32::from(land.size.max(1)).saturating_sub(1);
    (land.col + span, land.row + span)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::{LandType, TileDef};

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

    fn board(lands: Vec<LandDef>) -> BoardDef {
        BoardDef {
            board_name: "测试盘".into(),
            cell_size: 2.0,
            grid_cols: 8,
            grid_rows: 8,
            tiles: Vec::new(),
            lands,
            facility_fees: Default::default(),
        }
    }

    /// 网格长这样（编号 = `row * 5 + col`。用 5×5 而不是 3×3：3×3 盘上「正中一圈」
    /// 恰好等于全图，那种测试分不清 `Box(3)` 与 `Whole` 是不是真写对了）：
    /// ```text
    ///  0  1  2  3  4
    ///  5  6  7  8  9
    /// 10 11 12 13 14
    /// 15 16 17 18 19
    /// 20 21 22 23 24
    /// ```
    /// 第一行共用「上层」路段，其余每行一条自己的路段，所以 `Street` 最多命中一排。
    fn grid() -> BoardDef {
        board(
            (0..25u32)
                .map(|i| {
                    let row = i / 5;
                    let group = if row == 0 {
                        "上层".to_string()
                    } else {
                        format!("街{row}")
                    };
                    plot(i, i % 5, row, 1, Some(group))
                })
                .collect(),
        )
    }

    #[test]
    fn one_hits_only_the_center() {
        let b = grid();
        assert_eq!(
            area_of(&b, LandId(12), PlotArea::One),
            vec![LandId(12)],
            "只打中心"
        );
    }

    #[test]
    fn box_three_is_the_eight_neighbours_plus_the_center() {
        let b = grid();
        let want: Vec<LandId> = [6, 7, 8, 11, 12, 13, 16, 17, 18]
            .into_iter()
            .map(LandId)
            .collect();
        assert_eq!(
            area_of(&b, LandId(12), PlotArea::Box { side: 3 }),
            want,
            "3×3 该是正中九块，且按棋盘声明顺序给出（断言与将来的文案都要稳定）"
        );
    }

    #[test]
    fn box_three_at_a_corner_is_a_smaller_box_not_a_wrapped_one() {
        // 网格边缘不外绕：角上的 3×3 就是 2×2。外绕会让「在角落放一颗飞弹」
        // 比在正中威力更大，那与任何一份原文都不符
        let b = grid();
        let want: Vec<LandId> = [0, 1, 5, 6].into_iter().map(LandId).collect();
        assert_eq!(area_of(&b, LandId(0), PlotArea::Box { side: 3 }), want);
    }

    #[test]
    fn the_box_side_is_a_radius_and_degenerates_instead_of_erroring() {
        let b = grid();
        let c = LandId(12);
        // 半径 = (side-1)/2：4 与 3 同值，5 才多出一圈
        assert_eq!(
            area_of(&b, c, PlotArea::Box { side: 4 }),
            area_of(&b, c, PlotArea::Box { side: 3 }),
            "偶数边长不是错误，只是与「向下取奇」同值"
        );
        assert_eq!(
            area_of(&b, c, PlotArea::Box { side: 5 }).len(),
            25,
            "5×5 盘上的 5×5 就是全图"
        );
        assert_eq!(
            area_of(&b, c, PlotArea::Box { side: 1 }),
            area_of(&b, c, PlotArea::One),
            "1×1 退化成单块"
        );
        assert_eq!(
            area_of(&b, c, PlotArea::Box { side: 0 }),
            area_of(&b, c, PlotArea::One),
            "0 也退化成正中一块，而不是凭空炸出一个空集"
        );
    }

    #[test]
    fn a_two_by_two_lot_is_measured_by_its_footprint() {
        // 2×2 的商业地占 `col..col+1`，所以它与 col=2 那块只隔 1、与 col=3 那块隔 2。
        // 按左上角算间隙会让 3×3 在商业地产边上少拆一排 —— 那种错只在画面上看得出来
        let b = board(vec![
            plot(0, 0, 0, 2, None),
            plot(1, 2, 0, 1, None),
            plot(2, 3, 0, 1, None),
        ]);
        assert_eq!(
            area_of(&b, LandId(0), PlotArea::Box { side: 3 }),
            [0, 1].map(LandId).to_vec()
        );
    }

    #[test]
    fn street_follows_the_group_and_a_missing_group_does_not_widen_the_blast() {
        let b = grid();
        assert_eq!(
            area_of(&b, LandId(12), PlotArea::Street),
            [10, 11, 12, 13, 14].map(LandId).to_vec(),
            "「整路段」= 同一路段那一排，涨价卡的「整街」也是这一排"
        );
        let lone = board(vec![
            plot(0, 0, 0, 1, None),
            plot(1, 1, 0, 1, Some("街".into())),
        ]);
        assert_eq!(
            area_of(&lone, LandId(0), PlotArea::Street),
            vec![LandId(0)],
            "中心没有路段键时只打自己 —— 缺配置不该把全图算成一条街"
        );
    }

    #[test]
    fn whole_hits_every_land_declared() {
        let b = grid();
        assert_eq!(area_of(&b, LandId(12), PlotArea::Whole).len(), 25);
    }

    #[test]
    fn a_missing_center_selects_nothing() {
        let b = grid();
        for area in [
            PlotArea::One,
            PlotArea::Box { side: 9 },
            PlotArea::Street,
            PlotArea::Whole,
        ] {
            assert!(
                area_of(&b, LandId(99), area).is_empty(),
                "棋盘里没有这块地还照着 {area:?} 圈一遍 = 凭空毁一盘整盘"
            );
        }
    }

    /// P5 的炸弹爆心是**道路格**，不是地块。prototype 的坐标见 `board.rs`：
    /// 1 号南京路 (2,1) 挨着地块 0 的 (2,0)；10 号点券箱 (1,3) 离最近的地块也有 1 格。
    #[test]
    fn a_tile_centered_box_hits_the_plots_whose_footprints_are_near() {
        const PROTOTYPE: &str = include_str!("../../../assets/data/boards/prototype.json");
        let b = BoardDef::from_json(PROTOTYPE).expect("prototype.json 应能解析");
        // 在 1 号格（南京路）炸：3×3 覆盖 col 1..=3 / row 0..=2，
        // 命中地块 0 南京路 (2,0)、1 汉口路 (3,0)、2 外滩 (6,2) 不在
        assert_eq!(
            lands_around_tile(&b, TileIndex(1), 3),
            vec![LandId(0), LandId(1)]
        );
        // 9×9（半径 4）才够到外滩：爆心 (2,1) 到足迹 (6..=7, 2..=3) 的列间隙是 4
        assert!(
            lands_around_tile(&b, TileIndex(1), 9).contains(&LandId(2)),
            "外滩商业区的足迹应当被更宽的范围命中"
        );
        assert!(
            lands_around_tile(&b, TileIndex(99), 3).is_empty(),
            "越界格不该凭空圈出一片地"
        );
    }

    /// size=2 的地按足迹算：路格贴在它第二格旁边也算命中。
    #[test]
    fn a_two_by_two_lot_is_hit_by_a_tile_next_to_its_second_cell() {
        let mut b = board(vec![plot(0, 2, 0, 2, None)]);
        b.tiles = vec![
            TileDef {
                id: 0,
                col: 4,
                row: 0,
                kind: crate::board::TileKind::Road,
                name: "旁路".into(),
                land_id: None,
                branch: None,
                data: Default::default(),
            },
            TileDef {
                id: 1,
                col: 4,
                row: 2,
                kind: crate::board::TileKind::Road,
                name: "下角".into(),
                land_id: None,
                branch: None,
                data: Default::default(),
            },
        ];
        // 网格点 (4,0) 与足迹 (2..=3, 0..=1) 的列间隙是 1；半径 1 的 3×3 该命中
        assert_eq!(lands_around_tile(&b, TileIndex(0), 3), vec![LandId(0)]);
        // (4,2) 行间隙也是 1 → 对角仍命中
        assert_eq!(lands_around_tile(&b, TileIndex(1), 3), vec![LandId(0)]);
    }
}
