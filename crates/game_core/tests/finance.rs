//! B5b 经济入口的 App 级测试：ATM 资格、柜台风控、证券交易窗口、行情日历。
//!
//! 这一层要守的东西只有一句话：**资格判据在规则层**。面板会不会把按钮点错、AI 会不会
//! 替别人操作，都不该让「没路过银行的人凭空借到钱」成立，所以每条拒绝都单独钉一根。
//!
//! prototype 的格子对照（`assets/data/boards/prototype.json`）：
//! 4 外滩·北 / 5 外滩·南 / **6 银行** / 7 徐家汇(L3) / 8 机会 / **10 中央银行** / 11 西单 / 0 起点。
//! 从 5 号掷 2 点 = 经过 6（银行）、停在 7（无主地产）；从 4 号掷 2 点 = 经过 5、停在 6（银行）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::NextState;
use common::*;
use game_core::prelude::{
    AtmAccess, BankOp, Date, GameRules, GameSession, Pending, StockId, TradeSide, TurnOrder,
    TurnPhase,
};

fn me(app: &mut App) -> Entity {
    roster(app)[0].0
}
fn other(app: &mut App) -> Entity {
    roster(app)[1].0
}

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

/// 替当前玩家掷一次并把这一回合走完（`ready_on` 已经把他摆好位置）。
fn land(app: &mut App) {
    play_turns(app, turn_count(app) + 1);
}

/// 走到「柜台开着」的那一帧：p0 从 4 号掷 2 点停在 6 号银行格。
/// 用 `Hold` 策略 —— 自动应答会把柜台直接关掉，就什么都观察不到了。
fn open_counter() -> (App, Entity) {
    let (mut app, p0) = ready_on(4, 2, DecisionPolicy::Hold);
    roll(&mut app, p0);
    settle(&mut app, 8);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::BankTeller)),
        "前置没做到：柜台没开起来"
    );
    (app, p0)
}

// ===== ATM：路过即解锁，但只开到自己的下一回合 =====

#[test]
fn passing_a_bank_unlocks_cash_moves_but_not_borrowing() {
    let (mut app, p0) = ready_on(5, 2, DecisionPolicy::DeclineAll);
    assert_eq!(atm_holder(&app), None, "还没走路就该没有资格");

    land(&mut app);
    assert_eq!(
        atm_holder(&app),
        Some(p0),
        "经过 6 号银行格应解锁 ATM（停下才算柜台，路过就算 ATM）"
    );

    let cash_before = cash_of(&mut app, p0);
    bank_ask(&mut app, p0, BankOp::Deposit, 1_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p0).0, 1_000, "ATM 该办成存款");
    assert_eq!(cash_of(&mut app, p0), cash_before - 1_000);

    bank_ask(&mut app, p0, BankOp::Borrow, 5_000);
    settle(&mut app, 3);
    assert_eq!(
        bank_of(&mut app, p0).1,
        0,
        "ATM 上按不出贷款 —— 借钱要走进柜台"
    );
    let led = ledger(&app);
    assert!(
        led.contains(&"BankOp(P0存1000)".to_string()),
        "存款该留下流水：{led:?}"
    );
    assert!(
        !led.iter().any(|e| e.starts_with("BankOp(P0借")),
        "被拒的操作还留流水，UI 就会显示一笔没发生的贷款：{led:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn a_card_held_by_one_player_opens_no_window_for_the_other() {
    // 资格是「谁的」这件事必须显形：乙拿着卡不等于甲也能存
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = other(&mut app);
    app.world_mut().resource_mut::<AtmAccess>().0 = Some(p0);

    bank_ask(&mut app, p1, BankOp::Deposit, 1_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p1).0, 0, "没资格的人存不进去");
    assert!(
        !ledger(&app).iter().any(|e| e.starts_with("BankOp(P1")),
        "被拒的操作不该留流水"
    );

    // 同一句请求换个主语就办得成 —— 说明挡路的确实是资格，不是接线没通
    bank_ask(&mut app, p0, BankOp::Deposit, 1_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p0).0, 1_000);
}

#[test]
fn the_card_survives_other_players_turns_and_dies_at_its_owners_start() {
    // 「什么时候发卡」由上一条用例走真路径覆盖；这里只测「什么时候吞卡」，
    // 所以直接把轮转指针挪到谁身上 —— 否则 p1 那一掷会不会路过银行格，
    // 就把断言变成了对种子的赌注。
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = other(&mut app);
    app.world_mut().resource_mut::<AtmAccess>().0 = Some(p0);

    let begin_turn_for = |app: &mut App, who: Entity| {
        app.world_mut().resource_mut::<TurnOrder>().cursor = if who == p0 { 0 } else { 1 };
        app.world_mut()
            .resource_mut::<NextState<TurnPhase>>()
            .set(TurnPhase::TurnStart);
        settle(app, 2);
    };

    begin_turn_for(&mut app, p1);
    assert_eq!(current(&app), p1);
    assert_eq!(
        atm_holder(&app),
        Some(p0),
        "别人回合开始时不该顺手清掉 p0 的卡 —— 路过那一刻根本按不到"
    );

    begin_turn_for(&mut app, p0);
    assert_eq!(current(&app), p0);
    assert_eq!(atm_holder(&app), None, "自己的回合一开始，卡就该被吞掉");
}

// ===== 柜台：踩到才开，开了才能借还 =====

#[test]
fn stopping_on_the_bank_offers_the_counter_and_the_turn_waits_for_it() {
    let (mut app, _p0) = open_counter();
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "柜台开着时回合必须停在决策"
    );
    assert!(
        ledger(&app).iter().any(|e| e.starts_with("Bank(P0")),
        "利息与到站提示仍要走一条流水"
    );

    // 办完 → 回合结束
    set_policy(&mut app, DecisionPolicy::AcceptAll);
    settle(&mut app, 6);
    assert_eq!(pending_of(&app), None);
    assert_eq!(turn_count(&app), 1, "按「办完」才该收这一回合");
}

#[test]
fn every_counter_operation_moves_the_right_column() {
    let (mut app, p0) = open_counter();
    own(&mut app, 3, Some(0), 1, false); // 徐家汇 3800 × 2.0 = 可贷 7600
    set_cash(&mut app, p0, 10_000);

    for (op, amount) in [
        (BankOp::Deposit, 1_000),
        (BankOp::Withdraw, 500),
        (BankOp::Borrow, 5_000),
        (BankOp::Repay, 2_000),
    ] {
        bank_ask(&mut app, p0, op, amount);
        settle(&mut app, 3);
    }

    let (deposit, loan) = bank_of(&mut app, p0);
    assert_eq!(deposit, 500, "存 1000 取 500");
    assert_eq!(loan, 3_000, "借 5000 还 2000");
    assert_eq!(cash_of(&mut app, p0), 12_500, "10000-1000+500+5000-2000");
    let led = ledger(&app);
    for line in [
        "BankOp(P0存1000)",
        "BankOp(P0取500)",
        "BankOp(P0借5000)",
        "BankOp(P0还2000)",
    ] {
        assert!(led.contains(&line.to_string()), "缺一条柜台流水：{led:?}");
    }
    assert_cash_audit(&mut app, 20_000);
    assert_no_negative_cash(&mut app);
}

#[test]
fn borrowing_beyond_the_limit_is_refused_without_a_trail() {
    let (mut app, p0) = open_counter();
    // 一块地都不名下：可贷额度 0（缺陷 #1 修的是「恒为 0」，这条守的是「该有就有」的反面）
    bank_ask(&mut app, p0, BankOp::Borrow, 1_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p0).1, 0);
    assert_eq!(cash_of(&mut app, p0), 20_000, "被拒的贷款不能先把钱打进去");
    assert!(
        !ledger(&app).iter().any(|e| e.starts_with("BankOp(P0借")),
        "无流水才是「什么都没发生」的证据"
    );

    // 有了抵押品就借得到 —— 同一条请求，差别只在资格之外的那半边
    own(&mut app, 0, Some(0), 1, false); // 南京路 1500 × 2.0 = 3000
    bank_ask(&mut app, p0, BankOp::Borrow, 3_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p0).1, 3_000, "顶格额度内该借到");
    bank_ask(&mut app, p0, BankOp::Borrow, 1);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p0).1, 3_000, "再多 1 块就该拒");
}

#[test]
fn the_counter_belongs_to_the_player_it_was_offered_to() {
    let (mut app, p0) = open_counter();
    let p1 = other(&mut app);
    // 柜台挂在 p0 名下，p1 不能趁它开着替自己借钱
    bank_ask(&mut app, p1, BankOp::Borrow, 5_000);
    bank_ask(&mut app, p1, BankOp::Deposit, 1_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p1), (0, 0), "别人的柜台办不了自己的事");
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::BankTeller)),
        "且不该把柜台答掉"
    );
}

#[test]
fn repaying_more_than_owed_takes_only_what_is_owed() {
    let (mut app, p0) = open_counter();
    own(&mut app, 0, Some(0), 1, false);
    bank_ask(&mut app, p0, BankOp::Borrow, 3_000);
    settle(&mut app, 3);
    bank_ask(&mut app, p0, BankOp::Repay, 99_000);
    settle(&mut app, 3);
    let (deposit, loan) = bank_of(&mut app, p0);
    assert_eq!(loan, 0, "多还的部分不收");
    assert_eq!(deposit, 0, "还款只冲负债，不进存款");
    assert_eq!(cash_of(&mut app, p0), 20_000, "净账：+3000 再 -3000");
}

// ===== 证券：只在掷骰前可交易 =====

#[test]
fn buying_before_the_roll_fills_and_sells_after_it_does_not() {
    let (mut app, p0) = ready_on(5, 2, DecisionPolicy::Hold);
    // 掷骰之前：窗口开着
    trade(&mut app, p0, "stock_001", TradeSide::Buy, 10);
    settle(&mut app, 3);
    assert_eq!(
        shares_of(&mut app, p0, "stock_001"),
        10,
        "WaitRoll 里该买得到"
    );
    let price = price_of(&mut app, "stock_001");
    assert!(
        cash_of(&mut app, p0) <= 20_000 - (price * 10.0).ceil() as i64,
        "买股票要真扣钱"
    );

    // 掷出去之后停在 7 号无主地，`Hold` 让回合卡在 `WaitDecision` —— 那不是 WaitRoll
    roll(&mut app, p0);
    settle(&mut app, 8);
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "前置：回合该卡在决策上"
    );
    trade(&mut app, p0, "stock_001", TradeSide::Sell, 10);
    settle(&mut app, 3);
    assert_eq!(
        shares_of(&mut app, p0, "stock_001"),
        10,
        "「掷骰前可操作」之外的那段窗口不该受理交易"
    );
    assert!(
        !ledger(&app).iter().any(|e| e.starts_with("Trade(P0卖")),
        "被拒的交易不留流水"
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn a_trade_asked_by_somebody_else_is_dropped() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let (p0, p1) = (me(&mut app), other(&mut app));
    assert_eq!(current(&app), p0);
    // 轮到 p0，却是 p1 递进来的请求（面板缓存错主语、或 AI 代打写错人）
    trade(&mut app, p1, "stock_001", TradeSide::Buy, 10);
    settle(&mut app, 3);
    assert_eq!(
        shares_of(&mut app, p1, "stock_001"),
        0,
        "不是当前玩家就不受理"
    );
    assert_eq!(price_of(&mut app, "stock_001"), 80.0, "且完全没有成交痕迹");
}

// ===== 行情：一天一次，分红落在日历上 =====

#[test]
fn the_market_advances_once_per_turn_end() {
    let mut app = boot_default();
    boot_to_wait_roll(&mut app);
    assert_eq!(tick_of(&app), 0, "开局还没过完一天");
    play_turns(&mut app, 3);
    assert_eq!(tick_of(&app), 3, "三个回合结束 = 三天");
    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|e| e.starts_with("Price("))
            .count(),
        15,
        "一天 = 每只股各推进一次，不多走也不少走"
    );
}

#[test]
fn a_default_rules_game_pays_dividends_on_the_fifteenth_of_each_month() {
    // 锚点钉在 2048-01-01、每月 15 号派息：第 14 次 tick 之后才是第一次发钱
    let mut app = boot_default();
    pin_epoch(&mut app, Date::new(2048, 1, 1));
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    run_op(&mut app, move |books, _| books.credit(p0, 90_000));
    let stock = StockId("stock_001".to_string());
    run_op(&mut app, move |books, market| {
        market.trade(books, p0, &stock, TradeSide::Buy, 100);
    });

    let rules = *app.world().resource::<GameRules>();
    assert_eq!(rules.dividend_day, 15, "P2b 把派息日挪到月中");
    for _ in 0..13 {
        let before = cash_of(&mut app, p0);
        run_market_tick(&mut app);
        assert_eq!(cash_of(&mut app, p0), before, "还没到 15 号");
    }
    let before = cash_of(&mut app, p0);
    run_market_tick(&mut app);
    assert_eq!(today(&app), Date::new(2048, 1, 15));
    assert!(
        cash_of(&mut app, p0) > before,
        "第 14 次 tick 落在 1 月 15 日，该派息了"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// P2b ②：休市只挡股市。元旦与周六各测一次 —— ATM 存款照办，证券下单被拒；
/// 到了下一个工作日，同一条下单才成交。抵押那半条不用另测：`MortgageFacts` 里
/// 根本没有日历字段（结构上办不到），这一条正是把旧 `open` 位退役的依据。
#[test]
fn a_holiday_and_a_weekend_close_only_the_market() {
    let mut app = boot_default();
    pin_epoch(&mut app, Date::new(2048, 1, 1));
    let p0 = boot_to_wait_roll(&mut app);
    app.world_mut().resource_mut::<AtmAccess>().0 = Some(p0);

    // tick 0 = 元旦
    assert_eq!(today(&app), Date::new(2048, 1, 1));
    bank_ask(&mut app, p0, BankOp::Deposit, 1_000);
    settle(&mut app, 4);
    assert_eq!(cash_of(&mut app, p0), 19_000, "元旦银行该照办");
    trade(&mut app, p0, "stock_001", TradeSide::Buy, 1);
    settle(&mut app, 3);
    assert_eq!(shares_of(&mut app, p0, "stock_001"), 0, "元旦股市不开");

    // tick 3 = 1 月 4 日（周六）：同一条分叉再走一遍
    set_tick(&mut app, 3);
    bank_ask(&mut app, p0, BankOp::Deposit, 1_000);
    settle(&mut app, 4);
    assert_eq!(cash_of(&mut app, p0), 18_000, "周六银行该照办");
    trade(&mut app, p0, "stock_001", TradeSide::Buy, 1);
    settle(&mut app, 3);
    assert_eq!(shares_of(&mut app, p0, "stock_001"), 0, "周六股市不开");

    // tick 5 = 1 月 6 日（周一）：下单终于成交
    set_tick(&mut app, 5);
    trade(&mut app, p0, "stock_001", TradeSide::Buy, 1);
    settle(&mut app, 3);
    assert_eq!(shares_of(&mut app, p0, "stock_001"), 1, "工作日该放行");
}
