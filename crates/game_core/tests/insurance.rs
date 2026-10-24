//! 人寿保险（P10d②）的验收：保费按天、赔付按拘留天数、保单按自己的回合扣表。
//!
//! 拍板的口径（PLAN P10d②）与它要守的形状：
//! - **转盘摇出的是投保天数**，不是离场天数 —— 人原地不动，付 `premium_per_day × N`
//!   换一张 [`game_core::player::Insured`]；这与 B 型航空「摇几走几天」共享一个转盘、
//!   却是两种完全不同的产物，混进 `service::assess` 就是第二处会分叉的地方。
//! - **被拘留时按 `indemnity_per_day × 拘留天数` 赔现金**，公司账同时记**负数**
//!   （「亏损月是负数」从入账就是事实）；保单不因此作废。
//! - **免单不发保单**：免费卡把保费吃成 0 时公司没收着钱，保单就是凭空印的
//!   （与 P3c「凭据 = 改变了金额」同一条纪律）。
//!
//! 演示盘格子对照（P10d② 重排后）：**3 保险公司**（2 汉口路掷 1 点即踩到，
//! 转盘 4 面、保费 300/天、赔付 800/天）、**5 命运**（4 号掷 1 点抽卡）。

mod common;

use bevy::app::App;
use bevy::asset::Assets;
use bevy::ecs::entity::Entity;
use bevy::prelude::NextState;
use common::*;
use game_core::board::CompanyFee;
use game_core::prelude::{
    ActiveBoard, CardKind, CardSet, CompanyLedgers, DataHandles, Detention, DetentionPlace,
    Insured, Term, TurnOrder, TurnPhase,
};

/// prototype 上的保险公司格。
const INSURER: u32 = 3;

fn page_net(app: &App, stock: &str) -> Option<i64> {
    app.world()
        .resource::<CompanyLedgers>()
        .find(stock)
        .map(|p| p.net())
}

/// 命运池里某张牌的下标（与 `detention.rs` 同一条理由：按名字拿卡，不靠数组位置）。
fn fate_index(app: &App, id: &str) -> u32 {
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().fate;
    sets.get(handle)
        .expect("命运牌没加载")
        .0
        .iter()
        .position(|c| c.id == id)
        .unwrap_or_else(|| panic!("fate_cards.json 里没有 {id}")) as u32
}

/// 踩上保险公司格：p0 从 2 号掷 1 点。回 `(app, p0, 摇出的天数)`。
fn bought_a_policy() -> (App, Entity, u32) {
    let (mut app, p0) = ready_on(INSURER - 1, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app); // 神明按自己的钟刷出：小财神会把保费折半，这里验的不是它
    roll(&mut app, p0);
    play_turns(&mut app, 1);
    let days = policy_of(&app, p0).expect("投保要挂上保单").days_left;
    (app, p0, days)
}

/// 转盘摇出投保天数、保费按天计进公司账 —— 人原地不动，没有离场这回事。
#[test]
fn stepping_on_the_insurer_buys_a_policy_by_the_spin() {
    let (mut app, p0, days) = bought_a_policy();

    assert_eq!(
        tile_of(&app, p0),
        INSURER,
        "前置：从 2 号掷 1 点踩到保险公司"
    );
    assert!(
        (1..=4).contains(&days),
        "转盘 4 面，摇出的投保天数得在这一格里：{days}"
    );
    let policy = policy_of(&app, p0).expect("保单在身上");
    assert_eq!(policy.insurer, "stock_003", "保单挂着承保的那只股");
    assert_eq!(policy.indemnity_per_day, 800, "赔付单价写在格子上");
    let premium = 300 * days as i64;
    assert_eq!(
        cash_of(&mut app, p0),
        20_000 - premium,
        "保费 = 每天单价 × 天数"
    );
    assert_eq!(
        page_net(&app, "stock_003"),
        Some(premium),
        "保费进公司的当月账"
    );
    let led = ledger(&app);
    assert!(
        led.contains(&format!("Policy(P0@stock_003,{days}天${premium})")),
        "投保要有一条说得清「管几天、花多少」的凭据：{led:?}"
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Detain(")),
        "保险不是航空：人原地不动，不该有任何拘留"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 免费卡把保费吃成 0：公司没收着钱，就不欠任何保障 —— 保单不发。
///
/// 演示盘的保费（300×1..=4 ≤ 1200）够不到免费卡的门槛（2000×物价），所以这一条
/// 把承保格的保费临时改到 3000/天：门槛吃的是「实付超过 floor×物价才免」，改盘子
/// 而不是改卡，与 `coupon_tile` 同一条手法 —— 棋盘是 resource 里的一份快照。
#[test]
fn a_waived_premium_buys_no_policy() {
    let (mut app, p0) = ready_on(INSURER - 1, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app); // 同上：小财神的折半会把「免单」这场戏整个搅掉
    app.world_mut().resource_mut::<ActiveBoard>().def.tiles[INSURER as usize]
        .data
        .fee = Some(CompanyFee::Insure {
        sides: 4,
        premium_per_day: 3_000,
        indemnity_per_day: 800,
    });
    // 免费卡在掷骰之前打出（出牌窗口与证券同一条：WaitRoll 且无待决）
    let card = item_index(&app, "item_free").0;
    deal(&mut app, p0, card);
    play_card(&mut app, p0, card, None);
    settle(&mut app, 2);

    roll(&mut app, p0);
    play_turns(&mut app, 1);

    assert_eq!(cash_of(&mut app, p0), 20_000, "整笔免掉，钱包分文不动");
    assert!(
        policy_of(&app, p0).is_none(),
        "保费一分没掏，保单就是凭空印的 —— 不发"
    );
    let led = ledger(&app);
    assert!(
        !led.iter().any(|l| l.starts_with("Policy(")),
        "免单连投保凭据都不该有：{led:?}"
    );
    assert!(
        led.iter().any(|l| l.starts_with("Waive(P0$")),
        "免掉这件事本身要留凭据（否则「钱没动」读不出为什么）：{led:?}"
    );
    assert_eq!(
        page_net(&app, "stock_003"),
        None,
        "免单不记账 —— 公司没收着钱，分红日就不该多算一笔空账"
    );
}

/// 投保期间被拘留：按拘留天数赔现金，公司账同时记**负数**，保单不作废。
///
/// 保单用直插 `Insured` 摆好 —— 「怎么投保」上一条用例已经走真路径验过，
/// 这一条要守的是赔付那半条管线；拘留走真路径（命运 `fate_007` 掉水沟 = 住院 3 天），
/// 于是「赔付盯的是 `PlayerDetained` 消息」这条接线被真的走到。
#[test]
fn a_detention_during_the_policy_pays_for_every_day_he_is_held() {
    let (mut app, p0) = ready_on(4, 1, DecisionPolicy::DeclineAll); // 4 号掷 1 点 → 5 号命运
    insert(
        &mut app,
        p0,
        Insured {
            days_left: 4,
            insurer: "stock_003".to_string(),
            indemnity_per_day: 800,
        },
    );
    let fate = fate_index(&app, "fate_007");
    stack_card(&mut app, CardKind::Fate, fate);
    clear_gods(&mut app); // 神明的 deed 会直接改现金，赔付那条断言就废了
    play_turns(&mut app, 1);

    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Hospital,
            term: Term::Turns(3)
        }),
        "前置：掉进水沟住院 3 天"
    );
    assert_eq!(
        cash_of(&mut app, p0),
        20_000 + 800 * 3,
        "赔付 = 每天单价 × 拘留天数"
    );
    assert_eq!(
        page_net(&app, "stock_003"),
        Some(-2_400),
        "公司账记负数 —— 「亏损月是负数」从入账就是事实，分红日按比例扣持股人"
    );
    let led = ledger(&app);
    assert!(
        led.contains(&"Indemnity(P0@stock_003,3天$2400)".to_string()),
        "赔付要有一条凭据：{led:?}"
    );
    assert!(
        policy_of(&app, p0).is_some(),
        "保单不因赔了一次就作废：效期还剩几天照旧走表 —— 这是一段时期的保障，不是一次性赔案"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 保单到期是静默的：效期走完组件消失，没有凭据、也没有「续保」这回事。
///
/// 走表不掷骰：手动把轮转指针拨到谁身上再进 `TurnStart`（`finance.rs` 的同一手法）——
/// 掷骰会把他带去别的格子，而这条要守的只是「表走完、组件消失」。
fn begin_turn_for(app: &mut App, _who: Entity, seat: u32) {
    app.world_mut().resource_mut::<TurnOrder>().cursor = seat as usize;
    app.world_mut()
        .resource_mut::<NextState<TurnPhase>>()
        .set(TurnPhase::TurnStart);
    settle(app, 2);
}

#[test]
fn the_policy_expires_after_its_days_run_out() {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    insert(
        &mut app,
        p0,
        Insured {
            days_left: 1,
            insurer: "stock_003".to_string(),
            indemnity_per_day: 800,
        },
    );
    // T1 的 TurnStart 在插入之前已经走过了 —— 下一格是他自己的回合开头
    begin_turn_for(&mut app, p0, 0);

    assert!(
        policy_of(&app, p0).is_none(),
        "一天 = 一个自己的回合：他的回合一开始就走表，走到 0 组件就该消失"
    );
    assert!(
        !ledger(&app)
            .iter()
            .any(|l| l.starts_with("Indemnity(") || l.starts_with("Policy(")),
        "到期不发凭据（与「停留到期」同一口径：持久的读数是组件，不是流水）"
    );
}

/// 保单只扣**当前回合玩家**的那一份 —— 一天 = 一个自己的回合，
/// 与拘留/停留同一把尺。别人的回合走多少天，他的保单一格都不该少。
#[test]
fn a_policy_only_ticks_on_its_holders_own_turn() {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = roster(&mut app)[1].0;
    insert(
        &mut app,
        p0,
        Insured {
            days_left: 5,
            insurer: "stock_003".to_string(),
            indemnity_per_day: 800,
        },
    );
    begin_turn_for(&mut app, p0, 0);
    assert_eq!(
        policy_of(&app, p0).unwrap().days_left,
        4,
        "自己的回合开头扣一格"
    );
    begin_turn_for(&mut app, p1, 1);
    assert_eq!(
        policy_of(&app, p0).unwrap().days_left,
        4,
        "乙的回合开头不扣他的表"
    );
    begin_turn_for(&mut app, p0, 0);
    assert_eq!(
        policy_of(&app, p0).unwrap().days_left,
        3,
        "再一轮他自己的开头再扣一格"
    );
}
