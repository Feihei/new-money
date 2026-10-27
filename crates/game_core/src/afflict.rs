//! 厄运（P3a）：会落到某个玩家头上的**事**，与「钱」是两类账。
//!
//! [`crate::charge`] 管的是「一笔钱在离开钱包之前要过的那条链」，它的输入输出都是金额。
//! 而「被关进监狱 5 天」「原地停留」「向后转」不落金额 —— 但它们同样需要一个承受者、
//! 同样可能被抵消或转嫁，所以**形状必须与 `Charge` 一致**：
//!
//! ```text
//! Affliction { event, victim }            ↔   Charge { kind, payer/payee, cancellable, transferable }
//! Insulation::absorb(...)                 ↔   charge::stage(...) 里的 Adjustment::Cancel
//! ```
//!
//! 为什么抵消的**来源**是组件而不是 [`crate::charge::Corrections`] 那张表：表里的修正是
//! 「持续生效」（大穷神在这几天里把我所有支出翻倍），而免罪卡是**一次性**的（抵掉下一次
//! 就没了）。把一次性消耗塞进那张表，就会撞上「报价也调一次 `stage`」这个坑 ——
//! [`crate::settle::Books::quoted`] 只是给 UI 看个数，它不该花掉玩家手里的一张卡。
//! 护身挂在玩家身上，只有真正生效的那次结算才会去 `absorb` 一层。
//!
//! ## 两个位今天在一张表上
//!
//! 这一族以前自带一张 `AfflictDomain::forgivable` 穷尽表，而钱那一侧另有一张
//! `charge::transferable`。**P13b① 把两张收成一张**：读数全部搬到
//! [`crate::passive`] 那张**按具体事件逐行**答的表上，这里只剩「哪一行 + 落在谁头上」。
//!
//! 收成一张的理由是最尖的那一格：命运牌的出国旅游可以免、可以转，而踩到联合航空那一下
//! 两样都不可以 —— 它们是同一个 [`DetentionPlace::Away`](crate::player::DetentionPlace)、
//! 两个行。按「一族事」建键答不出这种差别，所以 [`Affliction`] 现在带的是**行**而不是族
//! （行怎么来的见 [`Affliction::detain`] 那条注）。
//!
//! 顺带一笔旧债：旧枚举里那一格 `Damage`（怪兽卡、飞弹、新闻天灾）**表上有、路上没有** ——
//! 全工程没有任何一处构造它，损害走的是 [`Books::damage`](crate::settle::Books::damage)。
//! ① 把它从枚举里删掉，答案留在表上那一行 [`PassiveEvent::Hazard`](crate::passive::PassiveEvent::Hazard)
//! （「不许留一格假答案」，PLAN §五 第 1 条）。
//!
//! ⚠️ 这一位只管**卡**。神明那一半（[`crate::gods::resists`]，大福神与天使的厄运豁免）
//! 仍然 blanket 挡所有族，而且**先问神再问卡**（[`crate::deck::resist`] 的注写了顺序为什么
//! 有内容）。所以「魔法屋的十二条不受任何卡与神影响」那半条不是这张表答的，
//! 是 `magic.rs` 根本不走 `resist` —— 见那里的模块头。

use bevy::prelude::*;

use crate::passive::{PassiveEvent, trip_of};
use crate::player::DetentionPlace;

/// 一件要落到某人头上的事。
///
/// 带的不是「后果的种类」而是**表上的那一行**：同一句「离场三天」，从命运牌来与从
/// 联合航空来是两行，而这两行的两列答案相反。行由造这件事的那一处点名，
/// 不许从 payload 反推（见 [`trip_of`] 的注）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affliction {
    /// 表上的哪一行（决定「可免」「可转嫁」两列）
    pub event: PassiveEvent,
    pub victim: Entity,
}

impl Affliction {
    /// 把一个人送去某一处。**行按去处数**，而那只对得上「卡／女巫」这一族来源：
    /// 命运牌、陷害卡、女巫的坐牢住院都从这里进，所以它们拿到的是可以免的那几行。
    /// 踩到联合航空那一条**不走这一句**，它自己点名
    /// [`PassiveEvent::AirlineTrip`](crate::passive::PassiveEvent::AirlineTrip)。
    pub fn detain(victim: Entity, place: DetentionPlace) -> Self {
        Self::at(trip_of(place), victim)
    }

    /// 原地停留 N 回合（停留卡、新闻「豪雨」、女巫「原地停留」）。
    ///
    /// 三条「处境变了」的效果共用 [`PassiveEvent::Hold`] 那一行，所以各留一个构造函数而不是
    /// `Affliction::new(Hold, victim)` 三处各写一遍：调用方写的是**它干的这件事**，
    /// 归哪一行由这里定 —— 那才是「新增一行要改几处」只有一个答案的形状。
    pub fn stay(victim: Entity) -> Self {
        Self::at(PassiveEvent::Hold, victim)
    }

    /// 接下来 N 个掷骰回合只走 1 格（乌龟卡，P5）。与 [`Self::stay`] 一行而两件事：
    /// 一个让人不动，一个让人走不远。
    pub fn slow(victim: Entity) -> Self {
        Self::at(PassiveEvent::Hold, victim)
    }

    /// 行进方向被反转（向后转）。
    pub fn reversal(victim: Entity) -> Self {
        Self::at(PassiveEvent::Hold, victim)
    }

    /// 造一件落在 `victim` 头上的事，行由上面那四个构造函数之一给。
    ///
    /// 它是**私有**的，而且不是偷懒：「这一行是哪件事」有两种来源 —— 从去处数出来
    /// （[`trip_of`]：命运牌、陷害卡、女巫那三条路）与**点名**（联合航空、住店那两条，
    /// 等 ② 接进这条路时自己点名）。把这一句开放出去，调用方就能挑一个跟手上那件事
    /// 不对得上的行，而那种错的形状是「命运牌的出国忽然免疫了」或反过来。
    const fn at(event: PassiveEvent, victim: Entity) -> Self {
        Self { event, victim }
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
///
/// ⚠️ 这一位是 **P13b② 的回炉对象**：10-27 拍「被动卡这一族从此没有出牌这个动作」，
/// 所以「按一次挂上位」这个容器要让位给「手里那张卡」—— 免罪卡的张数本来就在 `Hand` 里，
/// 抄一份到组件上就是两处数同一副牌。今天它还是 armed 的那套（行为与 P13 收口时一致），
/// 门进场那一刻删除。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Insulation {
    pub charges: u32,
}

impl Insulation {
    /// 用一层去抵消 `affliction`。返回 `true` = 这件事没有发生。
    ///
    /// 表上答「不可免」的那几行**不吃掉护身**（否则一张免罪卡会被「向后转」白白耗掉，
    /// 而玩家根本没有选择权 —— 免疫规则该免的是事，不是卡）。
    pub fn absorb(&mut self, affliction: &Affliction) -> bool {
        if !affliction.event.forgivable() || self.charges == 0 {
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
        let jail = Affliction::detain(victim(), DetentionPlace::Jail);
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
        // 转向今天就是那一条活例子：表上那一行不可免，于是护身一层都不该动
        let reversal = Affliction::reversal(victim());
        assert!(!shield.absorb(&reversal));
        assert_eq!(shield.charges, 1, "挡不掉的那件事也不该白吃一张免罪卡");
        // 同一层护身照样吃得掉可免的那一行
        assert!(shield.absorb(&Affliction::detain(victim(), DetentionPlace::Hospital)));
    }

    /// P13 定稿的免疫口径（10-25 拍板）+ P13b① 的那次并表：**只有「被关起来」那一族吃免罪卡**，
    /// 而「被关起来」在表上是三行（坐牢／住院／出国），不是旧枚举里那**一族**。
    ///
    /// 这一条从旧表 `only_a_trip_away_is_forgivable` 改过来的地方正是形状：
    /// 过去逐**族**问「护身吃不吃」，现在逐**行**问 —— 因为同一族里已经有一行
    /// （[`PassiveEvent::AirlineTrip`]）的答案与另外三行相反，族这个键装不下它。
    #[test]
    fn only_a_trip_away_is_forgivable() {
        for place in [
            DetentionPlace::Jail,
            DetentionPlace::Hospital,
            DetentionPlace::Away,
        ] {
            assert!(
                Affliction::detain(victim(), place).event.forgivable(),
                "{place:?} 该吃免罪卡 —— 免罪卡那句『抵消一次厄运』说的就是这一族"
            );
        }
        for event in [
            PassiveEvent::Hold,
            PassiveEvent::Hazard,
            PassiveEvent::AirlineTrip,
            PassiveEvent::Lodging,
        ] {
            assert!(
                !event.forgivable(),
                "{event:?} 不该被免罪卡挡掉 —— 它是处境变了或压根不吃卡，不是一件灾落下来"
            );
        }
        // 而「被关起来」那一族里最尖的一格：同去处、不同来源，两行答案相反
        assert_eq!(trip_of(DetentionPlace::Away), PassiveEvent::AwayTrip);
        assert!(PassiveEvent::AwayTrip.forgivable());
        assert!(!PassiveEvent::AirlineTrip.forgivable());
    }
}
