//! P3c② 的验收：免费卡是一张**存着等那笔大的**的卡。
//!
//! 四条各钉住一个形状：打出去那一刻什么都不发生（只是登记）、没到门槛不烧卡、
//! 到了门槛整笔免掉且登记处那条就此注销、报价问一百遍也不烧卡。
//! 全部走真出牌入口与真落格结算 —— 「登记 → 过链 → 注销」这条接线
//! 是被真的跑过的，不是 `charge.rs` 的单测冒充的（PLAN §四 底下那句「别用单元测试通过
//! 冒充一局里真的玩到」）。
//!
//! 门槛本身跟着物价走（PLAN §三 那句「>2000×物价」），所以这里也有一条把乘数钉住的用例：
//! 钉死 2000 的话，物价 2 倍时这张卡连一格空地都免得掉，就成了永久免费券。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{ChargeKind, LandId};

fn other(app: &mut App) -> Entity {
    roster(app)[1].0
}

/// 造一局「P0 即将踩到 P1 的 L0，名义租金 `rent`」。
fn rent_ahead(rent: i64) -> (App, Entity) {
    let (mut app, payer) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 0, Some(1), 1, false);
    flat_rent(&mut app, 0, rent);
    (app, payer)
}

/// 把免费卡发到出牌人手里并打出去。返回 (app, 出牌人)。
fn play_free_card(rent: i64) -> (App, Entity) {
    let (mut app, payer) = rent_ahead(rent);
    let free = item_index(&app, "item_free");
    deal(&mut app, payer, free.0);
    assert_eq!(corrections_len(&app), 0, "发牌那一刻不该登记任何东西");
    play_card(&mut app, payer, free.0, None);
    settle(&mut app, 2);
    (app, payer)
}

#[test]
fn playing_it_arms_a_correction_and_moves_no_money() {
    let (mut app, payer) = play_free_card(5_000);

    assert!(
        hand_of(&app, payer).is_empty(),
        "打出去就该把占的那格还回来"
    );
    assert_eq!(corrections_len(&app), 1, "登记处该挂着那条待用的免付");
    assert_eq!(cash_of(&mut app, payer), 20_000, "打出去这一刻什么都不发生");
    let log = ledger(&app);
    assert!(
        log.contains(&"Play(P0->P0:item_free)".to_string()),
        "出牌要留流水：{log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("Waive")),
        "还没踩到地，免付不该已经生效：{log:?}"
    );
}

/// 门槛是这张卡的全部玩法：没有它，这是一张「以后所有账一律免」的券，
/// 那不该是 45 点券的东西。没到门槛那一笔**照付**，而卡**留在登记处**。
#[test]
fn a_bill_below_the_floor_is_paid_in_full_and_the_card_survives() {
    let (mut app, payer) = play_free_card(1_500);
    play_turns(&mut app, 1);

    assert_eq!(cash_of(&mut app, payer), 18_500, "没到门槛就该照付 1500");
    assert_eq!(corrections_len(&app), 1, "没生效就不该把卡花掉");
    let log = ledger(&app);
    assert!(
        log.contains(&"Rent(P0->P1@L0$1500)".to_string()),
        "照付的那笔要留租金流水：{log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("Waive")),
        "没免掉就不该有免付凭据：{log:?}"
    );
}

/// 到了门槛：整笔免掉、两边都不动钱、登记处那条就此注销。
/// 「免付」不是「先付 2400 再退 2400」—— 那样现金审计会多出两笔对不上的搬运。
#[test]
fn a_bill_over_the_floor_is_waived_and_the_card_is_gone() {
    let (mut app, payer) = play_free_card(2_400);
    let owner = other(&mut app);
    play_turns(&mut app, 1);

    let log = ledger(&app);
    assert!(
        log.contains(&"Waive(P0$2400·Card)".to_string()),
        "免掉多少要看得见（这里没有钱动，只有这条能证明卡花掉了）：{log:?}"
    );
    assert_eq!(cash_of(&mut app, payer), 20_000, "付款人一分不动");
    assert_eq!(cash_of(&mut app, owner), 20_000, "业主一分不收");
    assert_eq!(corrections_len(&app), 0, "一次性：生效过就该注销");
}

/// 报价不许花卡 —— P3a 给免罪卡立的同一条纪律，这是钱那一侧的对应物。
/// 走真 `Books::quoted`：UI 每刷一次面板就问一次价，问一百遍也不该烧掉一张卡。
/// （`charge.rs` 有一条同内容的单测，那条证的是签名，这条证的是接线。）
#[test]
fn quoting_the_price_never_spends_the_card() {
    let (mut app, payer) = play_free_card(2_400);
    for _ in 0..3 {
        run_op(&mut app, move |books, _| {
            assert_eq!(
                books.quoted(ChargeKind::Rent { land: LandId(0) }, payer, 2_400),
                0,
                "报出来的价确实是被免掉的那个价"
            );
        });
    }
    assert_eq!(corrections_len(&app), 1, "报了三次价，卡还在登记处");
}

/// 门槛跟着物价水涨（PLAN §三 那句「>2000×物价」）。物价 2.0 时名义 1500 折成 3000，
/// 而门槛也是 4000 —— 不免。名义 2100 折成 4200 才过线。
#[test]
fn the_floor_rides_the_price_index() {
    let (mut app, payer) = play_free_card(1_500);
    set_price_index(&mut app, 2.0);
    play_turns(&mut app, 1);
    assert_eq!(
        cash_of(&mut app, payer),
        17_000,
        "3000 没超过 4000 的门槛，照付（物价 2 倍后的 1500）"
    );
    assert_eq!(corrections_len(&app), 1, "卡还在");

    let (mut app, payer) = play_free_card(2_100);
    set_price_index(&mut app, 2.0);
    play_turns(&mut app, 1);
    assert_eq!(cash_of(&mut app, payer), 20_000, "4200 过了 4000 的门槛");
    assert_eq!(corrections_len(&app), 0, "过了线就烧掉");
}
