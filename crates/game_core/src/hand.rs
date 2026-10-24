//! 手牌（P3a）：拿在手里、由玩家**主动打出**的那批卡。
//!
//! 与两副事件池的分别是「谁决定什么时候生效」：机会/命运是踩到格子就被抽、当场结算
//! （[`crate::deck::settle_card_landing`]），而道具卡在玩家手里可以一直躺着，等他想要
//! 的那个时刻打出去。所以这个模块管三件事：
//!
//! - **容器** [`Hand`]：有序、有上限、满了**替换最前面那张**（PLAN §三「持有与上限」行）。
//! - **入口** [`play_card_system`]：与证券下单同一条窗口（掷骰前、只有本人），走的是与
//!   真实按钮同一个请求消息，不是直接改组件。
//! - **承受者**：卡上的效果打在谁头上，由 `CardDef.target` 与请求里带的目标一起定 ——
//!   「A 用陷害卡打 B」与「命运牌把 A 自己关进去」共用同一条效果实现
//!   （[`crate::deck::apply_card`]），区别只是那个 `victim`。
//!
//! 上限为什么是「替换最前面」而不是「满了不让买」：4 代就是这么做的，而它比「不让买」
//! 多一层玩法 —— 只剩一格时玩家可以**赌**下一张更有用，旧的那张自动出局。这条规则要读得
//! 出来，所以 [`Hand::gain`] 把被挤掉的那张**返回**给调用方发流水，而不是内部悄悄丢弃
//! （「我明明买了卡，卡不见了」是查不动的那类投诉）。

use std::collections::VecDeque;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::card::{CardDef, CardEffect, CardKind, CardTarget};
use crate::deck::{CardId, CardSink, Shelf, apply_card};
use crate::land::LandId;
use crate::landing::PendingDecision;
use crate::msg::{CardGained, CardNotice, CardPlayed, PlayCardRequested};
use crate::occupancy::OccupancyKind;
use crate::player::PlayerId;
use crate::settle::Books;
use crate::turn::{TurnOrder, TurnPhase};

/// 手里拿着的卡，按获得顺序排（`front` 是最早拿到、也最先被挤掉的那张）。
///
/// 存 `CardId`（item 池的下标）而不是卡名字符串：与牌堆同一条理由 —— 卡定义是加载后
/// 不变的静态数组，下标即身份。可以重复：买两张一样的陷害卡占两格手牌。
#[derive(Component, Debug, Default, Clone, PartialEq, Eq)]
pub struct Hand {
    pub cards: VecDeque<CardId>,
}

impl Hand {
    pub fn len(&self) -> usize {
        self.cards.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    pub fn holds(&self, card: CardId) -> bool {
        self.cards.contains(&card)
    }

    /// 进一张卡，返回被挤掉的最前面那张（`None` = 没挤掉任何人）。
    ///
    /// 上限由调用方给（`GameRules::hand_limit`）而不是写死在这里：规则常量只有一个真相
    /// 来源（`rules.rs`），而这里要能被测试用 `limit = 1` 逼出边界。
    pub fn gain(&mut self, card: CardId, limit: u32) -> Option<CardId> {
        self.cards.push_back(card);
        let cap = (limit.max(1)) as usize;
        if self.cards.len() > cap {
            return self.cards.pop_front();
        }
        None
    }

    /// 打出一张：移除**第一张**匹配的（同 id 有两张时打掉一格，不是清空）。
    pub fn spend(&mut self, card: CardId) -> bool {
        match self.cards.iter().position(|held| *held == card) {
            Some(at) => {
                self.cards.remove(at);
                true
            }
            None => false,
        }
    }
}

/// 请求里带来的目标。`CardDef.target` 说这张卡**要**哪种目标，这里带的是**选了哪个**。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardAim {
    Player(Entity),
    Land(LandId),
}

/// 这张卡实际打到谁头上。`SelfOnly` 的卡把出牌人自己填进去。
///
/// 地块类填的是**这块地的业主**（P3c）：涨价与查封都是往别人钱袋上做事，流水里要说的是
/// 「谁的地被办了」。查不到业主（无主、或那块地压根不在棋盘上）就返回 `None` —— 宁可拒发
/// 也不猜一个承受者，猜出来的那个「业主」会在流水里变成一次莫须有的查封。
/// 购地卡（P3d④）是另一块地：它挑的**就是**无主地，所以它的承受者是出牌人自己，
/// 这也是它不并进 `Land` 那一格的原因（两种地块目标问不出同一个承受者）。
///
/// `land_owner` 由调用方查好再传进来（`Books::owner_of`），而不是在这里收一个 `Books`：
/// 这张表要能在单测里逐格断言，而 `Books` 是 SystemParam、造不出来给单元测试。
/// 「读世界」留在 system 里，「谁是谁」留在这里 —— 两边各自可测。
fn victim_of(
    def: &CardDef,
    aim: Option<CardAim>,
    player: Entity,
    land_owner: Option<Entity>,
) -> Option<Entity> {
    match (def.target, aim) {
        (CardTarget::SelfOnly, _) => Some(player),
        // `Player` 与 `Anybody` 在这一格是同一个问题（「打给谁」就是选中的那个人），
        // 它们的区别全在 `aim_is_legal` 那一侧（允不允许选自己）—— 承受者表不该再判一次
        (CardTarget::Player | CardTarget::Anybody, Some(CardAim::Player(target))) => Some(target),
        (CardTarget::Land, Some(CardAim::Land(_))) => land_owner,
        // 购地卡承受的也是出牌人自己：无主地没有业主可承受，而买下来之后地就归他 ——
        // 流水里那句「P0 → P1」在这里说的就是「他买下了这块地」，不是「他对谁做了事」
        (CardTarget::UnownedLand, Some(CardAim::Land(_))) => Some(player),
        (CardTarget::Land, None)
        | (CardTarget::Land, Some(CardAim::Player(_)))
        | (CardTarget::UnownedLand, None)
        | (CardTarget::UnownedLand, Some(CardAim::Player(_)))
        | (CardTarget::Player, None)
        | (CardTarget::Player, Some(CardAim::Land(_)))
        | (CardTarget::Anybody, None)
        | (CardTarget::Anybody, Some(CardAim::Land(_))) => None,
    }
}

/// 目标合不合法。判据集中在这里，UI 的「该不该亮」与规则层的「认不认」共用同一句
/// （同 `can_bank_op` 与 `bank_op_allowed` 的关系）。
///
/// 写成 `target × aim` 的十五格全表、不留 `_`：新增一种 `CardTarget` 或一种 `CardAim`
/// 时必须当场决定它和其余每种目标能不能配对（同 `landing::domain_of` 那条纪律）。
fn aim_is_legal(def: &CardDef, aim: Option<CardAim>, player: Entity, books: &Books) -> bool {
    match (def.target, aim) {
        // 给自己的卡不该带目标：面板上根本没有那颗按钮，带上来的是错请求
        (CardTarget::SelfOnly, None) => true,
        (CardTarget::SelfOnly, Some(CardAim::Player(_)))
        | (CardTarget::SelfOnly, Some(CardAim::Land(_))) => false,
        // 不能陷害自己（那是停留卡要做的事），也不能打一个已经离场的人
        (CardTarget::Player, Some(CardAim::Player(target))) => {
            target != player && books.id_of(target).is_some() && !books.is_bankrupt(target)
        }
        (CardTarget::Player, None) | (CardTarget::Player, Some(CardAim::Land(_))) => false,
        // 「一个人，含自己」（转向 / 停留）。与上面那一格只差 `target != player` 这一句，
        // 而那一句就是两张卡的区别所在 —— 所以它们各占一格，不并成一条带 bool 的判据。
        // 离场的人照样挡：给一个破产退出回合的人掉头，是往不在牌桌上的人身上做事
        (CardTarget::Anybody, Some(CardAim::Player(target))) => {
            books.id_of(target).is_some() && !books.is_bankrupt(target)
        }
        (CardTarget::Anybody, None) | (CardTarget::Anybody, Some(CardAim::Land(_))) => false,
        // 地块类：格子上得真有这块地，且真有人持有它（查封一块无主地没有意义 ——
        // 没有业主可办，而 UI 的候选列表用的就是这同一句判据）
        (CardTarget::Land, Some(CardAim::Land(land))) => books.owner_of(land).is_some(),
        (CardTarget::Land, None) | (CardTarget::Land, Some(CardAim::Player(_))) => false,
        // 购地卡：这块地此刻**买得下来吗**。判据直接借账本那句（`Books::buy_price` =
        // 无主 + 过了物价与修正之后的价付得起），因为出牌走的本来就是它。
        // 在这里另写一份 `owner.is_none() && cash >= 地价` 迟早会和账本分叉，
        // 而分叉长成「卡扣掉了、地没到手」那种查不动的样子
        (CardTarget::UnownedLand, Some(CardAim::Land(land))) => {
            books.buy_price(player, land).is_some()
        }
        (CardTarget::UnownedLand, None) | (CardTarget::UnownedLand, Some(CardAim::Player(_))) => {
            false
        }
    }
}

/// 出牌的第四道门：**这张卡的效果此刻有没有对象**。
///
/// 它与 [`aim_is_legal`] 分开，是因为问的不是「选的东西合不合法」而是「这件事有没有可做的
/// 对象」。P6d 那两张符都没有 `aim`（请哪位由 `nearest_god` 算、送的是自己身上那位），
/// 所以它们一句也进不了那张十五格表 —— 而它们恰恰是**会落空**的两张：路上可能一位神都没有，
/// 身上可能没有神。落空时卡必须留在手里（判据分叉的形状从来不是钱少了，是「卡没了事没成」）。
///
/// 回来的是**办不成的那一句**：规则层拿它当拒绝的理由，手牌面板拿它当灰按钮的文案。
/// 判据只有一份，两处各调一次 `nearest_god` —— 面板查的是 `Occupancy`，而 `Books` 里没有
/// 这一格，所以它自己开一条只读查询不算复制第二份真相（购地卡那次借 `buy_price` 是因为
/// 真相在账本里，这里不在）。
///
/// `opener` 与 `victim` 分开带，是 P9 的拍卖卡逼出来的：那张卡的**主语与承受者不是同一个人**
/// （开槌的人不能拍自己的地），而「这块地上不了拍」的四条理由里有两条要问出牌人。
/// 抽卡与自用的卡两者本来就是同一个人，由调用方各传一遍。
fn unmet_precondition(
    def: &CardDef,
    opener: Entity,
    victim: Entity,
    aim: Option<CardAim>,
    sink: &CardSink<'_, '_>,
) -> Option<String> {
    for effect in &def.effects {
        let why = match effect {
            CardEffect::SummonGod => {
                let (Ok(pos), Ok(heading)) =
                    (sink.positions.get(victim), sink.headings.get(victim))
                else {
                    return Some("说不清他在哪一格".to_string());
                };
                let gods = sink
                    .occupancies
                    .iter()
                    .filter_map(|(_, occ)| match occ.kind {
                        OccupancyKind::God(god) => occ.tile().map(|tile| (god, tile)),
                        _ => None,
                    });
                crate::gods::nearest_god(&sink.books.active.def, pos.tile, *heading, gods)
                    .is_none()
                    .then(|| crate::gods::NO_GOD_ON_THE_ROAD.to_string())
            }
            CardEffect::BanishGod => match sink.books.god_of(victim) {
                None => Some(crate::gods::NO_GOD_TO_BANISH.to_string()),
                Some(held) if !crate::gods::banishable(held.god) => {
                    Some(crate::gods::god_refuses_banish(held.god))
                }
                Some(_) => None,
            },
            CardEffect::Auction { .. } => {
                // 这张卡**会落空**，而落空的四种样子在画面上长得一模一样（按钮按下去没声音）。
                // 问的正是拍卖那一道唯一闸门 `can_open`：押着的地、查封中的地、自己的地、
                // 场上没有别人可出价 —— 四条都从它回来，于是「面板说能拍」与「规则层肯开场」
                // 不可能分叉（与 P6d 那两张符同一条纪律：**办不成时卡不消耗**）
                let Some(CardAim::Land(land)) = aim else {
                    return Some(crate::auction::OpenDeny::Unowned.label().to_string());
                };
                crate::auction::can_open(&sink.books, land, crate::auction::Gavel::Card(opener))
                    .err()
                    .map(|why| why.label().to_string())
            }
            CardEffect::Release { place } => {
                // 与「请神符找不到神」同一格形状：这一张要问「他此刻真的关在那一处吗」。
                // 今天**没有卡带这条效果**（它是新闻专用的，而新闻不摆按钮），这一格不许省：
                // 将来那张「交钱赎人」的卡若落到 `None`，症状就是卡被白扣而没人出来
                match sink.books.detained.get(victim).ok() {
                    Some(held) if held.place == *place => None,
                    Some(held) => Some(format!(
                        "他关在{}，不是{}",
                        held.place.label(),
                        place.label()
                    )),
                    None => Some(format!("他没有被关在{}", place.label())),
                }
            }
            // 其余每一条效果都不需要「此刻有没有对象」这一问：免罪与免费是挂上去就等着，
            // 拘留/停留/乌龟/转向打在谁身上都成立，地块类（涨价、查封、拆房）由 `aim_is_legal` 管。
            // `Levy` 也在这群里：税打在谁身上都成立，**抽到 0 元也是一笔办成的税**（流水里那条
            // `Lost(0)` 说的就是这件事），不是「没有对象」。
            // 穷尽而不是 `_ => None`：新加一条效果必须当场回答它会不会落空
            CardEffect::AddCash { .. }
            | CardEffect::DeductCash { .. }
            | CardEffect::Levy { .. }
            | CardEffect::Detain { .. }
            | CardEffect::MoveTo { .. }
            | CardEffect::Insulate { .. }
            | CardEffect::FreePass { .. }
            | CardEffect::Reverse
            | CardEffect::Stay { .. }
            | CardEffect::MoveCap { .. }
            | CardEffect::LandStatus { .. }
            | CardEffect::Damage { .. }
            | CardEffect::BuyLand => None,
        };
        if why.is_some() {
            return why;
        }
    }
    None
}

/// 「现在能不能替谁出牌」的三份读数。打包成一个参数包是为了让出牌 system 留在
/// clippy 的参数上限内，也因为这三份判据本来就是**一起成立或一起不成立**的：
/// 阶段、轮转、待决事项，少看任何一份都会开出一条不该有的窗口。
#[derive(SystemParam)]
pub struct PlayWindow<'w> {
    phase: Res<'w, State<TurnPhase>>,
    order: Option<Res<'w, TurnOrder>>,
    pending: Res<'w, PendingDecision>,
}

impl<'w> PlayWindow<'w> {
    /// 此刻能出牌的人；`None` = 窗口关着。表现层用它决定按钮亮不亮，与规则层认不认
    /// 是同一句判据（同 `bank_op_allowed` 与 `can_bank_op` 的关系）。
    pub fn opener(&self) -> Option<Entity> {
        if self.phase.get() != &TurnPhase::WaitRoll || self.pending.active().is_some() {
            return None;
        }
        self.order.as_ref()?.current()
    }
}

/// 出牌。三道门：窗口（掷骰前、本人、没被模态占住）、卡在手、目标合法。
///
/// 三种拒绝都**静默返回**（同 `bank_op_system`）：面板该把办不动的按钮亮成灰的，
/// 而不是按下去没声音。
pub fn play_card_system(
    mut sink: CardSink,
    mut requests: MessageReader<PlayCardRequested>,
    window: PlayWindow<'_>,
    shelf: Shelf<'_>,
    mut played: MessageWriter<CardPlayed>,
) {
    let Some(current) = window.opener() else {
        return;
    };
    for request in requests.read() {
        // 主语只认当前回合玩家。ATM 那种「认人不认回合」是地理事实，而打出一张卡会改变
        // 别人的回合 —— 在别人回合里替他决定「现在用不用免罪卡」不是便利，是越权
        if request.player != current {
            debug!("出牌被拒：{:?} 不是当前回合玩家", request.player);
            continue;
        }
        let Some(def) = shelf.get(CardKind::Item, request.card) else {
            debug!("出牌被拒：货架上没有 {:?}", request.card);
            continue;
        };
        if !hand_holds(&sink.kit.hands, current, request.card) {
            debug!("出牌被拒：手里没有 {}", def.id);
            continue;
        }
        if !aim_is_legal(def, request.aim, current, &sink.books) {
            debug!("出牌被拒：{} 的目标不合法", def.id);
            continue;
        }
        // 「这块地归谁」是读世界，交给调用方查；表本身只管「谁承受」（见 `victim_of`）
        let land_owner = match request.aim {
            Some(CardAim::Land(land)) => sink.books.owner_of(land),
            Some(CardAim::Player(_)) | None => None,
        };
        let Some(victim) = victim_of(def, request.aim, current, land_owner) else {
            debug!("出牌被拒：{} 找不到承受者", def.id);
            continue;
        };
        if let Some(why) = unmet_precondition(def, current, victim, request.aim, &sink) {
            debug!("出牌被拒：{} —— {why}", def.id);
            continue;
        }
        // 先扣卡再结算：效果执行到一半崩了，也不该留下一张还能再打一次的卡
        if let Ok(mut hand) = sink.kit.hands.get_mut(current) {
            hand.spend(request.card);
        }
        let outcomes = apply_card(&mut sink, current, victim, def, request.aim);
        played.write(CardPlayed {
            player: current,
            victim,
            card: CardNotice {
                id: def.id.clone(),
                kind: def.kind,
                title: def.title.clone(),
                desc: def.desc.clone(),
                outcomes,
            },
        });
    }
}

/// `Query` 的只读借用不能与后面的 `get_mut` 同时活着，所以这里判完就放手。
fn hand_holds(hands: &Query<&mut Hand, With<PlayerId>>, player: Entity, card: CardId) -> bool {
    hands.get(player).is_ok_and(|hand| hand.holds(card))
}

/// 把一张卡发进某人手里（P3d 的百货公司与 P6 的神明送卡都走这一句）。
///
/// 单独一个函数而不是让调用方直接改 `Hand`：上限那条规则与「被挤掉要发流水」是一体的，
/// 散到各处就会有一处忘了发。
pub fn deal_to(
    hand: &mut Hand,
    limit: u32,
    card: CardId,
    player: Entity,
    shelf: &Shelf<'_>,
    gained: &mut MessageWriter<CardGained>,
) {
    let pushed_out = hand.gain(card, limit);
    gained.write(CardGained {
        player,
        card: card_notice(shelf, card),
        pushed_out: pushed_out.map(|lost| card_notice(shelf, lost)),
    });
}

/// 下标 → 卡面凭据。查不到时给一条写明「数据里没有这张卡」的空卡面，而不是不发流水：
/// 卡确实进过手牌，只是名字读不出来 —— 那种数据错误要在流水里看得见。
///
/// `pub` 是给百货公司用的（`shop::shop_op_done` 卖出那笔也要写一条 `CardNotice`）：
/// 「下标怎么翻成人话」这件事只该有一处实现，否则一处说「<缺卡 3>」、另一处说「未知卡」。
pub fn card_notice(shelf: &Shelf<'_>, id: CardId) -> CardNotice {
    let found = shelf.get(CardKind::Item, id);
    CardNotice {
        kind: found.map_or(CardKind::Item, |c| c.kind),
        title: found.map(|c| c.title.clone()).unwrap_or_default(),
        desc: found.map(|c| c.desc.clone()).unwrap_or_default(),
        id: found
            .map(|c| c.id.clone())
            .unwrap_or_else(|| format!("<缺卡 {id:?}>")),
        outcomes: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(n: u32) -> CardId {
        CardId(n)
    }

    #[test]
    fn a_full_hand_pushes_out_the_oldest_not_the_newcomer() {
        // PLAN §三「满了替换最前面那张」：先进来的先出局，玩家赌的是「后面那张更有用」
        let mut hand = Hand::default();
        assert_eq!(hand.gain(card(0), 2), None);
        assert_eq!(hand.gain(card(1), 2), None);
        assert_eq!(hand.gain(card(2), 2), Some(card(0)), "挤掉的是最早那张");
        assert_eq!(hand.cards, VecDeque::from([card(1), card(2)]));
        assert_eq!(hand.len(), 2, "上限不是「装得下再挤」，是恒等于上限");
    }

    #[test]
    fn a_limit_of_one_still_holds_the_last_card() {
        // 退化值也要成立：limit=1 时「买一张就丢一张」，而不是丢成空手
        let mut hand = Hand::default();
        hand.gain(card(7), 1);
        assert_eq!(hand.gain(card(8), 1), Some(card(7)));
        assert_eq!(hand.cards, VecDeque::from([card(8)]));
        assert_eq!(
            hand.gain(card(9), 0),
            Some(card(8)),
            "0 按 1 理解，不清空手牌"
        );
    }

    #[test]
    fn spending_takes_one_copy_not_every_copy() {
        let mut hand = Hand::default();
        for id in [3, 3, 4] {
            hand.gain(card(id), 15);
        }
        assert!(hand.spend(card(3)));
        assert_eq!(
            hand.cards,
            VecDeque::from([card(3), card(4)]),
            "两张里打掉一张"
        );
        assert!(hand.spend(card(3)));
        assert_eq!(hand.cards, VecDeque::from([card(4)]));
        assert!(!hand.spend(card(3)), "没有第三张，spend 要说假");
    }

    #[test]
    fn a_self_only_card_aims_at_its_player_and_takes_no_aim() {
        let def = CardDef {
            id: "item_insulate".into(),
            kind: CardKind::Item,
            title: "免罪卡".into(),
            desc: String::new(),
            cost_points: Some(20),
            target: CardTarget::SelfOnly,
            effects: vec![],
        };
        let me = Entity::from_raw_u32(5).expect("索引不该撞上保留值");
        assert_eq!(victim_of(&def, None, me, None), Some(me));
        // 带了目标反而是错的（面板不该给免罪卡画一排对手按钮），但承受者仍是我自己
        let other = Entity::from_raw_u32(6).expect("索引不该撞上保留值");
        assert_eq!(
            victim_of(&def, Some(CardAim::Player(other)), me, None),
            Some(me)
        );
    }

    #[test]
    fn a_targetless_framing_card_has_no_victim() {
        let def = CardDef {
            id: "item_frame".into(),
            kind: CardKind::Item,
            title: "陷害卡".into(),
            desc: String::new(),
            cost_points: Some(30),
            target: CardTarget::Player,
            effects: vec![],
        };
        let me = Entity::from_raw_u32(5).expect("索引不该撞上保留值");
        assert_eq!(
            victim_of(&def, None, me, None),
            None,
            "没选对手的陷害卡不该悄悄打在自己头上"
        );
    }

    /// 地块卡的承受者 = 这块地的业主（P3c 补上的那一格）。
    fn land_def() -> CardDef {
        CardDef {
            id: "item_hike".into(),
            kind: CardKind::Item,
            title: "涨价卡".into(),
            desc: String::new(),
            cost_points: Some(40),
            target: CardTarget::Land,
            effects: vec![],
        }
    }

    #[test]
    fn a_land_card_lands_on_the_owner_of_the_aimed_plot() {
        let me = Entity::from_raw_u32(5).expect("索引不该撞上保留值");
        let owner = Entity::from_raw_u32(7).expect("索引不该撞上保留值");
        assert_eq!(
            victim_of(&land_def(), Some(CardAim::Land(LandId(0))), me, Some(owner)),
            Some(owner),
            "流水里要说的是「谁的地被涨价了」，不是按按钮的那个人"
        );
    }

    #[test]
    fn an_unowned_plot_has_nobody_to_punish() {
        // 无主地查不出业主 → `None` → 出牌被拒。猜一个承受者的话，那个「业主」会在
        // 流水里摊上一次莫须有的查封（这正是 P3a 把这格留空时写下的理由）
        let me = Entity::from_raw_u32(5).expect("索引不该撞上保留值");
        assert_eq!(
            victim_of(&land_def(), Some(CardAim::Land(LandId(0))), me, None),
            None
        );
    }

    /// 购地卡与涨价卡指着同一种东西（一块地），承受者却相反：前者的买家是自己。
    /// 这两格在 `CardTarget` 上分成两格，就是为了让这张表不必去读 `effects` 才知道该问谁。
    /// 转向卡与陷害卡都「选一个人」，区别只在允不允许把自己填进去 —— 而那一条差别
    /// 整个住在 `aim_is_legal` 里，这张承受者表对两种人目标给出同一个答案：选中谁就是谁。
    #[test]
    fn an_anybody_card_takes_whomever_the_button_named() {
        let turn = CardDef {
            id: "item_turn".into(),
            kind: CardKind::Item,
            title: "转向卡".into(),
            desc: String::new(),
            cost_points: Some(25),
            target: CardTarget::Anybody,
            effects: vec![crate::card::CardEffect::Reverse],
        };
        let me = Entity::from_raw_u32(5).expect("索引不该撞上保留值");
        let foe = Entity::from_raw_u32(6).expect("索引不该撞上保留值");
        assert_eq!(
            victim_of(&turn, Some(CardAim::Player(me)), me, None),
            Some(me),
            "打给自己的转向卡，承受者就是自己 —— 这正是它不叫 `Player` 的那一格"
        );
        assert_eq!(
            victim_of(&turn, Some(CardAim::Player(foe)), me, None),
            Some(foe)
        );
        assert_eq!(
            victim_of(&turn, None, me, None),
            None,
            "没选人是错请求：面板上那一排里永远有「自己」那颗，不可能选不出人"
        );
    }

    #[test]
    fn buying_an_unowned_plot_makes_the_buyer_the_victim() {
        let grab = CardDef {
            id: "item_grab".into(),
            kind: CardKind::Item,
            title: "购地卡".into(),
            desc: String::new(),
            cost_points: Some(30),
            target: CardTarget::UnownedLand,
            effects: vec![crate::card::CardEffect::BuyLand],
        };
        let me = Entity::from_raw_u32(5).expect("索引不该撞上保留值");
        let owner = Entity::from_raw_u32(7).expect("索引不该撞上保留值");
        assert_eq!(
            victim_of(&grab, Some(CardAim::Land(LandId(0))), me, None),
            Some(me),
            "无主地没有业主，买它的人就是承受者"
        );
        // 带着一个查得到的业主也不改答案：购地卡买的是无主地，那一格在这里什么都不说明
        assert_eq!(
            victim_of(&grab, Some(CardAim::Land(LandId(0))), me, Some(owner)),
            Some(me)
        );
        assert_eq!(
            victim_of(&grab, None, me, None),
            None,
            "没选地就没有「买下哪块」这件事"
        );
    }
}
