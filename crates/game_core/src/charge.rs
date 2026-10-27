//! 结算修正管线（P2①）：一笔钱在离开钱包之前要过的那条链。
//!
//! 为什么不是「发一条 `ChargeRequested` 消息、各 subscriber 上去改账」：消息是**帧末
//! 批量投递**的（见 [`crate::msg`] 模块头），而租金必须在同一帧内扣完 —— 付不起要当场
//! 判破产，回合机紧接着才知道是往下走还是收摊。所以链是**纯函数**，输入是数据而不是回调：
//! 物价指数与大穷神在管线眼里都是同一张表里的一行。P6 的神明往
//! [`Corrections`] 里登记，谁都不用回来改这条链。
//!
//! 三条纪律：
//! - **只从 `base` 算起**。[`stage`] 的金额永远由原始金额推出来，不在上一次的成果上叠 ——
//!   于是「报价」与「成交」调多少次都是同一个数，重复缩放这件事在结构上不可能发生。
//! - **豁免表是穷尽表，不许出现 `_`**（P1 的 [`crate::landing::domain_of`] 同一个
//!   形状）。新增一种钱而忘了决定它受不受物价缩放，编译不过。
//! - **那一位是「允不允许」而不是「会不会」**：[`Charge::cancellable`] 关掉，任何 `Cancel`
//!   都吃不动这笔账 —— 魔法屋的免疫规则改的就是这个位（P13），而不是去注册一条「抵消掉
//!   抵消」的修正。
//!
//! ## ③ 之后：这条链只剩「世界的状态」，玩家手里那三张卡搬进了那道门
//!
//! P3c② 的免费卡当年是登记处里的一条 `Cancel`（带 `one_shot` 与 `floor`），而当年给嫁祸卡
//! 预留的位置是同一条表上的 `Adjustment::TransferTo`。10-27 拍的那句「这一族没有出牌这个
//! 动作，事件那一刻才问」把两者都搬进了 [`crate::passive`] 那道门，于是
//! `Correction::{one_shot, floor}`、`Spent`、`Corrections::spend`、`Adjustment::TransferTo`
//! 与 `Charge::transferable` 一起**删除**：「这张卡什么时候算用掉」从「管线回报」变成
//! 「门里对同一笔只问一次」，而**报价不许花卡**那条纪律仍然由返回类型保证 —— 门问的时候
//! 只报价（[`crate::settle::Books::staged`]，拿不到下标也拿不到扣款权），花的那一刻是玩家
//! 答「用」的那一次。门槛与「免哪几族钱」跟着搬，读数仍然只有一处，只是那一处从管线搬到了门上。
//!
//! 留在链里的只有**不需要问人**的那些修正：神明、地块状态、规则给的特权。祂们不是玩家
//! 手里的一张卡，没有「要不要花」这一问。

use bevy::prelude::*;

use crate::land::LandId;

/// 这笔钱**是什么**。物价豁免表与抵消位都按它查，所以每种钱都必须能落进一类。
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

/// 允不允许转嫁给别人。✅ P13b① 把那张穷尽表**删掉了**，读数搬到 [`crate::passive`]
/// 那一张事件表上：「可免／可转嫁」原来是两条互不相干的链（钱看这里、事看
/// `afflict::forgivable`），而 10-27 实报的那一格 —— 同一个 `DetentionPlace::Away`，
/// 命运牌的出国可转、踩联合航空的不可转 —— 两张表都答不了，因为它们的键一个是「一笔账」、
/// 一个是「一族事」，而这一格要的是「哪一个事件」。
///
/// 于是这里只留一句**派生**：一种钱归哪一行，那一行答什么。归行见
/// [`PassiveEvent::of_charge`](crate::passive::PassiveEvent::of_charge)。
///
/// ⚠️ ③ 之后这一句**只由 [`Charge::new`] 消费**（填成账上那一格）：换人付款这件事
/// 从管线里搬到了那道门上，而门读的是账上那位而不是自己再查一次表 —— 与事那一侧
/// [`Affliction::event`](crate::afflict::Affliction::event) 同一条形状：**要活得比那一帧久的
/// 东西，得把自己那一格的答案带在身上**（它在门的队列里等人回答，而回答那一刻
/// 「这一笔允不允许转」必须还是构造它时那一个答案）。
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
    /// 允不允许换人付。这一格由 [`transferable`] 从事件表派生，而**读它的是被动卡那道门**
    /// （[`crate::passive`]）：管线里那个「换人付」的修正臂 `Adjustment::TransferTo`
    /// 已随 ③ 一起删掉 —— 嫁祸不再是「一条挂在人身上的修正」，而是「门在钱动之前问的一句」
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
    /// 只吃列出来的那几类。留着复数这一格，是因为「一条规则管两族钱」确实存在 ——
    /// 小财神那位给的是**租金与罚金都减半**（`gods::money_corrections`），两个域而同一位神：
    /// 登记成两条的话，面板上那位神就成了两条不相干的修正。
    ///
    /// ③ 之前这里举的例子是免费卡（它管的也是这两族）。那张卡现在走那道门、不进这张表，
    /// 而这一格没有因此变空 —— 一条规则跨两族钱这件事是神明先说出来的。
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
///
/// ⚠️ 这里**没有**「换人付」那一臂了：`Adjustment::TransferTo` 随 P13b③ 删掉 ——
/// 嫁祸卡从「挂在甲身上、把付款人改成乙的一条修正」变成了「门在钱动之前问乙之外的那一句」
/// （见 [`crate::passive`]）。所以这条枚举现在全是**不问人**的修正：抵、乘、加减。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Adjustment {
    /// 整笔抵消。只对 `cancellable` 的账生效。
    Cancel,
    /// 乘一个系数（大穷神过路费加倍 `2.0`、小财神被动支出减半 `0.5`）。
    Scale { by: f64 },
    /// 加/减一个固定额（「过路费 +500」这种）。减到底不会穿到负数。
    Flat { delta: i64 },
}

/// 这条修正是谁给的。唯一的作用是**让调试面板能说出「这 0 元是谁给的」**——
/// 查「钱为什么是这个数」的时候，少了这一栏就得回去读代码。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// 卡片。⚠️ ③ 之后**没有生产者**：被动卡那一族不再登记修正，而是在事件那一刻被那道门
    /// 问一次。这一位留着是因为 `Corrections` 的档是按来源分的（[`Corrections::release`]
    /// 的键就是它），删掉等于替「卡给的持续修正」那一族预先改掉这张表的档
    Card,
    /// 神明附身（P6 起）
    God,
    /// 规则本身（P10 的董事长免费、P12 的保释减免）
    Rule,
}

/// 一条登记好的修正。**都是常开的**：寿命由它的来源管（神明到期时那边清场），
/// 管线不会主动注销任何一条 —— ③ 之前那个「一次性 + 门槛」的形状（`one_shot`/`floor`）
/// 是给免费卡的，它现在住在那道门里（[`crate::passive::GateDesk::holds`] 读手牌，
/// 门槛读 [`GameRules::big_money_floor`](crate::rules::GameRules::big_money_floor)）。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Correction {
    /// 只对这位玩家承担的账生效
    pub bearer: Entity,
    pub scope: Scope,
    pub adjustment: Adjustment,
    pub source: Source,
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
        }
    }
}

/// 修正登记处。P6 的神明往这里 push，管线本身不动。
///
/// 这里**没有**「按名字撤销某条」的接口，是有意的：一条修正的寿命由它的来源管（神明到期即
/// 清场），那里才知道「什么时候该消失」。③ 之前唯一的例外是 `Self::spend`（一次性消耗何时
/// 算用掉只有管线知道），那条例外随一次性修正一起删掉了 —— 现在这张表只有 [`Self::register`]
/// 与 [`Self::release`] 两个动作，玩家手里那三张卡根本不在这里。
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

    /// 全表带下标。**下标只给调试面板编号用**（dump 那一栏要说「第 3 条」）：
    /// ③ 之前它还兼作 [`stage_and_spend`] 回报「哪一条被花掉」的凭证，那个出口随
    /// 一次性修正一起删了 —— 现在没有任何一条路能凭下标注销一条修正，
    /// 寿命完全由来源管（[`Self::release`]）。
    pub fn iter(&self) -> impl Iterator<Item = (usize, &Correction)> + '_ {
        self.items.iter().enumerate()
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
    /// 顺序保持：`retain` 不重排，所以剩下那些条目的下标只会整体前移，而面板那次
    /// [`Self::iter`] 的下标只在**同一次读数内**有意义。
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

/// 过一遍链：先物价指数（世界的状态，永远在），再按登记顺序应用各条修正。**只问金额**。
///
/// 这条链现在**只有一个入口**：它只算数，不动钱、不花任何东西。报价侧
/// （[`crate::settle::Books::quoted`]）与成交侧（[`crate::settle::Books::settle`]）走的是
/// **同一个函数**，所以「模态上写的价」与「扣掉的价」不可能不一致 —— 而成交那一刻多做的
/// 只有「按这个数扣」那一件事。
///
/// ③ 之前这里是两个函数（`stage` 与回报「哪几条一次性修正被花掉」的 `stage_and_spend`），
/// 那次拆分立的纪律是「报价不许花卡」。③ 之后**结构上更硬**：这张表里压根没有一次性条目，
/// 玩家手里那三张卡由 [`crate::passive`] 那道门管 —— 门问的时候也只算数（同一个 `stage`），
/// 花卡的动作发生在玩家答「用」的那一刻，与这条链无关。
///
/// 顺序是**规则**不是实现细节：物价 2.5 × 穷神加倍 = 5 倍，反过来先加倍再乘物价是同一个数，
/// 但「先抵消再乘」和「先乘再抵消」不是 —— 这里选前者（`Cancel` 之后 `amount` 归零，
/// 后面的系数乘不动零）。
pub fn stage(charge: Charge, price_index: f32, corrections: &Corrections) -> Charge {
    let mut charge = charge;
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
            Adjustment::Cancel => {
                // 位没关着而金额本来就是 0 的那些：什么都不做。写成显式的 `if` 而不是臂上的
                // guard，是为了让「为什么没生效」这件事留在代码里，而不是消失在 match 的空白处
                // （③ 之前这一支还要与 `TransferTo` 共用一个「位没开」的兜底臂，那两个位
                // 一起删掉之后，这个 `if` 就是它剩下的形状）
                if charge.cancellable && charge.amount > 0 {
                    charge.amount = 0;
                }
            }
            Adjustment::Scale { by } => charge.amount = scale(charge.amount, by),
            Adjustment::Flat { delta } => charge.amount = (charge.amount + delta).max(0),
        }
    }
    charge
}

/// 乘系数回到整数。四舍五入而不是 floor：物价 2.5 这种半数下，floor 会让相邻两级
/// 租金差出一块钱的毛刺，而玩家在租金表上读不到这一块钱从哪来。
///
/// `pub` 是因为被动卡那道门算**门槛**要用同一句取整（[`crate::passive`]：
/// 「这笔账超没超过 `big_money_floor × 物价`」）。门自己再乘一遍就是第二处取整口径，
/// 而两处取整不同的时候，「差一块钱免不免」这种话永远说不清。
pub fn scale(amount: i64, by: f64) -> i64 {
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

    /// 那一位是真的门，不是装饰：位没开，登记了 `Cancel` 也吃不动这笔账。
    ///
    /// ⚠️ ③ 之前这一条还测第二格（`transferable` 挡住 `Adjustment::TransferTo`）。那一臂
    /// 随嫁祸进门一起删了，而现在 `transferable` 那一位的读者是那道门而不是这条链
    /// —— 它的接线由 `passive::tests::the_charge_carries_its_row_answer` 钉。
    #[test]
    fn the_cancellable_bit_gates_the_adjustment() {
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
            "免疫之后那笔账不该被勾掉"
        );
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

    /// 「一条修正管两族钱」的形状：两族都吃到，而登记处仍然只有一条。
    ///
    /// 这一条今天的主人是小财神（租金与罚金都减半，`gods::money_corrections`）。③ 之前
    /// 举的是免费卡 —— 那张卡现在走那道门、不进这张表，而这条测试要钉的从来不是「谁登记的」，
    /// 是「跨两族若登记成两条，就会有两处各自的一半可以分别改掉」。
    #[test]
    fn a_multi_domain_scope_covers_both_family_without_two_rows() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(1),
            Scope::domains(&[ChargeDomain::Rent, ChargeDomain::Penalty]),
            Adjustment::Scale { by: 0.5 },
            Source::God,
        ));
        let rent = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            Some(player(2)),
            5_000,
        );
        let fine = Charge::new(ChargeKind::Penalty, Some(player(1)), None, 500);
        assert_eq!(stage(rent, 1.0, &corrections).amount, 2_500);
        assert_eq!(stage(fine, 1.0, &corrections).amount, 250);
        assert_eq!(corrections.len(), 1, "两族钱共用一条登记");
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

    /// 这条链是**纯的**：同一笔账过两次是同一个数，而登记处一条不少。
    ///
    /// ③ 之前这一条的形状是「报价侧拿不到下标，所以结构上花不掉卡」。现在这张表里
    /// 压根没有一次性条目，于是这条纪律换成了一个更小的东西：**这条链只有一个函数**，
    /// 报价与成交读的是同一个数（`Books::quoted` 与 `Books::settle` 之间不可能分叉），
    /// 而「不许在问人之前先花卡」由那道门自己管 —— 它报价走 `Books::staged`（`&self`），
    /// 花卡走玩家那句回答。
    #[test]
    fn the_chain_is_pure_so_a_quote_and_a_deal_read_the_same_number() {
        let mut corrections = Corrections::default();
        corrections.register(Correction::always(
            player(1),
            Scope::Any,
            Adjustment::Scale { by: 2.0 },
            Source::God,
        ));
        let charge = Charge::new(
            ChargeKind::Rent { land: land(1) },
            Some(player(1)),
            None,
            5_000,
        );
        for _ in 0..3 {
            assert_eq!(stage(charge, 1.0, &corrections).amount, 10_000);
        }
        assert_eq!(corrections.len(), 1, "过了三次链，登记处一条没少");
    }

    /// 神明离场要清干净他给的那几条（P6b）。这条测试要防的是两种相反的错：
    /// 少清（第八天租金还是双倍）与清过头（把同一个人身上别人给的那条一起撤掉）。
    #[test]
    fn releasing_a_god_takes_its_rows_and_nobody_elses() {
        let mut corrections = Corrections::default();
        // 同一个人身上：一位神给两条（租金加倍 + 奖金作废），另一条不是他给的
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
        // 同一个人身上另外一条**别的来源**的行：域挑的是罚款，吃不到下面要量的那笔租金
        corrections.register(Correction::always(
            player(1),
            Scope::Domain(ChargeDomain::Penalty),
            Adjustment::Cancel,
            Source::Card,
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
            "那一条不是那位神给的，撤销不该顺手带走"
        );
        assert_eq!(
            corrections.for_bearer(player(2)).count(),
            1,
            "别人的神不是他请的"
        );
        // 撤完之后那条链回到「没有这位神」的样子。剩下的两行一条管奖金、一条管罚款，
        // 都吃不到租金 —— 所以这一笔读得出的 1500 说的就是「加倍已经不在了」
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
