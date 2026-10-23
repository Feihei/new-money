//! B3 经济核心的 App 级测试：落格分派、决策闭环、破产、银行、股票、卡牌。
//!
//! 覆盖 §十 B3 行的验收，并把 §三 缺陷清单里 #1/#3/#4/#5 各自钉住一条回归。
//! 纯算账部分（租金全分支、贷款基数、买卖取整、股价 clamp）在 `economy.rs` 的表驱动
//! 单测里，这里只跑「经过 ECS 接线的那一遍」—— 两边都要有：单测管判定，集成测管接线。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::economy::dividend_per_share;
use game_core::prelude::{
    BankBalance, Bankrupt, Books, CardEffect, CardKind, DataHandles, Date, Detention,
    DetentionPlace, GameRules, GameSeed, GameSession, GameSetup, StaySentence, StockId, TileIndex,
    TileKind, TradeSide, TurnPhase,
};

// prototype 的格子对照（`assets/data/boards/prototype.json`）：
// 1 南京路(L0 brown 1500) / 2 汉口路(L1 brown 1500，挂监狱分支) /
// 4-5 外滩(L2 orange 4400) / 7 徐家汇(L3 red 3800) / 9 王府井(L4 yellow 2800) /
// 11 西单(L5 yellow 3000)；3 命运 / 6 银行 / 8 机会 / 10 中央银行 / 0 起点。

fn me(app: &mut App) -> Entity {
    roster(app)[0].0
}
fn other(app: &mut App) -> Entity {
    roster(app)[1].0
}

fn plot(app: &mut App, land: u32) -> (Option<u32>, u8) {
    holdings(app)
        .into_iter()
        .find(|(id, _, _)| *id == land)
        .map(|(_, owner, level)| (owner, level))
        .expect("prototype 里有这块地")
}

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

/// 走完 0 号玩家这一格（`ready_on` 已把他摆好位置）。
fn land(app: &mut App) {
    play_turns(app, 1);
}

// ===== 买地 =====

#[test]
fn unowned_property_offers_a_buy() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    land(&mut app);

    assert!(
        ledger(&app).contains(&"BuyOffer(P0,@0)".to_string()),
        "无主地该挂出报价：{:?}",
        ledger(&app)
    );
    assert_eq!(plot(&mut app, 0), (None, 0), "光是报价不该改变归属");
    assert_eq!(cash_of(&mut app, p0), 20_000, "拒绝报价不动现金");
    assert_eq!(turn_count(&app), 1, "拒绝之后回合要收得掉");
}

#[test]
fn accepting_the_offer_takes_ownership_of_the_bare_ground() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    land(&mut app);

    let led = ledger(&app);
    assert!(
        led.contains(&"Bought(P0@0$1500)".to_string()),
        "成交要留下价格：{led:?}"
    );
    assert_eq!(
        plot(&mut app, 0),
        (Some(0), 0),
        "刚买到的是空地（级数重判：0=空地，买地不再钳到 1）"
    );
    assert_eq!(cash_of(&mut app, p0), 18_500);
    assert_cash_audit(&mut app, 20_000);
}

/// 无主而带楼的地：**一次买下地 + 上面的建筑**（10-22 实报，那条消息的前半句是
/// 「破产后剩下的地变成无主，但上面建筑也是保留的」，这一条接的是后半句）。
///
/// 价 = 地价 + 已盖的每一层加盖费（南京路 1500 + 3×800 = 3900，级数重判后建筑数 = level），
/// 拿到的等级是那三层的楼而不是被抹平成空地。这两半各一条断言：只断钱不断等级，「收了楼的钱
/// 再把楼拆掉」那种形状就看不见；只断等级不断钱，「按空地价卖一栋楼」那种形状也看不见。
///
/// 前置用 `own(…, None, 3, …)` 而不是先打破产一局：破产那一趟留下的形状已由
/// `cant_pay_rent_bankrupts_the_payer_and_frees_their_lands` 与 `tests/auction.rs` 钉住，
/// 这里要问的是「走到那一格的人付多少」，而那是同一份世界状态的下一件事。
#[test]
fn an_unowned_plot_with_a_house_on_it_is_bought_with_the_house() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    own(&mut app, 0, None, 3, false); // 无主的南京路，立着一栋三层的楼
    land(&mut app);

    let led = ledger(&app);
    assert!(
        led.contains(&"Bought(P0@0$3900)".to_string()),
        "报价要含那三层加盖费（地价 1500 + 3×800）：{led:?}"
    );
    assert_eq!(
        plot(&mut app, 0),
        (Some(0), 3),
        "买家拿到的是那栋三层的楼，不是一块被抹平的空地"
    );
    assert_eq!(cash_of(&mut app, p0), 16_100, "20000 - 3900");
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn a_buy_anyone_cannot_afford_changes_nothing() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    set_cash(&mut app, p0, 900);
    land(&mut app);

    assert_eq!(plot(&mut app, 0), (None, 0), "钱不够时点了「买」也不该成交");
    assert_eq!(cash_of(&mut app, p0), 900, "更不该扣成负数");
    assert_no_negative_cash(&mut app);
    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "买不成也要把回合收掉，不能挂死"
    );
}

// ===== 租金 =====

#[test]
fn rent_moves_cash_from_payer_to_owner() {
    let (mut app, _) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 0, Some(1), 0, false); // P1 拥有南京路（空地）
    land(&mut app);

    assert!(
        ledger(&app).contains(&"Rent(P0->P1@L0$60)".to_string()),
        "P0 踩到 P1 的地：{:?}",
        ledger(&app)
    );
    let payer = me(&mut app);
    let owner = other(&mut app);
    assert_eq!(cash_of(&mut app, payer), 19_940);
    assert_eq!(cash_of(&mut app, owner), 20_060);
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn monopoly_doubles_the_rent_in_play() {
    let (mut app, _) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    // brown 组两块都在 P1 手里 → land 0 的 60 翻成 120
    own(&mut app, 0, Some(1), 0, false);
    own(&mut app, 1, Some(1), 0, false);
    land(&mut app);

    assert!(
        ledger(&app).contains(&"Rent(P0->P1@L0$120)".to_string()),
        "垄断应翻倍：{:?}",
        ledger(&app)
    );
}

#[test]
fn mortgaged_land_charges_nothing_and_opens_no_dialog() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 0, Some(1), 4, true);
    land(&mut app);

    let led = ledger(&app);
    assert!(
        !led.iter().any(|e| e.starts_with("Rent(")),
        "抵押中的地块不该收租：{led:?}"
    );
    assert_eq!(cash_of(&mut app, p0), 20_000);
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "没得收租也不必弹收据");
}

/// 缺陷 #3 那一族 + 「破产者名下的地该放回市场」这条修正的联合回归。
///
/// 回归的是**归属**那一半：Godot 只清玩家自己的列表、registry 里还写着死者。
/// 楼不在清掉的里面（10-22 实报）—— 徐家汇那两层跟着地一起留在街上，
/// 所以下面断的是 `(None, 2)` 而不是旧版的 `(None, 0)`。
#[test]
fn cant_pay_rent_bankrupts_the_payer_and_frees_their_lands() {
    let (mut app, p0) = ready_on(5, 6, DecisionPolicy::DeclineAll); // 5 → 11 王府井
    own(&mut app, 3, Some(0), 2, false); // P0 自己有块徐家汇
    own(&mut app, 4, Some(1), 0, false); // yellow 组
    own(&mut app, 5, Some(1), 0, false); // 整组在手 → 空地租 160×2 = 320
    set_cash(&mut app, p0, 50);
    land(&mut app);

    let led = ledger(&app);
    assert!(
        led.contains(&"Bankrupt(P0欠320)".to_string()),
        "付不出 320 → 当场破产：{led:?}"
    );
    assert!(
        app.world().entity(p0).get::<Bankrupt>().is_some(),
        "状态必须与说法一致（Godot 端只回文案不改状态）"
    );
    assert!(
        !led.iter().any(|e| e.starts_with("Rent(")),
        "扣不到钱时业主一分也拿不着：{led:?}"
    );
    let owner = other(&mut app);
    assert_eq!(cash_of(&mut app, owner), 20_000, "P1 没收到钱");
    assert_eq!(
        plot(&mut app, 3),
        (None, 2),
        "破产者名下的地块要放回市场（Godot 漏了这步，租金会永远打给鬼），而楼留在地上"
    );
    assert_eq!(cash_of(&mut app, p0), 0);
    assert_no_negative_cash(&mut app);
}

// ===== 升级 =====

#[test]
fn upgrade_offer_only_comes_with_a_monopoly_and_accept_raises_the_level() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    own(&mut app, 0, Some(0), 1, false);
    own(&mut app, 1, Some(0), 1, false);
    land(&mut app);

    let led = ledger(&app);
    assert!(
        led.contains(&"UpgradeOffer(P0,@0$800)".to_string()),
        "垄断 + 等级齐平 → 该问要不要升级：{led:?}"
    );
    assert!(
        led.contains(&"Upgraded(P0@0→2)".to_string()),
        "接受后升到 2 级：{led:?}"
    );
    assert_eq!(plot(&mut app, 0), (Some(0), 2));
    assert_eq!(cash_of(&mut app, p0), 19_200, "20000 - 800");
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn landing_on_your_own_incomplete_group_ends_the_turn_quietly() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    own(&mut app, 0, Some(0), 1, false); // 只拿到 brown 组的一半
    land(&mut app);

    let led = ledger(&app);
    assert!(
        !led.iter().any(|e| e.starts_with("UpgradeOffer("))
            && !led.iter().any(|e| e.starts_with("BuyOffer(")),
        "未垄断不该问升级，自己的地不该问买：{led:?}"
    );
    assert_eq!(plot(&mut app, 0), (Some(0), 1), "不该有任何变动");
    assert_eq!(cash_of(&mut app, p0), 20_000);
}

// ===== 银行 =====

/// P2④ 之前，谁踩到银行格就替**全场所有人**结一次息 —— 访客顺路踩一脚，别人的账也动了，
/// 而同桌没路过银行的人永远涨不了息。现在利息只认日历：踩银行只开柜台、发 ATM 卡。
#[test]
fn a_bank_visit_no_longer_compounds_anyones_balance() {
    let (mut app, p0) = ready_on(6, 2, DecisionPolicy::DeclineAll); // 6 号掷 2 点停在 8 号银行
    let p1 = other(&mut app);
    insert(
        &mut app,
        p0,
        BankBalance {
            deposit: 10_000,
            loan: 0,
            financing: 0,
        },
    );
    insert(
        &mut app,
        p1,
        BankBalance {
            deposit: 0,
            loan: 4_000,
            financing: 0,
        },
    );
    land(&mut app);

    assert!(
        ledger(&app).contains(&"Bank(P0,存10000贷0)".to_string()),
        "到访流水记的是踩上去那一刻的余额：{:?}",
        ledger(&app)
    );
    assert_eq!(bank_of(&mut app, p0), (10_000, 0), "访客自己也不该结息");
    assert_eq!(bank_of(&mut app, p1), (0, 4_000));
}

/// 结息挂上日历之后的两条不变式：日子没到一分不动，到了就**全场**都动。
/// 「不只是访客」这条老语义留着，只是触发者从「谁踩到银行」换成了「今天几号」。
#[test]
fn month_end_compounds_every_players_balance() {
    // 锚点钉在 1 月 1 日：回合 29 结束是 1 月 30 日（不该结），回合 30 结束是 1 月 31 日 = 月末
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(1),
    );
    pin_epoch(&mut app, Date::new(2048, 1, 1));
    app.insert_resource(DecisionPolicy::DeclineAll);
    let p0 = boot_to_wait_roll(&mut app);
    place(&mut app, p0, 0);
    let p1 = other(&mut app);
    // 两个人都按在原地（P6b 之后必须的）：这一条测的是「结息挂日历」与「利息不经过现金」，
    // 而 30 个回合的自动走路会让人踩到路上的神 —— 神**是**会动现金的（大财神给四位、
    // 大穷神抢四分之一）。被按住的人照样进入 `WaitRoll`、照样结束回合（P4④），
    // 所以日历与结息那两条路径一步没少，只是路上没人踩东西。
    for player in [p0, p1] {
        insert(&mut app, player, StaySentence { turns_left: 80 });
    }
    insert(
        &mut app,
        p0,
        BankBalance {
            deposit: 10_000,
            loan: 0,
            financing: 0,
        },
    );
    insert(
        &mut app,
        p1,
        BankBalance {
            deposit: 0,
            loan: 4_000,
            financing: 0,
        },
    );

    play_turns(&mut app, 29);
    assert_eq!(
        (bank_of(&mut app, p0), bank_of(&mut app, p1)),
        ((10_000, 0), (0, 4_000)),
        "1 月 30 日不是月末，不该有人涨息（今天是 {}）",
        today(&app)
    );

    play_turns(&mut app, 30);
    assert_eq!(bank_of(&mut app, p0), (11_000, 0), "储金月息 10%");
    assert_eq!(
        bank_of(&mut app, p1),
        (0, 4_600),
        "贷款月息 15%，且这一位整局没碰过银行"
    );
    assert_eq!(
        cash_of(&mut app, p0),
        20_000,
        "利息不经过现金（`bank_system.gd:74-89`）"
    );
}

/// 缺陷 #1 的端到端回归：Godot 端所有人的可贷额度恒为 0。
#[test]
fn loan_limit_comes_from_owned_lands() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    own(&mut app, 0, Some(0), 1, false);
    own(&mut app, 1, Some(0), 1, false); // brown 两块 1500+1500 → 上限 6000

    run_op(&mut app, move |books: &mut Books, _| {
        assert!(
            books.borrow(p0, 6_000),
            "顶格该贷得到（缺陷 #1：Godot 这里是 0）"
        )
    });
    run_op(&mut app, move |books, _| {
        assert!(!books.borrow(p0, 1), "已顶格，再加一块也该拒")
    });

    assert_eq!(bank_of(&mut app, p0), (0, 6_000));
    assert_eq!(cash_of(&mut app, p0), 26_000, "贷款进现金");
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn deposit_and_withdraw_leave_a_full_cash_trail() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);

    run_op(&mut app, move |books, _| {
        assert!(books.deposit(p0, 5_000), "现金够就该存下");
        assert!(!books.deposit(p0, 999_999), "现金不够就该拒");
        assert!(
            !books.deposit(p0, 0),
            "存 0 元就该拒（Godot 的 test_deposit_zero）"
        );
        assert!(books.withdraw(p0, 2_000));
        assert!(!books.withdraw(p0, 9_000), "超出余额就该拒");
    });

    assert_eq!(bank_of(&mut app, p0), (3_000, 0));
    assert_eq!(cash_of(&mut app, p0), 17_000, "20000 -5000 +2000");
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn repaying_only_takes_what_is_actually_owed() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    own(&mut app, 0, Some(0), 1, false); // 1500 → 上限 3000

    run_op(&mut app, move |books, _| {
        assert!(books.borrow(p0, 3_000));
        assert!(
            books.repay(p0, 9_999),
            "多还的部分不收（`bank_system.gd:62`）"
        );
        assert!(!books.repay(p0, 100), "已经无债可还");
    });

    assert_eq!(bank_of(&mut app, p0), (0, 0));
    assert_eq!(cash_of(&mut app, p0), 20_000, "+3000 -3000");
    assert_cash_audit(&mut app, 20_000);
}

// ===== 股票 =====

/// prototype 没有证券格（数据缺口，见 §三 的 B3 说明），所以把 `tile` 那格临时改成
/// `stock` 类型来跑这条路径。改的是 `ActiveBoard.def`，落格分派每帧重新读它。
fn retile_stock(app: &mut App, tile: u32) {
    retile(app, tile, TileKind::Stock);
}

/// 行情由**回合结束**推进，与踩到哪格无关（2026-10-15 的规则分叉，见 `market.rs` 文件头）。
/// 这条断言的是「一天只走一次」：踩证券格不再 tick，否则同一天行情会动两遍、
/// 日历与分红也对不齐。
#[test]
fn prices_move_once_per_turn_end_and_a_stock_tile_adds_no_extra_tick() {
    let (mut app, _) = ready_on(0, 3, DecisionPolicy::DeclineAll);
    retile_stock(&mut app, 3);
    assert_eq!(tick_of(&app), 0, "开局那天还没过完");

    land(&mut app);
    assert_eq!(tick_of(&app), 1, "一个回合结束 = 一天");
    let led = ledger(&app);
    assert_eq!(
        led.iter().filter(|e| e.starts_with("Price(")).count(),
        5,
        "一次 tick = 每只股各推进一格：{led:?}"
    );
    assert!(
        led.contains(&"StockVisit(P0)".to_string()),
        "踩到证券格仍要发一条消息给提示行：{led:?}"
    );
    assert_ne!(
        price_of(&mut app, "stock_001"),
        80.0,
        "tick 后价格必须动（缺陷 #4：Godot 只在买卖时发「价格变了」，而那里价格不会变）"
    );

    play_turns(&mut app, 3);
    assert_eq!(
        tick_of(&app),
        3,
        "踩格不该多推行情：走到第 3 天就该是 3 次 tick"
    );
    // 休市日（周末 ∪ 节假日）只推日历不推行情 —— 三天里有几天开盘由锚点与日历算，
    // 不把初版那条「固定 15 条」写死（P2b 换真实公历后它会随开局月份漂移）
    let holidays = app.world().resource::<game_core::prelude::HolidaySet>();
    let epoch = app.world().resource::<GameSession>().epoch;
    let open = (1..=3)
        .filter(|t| !holidays.is_market_closed(game_core::prelude::date_of(*t, epoch)))
        .count();
    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|e| e.starts_with("Price("))
            .count(),
        open * 5,
        "开盘 {open} 天 = 每只股各推进一次"
    );
}

#[test]
fn dividends_land_on_the_calendar_day_only() {
    // 锚点 2048-01-01、每月 15 号派息：tick 14 = 1/15、tick 45 = 2/15、tick 74 = 3/15
    let mut app = boot(
        GameSetup::default(),
        GameRules {
            dividend_rate: 0.5,
            ..GameRules::instant()
        },
        GameSeed::default(),
    );
    pin_epoch(&mut app, Date::new(2048, 1, 1));
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    // 建仓用 `credit` 而不是直改钱包：现金审计要求每笔变动都留下流水。
    // P10d① 起持有「有门面的股票」（stock_001 科技 / stock_005 航空）走的是
    // 公司真账那一半 —— 这里要钉的是兜底公式，所以特意买**没有门面**的那只
    run_op(&mut app, move |books, _| books.credit(p0, 480_000));
    let stock = StockId("stock_004".to_string());
    run_op(&mut app, move |books, market| {
        let filled = market
            .trade(books, p0, &stock, TradeSide::Buy, 1_000)
            .expect("50 万现金买 1000 股该成交")
            .0;
        assert_eq!(filled, 1_000)
    });

    let rules = *app.world().resource::<GameRules>();
    let mut paid_total = 0i64;
    let mut paid_on: Vec<u32> = Vec::new();
    for tick in 1..=75 {
        let before = cash_of(&mut app, p0);
        run_market_tick(&mut app);
        let delta = cash_of(&mut app, p0) - before;
        let date = game_core::prelude::date_of(tick, Date::new(2048, 1, 1));
        let want = if game_core::prelude::is_dividend_day(date, rules.dividend_day) {
            dividend_per_share(price_of(&mut app, "stock_004"), &rules) * 1_000
        } else {
            0
        };
        assert_eq!(delta, want, "第 {tick} 天（{date}）的现金变动应只来自分红");
        if delta > 0 {
            paid_on.push(tick);
        }
        paid_total += delta;
    }
    assert_eq!(
        paid_on,
        vec![14, 45, 74],
        "派息日应恰好落在每月的 15 号（1 月 15、2 月 15、3 月 15）"
    );
    assert!(
        paid_total > 0,
        "75 次 tick 里一次分红都没发，说明分红线没接通"
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn buying_more_than_the_wallet_allows_fills_partial_once() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    set_cash(&mut app, p0, 1_000);

    let stock = StockId("stock_001".to_string());
    run_op(&mut app, move |books, market| {
        let (filled, price) = market
            .trade(books, p0, &stock, TradeSide::Buy, 10_000)
            .expect("有钱就该买到一部分");
        assert!(
            filled > 0 && filled < 10_000,
            "部分成交：{filled} 股 @ {price}"
        );
        assert!(
            (price * filled as f32).ceil() as i64 <= 1_000,
            "缺陷 #5：部分成交也不能超付"
        );
    });

    assert!(cash_of(&mut app, p0) >= 0);
    assert!(shares_of(&mut app, p0, "stock_001") > 0);
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn selling_what_you_do_not_have_is_a_no_op() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    let stock = StockId("stock_001".to_string());

    run_op(&mut app, move |books, market| {
        assert_eq!(
            market.trade(books, p0, &stock, TradeSide::Sell, 10),
            None,
            "一股不持有时的卖出该是无成交，而不是「卖 0 股也算成功」"
        );
    });
    assert_eq!(cash_of(&mut app, p0), 20_000);
}

#[test]
fn selling_everything_removes_the_position() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p0 = me(&mut app);
    let stock = StockId("stock_002".to_string());

    run_op(&mut app, {
        let stock = stock.clone();
        move |books, market| {
            market
                .trade(books, p0, &stock, TradeSide::Buy, 5)
                .expect("60 元一股，两万现金买 5 股没问题");
        }
    });
    assert_eq!(shares_of(&mut app, p0, "stock_002"), 5);

    run_op(&mut app, move |books, market| {
        let (sold, _) = market
            .trade(books, p0, &stock, TradeSide::Sell, 100)
            .expect("有 5 股就能卖");
        assert_eq!(sold, 5, "最多只能卖掉持有量（`stock_market.gd:129`）");
    });
    assert_eq!(
        shares_of(&mut app, p0, "stock_002"),
        0,
        "卖空后不该在持仓里留一个 0 股条目（`stock_market.gd:142-146`）"
    );
    assert_cash_audit(&mut app, 20_000);
}

// ===== 卡牌 =====

#[test]
fn card_tile_draws_exactly_one_card_and_ends_the_turn() {
    let (mut app, _) = ready_on(0, 5, DecisionPolicy::DeclineAll); // 5 号是命运
    land(&mut app);

    let led = ledger(&app);
    let cards: Vec<&String> = led.iter().filter(|e| e.starts_with("Card(")).collect();
    assert_eq!(cards.len(), 1, "踩一次卡牌格恰好抽一张：{led:?}");
    assert!(
        cards[0].starts_with("Card(P0:fate_"),
        "命运格发的该是命运牌：{}",
        cards[0]
    );
    assert_eq!(turn_count(&app), 1, "卡面读完（自动应答）回合就该收掉");
}

/// 缺陷 #3 的直接回归：扣不起的罚款必须真的把人判破产。
#[test]
fn a_fine_you_cannot_pay_bankrupts_you_for_real() {
    let (mut app, p0) = ready_on(5, 5, DecisionPolicy::DeclineAll); // 5 → 10 机会
    set_cash(&mut app, p0, 1_000);
    stack_card(&mut app, CardKind::Chance, 2); // chance_003 交通罚款 2000
    land(&mut app);

    let led = ledger(&app);
    assert!(led.contains(&"Card(P0:chance_003)".to_string()), "{led:?}");
    assert!(
        led.contains(&"Bankrupt(P0欠2000)".to_string()),
        "Godot 端这里只回一句「现金不足，破产！」却不改状态：{led:?}"
    );
    assert!(
        app.world().entity(p0).get::<Bankrupt>().is_some(),
        "状态要与说法一致"
    );
    assert_eq!(cash_of(&mut app, p0), 0);
    assert_no_negative_cash(&mut app);
}

#[test]
fn a_detain_card_parks_the_token_at_the_junction_and_skips_two_turns() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll); // 命运格
    stack_card(&mut app, CardKind::Fate, 0); // fate_001 直接入狱 3 回合
    land(&mut app);

    let led = ledger(&app);
    assert!(
        led.contains(&"Detain(P0@监狱,格2)".to_string()),
        "监狱分支挂在 2 号格，逻辑位置该停在接入点而不是死胡同：{led:?}"
    );
    assert_eq!(tile_of(&app, p0), 2, "回来时要从接入点继续沿环前进");
    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Jail,
            turns_left: 3
        }),
        "拘留组件要带着「他在哪儿」—— 那是这三个去处唯一被存下来的差别"
    );

    // 3 回合刑期 = 跳过 2 个回合，第 3 次轮到他时当场回来（B2b 定的递减规则）。
    // 两人轮流，所以要走到第 7 格才见到 P0 的第三次回合开始。
    play_turns(&mut app, 7);
    let ev = events(&app);
    assert_eq!(
        ev.iter()
            .filter(|e| e.starts_with("DetainSkip(P0@监狱,"))
            .count(),
        2,
        "应恰好跳过两格：{ev:?}"
    );
    assert!(
        ev.iter().any(|e| e.starts_with("Lifted(P0@监狱,格2")),
        "第三格该放出狱凭据，而且要说得出他从哪儿回来：{ev:?}"
    );
    assert_eq!(detention_of(&app, p0), None);
}

#[test]
fn move_to_card_sends_the_token_without_settling_the_new_tile() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll); // 命运格
    stack_card(&mut app, CardKind::Fate, 1); // fate_002 回到起点 +2000
    land(&mut app);

    let led = ledger(&app);
    assert!(
        led.contains(&"Teleport(P0→0)".to_string()),
        "瞬移要有凭据：{led:?}"
    );
    assert_eq!(tile_of(&app, p0), 0);
    assert_eq!(
        cash_of(&mut app, p0),
        22_000,
        "那 2000 来自卡面，不是过路费"
    );
    assert_eq!(
        led.iter().filter(|e| e.starts_with("Card(")).count(),
        1,
        "落到新格子不该再触发一次结算（照搬 Godot：`move_to` 不连锁）：{led:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn a_card_naming_a_missing_tile_says_so_instead_of_guessing() {
    let (mut app, p0) = ready_on(5, 5, DecisionPolicy::DeclineAll); // 5 → 10 机会
    // 把一张卡的效果改成「去一个棋盘上不存在的格」：牌堆按 `CardSet` 下标发牌，
    // 卡面在资产里，所以这里就地改资产内容
    {
        let w = app.world_mut();
        let handles = w.resource::<DataHandles>().clone();
        let mut sets = w.resource_mut::<bevy::asset::Assets<game_core::prelude::CardSet>>();
        sets.get_mut(&handles.chance).expect("机会牌已加载").0[4].effects =
            vec![CardEffect::MoveTo {
                tile_name: "不存在的街".into(),
            }];
    }
    stack_card(&mut app, CardKind::Chance, 4); // chance_005 生日快乐
    land(&mut app);

    assert_eq!(
        tile_of(&app, p0),
        10,
        "格名找不到时不该猜一个位置（Godot 曾把「找不到」回退成 0=起点）"
    );
    assert_eq!(cash_of(&mut app, p0), 20_000, "效果失效就不该有别的副作用");
}

// ===== 长跑 =====

#[test]
fn forty_turns_keep_the_ledger_consistent() {
    // §八 说的「对局级集成」：买地真的发生、账目平、没有负现金、状态机不挂死
    let mut app = boot(GameSetup::default(), GameRules::instant(), GameSeed(2_026));
    set_policy(&mut app, DecisionPolicy::AcceptAll);
    assert!(run_until_world(&mut app));
    play_turns(&mut app, 40);

    let led = ledger(&app);
    assert!(
        led.iter().any(|e| e.starts_with("Bought(")),
        "40 回合里一张地都没买上，说明报价线没接通：{led:?}"
    );
    assert!(
        led.iter().any(|e| e.starts_with("Rent("))
            || led.iter().any(|e| e.starts_with("Bankrupt(")),
        "有人买了地之后必然出现租金或破产，否则收租线没接通：{led:?}"
    );
    assert!(
        led.iter().any(|e| e.starts_with("Card(")),
        "40 回合里一张卡都没抽到，说明卡牌线没接通：{led:?}"
    );
    assert_cash_audit(&mut app, 20_000);
    assert_no_negative_cash(&mut app);

    let over = phase(&app) == TurnPhase::Over;
    let turns = turn_count(&app);
    assert!(
        turns == 40 || over,
        "要么跑满 40 回合，要么提前终局，实际 {turns} 回合、阶段 {:?}",
        phase(&app)
    );
    // `TileIndex` 是 newtype：这里顺手用它确认全员都在主环上
    for (entity, _, _, tile) in roster(&mut app) {
        assert!(tile < 19, "{entity:?} 掉出了主环：{tile}");
        let _ = TileIndex(tile);
    }
}

#[test]
fn the_same_seed_replays_the_same_economy() {
    // 回放/复现的根：把整条 ledger 一起比，而不只是骰点
    let run = |seed: GameSeed| {
        let mut app = boot(GameSetup::default(), GameRules::instant(), seed);
        set_policy(&mut app, DecisionPolicy::AcceptAll);
        assert!(run_until_world(&mut app));
        play_turns(&mut app, 12);
        (ledger(&app), events(&app))
    };

    let (books_a, flow_a) = run(GameSeed(3_333));
    let (books_b, flow_b) = run(GameSeed(3_333));
    assert!(!books_a.is_empty(), "12 回合里经济侧一条消息都没有");
    assert_eq!(
        books_a, books_b,
        "同种子必须复现同一条经济流水（含抽卡顺序）"
    );
    assert_eq!(flow_a, flow_b, "同种子必须复现同一条回合流水");
    assert_ne!(
        books_a,
        run(GameSeed(3_334)).0,
        "换种子却得到同一条经济流水，说明抽卡/行情没读种子"
    );
}
