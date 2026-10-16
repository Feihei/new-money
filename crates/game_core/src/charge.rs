//! 结算修正管线（P2①）：一笔钱在离开钱包之前要过的那条链。
//!
//! 为什么不是「发一条 `ChargeRequested` 消息、各 subscriber 上去改账」：消息是**帧末
//! 批量投递**的（见 [`crate::msg`] 模块头），而租金必须在同一帧内扣完 —— 付不起要当场
//! 判破产，回合机紧接着才知道是往下走还是收摊。所以链是**纯函数**，输入是数据而不是回调：
//! 物价指数、免费卡、大穷神在管线眼里都是同一张表里的一行。P3 的卡与 P6 的神明各自往
//! [`Corrections`] 里登记，谁都不用回来改这条链。
//!
//! 三条纪律：
//! - **只从 `base` 算起**。[`stage`] 的金额永远由原始金额推出来，不在上一次的成果上叠 ——
//!   于是「报价」与「成交」调多少次都是同一个数，重复缩放这件事在结构上不可能发生。
//! - **豁免表与两个位都是穷尽表，不许出现 `_`**（P1 的 [`crate::landing::domain_of`] 同一个
//!   形状）。新增一种钱而忘了决定它受不受物价缩放，编译不过。
//! - **两个位是「允不允许」而不是「会不会」**：[`Charge::cancellable`] 关掉，任何 `Cancel`
//!   都吃不动这笔账 —— 魔法屋的免疫规则改的就是这个位（P13），而不是去注册一条「抵消掉
//!   抵消」的修正。

use bevy::prelude::*;

use crate::land::LandId;

/// 这笔钱**是什么**。物价豁免表与两个位都按它查，所以每种钱都必须能落进一类。
///
/// **管线的边界**：这里只有「账单」——有名目、可能被减免的钱。股票买卖与银行存取
/// **不在**这张表上：它们是账面搬运（买多少由 `economy::buy_plan` 按现金封顶，
/// 存取由 `deposit_plan`/`loan_plan` 封顶），没有「减免」这回事，硬套进管线只会
/// 让「存款被物价指数放大」这种荒谬事变得可能。物价不碰股价这条规则因此由
/// 「股价压根不过管线」来保证，回归见 `tests/price_index.rs`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeKind {
    /// 踩到别人地块的过路费
    Rent { land: LandId },
    /// 买地地价（收款方是市场，即 `payee: None`）
    BuyLand { land: LandId },
    /// 加盖一级的建筑费
    Upgrade { land: LandId },
    /// 罚款：卡牌罚金、命运损失、交保释金
    Penalty,
    /// 奖金：横财、小游戏得分、乐透
    Bonus,
    /// 神明造成的收支（P6 登记）。物价指数点名豁免这一类 —— 豁免要能写在表上才算规则，
    /// 写在注释里就只能靠人记得
    God,
}

/// 「这类钱归哪个域」。修正的作用域按域筛，不按 `ChargeKind` 筛 ——
/// kind 带着地块号，不能当键用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeDomain {
    Property,
    Penalty,
    Bonus,
    God,
}

/// kind → 域。**不许出现 `_`**。
pub const fn charge_domain_of(kind: ChargeKind) -> ChargeDomain {
    match kind {
        ChargeKind::Rent { .. } | ChargeKind::BuyLand { .. } | ChargeKind::Upgrade { .. } => {
            ChargeDomain::Property
        }
        ChargeKind::Penalty => ChargeDomain::Penalty,
        ChargeKind::Bonus => ChargeDomain::Bonus,
        ChargeKind::God => ChargeDomain::God,
    }
}

/// 物价指数缩不缩这一类钱（PLAN §三「物价指数」行）。**不许出现 `_`**。
pub const fn scales_with_price(kind: ChargeKind) -> bool {
    match kind {
        // 地价一族三个都缩。升级费不缩的话，物价 5 倍时加盖等于拿 1980 年的价盖楼
        ChargeKind::Rent { .. } | ChargeKind::BuyLand { .. } | ChargeKind::Upgrade { .. } => true,
        ChargeKind::Penalty | ChargeKind::Bonus => true,
        // PLAN §三 点名豁免神明金额：那是玩家记忆里的「固定数」，缩它等于改神明的剧本
        ChargeKind::God => false,
    }
}

/// 允不允许转嫁给别人（嫁祸卡读这一位）。
pub const fn transferable(kind: ChargeKind) -> bool {
    match kind {
        // 只有「我欠的」能嫁祸出去：过路费与罚款。买地要的是地契，转给别人付钱不等于给别人地
        ChargeKind::Rent { .. } | ChargeKind::Penalty => true,
        ChargeKind::BuyLand { .. }
        | ChargeKind::Upgrade { .. }
        | ChargeKind::Bonus
        | ChargeKind::God => false,
    }
}

/// 一笔账。`base` 是名义金额（租金表上写的那个），`amount` 是过完链之后的实付金额。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Charge {
    pub kind: ChargeKind,
    /// 谁付。`None` = 系统出钱（奖金）
    pub payer: Option<Entity>,
    /// 谁收。`None` = 收进系统（买地给市场、罚金交给银行）
    pub payee: Option<Entity>,
    pub base: i64,
    pub amount: i64,
    /// 允不允许被整笔抵消。六类钱**都存在**合法的「整笔不付」规则（免罪卡、买地免费、
    /// 大财神赖账、衰神得钱作废、厄运豁免），所以默认开着 —— 把它关掉的是免疫方
    /// （P13 魔法屋），而不是种类。这也是这个位必须挂在账上而不是挂在修正上的原因：
    /// 「这笔账免不免」是账的性质，不是「谁手上有什么卡」。
    pub cancellable: bool,
    pub transferable: bool,
}

impl Charge {
    pub fn new(kind: ChargeKind, payer: Option<Entity>, payee: Option<Entity>, base: i64) -> Self {
        Self {
            kind,
            payer,
            payee,
            base,
            amount: base,
            cancellable: true,
            transferable: transferable(kind),
        }
    }

    /// 付钱的一方，没有付钱方时是收钱的一方。修正按 bearer 找人。
    pub fn bearer(&self) -> Option<Entity> {
        self.payer.or(self.payee)
    }
}

/// 修正的作用域。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 什么账都吃（免罪卡：这一回合我什么都不付）
    Any,
    /// 只吃某一类
    Domain(ChargeDomain),
}

impl Scope {
    pub fn matches(self, kind: ChargeKind) -> bool {
        match self {
            Scope::Any => true,
            Scope::Domain(domain) => charge_domain_of(kind) == domain,
        }
    }
}

/// 一条修正对金额做的是什么。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Adjustment {
    /// 整笔抵消。只对 `cancellable` 的账生效。
    Cancel,
    /// 换人付。只对 `transferable` 的账生效，且只改 `payer`。
    TransferTo { to: Entity },
    /// 乘一个系数（大穷神过路费加倍 `2.0`、小财神被动支出减半 `0.5`）。
    Scale { by: f64 },
    /// 加/减一个固定额（「过路费 +500」这种）。减到底不会穿到负数。
    Flat { delta: i64 },
}

/// 这条修正是谁给的。唯一的作用是**让调试面板能说出「这 0 元是因为免费卡」**——
/// 查「钱为什么是这个数」的时候，少了这一栏就得回去读代码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 卡片（P3 起）
    Card,
    /// 神明附身（P6 起）
    God,
    /// 规则本身（P10 的董事长免费、P12 的保释减免）
    Rule,
}

/// 一条登记好的修正。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Correction {
    /// 只对这位玩家承担的账生效
    pub bearer: Entity,
    pub scope: Scope,
    pub adjustment: Adjustment,
    pub source: Source,
}

/// 修正登记处。P3 的卡、P6 的神明往这里 push，管线本身不动。
///
/// 没有「撤销」接口是有意的：一条修正的寿命由它的来源管（卡片打出即消失、神明到期即清场），
/// 那里才知道「什么时候该消失」。这里只留登记与读取两侧。
/// 玩家破产、实体被回收也不会误伤：`Entity` 相等是连**世代**一起比的，
/// 复用的那个索引带着新世代，`for_bearer` 认不出来，登记的条目就此变成惰性的。
#[derive(Resource, Debug, Default)]
pub struct Corrections {
    items: Vec<Correction>,
}

impl Corrections {
    pub fn register(&mut self, correction: Correction) {
        self.items.push(correction);
    }

    pub fn for_bearer(&self, bearer: Entity) -> impl Iterator<Item = &Correction> + '_ {
        self.items.iter().filter(move |c| c.bearer == bearer)
    }

    /// 全表，只给调试面板用（`NM_DUMP=1` 那一栏要能数出「现在挂着几条」）。
    pub fn iter(&self) -> impl Iterator<Item = &Correction> + '_ {
        self.items.iter()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// 过一遍链：先物价指数（世界的状态，永远在），再按登记顺序应用各条修正。
///
/// 顺序是**规则**不是实现细节：物价 2.5 × 穷神加倍 = 5 倍，反过来先加倍再乘物价是同一个数，
/// 但「先抵消再乘」和「先乘再抵消」不是 —— 这里选前者（`Cancel` 之后 `amount` 归零，
/// 后面的系数乘不动零）。
///
/// 转嫁之后**不再走一遍新付款人的修正**：A 的嫁祸会被 B 的免罪吃掉，那是另一条规则，
/// P3 做卡的时候再拍，别在这儿默认。
pub fn stage(mut charge: Charge, price_index: f32, corrections: &Corrections) -> Charge {
    charge.amount = if scales_with_price(charge.kind) {
        scale(charge.base, price_index as f64)
    } else {
        charge.base
    };
    let Some(bearer) = charge.bearer() else {
        return charge;
    };
    for correction in corrections.for_bearer(bearer) {
        if !correction.scope.matches(charge.kind) {
            continue;
        }
        match correction.adjustment {
            Adjustment::Cancel if charge.cancellable => charge.amount = 0,
            Adjustment::Scale { by } => charge.amount = scale(charge.amount, by),
            Adjustment::Flat { delta } => charge.amount = (charge.amount + delta).max(0),
            Adjustment::TransferTo { to }
                if charge.transferable && charge.payer.is_some() && to != bearer =>
            {
                charge.payer = Some(to);
            }
            // 位没开的那两条：什么都不做。写成显式分支是为了让「为什么没生效」这件事
            // 留在代码里，而不是消失在 match 的空白处
            Adjustment::Cancel | Adjustment::TransferTo { .. } => {}
        }
    }
    charge
}

/// 乘系数回到整数。四舍五入而不是 floor：物价 2.5 这种半数下，floor 会让相邻两级
/// 租金差出一块钱的毛刺，而玩家在租金表上读不到这一块钱从哪来。
fn scale(amount: i64, by: f64) -> i64 {
    if by == 1.0 {
        return amount;
    }
    (amount as f64 * by).round().max(0.0) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn land(id: u32) -> LandId {
        LandId(id)
    }

    fn player(n: u32) -> Entity {
        Entity::from_raw_u32(n).expect("索引不该撞上 Entity 的保留值")
    }

    /// 三张表的取值逐个钉住。穷尽表保证「新加一种钱必须做决定」，这一条保证
    /// 「已有种类的决定没被人顺手改掉」—— 物价缩不缩、能不能转嫁都是玩法口径，
    /// 改它等于改平衡，不该在一次无关的重构里顺手发生。
    #[test]
    fn the_three_tables_say_what_we_mean() {
        let rows: [(ChargeKind, ChargeDomain, bool, bool); 6] = [
            (
                ChargeKind::Rent { land: land(1) },
                ChargeDomain::Property,
                true,
                true,
            ),
            (
                ChargeKind::BuyLand { land: land(1) },
                ChargeDomain::Property,
                true,
                false,
            ),
            (
                ChargeKind::Upgrade { land: land(1) },
                ChargeDomain::Property,
                true,
                false,
            ),
            (ChargeKind::Penalty, ChargeDomain::Penalty, true, true),
            (ChargeKind::Bonus, ChargeDomain::Bonus, true, false),
            (ChargeKind::God, ChargeDomain::God, false, false),
        ];
        for (kind, domain, scales, can_shift) in rows {
            assert_eq!(charge_domain_of(kind), domain, "{kind:?} 归错域");
            assert_eq!(
                scales_with_price(kind),
                scales,
                "{kind:?} 受不受物价缩放说反了"
            );
            assert_eq!(transferable(kind), can_shift, "{kind:?} 能不能转嫁说反了");
            assert!(
                Charge::new(kind, None, None, 0).cancellable,
                "{kind:?} 的抵消位默认该开着（关掉它的是免疫方，不是种类）"
            );
        }
    }

    /// PLAN §三 点名的豁免：神明金额。物价 3.0 时它一分不变，其余五类统统翻三倍。
    #[test]
    fn price_index_scales_everything_except_god_amounts() {
        let none = Corrections::default();
        let god = Charge::new(ChargeKind::God, Some(player(1)), None, 5_000);
        assert_eq!(stage(god, 3.0, &none).amount, 5_000, "神明金额不该被缩放");
        for kind in [
            ChargeKind::Rent { land: land(1) },
            ChargeKind::BuyLand { land: land(1) },
            ChargeKind::Upgrade { land: land(1) },
            ChargeKind::Penalty,
            ChargeKind::Bonus,
        ] {
            let charge = Charge::new(kind, Some(player(1)), None, 5_000);
            assert_eq!(
                stage(charge, 3.0, &none).amount,
                15_000,
                "{kind:?} 该被缩放"
            );
        }
    }

    /// 指数 1.0 时管线必须是**透明的** —— 这是「物价 1.0 时既有测试逐条不变」那条
    /// 验收在纯函数侧的形状。
    #[test]
    fn an_index_of_one_changes_nothing() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(2),
            scope: Scope::Any,
            adjustment: Adjustment::Scale { by: 2.0 },
            source: Source::God,
        });
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 2_000);
        assert_eq!(stage(charge, 1.0, &corrections).amount, 2_000);
    }

    /// 两个位是真的门，不是装饰：位没开，登记了 `Cancel` 也吃不动这笔账。
    #[test]
    fn the_two_bits_gate_the_adjustments() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(1),
            scope: Scope::Any,
            adjustment: Adjustment::Cancel,
            source: Source::Card,
        });
        let mut charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 800);
        charge.cancellable = false;
        assert_eq!(
            stage(charge, 1.0, &corrections).amount,
            800,
            "免疫之后免罪卡不该生效"
        );

        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(1),
            scope: Scope::Any,
            adjustment: Adjustment::TransferTo { to: player(2) },
            source: Source::Card,
        });
        let mut charge = Charge::new(
            ChargeKind::BuyLand { land: land(3) },
            Some(player(1)),
            None,
            1_500,
        );
        charge.transferable = false;
        assert_eq!(stage(charge, 1.0, &corrections).payer, Some(player(1)));
    }

    /// 转嫁默认是关的：买地不能让别人替你买。
    #[test]
    fn only_rent_and_fines_can_be_shifted() {
        assert!(transferable(ChargeKind::Rent { land: land(1) }));
        assert!(transferable(ChargeKind::Penalty));
        assert!(!transferable(ChargeKind::BuyLand { land: land(1) }));
        assert!(!transferable(ChargeKind::Bonus));
    }

    /// `base` 不动、`amount` 每次从 `base` 重算 —— 报价与成交之间隔几帧都是同一个数。
    #[test]
    fn staging_is_idempotent_because_it_reads_base() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(1),
            scope: Scope::Domain(ChargeDomain::Property),
            adjustment: Adjustment::Scale { by: 2.0 },
            source: Source::God,
        });
        let charge = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            Some(player(2)),
            5_000,
        );
        let once = stage(charge, 2.0, &corrections);
        let twice = stage(once, 2.0, &corrections);
        assert_eq!(once.amount, twice.amount, "重复过链把金额又乘了一遍");
        assert_eq!(twice.base, 5_000);
        assert_eq!(twice.amount, 20_000);
    }

    /// 域筛得不对，修正就会误伤：只免地产钱的卡不该把罚款也勾掉。
    #[test]
    fn scope_filters_by_domain() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(1),
            scope: Scope::Domain(ChargeDomain::Property),
            adjustment: Adjustment::Cancel,
            source: Source::Card,
        });
        let rent = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            Some(player(2)),
            5_000,
        );
        let fine = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        assert_eq!(stage(rent, 1.0, &corrections).amount, 0);
        assert_eq!(stage(fine, 1.0, &corrections).amount, 500);
    }

    /// 别人的修正吃不到我的账。
    #[test]
    fn corrections_only_hit_their_bearer() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(9),
            scope: Scope::Any,
            adjustment: Adjustment::Cancel,
            source: Source::Card,
        });
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        assert_eq!(stage(charge, 1.0, &corrections).amount, 500);
        assert_eq!(corrections.len(), 1);
        assert!(!corrections.is_empty());
    }

    /// 纯收入（payer 是系统）时 bearer 落在收钱的一方，否则衰神「得钱作废」没有落点。
    #[test]
    fn a_payout_bears_on_its_receiver() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(2),
            scope: Scope::Any,
            adjustment: Adjustment::Cancel,
            source: Source::God,
        });
        let bonus = Charge::new(ChargeKind::Bonus, None, Some(player(2)), 3_000);
        assert_eq!(bonus.bearer(), Some(player(2)));
        assert_eq!(stage(bonus, 1.0, &corrections).amount, 0);
    }

    /// 减到底不穿负：`Flat` 的减法不该凭空造出一笔收入。
    #[test]
    fn flat_never_goes_below_zero() {
        let mut corrections = Corrections::default();
        corrections.register(Correction {
            bearer: player(1),
            scope: Scope::Any,
            adjustment: Adjustment::Flat { delta: -10_000 },
            source: Source::Rule,
        });
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        assert_eq!(stage(charge, 1.0, &corrections).amount, 0);
    }
}
