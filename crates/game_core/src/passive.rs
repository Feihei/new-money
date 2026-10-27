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
//! ## ② 之后：这里还住那道门
//!
//! 表是它的读数，[`Gate`] 是它的身体：一件**事**在落到世界上之前要过的咽喉（PLAN §五 第 1
//! 条）。`charge` 那一侧的账本来就有这样一个咽喉（`Books::settle`），而事这一侧以前有五个
//! —— 每一处都自己写一遍「构造 → 挡 → 写状态」，于是「免罪卡到底吃什么」这句话在五份
//! 代码里各说各的。② 把它们收成一个 [`submit`]，并把 P3a 那个 `Insulation` 组件删掉：
//! **卡在手里就是资格**，门在花掉那一刻直接从 [`Hand`](crate::hand::Hand) 里摘一张。
//!
//! 三条纪律都写在门里而不是写在来路上：问序**先抵消后转嫁**（[`Ask`] 是状态不是两个 `if`）、
//! **链最长两跳**（[`Job::shifted`] 那一位）、**同类多张只问一次**（[`GateDesk::holds`]
//! 回答的是「有没有」而不是「有几张」）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use std::collections::VecDeque;

use crate::afflict::Affliction;
use crate::auction::Gavel;
use crate::card::{CardEffect, PassiveCard};
use crate::charge::ChargeKind;
use crate::deck::{CardId, CardSink, Deck};
use crate::hand::CardAim;
use crate::landing::{Pending, PendingDecision};
use crate::magic::{MagicRitual, MagicSpell};
use crate::msg::{
    BlameShifted, Blocker, CardDrawn, CardNotice, CardPlayed, EffectOutcome, NewsHit,
    PassiveAnswer, PassiveMove, PassiveUsed,
};
use crate::news::NewsDef;
use crate::player::DetentionPlace;
use crate::turn::TurnPhase;

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
    /// 这张表存在的理由就在这一行（10-27 例外）。它今天还没有门上的来路：公司到访那一处
    /// 的主语是**一笔钱**（B 型收费），把那一处接进来归 ③（钱那侧的门），届时它才点名这一行。
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

// ===== 门（P13b②：事那侧）=====

/// 门里的一步「接下来要办的事」。
///
/// 为什么不直接是 [`CardEffect`]：四条来路的动词库不同一张 —— 卡面与新闻共用 `CardEffect`，
/// 女巫用的是 [`MagicSpell`]（那十二条里六条压根不在动词库里），而公司到访与旅馆住店
/// 连动词都没有（它们是一段结算的尾巴）。门要的是「下一步做什么」这一层，
/// 不是「哪张表里的哪一行」。
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    /// 卡面／新闻的一个动词
    Effect(CardEffect),
    /// 女巫的一道法术
    Spell(MagicSpell),
    /// 从命运牌堆再抽一张并当场办它（女巫「抽命运 3 张」的每一张）
    FateDraw,
}

/// 这道门挂着的一份**办事调用**。
///
/// 信封为什么装「一次调用」而不是「一条效果」：一条卡面可以带好几条效果，而门截住中间那条
/// 时，前面那些**已经落到世界上**（扣掉的钱不会退回来），后面那些还没跑。所以信封带着
/// `todo`（剩余条目）与 `done`（已经办成的凭据）答完继续跑，而不是把整次调用重跑一遍 ——
/// 重跑就是付两遍钱（与 P3d③「同一帧押两遍钱」同一条坑）。
#[derive(Debug, Clone)]
pub struct Job {
    /// 这一次调用原本挨它的那个人（卡面／法术点名的那位）。
    ///
    /// 嫁祸改的**不是**这一格，而是 [`Job::front`] 里那条 [`Affliction::victim`]：
    /// 一张卡的两件事不该因为第一件被转走就整个换人 —— 后面那些仍然打在原来那位头上。
    pub victim: Entity,
    /// 还没执行的条目。第一条就是此刻被门截住的那一条（它的读数在 [`Job::front`]）。
    pub todo: VecDeque<Step>,
    pub front: Option<Affliction>,
    /// 已经办成的那几条凭据。收尾时交给原路。
    pub done: Vec<EffectOutcome>,
    /// 「哪条数据要办这件事」（卡 id／新闻 id），只进日志与凭据，不进判据
    pub source: String,
    pub aim: Option<CardAim>,
    pub gavel: Gavel,
    /// 办完之后凭据交给谁（五格＝五条来路，**穷尽表**）
    pub tail: Tail,
    /// 问到哪一问（[`Ask::Offset`] → [`Ask::Shift`]，问完就落）
    pub ask: Ask,
    /// 「已经转过一次」那位（PLAN §五 第 4 条：链最长两跳、有界、同种子可复现）
    pub shifted: bool,
    /// 这一次调用**真的办了点什么**吗。女巫那份「落到谁头上」的名单只数真落下来的那位 ——
    /// 它替代的是旧 `cast_one` 那个 `bool` 返回值，而返回值在门里活不下去
    /// （一次调用可能被门拆成好几轮，最后那一轮才知道答案）
    pub effected: bool,
    /// 这件事最后落到谁头上了（嫁祸之后是接住的那位）。`None` = 没落下来
    /// （被挡掉了，或者压根没有这一件事）
    pub landed_on: Option<Entity>,
    /// 这一份信封**正挂着等人回答**吗 —— 「问出去了、还没答」的唯一记录。
    ///
    /// 为什么不是「最前面那份的 `front` 有没有值」：`one()` 在「这一行不用问」那一条路上
    /// 也会把 `front` 填上（下一轮 `land_now` 要落的就是它），那一格说的是**手上有什么事**，
    /// 不是**有没有人在答**。为什么也不是那个槽：槽在整个队列办完之前一直归门，而
    /// [`finish`] 派回原路时同帧就可能再挂起下一份信封 —— 那时外层 `pump` 若继续 pop，
    /// 就会把这份刚挂起来的信封当成「已经答过了」直接落地（行为由 `tests/magic.rs` 的
    /// `a_forgive_card_protects_its_own_holder_only` 钉，而它读的判据是 [`Gate::asking`]
    /// 那一早退还由 RC⑦ 验过牙齿）；这一格本身的形状由 `tests::a_job_being_run_is_not_a_job_being_asked` 钉。
    /// [`Gate::asking`] 与 [`gate_system`] 认的都是这一格。
    pub parked: bool,
}

impl Job {
    pub fn new(
        victim: Entity,
        todo: Vec<Step>,
        source: String,
        aim: Option<CardAim>,
        gavel: Gavel,
        tail: Tail,
    ) -> Self {
        Self {
            victim,
            todo: VecDeque::from(todo),
            front: None,
            done: Vec::new(),
            source,
            aim,
            gavel,
            tail,
            ask: Ask::Offset,
            shifted: false,
            effected: false,
            landed_on: None,
            parked: false,
        }
    }

    /// 这一次调用真正挨它的那位：嫁祸之后是接住的人，没嫁祸就是原本那位。
    /// 凭据（法事名单、新闻 hits）念的是这一个数。
    pub fn landed(&self) -> Entity {
        self.landed_on.unwrap_or(self.victim)
    }
}

/// 门上的那一问。**有界的两问**（PLAN §五 第 7 条：免罪？→ 嫁祸给谁？；
/// 账那侧的「免费？」是 ③ 加在第一问那一格里，不是第三问）。
///
/// 顺序有内容：先免掉就没有可转的东西，反过来会白烧一张嫁祸 —— 与 P6b① 那条
/// 「先问神、后花卡」同族。所以这一格是**状态**而不是两处各写一遍的 `if`。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ask {
    /// 免罪？（抵消类。账那侧的免费卡在 ③ 复用这一格，因为它是同一个「先抵消」的位置）
    Offset,
    /// 嫁祸给谁？（要过那位「已转过一次」）
    Shift,
}

impl Ask {
    /// 窗口上那一句问话。面板念的是这一句，所以它住在规则层。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Offset => "要用免罪卡吗？",
            Self::Shift => "要嫁祸给谁？",
        }
    }
}

/// 答完之后，这一次调用的凭据交给谁。五格＝五条来路，**穷尽表**：
/// 新增一条来路必须在这里回答「你的收据找谁」。
///
/// `Witch` 与 `News` 各带一份「还有一串人要落」的续作：一场法事筛出的是**一批人**、
/// 一条人档新闻点到的也是**一批人**，而门截住其中一个时，另外那几个还没落。
/// 续作存在信封里而不是存在来源的循环里，是因为来源那条 system 在门挂着的那几十帧里
/// 根本不会跑（窗口已经不是它的了）—— 这条与 P9 拍卖的 `standby` 是同一个形状。
#[derive(Debug, Clone)]
pub enum Tail {
    /// 抽到的那张牌。收尾＝发 [`CardDrawn`]；`receipt` 那一格说的是「要不要把那张卡面
    /// 摆给谁读」—— 命运格那条路给 `Some(抽卡人)`，而女巫「抽命运 3 张」给 `None`
    /// （一次装不下三张，而此刻说话的是法事窗口，P13 定的口径原样保留）
    Drawn {
        card: CardNotice,
        player: Entity,
        receipt: Option<Entity>,
    },
    /// 玩家打出的那一张。收尾＝发 [`CardPlayed`]
    /// （出牌人与承受者可以不是同一个人，而**嫁祸之后承受者又换了一位** —— 所以那条凭据
    /// 的 `victim` 读的是门答完的那一刻，不是出牌那一刻）
    Played { card: CardId, player: Entity },
    /// 女巫的一道法术，`waiting` 里那几个还没落。`hit` 与 `blocked` 各是一半 ——
    /// 「这道法术落到谁头上了」与「谁用手里的卡把它挡掉了」是两句不同的话，
    /// 合成一份名单的话，顶栏只能念出前者（那是 P13 那张免疫矩阵唯一看得见的证据）
    Witch {
        caster: Entity,
        spell: MagicSpell,
        hit: Vec<Entity>,
        blocked: Vec<Entity>,
        waiting: Vec<Entity>,
    },
    /// 新闻中心播的一条人档（`waiting` 里那几个还没落）。`item` 带整条定义而不是下标：
    /// 收尾要念标题，而门不该为了念一句话去查资产（那份资产在 `NewsSink` 上，
    /// 两条 `Res<Assets<NewsSet>>` 并存本身不冲突，但门不该依赖它）
    News {
        broadcaster: Entity,
        item: NewsDef,
        hits: Vec<NewsHit>,
        waiting: Vec<(Entity, Vec<Step>)>,
    },
}

/// 这道门本身。**槽仍然只有一个**（[`PendingDecision`]），这里挂的是「谁在等着答、答到哪一问」。
///
/// 队列由「这一笔事件 × 各人手里有什么卡」现场算出来，所以它**不是**拍卖那台会话机器
/// （拍卖的队列由座位序生成，一场是一场）。能从 P9 拿走的只有两样小东西：
/// [`Gate::holds_turn`] 这个问句，和「槽每次只挂该答的那一位」。
#[derive(Resource, Debug, Default)]
pub struct Gate {
    pub queue: VecDeque<Job>,
    /// 门押住回合时记下的**归还地址**。`WaitRoll` 里打的卡被门截住时，回合得停在
    /// `WaitDecision`，答完还得回到 `WaitRoll`（玩家还没掷骰）—— 与
    /// [`Auction::resume`](crate::auction::Auction::resume) 同一个理由、同一个形状。
    /// `None` = 门没有押过回合（在 `Landed` 那条链里，回合本来就被别人挂着）。
    pub resume: Option<TurnPhase>,
}

impl Gate {
    /// 这台机器**还占着回合吗**。与拍卖同一句问法，写给
    /// [`close_landing_system`](crate::landing::close_landing_system)：落格那条链的末尾
    /// 「没人等回答就把回合收尾」与本机的「把窗口挂到该答的人身上」写的可能是同一帧里
    /// 两个不同的 `NextState`，而次序没人保证（§6.1 坑 37）。让它先问一句。
    pub fn holds_turn(&self) -> bool {
        !self.queue.is_empty()
    }

    /// 此刻该答的那一位、他该答的那一问、以及那是表上的哪一行。面板把三样一起念出来。
    ///
    /// 判据是 [`Job::parked`] 而不是「最前面那份手上有没有事」—— 后者在「不用问、下一轮就落」
    /// 那一条路上也为真，那样一来 dump 会说「有人在答」而其实在场没人被问。
    pub fn asking(&self) -> Option<(Entity, Ask, PassiveEvent)> {
        let job = self.queue.front()?;
        if !job.parked {
            return None;
        }
        let aff = job.front?;
        Some((aff.victim, job.ask, aff.event))
    }

    /// 此刻排在最前面那份信封（dump 面板读它，别自己再数一遍队列）。
    pub fn front(&self) -> Option<&Job> {
        self.queue.front()
    }
}

/// 一道门要动的全部东西。`CardSink` 是「把一件事落到某人头上」那把工具包（P3a 起），
/// 这里在它上面加五样：那个槽、那个资源、那一格回合状态与它的前一格、法事草稿、
/// 命运牌堆，以及五条来路各自的凭据写手。
///
/// ⚠️ 四条来路的写手**只在这里有一份**：抽卡那条 system 原来自己写 `CardDrawn`，
/// 现在交给 [`crate::deck::finish_drawn`] 收尾 —— 两处各写一次的下一步就是「同一个数存两处」
/// （而收据的 `outcomes` 恰恰是门答完才凑齐的）。
///
/// ⚠️ 它**内含** `CardSink` 而不是并列（§6.1 坑 33）：`Books` 与 `Commands` 出现两次会在
/// 参数建立期直接撞。连带后果：任何想用门的 system 都不许再自己拿 `decks`／`hands`／
/// `shelf`／`Rng`，一律走 `desk.sink.kit` 与 `desk.decks`。
#[derive(SystemParam)]
pub struct GateDesk<'w, 's> {
    pub sink: CardSink<'w, 's>,
    pub gate: ResMut<'w, Gate>,
    pub pending: ResMut<'w, PendingDecision>,
    pub next: ResMut<'w, NextState<TurnPhase>>,
    /// 现在这一格在哪。`offer` 用它决定「要不要把回合押住」，而 `ResMut<NextState>`
    /// 写下去的世界读数要到转移之后才变 —— 所以这一格与那一格是两个参数（坑 37 的邻居）。
    pub phase: Res<'w, State<TurnPhase>>,
    /// 法事那一份草稿（女巫的续作与散场都要读写它）。与「窗口同生同灭」那条纪律一起
    /// 写在 [`MagicRitual`] 的注里。
    pub ritual: ResMut<'w, MagicRitual>,
    pub decks: Query<'w, 's, &'static mut Deck>,
    /// 道具栏（女巫「变卖所有道具」那一道要清空它）。
    ///
    /// ⚠️ 这一格让门的每一条 system 都带上 `&mut Inventory`，于是它与「放置道具」那族
    /// （[`place_item_system`](crate::occupancy::place_item_system)）在同一帧里有了先后要求。
    /// 由 `check.sh` 与 `cargo test -p game_core` 的那批 app 用例盯着：调度期的借用冲突
    /// 是**当场 panic**，不是静默降级（§6.1 坑 33）。
    pub inventories:
        Query<'w, 's, &'static mut crate::items::Inventory, With<crate::player::PlayerId>>,
    pub drawn: MessageWriter<'w, CardDrawn>,
    pub played: MessageWriter<'w, CardPlayed>,
    pub cast: MessageWriter<'w, crate::msg::MagicCast>,
    pub blocked: MessageWriter<'w, crate::msg::MagicBlocked>,
    pub newsfired: MessageWriter<'w, crate::msg::NewsFired>,
    pub used: MessageWriter<'w, PassiveUsed>,
    pub blame: MessageWriter<'w, BlameShifted>,
}

impl<'w, 's> GateDesk<'w, 's> {
    /// 手里有没有这一张被动卡（返回**那一张的下标**，花掉它就是花掉这一张）。
    ///
    /// 为什么按 `PassiveCard` 翻而不是按卡 id：三张被动卡在池里的位置是数据，
    /// 而门要问的是「这一行归哪张卡」—— 那一格的答案在表上
    /// （[`PassiveEvent::offset_card`]），拿它去比对卡上那一位，判据只有一处。
    ///
    /// **返回「有没有」而不是「几张」**不是省事：PLAN §五 第 7 条要的正是「同类多张只问一次」，
    /// 而张数在判据里没有位置（数它的那条循环就是那条要防的错）。
    pub fn holds(&self, player: Entity, card: PassiveCard) -> Option<CardId> {
        let hand = self.sink.kit.hands.get(player).ok()?;
        let items = self.sink.kit.shelf.items();
        hand.cards.iter().copied().find(|id| {
            items
                .get(id.0 as usize)
                .is_some_and(|def| def.passive == Some(card))
        })
    }

    /// 数一数手里这一张有几份。**只给面板念那句话用**（「用免罪卡（手里还有 2 张）」），
    /// 判据侧一律用 [`GateDesk::holds`]。
    pub fn count(&self, player: Entity, card: PassiveCard) -> u32 {
        let Ok(hand) = self.sink.kit.hands.get(player) else {
            return 0;
        };
        let items = self.sink.kit.shelf.items();
        hand.cards
            .iter()
            .filter(|id| {
                items
                    .get(id.0 as usize)
                    .is_some_and(|def| def.passive == Some(card))
            })
            .count() as u32
    }

    /// 花掉一张被动卡：从手牌摘掉那张，并把卡面交给流水。
    ///
    /// 先摘卡再发凭据，与出牌那条路同一条纪律（`hand::play_card_system`：效果执行到一半
    /// 也不该留下一张还能再花一次的卡）。**摘不到就一声不响地不发凭据** —— 那种情形
    /// 意味着判据与手牌分叉了，而分叉的方向是「玩家白拿了一次抵消」，比一句 warn 严重，
    /// 所以这里用 `warn!` 而不是 `debug!`。
    fn spend(&mut self, player: Entity, card: CardId) -> Option<CardNotice> {
        let notice = crate::hand::card_notice(&self.sink.kit.shelf, card);
        let mut hand = self.sink.kit.hands.get_mut(player).ok()?;
        if !hand.spend(card) {
            warn!("{player:?} 手里读不到那张被动卡，这一次抵消没花掉任何东西");
            return None;
        }
        Some(notice)
    }

    /// 可以点名给谁：在场、没破产、不是自己。与陷害卡那一条判据同源
    /// （`CardTarget::Player` 那句「不能陷害自己 + 不能打离场的人」），
    /// 所以这里**借 `books` 那一格**而不是再写一份名单 —— 恶人不在名单里：
    /// 他们手里没有卡，也不是 `PlayerId`（[`Books::solvent`](crate::settle::Books::solvent)
    /// 只数玩家）。名字不在这里念：面板按实体查 `PlayerName`，与 HUD 那几处同一条路。
    pub fn shift_candidates(&self, from: Entity) -> Vec<Entity> {
        blame_candidates(
            &self
                .sink
                .books
                .solvent()
                .into_iter()
                .map(|(who, _)| who)
                .collect::<Vec<_>>(),
            from,
        )
    }
}

/// 「可以点名给谁」的判据本体：在场、没破产、不是自己。
///
/// 为什么是一句自由函数而不是只挂在 [`GateDesk::shift_candidates`] 上：面板那一排候选按钮
/// 与门里「这一问要不要挂出来」用的是**同一句**（`GateDesk::next_question` 读的就是它）。
/// 两处各写一遍的形状是「按钮亮着而门说不能转」，那与 P9 那句「写的与扣的必须同一个表达式」
/// 是同一条纪律。名字不在这里念：面板按实体查 `PlayerName`，与 HUD 那几处同一条路。
///
/// 恶人不在名单里：他们手里没有卡，也不是 `PlayerId`
/// （[`Books::solvent`](crate::settle::Books::solvent) 只数玩家）。
pub fn blame_candidates(all: &[Entity], from: Entity) -> Vec<Entity> {
    all.iter().copied().filter(|who| *who != from).collect()
}

/// 把一件事交给门。**这是「事」那侧唯一的入口**（PLAN §五 第 1 条的那个咽喉）。
///
/// 调用方只换一行：从「自己 settle ＋ 自己发凭据」变成「构造一个 [`Job`] 交进来」。
/// 就地办完还是挂成窗口由门定 —— 返回 `()` 而不是 `bool`：来路**不该需要知道**，
/// 因为它的收尾已经装在 [`Job::tail`] 里，门什么时候办完什么时候派回。
pub fn submit(desk: &mut GateDesk, job: Job) {
    desk.gate.queue.push_back(job);
    pump(desk);
}

/// 一直办到「需要问人」或者队列空。
///
/// 队列是深度优先的：一个信封的续作（女巫的下一个承受者、DrawFate 抽到的那三张）排在
/// 别的信封之前，所以「一场法事的名单」不会与「一条新闻的名单」交错着问人。
fn pump(desk: &mut GateDesk) {
    while let Some(mut job) = desk.gate.queue.pop_front() {
        match one(desk, &mut job) {
            Out::Waiting => {
                // 先挂窗口再放回队列：`offer` 读的是信封里那件事，而它不该在借出之后被读
                job.parked = true;
                offer(desk, &job);
                desk.gate.queue.push_front(job);
                return;
            }
            Out::Running => desk.gate.queue.push_front(job),
            Out::Nested(kids) => {
                // 先把自己放回队首，再把子信封插到自己**前面** —— 于是次序是
                // [子1, 子2, 自己, 其余]：三张命运牌各自办完，才轮到这一次调用的收尾
                desk.gate.queue.push_front(job);
                for kid in kids.into_iter().rev() {
                    desk.gate.queue.push_front(kid);
                }
            }
            Out::Finished => {
                finish(desk, job);
                // 收尾派回原路时，那条尾巴可能**当场又交进来一份新信封**（女巫名单上的下一个人、
                // 新闻名单上的下一个、命运牌套出来的命运牌），而那一份若挂着问就正在等人回答。
                // 本轮必须在这里交还控制权：外层 `while` 再 pop 一次，就会把那份**等人回答的**
                // 信封当成「答完了的那条」直接落地 —— 症状是她手里那张免罪卡白躺在手里而人照样
                // 进监狱，而窗口没人认领（`gate_system` 只会说「门是空的」）。
                // 判据用 [`Gate::asking`]（队列最前面那份挂着吗），**不用那个槽**：
                // 槽在门办完一问之后仍然是 `Passive`（它还归门，直到队列空了才由
                // [`hand_back`] 收），拿它当判据就把「自己派回来的下一个承受者」也当成挂着的了。
                if desk.gate.asking().is_some() {
                    return;
                }
            }
        }
    }
    hand_back(desk);
}

/// 一次调用往下走一步的结果。
enum Out {
    /// 门要把窗口挂到某人身上了（本轮到此为止）
    Waiting,
    /// 办掉了一步，接着办同一条
    Running,
    /// 这一步生出几条**要先办完**的子调用（女巫「抽命运 3 张」的每一张）
    Nested(Vec<Job>),
    /// 条目跑空了：交给原路收尾
    Finished,
}

/// 哪一条卡面动词是「一件要问人的事」。**这张表是那条纪律的唯一落点**：
/// 新增一条动词必须当场表态它算不算「一件灾落下来」。
///
/// `pub` 是因为 [`crate::deck::apply_one`] 读的是**同一张表**：那条路（新闻的地档与机构档）
/// 不经过门，但它落地用的还是 [`crate::afflict::land`] —— 两处各认各的「哪几条是事」，
/// 就是「同一句 detain 在两个文件里各修一半」的另一种写法。
pub fn affair_of_card(effect: &CardEffect, victim: Entity) -> Option<Affliction> {
    match effect {
        CardEffect::Detain { place, turns } => Some(Affliction::detain(victim, *place, *turns)),
        CardEffect::Stay { turns } => Some(Affliction::stay(victim, *turns)),
        CardEffect::MoveCap { turns } => Some(Affliction::slow(victim, *turns)),
        CardEffect::Reverse => Some(Affliction::reversal(victim)),
        // 下面这些不落「事」：钱、地、地块状态、瞬移、请送神、停摆 ——
        // 它们各自那一格说得出为什么不是「一件灾」。`Damage`（地雷／飞弹／天灾）在表上有
        // 一行（[`PassiveEvent::Hazard`]）而路上不走这里：它改的是楼不是人，
        // 由 [`Books::damage`](crate::settle::Books::damage) 结账
        CardEffect::AddCash { .. }
        | CardEffect::DeductCash { .. }
        | CardEffect::Levy { .. }
        | CardEffect::Release { .. }
        | CardEffect::MoveTo { .. }
        | CardEffect::LandStatus { .. }
        | CardEffect::Damage { .. }
        | CardEffect::FreePass { .. }
        | CardEffect::BuyLand
        | CardEffect::Auction { .. }
        | CardEffect::Suspend { .. }
        | CardEffect::SummonGod
        | CardEffect::BanishGod => None,
    }
}

/// 哪一步是「一件要问人的事」。`Some` 的那几条就是 P13b② 收进来的**全部**五处
/// 「就地构造 → 就地挡 → 就地写状态」（`deck.rs` 四处 + `magic.rs` 一处），
/// 外加两处点名行（公司到访／旅馆住店）。
///
/// 法事那四条在构造完顺手标成 [`Affliction::ritual`]：神明量的是「落到某人头上的厄运」，
/// 而女巫那 3 天写在 [`MAGIC_DAYS`](crate::magic::MAGIC_DAYS) 这张表上 —— 免疫与天数加倍
/// 两句话都由那一格答，所以这里只认「这一步是不是法术」，不再回去问来路。
fn affair_of(step: &Step, victim: Entity) -> Option<Affliction> {
    match step {
        Step::Effect(effect) => affair_of_card(effect, victim),
        Step::Spell(spell) => Some(
            match spell {
                // 女巫判 3 天写在 [`crate::magic::MAGIC_DAYS`] 那一格（卡面的天数在卡上、这里的在这里）
                MagicSpell::Jail => {
                    Affliction::detain(victim, DetentionPlace::Jail, crate::magic::MAGIC_DAYS)
                }
                MagicSpell::Hospital => {
                    Affliction::detain(victim, DetentionPlace::Hospital, crate::magic::MAGIC_DAYS)
                }
                // 女巫的「原地停留」是字面 1 回合（P4④ 口径），与停留卡同一条
                MagicSpell::Stay => Affliction::stay(victim, 1),
                MagicSpell::TurnBack => Affliction::reversal(victim),
                // 其余六道**根本不构造 Affliction**：这就是 P13 那句
                // 「十二条里只有两条挡得住」的落地方式 —— 不是有一条 `if` 在挡，
                // 是那六条在类型上进不了这条路由
                MagicSpell::GainCard
                | MagicSpell::BuildHere
                | MagicSpell::BankCash
                | MagicSpell::DrawFate
                | MagicSpell::SellCards
                | MagicSpell::SellItems
                | MagicSpell::Demolish
                | MagicSpell::AuctionHere => return None,
            }
            .ritual(),
        ),
        Step::FateDraw => None,
    }
}

/// 办一步。四格返回见 [`Out`]。
fn one(desk: &mut GateDesk, job: &mut Job) -> Out {
    // 1. 门答完了的那件事：落
    if job.front.is_some() {
        land_now(desk, job);
        return Out::Running;
    }
    // 2. 还有条目吗
    let Some(step) = job.todo.front().cloned() else {
        return Out::Finished;
    };
    if let Some(aff) = affair_of(&step, job.victim) {
        job.todo.pop_front();
        // **先问神再问卡**（P6b① 那条顺序纪律）：神那一半不消耗任何东西，也不看这一行
        // 可不可免。反过来写，大福神在场时每一件事都会白白花掉玩家一张免罪卡，
        // 而那件事只有在他不再免疫的下一件上才看得见 —— 也就是看不见
        //
        // 「法事不吃神明的豁免」这一格现在挂在**事**上（[`Affliction::is_ritual`]）而不是
        // 来路上：同一条判据还要管衰神的天数加倍（`afflict::days_of`），而落地那一句拿不到来路。
        if !aff.is_ritual
            && let Some(god) = god_resists(desk, aff.victim)
        {
            job.done.push(EffectOutcome::Blocked {
                event: aff.event,
                by: Blocker::God(god),
            });
            return Out::Running;
        }
        job.front = Some(aff);
        job.ask = Ask::Offset;
        // 「不可免也不可转」的那些行在这里就答 `false` —— 于是 `Hold` 那一族
        // （停留／乌龟／掉头）与联合航空、住店两行**从来挂不出窗口**，
        // 这正是 P13 定的口径，而不是一条 `if` 挡住了它
        if next_question(desk, job) {
            return Out::Waiting;
        }
        return Out::Running;
    }
    let Some(step) = job.todo.pop_front() else {
        return Out::Finished;
    };
    match step {
        Step::Effect(effect) => {
            // `None` = 这一笔钱扣不起、人已经离场：剩下的条目不办（旧 `apply_effects`
            // 那句 `break` 的归宿）。凭据（`CouldNotPay` + `PlayerBankrupt`）在
            // `apply_one` 里已经发掉了
            if let Some(outs) = crate::deck::apply_one(
                &mut desk.sink,
                &effect,
                job.victim,
                &job.source,
                job.aim,
                job.gavel,
            ) {
                job.effected |= !outs.is_empty();
                job.done.extend(outs);
            } else {
                job.todo.clear();
            }
            Out::Running
        }
        Step::Spell(spell) => {
            job.effected |= crate::magic::cast_step(desk, spell, job.victim, job.gavel);
            Out::Running
        }
        Step::FateDraw => match crate::deck::draw_fate(desk, job.victim) {
            Some(kid) => {
                // 抽到了就算这道法术**办成了事**，即使那张牌后来被门挡掉：法事的名单数的
                // 是「这道法术落到谁头上了」，而不是「那三张牌各办了没有」
                job.effected = true;
                Out::Nested(vec![kid])
            }
            // 牌堆空了（一副都没有）：这一条什么也不办，与旧 `DrawFate` 那句 `break` 同义
            None => Out::Running,
        },
    }
}

/// 落地那一条（没人可答、或答完了要落的那一条）。
fn land_now(desk: &mut GateDesk, job: &mut Job) {
    let Some(aff) = job.front else {
        return;
    };
    let source = job.source.clone();
    let outs = crate::afflict::land(&mut desk.sink, &aff, &source);
    job.landed_on = Some(aff.victim);
    // 「办了」按**世界上真变了什么**判，不按「落下来了」判：棋盘没有那条分支时
    // `land` 交回一张空清单（旧 `cast_one` 那一句 `let Some(..) = detain(..) else { false }`
    // 的归宿），而顶栏不该对一块什么都没发生的人念出「他被关起来了」
    job.effected |= !outs.is_empty();
    job.done.extend(outs);
    job.front = None;
    job.ask = Ask::Offset;
}

/// 那位神是否豁免这一族厄运。返回他是谁，凭据要说得出**是哪一位**挡的
/// （① 留的那条欠账：以前那句「被抵消」在神明挡下时是句假话）。
fn god_resists(desk: &GateDesk, victim: Entity) -> Option<crate::gods::God> {
    let held = desk.sink.possessions.get(victim).ok()?;
    crate::gods::resists(held.god).then_some(held.god)
}

/// 门此刻该不该问、问哪一句。会把 [`Job::ask`] 推到下一格；推到底答 `false` = 可以落了。
///
/// **同类多张只答一次**（PLAN §五 第 7 条）就在这一句的形状里：它问的是「这个人手里
/// 有没有这一张」，不是「有几张」，也没有按张数循环的那条路。
fn next_question(desk: &GateDesk, job: &mut Job) -> bool {
    let Some(aff) = job.front else {
        return false;
    };
    loop {
        match job.ask {
            Ask::Offset => {
                let card = aff.event.offset_card();
                if aff.event.forgivable()
                    && card.is_some()
                    && desk.holds(aff.victim, expect_card(card)).is_some()
                {
                    return true;
                }
                job.ask = Ask::Shift;
            }
            Ask::Shift => {
                // 「不可免者不可转」这条蕴含在这里**不需要再判一次**：`shiftable()` 为真的
                // 行由表保证都可免，所以走到这一格时上面那一问已经查过他手里有没有卡
                let asked = aff.event.shiftable()
                    && !job.shifted
                    && desk.holds(aff.victim, PassiveCard::Shift).is_some()
                    && !desk.shift_candidates(aff.victim).is_empty();
                return asked;
            }
        }
    }
}

/// `offset_card()` 说「有张抵消卡管这一行」时给的就是这一张。写成 `expect` 而不是把
/// 上面那三格判断并成一个 `Option<PassiveCard>` 的 `if let`：那一格为 `None` 的行
/// 上面已经用 `forgivable()` 挡掉了，而这一句被走到时它**必须**是 `Some` ——
/// 那种「表上两格自相矛盾」的形状由 `passive::tests` 那条逐行用例挡，不在这里再赌一次。
fn expect_card(card: Option<PassiveCard>) -> PassiveCard {
    card.expect("表上那一行答「可免」却没说由哪张卡答")
}

/// 把窗口挂到该答的那一位身上，并记下**答完回哪一格**。
///
/// 回合停住有两条路（同一帧两处写 `NextState` 谁赢没人保证，§6.1 坑 37）：
/// 落格那条链末尾的 `close_landing_system` 会先问 [`Gate::holds_turn`]，所以从 `Landed`
/// 进来的不需要这里写；而 `WaitRoll` 里打的卡没有人替它写，就地把回合押住。
/// 归还地址取「此刻那一格」而不是写死 `TurnEnd`：`WaitRoll` 里出牌被门截住、答完就该
/// 回到 `WaitRoll`（玩家还没掷骰），而 `Landed` 与 `WaitDecision` 里的事答完都是往下走。
fn offer(desk: &mut GateDesk, job: &Job) {
    let Some(aff) = job.front else {
        return;
    };
    desk.pending.offer(aff.victim, Pending::Passive);
    if desk.gate.resume.is_none() {
        let here = *desk.phase.get();
        desk.gate.resume = Some(if here == TurnPhase::WaitRoll {
            TurnPhase::WaitRoll
        } else {
            TurnPhase::TurnEnd
        });
    }
    if desk.phase.get() != &TurnPhase::WaitDecision {
        desk.next.set(TurnPhase::WaitDecision);
    }
}

/// 信封跑完了：把凭据交给原路（五格穷尽）。
///
/// `Witch`／`News` 那两格先问「还有人要落吗」—— 有就起一条新的信封继续跑（**同一帧里**，
/// 所以名单上五个人不会拖五帧），没有才真的收尾。
fn finish(desk: &mut GateDesk, job: Job) {
    let landed = job.landed();
    let Job {
        victim,
        done,
        tail,
        effected,
        aim,
        gavel,
        ..
    } = job;
    match tail {
        Tail::Drawn {
            mut card,
            player,
            receipt,
        } => {
            card.outcomes = done;
            crate::deck::finish_drawn(desk, card, player, receipt);
        }
        Tail::Played { card, player } => {
            crate::deck::finish_played(desk, card, player, landed, done);
        }
        Tail::Witch {
            caster,
            spell,
            mut hit,
            mut blocked,
            mut waiting,
        } => {
            if effected {
                hit.push(landed);
            } else if done
                .iter()
                .any(|out| matches!(out, EffectOutcome::Blocked { .. }))
            {
                // 没落到他头上，而理由是一条**挡下来**的记录 —— 那一位要单独有凭据
                blocked.push(victim);
            }
            if waiting.is_empty() {
                crate::magic::finish_witch(desk, caster, spell, hit, blocked);
            } else {
                let who = waiting.remove(0);
                let source = format!("女巫「{}」", spell.label());
                submit(
                    desk,
                    Job::new(
                        who,
                        crate::magic::steps_of(spell),
                        source,
                        aim,
                        gavel,
                        Tail::Witch {
                            caster,
                            spell,
                            hit,
                            blocked,
                            waiting,
                        },
                    ),
                );
            }
        }
        Tail::News {
            broadcaster,
            item,
            mut hits,
            mut waiting,
        } => {
            // `landed` 而不是原来的那位：新闻点到的是甲，而甲把这一件嫁祸给了乙，
            // 那么「这一条新闻落在他头上」那句说的是乙。谁逃掉了另有 `BlameShifted` 说
            hits.push(NewsHit {
                player: landed,
                outcomes: done,
            });
            if waiting.is_empty() {
                crate::news::finish_news(desk, broadcaster, &item, hits);
            } else {
                let (who, todo) = waiting.remove(0);
                let source = item.id.clone();
                submit(
                    desk,
                    Job::new(
                        who,
                        todo,
                        source,
                        None,
                        Gavel::Public,
                        Tail::News {
                            broadcaster,
                            item,
                            hits,
                            waiting,
                        },
                    ),
                );
            }
        }
    }
}

/// 这台机器每帧的驱动：读回答、推到下一问、答完了把剩下的条目交回原路。
///
/// ⚠️ 它与 [`crate::landing::decision_system`] 同一条窗口（`Pending::Passive`），而那条
/// system 对这一格**三种 `Decision` 一律不认** —— 认了任何一条都会把窗口清掉，
/// 而门还在等下一问。这与拍卖那格的处理同形（见 `Pending::Auction` 那条注）。
pub fn gate_system(mut desk: GateDesk, mut answers: MessageReader<PassiveAnswer>) {
    let Some((asked, _ask, _event)) = desk.gate.asking() else {
        // 没有信封挂着时，回答无处可去 —— 读掉它们，别让一句旧回答躺到下一个信封上
        for answer in answers.read() {
            debug!(
                "{:?} 答了一句没有人在问的话（门是空的）：{:?}",
                answer.player, answer.what
            );
        }
        return;
    };
    // 全部读完再决定：`find()` 会短路，没读完的回答下一帧又被读出来，同一问就答了两次
    let mut reply: Option<PassiveMove> = None;
    for answer in answers.read() {
        if answer.player == asked {
            reply = Some(answer.what);
        }
    }
    let Some(moved) = reply else {
        // 还没答：**这一支什么都不写**。窗口是 [`offer`] 在挂上那一问时写进去的，而把它
        // 清掉的只有两处 —— 这里换问时与 [`hand_back`] 收摊时，都在下面。
        // 每帧重写一次看着像保险，实际是抢 authority：`offer` 是无条件覆盖那一个槽的，
        // 于是「别的窗口合法地拿到了那一格」这种缺陷会被每天抹平一次，症状永远看不见。
        // （RC④：把这一句重写拿掉，全工程零红 —— 今天没有任何邻居会清它，所以它不是防线，
        // 只是把上面那条纪律写成了代码的反面。回合停住由 [`Gate::holds_turn`] 与那个槽一起保证。）
        return;
    };
    let Some(mut job) = desk.gate.queue.pop_front() else {
        return;
    };
    // 这一问有人答了：挂着那一格先摘掉。`answer_moves` 若要往第二问（免罪→嫁祸）会自己
    // 重新钉上，而那一格由 `pump` 的 `Out::Waiting` 负责 —— 两处各写一次就成了「谁清不掉」
    job.parked = false;
    if !answer_moves(&mut desk, &mut job, moved, asked) {
        // 这一问办完了还要问下一问：挂窗口走的是 `pump` 那一支**同一个** [`offer`]，
        // 而不是在这里照着旧主语再挂一次。差别只在嫁祸那一答上 —— 它把 `front` 里那一位
        // 换成了接住的人，沿用答话之前的 `asked` 就等于把窗口挂在**已经脱身的那位**身上：
        // 面板亮着、点下去 `gate_system` 说「没人被问」，而回合永久停住（10-27 冒烟实拍到的
        // 就是这一格，判据在 `tests/passive.rs` 的那条链用例里）
        job.parked = true;
        offer(&mut desk, &job);
        desk.gate.queue.push_front(job);
        return;
    }
    desk.gate.queue.push_front(job);
    pump(&mut desk);
}

/// 处理一句回答。返回 `true` = 这一次调用可以往下跑了，`false` = 还要再问一句。
///
/// **放弃 = 事件照原样落**（与拍卖的 `Withdraw` 同形），而放弃**不消耗卡**
/// （P3d④a ⑤ 那条纪律：卡没了事没成＝缺陷）。
fn answer_moves(desk: &mut GateDesk, job: &mut Job, moved: PassiveMove, asked: Entity) -> bool {
    let Some(aff) = job.front else {
        return true;
    };
    let event = aff.event;
    let from = aff.victim;
    match (job.ask, moved) {
        // 免罪：花掉那张，这件事不落下来
        (Ask::Offset, PassiveMove::Use) => {
            let Some(card) = desk.holds(from, expect_card(event.offset_card())) else {
                warn!("{asked:?} 被问了一句他答不了的话：手里那张抵消卡不见了");
                job.front = None;
                return true;
            };
            let Some(notice) = desk.spend(from, card) else {
                job.front = None;
                return true;
            };
            desk.used.write(PassiveUsed {
                player: from,
                card: notice,
                event,
                moved_to: None,
            });
            job.done.push(EffectOutcome::Blocked {
                event,
                by: Blocker::Card,
            });
            job.front = None;
            true
        }
        // 嫁祸：换承受者，然后**回到第一问**（接住的那位照样可以被问免罪 ——
        // PLAN §五 第 3 条那句「这不是要拍的例外，而是门的自然行为」）
        (Ask::Shift, PassiveMove::Shift { to }) => {
            if !desk.shift_candidates(from).contains(&to) {
                debug!("嫁祸被拒：{to:?} 不在可点名的名单上");
                return true;
            }
            let Some(card) = desk.holds(from, PassiveCard::Shift) else {
                warn!("{asked:?} 被问了一句他答不了的话：手里那张嫁祸卡不见了");
                job.front = None;
                return true;
            };
            let Some(notice) = desk.spend(from, card) else {
                job.front = None;
                return true;
            };
            desk.used.write(PassiveUsed {
                player: from,
                card: notice,
                event,
                moved_to: Some(to),
            });
            desk.blame.write(BlameShifted { from, to, event });
            // 改的是 `front` 里那一位，不是 `job.victim`：一张卡的两件事不该因为
            // 第一件被转走就整个换人（后面那些仍然打在原来那位头上）
            job.front = Some(Affliction {
                event,
                victim: to,
                ..aff
            });
            job.shifted = true;
            // 新承受者先问抵消那一问；他自己「往下转」的那一问被他身上那一位
            // 「已转过一次」挡掉 —— 这就是「链最长两跳」的全部内容，不是一条特例
            job.ask = Ask::Offset;
            !next_question(desk, job)
        }
        // 放弃第一问：还有第二问要问（手里没嫁祸卡时 `next_question` 自己会答「不用问」）
        (Ask::Offset, PassiveMove::Decline) => {
            job.ask = Ask::Shift;
            !next_question(desk, job)
        }
        // 放弃第二问：什么都不花，往下走（落）
        (Ask::Shift, PassiveMove::Decline) => true,
        // 认不出来的那几格（面板误发、或门问「免不免」而回答是「嫁祸给谁」）：
        // **这一问继续挂着**。返回 `false` 而不是 `true` 是要害 —— 落在 `true` 那一支上，
        // 门会替他把这件事认下来，而那句「替他答」猜错了方向，比一句不响的窗口难查得多
        (Ask::Offset, PassiveMove::Shift { .. }) | (Ask::Shift, PassiveMove::Use) => false,
    }
}

/// 门空了以后收掉自己那格窗口，并把回合交还给挂住之前那一格。
///
/// 与拍卖散场那一条同形：**谁清窗口谁交回合**（坑 38），只 `clear` 会让回合机停在
/// `WaitDecision` 且不报错。三句各有分工：
/// - `resume` 读不到 = 门根本没押过回合（`Landed` 那条链里，回合是 `close_landing_system`
///   挂的），这里一句都不许写 —— 写了就在同一帧里与它抢 `NextState`（坑 37）；
/// - 槽里还是 `Passive` 才清：收尾派回原路时 [`crate::deck::finish_drawn`] 可能已经把
///   那张卡面收据挂上去了，那格窗口不是门的；
/// - 还挂着别的窗口就**不交回合**，交给那个窗口的主人去交（同上，别两处各写一次）。
fn hand_back(desk: &mut GateDesk) {
    let Some(return_to) = desk.gate.resume.take() else {
        return;
    };
    if matches!(desk.pending.active(), Some((_, Pending::Passive))) {
        desk.pending.clear();
    }
    if desk.pending.active().is_some() || desk.gate.holds_turn() {
        return;
    }
    desk.next.set(return_to);
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

    /// [`Gate::asking`] 认的是 [`Job::parked`] 那一格，而不是「最前面那份手上有没有事」。
    ///
    /// 这一格今天只有这一个可观察的形状，所以它得由函数本身钉住：`one()` 在「这一行不用问」
    /// 那一条路上也会填 `front`（下一轮 `land_now` 落的就是它），而那一格**不是**一问。
    /// 拿它当判据的话，症状是「有人在答」在面板、dump 与 `gate_system` 三处同时说错话，
    /// 而世界上没有任何人在等回答。
    #[test]
    fn a_job_being_run_is_not_a_job_being_asked() {
        let victim = entity(1);
        let mut job = Job::new(
            victim,
            Vec::new(),
            "test".to_string(),
            None,
            Gavel::Public,
            Tail::Played {
                card: CardId(0),
                player: victim,
            },
        );
        job.front = Some(Affliction::detain(victim, DetentionPlace::Jail, 3));
        let mut gate = Gate {
            queue: std::collections::VecDeque::from(vec![job]),
            resume: None,
        };

        assert_eq!(
            gate.asking(),
            None,
            "手上有一件事不等于有人在答 —— `parked` 那一格不能省"
        );
        assert!(
            gate.holds_turn(),
            "而回合确实还押着：这两个读数说的是两件事，别把它们合成一个"
        );

        if let Some(front) = gate.queue.front_mut() {
            front.parked = true;
        }
        assert_eq!(
            gate.asking(),
            Some((victim, Ask::Offset, E::JailTrip)),
            "钉上那一格才算问出去了"
        );
    }
}
