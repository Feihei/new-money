//! P13b③ 的验收：**钱那侧的门**。
//!
//! ③ 之前这张卡是管线上的一条修正：打出去 → 在登记处挂一条「待用的免付」→ 下一笔过线的
//! 账把它吃掉并注销（`stage_and_spend` 回报「真的花掉了哪几条」）。③ 之后这条链上**没有
//! 「一次」这个位**，也没有「换人付」那一条修正臂 —— 两张卡都成了同一道门的两个问句：
//!
//! ```text
//! 触发点 submit(一笔账) ──→ 门问「免不免、转给谁」──→ 答完了才 Books::settle
//! ```
//!
//! 于是这份文件问的四件事都是**顺序**而不是算术：
//! - **问在钱动之前**（[`the_door_asks_before_the_money_moves`]）：那一问挂着的时候两边
//!   钱包都是原数，收据也还没写 —— 「先扣再退」与「问完才扣」在现金审计上是同一个数，
//!   只有「落下来之前」那一刻能分辨；
//! - **门槛**（[`a_bill_below_the_floor_is_paid_in_full_and_never_asked`] 与 [`the_floor_rides_the_price_index`]）：
//!   不够线的账**连问都不问**，而卡留在手里；
//! - **放弃不消耗**（[`a_declined_question_leaves_the_card_in_hand`]）；
//! - **同类多张只问一次、只烧一张**（PLAN §五 第 7 条 + 验收⑨）。
//!
//! 走的都是真落格路径：`ready_on` + 真掷骰 + 真 `Books::settle`，不是 `charge::stage` 的单测
//! 冒充（PLAN §四 底下那句「别用单元测试通过冒充一局里真的玩到」）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{ChargeKind, LandId};

fn owner(app: &mut App) -> Entity {
    roster(app)[1].0
}

/// 走完 0 号玩家脚下这一格（与 `tests/price_index.rs` 同一句别名，同一件事）。
fn land(app: &mut App) {
    play_turns(app, 1);
}

/// 造一局「P0 即将踩到 P1 的 L0，名义租金 `rent`」，并按 `policy` 摆好自动应答。
///
/// `Hold` 是给「停在那一问上」的用例用的：那一句没答之前脚手架不替他答，于是
/// 「问出去了吗」有 `gate_asking` 这一份活证据，而不是只能从「钱没动」倒推。
fn rent_ahead(rent: i64, policy: DecisionPolicy) -> (App, Entity) {
    let (mut app, payer) = ready_on(0, 1, policy);
    own(&mut app, 0, Some(1), 1, false);
    flat_rent(&mut app, 0, rent);
    (app, payer)
}

/// 造一局「P0 手里有 `n` 张免费卡、马上踩到一笔 `rent` 的租」。
///
/// ③ 之后**没有「打出去」这一步**：卡在手里就是资格（`Insulation` 组件因此删了）。
/// 这里刻意不调 `play_card` —— 那张卡现在根本不是主动卡，打了要被拒。
fn free_cards(rent: i64, n: u8) -> (App, Entity) {
    let (mut app, payer) = rent_ahead(rent, DecisionPolicy::Hold);
    let card = item_index(&app, "item_free").0;
    for _ in 0..n {
        deal(&mut app, payer, card);
    }
    assert_eq!(free_in_hand(&app, payer), n as u32, "前置：卡没发到手里");
    (app, payer)
}

/// 验收③④⑤的共同骨架：那一问挂出来 → 钱没动 → 答「用」→ 一笔 0 元的账落下来。
///
/// 「两边钱包都还是原数」那一格是这一条的全部：**先扣再退**与**问完才扣**在最终余额上
/// 是同一个数，现金审计也抓不住（两边各留一条流水照样对得平），只有落下来之前那一眼分得出。
#[test]
fn the_door_asks_before_the_money_moves() {
    let (mut app, payer) = free_cards(5_000, 1);
    let keeper = owner(&mut app);

    land(&mut app);

    assert_eq!(
        gate_asking(&app),
        Some((payer, Ask::Offset, PassiveEvent::Rent)),
        "过路费那一问要挂在踩格的人身上，而且是抵消那一问"
    );
    assert_eq!(cash_of(&mut app, payer), 20_000, "答之前钱不该已经动过");
    assert_eq!(cash_of(&mut app, keeper), 20_000, "也不该已经收到");
    let log = ledger(&app);
    assert!(
        !log.iter().any(|l| l.starts_with("Rent(")),
        "收据是落地那一刻写的，问出去的那一刻还没有：{log:?}"
    );

    answer_gate(&mut app, payer, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(cash_of(&mut app, payer), 20_000, "免掉了，付款人一分不动");
    assert_eq!(cash_of(&mut app, keeper), 20_000, "业主一分不收");
    assert_eq!(free_in_hand(&app, payer), 0, "那张卡花掉了");
    let log = ledger(&app);
    assert!(
        log.contains(&"Rent(P0->P1@L0$0)".to_string()),
        "免单要看得见「为什么没付钱」：一张 0 元的收据，而不是没有收据：{log:?}"
    );
    assert!(
        log.iter()
            .any(|l| l.starts_with("Used(P0") && l.contains("$5000")),
        "「花掉一张卡」与「这张卡值多少」是同一条流水：{log:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 不够那道门槛：连问都不问，照付，而卡**留在手里**。
///
/// 「不问」与「问了而他说不用」在钱包上是同一个数，所以这里问的是窗口：`gate_asking`
/// 必须是 `None` 而 `gate_spends` 必须是 0（那句「小额账不弹窗」是 PLAN §五 第 5 条的话）。
#[test]
fn a_bill_below_the_floor_is_paid_in_full_and_never_asked() {
    let (mut app, payer) = free_cards(1_500, 1);
    land(&mut app);

    assert_eq!(gate_asking(&app), None, "1500 没超过 2000，不该弹窗");
    assert_eq!(cash_of(&mut app, payer), 18_500, "没到门槛就该照付");
    assert_eq!(free_in_hand(&app, payer), 1, "没生效就不该把卡花掉");
    assert_eq!(gate_spends(&app), 0, "门一张卡都没花");
    assert!(
        ledger(&app).contains(&"Rent(P0->P1@L0$1500)".to_string()),
        "照付的那笔要留租金流水：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 门槛跟着物价水涨（PLAN §三 那句「>2000×物价」）。物价 2.0 时名义 1500 折成 3000，
/// 而门槛也是 4000 —— 不问也不免；名义 2100 折成 4200 才过线。
///
/// 钉死 2000 的话，通胀会把这张卡稀释成一张永久免费券：那是旧 `Correction::floor` 的口径，
/// ③ 之后那句乘法住在门里（[`big_money_line`](game_core::passive::big_money_line)），
/// 而门槛的名义数只有 `GameRules::big_money_floor` 一处。
#[test]
fn the_floor_rides_the_price_index() {
    let (mut app, payer) = free_cards(1_500, 1);
    set_price_index(&mut app, 2.0);
    land(&mut app);
    assert_eq!(gate_asking(&app), None, "3000 没超过 4000 那道线，不该弹窗");
    assert_eq!(cash_of(&mut app, payer), 17_000, "照付物价 2 倍后的 1500");
    assert_eq!(free_in_hand(&app, payer), 1, "卡还在");

    let (mut app, payer) = free_cards(2_100, 1);
    set_price_index(&mut app, 2.0);
    land(&mut app);
    assert_eq!(
        gate_asking(&app),
        Some((payer, Ask::Offset, PassiveEvent::Rent)),
        "4200 过了 4000 的线，就该问"
    );
    ask_the_gate(&mut app, PassiveMove::Use);
    assert_eq!(cash_of(&mut app, payer), 20_000, "免掉的是折过价的那一笔");
}

/// 放弃那一问：账照原样落下，那张卡**分文不动地留在手里**（验收⑤在钱这一侧的形状）。
///
/// 「放弃不消耗」与「没到门槛」在钱包上是同一个数，而这一条摆的是**够得着门槛**的那笔，
/// 所以两句分开说得出：只有「答了不用」这一条路会走到这里。
#[test]
fn a_declined_question_leaves_the_card_in_hand() {
    let (mut app, payer) = free_cards(5_000, 1);
    land(&mut app);
    assert_eq!(
        gate_asking(&app),
        Some((payer, Ask::Offset, PassiveEvent::Rent)),
        "前置：这一问确实弹出去了（不然下面那句是空跑的）"
    );

    answer_gate(&mut app, payer, PassiveMove::Decline);
    settle(&mut app, 2);

    assert_eq!(cash_of(&mut app, payer), 15_000, "他说不用，那就照付");
    assert_eq!(free_in_hand(&app, payer), 1, "放弃不该消耗那张卡");
    assert_eq!(gate_spends(&app), 0, "一张卡都没花");
    assert_cash_audit(&mut app, 20_000);
}

/// 两张同类卡在门上是**同一问**（PLAN §五 第 7 条 + 验收⑨），而一笔账只烧掉一张。
///
/// 「只问一次」的证据是队列深度与答完之后的问句清单：`ask_the_gate` 会把挂着的问题一路
/// 答到底，返回它答过的每一句 —— 两张卡答出两句就是 2，这里必须是 1。
/// 「只烧一张」的证据是剩下的那张：它还在那位手里，下一笔过线的账照样问得出。
#[test]
fn two_identical_cards_are_one_question_and_burn_one() {
    let (mut app, payer) = free_cards(5_000, 2);
    land(&mut app);

    assert_eq!(
        gate_depth(&app),
        1,
        "两件事挤在一条队列里 = 有人把同一问数成了两遍"
    );
    let asked = ask_the_gate(&mut app, PassiveMove::Use);
    assert_eq!(
        asked,
        vec![(payer, Ask::Offset, PassiveEvent::Rent)],
        "两张卡问出两句，就是验收⑨那条「同类多张只问一次」的反面"
    );
    assert_eq!(free_in_hand(&app, payer), 1, "一笔账只该烧掉一张");
    assert_eq!(gate_spends(&app), 1, "流水里也该只有一条「花掉一张卡」");
    assert_eq!(cash_of(&mut app, payer), 20_000, "免掉了，一分不动");
    assert_cash_audit(&mut app, 20_000);
}

/// 报价问一百遍也不烧卡 —— P3a 给免罪卡立的那条纪律，在钱这一侧的对应物。
///
/// ③ 之后这条纪律有两个读点，都要问：管线那句 [`Books::staged`] 是纯的（旧
/// `stage_and_spend` 的返回类型把「花掉了哪几条」写进签名，那个类型删了），
/// **而门自己也要是纯的** —— 它每帧都要重读一次「这一笔过不过那道线」，
/// 读一次少一张卡的话，面板一刷就把玩家的那张券刷没了。
#[test]
fn quoting_the_price_never_spends_the_card() {
    let (mut app, payer) = free_cards(2_400, 1);
    land(&mut app);
    assert_eq!(
        gate_asking(&app),
        Some((payer, Ask::Offset, PassiveEvent::Rent)),
        "前置：先让门把那一问挂出来，下面那三次报价才是「等回答期间的报价」"
    );

    for _ in 0..3 {
        run_op(&mut app, move |books, _| {
            assert_eq!(
                books.quoted(ChargeKind::Rent { land: LandId(0) }, payer, 2_400),
                2_400,
                "报出来的价不该替玩家做决定 —— 免不免要他答"
            );
        });
        assert_eq!(
            gate_asking(&app),
            Some((payer, Ask::Offset, PassiveEvent::Rent))
        );
    }

    assert_eq!(free_in_hand(&app, payer), 1, "问了三次价，卡还在手里");
    assert_eq!(gate_spends(&app), 0, "一次都没花");
}
