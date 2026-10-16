//! 建筑损害阶梯（PLAN §三 那张表，P2② 的「拆房 / 征收」一族动词）。
//!
//! 你回忆的那套东西不是四条零散效果，是**三个维度**：深度 × 归属 × 范围。
//! 这里把前两个维度做成**类型上就写不出第四种**的形状：
//!
//! ```text
//! LandDamage::Degrade { levels }   降 N 层 —— 结构上没有 `seize` 这个开关
//! LandDamage::Clear   { seize }    归零 —— 要不要连地一起收，是这一支唯一的位
//! × DamageArea::One | Box{side} | Street | Whole
//! ```
//!
//! 于是「降层还夺地」那个不该存在的格子**压根表达不出来**（地都收了还留层做什么）。
//! 三个合法组合各有硬出处：`Degrade`（飞弹 3×3 降一级、恶魔所经降一级）、
//! `Clear{seize:false}`（怪兽卡一栋、恶魔卡整路段、工程车所经之地）、
//! `Clear{seize:true}`（命运#4 强制征收一栋、核子飞弹 9×9～全图）。
//!
//! **为什么值得单独一份**：P3 的怪兽卡与恶魔卡、P5 的飞弹／核子飞弹／定时炸弹、
//! P10 的工程车、P11 的外星人与山洪，全都在「选一个组合 + 选一个范围」。
//! 各阶段各写一遍就会各漏一处 —— 而漏的那一处永远是「夺地忘了清抵押」这种静默错账。
//!
//! 本模块只有**词汇与算术**，不碰实体：面积解析是 `BoardDef` 上的纯函数，
//! 单笔算术是 [`strike`]，真正写组件的那一句在 [`crate::settle::Books::damage`]。

use crate::board::{BoardDef, LandDef};
use crate::land::LandId;

/// 深度与归属。两支互斥，且只有 `Clear` 那支能动所有权。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandDamage {
    /// 降 `levels` 层，**永不动所有权**。0 级空地不会被降成负数（`saturating_sub`）。
    /// 出处：飞弹「威力 3×3、房屋下降一级」；恶魔附身「所经降一级（空地不拆）」。
    Degrade { levels: u8 },
    /// 建筑归零。`seize = true` 时土地也收回（`Owner` 归 `None`、抵押一并摘掉）。
    /// `false` = 只拆房子，地还是他的 —— 怪兽卡那一档。
    Clear { seize: bool },
}

impl LandDamage {
    /// 这次要不要动所有权。管线之外只有测试与调试面板会读它。
    pub fn seizes(&self) -> bool {
        matches!(self, Self::Clear { seize: true })
    }

    /// 夺地时要不要**顺带清掉这条格上的临时状态**（P5 的路障/地雷/定时炸弹、
    /// P6 的神明附身、点券箱）。PLAN §七 的拍板：**只有 `Box(3)` 及以上才清**。
    ///
    /// 理由不是「核子飞弹原文说神与物品全部消失」，而是那句话更可能是**范围**带来的、
    /// 不是**夺地**带来的。若单块夺地也清，命运#4「强制征收」就变成一次隐性清场 ——
    /// 玩家身上的神没了、路上的路障没了，而他只看见「一块地变成了无主」，
    /// 那是最难查的一类投诉。
    ///
    /// 今天还没有 occupancy 与 possession（P5/P6），这一句先把裁决写在这儿，
    /// 那两条落地时读它，而不是各自重新决定一次。
    pub fn sweeps_tile_state(&self, area: DamageArea) -> bool {
        let wide = match area {
            DamageArea::One | DamageArea::Street => false,
            DamageArea::Box { side } => side >= 3,
            DamageArea::Whole => true,
        };
        self.seizes() && wide
    }
}

/// 范围。棋盘是网格路网（ADR-001），所以「几×几」用网格距离算，不是世界坐标半径。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageArea {
    /// 只打中心这一块。
    One,
    /// 以中心为正中、边长为 `side` 的正方形（含中心）。`Box(3)` = 周围一圈，
    /// `Box(9)` = 核子飞弹那一档。按切比雪夫距离算，因此偶数边长与「向下取奇」同值
    /// （`Box(4)` 的半径就是 1，与 `Box(3)` 相同）—— 数据里写偶数不是错误，只是没有意义。
    Box { side: u32 },
    /// 整条路段：与中心同 `group` 的全部地块（「整路段夷为平地」「一排房屋消失」）。
    /// 中心那块没有 `group` 时只剩它自己 —— 缺配置不该扩大破坏面。
    Street,
    /// 全图。
    Whole,
}

/// 一次损害的产出。给日志、调试面板与将来的新闻文案用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DamageTally {
    /// 被扫到的地块数（含本来就是空地的那些 —— 范围是范围，不为「没打到东西」缩小）。
    pub hit: u32,
    /// 其中所有权被收走的块数。
    pub seized: u32,
}

/// 这块地在不在 `area` 里。`hub` 是中心地块的定义。
pub fn is_in_area(hub: &LandDef, other: &LandDef, area: DamageArea) -> bool {
    match area {
        DamageArea::One => hub.id == other.id,
        DamageArea::Street => match &hub.group {
            None => hub.id == other.id,
            Some(group) => other.group.as_deref() == Some(group.as_str()),
        },
        DamageArea::Box { side } => {
            let radius = side.saturating_sub(1) / 2;
            rect_gap(hub, other) <= radius
        }
        DamageArea::Whole => true,
    }
}

/// 面积解析：中心 + 范围 → 哪些地块挨打（按棋盘声明顺序，稳定可断言）。
///
/// 走 `BoardDef` 而不是查实体：范围是**静态拓扑**事实（网格坐标、路段归属都在定义里），
/// 复制一份「哪些地属于同一条街」到运行时就是第二份真相来源（§4.2 的 B2a 修正）。
/// 棋盘里没有中心那块地时返回空 —— 数据错误该表现为「什么都没拆」，而不是炸掉一盘整盘。
pub fn area_of(board: &BoardDef, center: LandId, area: DamageArea) -> Vec<LandId> {
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

/// 对单块地打一拳：把深度与归属落到「等级」与「业主」这两个数上。
///
/// 刻意写成对裸数据操作，而不是 `&mut QueryItem`：这样 0 级空地、`seize` 两种分支
/// 都能在单测里穷举，而 `Books::damage` 只负责找到实体、调用它、顺手摘抵押。
pub fn strike(level: &mut u8, owner: &mut Option<crate::player::PlayerId>, damage: LandDamage) {
    match damage {
        LandDamage::Degrade { levels } => *level = level.saturating_sub(levels),
        LandDamage::Clear { seize } => {
            *level = 0;
            if seize {
                *owner = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::PlayerId;

    /// 造一块地：`(编号, 列, 行, 占边长, 路段)`
    fn plot(id: u32, col: u32, row: u32, size: u8, group: Option<String>) -> LandDef {
        LandDef {
            id,
            col,
            row,
            size,
            name: format!("地{id}"),
            group,
            land_type: crate::board::LandType::Residential,
            price: 1_500,
            rent: vec![10, 50, 150, 450, 1_200, 3_000],
            upgrade_cost: 1_000,
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
    fn the_fourth_cell_is_unrepresentable() {
        // 「降层 + 夺地」这个组合压根写不出来：`Degrade` 那一支没有 `seize` 这个字段。
        // 这条测试断言的是**类型形状**，所以它只会因「有人给 Degrade 加了个开关」而红。
        let degrade = LandDamage::Degrade { levels: 1 };
        assert!(!degrade.seizes(), "降层那支不该能动所有权");
        assert!(LandDamage::Clear { seize: true }.seizes());
        assert!(!LandDamage::Clear { seize: false }.seizes());
    }

    #[test]
    fn degrade_never_touches_the_owner() {
        let mut level = 3;
        let mut owner = Some(PlayerId(1));
        strike(&mut level, &mut owner, LandDamage::Degrade { levels: 2 });
        assert_eq!(
            (level, owner),
            (1, Some(PlayerId(1))),
            "降两级：房子少了，地还是他的"
        );
    }

    #[test]
    fn degradation_stops_at_empty_ground() {
        // 空地不拆（恶魔卡那条原文就这么写的）：降到底也不会穿成负数
        let mut level = 1;
        let mut owner = Some(PlayerId(2));
        strike(&mut level, &mut owner, LandDamage::Degrade { levels: 9 });
        assert_eq!((level, owner), (0, Some(PlayerId(2))));
    }

    #[test]
    fn clearing_levels_the_building_and_maybe_the_deed() {
        let mut level = 5;
        let mut owner = Some(PlayerId(0));
        strike(&mut level, &mut owner, LandDamage::Clear { seize: false });
        assert_eq!((level, owner), (0, Some(PlayerId(0))), "怪兽卡：只拆房子");

        strike(&mut level, &mut owner, LandDamage::Clear { seize: true });
        // 已经是空地也必须再收一次地：`seize` 与「有没有房子可拆」无关
        assert_eq!((level, owner), (0, None), "强制征收：房屋同灭、土地收回");
    }

    #[test]
    fn one_hits_only_the_center() {
        let b = grid();
        assert_eq!(
            area_of(&b, LandId(12), DamageArea::One),
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
            area_of(&b, LandId(12), DamageArea::Box { side: 3 }),
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
        assert_eq!(area_of(&b, LandId(0), DamageArea::Box { side: 3 }), want);
    }

    #[test]
    fn the_box_side_is_a_radius_and_degenerates_instead_of_erroring() {
        let b = grid();
        let c = LandId(12);
        // 半径 = (side-1)/2：4 与 3 同值，5 才多出一圈
        assert_eq!(
            area_of(&b, c, DamageArea::Box { side: 4 }),
            area_of(&b, c, DamageArea::Box { side: 3 }),
            "偶数边长不是错误，只是与「向下取奇」同值"
        );
        assert_eq!(
            area_of(&b, c, DamageArea::Box { side: 5 }).len(),
            25,
            "5×5 盘上的 5×5 就是全图"
        );
        assert_eq!(
            area_of(&b, c, DamageArea::Box { side: 1 }),
            area_of(&b, c, DamageArea::One),
            "1×1 退化成单块"
        );
        assert_eq!(
            area_of(&b, c, DamageArea::Box { side: 0 }),
            area_of(&b, c, DamageArea::One),
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
            area_of(&b, LandId(0), DamageArea::Box { side: 3 }),
            [0, 1].map(LandId).to_vec()
        );
    }

    #[test]
    fn street_follows_the_group_and_a_missing_group_does_not_widen_the_blast() {
        let b = grid();
        assert_eq!(
            area_of(&b, LandId(12), DamageArea::Street),
            [10, 11, 12, 13, 14].map(LandId).to_vec(),
            "「整路段夷为平地」= 同一路段那一排"
        );
        let lone = board(vec![
            plot(0, 0, 0, 1, None),
            plot(1, 1, 0, 1, Some("街".into())),
        ]);
        assert_eq!(
            area_of(&lone, LandId(0), DamageArea::Street),
            vec![LandId(0)],
            "中心没有路段键时只打自己 —— 缺配置不该把全图算成一条街"
        );
    }

    #[test]
    fn whole_hits_every_land_declared() {
        let b = grid();
        assert_eq!(area_of(&b, LandId(12), DamageArea::Whole).len(), 25);
    }

    #[test]
    fn a_missing_center_damages_nothing() {
        let b = grid();
        for area in [
            DamageArea::One,
            DamageArea::Box { side: 9 },
            DamageArea::Street,
            DamageArea::Whole,
        ] {
            assert!(
                area_of(&b, LandId(99), area).is_empty(),
                "棋盘里没有这块地还照着 {area:?} 拆一遍 = 凭空毁一盘整盘"
            );
        }
    }

    #[test]
    fn only_a_wide_seize_sweeps_the_tile_state() {
        // PLAN §七 那条拍板：夺地 + `Box(3)` 及以上才顺带清 occupancy/附身
        let seize = LandDamage::Clear { seize: true };
        let keep = LandDamage::Clear { seize: false };
        let drop = LandDamage::Degrade { levels: 1 };
        for area in [DamageArea::One, DamageArea::Street] {
            assert!(
                !seize.sweeps_tile_state(area),
                "单块夺地不该隐性清场：{area:?}"
            );
        }
        for area in [DamageArea::Box { side: 3 }, DamageArea::Whole] {
            assert!(seize.sweeps_tile_state(area), "{area:?} 这一档要清");
            assert!(!keep.sweeps_tile_state(area), "不夺地就什么都不清");
            assert!(!drop.sweeps_tile_state(area), "降层不动归属，更不清状态");
        }
        assert!(
            !seize.sweeps_tile_state(DamageArea::Box { side: 1 }),
            "1×1 就是单块，跟 One 同判"
        );
    }
}
