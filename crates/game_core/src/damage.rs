//! 建筑损害阶梯（PLAN §三 那张表，P2② 的「拆房 / 征收」一族动词）。
//!
//! 你回忆的那套东西不是四条零散效果，是**三个维度**：深度 × 归属 × 范围。
//! 这里把前两个维度做成**类型上就写不出第四种**的形状：
//!
//! ```text
//! LandDamage::Degrade { levels }   降 N 层 —— 结构上没有 `seize` 这个开关
//! LandDamage::Clear   { seize }    归零 —— 要不要连地一起收，是这一支唯一的位
//! × PlotArea::One | Box{side} | Street | Whole   （范围在 [`crate::plot`]，P3c 起卡与损害共用）
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
//! 本模块只有**词汇与算术**，不碰实体：面积解析在 [`crate::plot`]，是 `BoardDef` 上的纯函数，
//! 单笔算术是 [`strike`]，真正写组件的那一句在 [`crate::settle::Books::damage`]。

use serde::Deserialize;

use crate::plot::PlotArea;

/// 深度与归属。两支互斥，且只有 `Clear` 那支能动所有权。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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
    pub fn sweeps_tile_state(&self, area: PlotArea) -> bool {
        let wide = match area {
            PlotArea::One | PlotArea::Street => false,
            PlotArea::Box { side } => side >= 3,
            PlotArea::Whole => true,
        };
        self.seizes() && wide
    }
}

/// 一次损害的产出。给日志、调试面板与将来的新闻文案用。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DamageTally {
    /// 被扫到的地块数（含本来就是空地的那些 —— 范围是范围，不为「没打到东西」缩小）。
    pub hit: u32,
    /// 其中所有权被收走的块数。
    pub seized: u32,
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
    fn only_a_wide_seize_sweeps_the_tile_state() {
        // PLAN §七 那条拍板：夺地 + `Box(3)` 及以上才顺带清 occupancy/附身
        let seize = LandDamage::Clear { seize: true };
        let keep = LandDamage::Clear { seize: false };
        let drop = LandDamage::Degrade { levels: 1 };
        for area in [PlotArea::One, PlotArea::Street] {
            assert!(
                !seize.sweeps_tile_state(area),
                "单块夺地不该隐性清场：{area:?}"
            );
        }
        for area in [PlotArea::Box { side: 3 }, PlotArea::Whole] {
            assert!(seize.sweeps_tile_state(area), "{area:?} 这一档要清");
            assert!(!keep.sweeps_tile_state(area), "不夺地就什么都不清");
            assert!(!drop.sweeps_tile_state(area), "降层不动归属，更不清状态");
        }
        assert!(
            !seize.sweeps_tile_state(PlotArea::Box { side: 1 }),
            "1×1 就是单块，跟 One 同判"
        );
    }
}
