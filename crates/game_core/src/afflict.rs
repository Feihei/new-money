//! 厄运（P3a）：会落到某个玩家头上的**事**，与「钱」是两类账。
//!
//! P2① 的 [`crate::charge`] 管的是「一笔钱在离开钱包之前要过的那条链」，它的输入输出都是
//! 金额。而「被关进监狱 5 天」「原地停留」「向后转」「房子被拆」不落金额 —— 但它们同样
//! 需要一个承受者、同样可能被抵消或转嫁，所以**形状必须与 `Charge` 一致**：
//!
//! ```text
//! Affliction { domain, victim, cancellable }   ↔   Charge { kind, payer/payee, cancellable, transferable }
//! Insulation::absorb(...)                      ↔   charge::stage(...) 里的 Adjustment::Cancel
//! ```
//!
//! 为什么抵消的**来源**是组件而不是 [`crate::charge::Corrections`] 那张表：表里的修正是
//! 「持续生效」（大穷神在这几天里把我所有支出翻倍），而免罪卡是**一次性**的（抵掉下一次
//! 就没了）。把一次性消耗塞进那张表，就会撞上「报价也调一次 `stage`」这个坑 ——
//! [`crate::settle::Books::quoted`] 只是给 UI 看个数，它不该花掉玩家手里的一张卡。
//! 护身挂在玩家身上，只有真正生效的那次结算才会去 `absorb` 一层。
//!
//! `cancellable` 这一位是给 P13 的免疫规则留的：魔法屋 12 条法术里只有坐牢与住院两条
//! 可被免罪抵消或嫁祸移出，其余 10 条「不受任何卡与神影响」。那是一条**按类别**的豁免，
//! 所以位挂在事上而不是挂在抵消方上 —— 与 P2 那条「这笔账免不免是账的性质」同一条理由。

use bevy::prelude::*;

/// 一类厄运。P13 的免疫矩阵按这一位逐格填「可抵消／不可抵消」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfflictDomain {
    /// 进拘留类状态：坐牢、住院、被绑架（P4 的 `Detention` 是它的落地）
    Detention,
    /// 原地停留 N 回合（停留卡、魔法屋「原地停留」）
    Stay,
    /// 接下来 N 个掷骰回合只走 1 格（乌龟卡，P5）。与 [`Self::Stay`] 分开：
    /// 一个让人不动，一个让人走不远。
    MoveCap,
    /// 行进方向被反转（向后转）
    Reversal,
    /// 建筑与地产受损（怪兽卡、飞弹、新闻天灾）—— 承受者是地块的业主
    Damage,
}

/// 一件要落到某人头上的事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affliction {
    pub domain: AfflictDomain,
    pub victim: Entity,
    /// 允不允许被整笔抵消。今天四类**都**允许，将来 P13 的免疫规则把某几类关掉 ——
    /// 与 [`crate::charge::Charge::cancellable`] 同一个道理：默认开着，关掉它的是免疫方。
    pub cancellable: bool,
}

impl Affliction {
    pub fn new(domain: AfflictDomain, victim: Entity) -> Self {
        Self {
            domain,
            victim,
            cancellable: true,
        }
    }
}

/// 护身：用**卡**抵消落到自己头上的厄运。存在即「至少还有一层」。
///
/// `charges` 是 `u32` 而不是 `Option<u32>`，是有意的收窄：`None` 那一格原本留给「神明附身
/// 那种不限次的免疫」（P3a 当时只有这一种用户，写在类型上像是提前铺路）。P6b② 落地时
/// 它变成一个**陷阱** —— 大福神的免疫与免罪卡会同时挂在一个人身上，而一个实体同一组件
/// 只能有一份：上身把 `Some(2)` 覆盖成 `None`、离场把整个组件摘掉，玩家手里那两层护身
/// 就这么无声消失了 —— 少花一张卡与多花一张卡一样难查。
///
/// 所以神明那一半**不在这个组件里**：它现读那位神是谁（[`crate::gods::resists`]），
/// 与 `Presence` 同一条纪律 —— 会过期的账不留副本。两条来源各只有一个写点，
/// 于是「谁吃掉谁的次数」这件事在结构上没有出现的机会。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insulation {
    pub charges: u32,
}

impl Insulation {
    /// 用一层去抵消 `affliction`。返回 `true` = 这件事没有发生。
    ///
    /// 不可抵消的那类**不吃掉护身**（否则一张免罪卡会被「向后转」白白耗掉，
    /// 而玩家根本没有选择权 —— 免疫规则该免的是事，不是卡）。
    pub fn absorb(&mut self, affliction: &Affliction) -> bool {
        if !affliction.cancellable || self.charges == 0 {
            return false;
        }
        self.charges -= 1;
        true
    }

    /// 一层都不剩了。空护身不该留在身上：面板会写出一个「有免罪卡但什么都不挡」的玩家。
    pub fn is_empty(&self) -> bool {
        self.charges == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn victim() -> Entity {
        Entity::from_raw_u32(3).expect("索引不该撞上保留值")
    }

    #[test]
    fn a_card_shield_absorbs_exactly_once() {
        let mut shield = Insulation { charges: 1 };
        let jail = Affliction::new(AfflictDomain::Detention, victim());
        assert!(shield.absorb(&jail), "第一件该被吃掉");
        assert_eq!(shield.charges, 0);
        assert!(
            !shield.absorb(&jail),
            "空护身不再抵消 —— 免罪卡一张抵一次，说成「这几天免疫」是另一位神的事"
        );
        assert!(
            shield.is_empty(),
            "耗尽的护身要被摘掉，别留一个什么都不挡的组件"
        );
    }

    /// 免疫规则免的是**事**，不是玩家的卡。
    #[test]
    fn an_immune_affliction_does_not_eat_the_shield() {
        let mut shield = Insulation { charges: 1 };
        let mut immune = Affliction::new(AfflictDomain::Reversal, victim());
        immune.cancellable = false;
        assert!(!shield.absorb(&immune));
        assert_eq!(shield.charges, 1, "不可抵消的那类不该把免罪卡白白耗掉");
        // 同一层护身接下来照样吃得掉可抵消的那类
        assert!(shield.absorb(&Affliction::new(AfflictDomain::Stay, victim())));
    }

    #[test]
    fn everything_is_cancellable_until_something_says_otherwise() {
        // 与 `Charge::new` 同一条默认：关掉位的是免疫方（P13），不是按类别预先写死
        for domain in [
            AfflictDomain::Detention,
            AfflictDomain::Stay,
            AfflictDomain::MoveCap,
            AfflictDomain::Reversal,
            AfflictDomain::Damage,
        ] {
            assert!(
                Affliction::new(domain, victim()).cancellable,
                "{domain:?} 默认不可抵消 —— 那 P13 的矩阵就没得关了"
            );
        }
    }
}
