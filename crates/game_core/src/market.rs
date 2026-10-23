//! 股票市场：行情 tick、分红、买卖。
//!
//! 对应 Godot `stock_market.gd`（200 行、`stocks` 与 `price_history` 两个弱类型字典）。
//! 这里字典没了：每只股一个实体，`StockId` / `StockPrice` / `PriceHistory` 是组件，
//! 静态配置仍在 `Assets<StockSet>` 里查（一份真相，见 §4.2）。
//!
//! 修掉的两条（`.agents/ARCHITECTURE.md` §三）：
//! - **#4**：买卖时发的 `stock_price_changed(price, current_price)` 两个值相同
//!   （价格根本没变），真正的波动事件反而没有。这里拆成 `StockPriceChanged`（波动，
//!   带真实 old/new）与 `StockTraded`（成交，带股数与成交价）。
//! - **#5**：`buy()` 现金不足时递归调用自身且无深度保护。这里一次算清可买股数
//!   （`economy::buy_plan`），并且有一条「花费绝不超过现金」的扫表性质测试。
//!
//! ⚠️ 与 Godot 的一处**规则分叉**（2026-10-15 拍）：行情不再等玩家踩到证券格，
//! 而是**每个回合结束 tick 一次**（[`market_tick_on_turn_end_system`]）。
//! 理由：`prototype.json` 的 12 格里没有证券格，照搬 `turn_manager.gd:372-378` 就得到
//! 「整局行情一动不动、分红永不发放」；更重要的是没有日历的市场不是市场。
//! 分红因此改挂日历：**每月 15 号派息**（`calendar::is_dividend_day`），
//! 一天就是一个回合，于是「每月 15 号」与「行情每天更新」是同一个数。
//! 踩到证券格（将来 40 格棋盘会有）只报到、不再推行情。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::calendar::{HolidaySet, date_of, is_dividend_day};
use crate::company::CompanyLedgers;
use crate::data::DataHandles;
use crate::economy;
use crate::landing::{Landed, Landing, LandingDomain};
use crate::msg::{
    CompanyPayout, StockPriceChanged, StockTradeRequested, StockTraded, StockVisited, TradeSide,
};
use crate::rng::Rng;
use crate::settle::Books;
use crate::stock::{PriceHistory, StockId, StockPrice, StockSet};
use crate::turn::TurnOrder;

/// 行情钟：全市场统一的 tick 计数（`stock_market.gd:21` 的 `tick_count`）。
/// 一个 tick 就是一天，日历（[`crate::economy::date_of`]）由它推出来，分红按它落的那天。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MarketClock {
    pub tick: u32,
}

/// 市场侧的运行时访问：股票实体 + 静态定义 + 随机流。
/// 现金与持仓仍归 [`Books`]（两个参数包合起来才是一次完整成交），所以这里刻意不碰钱包。
#[derive(SystemParam)]
pub struct Market<'w, 's> {
    sets: Res<'w, Assets<StockSet>>,
    data: Res<'w, DataHandles>,
    /// 节假日表（P2b）：休市看它。只读资源，不与 `Books` 里的 `ResMut<GameSession>` 冲突。
    holidays: Res<'w, HolidaySet>,
    rng: ResMut<'w, Rng>,
    clock: ResMut<'w, MarketClock>,
    stocks: Query<
        'w,
        's,
        (
            Entity,
            &'static StockId,
            &'static mut StockPrice,
            &'static mut PriceHistory,
        ),
    >,
    changed: MessageWriter<'w, StockPriceChanged>,
    traded: MessageWriter<'w, StockTraded>,
    /// 公司分红的凭据（P10d①）。真正的分钱在 [`crate::company::pay_company_dividends`]，
    /// 这里只是替它把消息递出去 —— 分红的公式（真账 ÷ 总股数）与市场无关，
    /// 市场只负责「到点了」这个时刻。
    payouts: MessageWriter<'w, CompanyPayout>,
}

impl<'w, 's> Market<'w, 's> {
    /// 现价。刻意用直接访问字段的写法而不是 `self.set()` 这类辅助方法：
    /// 返回引用一旦挂在 `&self` 上，本函数后面 `self.stocks.iter_mut()` /
    /// `self.changed.write()` 就全被这一次不可变借用挡住（借用检查按整个 self 算）。
    pub fn price(&self, id: &StockId) -> Option<f32> {
        self.stocks
            .iter()
            .find(|(_, s, _, _)| *s == id)
            .map(|(_, _, price, _)| price.0)
    }

    pub fn tick_count(&self) -> u32 {
        self.clock.tick
    }

    /// 一个回合结束 = 一天过去：推行情，到点分红。
    /// 全项目只有 [`market_tick_on_turn_end_system`] 调它 —— 一天只走一次，日历才对得齐。
    ///
    /// 周末与节假日**只推日历、不推行情**（P2b：休市只挡股市，银行全年无休）。
    /// tick 照常 +1，否则「1 回合 = 1 天」这条日历的地基就断了：休市的那两天会变成不存在。
    pub fn tick_at_turn_end(
        &mut self,
        books: &mut Books,
        ledgers: &mut CompanyLedgers,
        commands: &mut Commands,
    ) {
        self.clock.tick += 1;
        let date = date_of(self.clock.tick, books.session.epoch);
        if !self.holidays.is_market_closed(date) {
            self.advance_prices(books);
        }
        self.pay_dividends(books, ledgers, commands, date);
    }

    /// 一次 tick：每只股独立随机游走。迭代顺序由实体生成顺序决定，
    /// 同种子同开局即同序列（回放对得齐）。
    fn advance_prices(&mut self, books: &mut Books) {
        let rules = *books.rules;
        let cap = rules.max_price_history;
        let set = self.sets.get(&self.data.stocks);
        for (_, id, mut price, mut history) in self.stocks.iter_mut() {
            let Some(def) = set.and_then(|s| s.find(&id.0)) else {
                continue;
            };
            let (old, new) = economy::stock_tick(&mut price.0, def, &mut self.rng, &rules);
            economy::record_price(&mut history.0, new, cap);
            self.changed.write(StockPriceChanged {
                stock: id.clone(),
                old,
                new,
            });
        }
    }

    /// 分红：**每月 15 号**发放，一天就是一个回合，于是「每月 15 号」与「行情每天
    /// 更新」是同一个数（P2b 起分红与月末结息是两个日子）。
    ///
    /// P10d① 起公式分两半（10-20 实报的口径）：**有公司账的股票分当月真账**
    /// （服务费进的多就分得多，亏损月按比例扣钱 —— 正负同一条公式），归
    /// [`crate::company::pay_company_dividends`]；**没有公司账的股票**（盘上没有
    /// 那家门面）回落到旧的兜底公式 `floor(股价 × rate) × 股数`。两边以
    /// [`CompanyLedgers::find`] 有没有这一页分家 —— 同一只股永远不会被两半同时发，
    /// 「盘上有没有门面」是互斥的那道闸。
    fn pay_dividends(
        &mut self,
        books: &mut Books,
        ledgers: &mut CompanyLedgers,
        commands: &mut Commands,
        date: crate::calendar::Date,
    ) {
        let rules = *books.rules;
        if !is_dividend_day(date, rules.dividend_day) {
            return;
        }
        // 先发公司真账这一半（会扣钱、会破产，必须在兜底之前算完）
        crate::company::pay_company_dividends(commands, books, ledgers, date, &mut self.payouts);
        // 兜底那一半：只发没进过公司账的股票。持股的名单重新收一遍 ——
        // 上一半可能刚让某位持股人破产（亏损月扣穿了），破产者不发
        let prices: Vec<(StockId, f32)> = self
            .stocks
            .iter()
            .map(|(_, id, price, _)| (id.clone(), price.0))
            .collect();
        for (player, _) in books.solvent() {
            let total = books
                .holdings_of(player)
                .iter()
                .filter(|(id, _)| ledgers.find(&id.0).is_none())
                .map(|(id, shares)| {
                    prices
                        .iter()
                        .find(|(p, _)| p == id)
                        .map(|(_, price)| economy::dividend_per_share(*price, &rules))
                        .unwrap_or(0)
                        * *shares as i64
                })
                .sum();
            books.credit(player, total);
        }
    }

    /// 成交：改持仓 + 走现金的唯一出口 + 发 `StockTraded`。
    /// 返回实际成交股数（现金不足时部分成交，见 `economy::buy_plan`）。
    pub fn trade(
        &mut self,
        books: &mut Books,
        player: Entity,
        stock: &StockId,
        side: TradeSide,
        shares: u32,
    ) -> Option<(u32, f32)> {
        let price = self.price(stock)?;
        let owned = books
            .holdings_of(player)
            .iter()
            .find(|(id, _)| id == stock)
            .map(|(_, n)| *n)
            .unwrap_or(0);

        let plan = match side {
            TradeSide::Buy => economy::buy_plan(price, books.balance(player), shares),
            TradeSide::Sell => economy::sell_plan(price, owned, shares),
        }?;
        let (filled, cash) = plan;

        match side {
            // `debit` 失败在这里不可能发生（`buy_plan` 已按现金算过），
            // 但用 `?` 而不是 `expect`：市场付不起钱就是不该成交
            TradeSide::Buy => books.debit(player, cash).ok()?,
            TradeSide::Sell => books.credit(player, cash),
        }
        books.set_holdings(
            player,
            stock.clone(),
            match side {
                TradeSide::Buy => owned + filled,
                TradeSide::Sell => owned - filled,
            },
        );
        self.traded.write(StockTraded {
            player,
            stock: stock.clone(),
            shares: filled,
            price,
            side,
        });
        Some((filled, price))
    }
}

/// 踩到证券格的分派入口。**只报到**：行情已经改到每回合结束推进（见文件头），
/// 这里再 tick 一次就会一天走两遍、日历与分红对不齐。
pub fn settle_market_landing(landing: Res<Landing>, mut visited: MessageWriter<StockVisited>) {
    let Some(Landed {
        player,
        domain: LandingDomain::Market,
        ..
    }) = landing.0
    else {
        return;
    };
    visited.write(StockVisited { player });
}

/// 回合结束 = 一天过去。与 `end_turn_system` 同批，两者互不冲突（一个动钱包与行情，
/// 一个动轮转与回合计数），所以不排顺序 —— 谁先跑都不改结果。
///
/// 刻意**不**挂 `not_paused`：暂停挡的是「玩家还能不能行动」，而这一句是状态机自己
/// 走到 `TurnEnd` 才触发的。若让它被暂停挡住而 `end_turn_system` 照跑，日历就会比
/// 回合数慢，「每月 1 号派息」变成一句看不见的错话。
pub fn market_tick_on_turn_end_system(
    mut commands: Commands,
    mut books: Books,
    mut market: Market,
    mut ledgers: ResMut<CompanyLedgers>,
) {
    market.tick_at_turn_end(&mut books, &mut ledgers, &mut commands);
}

/// 受理买卖请求。只在 `WaitRoll` 里跑 —— 「掷骰之前可以安排本回合的钱怎么花」这条
/// 窗口由状态机给，不靠 UI 记得把按钮点亮成灰的。
///
/// 只认当前回合玩家（与 `roll_dice_system` 同一手法）：别人递进来的请求读掉就丢，
/// 于是「甲的证券面板替乙买股」在结构上不可能发生。部分成交（现金不够买满）由
/// `economy::buy_plan` 负责，这里不补判。
pub fn trade_request_system(
    mut books: Books,
    mut market: Market,
    order: Res<TurnOrder>,
    mut requests: MessageReader<StockTradeRequested>,
) {
    let Some(current) = order.current() else {
        return;
    };
    // 周末与节假日休市（P2b）：不开盘就不许下单。只挡股市 —— 银行全年无休。
    // 挡在这里而不是 `economy::buy_plan` 里 —— 成交函数不该知道今天是星期几。
    if market
        .holidays
        .is_market_closed(date_of(market.tick_count(), books.session.epoch))
    {
        for r in requests.read() {
            debug!("交易被拒：周末股市休市（{:?} {}）", r.player, r.stock.0);
        }
        return;
    }
    for r in requests.read() {
        if r.player != current {
            debug!("交易被拒：{:?} 不是当前回合玩家", r.player);
            continue;
        }
        market.trade(&mut books, current, &r.stock, r.side, r.shares);
    }
}
