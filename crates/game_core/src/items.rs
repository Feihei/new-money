//! 道具栏（P5）：按 kind 计数堆叠的实物道具，与 `Hand`（卡）是两样东西。
//!
//! 为什么分成两个容器而不是把道具也做成卡：
//! - **卡有身份，道具只有数量**。手里两张陷害卡是两条可分别打出的记录（`VecDeque<CardId>`），
//!   而两个路障在规则上完全一样 —— 存成两条只会让「用掉哪一个」成为一道无意义的选择。
//! - **来源与去处不同**。卡来自百货、按张买卖；道具来自开局配送、研究所与宝箱，
//!   用掉就少一个。把它们塞进同一个 `Hand`，上限那条「满了挤掉最前面那张」就会
//!   开始作用于路障，而 4 代对这两条库存的规则本来就不一样。
//!
//! P5 只落四种：路障、地雷、定时炸弹、遥控骰子。机器娃娃、机器工人、飞弹这些
//! 要等它们各自的效果（P10 研究所、P11 新闻）落地时才进来 —— 空变体挂在这里
//! 只会让面板长出一排按不动的按钮。

use std::collections::BTreeMap;

use bevy::prelude::*;

use crate::occupancy::OccupancyKind;

/// 一种可堆叠的实物道具。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemKind {
    Roadblock,
    Mine,
    TimeBomb,
    RemoteDice,
}

impl ItemKind {
    /// 面板按这个顺序排（枚举顺序 = 使用频率的直觉顺序，不是字母序）。
    pub const ALL: [Self; 4] = [
        Self::Roadblock,
        Self::Mine,
        Self::TimeBomb,
        Self::RemoteDice,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Roadblock => "路障",
            Self::Mine => "地雷",
            Self::TimeBomb => "定时炸弹",
            Self::RemoteDice => "遥控骰子",
        }
    }

    /// 这句道具「放到路上」变成哪一种 occupancy。`None` = 它不是放置物
    /// （遥控骰子改的是这一回合走几步）。
    pub const fn occupancy(self) -> Option<OccupancyKind> {
        match self {
            Self::Roadblock => Some(OccupancyKind::Roadblock),
            Self::Mine => Some(OccupancyKind::Mine),
            Self::TimeBomb => Some(OccupancyKind::TimeBomb),
            Self::RemoteDice => None,
        }
    }

    /// 研究所售价（PLAN §三「研究所独占 5 种」）。今天没有商店读它，
    /// 但它是道具的静态属性，与卡面的 `cost_points` 同一层；P10 装配研究所时直接用，
    /// 不必到时候再往数据里补一列。
    pub const fn price_points(self) -> u32 {
        match self {
            Self::Roadblock => 30,
            Self::Mine => 40,
            Self::TimeBomb => 50,
            Self::RemoteDice => 30,
        }
    }
}

/// 道具栏。存在即「这个人身上带着这些道具」，空栏也照样挂着 ——
/// 与 `Hand` 同一条理由：没有组件与「有但空」在面板上是两件事，`Option` 会把它们混成一件。
#[derive(Component, Debug, Default, Clone, PartialEq, Eq)]
pub struct Inventory {
    counts: BTreeMap<ItemKind, u32>,
}

impl Inventory {
    /// 开局配送。PLAN §三：机器娃娃 / 路障 / 机器工人开局即送 ——
    /// P5 只有路障有实现，所以先送一个；另两个等它们的效果落地时再加同一行。
    pub fn starting() -> Self {
        let mut inventory = Self::default();
        inventory.gain(ItemKind::Roadblock, 1);
        inventory
    }

    pub fn count(&self, kind: ItemKind) -> u32 {
        self.counts.get(&kind).copied().unwrap_or(0)
    }

    pub fn holds(&self, kind: ItemKind) -> bool {
        self.count(kind) > 0
    }

    pub fn is_empty(&self) -> bool {
        self.counts.values().all(|n| *n == 0)
    }

    /// 进 `n` 个，返回进完之后的持有量。`n == 0` 什么都不做（不是一条错误）——
    /// 那句早退是必需的：`entry().or_insert(0)` 会为一次「进 0 个」在表里留下一个
    /// 计数为 0 的槽，而面板的 `entries()` 就会吐出「路障 0」（与 [`Self::spend`]
    /// 扣到 0 就删 key 同一条纪律）。
    pub fn gain(&mut self, kind: ItemKind, n: u32) -> u32 {
        if n == 0 {
            return self.count(kind);
        }
        let slot = self.counts.entry(kind).or_insert(0);
        *slot = slot.saturating_add(n);
        *slot
    }

    /// 全部清空（P13「变卖所有道具」）。返回**变卖前**的那份清单 ——
    /// 价钱在调用方算（它读的是世界此刻的标价表），这里只负责「一次性见底」；
    /// 先取后清，两条写不完的中间态没有位置。
    pub fn clear_all(&mut self) -> Vec<(ItemKind, u32)> {
        std::mem::take(&mut self.counts).into_iter().collect()
    }

    /// 用掉 `n` 个。不够时**一个都不扣**并返回 `false`（与 `Books::spend_points` 同一条
    /// 「不许半途」的纪律）。扣到 0 的 key 直接删掉，面板的 `entries()` 才不会吐出「路障 0」。
    pub fn spend(&mut self, kind: ItemKind, n: u32) -> bool {
        let held = self.count(kind);
        if held < n {
            return false;
        }
        if held == n {
            self.counts.remove(&kind);
        } else {
            self.counts.insert(kind, held - n);
        }
        true
    }

    /// `(种类, 数量)`，按枚举顺序。面板据此重建，顺序因此是确定的。
    pub fn entries(&self) -> Vec<(ItemKind, u32)> {
        ItemKind::ALL
            .into_iter()
            .filter_map(|kind| {
                let n = self.count(kind);
                (n > 0).then_some((kind, n))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_inventory_says_so_and_a_starting_one_has_a_roadblock() {
        assert!(Inventory::default().is_empty());
        let start = Inventory::starting();
        assert_eq!(start.count(ItemKind::Roadblock), 1);
        assert!(!start.is_empty());
        assert_eq!(
            start.entries(),
            vec![(ItemKind::Roadblock, 1)],
            "空种类不该出现在面板的清单里"
        );
    }

    #[test]
    fn counts_stack_and_spending_takes_exactly_what_was_asked() {
        let mut inv = Inventory::default();
        assert_eq!(inv.gain(ItemKind::Mine, 2), 2);
        assert_eq!(inv.gain(ItemKind::Mine, 3), 5, "按 kind 堆叠");
        assert!(inv.spend(ItemKind::Mine, 2));
        assert_eq!(inv.count(ItemKind::Mine), 3);
        assert!(
            !inv.spend(ItemKind::Mine, 4),
            "不够时一个都不扣，而不是扣到 0"
        );
        assert_eq!(inv.count(ItemKind::Mine), 3);
        assert!(inv.spend(ItemKind::Mine, 3));
        assert!(!inv.holds(ItemKind::Mine));
        assert!(inv.entries().is_empty(), "扣空的种类要从清单里消失");
    }

    #[test]
    fn placement_items_name_the_occupancy_and_remote_dice_does_not() {
        assert_eq!(
            ItemKind::Roadblock.occupancy(),
            Some(OccupancyKind::Roadblock)
        );
        assert_eq!(ItemKind::Mine.occupancy(), Some(OccupancyKind::Mine));
        assert_eq!(
            ItemKind::TimeBomb.occupancy(),
            Some(OccupancyKind::TimeBomb)
        );
        assert_eq!(
            ItemKind::RemoteDice.occupancy(),
            None,
            "遥控骰子改的是这一回合走几步，不往路上放东西"
        );
        for kind in ItemKind::ALL {
            assert!(!kind.label().is_empty());
            assert!(kind.price_points() > 0, "{} 没有价", kind.label());
        }
    }
}
