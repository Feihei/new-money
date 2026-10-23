//! 百货公司（P3d②）：点券买卖道具卡，回收打 9 折，每次进入货架不同。
//!
//! PLAN §三「百货公司」行有四条，这个文件落前两条（另两条不归这一格：「大/小福神送卡」
//! 是 P6 神明，「控股者白拿一件 + 每月 15 号盈余分红」是 P10 股东特权与日历）。
//! 三条口径决定了这里的形状：
//!
//! - **只认柜台**。与银行的借/还同一类：踩到那一格、停在上面、按「离开」之前这一段才算。
//!   路过百货格**不给**任何资格 —— ATM 那种「地理事实换长期窗口」是银行独有的待遇，
//!   而「逛店」这件事本来就要停下来。
//! - **只花点券**。与现金两套账（PLAN §一「永不通兑」），所以这里一次 `Wallet` 都不碰，
//!   `Books::award_points` / `Books::spend_points` 是唯一出入口。
//! - **货架每次进店重抽**。[`ShopStock`] 是「这一趟的货」，不是全店的库存表：买走一格就空
//!   一格，否则一次进店就能囤满 15 张同一张卡，「每次进入货架不同」这条当场白写。
//!   抽签走 `Rng` 的**百货子流**（见 `rng.rs`：买几次货不该改变下一张机会牌是谁）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::card::CardKind;
use crate::company::chairman_of;
use crate::deck::{CardId, Shelf};
use crate::hand::{Hand, card_notice, deal_to};
use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};
use crate::msg::{CardGained, ChairmanTook, ShopOpDone, ShopOpRequested, TradeSide};
use crate::player::PlayerId;
use crate::rng::Rng;
use crate::rules::GameRules;
use crate::settle::Books;
use crate::turn::TurnPhase;

/// 这一趟进店摆出来的货：一个 [`CardId`] = 一个货位。
///
/// 是资源而不是道路格上的实体（AGENTS.md 硬约束第 4 条「道路格不建实体」）：同一时刻只有
/// 一个人站在柜台前（`PendingDecision` 只挂一位），所以全盘一份货架就够 —— 与点券箱的
/// 冷却账本同一个处理。
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct ShopStock {
    /// 还没卖掉的货位，按抽签顺序（`Vec` 的顺序就是货架的陈列顺序，面板照它排）。
    pub slots: Vec<CardId>,
}

impl ShopStock {
    /// 这一趟摆了这张卡吗。
    pub fn stocks(&self, card: CardId) -> bool {
        self.slots.contains(&card)
    }

    /// 卖掉一格：返回是否真有这么一格。只删**第一个**匹配（同 `Hand::spend` 的口径 ——
    /// 货架是不放回抽的，本来也不该有重复，但判据写成一条而不是靠「数据不会重样」）。
    pub fn take_slot(&mut self, card: CardId) -> bool {
        match self.slots.iter().position(|held| *held == card) {
            Some(at) => {
                self.slots.remove(at);
                true
            }
            None => false,
        }
    }
}

/// 抽 `slots` 个货位（**不放回**）。
///
/// 纯函数只吃「池子多大」而不是整副卡定义，所以这一条能在单测里逐格断言，不必造一个世界。
/// 部分 Fisher-Yates：只把前 `want` 个位置换到位，剩下的整副重排是白做的。
pub fn restock(pool: usize, slots: usize, rng: &mut Rng) -> Vec<CardId> {
    if pool == 0 {
        return Vec::new();
    }
    let want = slots.min(pool);
    let mut ids: Vec<CardId> = (0..pool as u32).map(CardId).collect();
    for i in 0..want {
        // 候选池从 `i` 往后（前面的位置已经定稿），`last` 在 `i == pool-1` 时是 0 → 自交换
        let last = (pool - 1 - i) as u32;
        let j = i + rng.pick_shop(0..=last) as usize;
        ids.swap(i, j);
    }
    ids.truncate(want);
    ids
}

/// 回收价 = 卡面点券价 × 折扣分子 / 100，**向下取整**（20 → 18，45 → 40）。
///
/// 整数乘除而不是 `cost as f32 * 0.9`：这条折扣是玩家能算的账，多一次舍入就对不上一次。
pub fn buyback(cost: u32, percent: u32) -> u32 {
    cost.saturating_mul(percent) / 100
}

/// 一笔买卖办不成的原因。UI 的灰按钮文案与规则层的拒绝**共用这一张表**
/// （同 `hand::aim_is_legal`：判据只写一次，面板只是把它翻成人话）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShopDeny {
    /// 人不站在柜台前 —— 没踩到百货格，或者已经按了「离开」
    NoCounter,
    /// 这一趟的货架上没有这张卡（被别人买走了，或压根没摆出来）
    NotStocked,
    /// 手里没有这张卡
    NotHeld,
    /// 点券不够
    TooBroke,
    /// 这张卡没有 `cost_points` —— 数据错误，`CardSet::validate` 已在加载期拦下
    NoPrice,
}

impl ShopDeny {
    /// 灰按钮上要写的那句话。**每颗灰按钮都得带一句为什么**，否则「按不动」与「坏了」
    /// 在玩家眼里是同一件事（ARCHITECTURE §6.1 坑 27 的 UI 版）。
    pub fn label(self) -> &'static str {
        match self {
            Self::NoCounter => "先踩到百货格",
            Self::NotStocked => "这一趟没摆这张",
            Self::NotHeld => "手里没有这张",
            Self::TooBroke => "点券不够",
            Self::NoPrice => "数据里没有售价",
        }
    }
}

/// 买或卖此刻办不办得成。判据全部由调用方查好再传进来（同 `bank::bank_op_allowed`），
/// 于是这张表既能被 system 用，也能在单测里逐格断言。
///
/// 两问的顺序是刻意的：先问**窗口**（站在柜台前吗），再问**这一笔**。窗口不对时后面
/// 三条都不必查，而 UI 上也只有这一种情况需要把整块面板灰掉。
pub fn shop_op_allowed(
    side: TradeSide,
    at_counter: bool,
    price: Option<u32>,
    in_stock: bool,
    in_hand: bool,
    points: u32,
) -> Result<(), ShopDeny> {
    if !at_counter {
        return Err(ShopDeny::NoCounter);
    }
    let cost = price.ok_or(ShopDeny::NoPrice)?;
    match side {
        TradeSide::Buy => {
            if !in_stock {
                return Err(ShopDeny::NotStocked);
            }
            if points < cost {
                return Err(ShopDeny::TooBroke);
            }
            Ok(())
        }
        // 回收不问点券、也不问货架（手里那张本来就是从某一次货架上买来的），只问「有没有」
        TradeSide::Sell => {
            if in_hand {
                Ok(())
            } else {
                Err(ShopDeny::NotHeld)
            }
        }
    }
}

/// 踩到百货格：重抽一次货架，再把这一格变成「柜台」—— 挂上待决事项，回合停在
/// `WaitDecision` 直到玩家按「离开」（形状与 `bank::settle_bank_landing` 同一条）。
pub fn settle_shop_landing(
    landing: Res<Landing>,
    rules: Res<GameRules>,
    shelf: Shelf<'_>,
    mut rng: ResMut<Rng>,
    mut stock: ResMut<ShopStock>,
    mut pending: ResMut<PendingDecision>,
) {
    let Some(Landed {
        player,
        domain: LandingDomain::Shop,
        ..
    }) = landing.0
    else {
        return;
    };
    stock.slots = restock(shelf.items().len(), rules.shop_slots as usize, &mut rng);
    debug!("百货换货：{:?} 进店，货架 {:?}", player, stock.slots);
    pending.offer(player, Pending::Shop);
}

/// 白拿一桌要动的全部东西（§六 坑 8：九样参数散装就顶爆 clippy 的上限）。
#[derive(SystemParam)]
pub struct FreebieDesk<'w, 's> {
    pub books: Books<'w, 's>,
    pub pending: Res<'w, PendingDecision>,
    pub landing: Res<'w, Landing>,
    pub stock: ResMut<'w, ShopStock>,
    pub shelf: Shelf<'w>,
    pub hands: Query<'w, 's, &'static mut Hand, With<PlayerId>>,
    pub rng: ResMut<'w, Rng>,
    pub gained: MessageWriter<'w, CardGained>,
    pub took: MessageWriter<'w, ChairmanTook>,
}

/// 董事长白拿一件（P10d② 特权）：百货格绑的那只股的持股最多者，进店先**免费**
/// 从货架上随机拿走一件，然后照常走买/卖流程（窗口照开、点券照花）。
///
/// 排在 [`settle_shop_landing`] 之后（同一条 `OnEnter(Landed)` 链）：货架刚重抽完，
/// 白拿拿的就是「这一趟的货」。抽签走 `Rng` 的白拿子流（第十五条），它与百货
/// 陈列子流分家 —— 董事长逛不逛街，不该挪动别人的货架。
///
/// 「白拿」的账目形状：货架少一格、手牌多一张、**点券分文不动**。免费不是折扣
/// （花 0 券也算一次交易、也留 ShopOpDone），是「没这回事」—— 与董事长踩自家
/// 公司连流水都没有是同一条纪律，凭据由 [`ChairmanTook`] 单独给。
pub fn chairman_freebie_system(mut desk: FreebieDesk<'_, '_>) {
    let Some((player, Pending::Shop)) = desk.pending.active() else {
        return;
    };
    // 百货格绑哪只股（validate 已保证必带）—— 董事长身份挂在股上，不挂格上
    let Some(Landed { tile, .. }) = desk.landing.0 else {
        return;
    };
    let Some(tile) = desk.books.active.def.tile(tile.0 as i64) else {
        return;
    };
    let Some(stock_id) = tile.data.stock.clone() else {
        return;
    };
    if chairman_of(&desk.books, &stock_id) != Some(player) {
        return;
    }
    if desk.stock.slots.is_empty() {
        return; // 货架空了：白拿免谈（面板那句「被买空了」同样解释这一格）
    }
    let pick = desk
        .rng
        .pick_freebie(0..=(desk.stock.slots.len() - 1) as u32) as usize;
    let card = desk.stock.slots.remove(pick);
    if let Ok(mut hand) = desk.hands.get_mut(player) {
        deal_to(
            &mut hand,
            desk.books.rules.hand_limit,
            card,
            player,
            &desk.shelf,
            &mut desk.gained,
        );
    }
    desk.took.write(ChairmanTook {
        player,
        stock: stock_id,
        item: card_notice(&desk.shelf, card).title,
    });
}

/// 办一笔百货交易要动的全部东西。打包成参数包是这仓库的房规（§六 坑 8），而且这几样
/// 本来就是**一起成立或一起不成立**的：钱（`Books`）、货（`ShopStock`）、卡（`Hand`）、
/// 窗口（`PendingDecision` 与阶段）。少读一样就开出一条不该存在的买卖，而
/// `#[allow(clippy::too_many_arguments)]` 会把「一个 system 要读九样东西」这个信号吃掉。
#[derive(SystemParam)]
pub struct ShopDesk<'w, 's> {
    pub books: Books<'w, 's>,
    pub stock: ResMut<'w, ShopStock>,
    pub pending: Res<'w, PendingDecision>,
    pub phase: Res<'w, State<TurnPhase>>,
    pub hands: Query<'w, 's, &'static mut Hand, With<PlayerId>>,
    pub gained: MessageWriter<'w, CardGained>,
    pub done: MessageWriter<'w, ShopOpDone>,
}

/// 办一笔百货交易。不改回合状态、也不该改 —— 买与卖是玩家站在柜台前的自由动作，
/// 回合仍然由「离开」那一句交还（§4.6 红线在 UI 侧的镜像，同 `bank_op_system`）。
///
/// **只认柜台前那个人**：`Pending::Shop` 挂的是谁，就花谁的钱。别人的点击不能替他买东西
/// —— 与 ATM 的「认人不认回合」不同，这一格没有长期资格，窗口就是全部的判据。
///
/// 拒绝都走 `debug!` 之后静默返回：面板该把办不动的按钮亮成灰的（灰的理由由
/// [`ShopDeny::label`] 给），而不是按下去没声音。
pub fn shop_op_system(
    mut desk: ShopDesk<'_, '_>,
    shelf: Shelf<'_>,
    mut requests: MessageReader<ShopOpRequested>,
) {
    if matches!(desk.phase.get(), TurnPhase::Boot | TurnPhase::Over) {
        // 终局之后还在买卖卡，回放与存档就对不上账了
        return;
    }
    for r in requests.read() {
        let Some(def) = shelf.get(CardKind::Item, r.card) else {
            debug!("百货被拒：商品池里没有第 {:?} 张", r.card.0);
            continue;
        };
        let at_counter = desk.pending.active() == Some((r.player, Pending::Shop));
        let in_hand = desk
            .hands
            .get(r.player)
            .is_ok_and(|hand| hand.holds(r.card));
        if let Err(why) = shop_op_allowed(
            r.side,
            at_counter,
            def.cost_points,
            desk.stock.stocks(r.card),
            in_hand,
            desk.books.points_of(r.player),
        ) {
            debug!(
                "百货被拒：{:?} 想{:?}第 {:?} 张 —— {}",
                r.player,
                r.side,
                r.card.0,
                why.label()
            );
            continue;
        }
        // 三道门都过了才动手（价格、柜台、货或卡，都由上面那张表判完）
        trade(&mut desk, &shelf, r, def.cost_points.unwrap_or(0));
    }
}

/// 真正动手那一半。抽出来只为让「三道门都过了」与「扣钱、发货、留凭据」各自读得清楚。
///
/// 三步的顺序是有意的：**先扣钱**。[`Books::spend_points`] 不足额时一分都不扣，所以它排在
/// 动货架与动手牌之前 —— 反过来写的话，钱没扣成而货已经下架，那张卡在货架上就消失了。
fn trade(desk: &mut ShopDesk<'_, '_>, shelf: &Shelf<'_>, r: &ShopOpRequested, price: u32) {
    // 没有 `Hand` 组件的人既买不了也卖不了（开局装配就带着它，这里只是不为不可能留分支）
    let Ok(mut hand) = desk.hands.get_mut(r.player) else {
        return;
    };
    let (side, moved) = match r.side {
        TradeSide::Buy => {
            if !desk.books.spend_points(r.player, price) {
                return;
            }
            desk.stock.take_slot(r.card);
            deal_to(
                &mut hand,
                desk.books.rules.hand_limit,
                r.card,
                r.player,
                shelf,
                &mut desk.gained,
            );
            (TradeSide::Buy, price)
        }
        // 卖出与买入对称：先删卡再给钱，半途失败不许留下「点券到手而卡还在手里」
        TradeSide::Sell => {
            let back = buyback(price, desk.books.rules.shop_buyback);
            if !hand.spend(r.card) {
                return;
            }
            desk.books.award_points(r.player, back);
            (TradeSide::Sell, back)
        }
    };
    desk.done.write(ShopOpDone {
        player: r.player,
        side,
        card: card_notice(shelf, r.card),
        points: moved,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(slots: &[u32]) -> Vec<CardId> {
        slots.iter().map(|&at| CardId(at)).collect()
    }

    #[test]
    fn a_restock_draws_distinct_slots() {
        let shelf = restock(20, 3, &mut Rng::new(7));
        assert_eq!(shelf.len(), 3, "几个货位就该抽几张");
        let mut sorted = shelf.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), 3, "不放回抽签不该重样: {shelf:?}");
        for card in &shelf {
            assert!(card.0 < 20, "货位越界: {card:?}");
        }
    }

    /// 货位比卡多：把整副摆出来就够了，不该凭空多出格子（也不该 panic）。
    #[test]
    fn a_shelf_never_holds_more_than_the_pool() {
        assert_eq!(restock(5, 12, &mut Rng::new(1)).len(), 5);
        let mut sorted = restock(4, 4, &mut Rng::new(3));
        sorted.sort();
        assert_eq!(sorted, ids(&[0, 1, 2, 3]), "货位=池子时等于整副摆出");
        assert!(restock(0, 3, &mut Rng::new(3)).is_empty(), "空池子不该有货");
    }

    #[test]
    fn a_restock_is_seeded_and_seed_sensitive() {
        let shelf = |seed: u64| restock(20, 3, &mut Rng::new(seed));
        assert_eq!(shelf(11), shelf(11), "同种子必须复现同一趟货架");
        assert_ne!(shelf(11), shelf(12), "换种子同一趟货架说明抽签没读种子");
    }

    #[test]
    fn taking_a_slot_empties_only_that_one() {
        let mut stock = ShopStock {
            slots: ids(&[2, 0, 4]),
        };
        assert!(stock.stocks(CardId(0)));
        assert!(!stock.stocks(CardId(3)));
        assert!(stock.take_slot(CardId(0)));
        assert_eq!(stock.slots, ids(&[2, 4]), "只删那一格，顺序不变");
        assert!(!stock.take_slot(CardId(0)), "同一格卖两次 —— 货架不该允许");
    }

    /// 9 折 = 向下取整，而且是在整数上取：20→18、45→40、5→4。
    /// 玩家要能心算验证「这张卡卖回多少钱」，浮点乘会把这个数变成两次舍入的结果。
    #[test]
    fn the_buyback_is_a_rounded_down_discount() {
        for (cost, want) in [(20, 18), (45, 40), (5, 4), (30, 27), (0, 0)] {
            assert_eq!(buyback(cost, 90), want, "{cost} 点券的卡该卖回 {want}");
        }
        assert!(buyback(u32::MAX, 90) > 0, "大数不该溢出成 0");
    }

    #[test]
    fn buying_needs_stock_and_points_while_selling_only_needs_the_card() {
        // 买入：三样都得有 —— 柜台、货在架上、点券够
        assert_eq!(
            shop_op_allowed(TradeSide::Buy, true, Some(30), true, false, 30),
            Ok(())
        );
        assert_eq!(
            shop_op_allowed(TradeSide::Buy, true, Some(30), true, false, 29),
            Err(ShopDeny::TooBroke),
            "点券差 1 也算买不起"
        );
        assert_eq!(
            shop_op_allowed(TradeSide::Buy, true, Some(30), false, true, 500),
            Err(ShopDeny::NotStocked),
            "手里有同一张卡不等于货架还有一张"
        );
        // 卖出：不问点券、不问货架，只问手里有没有
        assert_eq!(
            shop_op_allowed(TradeSide::Sell, true, Some(30), false, true, 0),
            Ok(())
        );
        assert_eq!(
            shop_op_allowed(TradeSide::Sell, true, Some(30), true, false, 500),
            Err(ShopDeny::NotHeld)
        );
    }

    /// 窗口排在最前：不在柜台前时，连「买不买得起」都不必问。
    /// 这一条是「别人的点击不能替他花钱」与「离开之后还能买」的共同拒绝路径。
    #[test]
    fn nobody_but_the_customer_at_the_counter_may_trade() {
        for side in [TradeSide::Buy, TradeSide::Sell] {
            assert_eq!(
                shop_op_allowed(side, false, Some(30), true, true, 500),
                Err(ShopDeny::NoCounter),
                "{side:?} 在没有柜台时还放行"
            );
        }
    }

    #[test]
    fn a_card_without_a_price_is_a_data_error_not_a_free_item() {
        for side in [TradeSide::Buy, TradeSide::Sell] {
            assert_eq!(
                shop_op_allowed(side, true, None, true, true, 500),
                Err(ShopDeny::NoPrice),
                "{side:?} 让一张没有售价的卡办了 —— 那是白送或永远卖不掉"
            );
        }
    }

    /// 灰按钮的话要说得出，而且各不相同：面板上出现一句空字符串或两句同样的话，
    /// 玩家就分不清「没摆这张」与「点券不够」。
    #[test]
    fn every_denial_says_something_different() {
        let all = [
            ShopDeny::NoCounter,
            ShopDeny::NotStocked,
            ShopDeny::NotHeld,
            ShopDeny::TooBroke,
            ShopDeny::NoPrice,
        ];
        let words: Vec<&str> = all.iter().map(|d| d.label()).collect();
        assert!(words.iter().all(|w| !w.is_empty()), "有句灰按钮的话是空的");
        let mut uniq = words.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), all.len(), "有两种拒绝说了同一句话: {words:?}");
    }
}
