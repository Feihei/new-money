//! 神明附身（P6）：路上会自己走动的十三位，**踩到才算上身**。
//!
//! ## 「神在哪」只有两种答案
//!
//! 在某一格上（[`Occupancy`](crate::occupancy::Occupancy) 的 `God(g)` 形态），或者在某个
//! 人身上（[`Possession`]）。神上身那一刻路上那个实体就消失 —— 这不是两份真相，因为两种
//! 形态互斥，而格子占用仍由同一条 [`place`](crate::occupancy::place) 闸门管着
//! （ADR-002 §3 给 P6 预留的就是这条路）。
//!
//! 为什么不复用炸弹那条 `OccupancyAt::Carried`：炸弹带的是**步数**、可以在两人之间转让、
//! 「谁带着它」只是它暂时停的地方；神带的是**天**、不能转让、而且一上身就要往修正登记处
//! 挂一整条链（`Correction.bearer` 要的就是这个人）。做成玩家身上的组件还白送一条结构
//! 保证：**一人一神** —— 一个实体同一个组件类型只能挂一份，所以「他身上有两位神」在类型上
//! 写不出来，不必靠一句「先把旧的摘掉」的判据去守（[`attach`] 里那句只管凭据与对位）。
//!
//! ## 一位神的三样身份，全部写成表
//!
//! [`God::days`]（管几天）与 [`God::rival`]（对位是谁、不同场）是**穷尽 `match`，不许
//! 出现 `_`** —— 与 `landing::domain_of`、`occupancy::trigger_of`、`deck::order_of` 同一条
//! 纪律：加一位神时被迫当场回答「它管几天、它的对位是谁」。
//! [`God::roams`] 只有一条例外（死神不上街），所以它写成一句比较而不是十二行 `match` ——
//! 新加的一位默认会上街，这是这里唯一合理的默认值。
//!
//! ## 天怎么数
//!
//! 与 [`Detention`](crate::player::Detention) 同一族：**数的是他自己的回合**，在他自己的
//! 回合开始那一刻扣一格。于是 PLAN §三 那条验收「坐牢 3 天期间神的 7 天照算 3 天」是自然
//! 结果而不是特例 —— 人被拘留时他的回合照样开始、照样扣。地块上的限时状态（涨价 5 天）数的
//! 是**全场回合**；两种尺的分工从 P3c 起就在：身上的计时跟着人，路上的跟着世界。
//!
//! ## 效果落在哪儿
//!
//! 上身即登记、离场即撤销，两处各只有一个出口（[`attach`] / [`detach`]），而**一次性的那一笔**
//! 也只有 [`bestow`] 一个地方会做 —— 三件事绑成一句，因为「挂上组件却忘了装配效果」不会崩、
//! 也不会红，只会得到一个踩到财神而账上什么都不动的世界。
//!
//! - **常驻的那一半**走 P2 的 [`Corrections`](crate::charge::Corrections)：一张穷尽表
//!   （[`money_corrections`]）说清每位神碰哪几族钱。
//! - **当场的那一半**走账本：给钱、挨个收钱、抢钱、送卡，全在 [`bestow`] 里，
//!   产出凭据 [`GodDeed`](crate::msg::GodDeed)。
//! - **不是钱的那几处**还没装配：衰神的「买地建屋必败」与「丢卡」、大福神的「加盖多盖一层」、
//!   厄运天数的加倍与豁免（P6b②），以及天使／恶魔／土地公的「所经加盖／降级／直接占有」与
//!   死神的「全场损失归你」（P6c）。它们各自的读点等落地时写在自己的函数注里。

use std::collections::HashSet;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::{BoardDef, TileIndex};
use crate::charge::{
    Adjustment, Charge, ChargeDomain, ChargeKind, Correction, Corrections, Scope, Source,
};
use crate::deck::{CardId, Shelf};
use crate::hand::{Hand, card_notice, deal_to};
use crate::msg::{
    CardGained, CardLost, CardNotice, GodAppeared, GodDeed, GodExit, GodLeft, GodPossessed,
    PlayerArrived,
};
use crate::occupancy::{Occupancy, OccupancyAt, OccupancyKind, place};
use crate::player::PlayerId;
use crate::rng::{Rng, four_digits};
use crate::settle::Books;
use crate::world::{ActiveBoard, Gameplay};

/// 开局在路上放几位神（装配那一步，一次放满）。
pub const STARTING_GODS: usize = 2;
/// 路上常驻几位神（不含已经附身在谁身上的）。少了补刷，多了不动，每回合至多补一位。
pub const MAX_ON_BOARD: usize = 2;
/// 一次刷出最多试几格找空位。试满仍没有就这一回合不刷 —— 而不是去拆别人放的路障。
const SPAWN_TRIES: usize = 8;

/// 一位神。`ALL` 的顺序就是**抽取顺序**，所以调整它等于改每一局的刷出结果 ——
/// 改完要重新核对那批按种子断言的用例。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum God {
    BigRich,
    SmallRich,
    BigFortune,
    SmallFortune,
    BigPoor,
    SmallPoor,
    BigDoom,
    SmallDoom,
    Angel,
    Devil,
    LandDeity,
    Death,
}

/// 与某位神争同一路的那一位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rival {
    /// 大小对位（大财神 ⇄ 小财神、天使 ⇄ 恶魔）
    God(God),
    /// 土地公的对位是恶犬：它不是神，但占的是同一路，而且正是「期满接位」的下一任
    Dog,
    /// 没有对位，与谁都能同时在场
    Alone,
}

impl God {
    pub const ALL: [God; 12] = [
        Self::BigRich,
        Self::SmallRich,
        Self::BigFortune,
        Self::SmallFortune,
        Self::BigPoor,
        Self::SmallPoor,
        Self::BigDoom,
        Self::SmallDoom,
        Self::Angel,
        Self::Devil,
        Self::LandDeity,
        Self::Death,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::BigRich => "大财神",
            Self::SmallRich => "小财神",
            Self::BigFortune => "大福神",
            Self::SmallFortune => "小福神",
            Self::BigPoor => "大穷神",
            Self::SmallPoor => "小穷神",
            Self::BigDoom => "大衰神",
            Self::SmallDoom => "小衰神",
            Self::Angel => "天使",
            Self::Devil => "恶魔",
            Self::LandDeity => "土地公",
            Self::Death => "死神",
        }
    }

    /// 附身管几天。PLAN §三：默认 7 天，死神 13 天。
    pub const fn days(self) -> u32 {
        match self {
            Self::Death => 13,
            _ => 7,
        }
    }

    /// 对位：同一路同时只许一位在场（PLAN §三「大的走了才刷小的」）。
    pub const fn rival(self) -> Rival {
        match self {
            Self::BigRich => Rival::God(Self::SmallRich),
            Self::SmallRich => Rival::God(Self::BigRich),
            Self::BigFortune => Rival::God(Self::SmallFortune),
            Self::SmallFortune => Rival::God(Self::BigFortune),
            Self::BigPoor => Rival::God(Self::SmallPoor),
            Self::SmallPoor => Rival::God(Self::BigPoor),
            Self::BigDoom => Rival::God(Self::SmallDoom),
            Self::SmallDoom => Rival::God(Self::BigDoom),
            Self::Angel => Rival::God(Self::Devil),
            Self::Devil => Rival::God(Self::Angel),
            Self::LandDeity => Rival::Dog,
            Self::Death => Rival::Alone,
        }
    }

    /// 会不会自己上街被踩到。**死神不会**：它只由 P13 魔法屋的女巫召给某个人
    /// （PLAN §三「有人投降时由魔法屋女巫问『想让谁附身』」）。把它放进随机池，
    /// 「出门就撞死神」会变成一件玩家只能受着的事，而那条神的设计前提是「有人投降」。
    pub const fn roams(self) -> bool {
        !matches!(self, Self::Death)
    }
}

/// 哪些「位」已经被占着。**每次现读世界算出来的快照**，不是一个长期维护的 resource：
/// 神就那两处（路上、人身上），另存一份账一定过期。
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Presence {
    /// 已经在场的每一位（路上 + 身上）
    pub gods: Vec<God>,
    /// 路上有没有恶犬（土地公的对位）
    pub dog: bool,
}

impl Presence {
    /// 从两半读出来：路上的放置物种类，与每个人身上那一位。
    ///
    /// 签名收迭代器而不是 `Query`，因为装配那一步还没有任何查询可给（那一刻只有
    /// `Commands`），硬要传 `Query` 就得为它造一条假查询 —— 那比传空迭代器假得多。
    pub fn observe(
        on_board: impl Iterator<Item = OccupancyKind>,
        attached: impl Iterator<Item = God>,
    ) -> Self {
        let mut seen = Self::default();
        for kind in on_board {
            match kind {
                OccupancyKind::God(god) => seen.gods.push(god),
                OccupancyKind::Dog => seen.dog = true,
                _ => {}
            }
        }
        seen.gods.extend(attached);
        seen
    }

    pub fn holds(&self, god: God) -> bool {
        self.gods.contains(&god)
    }

    /// 这一位的对位在场吗。
    pub fn rival_taken(&self, god: God) -> bool {
        match god.rival() {
            Rival::God(other) => self.holds(other),
            Rival::Dog => self.dog,
            Rival::Alone => false,
        }
    }

    /// 现在还能刷谁：自己不在场、对位也不在场、而且它愿意自己上街。
    ///
    /// 顺序跟着 `God::ALL`，所以同一个在场集合永远给出同一个池 —— 刷出结果只由那条
    /// 随机子流决定，「同种子同刷出」才有得可断言。
    pub fn pool(&self) -> Vec<God> {
        God::ALL
            .into_iter()
            .filter(|god| god.roams() && !self.holds(*god) && !self.rival_taken(*god))
            .collect()
    }
}

/// 存在即被这位神附身。`days_left` 数的是**他自己的剩余回合**，扣到最后一格那一回合离场
/// （见模块头「天怎么数」）。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Possession {
    pub god: God,
    pub days_left: u32,
}

impl Possession {
    pub fn new(god: God) -> Self {
        Self {
            god,
            days_left: god.days(),
        }
    }
}

/// 从世界读在场快照（system 侧的薄封装，判据都在 [`Presence::observe`]）。
pub fn presence_of(
    occupancies: &Query<(Entity, &Occupancy)>,
    possessions: &Query<&Possession, With<PlayerId>>,
) -> Presence {
    Presence::observe(
        occupancies.iter().map(|(_, occupancy)| occupancy.kind),
        possessions.iter().map(|held| held.god),
    )
}

// ===== 效果装配（P6b） =====

/// 这位神挂在修正登记处的那几条。**穷尽表，不许出现 `_`** —— 加一位神必须当场回答
/// 「它改不改钱、改哪族钱」（与 `landing::domain_of`、`occupancy::trigger_of` 同一条纪律）。
///
/// 返回 `Vec` 而不是 `&'static [Correction]`：`Correction.bearer` 是一个 `Entity`，
/// 只有上身那一刻才知道是谁。
///
/// 四条口径，都是 PLAN §三 神明行那一句的展开：
/// - **「被动支出」＝过路费与罚款两族，不含买地与加盖。** 后两者是主动花出去换资产的钱
///   （`ChargeDomain::Acquire`），小财神不该替它们打折 —— 那族钱归大福神与小福神管。
/// - **「买地免费」按域写，会把加盖费一起免掉。** 管线最细的键是域而不是 kind
///   （见 `Scope` 的注：kind 带地块号，当不了键）。要只免地价就得先给 `Scope` 加一格，
///   而不是在这里加一句 `if`。
/// - **穷神的「过路费加倍」与衰神的「得钱作废」各只碰自己那一族**，交叉不生效：
///   大衰神在场时过路费照付，大穷神在场时奖金照领。
/// - **天使／恶魔／土地公／死神返回空表**：它们改的是房子与归属，不是金额（P6c）。
///   死神那条更不是「挂在 bearer 上」的形状 —— 它要让**别人**的损失算到他头上，
///   得往别人的行里登记，届时单开一句，而不是把这张表改成查两遍。
pub fn money_corrections(god: God, bearer: Entity) -> Vec<Correction> {
    let row = |scope: Scope, adjustment: Adjustment| {
        Correction::always(bearer, scope, adjustment, Source::God)
    };
    match god {
        // 大财神：过路费整笔不付（PLAN 那句「可赖账」）。
        // 「踩别人旅馆不付也不住」的**不付**也在这一条里 —— 设施收费归 `Rent` 域（§三 设施行），
        // 而「不住」不是钱，要等 P10 真有旅馆格才有着落
        God::BigRich => vec![row(Scope::Domain(ChargeDomain::Rent), Adjustment::Cancel)],
        // 小财神：被动支出减半（那笔「向每人收三位数」是一次性的，在 [`bestow`] 里）
        God::SmallRich => vec![row(
            Scope::domains(&[ChargeDomain::Rent, ChargeDomain::Penalty]),
            Adjustment::Scale { by: 0.5 },
        )],
        God::BigFortune => vec![row(
            Scope::Domain(ChargeDomain::Acquire),
            Adjustment::Cancel,
        )],
        God::SmallFortune => vec![row(
            Scope::Domain(ChargeDomain::Acquire),
            Adjustment::Scale { by: 0.5 },
        )],
        God::BigPoor => vec![row(
            Scope::Domain(ChargeDomain::Rent),
            Adjustment::Scale { by: 2.0 },
        )],
        God::SmallPoor => vec![row(
            Scope::Domain(ChargeDomain::Rent),
            Adjustment::Scale { by: 1.5 },
        )],
        // 大衰神：罚钱加倍、得钱作废。**两张表**而不是 `Scope::Any` 的一条 ——
        // 它对支出的放大与对收入的抹除是两个不同的数，而它显然是一张神给的两条规则
        God::BigDoom => vec![
            row(
                Scope::Domain(ChargeDomain::Penalty),
                Adjustment::Scale { by: 2.0 },
            ),
            row(Scope::Domain(ChargeDomain::Bonus), Adjustment::Cancel),
        ],
        God::SmallDoom => vec![
            row(
                Scope::Domain(ChargeDomain::Penalty),
                Adjustment::Scale { by: 1.5 },
            ),
            row(
                Scope::Domain(ChargeDomain::Bonus),
                Adjustment::Scale { by: 0.5 },
            ),
        ],
        God::Angel | God::Devil | God::LandDeity | God::Death => Vec::new(),
    }
}

/// 上身即登记。**只有这一处写登记**，所以它先撤销再挂：同一帧里「顶走旧神、挂上新神」
/// 与「调试旋钮把同一位按两次」都必须收敛到同一条链 —— 而 [`Corrections::release`]
/// 键的是 `(bearer, Source::God)`，撤销时不必知道旧的那位给了哪几条。
///
/// 就地写（`&mut Corrections`）而不是 `Commands`：修正要在**同一条落格链**里被下一块结算读到
/// （`Landed` 那一串排在后面的 `settle_property_landing` 就是读者），而资源写入是即时的。
pub fn grant_effects(corrections: &mut Corrections, god: God, bearer: Entity) {
    corrections.release(bearer, Source::God);
    for correction in money_corrections(god, bearer) {
        corrections.register(correction);
    }
}

/// 离场即撤销。返回撤掉几条 —— 调用方一般不用它，那是给「神到期了但租金还是双倍」
/// 这类缺陷留的一个可断言的数。
pub fn revoke_effects(corrections: &mut Corrections, bearer: Entity) -> usize {
    corrections.release(bearer, Source::God)
}

/// 上身那一刻**当场做完的那件事**（给钱／挨个收钱／抢钱／送卡）。
///
/// 与常驻的 [`money_corrections`] 分开是两种寿命：那一张表管「这七天里他的每一笔账」，
/// 这一句只在他踩到神的那一帧做一次。两者都由 [`attach`] 调，所以没有调用点能只装配一半。
///
/// 钱全部走账本而不是直接写 `Wallet`：小财神那笔是**有名目的钱**，被收的人付不起就要当场
/// 破产（[`Books::settle`] 是这类钱的唯一入口，见 settle.rs 模块头）。大财神那笔纯收入
/// 走 [`Books::credit`] —— 它没有付款人，也就没有「付不起」。
fn bestow(god: God, bearer: Entity, desk: &mut GodDesk<'_, '_>) {
    let mut cash = 0i64;
    let mut cards = Vec::new();
    match god {
        // 四位数：千位 1–9、后三位各 0–9 → 1000–9999。逐位取数而不是 `rng(1000..=8999)`
        // 一次取完，是因为表现层的老虎机要逐位停（与春节红包同一台机器），
        // 而 PLAN §三 点名两者「共用同一个生成函数，别各写一套」
        God::BigRich => {
            let leading = 1 + desk.rng.pick_god(0..=8);
            let rest = [0u32; 3].map(|_| desk.rng.pick_god(0..=9));
            let gift = four_digits(leading, rest);
            desk.books.credit(bearer, gift);
            cash = gift;
        }
        God::SmallRich => {
            // 名单先算好再去收：`settle` 会把付不起的人判破产，而那是**在改世界**，
            // 不能让它在一次迭代里改动自己正在读的名单
            let victims: Vec<Entity> = desk
                .books
                .solvent()
                .into_iter()
                .map(|(player, _)| player)
                .filter(|player| *player != bearer)
                .collect();
            for victim in victims {
                // 每人独立抽一位三位数：在场人数不同取数次数就不同，这是神自己那条流上的
                // 事，不该回头去挪骰点（见 `Rng` 的注）
                let base = 100 + desk.rng.pick_god(0..=899) as i64;
                let charge = Charge::new(ChargeKind::God, Some(victim), Some(bearer), base);
                let Ok(done) = desk.books.settle(&mut desk.commands, charge) else {
                    continue;
                };
                cash += done.amount;
            }
        }
        God::BigFortune => cards = deal_god_cards(desk, bearer, god, 2),
        God::SmallFortune => cards = deal_god_cards(desk, bearer, god, 1),
        God::BigPoor => {
            let divisor = i64::from(desk.books.rules.poor_cash_divisor.max(1));
            let loss = desk.books.balance(bearer) / divisor;
            if loss > 0 {
                // 收款方是 `None` = 这笔钱离开游戏（进系统口袋），于是下一回合的物价指数
                // 分子里少掉它 —— 「穷神把钱抢走了」在指数上留得下痕迹，这正是选
                // 「过账本」而不是「`debit` 完事」的理由
                let charge = Charge::new(ChargeKind::God, Some(bearer), None, loss);
                let Ok(done) = desk.books.settle(&mut desk.commands, charge) else {
                    return;
                };
                cash = -done.amount;
            }
        }
        // 小穷神只把过路费抬上去，不当场抢钱：那是「大」的那一半独有的剧本
        // 两位衰神当场什么都不做 —— 他们的钱在 [`money_corrections`] 里，「丢卡」归 P6b②
        God::SmallPoor | God::BigDoom | God::SmallDoom => {}
        God::Angel | God::Devil | God::LandDeity | God::Death => {}
    }
    if cash != 0 || !cards.is_empty() {
        desk.deeds.write(GodDeed {
            player: bearer,
            god,
            cash,
            cards,
        });
    }
}

/// 福神送卡。池是**整副道具卡**而不是百货货架：神不该被「这一趟店里没上这款」限制住
/// （与圣诞礼物同一条理由，见 `calendar_fx::deliver_gifts`）。
/// 上限与「满了挤掉最前面那张」全交给 [`deal_to`]，这里不复制第二条判据。
fn deal_god_cards(
    desk: &mut GodDesk<'_, '_>,
    bearer: Entity,
    god: God,
    want: usize,
) -> Vec<CardNotice> {
    let pool = desk.shelf.items().len() as u32;
    if pool == 0 {
        // 数据缺道具池是一件要说得出来的事，不是「神什么也没给」
        warn!("{} 送卡没有可发的卡：道具池为空", god.label());
        return Vec::new();
    }
    let limit = desk.books.rules.hand_limit;
    let mut given = Vec::new();
    for _ in 0..want {
        let card = CardId(desk.rng.pick_god(0..=pool - 1));
        let Ok(mut hand) = desk.hands.get_mut(bearer) else {
            break;
        };
        deal_to(
            &mut hand,
            limit,
            card,
            bearer,
            &desk.shelf,
            &mut desk.gained,
        );
        given.push(card_notice(&desk.shelf, card));
    }
    given
}

// ===== 不是钱的那几处（P6b②）=====

/// 这位神在场时，落到 bearer 头上的**事**（拘留／停留／转向／乌龟）整件不生效。
///
/// **穷尽表，不许 `_`**，理由与 [`money_corrections`] 同一条：加一位神必须当场回答
/// 「它免不免厄运」。今天只有两句真话（大福神与天使免 —— PLAN §三 神明行，与被绑架那行的
/// 「天使与福神可免」是同一条规则的两个说法），其余八位都是「不免」——
/// 而那八行是**写下来的不免**，不是漏掉。
///
/// 为什么这是一句现读的判据而不是 [`Insulation`](crate::afflict::Insulation) 的一格：
/// 卡给的护身有**次数**、会被吃掉，神给的免疫没有次数、随神一起消失。两者塞进同一个组件
/// 就必然出现「上身覆盖掉卡、离场把卡一起摘走」—— 一个不崩也不会红的丢账。
/// 所以这里只回答「此刻这位神免不免」，那份次数留在它自己的组件里。
pub fn resists(god: God) -> bool {
    match god {
        God::BigFortune | God::Angel => true,
        God::SmallFortune | God::BigRich | God::SmallRich => false,
        God::BigPoor | God::SmallPoor => false,
        God::BigDoom | God::SmallDoom => false,
        // 恶魔与死神在 PLAN §三 那一行里是「延长厄运天数」而不是「免除」，土地公不管事
        God::Devil | God::LandDeity | God::Death => false,
    }
}

/// 这位神在场时，落到 bearer 头上的**厄运天数**乘几。
///
/// 同样是穷尽表。今天只有大衰神一位说话（PLAN §三「衰神＝…厄运天数加倍」），
/// 而 PLAN §三 被绑架那一行还写着「恶魔／死神附身会延长」—— 那两位现在填的是**中性值 1**，
/// 意思是「今天不延长」，不是「没想过」：P6c 落地时必须回到这张表改一个数，
/// 而不是在某个调用点旁边补一句 `if`。小衰神按「小的那一半只动钱」读（与 P6b①
/// 那条「抢钱是大穷神独有」同一个分法）。
pub fn doom_multiplier(god: God) -> u32 {
    match god {
        God::BigDoom => 2,
        God::SmallDoom
        | God::BigFortune
        | God::SmallFortune
        | God::BigRich
        | God::SmallRich
        | God::BigPoor
        | God::SmallPoor
        | God::Angel
        | God::Devil
        | God::LandDeity
        | God::Death => 1,
    }
}

/// 神在不在场由调用方查（这里只回答「那位神说几倍」），乘完向下取整。
///
/// 判 3 变 6、判 1 变 2 —— 与 P4④ 那条「天数只来自卡面」不冲突：卡面那个数是**名义值**，
/// 过的是这条乘法，就像租金过的是 [`crate::charge::stage`]。
pub fn doom_days(god: Option<God>, days: u32) -> u32 {
    days * god.map_or(1, doom_multiplier)
}

/// 这位神在场时，**置产这一族的事办不成**：买地必败、加盖必败（PLAN §三 衰神那一格，
/// 与 P9 那行「衰神／死神附身者买地必流标」是同一条判据的两种场合）。
///
/// 它刻意**不是**一条 `Adjustment::Cancel`。`Cancel` 把金额改成 0，而 0 元成交仍然是成交
/// —— 那是大福神的「买地免费」，正好是相反的那张卡。PLAN §三 设施行那句「三种豁免都只是
/// 同一个 subscriber 把金额改成 0」说的是**付钱义务**的豁免；「事办不成」在这里没有金额可改，
/// 所以它走 [`crate::settle::Books::acquire_denied`] 这一句判据，不进管线。
///
/// 死神在这里就说「是」：PLAN §三 已经写了它要必败，而它今天只能由 P13 的女巫召来 ——
/// 表先说对，召唤那条路来了自然读得到。
pub fn denies_acquire(god: God) -> bool {
    match god {
        God::BigDoom | God::SmallDoom | God::Death => true,
        God::BigRich | God::SmallRich | God::BigFortune | God::SmallFortune => false,
        God::BigPoor | God::SmallPoor | God::Angel | God::Devil | God::LandDeity => false,
    }
}

/// 这位神在场时，一次加盖**多盖几层**（PLAN §三 大福神「加盖多盖一层」）。
///
/// 单位是「层」而不是「倍」：`1 + extra` 之后还要被 `max_building_level` 夹住，
/// 所以封顶那一层上多盖的那一层会少掉 —— 那不是缺陷，4 代也没有「盖穿顶」这件事。
/// 它与 [`denies_acquire`] 是相反的一对，正好落在同一个域上：大福神多盖一层、
/// 大衰神一层都不许盖。两个都写成表而不是在 `Books::upgrade` 里各加一句 `if`。
pub fn extra_levels(god: God) -> u8 {
    match god {
        God::BigFortune => 1,
        God::SmallFortune
        | God::BigRich
        | God::SmallRich
        | God::BigPoor
        | God::SmallPoor
        | God::BigDoom
        | God::SmallDoom
        | God::Angel
        | God::Devil
        | God::LandDeity
        | God::Death => 0,
    }
}

/// 这位神在场时，**每回合从他手里弄走几张卡**（PLAN §三 衰神那一格的第一项）。
///
/// 大小各差一张，与钱那两档（加倍 vs 一成半）同一条分法。其余九位都是 0 —— 写下来而不是
/// 省掉，是为了让「加一位神」时这一格也要被回答一次。
pub fn cards_lost_per_turn(god: God) -> u32 {
    match god {
        God::BigDoom => 2,
        God::SmallDoom => 1,
        God::BigRich | God::SmallRich | God::BigFortune | God::SmallFortune => 0,
        God::BigPoor | God::SmallPoor | God::Angel | God::Devil | God::LandDeity | God::Death => 0,
    }
}

/// 衰神那一回合的抽卡：随机弄走 `cards_lost_per_turn` 张，手里空了就少弄几张。
///
/// 它是**容器上的一个 drain**，不是账（不碰 `Corrections`）也不是一时的事（不挂
/// `Affliction`）：上限、挤档、流水这些规矩全在 `hand.rs`，这里只负责「谁被拿走一张」。
/// 抽哪一张走神明那条子流（[`Rng::pick_god`]）：一位神弄丢几张卡，不该改变下一个骰点
/// 与下一张机会牌是谁。
///
/// 返回真的弄走了几张 —— 给日志与测试用，凭据本身逐张写在 [`CardLost`] 里。
pub fn drain_cards(
    god: God,
    player: Entity,
    hands: &mut Query<&mut Hand, With<PlayerId>>,
    shelf: &Shelf<'_>,
    rng: &mut Rng,
    lost: &mut MessageWriter<CardLost>,
) -> u32 {
    let Ok(mut hand) = hands.get_mut(player) else {
        return 0;
    };
    let mut gone = 0u32;
    for _ in 0..cards_lost_per_turn(god) {
        if hand.is_empty() {
            break;
        }
        let at = rng.pick_god(0..=hand.len() as u32 - 1) as usize;
        let Some(card) = hand.cards.get(at).copied() else {
            break;
        };
        // `spend` 摘的是**第一张**同 id 的那格：手里有两张陷害卡时，弄丢一张就是少一张，
        // 不必再判「丢的到底是第几格」—— 那件事没有读者
        hand.spend(card);
        lost.write(CardLost {
            player,
            card: card_notice(shelf, card),
            by: god,
        });
        gone += 1;
    }
    gone
}

/// 附身与它的一次性效果要动的全部东西。收成参数包是房规（§六 坑 8），
/// 而这几样确实是同一件事：谁上身、挂哪几条修正、当场给／抢／送多少。
///
/// ⚠️ **`corrections` 不单独成字段**：[`Books`] 已经带着 `ResMut<Corrections>`，
/// 这里再要一份会在参数建立期撞成借用冲突（与 P4② 那条同一个坑）。
/// 要写登记处就写 `desk.books.corrections`。
#[derive(SystemParam)]
pub struct GodDesk<'w, 's> {
    pub commands: Commands<'w, 's>,
    pub books: Books<'w, 's>,
    pub rng: ResMut<'w, Rng>,
    pub shelf: Shelf<'w>,
    pub hands: Query<'w, 's, &'static mut Hand, With<PlayerId>>,
    pub possessions: Query<'w, 's, &'static Possession, With<PlayerId>>,
    pub gained: MessageWriter<'w, CardGained>,
    pub deeds: MessageWriter<'w, GodDeed>,
    pub possessed: MessageWriter<'w, GodPossessed>,
    pub left: MessageWriter<'w, GodLeft>,
}

// ===== 上身与离场 =====

/// 一位神离场（到期 / 被顶走 / 被送神符送走）：撤销登记、摘组件、留凭据，三件事一起走。
///
/// 签名收零件而不是收 [`GodDesk`]，是因为它有两个调用方而它们的参数包拿不到同一个 desk：
/// 落脚那条有 desk，回合机那条只有 [`TurnDesk`](crate::turn::TurnDesk) —— 后者读 `Detention`
/// 用 `&Detention`，而 `Books` 里带着 `&mut Detention`（P4② 搬进去的），两者塞进同一条
/// system 会在参数建立期直接撞成借用冲突。所以这条函数的参数按最小需要给，
/// 两处共用同样这三件事。
pub fn detach(
    commands: &mut Commands,
    corrections: &mut Corrections,
    player: Entity,
    god: God,
    reason: GodExit,
    left: &mut MessageWriter<GodLeft>,
) {
    revoke_effects(corrections, player);
    commands.entity(player).remove::<Possession>();
    left.write(GodLeft {
        player,
        god,
        reason,
    });
}

/// 一位神上身。**只有这一个出口**，因为四件事必须同时成立：旧的那位留一条「怎么走的」
/// 凭据、新的挂上、新神的那条修正链登记完、一次性效果当场做完。
///
/// 「挂状态」与「装配效果」合成一句而不是留给调用方两行，是因为后者被忘记时不会崩、
/// 也不会红 —— 只会得到「踩到财神，身上多一位、账上什么都不动」，而那正是最难查的那类缺陷
/// （P3d④b 那次「方向存着但没人读」同形）。
///
/// 用 `Commands` 挂组件而不是就地写：落脚触发一人一回合只可能有一次，同帧没有第二条请求
/// 要看见这一条 —— 与 P3d③ 抵押那次「同帧押两遍钱」不是同一个形状。
/// 修正登记处（[`grant_effects`]）却**必须**就地写：同一串 `Landed` 里排在后面的那块地产
/// 结算就是它的读者。
pub fn attach(desk: &mut GodDesk<'_, '_>, player: Entity, god: God, tile: TileIndex) {
    let replaced = desk.possessions.get(player).ok().map(|held| held.god);
    if let Some(old) = replaced {
        detach(
            &mut desk.commands,
            &mut desk.books.corrections,
            player,
            old,
            GodExit::Replaced,
            &mut desk.left,
        );
    }
    grant_effects(&mut desk.books.corrections, god, player);
    desk.commands.entity(player).insert(Possession::new(god));
    bestow(god, player, desk);
    desk.possessed.write(GodPossessed {
        player,
        god,
        days: god.days(),
        tile,
        replaced,
    });
}

/// 踩到路上的神 → 上身（ADR-002 §5 的「落脚」那一行）。
///
/// 单独一条 `PlayerArrived` 的 reader，而不是塞进宝箱那条：落脚触发的两样东西寿命完全不
/// 同 —— 宝箱给一件道具就消失，神要顶掉旧神、挂一整条修正链、做完一次性效果、腾出一个对位槽。
/// 消息读者各有自己的游标，两条并读同一个 `PlayerArrived` 不会互相吃掉。
///
/// 排在 `settle_property_landing` **之前**（`lib.rs` 的 `Landed` 链）：一位神站在别人名下
/// 那块地上时，「他这七天付双倍」必须从**脚下这一格**就开始生效，而不是从下一格。
pub fn settle_god_landing_system(
    mut desk: GodDesk<'_, '_>,
    mut arrivals: MessageReader<PlayerArrived>,
    occupancies: Query<(Entity, &Occupancy)>,
) {
    for arrival in arrivals.read() {
        let hit = occupancies.iter().find(|(_, occupancy)| {
            occupancy.at == OccupancyAt::OnTile(arrival.to)
                && matches!(occupancy.kind, OccupancyKind::God(_))
        });
        let Some((entity, occupancy)) = hit else {
            continue;
        };
        let OccupancyKind::God(god) = occupancy.kind else {
            continue;
        };
        let player = arrival.player;
        let tile = arrival.to;
        attach(&mut desk, player, god, tile);
        // 它已经变成他身上的那位；路上的实体就此消失，那一格的槽位同时空出来
        desk.commands.entity(entity).despawn();
    }
}

// ===== 天怎么扣 =====

/// 这个人的回合开始：他身上那位神老一天，扣到最后一格就当场离场。
///
/// **必须排在 [`begin_turn_system`](crate::turn::begin_turn_system) 的拘留分支之前**调用，
/// 这样「被关着的人神也照老」不需要第二条判据 —— 那正是 PLAN §三 的验收③。
/// 递减就地写（`&mut Possession`）而不是 `Commands`：同一次回合开始里，扣完的数要立刻是
/// 面板与 dump 读到的那个数；只有「摘掉组件」这一步交给 `Commands`。
///
/// `corrections` 是这条签名上唯一看起来多余的参数，但它挡的是一个不崩也不会红的缺陷：
/// 神走了而它给的那几条还挂着，于是「大穷神七天」变成「终身双倍租金」，而第七天之后的
/// 每一笔账都读得对、只是读的是死神的账。
///
/// 回来的是**扣完之后**他身上还有谁（`None` = 这一回合开始他自由了）。调用方必须读这个
/// 返回值而不是再查一次组件：摘组件那一步走的是 `Commands`，**同一帧查得到旧值** ——
/// 于是「期满那一回合还照常被衰神丢一张卡」会静默地成立。同一条坑 P3d③ 抵押那次踩过
/// （「同帧两条押同一块地都读到还没押」），那次得到的结论是：**能就地写的就地写，
/// 交给 `Commands` 的那一步要把结果自己说出去**。
///
/// 谁走掉了仍然由 [`GodLeft`] 说，恶犬接位那条后果读消息
/// （见 [`land_deity_succession_system`]），这里不留第二条通知通道。
pub fn age_possession(
    commands: &mut Commands,
    corrections: &mut Corrections,
    possessions: &mut Query<&mut Possession, With<PlayerId>>,
    player: Entity,
    left: &mut MessageWriter<GodLeft>,
) -> Option<God> {
    let Ok(mut held) = possessions.get_mut(player) else {
        return None;
    };
    if held.days_left > 1 {
        held.days_left -= 1;
        return Some(held.god);
    }
    let god = held.god;
    detach(commands, corrections, player, god, GodExit::Expired, left);
    None
}

/// 土地公期满 → 恶犬接位（PLAN §三「土地公＝…期满变恶犬」）。
///
/// 读的是 [`GodLeft`] 而不是在 `age_possession` 里顺手放：那一句只管「神走了」，
/// 而接位是它的一条后果，晚一帧成立没有任何规则读得到差别。单独一条 system 还换来
/// 「谁接位」这件事只在这里出现一次 —— P12 的小偷将来要把恶犬搬走，也只改这一处附近。
///
/// 恶犬放在他**此刻站的那一格**。挤不进去（那一格已经有东西）就顺着环往前找第一格空的：
/// 「接位」不该因为路上有个路障就整件消失。
pub fn land_deity_succession_system(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut lefts: MessageReader<GodLeft>,
    positions: Query<&crate::player::TokenPosition, With<PlayerId>>,
    occupancies: Query<(Entity, &Occupancy)>,
) {
    for gone in lefts.read() {
        if gone.god != God::LandDeity || gone.reason != GodExit::Expired {
            continue;
        }
        let Ok(pos) = positions.get(gone.player) else {
            continue;
        };
        let len = active.def.loop_len();
        let mut claimed = HashSet::new();
        for step in 0..len {
            let tile = TileIndex((pos.tile.0 + step) % len);
            let existing = occupancies.iter().map(|(e, o)| (e, o.at));
            if place(
                &mut commands,
                existing,
                &mut claimed,
                OccupancyKind::Dog,
                tile,
            )
            .is_some()
            {
                debug!("土地公期满，恶犬接位于格 {}", tile.0);
                break;
            }
        }
    }
}

// ===== 刷出与漫游 =====

/// 找一格把这位神放下去。走 [`place`] 那道唯一闸门，所以「一格一件」不在这里判第二遍。
/// 起点（0 号）不参与 —— 与开局宝箱同一条理由。
fn place_one(
    commands: &mut Commands,
    def: &BoardDef,
    existing: impl Iterator<Item = (Entity, OccupancyAt)> + Clone,
    claimed: &mut HashSet<TileIndex>,
    rng: &mut Rng,
    god: God,
) -> Option<TileIndex> {
    let span = def.loop_len().saturating_sub(1);
    if span == 0 {
        return None;
    }
    for _ in 0..SPAWN_TRIES {
        let tile = TileIndex(1 + rng.pick_god(0..=span - 1));
        if place(
            commands,
            existing.clone(),
            claimed,
            OccupancyKind::God(god),
            tile,
        )
        .is_some()
        {
            return Some(tile);
        }
    }
    None
}

/// 开局在路上放 [`STARTING_GODS`] 位（装配流程里调，与宝箱同一步）。
///
/// `taken` 是开局已经占着的格（宝箱）。那一刻它们还只是 `Commands` 里的 spawn，查询读不到，
/// 所以由调用方把已经选好的格交给这里 —— 而不是假装读得到。
pub fn spawn_starting_gods(
    commands: &mut Commands,
    def: &BoardDef,
    rng: &mut Rng,
    taken: &[TileIndex],
) -> Vec<(God, TileIndex)> {
    let mut claimed: HashSet<TileIndex> = taken.iter().copied().collect();
    let mut presence = Presence::observe(std::iter::empty(), std::iter::empty());
    let mut placed = Vec::new();
    for _ in 0..STARTING_GODS {
        let pool = presence.pool();
        if pool.is_empty() {
            break;
        }
        let god = pool[rng.pick_god(0..=pool.len() as u32 - 1) as usize];
        let Some(tile) = place_one(commands, def, std::iter::empty(), &mut claimed, rng, god)
        else {
            continue;
        };
        presence.gods.push(god);
        placed.push((god, tile));
    }
    placed
}

/// 每回合至多补一位，把路上凑到 [`MAX_ON_BOARD`] 位。
///
/// 只补一位是刻意的：一次放满会让「神突然多了两位」这种场面出现，而补刷本来是背景事件。
/// 开局那一步（[`spawn_starting_gods`]）是一次放满的，因为那一刻还没有任何玩家在场上晃。
pub fn top_up_gods_system(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut rng: ResMut<Rng>,
    occupancies: Query<(Entity, &Occupancy)>,
    possessions: Query<&Possession, With<PlayerId>>,
    mut appeared: MessageWriter<GodAppeared>,
    mut claimed: Local<HashSet<TileIndex>>,
) {
    claimed.clear();
    if occupancies
        .iter()
        .filter(|(_, o)| matches!(o.kind, OccupancyKind::God(_)))
        .count()
        >= MAX_ON_BOARD
    {
        return;
    }
    let pool = presence_of(&occupancies, &possessions).pool();
    if pool.is_empty() {
        debug!("神明补刷跳过：所有能上街的都到场了，或对位被占");
        return;
    }
    let god = pool[rng.pick_god(0..=pool.len() as u32 - 1) as usize];
    let existing = occupancies.iter().map(|(e, o)| (e, o.at));
    if let Some(tile) = place_one(
        &mut commands,
        &active.def,
        existing,
        &mut claimed,
        &mut rng,
        god,
    ) {
        appeared.write(GodAppeared { god, tile });
    }
}

/// 每回合结束，路上每位神自己挪一格（4 代的神在街上晃，不是钉在原地等踩）。
///
/// 两条边界都来自 ADR-002：
/// - **挪到有人站着的格可以**（「这一格上有人」那条只挡玩家主动放置，§3 明说换格不走它）；
/// - **挪到已有放置物的格不行**（一格一件对谁都一样）。挤不进去就这一回合站在原地。
///
/// **走到玩家那一格不算附身**：附身只由落脚触发。ADR-002 §9 把这半条交给 P6 拍板，这里拍的
/// 是「不算」—— 回合机此刻在 `TurnEnd`，没有任何人正在行动；让「神自己走了一步」去改一个人
/// 身上的状态与整条修正链，等于把附身从「我踩到财神」变成一件世界事件，而 4 代给玩家的
/// 正是前一句。
///
/// 先收集要挪的、再逐个就地写：同一个目标格不能被两位神占着，而「后一位看得见前一位的
/// 新位置」这件事不能靠迭代器与 `get_mut` 的交错顺序来说话 —— 所以每挪完一位就把它的
/// 新格记进 `claimed`，`claimed` 一开始装着所有神的旧格（那等于「不许互相踩位」）。
pub fn wander_gods_system(
    active: Res<ActiveBoard>,
    mut rng: ResMut<Rng>,
    mut occupancies: Query<(Entity, &mut Occupancy), With<Gameplay>>,
) {
    let len = active.def.loop_len();
    if len < 2 {
        return;
    }
    let moving: Vec<(Entity, TileIndex)> = occupancies
        .iter()
        .filter_map(|(entity, occupancy)| match occupancy.kind {
            OccupancyKind::God(_) => occupancy.tile().map(|tile| (entity, tile)),
            _ => None,
        })
        .collect();
    let mut claimed: Vec<TileIndex> = moving.iter().map(|(_, from)| *from).collect();
    for (entity, from) in moving {
        // 一步，方向各半：神比人晃得慢，而 13 格的盘面上一次挪两格就成了巡逻
        let forward = rng.pick_god(0..=1) == 0;
        let to = TileIndex(if forward {
            (from.0 + 1) % len
        } else {
            (from.0 + len - 1) % len
        });
        let blocked = claimed.contains(&to)
            || occupancies
                .iter()
                .any(|(_, o)| o.at == OccupancyAt::OnTile(to));
        claimed.retain(|taken| *taken != from);
        if blocked {
            continue;
        }
        if let Ok((_, mut occupancy)) = occupancies.get_mut(entity) {
            occupancy.at = OccupancyAt::OnTile(to);
            claimed.push(to);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::charge::{self, ChargeKind};
    use crate::land::LandId;

    fn player(n: u32) -> Entity {
        Entity::from_raw_u32(n).expect("索引不该撞上 Entity 的保留值")
    }

    fn rent() -> ChargeKind {
        ChargeKind::Rent { land: LandId(1) }
    }

    /// 把一位神的链挂上，再问「这笔 5000 的**支出**实付多少」。
    /// 验收①在纯函数那一层的形状：一条链只碰自己那一族钱，交叉不生效。
    fn pay_as(god: God, kind: ChargeKind) -> i64 {
        let mut corrections = Corrections::default();
        grant_effects(&mut corrections, god, player(1));
        let charge = Charge::new(kind, Some(player(1)), Some(player(2)), 5_000);
        charge::stage(charge, 1.0, &corrections).amount
    }

    /// 同一句问法，问的是**收入**：payer 是系统，bearer 落在收钱的那一方。
    /// 没有这一句就写不出「衰神让到手的钱作废」—— 那条规则要求 `Scope` 命中付款人这一侧。
    fn earn_as(god: God) -> i64 {
        let mut corrections = Corrections::default();
        grant_effects(&mut corrections, god, player(1));
        let charge = Charge::new(ChargeKind::Bonus, None, Some(player(1)), 5_000);
        charge::stage(charge, 1.0, &corrections).amount
    }

    #[test]
    fn each_god_s_chain_only_touches_its_own_family_of_money() {
        // PLAN §三 神明行的八句，翻成一张金额表。四个数都取 5000 起，
        // 于是「加倍／减半／整笔免」各自一眼可读
        let rows: [(God, i64, i64, i64); 8] = [
            //        过路费  买地   罚款
            (God::BigRich, 0, 5_000, 5_000),
            (God::SmallRich, 2_500, 5_000, 2_500),
            (God::BigFortune, 5_000, 0, 5_000),
            (God::SmallFortune, 5_000, 2_500, 5_000),
            (God::BigPoor, 10_000, 5_000, 5_000),
            (God::SmallPoor, 7_500, 5_000, 5_000),
            (God::BigDoom, 5_000, 5_000, 10_000),
            (God::SmallDoom, 5_000, 5_000, 7_500),
        ];
        for (god, on_rent, on_buy, on_fine) in rows {
            assert_eq!(pay_as(god, rent()), on_rent, "{} 的过路费", god.label());
            assert_eq!(
                pay_as(god, ChargeKind::BuyLand { land: LandId(1) }),
                on_buy,
                "{} 的买地钱",
                god.label()
            );
            assert_eq!(
                pay_as(god, ChargeKind::Penalty),
                on_fine,
                "{} 的罚款",
                god.label()
            );
        }
        // 收入那一侧只有衰神碰它：财神与福神不该替别人把到手的钱削掉
        assert_eq!(earn_as(God::BigDoom), 0, "大衰神：得钱作废");
        assert_eq!(earn_as(God::SmallDoom), 2_500, "小衰神：得钱减半");
        for god in [
            God::BigRich,
            God::SmallRich,
            God::BigFortune,
            God::SmallFortune,
            God::BigPoor,
            God::SmallPoor,
        ] {
            assert_eq!(earn_as(god), 5_000, "{} 不该碰到手的钱", god.label());
        }
        // 后四位在这一层什么都不是（它们改的是房子与归属，P6c）
        for god in [God::Angel, God::Devil, God::LandDeity, God::Death] {
            assert!(
                money_corrections(god, player(1)).is_empty(),
                "{} 的钱要等 P6c，先别提前挂上",
                god.label()
            );
            assert_eq!(pay_as(god, rent()), 5_000);
        }
    }

    /// [`Corrections::release`] 键的是 `(bearer, Source::God)`。这两位在表上必须一致，
    /// 否则「神走了」永远撤不掉他给的那几条 —— 症状不是崩，是第八天租金还是双倍。
    #[test]
    fn every_row_a_god_leaves_is_reachable_by_the_exit_that_clears_it() {
        for god in God::ALL {
            for row in money_corrections(god, player(1)) {
                assert_eq!(row.source, Source::God, "{} 的链来源要写神", god.label());
                assert_eq!(
                    row.bearer,
                    player(1),
                    "{} 的链主语是被附身的那位",
                    god.label()
                );
                assert!(
                    !row.one_shot,
                    "{} 的链是常开的：一次性的那种是卡，不是神",
                    god.label()
                );
                assert_eq!(row.floor, 0, "神的链没有门槛，门槛是免费卡那一条的形状");
            }
        }
    }

    #[test]
    fn granting_twice_leaves_one_chain_and_the_last_god_wins() {
        let mut corrections = Corrections::default();
        // 大衰神给两条
        grant_effects(&mut corrections, God::BigDoom, player(1));
        grant_effects(&mut corrections, God::BigDoom, player(1));
        assert_eq!(
            corrections.len(),
            2,
            "同一位神按两次不该把链叠成四条（旋钮与读档都会走到这条路）"
        );
        // 顶走：换成大财神，前一位那两条一条都不留
        grant_effects(&mut corrections, God::BigRich, player(1));
        assert_eq!(corrections.len(), 1, "旧神的链要跟着旧神一起走");
        assert_eq!(revoke_effects(&mut corrections, player(1)), 1);
        assert!(corrections.is_empty(), "离场之后登记处不该留下尸体");
    }

    /// 别人的链不吃我的账 —— 这条在 `charge.rs` 有单测，这里是**神**这一侧的同一条：
    /// 两人各挂一位穷神与财神时，互相不该看得见。
    #[test]
    fn two_players_chains_do_not_cross() {
        let mut corrections = Corrections::default();
        grant_effects(&mut corrections, God::BigPoor, player(1));
        grant_effects(&mut corrections, God::BigRich, player(2));
        assert_eq!(corrections.len(), 2);
        let mine = Charge::new(rent(), Some(player(2)), Some(player(1)), 5_000);
        assert_eq!(
            charge::stage(mine, 1.0, &corrections).amount,
            0,
            "大财神挂在我身上，我付的租就该免掉（业主的穷神只抬他自己应付的那笔）"
        );
        let theirs = Charge::new(rent(), Some(player(1)), Some(player(2)), 5_000);
        assert_eq!(charge::stage(theirs, 1.0, &corrections).amount, 10_000);
    }

    #[test]
    fn every_god_has_a_distinct_name_and_a_lifespan() {
        let mut names: Vec<&str> = Vec::new();
        for god in God::ALL {
            assert!(!god.label().is_empty());
            assert!(god.days() > 0, "{} 一天都不管，那它不该是神", god.label());
            names.push(god.label());
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), God::ALL.len(), "两位神同名，面板分不开");
        assert_eq!(God::Death.days(), 13, "死神 13 天是 PLAN §三 写死的");
        assert_eq!(God::BigRich.days(), 7, "默认 7 天是 PLAN §三 写死的");
    }

    #[test]
    fn rivals_come_in_pairs_and_death_walks_alone() {
        for god in God::ALL {
            match god.rival() {
                Rival::God(other) => {
                    assert_eq!(
                        other.rival(),
                        Rival::God(god),
                        "{} 说 {} 是它的对位，那边不认",
                        god.label(),
                        other.label()
                    );
                }
                Rival::Dog => assert_eq!(god, God::LandDeity, "只有土地公的对位是恶犬"),
                Rival::Alone => assert_eq!(god, God::Death, "只有死神没有对位"),
            }
        }
        assert!(!God::Death.roams(), "死神不上街：它是被召来的");
        for god in God::ALL {
            if god != God::Death {
                assert!(god.roams(), "{} 该能自己上街", god.label());
            }
        }
    }

    #[test]
    fn a_pair_keeps_its_counterpart_out_of_the_pool_until_it_leaves() {
        // 验收②的形状，先在纯函数这一层钉住：大的还在，小的不能出场
        let presence = Presence {
            gods: vec![God::BigRich],
            dog: false,
        };
        let pool = presence.pool();
        assert!(!pool.contains(&God::SmallRich), "大的还站着，小的不能出场");
        assert!(!pool.contains(&God::BigRich), "已经在场的不能再刷一次");
        assert!(!pool.contains(&God::Death), "死神永远不在随机池里");

        let left = Presence::default();
        assert!(
            left.pool().contains(&God::SmallRich),
            "对位空出来了，小的这才可以入场"
        );
        // 12 位里除死神都能上街；空场时池就是这 11 位
        assert_eq!(left.pool().len(), 11);
    }

    #[test]
    fn the_land_deity_and_the_dog_hold_each_other_out() {
        let with_deity = Presence {
            gods: vec![God::LandDeity],
            dog: false,
        };
        assert!(
            !with_deity.pool().contains(&God::LandDeity),
            "它自己已经在场"
        );
        let with_dog = Presence {
            gods: vec![God::BigRich],
            dog: true,
        };
        assert!(
            !with_dog.pool().contains(&God::LandDeity),
            "路上有恶犬，土地公就不刷了 —— 同一路只许一位"
        );
        assert!(
            !with_dog.pool().contains(&God::SmallRich),
            "大财神在场，小财神这一路也不能出"
        );
        assert!(
            with_dog.pool().contains(&God::Angel),
            "恶犬只挡土地公那一路，不该把天使也一起挡掉"
        );
    }

    #[test]
    fn observe_sees_both_halves_of_where_a_god_can_be() {
        // 路上与身上都算在场：对位规则问的是「同一路有几位」，不是「它站在哪儿」
        let kinds = [OccupancyKind::God(God::BigPoor), OccupancyKind::Roadblock];
        let attached = [God::SmallFortune];
        let seen = Presence::observe(kinds.iter().copied(), attached.iter().copied());
        assert_eq!(seen.gods, vec![God::BigPoor, God::SmallFortune]);
        assert!(!seen.dog);
        assert!(
            !seen.pool().contains(&God::SmallPoor),
            "大穷神在路上，小穷神也不能出"
        );

        let dogs = Presence::observe([OccupancyKind::Dog].iter().copied(), [].iter().copied());
        assert!(dogs.dog);
    }
}
