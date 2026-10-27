//! 魔法屋（P13）：女巫的两步法事 —— 先筛出**条件**命中的人，再施**一道法术**。
//!
//! 一句话的形状：这是「效果动词库」的第三个用户（命运、新闻之外），但它与那两位
//! 分享的不是**代码**而是**出口** —— 十二种法术没有一种自己发明机制，全部借既有
//! 那几条正路：得卡走 `deal_to`、加盖走 `Books::upgrade_witch`、拆楼走
//! `Books::damage`、拍卖走 `Auction::requested`、坐牢/住院/停留/转向走 `CardSink`
//! 的那几条（与卡同一套工具），抽命运直接抽三张命运牌（每张照常过 `apply_card`，
//! 所以「受神明修正」不是魔法屋写的特例，是它根本没挡）。
//!
//! 免疫规则是这一格的全部难度（PLAN §三 那行），而它的答案是**构造方式**的差别：
//! 十二条法术里只有坐牢与住院两条构造 [`crate::afflict::Affliction`]（那两行在
//! [`crate::passive`] 那张事件表上答"可免"），其余十条根本不进那条路 ——
//! 于是「不受任何卡影响」不需要任何查询去逐条挡，漏挡与误挡在类型上都没有位置。
//! P13b② 之后那两条不再自己挡：它们整份交给 [`crate::passive::submit`]，
//! 由那道门去问「手里有没有免罪卡、要不要嫁祸」；而**神明那一半仍然不接**
//! （那四条在 [`crate::passive::affair_of`] 里被标成
//! [`is_ritual`](crate::afflict::Affliction::is_ritual)，而神明那一族 —— 大福神的免疫与
//! 大衰神的天数加倍 —— 只量**法事以外**的厄运），
//! 连 [`crate::gods::doom_days`] 的天数加倍也不接：大福神豁免的是厄运、
//! 大衰神加倍的也是厄运，而女巫的法术不是厄运 —— 免疫矩阵里
//! 「身上有大福神／大衰神」那两格的期望都是**照常生效、照 3 天**。
//!
//! 两步窗口：[`crate::landing::Pending::Magic`]（选条件）→ [`crate::landing::Pending::MagicSpell`]
//! （施法）。筛出的人在 [`MagicRitual`] 那份草稿里，随窗口同生同灭 —— 玩家在第一步
//! 之后反悔的唯一方式是按「离开」，没有任何一条路「跳过条件直接施法」。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::TileIndex;
use crate::deck::{CardId, CardSink};
use crate::land::LandId;
use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};
use crate::msg::{MagicFilterPicked, MagicOffered, MagicSpellPicked, MagicTargets};
use crate::passive::{GateDesk, Job, Step, Tail, submit};
use crate::player::{Gender, Mount, PlayerId};
use crate::settle::Books;
use crate::turn::TurnPhase;
use crate::world::ActiveBoard;

/// 女巫给的条件（P13，PLAN §三 那一行的 11 种）。
///
/// **玩家不能指定具体人** —— 这一行是玩法而不是限制：指定人会让「害人」变成一次
/// 查名操作，条件把它变成一次博弈（「现金最多」在通胀局里未必是最富的那位）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagicFilter {
    /// 走路的人：此刻该行动的那一位。踩到魔法屋结算时那就是踩格者自己。
    Walker,
    /// 存款最多（并列取回合序第一个 —— 「最多」永远有答案，没有并列抽签）。
    TopDeposit,
    /// 所有女生
    AllGirls,
    /// 所有男生
    AllBoys,
    /// 开机车的
    OnBike,
    /// 开汽车的
    OnCar,
    /// 点券最多
    TopPoints,
    /// 土地最多（名下地块数）
    TopLands,
    /// 现金最多
    TopCash,
    /// 神灵附身的
    Possessed,
    /// 财产最多（现钱 + 存款 − 负债 + 股票市值 + 名下地的现值）
    TopWealth,
}

impl MagicFilter {
    pub const ALL: [MagicFilter; 11] = [
        MagicFilter::Walker,
        MagicFilter::TopDeposit,
        MagicFilter::AllGirls,
        MagicFilter::AllBoys,
        MagicFilter::OnBike,
        MagicFilter::OnCar,
        MagicFilter::TopPoints,
        MagicFilter::TopLands,
        MagicFilter::TopCash,
        MagicFilter::Possessed,
        MagicFilter::TopWealth,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            MagicFilter::Walker => "走路的人",
            MagicFilter::TopDeposit => "存款最多",
            MagicFilter::AllGirls => "所有女生",
            MagicFilter::AllBoys => "所有男生",
            MagicFilter::OnBike => "开机车",
            MagicFilter::OnCar => "开汽车",
            MagicFilter::TopPoints => "点券最多",
            MagicFilter::TopLands => "土地最多",
            MagicFilter::TopCash => "现金最多",
            MagicFilter::Possessed => "神灵附身",
            MagicFilter::TopWealth => "财产最多",
        }
    }
}

/// 十二种法术（P13，PLAN §三 那一行的 12 种）。
///
/// 「作用对象」分两族：**作用于人**的（得卡/存现金/抽命运/变卖卡/变卖道具/坐牢/
/// 住院/停留/转向）与**作用于他脚下的地**的（加盖/拆除/拍卖）。两族共享「承受者」
/// 那个词，但后果的路完全不同 —— 穷尽表把每一条各自接的出口写死，加第九种法术时
/// 编译器会来问。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MagicSpell {
    /// 得一张卡（道具池随机，走 `deal_to` —— 手牌满 15 挤掉最前面那张的规则不用重写）
    GainCard,
    /// 就地加盖：他**脚下**那块地免费升一级（押着的与已满级的落空）
    BuildHere,
    /// 存现金：他的全部现金转入存款（防强盗那一刀 —— 是好事，女巫不管你要不要）
    BankCash,
    /// 抽命运 3 张：立刻连抽三张命运牌，每张照常结算 —— 所以「受神明修正」
    /// 不是这里写的特例，是它根本没挡（PLAN §三 那句的落地方式）
    DrawFate,
    /// 变卖所有卡：手牌按 9 折（与卖回百货同一条价）换成点券
    SellCards,
    /// 变卖所有道具：道具栏按研究所标价换成点券
    SellItems,
    /// 坐牢 3 天。**可被免罪抵消**（12 条里两条之一）
    Jail,
    /// 住院 3 天。**可被免罪抵消**（12 条里两条之一）
    Hospital,
    /// 原地停留（字面 1 回合，P4④ 口径；不可被抵消 —— 直接写状态）
    Stay,
    /// 就地拆除：他脚下那块地降一级（保所有权 —— 与定时炸弹同一条损害形状）
    Demolish,
    /// 向后转（不可被抵消 —— 直接翻方向）
    TurnBack,
    /// 拍卖他脚下那块地（开槌的是施法的玩家，拍出的钱归他 —— 与拍卖卡同一条）
    AuctionHere,
}

impl MagicSpell {
    pub const ALL: [MagicSpell; 12] = [
        MagicSpell::GainCard,
        MagicSpell::BuildHere,
        MagicSpell::BankCash,
        MagicSpell::DrawFate,
        MagicSpell::SellCards,
        MagicSpell::SellItems,
        MagicSpell::Jail,
        MagicSpell::Hospital,
        MagicSpell::Stay,
        MagicSpell::Demolish,
        MagicSpell::TurnBack,
        MagicSpell::AuctionHere,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            MagicSpell::GainCard => "得一张卡",
            MagicSpell::BuildHere => "就地加盖",
            MagicSpell::BankCash => "存现金",
            MagicSpell::DrawFate => "抽命运 3 张",
            MagicSpell::SellCards => "变卖所有卡",
            MagicSpell::SellItems => "变卖所有道具",
            MagicSpell::Jail => "坐牢 3 天",
            MagicSpell::Hospital => "住院 3 天",
            MagicSpell::Stay => "原地停留",
            MagicSpell::Demolish => "就地拆除",
            MagicSpell::TurnBack => "向后转",
            MagicSpell::AuctionHere => "拍卖当格土地",
        }
    }

    /// 这一条落到人头上是什么。第二步那一屏一排十二颗按钮，只有名字的话玩家点不下去 ——
    /// 与卡面 `desc` 同一条分工：文案在规则层，面板只念。
    pub const fn desc(self) -> &'static str {
        match self {
            MagicSpell::GainCard => "从货架上摸一张道具卡进手牌（满了挤掉最前面那张）",
            MagicSpell::BuildHere => "他脚下那一格免费升一层，不管那块地是谁的",
            MagicSpell::BankCash => "他身上的现金全部转进存款（防抢，也防自己的手痒）",
            MagicSpell::DrawFate => "当场连抽三张命运牌，每张照常结算（神明的修正吃得到）",
            MagicSpell::SellCards => "手牌全部按九折换成点券，一张不剩",
            MagicSpell::SellItems => "道具栏全部按标价换成点券，一件不剩",
            MagicSpell::Jail => "把他关进监狱 3 回合 —— 十二道里两道挡得住的之一（免罪卡）",
            MagicSpell::Hospital => "送他去医院 3 回合，期间他的地不收租（免罪卡挡得住）",
            MagicSpell::Stay => "轮到他时那一回合走不了（人还在场上，地照收租）",
            MagicSpell::Demolish => "他脚下那栋楼降一层，地还是他的",
            MagicSpell::TurnBack => "他掉头，沿同一圈倒着走（再施一次就转回来）",
            MagicSpell::AuctionHere => "他脚下那块地当场挂拍，起拍是它此刻的身价",
        }
    }
}

/// 女巫判 3 天：PLAN §三 那一行的原文（坐牢 3／住院 3）。卡面的天数在卡上，
/// 这里的天数在这张表上 —— 两处各是各的唯一口径。
pub const MAGIC_DAYS: u32 = 3;

/// 一场法事的草稿：第一步选定的条件与它筛出的人。
///
/// 与 [`crate::landing::PendingDecision`] 同生同灭（施法完成或离场即清），但它是
/// **资源**而不是 Pending 的字段 —— `Pending` 是 `Copy` 的枚举，装不下一个 `Vec`，
/// 而「筛出了谁」是 UI 第二步窗口与施法结算都要读的同一份事实，存两处迟早漂。
#[derive(Resource, Debug, Default, Clone)]
pub struct MagicRitual {
    pub draft: Option<(MagicFilter, Vec<Entity>)>,
}

/// 踩格那一段：把两步窗口的第一步挂上。排在保释之后（链尾），
/// 理由与游乐场、彩票那两条同一条 —— 柜台永远最后问。
pub fn settle_magic_landing(
    landing: Res<Landing>,
    mut pending: ResMut<PendingDecision>,
    mut offered: MessageWriter<MagicOffered>,
) {
    let Some(Landed {
        player,
        domain: LandingDomain::Magic,
        ..
    }) = landing.0
    else {
        return;
    };
    pending.offer(player, Pending::Magic);
    offered.write(MagicOffered { player });
}

/// 筛条件的全部工具。名字来源与位置读数都在这一包里 —— 11 条判据共享同一份
/// 「谁是场上玩家」的读数，散到 11 个函数参数上就把 clippy 的上限撞穿了。
#[derive(SystemParam)]
pub struct FilterDesk<'w, 's> {
    pub books: Books<'w, 's>,
    /// 持股现价从这里读（`Market::price`）：价格的写侧在行情那一侧，
    /// 两个参数包并存本来就是 `trade_request_system` 的形状。
    pub market: crate::market::Market<'w, 's>,
    faces: Query<'w, 's, (Entity, &'static PlayerId, &'static Gender)>,
    mounts: Query<'w, 's, &'static Mount>,
    possessions: Query<'w, 's, &'static crate::gods::Possession>,
}

/// 按条件筛人。**这里只有一个口径**：第二步窗口显示的名单与法术落到的头
/// 必须是同一批 —— 所以两个系统都调它，而不是一个筛一个猜。
///
/// 「最多」那一族并列取**回合序第一个**（`faces` 的迭代序）：永远有答案，
/// 没有并列抽签 —— 那会把一次取随机数的副作用塞进一条纯判据里。
pub fn filter_targets(filter: MagicFilter, current: Entity, desk: &FilterDesk) -> Vec<Entity> {
    // 玩家名单按回合序（实体生成序就是它 —— spawn_players 按名单顺序铺）。
    // 破产者**不在名单里**：他已经离场，女巫的法术落不到一个不在牌桌上的人。
    let everyone: Vec<Entity> = desk
        .faces
        .iter()
        .filter(|(e, _, _)| !desk.books.is_bankrupt(*e))
        .map(|(e, _, _)| e)
        .collect();
    match filter {
        MagicFilter::Walker => vec![current],
        MagicFilter::TopDeposit => top_by(&everyone, |e| desk.books.bank_of(*e).deposit),
        MagicFilter::TopCash => top_by(&everyone, |e| desk.books.balance(*e)),
        MagicFilter::TopPoints => top_by(&everyone, |e| desk.books.points_of(*e) as i64),
        MagicFilter::TopLands => top_by(&everyone, |e| desk.books.land_count_of(*e) as i64),
        MagicFilter::TopWealth => top_by(&everyone, |e| wealth_of(desk, e)),
        MagicFilter::AllGirls => everyone
            .iter()
            .copied()
            .filter(|e| {
                desk.faces
                    .get(*e)
                    .is_ok_and(|(_, _, g)| *g == Gender::Female)
            })
            .collect(),
        MagicFilter::AllBoys => everyone
            .iter()
            .copied()
            .filter(|e| desk.faces.get(*e).is_ok_and(|(_, _, g)| *g == Gender::Male))
            .collect(),
        MagicFilter::OnBike => everyone
            .iter()
            .copied()
            .filter(|e| desk.mounts.get(*e).is_ok_and(|m| *m == Mount::Bike))
            .collect(),
        MagicFilter::OnCar => everyone
            .iter()
            .copied()
            .filter(|e| desk.mounts.get(*e).is_ok_and(|m| *m == Mount::Car))
            .collect(),
        MagicFilter::Possessed => everyone
            .iter()
            .copied()
            .filter(|e| desk.possessions.get(*e).is_ok())
            .collect(),
    }
}

/// 「最多」那一族的公共形状：取最大，并列取**先出现的**（回合序）。
/// 0 也是合法答案 —— 「存款最多」在没有一个人存过钱时照样筛出第一个人，
/// 女巫的窗口不该因为全场破产而拒绝开台。
fn top_by(rows: &[Entity], value: impl Fn(&Entity) -> i64) -> Vec<Entity> {
    let mut best: Option<(i64, Entity)> = None;
    for e in rows {
        let v = value(e);
        let replace = match best {
            None => true,
            Some((bv, _)) => v > bv,
        };
        if replace {
            best = Some((v, *e));
        }
    }
    best.map(|(_, e)| vec![e]).unwrap_or_default()
}

/// 单人财富（`TopWealth` 的口径）：现金 + 存款 − 负债 + 持股现值 + 名下地的现值。
///
/// 「地的现值」走 [`Books::market_value`] 那条式子（地价 + 楼，过物价）—— 与拍卖起拍
/// 同一个数：女巫眼里的「最有钱」与拍卖台上的「最值钱」不该是两套账。`financing`
/// 是欠银行保险的那笔（P10d②），与贷款同侧扣。地块与持股的读数全在账本里
/// （[`Books::land_count_of`] 同一条理由），这一句只是把它们加起来。
fn wealth_of(desk: &FilterDesk, who: &Entity) -> i64 {
    let bank = desk.books.bank_of(*who);
    let mut worth = desk.books.balance(*who) + bank.deposit - bank.loan - bank.financing;
    // 名下地的现值：逐块问账本（`Books` 的 plots 是写侧，外面开读查询会撞）
    for land in desk.books.owned_lands(*who) {
        worth += desk.books.market_value(land).unwrap_or(0);
    }
    // 持股现值：清单从账本出、现价从行情出（`Market::price`）
    for (id, shares) in desk.books.stock_holdings_of(*who) {
        worth += (desk.market.price(&id).unwrap_or(0.0) as i64) * (shares as i64);
    }
    worth
}

/// 第一步的回答：条件落地、筛人、开第二步窗口。
///
/// 窗口挂着 [`crate::landing::Pending::Magic`] 且主语对才受理 —— 与 `bail_op_system`
/// 同一条门禁形状。**空名单直接落空**：第二步没有可念的对象，把玩家摆到一排
/// 「点不着任何人的法术」前面，比现在就收摊更糟。
///
/// 落空那一条**自己交还回合**（`next.set(TurnEnd)`）而不是只 `pending.clear()`：
/// 交还回合 normally 是 `decision_system` 的活，而它只在「有人答了一句」时才动回合机 ——
/// 这一步没人答话（面板上那颗按钮本来就是灰的，规则层却必须假设有人能把它送进来），
/// 于是只清窗口的写法会把回合机永久停在 `WaitDecision`，且不报错、只是不动了。
/// 这与 [`crate::landing::close_landing_system`] 那句「问一句 `Auction::holds_turn`」
/// 是同一条坑的另一种形状：**谁清窗口，谁就得负责把回合交出去**。
///
/// 两处可能同帧写 `NextState` 的只有 `decision_system`，而它一进「窗口没了」就返回，
/// 且两条路写的都是同一个 `TurnEnd` —— 所以这里不需要赌次序（§6.1 坑 37）。
pub fn magic_filter_system(
    mut ritual: ResMut<MagicRitual>,
    mut pending: ResMut<PendingDecision>,
    order: Res<crate::turn::TurnOrder>,
    mut next: ResMut<NextState<crate::turn::TurnPhase>>,
    mut picks: MessageReader<MagicFilterPicked>,
    mut targets: MessageWriter<MagicTargets>,
    desk: FilterDesk,
) {
    let Some((asked, Pending::Magic)) = pending.active() else {
        return;
    };
    let Some(current) = order.current() else {
        return;
    };
    let mut answer = None;
    for p in picks.read() {
        if p.player == asked {
            answer = Some(p.filter);
        }
    }
    let Some(filter) = answer else {
        return;
    };
    let found = filter_targets(filter, current, &desk);
    targets.write(MagicTargets {
        player: asked,
        filter,
        targets: found.clone(),
    });
    if found.is_empty() {
        // 落空也是一次完成的法事：窗口收掉，回合照常往下走
        pending.clear();
        next.set(crate::turn::TurnPhase::TurnEnd);
        return;
    }
    ritual.draft = Some((filter, found));
    pending.offer(asked, Pending::MagicSpell(filter));
}

/// 施法那一步：把这道法术**整份交给那道门**，然后收掉自己的窗口。
///
/// 工具包是 [`GateDesk`]（内含 `CardSink`）—— 坐牢/住院/停留/转向四条走门，
/// 其余八条走 [`cast_step`]；随机流、货架、手牌、发卡凭据在 `desk.sink.kit`，
/// 牌堆与道具栏在 `desk` 上。这里再自带一份就是同一个 system 里两把 `ResMut<Rng>`，
/// 参数建立期直接撞成 B0001（坑 33 的资源版）。
pub fn magic_spell_system(mut desk: GateDesk, mut picks: MessageReader<MagicSpellPicked>) {
    let Some((asked, Pending::MagicSpell(_))) = desk.pending.active() else {
        // 草稿与第二步窗口**同生同灭**：玩家按「离开」时 `decision_system` 只收窗口，
        // 而「这场法事筛出了谁」是资源里的东西 —— 不收它就会留下一份过期名单，
        // dump 与面板都读得到它，且回主菜单那一帧也没有任何东西替它清账（坑 22/25 同族）
        if desk.ritual.draft.is_some() {
            desk.ritual.draft = None;
        }
        return;
    };
    let mut answer = None;
    for p in picks.read() {
        if p.player == asked {
            answer = Some(p.spell);
        }
    }
    let Some(spell) = answer else {
        return;
    };
    let Some((_filter, wanted)) = desk.ritual.draft.clone() else {
        // 草稿与窗口分叉了（不该发生 —— 两步同生同灭）。窗口照关，法术不施：
        // 宁可落空，也不把一个查不到名单的窗口留给下一位。交还回合与上面那条纪律同一条：
        // **谁清窗口谁交回合**，这里也一样不能只 `clear`
        warn!("魔法屋的草稿没了，法术落空");
        desk.pending.clear();
        desk.next.set(TurnPhase::TurnEnd);
        return;
    };
    // 草稿与名单一次消耗掉：门里那份 `Tail::Witch` 现在就是「还剩谁」的唯一记录，
    // 资源里再留一份就是第二份真相（而它在门挂着的几十帧里会读到过期名单）
    desk.ritual.draft = None;
    // 窗口先收掉再交出去 —— 顺序反了也不出错（`offer` 会覆盖同一个槽），但这句话
    // 说的是**归属**：从这一行起，那条窗口是门的，不再是法事的。
    // 交还回合因此也归门（`finish_witch`），这里不能再写 `next.set`（坑 37）
    desk.pending.clear();
    let mut waiting = wanted;
    if waiting.is_empty() {
        // 第一步已经把空名单挡成落空了（`magic_filter_system`），这一格走不到。
        // 写出来是为了让「一场没人可施的法事」也有一个凭据上的答案：空名单、空阻塞
        finish_witch(&mut desk, asked, spell, Vec::new(), Vec::new());
        return;
    }
    let first = waiting.remove(0);
    let source = format!("女巫「{}」", spell.label());
    submit(
        &mut desk,
        Job::new(
            first,
            steps_of(spell),
            source,
            None,
            crate::auction::Gavel::Card(asked),
            Tail::Witch {
                caster: asked,
                spell,
                hit: Vec::new(),
                blocked: Vec::new(),
                waiting,
            },
        ),
    );
}

/// 一道法术在门里是几行条目。
///
/// 十一道是**一条**（`Step::Spell`，或者它本身就是那件「事」），只有「抽命运 3 张」是三条：
/// 每一张都要单独办完（其中任何一张都可能是「直接入狱」那种要问一句的事），
/// 而它们的消耗次序与旧 `cast_one` 那个循环逐字相同 —— 抽一张、办一张，
/// 同一个种子因此跑出同一局。
pub fn steps_of(spell: MagicSpell) -> Vec<Step> {
    if spell == MagicSpell::DrawFate {
        return vec![Step::FateDraw; 3];
    }
    vec![Step::Spell(spell)]
}

/// 施法完成：落槌名单与「被挡掉的那几位」各自成凭据，然后交还回合。
///
/// **交还回合的是这一句而不是 `magic_spell_system`**（坑 38 那条「谁清窗口谁交回合」）：
/// 名单可能一次跑完（没有门要问），也可能拖到几十帧之后（有人在答免罪卡），
/// 而两种情况下收尾的都是这最后一位。
///
/// `AuctionHere` 那一道把回合**押给了拍卖机**：`auction_hold_system` 建场之后自己
/// `next.set(WaitDecision)` 并把窗口挂到该出价的人身上。这里若也写 `TurnEnd`，同一回合
/// 就有两处各写一个 `NextState` 而次序没人保证（§6.1 坑 37），症状是回合指针已经交给
/// 下一个人、拍卖台还挂着 —— 而一句错话也不说。落空（`hit` 为空）不算押出去，仍由这里交回合。
pub fn finish_witch(
    desk: &mut GateDesk,
    caster: Entity,
    spell: MagicSpell,
    hit: Vec<Entity>,
    blocked: Vec<Entity>,
) {
    for victim in blocked {
        desk.blocked.write(crate::msg::MagicBlocked {
            player: caster,
            spell,
            victim,
        });
    }
    // **施法就是这一步的收尾**：一颗法术按钮既办完事又交还回合，与建设柜台那种
    // 「办完窗口照开、还要再按一次先不建」不同 —— 草稿一次就消耗掉，留着窗口只会让
    // 玩家对着一排「点下去只会落空」的按钮。
    let handed_to_auction = spell == MagicSpell::AuctionHere && !hit.is_empty();
    desk.cast.write(crate::msg::MagicCast {
        player: caster,
        spell,
        targets: hit,
    });
    if !handed_to_auction {
        desk.next.set(TurnPhase::TurnEnd);
    }
}

/// 把一道**不需要问人**的法术落到一个人头上。返回 `true` = 生效了（进 [`MagicCast::targets`]）。
///
/// 「不需要问人」这一判据不在这里，而在 [`crate::passive::affair_of`]：坐牢、住院、原地停留、
/// 向后转四条是**事**，它们在门里就被截住，走不到这一句。剩下的六道（得卡、加盖、存现金、
/// 变卖两样、拆除、拍卖）改的是钱、资产或手牌 —— 「十二条里只有两条挡得住」这句话
/// 今天仍然由**类型**说，而不是由这里的一条 `if` 说。
///
/// `DrawFate` 那一格走不到（它的条目是三条 `Step::FateDraw`），写成 `warn!` 而不是静默 `false`：
/// 一个永远不会被走的臂与一条永远不会响的静默降级，在代码里是同一件事。
///
/// `gavel` 是**这一槌凭什么敲**，由门里那份信封带来（[`Job::gavel`](crate::passive::Job)）而不是
/// 在这里现编：旧 `cast_one` 那一份写的是「开槌的人＝施法者」，而这一格若由这里替成
/// 「被施法的人」，`can_open` 就会把「他把自己的地挂上自己的拍卖台」那条拒掉 —— 症状不是报错，
/// 是顶栏念出「女巫把他挂上拍卖台」而台上没有这一槌（名单空着）。
#[allow(clippy::too_many_arguments)]
pub fn cast_step(
    desk: &mut GateDesk,
    spell: MagicSpell,
    target: Entity,
    gavel: crate::auction::Gavel,
) -> bool {
    // 随机流 / 货架 / 手牌 / 发卡凭据都从 `desk.sink.kit` 取，而且**现借现还**：
    // 任何一次跨分支的借用都会把下一条挡住（NLL 的形状，与旧 `cast_one` 同一条）
    match spell {
        MagicSpell::Jail | MagicSpell::Hospital | MagicSpell::Stay | MagicSpell::TurnBack => {
            // 那四条是「事」，归门管（`affair_of` 对它们答 `Some`）。走到这里只可能是
            // 两条表分叉了 —— 宁可响一声，也不要替门把那一问吞掉
            warn!("门里那四条法术走进了「不问人」那一条路（affair_of 与这里分叉了）");
            false
        }
        MagicSpell::DrawFate => {
            warn!("「抽命运 3 张」的条目应该是 `Step::FateDraw`（steps_of 与这里分叉了）");
            false
        }
        // ===== 作用于人的其余三条 =====
        MagicSpell::GainCard => {
            // 「得一张卡」发的是**道具卡**（货架那一副）：与「得一张命运牌」的差别
            // 在卡有没有 target——而这一格的玩家直觉就是「从货架上摸一张」。
            // 下标走专用子流：这一局施过几次法不该改变牌堆序
            let picked = {
                let count = desk.sink.kit.shelf.items().len() as u32;
                if count == 0 {
                    return false;
                }
                CardId(desk.sink.kit.rng.pick_magic(0..=count - 1))
            };
            let limit = desk.sink.books.rules.hand_limit;
            let Ok(mut hand) = desk.sink.kit.hands.get_mut(target) else {
                return false;
            };
            crate::hand::deal_to(
                &mut hand,
                limit,
                picked,
                target,
                &desk.sink.kit.shelf,
                &mut desk.sink.kit.gained,
            );
            true
        }
        MagicSpell::BankCash => {
            let all = desk.sink.books.bank_all_cash(target);
            all > 0
        }
        MagicSpell::SellCards => {
            let Ok(mut hand) = desk.sink.kit.hands.get_mut(target) else {
                return false;
            };
            if hand.is_empty() {
                return false;
            }
            // 价与卖回百货同一条式子（9 折）：`CardId` 是道具池下标，
            // 售价从同一副池的定义里读 —— 两处对不上是数据错，按 0 卖并照发凭据
            let mut earned = 0u32;
            while let Some(id) = hand.cards.pop_front() {
                let price = desk
                    .sink
                    .kit
                    .shelf
                    .items()
                    .get(id.0 as usize)
                    .and_then(|def| def.cost_points)
                    .unwrap_or(0);
                earned += crate::shop::buyback(price, 90);
            }
            if earned > 0 {
                desk.sink.books.award_points(target, earned);
            }
            true
        }
        MagicSpell::SellItems => {
            let Ok(mut inventory) = desk.inventories.get_mut(target) else {
                return false;
            };
            let rows = inventory.clear_all();
            if rows.is_empty() {
                return false;
            }
            // 价按研究所标价（`price_points`）全额 —— 变卖不是折扣柜台，
            // 女巫不抽成；点券给**被施法的人**（是他的东西换的）
            let earned: u32 = rows
                .iter()
                .map(|(kind, count)| kind.price_points() * count)
                .sum();
            if earned > 0 {
                desk.sink.books.award_points(target, earned);
            }
            true
        }
        // ===== 作用于脚下地的那三条 =====
        MagicSpell::BuildHere => {
            let Some(land) = land_under(&desk.sink.books.active, target, &desk.sink) else {
                return false;
            };
            desk.sink.books.upgrade_witch(land).is_some()
        }
        MagicSpell::Demolish => {
            let Some(land) = land_under(&desk.sink.books.active, target, &desk.sink) else {
                return false;
            };
            // 降一级、保所有权（与定时炸弹同一支损害形状）—— 女巫的「拆除」不是征收
            let before = desk.sink.books.view(land).map(|view| view.level);
            let tally = desk.sink.books.damage(
                land,
                crate::damage::LandDamage::Degrade { levels: 1 },
                crate::plot::PlotArea::One,
            );
            // 「生效」按**层数真的少了一层**判，不按 `tally.hit`：那条数的是范围盖到几块地，
            // 而 0 级空地（`saturating_sub` 的地板）也在里面算一次命中。拿 `hit` 当答案的话，
            // 顶栏会对一块空地念出「女巫拆掉了她的楼」—— 面板说的与世界上发生的分叉，
            // 与 P9 那句「写的与扣的必须同一个表达式」是同一条纪律。
            tally.hit > 0
                && before.is_some_and(|was| {
                    desk.sink
                        .books
                        .view(land)
                        .is_some_and(|now| now.level < was)
                })
        }
        MagicSpell::AuctionHere => {
            let Some(land) = land_under(&desk.sink.books.active, target, &desk.sink) else {
                return false;
            };
            let listing = crate::auction::Listing { land, gavel };
            // 这一问与拍卖机用的是**同一句**（[`crate::auction::can_open`]，只读且不依赖座位序，
            // 它的注早就写了「既能在出牌前当『会不会落空』那一问，也能在建场前用」）。
            // 不问的后果不是钱而是**话**：那块地若押着、无主、是施法者自己的、或场上没有别人，
            // 机器下一帧会把这一槌丢掉并只留一句 `warn!`，而顶栏已经说了「女巫把他挂上拍卖台」——
            // 面板报的「生效」与世界上发生的事分叉。机器在开场那一刻还会再问一次，两处不冲突
            if crate::auction::can_open(&desk.sink.books, land, listing.gavel).is_err() {
                return false;
            }
            // 与拍卖卡同一条快道（`requested`）：开槌的是施法的玩家、钱归他
            desk.sink.books.auction.requested = Some(listing);
            true
        }
    }
}

/// 他脚下那一格绑定的地（`TileDef.land_id`）。三条地法术共用这一个读数：
/// 「就地/当格」的答案只有一个来源。
fn land_under(active: &ActiveBoard, target: Entity, sink: &CardSink) -> Option<LandId> {
    let tile = sink.positions.get(target).ok()?.tile;
    let at = TileIndex(tile.0);
    active
        .def
        .tile(at.0 as i64)
        .and_then(|t| t.land_id)
        .map(LandId)
}
