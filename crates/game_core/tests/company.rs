//! 公司企业（P10d①）：门面格、董事长免单、损益账、真账分红。
//!
//! 四条验收，各对应 PLAN P10d① 的一个「必须真的如此」：
//! - **钱进账本不进口袋**：访客付费、董事长一分不得 —— 损益账是公司的账，
//!   不是「最大股东的钱包」的别名（分红日之前谁也别想花它）。
//! - **董事长免单连账都不产生**：与步行踩加油站同一格 —— 不是免掉一笔债。
//! - **分红分真账**：盈利按持股摊、亏损按比例扣（扣不出当场破产）——
//!   10-20 实报口径把「股价 × rate」那条纯比率公式降级成无账可查时的兜底。
//! - **一期一结**：分红日之后账页翻新，上月的服务费不再混进下月的分红。
//!
//! 落格走真路径（`place` + 掷骰 + `play_turns`），被测的是落格分派与结算 system。
//! 演示盘格子对照（P10d① 重排后）：**10 科技公司**（9 王府井掷 1 点即踩到，
//! C 型维修费 120/步）、**12 航空公司**（11 点券箱掷 1 点即踩到，B 型转盘 400/天）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{CompanyLedgers, GameRules, chairman_of};

/// 默认种子下神明会附身（小财神从别人钱包抽钱那种 `GodDeed`），
/// 而这一族的断言全是「钱为什么是这个数」—— 神明的剧本是另一族用例的事，清场。
fn two_players(app: &mut App) -> (Entity, Entity) {
    let roster = roster(app);
    assert_eq!(roster.len(), 2, "演示盘两人局");
    (roster[0].0, roster[1].0)
}

fn company_line(app: &App) -> Option<String> {
    ledger(app).into_iter().find(|l| l.starts_with("Company("))
}

fn dividend_lines(app: &App, stock: &str) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|l| l.starts_with("Dividend(") && l.contains(&format!("@{stock}:")))
        .collect()
}

fn page(app: &App, stock: &str) -> (i32, u32, i64) {
    let ledgers = app.world().resource::<CompanyLedgers>();
    let page = ledgers.find(stock).expect("该公司的账页存在");
    (page.year, page.month, page.net())
}

// ===== 踩到门面格 =====

/// 董事长踩自己公司：免单不是打折 —— 一分不付、账上也没有这一笔。
#[test]
fn the_chairman_steps_in_without_a_bill() {
    let (mut app, _) = ready_on(9, 1, DecisionPolicy::AcceptAll);
    let (p0, _p1) = two_players(&mut app);
    set_shares(&mut app, p0, "stock_001", 100);

    roll(&mut app, p0);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 10, "前置：从 9 号掷 1 点踩到科技公司");
    assert_eq!(cash_of(&mut app, p0), 20_000, "董事长一分不付");
    assert!(
        app.world()
            .resource::<CompanyLedgers>()
            .find("stock_001")
            .is_none(),
        "免单连账页都不开 —— 「没这回事」不是「记了一笔 0」"
    );
    let line = company_line(&app).expect("免单也要有收据（面板得说得出为什么）");
    assert!(line.contains("$0"), "收据金额 0：{line}");
    assert!(line.contains("董事长P0"), "收据说得清为什么免：{line}");
}

/// 访客付费：钱进公司的账，**不进董事长的口袋** —— 持股最多者免单、收租是两回事。
#[test]
fn a_visitor_pays_into_the_companys_books_not_anyones_pocket() {
    let (mut app, _) = ready_on(9, 1, DecisionPolicy::AcceptAll);
    let (p0, p1) = two_players(&mut app);
    // p1 一股就是董事长；p0 一股不持，是访客
    set_shares(&mut app, p1, "stock_001", 1);

    roll(&mut app, p0);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 10);
    // C 型维修费 120/步，骰面 1 → 120。钱离开钱包、进了账本
    assert_eq!(cash_of(&mut app, p0), 20_000 - 120, "访客照付");
    assert_eq!(
        cash_of(&mut app, p1),
        20_000,
        "董事长一分不得 —— 这是分红日才见的东西"
    );
    assert_eq!(page(&app, "stock_001").2, 120, "服务费记进当月损益账");
    let line = company_line(&app).expect("收据");
    assert!(line.contains("$120") && line.contains("董事长P1"), "{line}");
}

/// 航空公司（B 型）：转盘摇出几就出国几天，费用按天计，人被送去 `Away`。
#[test]
fn the_airline_bills_by_the_day_and_sends_him_abroad() {
    let (mut app, _) = ready_on(11, 1, DecisionPolicy::AcceptAll);
    let (p0, _p1) = two_players(&mut app);

    roll(&mut app, p0);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 12, "前置：从 11 号掷 1 点踩到航空公司");
    // 转盘摇数走服务子流，具体几天是种子的事；天数与费用、拘留必须对得上 ——
    // 「费用按天计」与「摇出几离场几天」是同一次摇数的两个读数
    let days = detention_of(&app, p0)
        .expect("B 型要把人送去国外")
        .turns_left;
    assert!((1..=6).contains(&days));
    assert_eq!(
        page(&app, "stock_005").2,
        400 * days as i64,
        "费用 = 每天单价 × 天数"
    );
    assert_eq!(cash_of(&mut app, p0), 20_000 - 400 * days as i64);
    let line = company_line(&app).expect("收据");
    assert!(line.contains(&format!("住{days}天")), "{line}");
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l.starts_with("Detain(P0@国外,格12)")),
        "出国那个地方不在棋盘上，接入点就是他此刻那一格——他已走到 12 号才被拦下（P4① 的试金石）",
    );
}

// ===== 分红 =====

/// 分红分的是当月真账：3000 利润 ÷ 1000 总股 = 每股 3，按持股比例摊。
/// 「有公司账的股票」不吃兜底公式 —— 同一天只发一次。
#[test]
fn dividends_pay_the_months_real_profit_by_share() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    boot_to_wait_roll(&mut app);
    let (p0, p1) = two_players(&mut app);
    set_shares(&mut app, p0, "stock_001", 600);
    set_shares(&mut app, p1, "stock_001", 400);
    // 分红日之前攒下的利润
    add_company_entry(&mut app, "stock_001", 3_000, "服务费");

    // 推到 15 号（月末锚点是 1 日，第 15 个 tick 就是 15 号）
    for _ in 0..15 {
        run_market_tick(&mut app);
    }

    assert_eq!(cash_of(&mut app, p0), 20_000 + 1_800, "600 股 × 每股 3");
    assert_eq!(cash_of(&mut app, p1), 20_000 + 1_200, "400 股 × 每股 3");
    let lines = dividend_lines(&app, "stock_001");
    assert_eq!(lines.len(), 2, "两位持股人各一条：{lines:?}");
    assert!(lines.iter().any(|l| l.contains("每股3×600股,当月3000")));
    // 账已结清：这一页空了（下月从零开始）
    assert_eq!(page(&app, "stock_001").2, 0);
}

/// 亏损月是负数、按比例扣现金 —— 扣不出的人当场破产（与租金同一条判据）。
#[test]
fn a_loss_month_bills_the_shareholders_and_bankrupts_the_overdrawn() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    boot_to_wait_roll(&mut app);
    let (p0, p1) = two_players(&mut app);
    set_shares(&mut app, p0, "stock_001", 1);
    set_shares(&mut app, p1, "stock_001", 999);
    add_company_entry(&mut app, "stock_001", -100_000, "保险赔付");
    set_cash(&mut app, p1, 50_000);

    for _ in 0..15 {
        run_market_tick(&mut app);
    }

    assert_eq!(cash_of(&mut app, p0), 20_000 - 100, "1 股 × 每股 -100");
    assert_eq!(
        cash_of(&mut app, p1),
        0,
        "999 股要扣 99,900，5 万现金付不起 → 破产清零"
    );
    assert!(
        ledger(&app).iter().any(|l| l.contains("Bankrupt(P1")),
        "破产要在流水里说得出是谁"
    );
    let lines = dividend_lines(&app, "stock_001");
    assert_eq!(lines.len(), 2, "扣钱也是分红，两人各一条：{lines:?}");
    assert!(lines.iter().any(|l| l.contains("每股-100")));
}

/// 没有公司账的股票走市场的兜底公式；有的走真账 —— 同一只股永远不会被两半同时发。
#[test]
fn a_stock_without_a_door_still_pays_the_market_fallback() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    let p0 = boot_to_wait_roll(&mut app);
    set_shares(&mut app, p0, "stock_001", 1_000);
    set_shares(&mut app, p0, "stock_004", 1_000);
    add_company_entry(&mut app, "stock_001", 3_000, "服务费");

    for _ in 0..15 {
        run_market_tick(&mut app);
    }

    // stock_001：真账 3000 ÷ 1000 股 = 每股 3 —— 与它的股价无关
    let real = dividend_lines(&app, "stock_001");
    assert_eq!(real.len(), 1, "{real:?}");
    assert!(real[0].contains("每股3×1000股,当月3000"), "{real:?}");
    // stock_004：没有门面 → 兜底 = floor(股价 × rate) × 股数。兜底那一半按人
    // 合并成一笔 credit、不发 Dividend 流水（P2b 时代的旧行为），所以只能用现金对账：
    let rules = *app.world().resource::<GameRules>();
    let per_share = game_core::economy::dividend_per_share(price_of(&mut app, "stock_004"), &rules);
    assert_eq!(
        per_share * 1_000,
        cash_of(&mut app, p0) - 20_000 - 3_000,
        "两半各发各的：真账 3000 + 兜底 {per_share}×1000"
    );
}

/// 摊不到一股（|利润| < 总股数）的月份不发也不扣 —— 与乐透「余数留池」同一口径。
#[test]
fn a_month_too_small_to_split_pays_nothing() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    let p0 = boot_to_wait_roll(&mut app);
    set_shares(&mut app, p0, "stock_001", 1_000);
    add_company_entry(&mut app, "stock_001", 999, "服务费");

    for _ in 0..15 {
        run_market_tick(&mut app);
    }

    assert_eq!(cash_of(&mut app, p0), 20_000, "999 ÷ 1000 = 每股 0，不发");
    assert!(dividend_lines(&app, "stock_001").is_empty());
    // 但账还是结了：这一页清空，下月不背这 999
    assert_eq!(page(&app, "stock_001").2, 0);
}

/// 跨月翻页：上月没结清的账不滚进下月（分红日结账是唯一的出口）。
#[test]
fn crossing_the_month_opens_a_fresh_page() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    let p0 = boot_to_wait_roll(&mut app);
    set_shares(&mut app, p0, "stock_001", 1_000);
    add_company_entry(&mut app, "stock_001", 3_000, "服务费");

    // 推过月末（30 天的月份）：15 号分掉 3000，之后翻新页
    for _ in 0..40 {
        run_market_tick(&mut app);
    }
    add_company_entry(&mut app, "stock_001", 7, "服务费");

    let (year, month, net) = page(&app, "stock_001");
    assert_eq!(net, 7, "新页只有新账：{year}-{month}");
    // 再到下个 15 号，分的只有这 7 块的整数部分（7 ÷ 1000 = 每股 0，什么都不发）
    for _ in 0..20 {
        run_market_tick(&mut app);
    }
    assert_eq!(
        cash_of(&mut app, p0),
        20_000 + 3_000,
        "上月的 3000 在上月分掉了"
    );
}

/// 持股反超就换人：董事长是持股数的导出值，每次踩格现算。
#[test]
fn whoever_holds_the_most_is_the_chairman_right_now() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    boot_to_wait_roll(&mut app);
    let (p0, p1) = two_players(&mut app);
    set_shares(&mut app, p0, "stock_001", 10);
    run_op(&mut app, move |books, _| {
        assert_eq!(chairman_of(books, "stock_001"), Some(p0), "10 股 vs 0 股");
    });
    set_shares(&mut app, p1, "stock_001", 11);
    run_op(&mut app, move |books, _| {
        assert_eq!(
            chairman_of(books, "stock_001"),
            Some(p1),
            "反超即换人，不需要任何仪式"
        );
    });
    set_shares(&mut app, p1, "stock_001", 10);
    run_op(&mut app, move |books, _| {
        assert_eq!(
            chairman_of(books, "stock_001"),
            Some(p0),
            "并列时先到者留任 —— 「反超才换人」是这条规则的操作化"
        );
    });
}

/// 破产者当不了董事长：他已离场，`solvent()` 把他筛掉了。
///
/// 直插 `Bankrupt` 组件而不是走完整清算：这一条验的只是 `chairman_of` 的那道筛，
/// 「破产摘股」是 P9 清算那族用例的事（那里已钉住股票归无主）—— 把两件事并进一条
/// 用例，红的理由就说不清是哪一半。
#[test]
fn a_bankrupt_holder_does_not_keep_the_chair() {
    let mut app = boot(
        game_core::prelude::GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    app.insert_resource(DecisionPolicy::AcceptAll);
    boot_to_wait_roll(&mut app);
    let (p0, p1) = two_players(&mut app);
    set_shares(&mut app, p0, "stock_001", 10);
    set_shares(&mut app, p1, "stock_001", 5);
    insert(&mut app, p1, game_core::prelude::Bankrupt);
    run_op(&mut app, move |books, _| {
        assert_eq!(
            chairman_of(books, "stock_001"),
            Some(p0),
            "5 股的死人不算数，10 股的活人当董事长"
        );
    });
}
