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
//! 两个形状各有一个**域**表：[`crate::charge::transferable`] 答「这一族钱允不允许转嫁」，
//! 下面的 [`AfflictDomain::forgivable`] 答「这一族事允不允许被一张免罪卡挡掉」。
//!
//! 免疫口径由 P13 定稿（10-25 拍板）：**免罪卡只挡「被关起来」那一族**（陷害、命运牌的
//! 坐牢/住院/出国、女巫的坐牢/住院），乌龟／停留／转向／损害**不吃护身**。理由不是嫌卡太强，
//! 是那句卡面文案：「抵消落到自己头上的一次**厄运**」——被人按住一回合、走不远、掉了个头，
//! 是他这一趟的处境变了，不是有一件灾落在他头上。挡掉这些会让免罪卡变成一张万能解，
//! 而它玩家手里那一层该留给真正把人拎走的那一下。
//!
//! ⚠️ 这一位只管**卡**。神明那一半（[`crate::gods::resists`]，大福神与天使的厄运豁免）
//! 仍然 blanket 挡所有五族，而且**先问神再问卡**（[`crate::deck::resist`] 的注写了顺序为什么
//! 有内容）。所以「魔法屋的十二条不受任何卡与神影响」那半条不是这张表答的，
//! 是 `magic.rs` 根本不走 `resist` —— 见那里的模块头。

use bevy::prelude::*;

/// 一类厄运。免疫矩阵按这一位逐格填「免罪卡挡不挡」，答案在
/// [`AfflictDomain::forgivable`] 那一张穷尽表上。
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
    /// 允不允许被一张免罪卡整笔抵消。它**不是**「这位玩家免疫吗」——那是 [`Self::domain`]
    /// 过 [`AfflictDomain::forgivable`] 之后、再由 [`Insulation`] 自己回答的第二问。
    pub cancellable: bool,
}

impl AfflictDomain {
    /// 这一族事允不允许被**免罪卡**挡掉（P13 定稿的免疫口径，10-25 拍板）。
    ///
    /// 穷尽表：新增一族必须当场回答它，漏答不会静默变成「挡得住」或「挡不住」。
    /// 只有 [`Self::Detention`] 一行是 `true` —— 免罪卡卡面写的是「抵消一次**厄运**」，
    /// 而被人按住一回合、走不远、掉了个头是处境变了，不是有一件灾落在头上。
    ///
    /// 这一位**不管神明**：大福神与天使的厄运豁免挡所有族（[`crate::gods::resists`]）。
    /// 也不管魔法屋：女巫那十二条根本不走这条路（`magic.rs` 模块头）。
    pub const fn forgivable(self) -> bool {
        match self {
            Self::Detention => true,
            Self::Stay | Self::MoveCap | Self::Reversal | Self::Damage => false,
        }
    }
}

impl Affliction {
    /// 造一件落到 `victim` 头上的事。**不可抵消的那几族在这里就已经带上答案**：
    /// 让调用方各填一位，就会出现「某条新法术忘了填 false，于是白吃玩家一张免罪卡」。
    pub fn new(domain: AfflictDomain, victim: Entity) -> Self {
        Self {
            domain,
            victim,
            cancellable: domain.forgivable(),
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
        // 转向今天就是那一条活例子：不可抵消，于是护身一层都不该动
        let reversal = Affliction::new(AfflictDomain::Reversal, victim());
        assert!(!shield.absorb(&reversal));
        assert_eq!(shield.charges, 1, "挡不掉的那件事也不该白吃一张免罪卡");
        // 同一层护身照样吃得掉可抵消的那一族
        assert!(shield.absorb(&Affliction::new(AfflictDomain::Detention, victim())));
    }

    /// P13 定稿的免疫口径（10-25 拍板）：**只有「被关起来」那一族吃免罪卡**。
    ///
    /// 这一条是穷尽表，所以它的价值在「新增一族必须当场表态」——把它删掉，
    /// `forgivable` 的 match 就少了一条被钉住的分支。
    #[test]
    fn only_a_trip_away_is_forgivable() {
        assert!(
            AfflictDomain::Detention.forgivable(),
            "免罪卡卡面那句「抵消一次厄运」吃的就是这一族：坐牢／住院／出国"
        );
        for domain in [
            AfflictDomain::Stay,
            AfflictDomain::MoveCap,
            AfflictDomain::Reversal,
            AfflictDomain::Damage,
        ] {
            assert!(
                !Affliction::new(domain, victim()).cancellable,
                "{domain:?} 不该被免罪卡挡掉 —— 它是处境变了，不是一件灾落下来"
            );
        }
    }
}
