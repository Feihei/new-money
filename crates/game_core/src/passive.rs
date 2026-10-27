//! 「可免／可转嫁」的**事件表**（P13b①）。设计正文是 `.agents/PLAN.md` §五 那一节
//! 「被动卡这道门与事件表」，这里只住表与它的读数。
//!
//! ## 为什么是一张表而不是两张
//!
//! P3a 之后规则层有两条互不相干的免疫路：[`crate::charge`] 里那个 `transferable(kind)`
//! （钱）与 `afflict` 里那个 `AfflictDomain::forgivable`（事）。这两张表的键一个是「一笔账」、
//! 一个是「一族事」，而 10-27 实报的那一格证明**两种键都答不了**：「出国旅游」这件事，
//! 命运牌发给你的那一张可以免、可以转，而**踩到联合航空**那一下不可以 ——
//! 同一个后果、同一个模型字段（[`DetentionPlace::Away`](crate::player::DetentionPlace)），
//! 按来源要答两种话。
//!
//! 所以这张表的键既不是「事 vs 账」也不是「后果」，而是**具体事件本身**，一行一格。
//! 行由**造这件事的那一处**报出来而不是由后果反推出来：见 [`trip_of`] 那条注 ——
//! 从 `place` 数得出「他在哪儿」，数不出「是谁把他送去的」，而后半句才是这一行要答的。
//!
//! ## 那一列被一条蕴含管住
//!
//! **不可免者不可转**（10-27 定）：[`PassiveEvent::shiftable`] 为真的那一行，
//! [`PassiveEvent::forgivable`] 必须也为真。它不是又一批格子，而是一条横切判据 ——
//! 于是新增一行只有一格要表态，另一格被它管住。
//!
//! 为什么非要有这条：「可免＝否而可转＝是」那一格等于在说「买不起免罪卡也没关系，
//! 买张嫁祸照样把这一下甩掉」，那正是从旁边捅穿 P13 花整阶段立的那条口径。
//! 它落成一条逐行断言的用例（[`tests::what_shifts_can_also_be_forgiven`]），一张表一处管 ——
//! 散在各臂里的 `if` 会在下一行加进来的那天失效。
//!
//! ## 这一模块今天**只是表**
//!
//! 门（那道咽喉、那个窗口、三问的顺序）在 P13b②③ 落地。① 干的是把两条旧表的答案搬进
//! 这一张：`charge::transferable` 与 `AfflictDomain::forgivable` 当场删除，而
//! [`Charge::transferable`](crate::charge::Charge) 与 [`Charge::cancellable`] 两格**里由卡答的
//! 那半条**改由这张表派生 —— 判据从两处收成一处，是门进场之前的地基。

use crate::card::PassiveCard;
use crate::charge::ChargeKind;
use crate::player::DetentionPlace;

/// 表上的一行 = 一个**具体事件**。
///
/// 排序按 PLAN §五 那张表的行序：先「失去行动能力」那一族（含三行还没有载体的），
/// 再「处境变了」那一族，再损害，最后是账。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassiveEvent {
    // ===== 失去行动能力那一族（免罪卡吃这一族）=====
    /// 坐牢（命运牌／陷害卡／女巫）
    JailTrip,
    /// 住院（命运牌／女巫）
    HospitalTrip,
    /// 出国旅游（命运牌／女巫）
    AwayTrip,
    /// 出国旅游（**踩到联合航空**）。与上面那一行是两个事件而不是一行两种来源 ——
    /// 这张表存在的理由就在这一行（10-27 例外）。它今天不走 [`crate::afflict`] 那条路
    /// （公司到访直接调 [`crate::landing::detain`]），等 ② 把五处收成一道门时才有入口。
    AirlineTrip,
    /// 被外星人劫持 —— **今天没有载体**（PLAN §三 那行仍未装配）。表先留一行：
    /// 装配它的人不许漏过表态，这正是穷尽表的价值。
    Abduction,
    /// 梦游 —— 同上（§三 反制链那句原话里有它，世界里没有它）
    Sleepwalk,
    /// 冬眠 —— 同上
    Hibernation,
    /// 住店（踩到别人的旅馆地块，B 型原型装配出来的那一个）
    Lodging,
    // ===== 处境变了，而不是一件灾落下来 =====
    /// 停留／乌龟／掉头。**一行三件事**：它们的共同点是「他这一趟的处境变了」，
    /// 而免罪卡那句「抵消落到自己头上的一次**厄运**」说的不是这个。
    /// 「可转」那一格在这一行不是独立表态，而是「不可免者不可转」的推论（10-27）。
    Hold,
    /// 踩地雷／被狗咬／定时炸弹爆炸／飞弹·核弹击中。
    ///
    /// 这一行今天同样**不构造** [`Affliction`](crate::afflict::Affliction) —— 走的是
    /// [`Books::damage`](crate::settle::Books::damage)。它与 `Abduction` 那三行的分别是
    /// 「有路可走而这条路不吃卡」与「压根没有这条路」，而对表来说两句都是「不许漏过表态」。
    ///
    /// ⚠️ 这一行是旧 `AfflictDomain::Damage` 那格的**归宿**：那个枚举位今天没有任何地方
    /// 构造它（PLAN §五 第 1 条点名的那笔旧债），已经从枚举里删掉，答案留在这里。
    Hazard,
    // ===== 账（免费卡吃这一族）=====
    /// 过路费／租金
    Rent,
    /// 罚款／税／服务费／公司到访 —— 「我欠的」那一族，所以也是嫁祸能吃下的那一族
    Bill,
    /// 买地／加盖／建商业设施：换的是**资产**，不是一笔债。免费卡管不到它
    /// （「买地免费」是大福神的特权，那是规则给的一条修正，不是玩家手里的一张卡），
    /// 嫁祸更管不到 —— 替别人付地钱不等于把地契给他。
    Acquire,
    /// 进账（横财、小游戏得分、乐透）。没有「免掉一笔收入」这种玩法：
    /// 衰神的「得钱作废」是规则给的 `Cancel`，不是玩家手里那张卡
    Windfall,
    /// 神明造成的收支。物价那一格与「可免」那一格今天都关着
    /// （PLAN §三 点名豁免神明金额），被动卡这一族照旧管不到它
    GodDeal,
}

impl PassiveEvent {
    /// 整张表。逐行断言的用例读它，所以**新增一行而没进这张数组**那条用例第一个红。
    pub const ALL: &'static [PassiveEvent] = &[
        Self::JailTrip,
        Self::HospitalTrip,
        Self::AwayTrip,
        Self::AirlineTrip,
        Self::Abduction,
        Self::Sleepwalk,
        Self::Hibernation,
        Self::Lodging,
        Self::Hold,
        Self::Hazard,
        Self::Rent,
        Self::Bill,
        Self::Acquire,
        Self::Windfall,
        Self::GodDeal,
    ];

    /// 人话：这一行在屏幕上叫什么。顶栏、模态与 dump 三处要说同一句话，所以映射只写一次。
    pub const fn label(self) -> &'static str {
        match self {
            Self::JailTrip => "坐牢",
            Self::HospitalTrip => "住院",
            Self::AwayTrip => "出国旅游",
            Self::AirlineTrip => "出国旅游",
            Self::Abduction => "被外星人劫持",
            Self::Sleepwalk => "梦游",
            Self::Hibernation => "冬眠",
            Self::Lodging => "住店",
            Self::Hold => "被按住",
            Self::Hazard => "意外损害",
            Self::Rent => "过路费",
            Self::Bill => "罚款",
            Self::Acquire => "置产",
            Self::Windfall => "进账",
            Self::GodDeal => "神明的账",
        }
    }

    /// 这一行的「可免」由**哪张**卡答；`None` = 没有任何抵消卡吃它。
    ///
    /// 返回一张卡而不是一个 bool：免罪卡挡的是**事**、免费卡挡的是**账**，一行只属一侧，
    /// 而门要拿这个答案去翻对应那副手牌。写成 bool 就得在两处各猜一次「这一行是事还是账」。
    ///
    /// **穷尽表，不许出现 `_`**（与 [`crate::landing::domain_of`] 同一条纪律）：
    /// 新增一行必须当场决定它归哪张卡管、或者干脆不归任何一张卡。
    pub const fn offset_card(self) -> Option<PassiveCard> {
        use PassiveEvent as E;
        match self {
            E::JailTrip | E::HospitalTrip | E::AwayTrip => Some(PassiveCard::Forgive),
            // 还没装配的那三行照样点名免罪卡：表先说对，等载体进来
            E::Abduction | E::Sleepwalk | E::Hibernation => Some(PassiveCard::Forgive),
            E::Rent | E::Bill => Some(PassiveCard::FreePass),
            // 两行「出国」是两个事件：命运牌那一张可以免，联合航空那一下不可以
            E::AirlineTrip | E::Lodging => None,
            E::Hold | E::Hazard | E::Acquire | E::Windfall | E::GodDeal => None,
        }
    }

    /// 免罪卡与免费卡能吃下这一行吗（表上「可免」那一列）。
    pub const fn forgivable(self) -> bool {
        self.offset_card().is_some()
    }

    /// 嫁祸卡能吃下这一行吗（表上「可转嫁」那一列）。
    ///
    /// **穷尽表**，而每一行都被 [`Self::forgivable`] 管着：那条蕴含由
    /// [`tests::what_shifts_can_also_be_forgiven`] 逐行验，不靠这里写对。
    pub const fn shiftable(self) -> bool {
        use PassiveEvent as E;
        match self {
            // 被关起来那一族：有载体的三行 10-27 答了「可转」
            E::JailTrip | E::HospitalTrip | E::AwayTrip => true,
            // 还没装配的三行照答「可转」，等载体自己进来（表先说对）
            E::Abduction | E::Sleepwalk | E::Hibernation => true,
            // 10-27 的例外两行 + 被按住那一族 + 损害：不可免，所以按蕴含也不可转
            E::AirlineTrip | E::Lodging | E::Hold | E::Hazard => false,
            // 只有「我欠的」能嫁祸出去（沿用旧表那句原话）
            E::Rent | E::Bill => true,
            E::Acquire | E::Windfall | E::GodDeal => false,
        }
    }

    /// 一笔**账**是哪一行（[`Charge`] 的构造读它，所以「哪种钱归哪行」只有这一处映射）。
    ///
    /// 归行按「这笔钱是什么」：`Service` 归 [`PassiveEvent::Bill`] 而不是 `Rent`，因为表上
    /// 那一行写的是「罚款／税／服务费／公司到访」—— 它与 `Rent` 的两列答案今天相同，
    /// 但相同是两个数而不是一个数：它们是两种钱，而下一句要改其中一种时不该连坐。
    pub const fn of_charge(kind: ChargeKind) -> Self {
        match kind {
            ChargeKind::Rent { .. } => Self::Rent,
            ChargeKind::Service | ChargeKind::Penalty | ChargeKind::Levy => Self::Bill,
            ChargeKind::BuyLand { .. } | ChargeKind::Upgrade { .. } | ChargeKind::Build { .. } => {
                Self::Acquire
            }
            ChargeKind::Bonus => Self::Windfall,
            ChargeKind::God => Self::GodDeal,
        }
    }
}

/// 把「被关起来」这一族按去处数成行 —— **只对得上来源的地方有效**。
///
/// `Away` 那一格是本表最容易被误用的一处：命运牌的出国旅游与踩到联合航空的出国旅游是
/// 同一个 [`DetentionPlace`]、两个行。所以这一句只能由「卡／女巫那一条路」调用（那里
/// 确实只有 trip 这一族），而公司到访那一条**必须自己点名** [`PassiveEvent::AirlineTrip`]，
/// 不许从 `place` 反推。有一条用例钉住这一格
/// （[`tests::the_same_trip_is_two_rows_from_one_place`]）。
pub const fn trip_of(place: DetentionPlace) -> PassiveEvent {
    match place {
        DetentionPlace::Jail => PassiveEvent::JailTrip,
        DetentionPlace::Hospital => PassiveEvent::HospitalTrip,
        DetentionPlace::Away => PassiveEvent::AwayTrip,
        // 住店不在「trip」那一族里挨这一问：旅馆那一格是「强制住宿」，与「出国」是两行，
        // 而它是 10-27 的例外之一 —— 从去处数出行时它必须是**关着答案**的那一行
        DetentionPlace::Hotel => PassiveEvent::Lodging,
    }
}

#[cfg(test)]
mod tests {
    use super::PassiveEvent as E;
    use super::*;
    use crate::charge::{Charge, ChargeDomain};
    use crate::land::LandId;
    use bevy::prelude::Entity;

    fn entity(n: u32) -> Entity {
        Entity::from_raw_u32(n).expect("索引不该撞上保留值")
    }

    fn land() -> LandId {
        LandId(1)
    }

    /// 这张表的**承重墙**（10-27 的原话：不可免者不可转）。
    ///
    /// 一行都不许省：写成一整表一处断言，而不是散在各臂里的 `if` —— 散开的版本
    /// 在下一行加进来的那天就没人管了。而它红的时候说的话很具体：
    /// 有人给免罪卡开了一条旁路（「这一下免不掉，但你花张嫁祸照样甩得掉」）。
    #[test]
    fn what_shifts_can_also_be_forgiven() {
        let shifted: Vec<_> = E::ALL.iter().filter(|row| row.shiftable()).collect();
        assert!(
            !shifted.is_empty(),
            "整张表一行都不可转 —— 那这张用例是空跑的，而嫁祸卡明明有六行可转"
        );
        for row in E::ALL {
            assert!(
                !row.shiftable() || row.forgivable(),
                "{row:?} 可转嫁却不可免 —— 那就是在说「买不起免罪卡也没关系，\
                 买张嫁祸照样把这一下甩掉」，而这条旁路正是 P13 花整阶段堵住的"
            );
        }
    }

    /// 逐行钉住两列的读数。穷尽表保证「新增一行必须当场表态」，这一条保证
    /// 「已有各行说的还是那句话」—— 改它等于改玩法口径，不该在一次无关的重构里顺手发生。
    #[test]
    fn every_row_answers_its_two_columns() {
        // (行, 可免, 可转嫁) —— 与 PLAN §五 那张表逐行对齐
        let rows: [(E, bool, bool); 15] = [
            (E::JailTrip, true, true),
            (E::HospitalTrip, true, true),
            (E::AwayTrip, true, true),
            (E::AirlineTrip, false, false),
            (E::Abduction, true, true),
            (E::Sleepwalk, true, true),
            (E::Hibernation, true, true),
            (E::Lodging, false, false),
            (E::Hold, false, false),
            (E::Hazard, false, false),
            (E::Rent, true, true),
            (E::Bill, true, true),
            (E::Acquire, false, false),
            (E::Windfall, false, false),
            (E::GodDeal, false, false),
        ];
        assert_eq!(
            rows.len(),
            E::ALL.len(),
            "表上加了行而这张对照表没跟着加 —— 上面那条蕴含用例就此管不到它"
        );
        for (row, can_forgive, can_shift) in rows {
            assert!(E::ALL.contains(&row), "{row:?} 没进 `ALL`");
            assert_eq!(row.forgivable(), can_forgive, "{row:?} 的「可免」说反了");
            assert_eq!(row.shiftable(), can_shift, "{row:?} 的「可转嫁」说反了");
        }
    }

    /// 「可免」那一格要说清是**哪张卡**答的：免罪卡挡事、免费卡挡账，一行只属一侧。
    ///
    /// 反过来的那一格更要钉住：`offset_card()` 说得出卡而 `forgivable()` 说「不免」，
    /// 或者压根没卡管却答了「可免」，门就会去翻一副根本没有这一张的手牌、
    /// 或者对着一张不存在的卡问人。
    #[test]
    fn each_forgivable_row_names_which_card_answers_it() {
        for row in E::ALL {
            match row.offset_card() {
                None => assert!(!row.forgivable(), "{row:?} 没有卡管却答了「可免」"),
                Some(card) => {
                    assert!(row.forgivable(), "{row:?} 点名了 {card:?} 却答「不可免」");
                    assert!(
                        !matches!(card, PassiveCard::Shift),
                        "{row:?} 把嫁祸卡当成了抵消卡 —— 它转走这件事，不免除这件事"
                    );
                }
            }
        }
        assert_eq!(E::JailTrip.offset_card(), Some(PassiveCard::Forgive));
        assert_eq!(E::Rent.offset_card(), Some(PassiveCard::FreePass));
        assert_eq!(
            E::Hold.offset_card(),
            None,
            "免罪卡不吃「被按住」（P13 定稿）"
        );
        assert_eq!(
            E::Hazard.offset_card(),
            None,
            "损害也不吃（今天连构造都没有）"
        );
    }

    /// 账那侧的映射与两列必须自洽。前两条等价式是旧的 `charge::transferable` 那张穷尽表的
    /// **替身**：删掉那张表不等于删掉它钉住的读数，读数搬到这里来了。
    ///
    /// 第三条是**跨表**的：`免费卡` 的作用域是「`Rent` 与 `Penalty` 两域」（`deck.rs` 那个
    /// `FREE_PASS_DOMAINS`），而表上答「可免」的账那两行必须与它同值。两处各改一半的形状是
    /// 「域筛掉了、表说可以免」或反过来 —— 那种分叉不崩，只是那张卡在某一类钱上悄悄失效。
    #[test]
    fn each_kind_of_money_falls_on_one_row_and_keeps_its_two_answers() {
        let rows: [(ChargeKind, E); 9] = [
            (ChargeKind::Rent { land: land() }, E::Rent),
            (ChargeKind::Service, E::Bill),
            (ChargeKind::Penalty, E::Bill),
            (ChargeKind::Levy, E::Bill),
            (ChargeKind::BuyLand { land: land() }, E::Acquire),
            (ChargeKind::Upgrade { land: land() }, E::Acquire),
            (ChargeKind::Build { land: land() }, E::Acquire),
            (ChargeKind::Bonus, E::Windfall),
            (ChargeKind::God, E::GodDeal),
        ];
        for (kind, row) in rows {
            assert_eq!(E::of_charge(kind), row, "{kind:?} 归错行");
            assert_eq!(
                crate::charge::transferable(kind),
                row.shiftable(),
                "{kind:?} 的可转嫁读数与它那一行不一致（`charge::transferable` 是这一行的派生）"
            );
            let domain = crate::charge::charge_domain_of(kind);
            assert_eq!(
                row.forgivable(),
                matches!(domain, ChargeDomain::Rent | ChargeDomain::Penalty),
                "{kind:?} 归 {domain:?} 域，而免付卡那张作用域表与这一行的「可免」分叉了"
            );
        }
    }

    /// 表上最尖的那一格：同一个 `DetentionPlace::Away`，命运牌可免可转，联合航空不可免不可转。
    /// 这张表的存在理由就是这一格 —— 旧的按「事 vs 账」分家的两张表在这里必然答错一边。
    #[test]
    fn the_same_trip_is_two_rows_from_one_place() {
        assert_eq!(trip_of(DetentionPlace::Away), E::AwayTrip);
        assert!(E::AwayTrip.forgivable() && E::AwayTrip.shiftable());
        // 公司到访那一条**点名**另一行，而它在屏幕上说的是同一句话
        assert_eq!(E::AirlineTrip.label(), E::AwayTrip.label());
        assert!(!E::AirlineTrip.forgivable() && !E::AirlineTrip.shiftable());
        assert_ne!(
            E::AirlineTrip,
            E::AwayTrip,
            "两行必须还是两行 —— 合成一行就是那张旧表的错"
        );
        // 从去处数出行的那一句不许把旅馆算成出国：它是 10-27 的另一条例外
        assert_eq!(trip_of(DetentionPlace::Hotel), E::Lodging);
        assert!(!E::Lodging.forgivable());
    }

    /// 三行「还没有载体」的是**留给装配它的人表态的**，不是三条死码：它们必须与那三行
    /// 有载体的一样答「可免可转」，否则 `Abduction` 一旦落地就成了一条悄悄不可免的厄运
    /// （那种错不会崩，只会让玩家说「这张卡是假的」）。
    #[test]
    fn the_reserved_rows_say_what_the_shipped_ones_say() {
        for row in [E::Abduction, E::Sleepwalk, E::Hibernation] {
            assert!(
                row.forgivable() && row.shiftable(),
                "{row:?} 那三行是试金石"
            );
            assert_eq!(row.offset_card(), Some(PassiveCard::Forgive));
        }
    }

    /// 表上的答案必须真的落到世界里：[`Charge`] 身上那两位由行派生，而管线的
    /// `Adjustment::Cancel`／`TransferTo` 读的就是那两位。
    ///
    /// 这一条是「表不是文档」的那半句：把 `of_charge` 接错一根线，上面那几张对照表
    /// 照样全绿（它们读的是表，不是世界）。
    #[test]
    fn the_charge_carries_its_row_answer() {
        let mine = entity(1);
        let rent = Charge::new(ChargeKind::Rent { land: land() }, Some(mine), None, 3_000);
        assert!(rent.transferable, "过路费那一行答的是可转");
        let buy = Charge::new(
            ChargeKind::BuyLand { land: land() },
            Some(mine),
            None,
            3_000,
        );
        assert!(
            !buy.transferable,
            "置产那一行答的是不可转：替别人付地钱不等于给他地契"
        );
        let bonus = Charge::new(ChargeKind::Bonus, None, Some(mine), 3_000);
        assert!(!bonus.transferable);
        // 抵消那一位**不由这张表派生**：`Charge::cancellable` 问的是「任何 `Cancel`
        // 吃不吃得动这笔账」，神明与规则给的修正也读它（小福神的买地免费、衰神的得钱作废）。
        // 这张表管的是**被动卡**那一族，别把它扩成整条链的总闸 ——
        // 关掉 `cancellable` 的是免疫方（P13 魔法屋），不是种类
        assert!(rent.cancellable && buy.cancellable && bonus.cancellable);
    }
}
