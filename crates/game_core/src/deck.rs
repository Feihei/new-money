//! 牌堆：机会 / 命运两副牌（`card_deck.gd`）。
//!
//! 卡定义是静态资产（`Assets<CardSet>`），牌堆实体只持 **下标**（`CardId`），
//! 于是「洗牌」是排下标，不复制卡面数据，也不会出现两份卡面真相。
//!
//! 与 Godot 有两处实质差别：
//! - 洗牌用全局 `_rng.randomize()`（`card_deck.gd:53`）不可复现（缺陷 #6），这里改用 `Rng`
//!   的**牌堆子流** —— 同种子复现同一副顺序，而抽卡不会挪动骰子流（见 `rng.rs` 的分流理由）。
//! - 命运那副**不再洗**：PLAN §三 把 4 代的命运记成「固定顺序循环」，于是发法成了牌堆自己的
//!   一个位（[`DeckOrder`]），而不是散在抽卡处的一个 `if`。机会照旧随机。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::afflict::{AfflictDomain, Affliction, Insulation};
use crate::board::TileIndex;
use crate::card::{CardDef, CardEffect, CardKind, CardSet, LevyBasis};
use crate::charge::{Adjustment, Charge, ChargeDomain, ChargeKind, Correction, Scope, Source};
use crate::data::DataHandles;
use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision, detain, teleport};
use crate::msg::{
    CardDrawn, CardNotice, DetentionLifted, EffectOutcome, LandBought, PlayerDetained,
    PlayerTeleported,
};
use crate::occupancy::{Occupancy, OccupancyAt, OccupancyKind};
use crate::player::{
    Detention, Heading, Mount, MoveCap, PlayerId, StaySentence, Term, TokenPosition, Walker,
};
use crate::rng::Rng;
use crate::settle::Books;

/// 牌堆里的一张牌 = 它在对应 `CardSet` 里的下标。
///
/// 用下标而不是 `String` id：卡池是加载后不变的静态数组，下标即身份，
/// 存盘时随 `GameSeed` 一起足够重建（§九）；而 `CardNotice` 给 UI 的是 id 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CardId(pub u32);

/// 一副牌怎么发。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeckOrder {
    /// 开局洗一次，抽穿再洗（机会格 —— 4 代那里也是随机的）。
    Shuffled,
    /// 按**数据顺序**循环发，抽穿回到第一张（命运格，PLAN §三「固定顺序循环」）。
    ///
    /// 这一支不读随机流：顺序完全由 `fate_cards.json` 的行序决定。所以「连踩四次命运能
    /// 数出顺序」这条验收与种子无关，而存档也只要存「发到第几张」。
    Fixed,
}

/// 每种牌怎么发。**穷尽表，不许出现 `_`** —— 与 [`crate::landing::domain_of`]、
/// [`crate::charge::scales_with_price`] 同一条纪律：新增一副池必须当场决定它的发法。
pub const fn order_of(kind: CardKind) -> DeckOrder {
    match kind {
        CardKind::Chance => DeckOrder::Shuffled,
        CardKind::Fate => DeckOrder::Fixed,
        // 道具卡没有牌堆（它的来源是货架，`settle_card_landing` 对 `Item` 直接 return）。
        // 这一格写什么都行，写在这里是为了「哪天给道具卡开一格」时当场暴露这个决定还没做。
        CardKind::Item => DeckOrder::Shuffled,
    }
}

/// 一副可循环抽的牌。存在即「这一堆牌」，机会 / 命运各一个实体。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Deck {
    pub kind: CardKind,
    /// 发法。由 [`order_of`] 在装配时定，之后不变。
    pub order: DeckOrder,
    /// 还能抽的牌，`last` 是堆顶（与 `card_deck.gd:63` 的 `pop_back()` 一致）。
    pub draw_pile: Vec<CardId>,
    /// 已抽出的牌。抽穿后整副重发（`card_deck.gd:59-62` 的自动洗牌）。
    pub discard: Vec<CardId>,
}

impl Deck {
    /// 按 `kind` 的发法装配一副满牌（开局用）。
    pub fn dealt(kind: CardKind, cards: &CardSet, rng: &mut Rng) -> Self {
        let n = cards.0.len();
        let mut deck = Self {
            kind,
            order: order_of(kind),
            draw_pile: Vec::new(),
            discard: Vec::new(),
        };
        deck.deal(n, rng);
        deck
    }

    pub fn remaining(&self) -> usize {
        self.draw_pile.len()
    }

    pub fn total(&self) -> usize {
        self.draw_pile.len() + self.discard.len()
    }

    /// 发满 `n` 张。开局装配与抽穿重发走**同一条**，所以「循环」这件事只有一处实现。
    ///
    /// `Fixed` 那支按索引**倒序**入堆：`draw_pile` 从堆尾弹出，倒着放才正着发。
    fn deal(&mut self, n: usize, rng: &mut Rng) {
        self.discard.clear();
        self.draw_pile = match self.order {
            DeckOrder::Fixed => (0..n as u32).rev().map(CardId).collect(),
            DeckOrder::Shuffled => {
                let mut pile: Vec<CardId> = (0..n as u32).map(CardId).collect();
                fisher_yates(&mut pile, rng);
                pile
            }
        };
    }

    /// 整副重洗：恢复成完整的 n 张再打乱（`card_deck.gd:48-54` 的 `shuffle()` 就是这个，
    /// 不是「弃牌堆倒回去」—— 弃牌堆只是记账，供存档与回放看「这局抽过哪些牌」。
    /// 定序那副牌没有「重洗」这回事，走到这里会直接按顺序重发。
    pub fn shuffle(&mut self, rng: &mut Rng) {
        let n = self.total();
        self.deal(n, rng);
    }

    /// 抽一张。牌堆空了先自动重发（Godot 的 `test_deck_auto_reshuffle_when_empty`）。
    /// 空牌堆（一副都没有）返回 `None` —— 数据错误由 `CardSet::validate` 拦在加载期。
    pub fn draw(&mut self, rng: &mut Rng) -> Option<CardId> {
        if self.draw_pile.is_empty() {
            if self.total() == 0 {
                return None;
            }
            self.shuffle(rng);
        }
        let card = self.draw_pile.pop()?;
        self.discard.push(card);
        Some(card)
    }
}

/// 种子化 Fisher-Yates：从堆尾往前，每个位置与本机之前的某处交换。
fn fisher_yates(pile: &mut [CardId], rng: &mut Rng) {
    for i in (1..pile.len()).rev() {
        let j = rng.pick(0..=i as u32) as usize;
        pile.swap(i, j);
    }
}

/// 牌堆 → 卡定义的查询（牌堆只存下标，卡面在资产里）。
pub fn card_of(set: &CardSet, id: CardId) -> Option<&CardDef> {
    set.0.get(id.0 as usize)
}

/// 三副卡池的查询入口：「读哪三个文件」+「加载出来的资产」。
///
/// 这两样永远成对出现 —— 下标只有在**正确的那副池**里才指得对一张卡，所以打包成一个
/// 参数包而不是让每个调用方各拿两份（`CardSink` 同一条房规，§六 坑 8）。
/// `kind → handle` 这张分派表写在这里一次，抽卡与出牌两条入口共用它。
#[derive(SystemParam)]
pub struct Shelf<'w> {
    sets: Res<'w, Assets<CardSet>>,
    data: Res<'w, DataHandles>,
}

impl<'w> Shelf<'w> {
    /// 某副池的整副卡（开局校验、牌堆装配、货架用）。
    pub fn set(&self, kind: CardKind) -> Option<&CardSet> {
        let handle = self.handle(kind);
        self.sets.get(handle)
    }

    /// 某副池里的一张卡。
    pub fn get(&self, kind: CardKind, id: CardId) -> Option<&CardDef> {
        self.set(kind).and_then(|set| card_of(set, id))
    }

    /// 整副货架（`Item` 池）。百货公司列货位用的就是这一张。
    pub fn items(&self) -> &[CardDef] {
        self.set(CardKind::Item).map_or(&[], |set| &set.0)
    }

    fn handle(&self, kind: CardKind) -> &Handle<CardSet> {
        match kind {
            CardKind::Chance => &self.data.chance,
            CardKind::Fate => &self.data.fate,
            // 道具卡没有对应的格子（`domain_of` 不产出 `Card(Item)`），它的「池」是货架。
            // 这一支行文与上面两行同形，是为了让「哪天给道具卡开一格」这件事在
            // 分派表上留痕，而不是靠调用方记得去拿 `data.items`
            CardKind::Item => &self.data.items,
        }
    }
}

// ===== 效果的执行：抽到与打出共用同一条 =====

/// 结算一张卡要动的全部东西。打包成参数包是这仓库的房规（§六 坑 8），而这张包正好
/// 也是**两条入口共用的那一份**：踩到格子抽卡（[`settle_card_landing`]）与玩家主动
/// 打出一张卡（[`crate::hand::play_card_system`]）走的是同一个 [`apply_card`]。
///
/// 这不是顺手复用，是刻意的：一张卡的效果只有一套语义。两条入口各写一遍的话，
/// 「陷害卡把对手关进去」与「命运牌把自己关进去」就会在某一天开始各修一半。
#[derive(SystemParam)]
pub struct CardSink<'w, 's> {
    pub books: Books<'w, 's>,
    pub commands: Commands<'w, 's>,
    /// 位置（P12 起按 `Walker` 收）。传给 [`crate::landing::detain`] 的签名要的是行动者那一族，
    /// 而这一族里今天会被卡点到的仍然只有玩家 —— 恶人不在 `CardTarget` 的候选名单上。
    pub positions: Query<'w, 's, &'static mut TokenPosition, With<Walker>>,
    pub insulations: Query<'w, 's, &'static mut Insulation>,
    /// 身上挂着哪位神（P6b②）。**只读**，而且它不是效果的存放处 —— 神给的免疫与天数
    /// 加倍每次都现读这一格（见 [`crate::gods::resists`]），因为「附身期间」这件事的
    /// 真相只有 `Possession` 一份，抄进 `Insulation` 就会有两份到期时间。
    pub possessions: Query<'w, 's, &'static crate::gods::Possession, With<PlayerId>>,
    /// 绕行方向。转向卡要**就地**翻它（见 [`apply_card`] 里那一支），所以是 `&mut` 查询
    /// 而不是 `Commands`。
    pub headings: Query<'w, 's, &'static mut Heading, With<PlayerId>>,
    /// 拘留计时**不在这里**，在 `sink.books.detained`（P4②）：账本要读得到「谁被关着」才填得出
    /// `LandView.owner_detained`，而同一个 system 里对 `Detention` 一读一写会直接撞成借用冲突
    /// （与本文件头那句「三块都要 `&mut Wallet` 就得集中一处」同一条理由）。就地写那条不变，
    /// 见 [`crate::landing::detain`]。
    /// 停留计时。与 `headings` 同样是**就地**写（见 [`apply_card`] 的 `Stay` 那一支）：
    /// 第二张停留卡要看得见第一张，而这里读出来的数还要原样报进凭据。
    pub stays: Query<'w, 's, &'static mut StaySentence>,
    /// 乌龟卡的计时（P5）。与 `stays` 同样就地写：第二张要看得见第一张的剩余回合。
    pub caps: Query<'w, 's, &'static mut MoveCap>,
    /// 他骑的是什么（P5）。**只读**，而且只有新闻那条路需要它：`NewsScope::EveryoneIf` 的
    /// 「步行的人／有车的人」那一筛（P11b② 的豪雨与交通堵塞）。
    ///
    /// 为什么在 `CardSink` 上而不是塞进 `Books`：账本要读的是钱与地，「他今天开不开车」
    /// 不参与任何一条结算判据（`Mount` 只决定摇几颗骰子）。塞进 `Books` 的话，
    /// 「谁被关着」那一格就有个邻居要说不出为什么在这儿。
    pub mounts: Query<'w, 's, &'static Mount, With<PlayerId>>,
    pub detained: MessageWriter<'w, PlayerDetained>,
    /// 放人那一条（P11b② 的新闻「提前出院」）。与**期满同一条凭据**：世界效果那半
    /// （摘组件 + 接入点）只有一份答案，差别只在谁开的门 —— 那条写在 `msg::EffectOutcome` 上。
    pub released: MessageWriter<'w, crate::msg::DetentionLifted>,
    pub teleported: MessageWriter<'w, PlayerTeleported>,
    /// 买地成交的流水。购地卡走的是 `Books::buy` 那条正路，所以它必须和「踩到无主地买下来」
    /// 发同一条凭据 —— 归属变了这件事只有一种说法，而顶栏与 dump 读的就是这一条。
    pub bought: MessageWriter<'w, LandBought>,
    /// 路上站着的东西（P6d 的两张符要读它：请神符找最近那位、送神符带走身上那颗炸弹）。
    ///
    /// **只读**：请神符把那位神从路上抹掉走的是 `Commands`（与落脚那条
    /// [`settle_god_landing_system`](crate::gods::settle_god_landing_system) 同一个形状），
    /// 而出牌发生在 `WaitRoll`、路上没有任何东西在同一帧改这些实体 —— 要 `&mut` 只会让
    /// 这张包与漫游／放置那几条 system 撞起来。
    pub occupancies: Query<'w, 's, (Entity, &'static Occupancy)>,
    /// 上身那一刻要动的非账本那几样（随机流、货架、手牌、四条凭据）。
    ///
    /// **为什么在 `CardSink` 上而不是让出牌那条 system 另拿一个 `GodDesk`**：`GodDesk` 带着
    /// `Books` 与 `Commands`，与这张包里的同两格在参数建立期直接撞（P4② 那条同一个坑）。
    /// P6d 的请神符要在出牌时上身，所以拆的是包不是判据 —— `attach` 收零件，谁凑谁的事。
    /// 连带后果：`play_card_system` 与 `settle_card_landing` 不再自己拿 `hands`／`shelf`，
    /// 它们读 `sink.kit` 的那两份（同一 system 里两条 `&mut Hand` 同样会撞）。
    pub kit: crate::gods::BestowKit<'w, 's>,
}

/// 免费卡免的是哪几族钱（PLAN §三「**免费**抵 >2000×物价 的租金/罚金/税」）。
///
/// ✅ P11b② 那三条里的「税」真的来了（`ChargeKind::Levy`），而它落在 `Penalty` 那一族 ——
/// 这张卡不必改一行就覆盖到它，正是当初把域写成 `Penalty` 而不是 `Fine` 的理由。
/// **不含 `Acquire`（买地、加盖）**：「买地免费」是大福神的特权（PLAN §三 神明行），
/// 两张卡各自有价，合成一个域就分不开了。
const FREE_PASS_DOMAINS: &[ChargeDomain] = &[ChargeDomain::Rent, ChargeDomain::Penalty];

/// 把一张卡的全部效果打在 `victim` 头上，返回每条效果的凭据给 UI 拼文案。
///
/// `victim` 是参数而不是「抽卡的人」，是因为同一张卡面既可能落在自己身上（命运的
/// 「直接入狱」）也可能落在对手身上（陷害卡）—— 区别只在 `CardDef.target`，
/// 不在效果里。三处刻意的规则：
/// - **扣不到钱就真破产**（缺陷 #3）：`CouldNotPay` 与 `PlayerBankrupt` 同帧成对，
///   且卡上剩下的效果不再执行 —— 人已经离场。
/// - **`move_to` 不连锁结算新格子**：Godot 端踩到「回到起点」也不会领过路费奖金，
///   这里照做。连锁会引入「卡牌把玩家送到卡牌格」这类环，属于设计而非移植。
/// - **`go_to_jail` 先过护身**（P3a）：被抵消的那次不会留下半条入狱流水。
///
/// 为什么同时带 `player` 与 `victim` 两个人：绝大多数效果只用得到其中一个（抽卡那条路
/// 两者就是同一个人），而 [`crate::card::CardEffect::Auction`]（P9）的**主语与承受者天生
/// 不是同一个人** —— 开槌的是出牌人（他不参与竞价、拍出的钱归他），被拍的是别人手里的地。
/// 为它一个去给 `CardSink` 加「当前回合玩家」之类的位置参数是把判据塞错地方：
/// 出牌人与回合玩家在两处判据里本来就可能不一致（P3a 那条「主语只认当前回合玩家」写在
/// `play_card_system`，不在这里）。
///
/// `player` 到这里只活成一件事：[`crate::auction::Gavel::Card`] 的那一位。再往下传给
/// `apply_effects` 就成了一个没人读的形参（P11c① 之后编译器会直接报 unused），所以它在
/// 这一层就被收进槌里。
pub fn apply_card(
    sink: &mut CardSink,
    player: Entity,
    victim: Entity,
    def: &CardDef,
    aim: Option<crate::hand::CardAim>,
) -> Vec<EffectOutcome> {
    apply_effects(
        sink,
        victim,
        &def.effects,
        &def.id,
        aim,
        crate::auction::Gavel::Card(player),
    )
}

/// 一组效果打在 `victim` 头上，返回每条效果的凭据。上面那两条入口（抽卡 / 出牌）之外，
/// P11 的**新闻池**是第三个调用方（[`crate::news::settle_news_landing_system`]）——
/// 那不是顺手复用，是 PLAN §四 排序原则第 3 条「命运 / 新闻 / 魔法屋不各写一套效果」
/// 的落地方式：把这张 `match` 复制一份的代价，是「同一句 `detain` 在两个文件里各修一半」，
/// 而那种分叉不会崩，只会变成玩家口中的「命运里的住院三天和新闻里的不一样长」。
///
/// `source` 是「哪条数据要办这件事」（卡 id 或新闻 id），只进日志，不进判据：三条入口的
/// id 各有各的命名空间，而一条效果的含义不该随它从哪副池里来而变。
/// `effects` 收切片而不是收 `&CardDef`：新闻项与卡面的字段表本来就不同（新闻带 `scope`、
/// 不带 `kind`/`target`/`cost_points`），共用的是**动词**那一层，不是「一条数据」那一层。
///
/// `gavel` 只有一条动词读它（[`crate::card::CardEffect::Auction`]），而那条的**含义**确实随
/// 来路变：同一句 `auction`，从拍卖卡来是「拍别人手里那块地、钱归我」，从新闻中心来是
/// 「把这一带的公有地挂牌、钱归银行」。由调用方显式交进来而不是在这里猜（「开槌人==承受者」
/// 那种巧合在两条路径下都成立过，而第三条入口一加它就会静默选错一侧）。
pub fn apply_effects(
    sink: &mut CardSink,
    victim: Entity,
    effects: &[CardEffect],
    source: &str,
    aim: Option<crate::hand::CardAim>,
    gavel: crate::auction::Gavel,
) -> Vec<EffectOutcome> {
    let mut outcomes = Vec::new();
    for effect in effects {
        match effect {
            CardEffect::AddCash { amount } => {
                // 走管线：物价 2.5 时这张「横财」卡真的多给钱，而衰神的「得钱作废」
                // 是一条 Cancel 就能勾掉的账（所以 Bonus 的 `cancellable` 是开着的）
                let charge = Charge::new(ChargeKind::Bonus, None, Some(victim), *amount);
                let got = sink
                    .books
                    .settle(&mut sink.commands, charge)
                    .map(|done| done.amount)
                    .unwrap_or(0);
                outcomes.push(EffectOutcome::Gained(got));
            }
            CardEffect::DeductCash { amount } => {
                let charge = Charge::new(ChargeKind::Penalty, Some(victim), None, *amount);
                match sink.books.settle(&mut sink.commands, charge) {
                    Ok(done) => outcomes.push(EffectOutcome::Lost(done.amount)),
                    Err(short) => {
                        outcomes.push(EffectOutcome::CouldNotPay {
                            needed: short.needed,
                        });
                        // 人已破产，卡上剩下的效果不必再执行
                        break;
                    }
                }
            }
            CardEffect::Levy { basis, percent } => {
                // 基数是**此刻**的数（他手上有多少钱、名下那些地今天值多少），不是条目上的
                // 名义数 —— 所以这条在「缩不缩物价」那一格填「不缩」：基数已经含今天的价，
                // 再乘一次指数就成了「物价 2 倍时 10% 的税悄悄变 20%」（见 `charge::scales_with_price`）
                let base = match basis {
                    // 现金不会是负的（`debit` 挡在破产那一侧），`max(0)` 挡的是「负基数
                    // 乘出一个负税率、于是税变成一笔收入」这种形状
                    LevyBasis::Cash => sink.books.balance(victim).max(0),
                    LevyBasis::LandValue => sink.books.land_value_of(victim),
                };
                let amount = base.saturating_mul(*percent as i64) / 100;
                let charge = Charge::new(ChargeKind::Levy, Some(victim), None, amount);
                match sink.books.settle(&mut sink.commands, charge) {
                    // 0 元税照样留一条 `Lost(0)`：名下没地的人被抽地价税，「什么都没发生」与
                    // 「这条新闻是空的」在玩家眼里是两件事，流水要说得出是前者
                    Ok(done) => outcomes.push(EffectOutcome::Lost(done.amount)),
                    Err(short) => {
                        outcomes.push(EffectOutcome::CouldNotPay {
                            needed: short.needed,
                        });
                        break;
                    }
                }
            }
            CardEffect::Detain { place, turns } => {
                let affliction = Affliction::new(AfflictDomain::Detention, victim);
                if resist(sink, &affliction) {
                    outcomes.push(EffectOutcome::Blocked {
                        domain: affliction.domain,
                    });
                    continue;
                }
                let days = doom_days_for(sink, victim, *turns);
                // 接入点与死胡同格由 `detain` 一处算（三个去处共用）。它查不到那条分支时
                // 什么都不改并返回 `None` —— 与「数据写了个棋盘上没有的格名」同一条纪律：
                // 不猜一个格子，也不回退到起点。返回的那条拘留**可能是原来更久的那一张**，
                // 所以凭据一律用它给的值，而不是算完的那个倍数
                let Some((held, junction, end)) = detain(
                    &mut sink.commands,
                    &mut sink.positions,
                    &mut sink.books.detained,
                    &sink.books.active.def,
                    victim,
                    *place,
                    // 卡面写几天就是几天（10-20 拍板「天数只来自卡面」）。恶人专用的那一支
                    // `UntilBailed` 从这里出不去：卡打不到恶人，而「踩到某格就被关」也没有
                    Term::Turns(days),
                ) else {
                    debug!(
                        "这张棋盘没有「{}」那条分支，把 {} 送去那里这条路落空",
                        place.label(),
                        source
                    );
                    continue;
                };
                sink.detained.write(PlayerDetained {
                    player: victim,
                    place: held.place,
                    junction,
                    end,
                });
                outcomes.push(EffectOutcome::Detained {
                    place: held.place,
                    // 读的是 `detain` 返回的那一张而不是 `days`：更久的旧账赢。
                    // 「赢的会不会是笼中那一张（读不出数）」这条路今天走不到 ——
                    // 卡与新闻都打不到恶人（`CardTarget` 与 `NewsAudience` 的承受者都是玩家），
                    // 所以兜底取的正是卡上写的那个数
                    turns: held.term.turns().unwrap_or(days),
                    junction,
                });
            }
            CardEffect::Release { place } => {
                // 只放「确实关在这一处」的人。读的是 `books.detained`（P4② 之后账本读得到
                // 谁被关着），而作用域那一档已经按处筛过一遍（`news::mismatch` 拦的就是
                // 「筛医院、放监狱」这种写法）—— 两道之间若哪天分叉，这里是不发凭据的那一道：
                // 宁可这一条新闻看起来没放到人，也不要凭空把一个人从监狱里放出去
                let Some(held) = sink.books.detained.get(victim).ok().copied() else {
                    debug!("{} 要放人，可 {victim:?} 身上没有拘留计时", source);
                    continue;
                };
                if held.place != *place {
                    debug!(
                        "{} 放的是「{}」，而他关在「{}」：两道判据分叉了",
                        source,
                        place.label(),
                        held.place.label()
                    );
                    continue;
                }
                // 接入点 = 他此刻那一格。`detain` 早把他挪到了那条分支的接入点上，
                // 所以这里不搬棋子、只报那一格（与期满同一条出口，见 `msg::DetentionLifted`）
                let junction = sink
                    .positions
                    .get(victim)
                    .map(|p| p.tile)
                    .unwrap_or(TileIndex(0));
                // 摘组件走 `Commands`（帧末生效）。同一帧连放好几个人是安全的：
                // 名单在 `fire_news` 那里一次算完，而本帧内没有第二条路再读「他还关着吗」
                sink.commands.entity(victim).remove::<Detention>();
                sink.released.write(DetentionLifted {
                    player: victim,
                    place: held.place,
                    junction,
                });
                outcomes.push(EffectOutcome::Released {
                    place: held.place,
                    junction,
                });
            }
            CardEffect::MoveTo { tile_name } => {
                let target = sink
                    .books
                    .active
                    .def
                    .tiles
                    .iter()
                    .position(|t| t.name == *tile_name)
                    .map(|idx| TileIndex(idx as u32));
                match target {
                    Some(tile) => {
                        teleport(&mut sink.positions, victim, tile);
                        sink.teleported.write(PlayerTeleported {
                            player: victim,
                            tile,
                        });
                        outcomes.push(EffectOutcome::Teleported {
                            tile,
                            name: tile_name.clone(),
                        });
                    }
                    // 数据写了个棋盘上没有的格名：不猜、也不退回起点
                    // （`card_effects.gd:76-81` 已经修过一次「回退到 0=起点」的坑）
                    None => outcomes.push(EffectOutcome::UnknownTile {
                        name: tile_name.clone(),
                    }),
                }
            }
            CardEffect::Insulate { charges } => {
                // 护身叠层：两张免罪卡 = 抵两次。神明那一半不在这个组件里（见
                // [`crate::afflict::Insulation`] 的注），所以这里没有「不限次」要保护
                let stacked = match sink.insulations.get_mut(victim) {
                    Ok(mut existing) => {
                        existing.charges = existing.charges.saturating_add(*charges);
                        existing.charges
                    }
                    Err(_) => {
                        sink.commands
                            .entity(victim)
                            .insert(Insulation { charges: *charges });
                        *charges
                    }
                };
                outcomes.push(EffectOutcome::Shielded { charges: stacked });
            }
            CardEffect::LandStatus { status, days, area } => {
                // 锚点来自**请求里的目标**，不在卡面上：同一张「涨价卡」打在南京路还是
                // 上园路是玩家按下去那一刻才定的。抽卡那条路（`settle_card_landing`）
                // 永远没有 aim，所以数据若把这种效果写进机会/命运池，这里就是落空 ——
                // `CardSet::validate` 已经在加载期拦了（要求 target = land），这一句
                // debug 是第二道：坏数据不该静默改着游戏状态。
                let Some(crate::hand::CardAim::Land(center)) = aim else {
                    debug!("{} 要盖地块状态，但这次没有地块目标", source);
                    continue;
                };
                let lands =
                    sink.books
                        .stamp_status(&mut sink.commands, center, *status, *area, *days);
                outcomes.push(EffectOutcome::Stamped {
                    status: *status,
                    lands,
                    days: *days,
                });
            }
            CardEffect::Damage { depth, area } => {
                // 与 `LandStatus` 同一条：锚点来自 aim（新闻给的是播报者脚下那块地，
                // 卡给的是玩家点的那块），没有锚就落空而不猜一块 —— 猜哪一块都是替玩家做决定
                let Some(crate::hand::CardAim::Land(center)) = aim else {
                    debug!("{} 要打建筑损害，但这次没有地块目标", source);
                    continue;
                };
                // 清单先问再打：地块类新闻要把「拆了哪几块」念出来（见 `Books::area_of`）
                let lands = sink.books.area_of(center, *area);
                let n = lands.len() as u32;
                let tally = sink.books.damage_lands(&lands, *depth);
                outcomes.push(EffectOutcome::Razed {
                    lands: n,
                    seized: tally.seized,
                });
            }
            CardEffect::FreePass { floor } => {
                // 登记一条**带门槛的一次性 `Cancel`**。三样信息各有归属：门槛与「免哪几族钱」
                // 是这张卡的玩法（所以写在这里，不写在管线里），而「什么时候算用掉」是世界的
                // 行为（所以由 `Books::settle` 在生效那一笔注销，这里不承诺寿命）。
                //
                // 打出去那一刻**什么都不发生** —— 没有流水金额、没有组件、屏幕上没有变化。
                // 这不是静默失效：`FreePassArmed` 那条凭据就是「从今天起可以免一次」，
                // 而它值多少钱取决于下一笔多大的账先来。
                sink.books.register(Correction::one_shot(
                    victim,
                    Scope::domains(FREE_PASS_DOMAINS),
                    Adjustment::Cancel,
                    Source::Card,
                    *floor,
                ));
                outcomes.push(EffectOutcome::FreePassArmed { floor: *floor });
            }
            CardEffect::BuyLand => {
                // 要买哪块地同样来自**请求**，不在卡面上（与 `LandStatus` 的锚点同一条理由）。
                // 抽卡那条路没有 aim，所以数据把这张卡写进机会/命运池就是静默失效 ——
                // `validate` 已在加载期要求 `target: "unowned_land"`，这一句是第二道
                let Some(crate::hand::CardAim::Land(land)) = aim else {
                    debug!("{} 要买地，但这次没有地块目标", source);
                    continue;
                };
                // 买这一句走的是账本的正路：地价过物价、过小福神的「买地免费」，
                // 扣的是实付额，归属与等级也由 `Books::buy` 一处改
                match sink.books.buy(victim, land) {
                    Some(price) => {
                        sink.bought.write(LandBought {
                            player: victim,
                            land,
                            price,
                        });
                        outcomes.push(EffectOutcome::Purchased { land, price });
                    }
                    // 出牌那道门（`aim_is_legal`）读的正是 `buy` 读的同一个价，同一帧同一份
                    // 账本 —— 走到这里只剩一种可能：两道判据哪天开始分叉了。那是 bug，
                    // 不是玩家能遇到的处境，所以报给日志而不是编一条永远看不见的凭据
                    None => warn!("{} 打出去了却没买下这块地：出牌判据与账本分叉了", source),
                }
            }
            CardEffect::Auction { area } => {
                // 拍哪块地同样来自**请求**，不在卡面上（与 `BuyLand` 同一条理由）。
                // 出牌那道门已经问过 `can_open`（`unmet_precondition`），所以走到这里
                // 只剩「把这一槌许下去」—— 真正开场由 `auction::auction_hold_system` 做，
                // 因为它才拿得到座位序（`TurnOrder`），而「先问谁」是拍卖机的事不是这张卡的事
                let Some(crate::hand::CardAim::Land(center)) = aim else {
                    debug!("{} 要拍卖地，但这次没有地块目标", source);
                    continue;
                };
                let lands = match gavel {
                    // 卡那一槌：只拍挑中那一块（`area` 在卡面数据里缺省成 `One`），
                    // 而它走 `requested` 那一条快道 —— 拍卖卡是掷骰前打的，那一槌排在
                    // 任何清算之前（`next_lot` 的优先级）
                    crate::auction::Gavel::Card(founder) => {
                        sink.books.auction.requested = Some(crate::auction::Listing {
                            land: center,
                            gavel: crate::auction::Gavel::Card(founder),
                        });
                        vec![center]
                    }
                    // 新闻那条「公开拍卖公有土地」：范围内**本来无主**的每一块各挂一场，
                    // 排队由拍卖机自己做（一次一块），所得归银行
                    crate::auction::Gavel::Public => {
                        let found = sink.books.public_lands(center, *area);
                        for land in found.clone() {
                            sink.books
                                .auction
                                .standby
                                .push_back(crate::auction::Listing { land, gavel });
                        }
                        found
                    }
                    // 清算那一槌不由任何一条数据发出：它是 `Books::bankrupt` 直接写
                    // `liquidation` 那份料的结果，走的是拍卖机自己的路。这一格不许省，
                    // 否则「哪条池子里真出现清算拍卖」这件事会在某次改动后静默选成 `Public`
                    crate::auction::Gavel::Liquidation(_) => {
                        warn!(
                            "{} 要替一场清算敲槌：清算的上拍由拍卖机自己排，不该经过这里",
                            source
                        );
                        Vec::new()
                    }
                };
                outcomes.push(EffectOutcome::Auctioned { lands });
            }
            CardEffect::Suspend { what, days } => {
                // 三条来路（卡／命运／新闻）里今天只有新闻走得通：`CardSet::validate` 把卡挡在
                // 池外，而命运与机会池没有条目带它。这一格因此**不读 aim、不读 victim** ——
                // 它改的是一条规矩，谁站在这儿都一样。
                //
                // 这里**不推凭据**：机构档的承受者既不是人也不是地，那条新闻的「办成了什么」
                // 全在 `NewsAudience::Institution{what, days}` 那一格里（一家、几天）。
                // 再发一条 `EffectOutcome` 就是同一个数存两处，而两处会各自漂
                let _ = sink.books.session.suspensions.suspend(*what, *days);
            }
            CardEffect::Reverse => {
                // 与入狱同一条形状：它先过护身。被抵消的那次不该留下半个掉头 ——
                // 「方向没变」与「方向变了又变回来」在流水上是两件事
                let affliction = Affliction::new(AfflictDomain::Reversal, victim);
                if resist(sink, &affliction) {
                    outcomes.push(EffectOutcome::Blocked {
                        domain: affliction.domain,
                    });
                    continue;
                }
                // 就地翻，不走 `Commands`：同一帧两张转向卡**该**翻两次（翻两次=原样），
                // 而延迟写会让第二条读不到第一条 —— 与 P3d③ 那个「同一帧押两遍钱」同一条坑
                let Ok(mut heading) = sink.headings.get_mut(victim) else {
                    warn!(
                        "{} 要替 {victim:?} 掉头，可他身上没有 Heading 组件（spawn_players 漏挂了？）",
                        source
                    );
                    continue;
                };
                *heading = heading.flipped();
                outcomes.push(EffectOutcome::Reversed { heading: *heading });
            }
            CardEffect::Stay { turns } => {
                // 还是那条形状：先过护身（PLAN §三「免罪抵消坐牢/住院/停留/转向」）。
                // 停留是**事**不是钱，所以它不挂 `Corrections`（P3a 的划分）
                let affliction = Affliction::new(AfflictDomain::Stay, victim);
                if resist(sink, &affliction) {
                    outcomes.push(EffectOutcome::Blocked {
                        domain: affliction.domain,
                    });
                    continue;
                }
                let days = doom_days_for(sink, victim, *turns);
                // 就地写而不是 `Commands`：同一个窗口打两张，第二条要读得到第一条的剩余值。
                // **取较大**而不是相加或覆盖：给一个已经停留 2 回合的人再打一张 1，
                // 覆盖等于替他减刑 —— 惩罚卡不该有让人早走的副作用。
                // 字面口径（P4④）：写 1 就停 1 回合，不再有「判 3 = 跳 2」那半条 ——
                // 计数由玩家按「停留」时扣（`take_stay_system`），这里只写状态
                let left = match sink.stays.get_mut(victim) {
                    Ok(mut held) => {
                        held.turns_left = held.turns_left.max(days);
                        held.turns_left
                    }
                    Err(_) => {
                        sink.commands
                            .entity(victim)
                            .insert(StaySentence { turns_left: days });
                        days
                    }
                };
                outcomes.push(EffectOutcome::Stayed { turns: left });
            }
            CardEffect::MoveCap { turns } => {
                // 与停留同一条反制形状：免罪挡的是**事**，不是钱
                let affliction = Affliction::new(AfflictDomain::MoveCap, victim);
                if resist(sink, &affliction) {
                    outcomes.push(EffectOutcome::Blocked {
                        domain: affliction.domain,
                    });
                    continue;
                }
                let days = doom_days_for(sink, victim, *turns);
                // 就地写 + 取较大：两张乌龟卡不叠加，短的不覆盖长的（惩罚卡不该替人加速）。
                // 计数在每次掷骰时扣一格（`turn::roll_dice_system`），不是打出即扣
                let left = match sink.caps.get_mut(victim) {
                    Ok(mut held) => {
                        held.turns_left = held.turns_left.max(days);
                        held.turns_left
                    }
                    Err(_) => {
                        sink.commands
                            .entity(victim)
                            .insert(MoveCap { turns_left: days });
                        days
                    }
                };
                outcomes.push(EffectOutcome::Slowed { turns: left });
            }
            CardEffect::SummonGod => {
                // 选哪位**不由玩家挑**：那张表算的就是这张卡的赌性（见 `card.rs` 那条注）。
                // 判据与手牌面板那句灰话读的是同一句 `nearest_god`，所以这里再算一次不会分叉
                let (Ok(pos), Ok(heading)) =
                    (sink.positions.get(victim), sink.headings.get(victim))
                else {
                    debug!("请神被跳过：那位玩家没有位置或朝向可数");
                    continue;
                };
                let (at, heading) = (pos.tile, *heading);
                // 名单先抄下来：`attach` 要 `&mut sink`，而那条借用了 `sink.occupancies` 的迭代
                // 不能同时活着（P5 炸弹与 `bestow` 在同一处踩过）
                let standing: Vec<(Entity, crate::gods::God, TileIndex)> = sink
                    .occupancies
                    .iter()
                    .filter_map(|(entity, occ)| match occ.kind {
                        OccupancyKind::God(god) => occ.tile().map(|tile| (entity, god, tile)),
                        _ => None,
                    })
                    .collect();
                let Some((god, from, steps)) = crate::gods::nearest_god(
                    &sink.books.active.def,
                    at,
                    heading,
                    standing.iter().map(|&(_, god, tile)| (god, tile)),
                ) else {
                    debug!("请神符没有可请的对象：路上现在一位神都没有");
                    continue;
                };
                // 走的是与「踩到」同一个出口：请来的人与踩到的人是同一个人，只是来的路不同，
                // 所以那一次性的那一笔（给钱／抢钱／送卡）照做
                crate::gods::attach(
                    &mut sink.commands,
                    &mut sink.books,
                    &mut sink.kit,
                    victim,
                    god,
                    at,
                );
                // 他已经从路上「不在了」：留在原地会变成一位站着没人能再踩到的神，
                // 而那一格的槽位也得腾出来（ADR-002 一格一件）
                if let Some((entity, _, _)) = standing.iter().find(|(_, _, tile)| *tile == from) {
                    sink.commands.entity(*entity).despawn();
                }
                outcomes.push(EffectOutcome::Summoned { god, from, steps });
            }
            CardEffect::BanishGod => {
                let Some(held) = sink.books.god_of(victim) else {
                    debug!("送神符没有可送的对象：那位身上没有神");
                    continue;
                };
                if !crate::gods::banishable(held.god) {
                    debug!("{} 不吃送神符", held.god.label());
                    continue;
                }
                // 炸弹的账在 detach **之前**读：它读的是路上（携带中的）那件东西，
                // 而 detach 要 `&mut sink`
                let carried = sink.occupancies.iter().find_map(|(entity, occ)| {
                    let bomb = matches!(occ.kind, OccupancyKind::TimeBomb);
                    let mine = matches!(occ.at, OccupancyAt::Carried { by, .. } if by == victim);
                    (bomb && mine).then_some(entity)
                });
                crate::gods::detach(
                    &mut sink.commands,
                    &mut sink.books.corrections,
                    victim,
                    held.god,
                    crate::msg::GodExit::Banished,
                    &mut sink.kit.left,
                );
                // PLAN §三 神明行那句「送神符连带送走身上定时炸弹」：同一张符办两件事，
                // 而第二件事要说出来（`bomb` 那个 bool），否则玩家不知道自己顺手丢了什么
                if let Some(entity) = carried {
                    sink.commands.entity(entity).despawn();
                }
                outcomes.push(EffectOutcome::Banished {
                    god: held.god,
                    bomb: carried.is_some(),
                });
            }
        }
    }
    outcomes
}

/// 这件厄运有没有被挡下来。**两个来源、一个出口**：
/// 神给的免疫先问（它不消耗任何东西），否则用掉一层卡给的护身。
///
/// 顺序是有内容的：反过来写，大福神在场时每一件事都会白白吃掉一层免罪卡，
/// 玩家手里那两张会在他还「免疫」的时候悄悄清零 —— 而那件事只有在他不再免疫的
/// 下一件上才看得见，也就是看不见。
fn resist(sink: &mut CardSink<'_, '_>, affliction: &Affliction) -> bool {
    let victim = affliction.victim;
    if sink
        .possessions
        .get(victim)
        .is_ok_and(|held| crate::gods::resists(held.god))
    {
        return true;
    }
    let Ok(mut shield) = sink.insulations.get_mut(victim) else {
        return false;
    };
    if !shield.absorb(affliction) {
        return false;
    }
    // 挡住了才把空护身收走 —— 留着一个 0 层的组件，面板上就是一个
    // 「有免罪卡但什么都不挡」的玩家
    if shield.is_empty() {
        sink.commands.entity(victim).remove::<Insulation>();
    }
    true
}

/// 落在 `victim` 头上的厄运天数，过一遍「那位神说延长几倍」。
///
/// 三个调用点（拘留／停留／乌龟）共用这一句，所以倍数只写在
/// [`crate::gods::doom_multiplier`] 一处 —— 与钱那一侧「乘数只在 `money_corrections`
/// 里写一次」是同一条纪律。
fn doom_days_for(sink: &CardSink<'_, '_>, victim: Entity, days: u32) -> u32 {
    let god = sink.possessions.get(victim).ok().map(|held| held.god);
    crate::gods::doom_days(god, days)
}

// ===== 落格那一段：抽卡并当场结算效果 =====

/// 踩到机会 / 命运格：抽一张、立刻结算效果、把卡面挂成「等玩家读完」。
///
/// 对应 Godot `turn_manager.gd:389-407` + `card_effects.gd`。实质差别都收在
/// [`apply_card`] 那一段的注释里；这里只剩「从哪副牌抽、抽到的发给谁」这一层。
pub fn settle_card_landing(
    mut sink: CardSink,
    mut decks: Query<'_, '_, &'static mut Deck>,
    shelf: Shelf<'_>,
    landing: Res<'_, Landing>,
    mut pending: ResMut<'_, PendingDecision>,
    mut drawn: MessageWriter<'_, CardDrawn>,
) {
    let Some(Landed {
        player,
        domain: LandingDomain::Card(expect),
        ..
    }) = landing.0
    else {
        return;
    };

    // 牌堆与卡定义按种类配对：机会格抽机会牌，命运格抽命运牌。
    // 这条配对写在 `domain_of` 的分派表里，所以这里不必再 `match` 一遍格类型。
    // `Item` 单独挡在这里：道具卡**不该被抽到**（它没有格子，来源是货架），
    // 而这一行存在的意义就是让「哪天有人给道具卡开一格」在这儿编译停一下。
    let kind = match expect {
        CardKind::Chance | CardKind::Fate => expect,
        CardKind::Item => return,
    };
    // `shelf` 仍然是这条 system 自己的参数而不是 `sink.kit.shelf`：`Shelf` 读的是两份 `Res`，
    // 共享读可以并存，而 `def` 要一路活到 `apply_card(&mut sink, ...)` 之后 —— 让它借在
    // `sink` 里面就会与那次 `&mut sink` 撞掉。随机流没有这个问题（取完就放手），所以它走 kit。
    let Some(set) = shelf.set(kind) else {
        return;
    };
    let Some(mut deck) = decks.iter_mut().find(|d| d.kind == kind) else {
        return;
    };
    let Some(drawn_id) = deck.draw(&mut sink.kit.rng) else {
        debug!("{kind:?} 牌堆是空的，本回合不抽卡");
        return;
    };
    let Some(def) = card_of(set, drawn_id) else {
        return;
    };

    // 抽到的卡没有目标可选（`aim: None`）：两副事件池的效果都是打在抽卡人自己身上的，
    // 而带 `target` 的道具卡压根不在池里（`settle_card_landing` 对 `Item` 直接 return）
    // 抽卡这条路没有「别人」：主语与承受者是同一个人（见 `apply_card` 的文档注）
    let outcomes = apply_card(&mut sink, player, player, def, None);

    drawn.write(CardDrawn {
        player,
        card: CardNotice {
            id: def.id.clone(),
            kind: def.kind,
            title: def.title.clone(),
            desc: def.desc.clone(),
            outcomes,
        },
    });
    pending.offer(player, Pending::Card);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardEffect;

    fn set(n: usize, kind: CardKind) -> CardSet {
        CardSet(
            (0..n)
                .map(|i| CardDef {
                    id: format!("{kind:?}_{i:03}"),
                    kind,
                    title: format!("卡{i}"),
                    desc: "测试".into(),
                    cost_points: None,
                    target: crate::card::CardTarget::SelfOnly,
                    effects: vec![CardEffect::AddCash { amount: 100 }],
                })
                .collect(),
        )
    }

    #[test]
    fn shuffled_deck_holds_every_card_exactly_once() {
        let cards = set(10, CardKind::Chance);
        let mut rng = Rng::new(3);
        let deck = Deck::dealt(CardKind::Chance, &cards, &mut rng);
        assert_eq!(deck.remaining(), 10, "总张数不能因洗牌增减");
        let mut sorted = deck.draw_pile.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            (0..10).map(CardId).collect::<Vec<_>>(),
            "洗牌不该丢牌也不该重出一张"
        );
    }

    #[test]
    fn shuffle_is_seeded_and_seed_sensitive() {
        let cards = set(7, CardKind::Chance);
        let order =
            |seed: u64| Deck::dealt(CardKind::Chance, &cards, &mut Rng::new(seed)).draw_pile;
        assert_eq!(order(21), order(21), "同种子必须复现同一副顺序");
        assert_ne!(order(21), order(22), "换种子却同一副顺序说明洗牌没读种子");
    }

    /// 命运那副按**数据顺序**发，而且抽穿回到第一张（PLAN §三「固定顺序循环」）。
    /// 「换种子顺序不变」这一半是重点：定序意味着它**不该**读随机流，
    /// 于是存档只要存「发到第几张」，不必存一副洗好的牌。
    #[test]
    fn the_fate_deck_deals_in_data_order_and_wraps() {
        let cards = set(3, CardKind::Fate);
        let mut deck = Deck::dealt(CardKind::Fate, &cards, &mut Rng::new(77));
        assert_eq!(deck.order, DeckOrder::Fixed, "命运的发法被改掉了");
        let drawn: Vec<CardId> = (0..7)
            .map(|_| deck.draw(&mut Rng::new(1)).expect("循环发牌不该抽空"))
            .collect();
        assert_eq!(
            drawn,
            vec![
                CardId(0),
                CardId(1),
                CardId(2),
                CardId(0),
                CardId(1),
                CardId(2),
                CardId(0)
            ],
            "第三张之后该回到第一张"
        );

        let other_seed = Deck::dealt(CardKind::Fate, &cards, &mut Rng::new(5)).draw_pile;
        assert_eq!(
            other_seed,
            Deck::dealt(CardKind::Fate, &cards, &mut Rng::new(9)).draw_pile,
            "定序的牌堆读了种子，那 32 条的顺序就不是数据说了算的了"
        );
    }

    #[test]
    fn drawing_decrements_and_never_repeats_within_a_pass() {
        let cards = set(5, CardKind::Chance);
        let mut deck = Deck::dealt(CardKind::Chance, &cards, &mut Rng::new(9));
        assert_eq!(deck.remaining(), 5);
        let mut drawn = Vec::new();
        for _ in 0..5 {
            drawn.push(deck.draw(&mut Rng::new(1)).expect("还有牌"));
        }
        assert_eq!(deck.remaining(), 0);
        assert_eq!(deck.discard.len(), 5, "抽出的牌进弃牌堆");
        let mut uniq = drawn.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 5, "一轮之内不该重样：{drawn:?}");
    }

    #[test]
    fn empty_pile_reshuffles_and_keeps_going() {
        // Godot 的 test_deck_auto_reshuffle_when_empty：抽穿后下一张照样有
        let cards = set(3, CardKind::Chance);
        let mut deck = Deck::dealt(CardKind::Chance, &cards, &mut Rng::new(4));
        let mut rng = Rng::new(4);
        let mut second_pass = Vec::new();
        for i in 0..6 {
            let c = deck
                .draw(&mut rng)
                .unwrap_or_else(|| panic!("第 {i} 次抽牌抽空了"));
            if i >= 3 {
                second_pass.push(c);
            }
        }
        assert_eq!(second_pass.len(), 3, "重洗后仍该发满 3 张");
        assert_eq!(deck.remaining(), 0);
    }

    #[test]
    fn empty_deck_reports_none_instead_of_panicking() {
        let cards = CardSet(Vec::new());
        let mut deck = Deck {
            kind: CardKind::Chance,
            order: DeckOrder::Shuffled,
            draw_pile: Vec::new(),
            discard: Vec::new(),
        };
        assert_eq!(
            deck.draw(&mut Rng::new(0)),
            None,
            "一副空牌不该抽（也不该 panic）"
        );
        assert_eq!(deck.total(), 0);
        let _ = cards;
    }

    #[test]
    fn card_lookup_maps_index_to_definition() {
        let cards = set(4, CardKind::Chance);
        assert_eq!(card_of(&cards, CardId(2)).unwrap().title, "卡2");
        assert!(
            card_of(&cards, CardId(99)).is_none(),
            "越界下标应查不到而不是崩"
        );
    }
}
