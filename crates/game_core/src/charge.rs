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
    /// 服务收费（P10①）：商业用地的事件费与公司企业的服务费，原型见 [`crate::service`]。
    /// 不挂 `land`：收款方是地主（购物中心、旅馆、加油站）或董事长（航空、IBM），
    /// 但「公司的地是哪块」没有答案，挂了 `land` 就得编一个假的。
    /// 归 [`ChargeDomain::Rent`] 是 `Rent` 行注释里那句话兑现 —— 免费卡、大财神、
    /// 嫁祸卡、物价指数四条既有规则**不改一行**就吃得到它。
    Service,
    /// 买地地价（收款方是市场，即 `payee: None`）
    BuyLand { land: LandId },
    /// 加盖一级的建筑费
    Upgrade { land: LandId },
    /// 建商业设施（P10c）：空商业地上立起购物中心／旅馆／加油站那一族。
    /// 与 [`Upgrade`] 分开不是造价算法需要（造价都是 `upgrade_cost`），是**语义**需要：
    /// 楼是「往上盖一级」，设施是「这块地上出现一个新东西」——将来设施造价或特权
    /// 与楼分开调时，不用回来拆这条表。归 [`ChargeDomain::Acquire`]（买的是资产）。
    Build { land: LandId },
    /// 罚款：卡牌罚金、命运损失、交保释金
    Penalty,
    /// 按「今天的数」抽的一成税（P11b② 的新闻）。它与 [`Self::Penalty`] 分开一格，
    /// 不为语义而为**那一条表**：罚金的名义数写在条目上、要随物价乘，而税的基数是从世界里
    /// 现读的（他手上的现金、名下地块的现值），再乘一次指数就成了「物价 2 倍时税率翻倍」。
    /// 归域仍然归 [`ChargeDomain::Penalty`] —— 免费卡免得掉它、嫁祸卡转得出去，
    /// 这两条与罚金同一条道理，不该为它另开一族修正。
    Levy,
    /// 奖金：横财、小游戏得分、乐透
    Bonus,
    /// 神明造成的收支（P6 登记）。物价指数点名豁免这一类 —— 豁免要能写在表上才算规则，
    /// 写在注释里就只能靠人记得
    God,
}

/// 「这类钱归哪个域」。修正的作用域按域筛，不按 `ChargeKind` 筛 ——
/// kind 带着地块号，不能当键用。
///
/// **过路费与「取得不动产」是两个域**，虽然它们都是钱与地的关系：4 代里「踩别人地不付钱」
/// 与「买地免费」是两张不同的卡（免费卡 / 大福神），P10 的董事长特权又只吃前者。
/// 把它们并成一个 `Property` 的话，第一张想要其中一半的规则就得回来拆这张表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChargeDomain {
    /// 踩到别人地付的那笔 —— 「过路费」这一族，P10 的设施收费也归这里
    Rent,
    /// 买地、加盖：钱换的是**资产**，不是免掉的债
    Acquire,
    Penalty,
    Bonus,
    God,
}

/// kind → 域。**不许出现 `_`**。
pub const fn charge_domain_of(kind: ChargeKind) -> ChargeDomain {
    match kind {
        ChargeKind::Rent { .. } | ChargeKind::Service => ChargeDomain::Rent,
        ChargeKind::BuyLand { .. } | ChargeKind::Upgrade { .. } | ChargeKind::Build { .. } => {
            ChargeDomain::Acquire
        }
        ChargeKind::Penalty | ChargeKind::Levy => ChargeDomain::Penalty,
        ChargeKind::Bonus => ChargeDomain::Bonus,
        ChargeKind::God => ChargeDomain::God,
    }
}

/// 物价指数缩不缩这一类钱（PLAN §三「物价指数」行）。**不许出现 `_`**。
pub const fn scales_with_price(kind: ChargeKind) -> bool {
    match kind {
        // 地价一族三个都缩。升级费不缩的话，物价 5 倍时加盖等于拿 1980 年的价盖楼
        ChargeKind::Rent { .. }
        | ChargeKind::Service
        | ChargeKind::BuyLand { .. }
        | ChargeKind::Upgrade { .. }
        | ChargeKind::Build { .. } => true,
        ChargeKind::Penalty | ChargeKind::Bonus => true,
        // 基数是从世界里**现读**的（他手上的现金、名下地块的现值），今天的价格已经在数里了。
        // 再乘一次指数等于物价 2 倍时把「10% 所得税」悄悄改成 20% —— 税率是新闻写死的，
        // 不该被另一条规则放大。神明那一格同理不缩，但那是「固定数」的理由，这一格是
        // 「已经缩过一遍」的理由，两句不同
        ChargeKind::Levy => false,
        // PLAN §三 点名豁免神明金额：那是玩家记忆里的「固定数」，缩它等于改神明的剧本
        ChargeKind::God => false,
    }
}

/// 允不允许转嫁给别人（嫁祸卡读这一位）。
///
/// ✅ P13b① 把这张表**删掉了**，读数搬到 [`crate::passive`] 那一张事件表上：
/// 「可免／可转嫁」原来是两条互不相干的链（钱看这里、事看 `afflict::forgivable`），
/// 而 10-27 实报的那一格 —— 同一个 `DetentionPlace::Away`，命运牌的出国可转、
/// 踩联合航空的不可转 —— 两张表都答不了，因为它们的键一个是「一笔账」、一个是「一族事」，
/// 而这一格要的是「哪一个事件」。
///
/// 于是这里只留一句**派生**：一种钱归哪一行，那一行答什么。归行见
/// [`PassiveEvent::of_charge`](crate::passive::PassiveEvent::of_charge)。
pub const fn transferable(kind: ChargeKind) -> bool {
    crate::passive::PassiveEvent::of_charge(kind).shiftable()
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
    /// 只吃列出来的那几类。留着复数这一格，是因为「一张卡管两族钱」确实存在 ——
    /// 免费卡管的是**租金与罚金**，两者不同域，而它显然是同一张卡：登记成两条的话，
    /// 一次生效就烧掉两张。
    ///
    /// 切片必须是 `&'static`（`Correction` 要 `Copy`），所以这一格只能吃**写下来的表**，
    /// 运行时算出来的单个域请走 [`Scope::Domain`] —— 那不是妥协，是同一件事的两种来源。
    Domains(&'static [ChargeDomain]),
}

impl Scope {
    pub const fn domains(list: &'static [ChargeDomain]) -> Self {
        Self::Domains(list)
    }

    pub fn matches(self, kind: ChargeKind) -> bool {
        match self {
            Scope::Any => true,
            Scope::Domain(domain) => charge_domain_of(kind) == domain,
            Scope::Domains(list) => list.contains(&charge_domain_of(kind)),
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
    /// **门槛（名义额）**：过完物价的实付金额超过 `floor × 物价` 才生效。
    /// 0 = 没有门槛（今天只有免费卡用这一位，而它写的是 2000）。
    ///
    /// 门槛跟着物价走，是为了让「值不值得免」这件事不随通胀贬值：钉死 2000 的话，
    /// 物价 5 倍时这张卡连一格空地都免得掉，那就成了永久免费券。
    pub floor: i64,
    /// 生效一次就消失。`true` 时 [`stage_and_spend`] 会回报它的下标，
    /// 由**真正扣款的那条路**（[`crate::settle::Books::settle`]）注销它。
    ///
    /// 一次性消耗为什么不做成「登记时就知道到期」：免费卡打出之后**什么时候**用掉
    /// 取决于哪笔账先来，那是世界的行为而不是登记时的信息。所以寿命规则留在这里，
    /// 而「到此为止」这个动作由管线回报。
    pub one_shot: bool,
}

impl Correction {
    /// 常开的一条：寿命由它的来源管（神明到期时那边清场），管线不会主动注销它。
    pub const fn always(
        bearer: Entity,
        scope: Scope,
        adjustment: Adjustment,
        source: Source,
    ) -> Self {
        Self {
            bearer,
            scope,
            adjustment,
            source,
            floor: 0,
            one_shot: false,
        }
    }

    /// 一次性的一条，带门槛（名义额）。两个构造函数分开放，是因为「这条什么时候消失」
    /// 是登记方必须说清的事，而六字段的字面量会让每个调用点各自决定一遍。
    pub const fn one_shot(
        bearer: Entity,
        scope: Scope,
        adjustment: Adjustment,
        source: Source,
        floor: i64,
    ) -> Self {
        Self {
            bearer,
            scope,
            adjustment,
            source,
            floor,
            one_shot: true,
        }
    }
}

/// 修正登记处。P3 的卡、P6 的神明往这里 push，管线本身不动。
///
/// 这里**没有**「按名字撤销某条」的接口，是有意的：一条修正的寿命由它的来源管（神明到期即
/// 清场），那里才知道「什么时候该消失」。唯一的例外是 [`Self::spend`] —— 一次性消耗什么时候
/// 算用掉，只有管线知道，所以那条路必须由管线回报。
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

    /// 全表带下标。下标是给 [`stage_and_spend`] 用的（它要能回报「哪一条被花掉了」），
    /// 调试面板那一栏只用条目本身。
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Correction)> + '_ {
        self.items.iter().enumerate()
    }

    /// 注销 [`stage_and_spend`] 回报的那几条，返回 `(原来的下标, 条目)`，按下标升序。
    /// 把下标一起交回去是为了让调用方能按号找到对应的那笔「省下多少」，而不必赌两边长度一致。
    ///
    /// 倒序删：从前头删会把后面所有下标都挪一位，而那种错不崩、只是少花一张卡 —— 正是最难查的那类。
    /// 越界的下标直接跳过，因为调用方给的是**同一帧同一次回报**里的下标，越界意味着有人在
    /// 中间又删过一轮，那件事值得用「没删掉」暴露出来，不值得为此 panic 一局对局。
    pub fn spend(&mut self, at: &[usize]) -> Vec<(usize, Correction)> {
        let mut holes = at.to_vec();
        holes.sort_unstable();
        holes.dedup();
        let mut taken: Vec<(usize, Correction)> = Vec::with_capacity(holes.len());
        for at in holes.into_iter().rev() {
            if at < self.items.len() {
                taken.push((at, self.items.remove(at)));
            }
        }
        taken.reverse();
        taken
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// 撤销某个人身上、由某个来源给的全部修正。**返回撤掉了几条**。
    ///
    /// 本文件头说「没有按名字撤销某条的接口，是有意的 —— 寿命由来源管」，这一句是它的落地：
    /// 来源知道什么时候该消失（神到期、被顶走、被送神符送走），所以它必须有一个出口，
    /// 否则「附身七天、第八天租金还是双倍」这种错没有任何地方能拦住。
    /// 键是 `(bearer, source)` 而不是条目 id：同一条链上给一个人的几条修正**寿命相同**
    /// （都是那位神在场的那几天），而 id 会逼来源自己记账 —— 那正是这张表不想有的第二份真相。
    ///
    /// 顺序保持：`retain` 不重排，所以剩下那些条目的下标只会整体前移，
    /// 而 [`stage_and_spend`] 的下标是**同一次调用内**算的，跨调用没有意义。
    pub fn release(&mut self, bearer: Entity, source: Source) -> usize {
        let before = self.items.len();
        self.items
            .retain(|c| !(c.bearer == bearer && c.source == source));
        before - self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// 一条一次性修正**当真的生效了**：`at` 是它在这张表里的下标，`saved` 是 bearer 因此
/// 少付（或少收）多少。`saved` 是给流水用的，不是给算账用的 —— 金额已经落在 `charge.amount` 上了。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Spent {
    pub at: usize,
    pub saved: i64,
}

/// 过一遍链：先物价指数（世界的状态，永远在），再按登记顺序应用各条修正。**只问金额**。
///
/// 报价侧（[`crate::settle::Books::quoted`]）走这一条而不是 [`stage_and_spend`]：它拿不到下标，
/// 于是**结构上不可能**在报一次价的时候把玩家手里那张卡花掉。P3a 给免罪卡立的同一条纪律，
/// 在这里由返回类型而不是由注释保证。
pub fn stage(charge: Charge, price_index: f32, corrections: &Corrections) -> Charge {
    stage_and_spend(charge, price_index, corrections).0
}

/// 同一条链，外加回报「这一笔花掉了哪几条一次性修正」。
///
/// 顺序是**规则**不是实现细节：物价 2.5 × 穷神加倍 = 5 倍，反过来先加倍再乘物价是同一个数，
/// 但「先抵消再乘」和「先乘再抵消」不是 —— 这里选前者（`Cancel` 之后 `amount` 归零，
/// 后面的系数乘不动零）。
///
/// 转嫁之后**不再走一遍新付款人的修正**：A 的嫁祸会被 B 的免罪吃掉，那是另一条规则，
/// P3 做卡的时候再拍，别在这儿默认。
pub fn stage_and_spend(
    mut charge: Charge,
    price_index: f32,
    corrections: &Corrections,
) -> (Charge, Vec<Spent>) {
    charge.amount = if scales_with_price(charge.kind) {
        scale(charge.base, price_index as f64)
    } else {
        charge.base
    };
    let Some(bearer) = charge.bearer() else {
        return (charge, Vec::new());
    };
    let mut spent = Vec::new();
    for (at, correction) in corrections.iter() {
        if correction.bearer != bearer || !correction.scope.matches(charge.kind) {
            continue;
        }
        // 门槛读的是**过完物价之后**的实付金额，而门槛自己也要乘物价，否则通胀会把这张卡
        // 稀释成永久免费券（见 `Correction::floor`）
        if correction.floor > 0 && charge.amount <= scale(correction.floor, price_index as f64) {
            continue;
        }
        let before = charge.amount;
        // 「生效了」必须是一个算出来的事实，而不是「这条修正被走到了」：两张免费卡挂在同一个人
        // 身上，一笔 3000 的租金只该烧掉一张 —— 第二张走到这里时金额已经是 0，什么都没改变，
        // 于是它没被花掉。
        let mut fired = false;
        match correction.adjustment {
            Adjustment::Cancel if charge.cancellable && charge.amount > 0 => {
                charge.amount = 0;
                fired = true;
            }
            Adjustment::Scale { by } => {
                charge.amount = scale(charge.amount, by);
                fired = charge.amount != before;
            }
            Adjustment::Flat { delta } => {
                charge.amount = (charge.amount + delta).max(0);
                fired = charge.amount != before;
            }
            Adjustment::TransferTo { to }
                if charge.transferable && charge.payer.is_some() && to != bearer =>
            {
                charge.payer = Some(to);
                fired = true;
            }
            // 位没开、或金额本来就是 0 的那几条：什么都不做。写成显式分支是为了让
            // 「为什么没生效」这件事留在代码里，而不是消失在 match 的空白处
            Adjustment::Cancel | Adjustment::TransferTo { .. } => {}
        }
        if correction.one_shot && fired {
            // 转嫁那一支改的是 `payer` 不是 `amount`，所以它的「省下」是整笔
            let saved = if matches!(correction.adjustment, Adjustment::TransferTo { .. }) {
                before
            } else {
                before - charge.amount
            };
            spent.push(Spent { at, saved });
        }
    }
    (charge, spent)
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
        let rows: [(ChargeKind, ChargeDomain, bool, bool); 8] = [
            (
                ChargeKind::Rent { land: land(1) },
                ChargeDomain::Rent,
                true,
                true,
            ),
            (ChargeKind::Service, ChargeDomain::Rent, true, true),
            (
                ChargeKind::BuyLand { land: land(1) },
                ChargeDomain::Acquire,
                true,
                false,
            ),
            (
                ChargeKind::Upgrade { land: land(1) },
                ChargeDomain::Acquire,
                true,
                false,
            ),
            (
                ChargeKind::Build { land: land(1) },
                ChargeDomain::Acquire,
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
        corrections.register(Correction::always(
            player(2),
            Scope::Any,
            Adjustment::Scale { by: 2.0 },
            Source::God,
        ));
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 2_000);
        assert_eq!(stage(charge, 1.0, &corrections).amount, 2_000);
    }

    /// 两个位是真的门，不是装饰：位没开，登记了 `Cancel` 也吃不动这笔账。
    #[test]
    fn the_two_bits_gate_the_adjustments() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(1),
            Scope::Any,
            Adjustment::Cancel,
            Source::Card,
        ));
        let mut charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 800);
        charge.cancellable = false;
        assert_eq!(
            stage(charge, 1.0, &corrections).amount,
            800,
            "免疫之后免罪卡不该生效"
        );

        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(1),
            Scope::Any,
            Adjustment::TransferTo { to: player(2) },
            Source::Card,
        ));
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
        corrections.register(Correction::always(
            player(1),
            Scope::domains(&[ChargeDomain::Rent]),
            Adjustment::Scale { by: 2.0 },
            Source::God,
        ));
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

    /// 域筛得不对，修正就会误伤：只免过路费的卡不该把买地钱与罚款也勾掉。
    /// （`Rent` 与 `Acquire` 分成两个域就是为了这一条能写出来 —— 免费卡与大福神
    /// 在 4 代里是两张不同的卡，合成一个 `Property` 就没法只说中其中一张。）
    ///
    /// 两种写法各测一遍：`Domain` 是运行时算出来的单个域，`Domains` 是写下来的表。
    #[test]
    fn scope_filters_by_domain() {
        for scope in [
            Scope::Domain(ChargeDomain::Rent),
            Scope::domains(&[ChargeDomain::Rent]),
        ] {
            let mut corrections = Corrections::default();
            corrections.register(Correction::always(
                player(1),
                scope,
                Adjustment::Cancel,
                Source::Card,
            ));
            let rent = Charge::new(
                ChargeKind::Rent { land: land(1) },
                Some(player(1)),
                Some(player(2)),
                5_000,
            );
            let buy = Charge::new(
                ChargeKind::BuyLand { land: land(1) },
                Some(player(1)),
                None,
                5_000,
            );
            let fine = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
            assert_eq!(stage(rent, 1.0, &corrections).amount, 0, "{scope:?}");
            assert_eq!(stage(buy, 1.0, &corrections).amount, 5_000, "{scope:?}");
            assert_eq!(stage(fine, 1.0, &corrections).amount, 500, "{scope:?}");
        }
    }

    /// 免费卡那一族「两域一张卡」的形状：一次生效只烧一条登记。
    #[test]
    fn a_multi_domain_scope_covers_both_family_without_two_rows() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::one_shot(
            player(1),
            Scope::domains(&[ChargeDomain::Rent, ChargeDomain::Penalty]),
            Adjustment::Cancel,
            Source::Card,
            0,
        ));
        let rent = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            Some(player(2)),
            5_000,
        );
        let fine = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        let (rent, spent_a) = stage_and_spend(rent, 1.0, &corrections);
        let (fine, spent_b) = stage_and_spend(fine, 1.0, &corrections);
        assert_eq!((rent.amount, fine.amount), (0, 0));
        assert_eq!((spent_a.len(), spent_b.len()), (1, 1), "两笔各花掉一次");
    }

    /// 别人的修正吃不到我的账。
    #[test]
    fn corrections_only_hit_their_bearer() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(9),
            Scope::Any,
            Adjustment::Cancel,
            Source::Card,
        ));
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        assert_eq!(stage(charge, 1.0, &corrections).amount, 500);
        assert_eq!(corrections.len(), 1);
        assert!(!corrections.is_empty());
    }

    /// 纯收入（payer 是系统）时 bearer 落在收钱的一方，否则衰神「得钱作废」没有落点。
    #[test]
    fn a_payout_bears_on_its_receiver() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(2),
            Scope::Any,
            Adjustment::Cancel,
            Source::God,
        ));
        let bonus = Charge::new(ChargeKind::Bonus, None, Some(player(2)), 3_000);
        assert_eq!(bonus.bearer(), Some(player(2)));
        assert_eq!(stage(bonus, 1.0, &corrections).amount, 0);
    }

    /// 减到底不穿负：`Flat` 的减法不该凭空造出一笔收入。
    #[test]
    fn flat_never_goes_below_zero() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(1),
            Scope::Any,
            Adjustment::Flat { delta: -10_000 },
            Source::Rule,
        ));
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        assert_eq!(stage(charge, 1.0, &corrections).amount, 0);
    }

    /// 免费卡的门槛：不到就不生效，也就**不烧卡**。这一位是这张卡的全部玩法 ——
    /// 没有门槛的话它是一张「以后所有账一律免」的券，那不该是 40 点券的东西。
    #[test]
    fn a_floor_too_high_for_the_bill_leaves_the_card_unspent() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::one_shot(
            player(1),
            Scope::Any,
            Adjustment::Cancel,
            Source::Card,
            2_000,
        ));
        let small = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 1_500);
        let (small, spent) = stage_and_spend(small, 1.0, &corrections);
        assert_eq!(small.amount, 1_500, "没到门槛的账不该被免掉");
        assert!(spent.is_empty(), "没生效就不该花掉那张卡");

        let big = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 2_400);
        let (big, spent) = stage_and_spend(big, 1.0, &corrections);
        assert_eq!(big.amount, 0);
        assert_eq!(
            spent,
            vec![Spent {
                at: 0,
                saved: 2_400
            }]
        );
    }

    /// 门槛跟着物价走（PLAN §三 那句「>2000×物价」）。物价 2.0 时 3000 的账**仍然不免**：
    /// 名义 1500 × 2 = 3000 而门槛也是 2000 × 2 = 4000。钉死门槛的话，通胀会把这张卡
    /// 稀释成一张永久免费券。
    #[test]
    fn the_floor_scales_with_the_price_index() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::one_shot(
            player(1),
            Scope::Any,
            Adjustment::Cancel,
            Source::Card,
            2_000,
        ));
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 1_500);
        assert_eq!(stage_and_spend(charge, 2.0, &corrections).1, Vec::new());
        let bigger = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 2_100);
        assert_eq!(stage_and_spend(bigger, 2.0, &corrections).0.amount, 0);
    }

    /// 两张卡挂在同一个人身上，一笔账只该烧掉一张。第二张走到这里时金额已经是 0，
    /// 「什么都没改变」就是它没生效 —— 这条判据不是细节，是「少花一张卡」与「多花一张卡」的区别。
    #[test]
    fn one_bill_burns_exactly_one_of_two_stacked_cards() {
        let mut corrections = Corrections::default();
        for _ in 0..2 {
            corrections.register(Correction::one_shot(
                player(1),
                Scope::Any,
                Adjustment::Cancel,
                Source::Card,
                0,
            ));
        }
        let charge = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 900);
        let (charge, spent) = stage_and_spend(charge, 1.0, &corrections);
        assert_eq!(charge.amount, 0);
        assert_eq!(spent.len(), 1, "一笔账烧了两张卡");
        let left = corrections.spend(&spent.iter().map(|s| s.at).collect::<Vec<_>>());
        assert_eq!(left.len(), 1);
        assert_eq!(corrections.len(), 1, "注销之后该剩一张");
    }

    /// 报价侧走 [`stage`]：它连下标都拿不到，所以**结构上**不可能在报一次价的时候花掉玩家的卡。
    /// 这条是 P3a 给免罪卡立的纪律在管线签名上的形状。
    #[test]
    fn quoting_never_spends_a_one_shot() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::one_shot(
            player(1),
            Scope::Any,
            Adjustment::Cancel,
            Source::Card,
            0,
        ));
        let charge = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            None,
            5_000,
        );
        for _ in 0..3 {
            assert_eq!(stage(charge, 1.0, &corrections).amount, 0);
        }
        assert_eq!(corrections.len(), 1, "报价报了三次，卡还在");
    }

    /// 神明离场要清干净他给的那几条（P6b）。这条测试要防的是两种相反的错：
    /// 少清（第八天租金还是双倍）与清过头（把同一个人手里的免费卡一起撤掉）。
    #[test]
    fn releasing_a_god_takes_its_rows_and_nobody_elses() {
        let mut corrections = Corrections::default();
        // 同一个人身上：一位神给两条（租金加倍 + 奖金作废），一张卡给一条（免付）
        corrections.register(Correction::always(
            player(1),
            Scope::Domain(ChargeDomain::Rent),
            Adjustment::Scale { by: 2.0 },
            Source::God,
        ));
        corrections.register(Correction::always(
            player(1),
            Scope::Domain(ChargeDomain::Bonus),
            Adjustment::Cancel,
            Source::God,
        ));
        corrections.register(Correction::one_shot(
            player(1),
            Scope::Any,
            Adjustment::Cancel,
            Source::Card,
            2_000,
        ));
        // 另一个人身上也挂着一位神，不该被这次撤销碰到
        corrections.register(Correction::always(
            player(2),
            Scope::Domain(ChargeDomain::Rent),
            Adjustment::Scale { by: 2.0 },
            Source::God,
        ));

        assert_eq!(corrections.release(player(1), Source::God), 2);
        assert_eq!(corrections.len(), 2, "只该撤掉那两条神给的");
        assert!(
            corrections
                .for_bearer(player(1))
                .any(|c| c.source == Source::Card),
            "免费卡不是那位神给的，撤销不该顺手带走"
        );
        assert_eq!(
            corrections.for_bearer(player(2)).count(),
            1,
            "别人的神不是他请的"
        );
        // 撤完之后那条链回到「没有这位神」的样子。故意挑一笔**够不着免费卡门槛**的金额：
        // 用 5000 的话那笔会被剩下的那条 `Cancel` 吃掉，读不出「加倍已经不在了」
        let rent = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            None,
            1_500,
        );
        assert_eq!(
            stage(rent, 1.0, &corrections).amount,
            1_500,
            "还在的话该是 3000"
        );
        assert_eq!(
            corrections.release(player(1), Source::God),
            0,
            "重复撤销要说得出「什么都没撤」"
        );
    }
}
