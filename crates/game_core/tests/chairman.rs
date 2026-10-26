//! 董事长特权（P10d②）的验收：百货白拿一件、建设免费升两层、银行融资三件套。
//!
//! 三条特权各挂一只股（10-20 拍板）：百货绑 `stock_004`、日产绑 `stock_002`、
//! 银行保险绑 `stock_003` —— 身份是持股数的**导出值**（`chairman_of` 每次现算），
//! 所以「反超即换人、换人即收回」不需要任何仪式，逐帧对账就够了。
//!
//! 演示盘格子对照（P10d② 重排后）：**16 百货公司**（15 西单掷 1 点即踩到）、
//! **4 日产建设**（3 号掷 1 点）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{GameSession, LandId, Pending, ShopStock};

/// 百货格。 chairman 白拿发生在 `settle_shop_landing`（整趟重抽）之后。
const SHOP: u32 = 18;

fn other(app: &mut App) -> Entity {
    roster(app)[1].0
}

fn shelf_len(app: &App) -> usize {
    app.world().resource::<ShopStock>().slots.len()
}

// ===== 百货白拿 =====

/// 董事长走到百货格先随机拿一件，再走正常进店消费流程 —— 白拿不花点券、
/// 货架真少一格、卡真进手牌，三件事各有一条证据。
#[test]
fn the_chairman_takes_one_item_before_shopping() {
    let (mut app, p0) = ready_on(SHOP - 1, 1, DecisionPolicy::Hold);
    clear_gods(&mut app); // 神明按自己的钟刷出：deed 直接改现金/点券，这里的账要干净
    let p1 = other(&mut app);
    set_shares(&mut app, p0, "stock_004", 1);
    set_points(&mut app, p0, 100);
    let hand_before = hand_of(&app, p0).len();
    roll(&mut app, p0);
    play_turns(&mut app, 1);
    let after_chairman_shelf = shelf_len(&app);
    let after_chairman_hand = hand_of(&app, p0).len();

    assert_eq!(tile_of(&app, p0), SHOP);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Shop)),
        "白拿不替他答柜台 —— 他还要走正常进店消费流程"
    );
    // 货架「少一格」的对照组：同一种子重跑一局、不给他股份 —— 重抽张数只由种子决定，
    // 两个世界的货架差那一格，就是白拿拿走的
    let (mut plain, _q0) = ready_on(SHOP - 1, 1, DecisionPolicy::Hold);
    set_shares(&mut plain, p1, "stock_004", 1); // 董事长是乙：p0 白拿免谈
    roll(&mut plain, _q0);
    play_turns(&mut plain, 1);
    assert_eq!(
        after_chairman_shelf + 1,
        shelf_len(&plain),
        "同一种子下，董事长的货架比普通访客少一件 —— 那件进了他的手牌"
    );
    assert_eq!(
        after_chairman_hand,
        hand_before + 1,
        "卡要真的进手牌（满手挤出是 deal_to 的事，这里手是空的）"
    );
    assert_eq!(points_of(&app, p0), 100, "白拿不动点券 —— 免费不是折扣");
    let led = ledger(&app);
    assert!(
        led.iter().any(|l| l.starts_with("Freebie(P0@stock_004,")),
        "免费要有一条说得清为什么的凭据：{led:?}"
    );
    assert!(
        !led.iter()
            .any(|l| l.starts_with("ShopOpDone(") || l.starts_with("Shop(")),
        "他还没买东西：白拿不该冒充一笔成交"
    );
}

/// 不是董事长就什么都没有 —— 白拿挂在股上，不挂格上。
#[test]
fn nobody_else_takes_anything() {
    let (mut app, p0) = ready_on(SHOP - 1, 1, DecisionPolicy::Hold);
    clear_gods(&mut app);
    let hand_before = hand_of(&app, p0).len();
    roll(&mut app, p0);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), SHOP);
    assert_eq!(hand_of(&app, p0).len(), hand_before, "没股份的人空手进店");
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Freebie(")),
        "白拿凭据也不该有"
    );
}

// ===== 建设免费升两层 =====

/// 建设公司（`stock_002`）的董事长在日产窗口里升自己的地：一次请求 +2 层、分文不花。
/// 普通玩家照价付 800 的那条对照在 `tests/remote_build.rs`。
#[test]
fn the_build_chairman_upgrades_two_levels_for_free() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::Hold);
    clear_gods(&mut app); // 衰神会 deny acquire、小财神折半造价 —— 升级判据要读干净的
    set_shares(&mut app, p0, "stock_002", 1);
    // 加盖判据要求整组同色地都在手：brown 组 = 南京路 + 汉口路
    own(&mut app, 0, Some(0), 1, false);
    own(&mut app, 1, Some(0), 1, false);
    roll(&mut app, p0);
    play_turns(&mut app, 1);
    assert_eq!(tile_of(&app, p0), 4, "前置：踩到日产建设");

    remote_build_ask(&mut app, p0, LandId(0));
    settle(&mut app, 2);

    assert!(
        ledger(&app).contains(&"Upgraded(P0@0→3)".to_string()),
        "一次请求升两层（Lv1 → Lv3）：{:?}",
        ledger(&app)
    );
    assert_eq!(
        cash_of(&mut app, p0),
        20_000,
        "特权是免费的 —— 不过 quoted 管线"
    );
}

// ===== 银行融资三件套 =====

/// ① 挪用：从他人存款划钱进自己现金，`financing` 记等额债（不计息、不与贷款混）；
/// 余额不足按实挪数成交；不是董事长的人发同样的请求，一分都挪不动。
#[test]
fn the_finance_chairman_borrows_from_deposits() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app); // 神明 deed 直接改现金，融资金额的断言要干净
    let p1 = other(&mut app);
    set_shares(&mut app, p0, "stock_003", 1); // p0 = 银行保险的董事长
    run_op(&mut app, move |books, _| {
        assert!(books.deposit(p1, 1_000), "前置：乙存了 1000");
    });

    financing_ask(&mut app, p0, p1, 800);
    settle(&mut app, 2);
    assert_eq!(cash_of(&mut app, p0), 20_800, "800 进董事长现金");
    assert_eq!(financing_of(&app, p0), 800, "融资债同步记账");
    assert_eq!(bank_of(&mut app, p1).0, 200, "乙的存款真被划走");
    assert_eq!(
        app.world().resource::<GameSession>().bank_vault,
        200,
        "储备同步 -800（挪的是真金，不是银行凭空造的）"
    );
    assert!(
        ledger(&app).contains(&"FinDraw(P0$800)".to_string()),
        "成交要留凭据：{:?}",
        ledger(&app)
    );

    // 挪 5000 而他只有 200：按实挪 200 —— 部分成交，不是拒也不是凭空发钱
    financing_ask(&mut app, p0, p1, 5_000);
    settle(&mut app, 2);
    assert_eq!(cash_of(&mut app, p0), 21_000);
    assert_eq!(financing_of(&app, p0), 1_000);
    assert_eq!(bank_of(&mut app, p1).0, 0);
    assert!(ledger(&app).contains(&"FinDraw(P0$200)".to_string()));

    // 不是董事长的人发同样的请求：读掉就丢
    run_op(&mut app, move |books, _| {
        assert!(books.deposit(p0, 500), "前置：甲自己也存了 500");
    });
    financing_ask(&mut app, p1, p0, 300);
    settle(&mut app, 2);
    assert_eq!(bank_of(&mut app, p0).0, 500, "乙挪不动甲的存款");
    assert_eq!(financing_of(&app, p1), 0);
    assert_cash_audit(&mut app, 20_000);
}

/// ② 兜底：储备被取穿（负数）后，董事长补差额 —— 补进的钱同样记成他的融资债。
///
/// 「取穿」从哪儿来：取款按**个人余额**钳（`withdraw_plan`），而储备与 Σ余额 的
/// 恒等式（存款/取款/挪用都两边同减）有一个已知的缺口 —— **月末利息入账只加
/// 储户余额、不加储备**（银行造钱付息，P2 的口径）。利息链路在 `tests/finance.rs`
/// 走过真路径，这里直接把那个差额摆出来：储备 1100、乙余额 1100，他取走全部，
/// 储备当场 -100。分两幕走：先**没有**董事长（谁也不兜，负数摆得住），再授身份
/// —— 下一帧兜底就该自动发生，否则这条规则等于没写。
#[test]
fn the_chairman_covers_the_vault_when_a_withdrawal_overdraws_it() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app);
    let p1 = other(&mut app);
    run_op(&mut app, move |books, _| {
        assert!(books.deposit(p1, 1_100));
    });
    // 摆出利息留下的缺口：储备比 Σ余额 少 100（一期利息的账目形状）
    app.world_mut().resource_mut::<GameSession>().bank_vault -= 100;
    // 乙取走名下全部 1100：个人余额够付 —— 此时还没有董事长，负储备没人兜
    run_op(&mut app, move |books, _| {
        assert!(
            books.withdraw(p1, 1_100),
            "取款只问个人余额 —— 透支是允许的"
        );
    });
    assert_eq!(
        app.world().resource::<GameSession>().bank_vault,
        -100,
        "前置：储备被取穿且无人兜底"
    );

    // 授予身份：下一帧兜底就该自己发生
    set_shares(&mut app, p0, "stock_003", 1);
    settle(&mut app, 2);
    assert_eq!(cash_of(&mut app, p0), 19_900, "董事长垫了 100 进储备");
    assert_eq!(financing_of(&app, p0), 100, "垫的钱记成他的融资债");
    assert_eq!(
        app.world().resource::<GameSession>().bank_vault,
        0,
        "差额补齐，储备回到 0"
    );
    assert!(
        ledger(&app).contains(&"FinDraw(P0$100)".to_string()),
        "兜底与挪用共用一条 FinDraw 凭据：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, 20_000);
}

/// ③ 收回：丢了董事长身份（持股被反超）的瞬间，融资债用他的现金当场清算。
#[test]
fn losing_the_chair_clears_the_debt_on_the_spot() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app);
    let p1 = other(&mut app);
    set_shares(&mut app, p0, "stock_003", 1);
    run_op(&mut app, move |books, _| {
        assert!(books.deposit(p1, 1_000));
    });
    financing_ask(&mut app, p0, p1, 800);
    settle(&mut app, 2);
    assert_eq!(financing_of(&app, p0), 800, "前置：他背着 800 的融资债");

    set_shares(&mut app, p1, "stock_003", 2); // 乙反超：董事长当场换人
    settle(&mut app, 2);

    assert_eq!(
        cash_of(&mut app, p0),
        20_000,
        "800 债用现金还清（20800 - 800）"
    );
    assert_eq!(
        financing_of(&app, p0),
        0,
        "债必须清零 —— 身份没了，挂在谁名下都没有意义"
    );
    assert_eq!(
        app.world().resource::<GameSession>().bank_vault,
        1_000,
        "还的钱进储备（1000 - 800 + 800）"
    );
    assert!(
        ledger(&app).contains(&"FinRecall(P0$800)".to_string()),
        "收回要留凭据：{:?}",
        ledger(&app)
    );
}

/// 现金不够还的部分记成银行的坏账：储备回不到原位，但债照清零。
#[test]
fn a_debt_bigger_than_his_pocket_becomes_bad_debt() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app);
    let p1 = other(&mut app);
    set_shares(&mut app, p0, "stock_003", 1);
    run_op(&mut app, move |books, _| {
        assert!(books.deposit(p1, 1_000));
    });
    financing_ask(&mut app, p0, p1, 800);
    settle(&mut app, 2);
    set_cash(&mut app, p0, 300); // 他把挪来的钱花光了

    set_shares(&mut app, p1, "stock_003", 2);
    settle(&mut app, 2);

    assert_eq!(cash_of(&mut app, p0), 0, "现金倾囊而还");
    assert_eq!(financing_of(&app, p0), 0, "债照清零");
    assert_eq!(
        app.world().resource::<GameSession>().bank_vault,
        500,
        "储备只回得来 300 —— 剩下 500 是银行的坏账，谁也不再欠"
    );
    assert!(
        ledger(&app).contains(&"FinRecall(P0$300)".to_string()),
        "凭据要写「实还多少」，顶栏那句「还上了多少」读的就是它：{:?}",
        ledger(&app)
    );
}
