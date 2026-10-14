//! 股票市场：行情 tick、分红、买卖。
//!
//! 对应 Godot `stock_market.gd`（200 行、`stocks` 与 `price_history` 两个弱类型字典）。
//! 这里字典没了：每只股一个实体，`StockId` / `StockPrice` / `PriceHistory` 是组件，
//! 静态配置仍在 `Assets<StockSet>` 里查（一份真相，见 §4.2）。
//!
//! 修掉的两条（`.agents/MIGRATION.md` §三）：
//! - **#4**：买卖时发的 `stock_price_changed(price, current_price)` 两个值相同
//!   （价格根本没变），真正的波动事件反而没有。这里拆成 `StockPriceChanged`（波动，
//!   带真实 old/new）与 `StockTraded`（成交，带股数与成交价）。
//! - **#5**：`buy()` 现金不足时递归调用自身且无深度保护。这里一次算清可买股数
//!   （`economy::buy_plan`），并且有一条「花费绝不超过现金」的扫表性质测试。
//!
//! ⚠️ 与 §4.5 表格的一处偏离：`stock_tick_system` 原写「`in_state(Landed)` 之前」，
//! 实际按 Godot 代码走 —— 只有踩到证券格才 tick（`turn_manager.gd:372-378`）。
//! 顺带记录一个数据缺口：`prototype.json` 的 12 格里**没有 stock 格**，
//! 所以照搬的话整局行情一动不动、分红永不发放（见 §三 表下的 B3 说明）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::TileKind;
use crate::data::DataHandles;
use crate::economy;
use crate::landing::{Landed, Landing};
use crate::msg::{StockPriceChanged, StockTraded, StockVisited, TradeSide};
use crate::rng::Rng;
use crate::settle::Books;
use crate::stock::{PriceHistory, StockId, StockPrice, StockSet};

/// 行情钟：全市场统一的 tick 计数（`stock_market.gd:21` 的 `tick_count`）。
/// 分红按它取模，所以只有一处能加它。
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

    /// 踩到证券格：推进行情，到点分红（Godot 的 `tick()` + `try_pay_dividends()` 那一对）。
    pub fn settle_visit(&mut self, books: &mut Books) {
        self.advance_prices(books);
        self.pay_dividends(books);
    }

    /// 一次 tick：每只股独立随机游走。迭代顺序由实体生成顺序决定，
    /// 同种子同开局即同序列（回放对得齐）。
    fn advance_prices(&mut self, books: &mut Books) {
        let rules = *books.rules;
        self.clock.tick += 1;
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

    /// 分红：每 `dividend_interval` 个 tick 一次，`floor(股价 × rate) × 股数`
    /// 进现金（`stock_market.gd:156-176`）。破产者不发（Godot 的 `status` 过滤）。
    fn pay_dividends(&mut self, books: &mut Books) {
        let rules = *books.rules;
        if self.clock.tick == 0 || rules.dividend_interval == 0 {
            return;
        }
        if !self.clock.tick.is_multiple_of(rules.dividend_interval) {
            return;
        }
        // 先把价格收成小表再发钱：迭代持仓时不该挂着 `self.stocks` 的可变借用
        let prices: Vec<(StockId, f32)> = self
            .stocks
            .iter()
            .map(|(_, id, price, _)| (id.clone(), price.0))
            .collect();
        for (player, _) in books.solvent() {
            let total = books
                .holdings_of(player)
                .iter()
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

/// 踩到证券格的分派入口。
pub fn settle_market_landing(
    mut books: Books,
    mut market: Market,
    landing: Res<Landing>,
    mut visited: MessageWriter<StockVisited>,
) {
    let Some(Landed {
        player,
        kind: TileKind::Stock,
        ..
    }) = landing.0
    else {
        return;
    };
    market.settle_visit(&mut books);
    visited.write(StockVisited { player });
}
