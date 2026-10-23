//! 抵押与赎回（P3d③）的 App 级测试：真踩银行那一格、真按柜台上的按钮。
//!
//! 这一层要守的四件事：
//! - **窗口**：与借/还同一格 —— 踩到银行、停在上面、按「办完」之前那一段才办得成。
//!   银行全年无休（P2b），周末走进柜台照样办；别人的点击、离开之后的点击都不算。
//! - **一块地只押一次**：同一帧里两条押同一块地的请求只成一次（这条是 `Mortgaged`
//!   从「存在即押」的单位标记改成「带本金的值」的全部理由）。
//! - **它进的是负债那一格，不是资产的黑洞**：押出去对物价指数必须中性，否则
//!   「谁缺钱」就在替「所有人的房租」定价。
//! - **写侧真的接上了读侧**：押出去之后 `economy` 那三处判定（租金 / 垄断 / 可贷额度）
//!   当场跟着变，而不是只有组件写了进去。
//!
//! prototype 的地块对照：**0 南京路**（地价 1500、加盖费 800、brown 组）。
//! 默认规则下 Lv1 的南京路账面 1500 → 押到手 **750**、赎回 **825**。
//! 这两个数写死在用例里是故意的：改 `mortgage_percent` / `redeem_percent` 必须同时改这里，
//! 那是一次口径变更，不该悄悄过去。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{LandStatus, MortgageOp, Pending, TurnPhase};

/// 柜台开着、p0 名下有 0 号南京路（空地 Lv0）的那一帧。
fn desk() -> (App, Entity) {
    // 4 号掷 2 点 = 停在 6 号银行格（与 `finance.rs` 的柜台同一条前置）
    let (mut app, p0) = ready_on(4, 2, DecisionPolicy::Hold);
    own(&mut app, 0, Some(0), 0, false); // 级数重判后 0=空地，账面就是地价 1500
    roll(&mut app, p0);
    settle(&mut app, 8);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::BankTeller)),
        "前置没做到：踩到银行格却没开出柜台"
    );
    (app, p0)
}

fn take(app: &mut App, player: Entity, land: u32) {
    mortgage_ask(app, player, MortgageOp::Take, land);
    settle(app, 3);
}

fn redeem(app: &mut App, player: Entity, land: u32) {
    mortgage_ask(app, player, MortgageOp::Redeem, land);
    settle(app, 3);
}

fn mortgage_lines(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|line| line.starts_with("Mortgage("))
        .collect()
}

// ===== 押出去 =====

#[test]
fn mortgaging_pays_half_the_book_value_and_leaves_a_trail() {
    let (mut app, p0) = desk();
    assert_eq!(mortgage_of(&app, 0), None, "前置：这块地还没押");

    take(&mut app, p0, 0);

    assert_eq!(cash_of(&mut app, p0), 20_750, "1500 的账面该到手 750");
    assert_eq!(mortgage_of(&app, 0), Some(750), "本金要存在地块上");
    assert_eq!(
        mortgage_lines(&app),
        vec!["Mortgage(P0押L0·750)".to_string()],
        "成交凭据要认得出是哪块地、过手多少"
    );
    // 到手那一笔走的是 `Books::credit`，所以现金审计仍然逐条对得上
    assert_cash_audit(&mut app, 20_000);
}

/// 加盖花掉的钱要在抵押里回来一部分，否则「先投资再救急」等于把建筑费扔了。
/// Lv3 = 地价 1500 + 加盖三次 ×800 = 3900 → 到手 1950（级数重判后建筑数 = level）。
#[test]
fn a_built_plot_mortgages_for_what_was_spent_on_it() {
    let (mut app, p0) = desk();
    own(&mut app, 0, Some(0), 3, false);

    take(&mut app, p0, 0);

    assert_eq!(cash_of(&mut app, p0), 21_950, "账面 3900 该到手 1950");
    assert_eq!(mortgage_of(&app, 0), Some(1950));
}

/// 写侧接上读侧：押出去之后，`Books::rent_of`（踩上去要收的那同一个数）当场归零。
/// 这条是「抵押不是只写了个组件」的证据 —— 三处判定里最玩家可见的那一处。
#[test]
fn a_mortgaged_plot_stops_earning_while_it_is_held() {
    let (mut app, p0) = desk();
    let before = rent_of(&mut app, 0);
    assert!(before > 0, "前置：空地也收租（Lv0 查 rent[0]）");

    take(&mut app, p0, 0);

    assert_eq!(
        rent_of(&mut app, 0),
        0,
        "押给银行的地还在收租 —— 谁付给谁？"
    );
}

// ===== 赎回来 =====

#[test]
fn redeeming_costs_a_tenth_more_than_it_paid_and_unlocks_the_plot() {
    let (mut app, p0) = desk();
    take(&mut app, p0, 0);

    redeem(&mut app, p0, 0);

    assert_eq!(
        cash_of(&mut app, p0),
        20_000 - 75,
        "750 到手、825 付掉，净差就是那一成手续费"
    );
    assert_eq!(mortgage_of(&app, 0), None, "赎回之后这块地该真的自由了");
    assert_eq!(rent_of(&mut app, 0), 60, "租金跟着回来");
    assert_eq!(
        mortgage_lines(&app),
        vec![
            "Mortgage(P0押L0·750)".to_string(),
            "Mortgage(P0赎L0·825)".to_string()
        ],
        "两笔都要留痕"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 钱不够时**一分不扣、地还押着** —— 与 `Books::debit` 同一条不许半途的纪律。
#[test]
fn a_redeem_you_cannot_afford_changes_nothing() {
    let (mut app, p0) = desk();
    take(&mut app, p0, 0);
    set_cash(&mut app, p0, 800); // 差 25

    redeem(&mut app, p0, 0);

    assert_eq!(cash_of(&mut app, p0), 800, "赎回失败却动了钱");
    assert_eq!(
        mortgage_of(&app, 0),
        Some(750),
        "赎不起就把地解押了 —— 那是白送"
    );
    assert_eq!(mortgage_lines(&app).len(), 1, "多出来那条就是失败的那一笔");
}

// ===== 一块地一次 =====

/// 同一帧里两条押同一块地的请求只成一次。
///
/// 这条不是顺手写的：`Mortgaged` 若是「存在即押」的单位标记、靠 `Commands` 延迟写进去，
/// 第二条请求读到的仍是「没押」，于是同一块地付两遍钱 —— 而两遍都是合法的。
#[test]
fn two_requests_in_one_frame_mortgage_the_plot_once() {
    let (mut app, p0) = desk();
    mortgage_ask(&mut app, p0, MortgageOp::Take, 0);
    mortgage_ask(&mut app, p0, MortgageOp::Take, 0);
    settle(&mut app, 3);

    assert_eq!(cash_of(&mut app, p0), 20_750, "第二笔是白印的 750");
    assert_eq!(mortgage_lines(&app).len(), 1, "同一块地留了两条成交凭据");
}

#[test]
fn one_plot_cannot_be_mortgaged_twice_across_frames() {
    let (mut app, p0) = desk();
    take(&mut app, p0, 0);

    take(&mut app, p0, 0);

    assert_eq!(
        cash_of(&mut app, p0),
        20_750,
        "隔一帧再按一次，第二遍照样不该成"
    );
    assert_eq!(mortgage_of(&app, 0), Some(750), "本金被第二笔改写了");
}

// ===== 窗口：谁办得成、开到什么时候 =====

#[test]
fn somebody_elses_plot_stays_theirs() {
    let (mut app, p0) = desk();
    let p1 = roster(&mut app)[1].0;

    take(&mut app, p1, 0);

    assert_eq!(mortgage_of(&app, 0), None, "别人的柜台替别人押了地");
    assert_eq!(cash_of(&mut app, p1), 20_000, "乙按的按钮给乙发了 750");
    assert_eq!(cash_of(&mut app, p0), 20_000, "甲的地被乙的点击换成了钱");
    assert!(mortgage_lines(&app).is_empty(), "被拒的一笔留了成交凭据");
}

/// 「办完」关掉柜台之后，这一格就不再开门了 —— 与百货柜台同一条形状（`tests/shop.rs`）。
#[test]
fn leaving_the_counter_closes_the_mortgage_window() {
    let (mut app, p0) = desk();
    // 换成非 Hold 策略 = 替玩家按那颗「办完」
    set_policy(&mut app, DecisionPolicy::AcceptAll);
    settle(&mut app, 6);
    assert_eq!(pending_of(&app), None, "前置：「办完」没关掉柜台");

    take(&mut app, p0, 0);

    assert_eq!(cash_of(&mut app, p0), 20_000, "柜台关了还能押");
    assert_eq!(mortgage_of(&app, 0), None);
}

/// 查封中的地块既押不出去也赎不回来：银行不收有纠纷的抵押物，也不替法院解押。
#[test]
fn a_sealed_plot_moves_neither_way() {
    let (mut app, p0) = desk();
    stamp(&mut app, 0, LandStatus::Sealed, 3);

    take(&mut app, p0, 0);
    assert_eq!(mortgage_of(&app, 0), None, "查封中的地押出去了");

    // 押上之后再封，赎回那一支同样挡
    own(&mut app, 1, Some(0), 0, false);
    take(&mut app, p0, 1);
    assert_eq!(mortgage_of(&app, 1), Some(750), "前置：1 号该押得动");
    stamp(&mut app, 1, LandStatus::Sealed, 3);

    redeem(&mut app, p0, 1);
    assert_eq!(
        mortgage_of(&app, 1),
        Some(750),
        "查封中把地赎回来了 —— 付了钱还以为解了押"
    );
}

// ===== 与另外两套账的接缝 =====

/// 押出去对物价指数**中性**：现金进 750、债记 750，那块地还在（可赎回）。
/// 少了负债那一格，每有一个人救急就全场通缩一点，而指数是下一回合每笔租金的乘数。
#[test]
fn mortgaging_is_neutral_to_the_price_index() {
    let (mut app, p0) = desk();
    let before = total_assets(&mut app, 1.0);

    take(&mut app, p0, 0);

    assert_eq!(
        total_assets(&mut app, 1.0),
        before,
        "抵押一次就改了全场资产总和 —— 物价指数会跟着动"
    );

    redeem(&mut app, p0, 0);
    assert_eq!(
        total_assets(&mut app, 1.0),
        before - 75,
        "赎回离开的那 75 就是手续费，它真的付给了银行"
    );
}

/// 押出去会**缩小可贷基数**（那块地不再担保任何东西），但不会让已经借到的钱被抽走。
/// 后半条是刻意写的：银行当场抽贷会把「救急」变成「逼债」，而规则层没写这一条。
#[test]
fn mortgaging_shrinks_the_borrowing_base_but_not_the_debt_already_owed() {
    let (mut app, p0) = desk();
    bank_ask(&mut app, p0, game_core::prelude::BankOp::Borrow, 3_000);
    settle(&mut app, 3);
    assert_eq!(
        bank_of(&mut app, p0).1,
        3_000,
        "前置：1500 × 2.0 该借到 3000"
    );

    take(&mut app, p0, 0);

    assert_eq!(bank_of(&mut app, p0).1, 3_000, "押地当天就被抽贷");
    bank_ask(&mut app, p0, game_core::prelude::BankOp::Borrow, 1);
    settle(&mut app, 3);
    assert_eq!(
        bank_of(&mut app, p0).1,
        3_000,
        "地都押出去了还能继续借 —— 可贷基数没跟着掉"
    );
}

/// 人离场时，押着的地与那笔债一起清 —— 无主地上留着抵押等于替一个不存在的主人挡租金，
/// 而一笔没人还的债留在指数里等于凭空通缩。
///
/// **楼不在一起清的那一半里**（10-22 实报）：摘掉的只有归属与抵押，那块地上 Lv1 的那一栋
/// 照样立着，等下一位走到那一格的人**连楼一起付**（`tests/settle.rs` 钉的是那一半）。
/// 旧版在这里断的是 `(None, 0, false)`，而那条断言正是「先清零、成交再还」那一趟的产物。
#[test]
fn bankruptcy_takes_the_mortgage_off_the_deed() {
    let (mut app, p0) = desk();
    take(&mut app, p0, 0);
    assert_eq!(mortgage_of(&app, 0), Some(750), "前置：0 号押着");

    // 关掉柜台，再把 p0 摆到「踩上别人整条 yellow 组」的那一步
    set_policy(&mut app, DecisionPolicy::AcceptAll);
    settle(&mut app, 6);
    set_policy(&mut app, DecisionPolicy::DeclineAll);
    own(&mut app, 4, Some(1), 0, false);
    own(&mut app, 5, Some(1), 0, false); // 整组在手 → 王府井空地租 160×2 = 320
    set_cash(&mut app, p0, 50);
    make_current(&mut app, p0);
    place(&mut app, p0, 3);
    roll(&mut app, p0);
    settle(&mut app, 8);

    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("Bankrupt(P0")),
        "没走到破产这一步，后面的断言都是空的：{:?}",
        ledger(&app)
    );
    assert_eq!(
        plot_state(&mut app, 0),
        (None, 0, false),
        "人走了，地不该替一个不存在的债主挡租金 —— 押的是空地，留下的也是空地"
    );
    assert_eq!(mortgage_of(&app, 0), None, "抵押本金没跟着清掉");
    // 两人盘上打破产一个就是终局（`game_over_system` 那条），所以这里要的是「停得干净」
    // 而不是「还能继续掷骰」—— 真正要盯的是那一格抵押已经没了
    assert_eq!(phase(&app), TurnPhase::Over, "只剩一人，回合机该停在终局");
}
