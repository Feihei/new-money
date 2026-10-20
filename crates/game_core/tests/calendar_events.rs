//! 日历事件（P2b）：圣诞礼物与春节红包。
//!
//! 这两件事的验收都落在「日子到了没有、谁收到、钱与卡有没有留下凭据」四问上。
//! 日历本身（公历换算、节假日表）的单测在 `calendar.rs`，这里只跑经过 ECS 接线的那一遍。

mod common;

use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use common::*;
use game_core::prelude::{
    Bankrupt, CardId, Date, GameRules, GameSeed, GameSession, GameSetup, Hand, Rng, TurnPhase,
    deliver_calendar_events_system,
};

fn boot_at(epoch: Date, seed: GameSeed) -> App {
    let mut app = boot(GameSetup::default(), GameRules::instant(), seed);
    pin_epoch(&mut app, epoch);
    app
}

fn hands(app: &mut App) -> Vec<(Entity, Vec<CardId>)> {
    roster(app)
        .into_iter()
        .map(|(e, ..)| (e, hand_of(app, e)))
        .collect()
}

fn events(app: &App, prefix: &str) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|line| line.starts_with(prefix))
        .collect()
}

#[test]
fn christmas_gives_every_present_player_one_item_card() {
    let mut app = boot_at(Date::new(2048, 12, 25), GameSeed::default());
    boot_to_wait_roll(&mut app);

    let dealt = hands(&mut app);
    assert_eq!(dealt.len(), 2);
    for (player, cards) in &dealt {
        assert_eq!(cards.len(), 1, "{player:?} 该收到一张礼物，实际 {cards:?}");
    }
    assert_eq!(events(&app, "Gifts("), vec!["Gifts(2048年,2张)"]);
    assert_eq!(
        app.world().resource::<GameSession>().last_gift_year,
        Some(2048),
        "发过的年份要留位，读档回到同一天才不会再发一轮"
    );
    // 礼物不碰现金也不碰点券：卡是从整副道具池里来的
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn a_bankrupt_player_gets_no_gift() {
    let mut app = boot_at(Date::new(2048, 12, 25), GameSeed::default());
    // 装配完成、`TurnStart` 还没转移的那一帧把乙判破产（他不再「在场」）
    assert!(run_until_world(&mut app));
    let p1 = roster(&mut app)[1].0;
    insert(&mut app, p1, Bankrupt);
    assert!(run_until_phase(&mut app, TurnPhase::WaitRoll, 30));

    assert_eq!(hand_of(&app, p1).len(), 0, "破产者不该收到礼物");
    assert_eq!(events(&app, "Gifts("), vec!["Gifts(2048年,1张)"]);
}

#[test]
fn a_full_hand_pushes_out_the_oldest_when_the_gift_arrives() {
    let mut app = boot_at(Date::new(2048, 12, 25), GameSeed::default());
    assert!(run_until_world(&mut app));
    let p0 = roster(&mut app)[0].0;
    // 先把手牌塞满（0..15），礼物进来时挤掉的必须是最前面那张 0 ——
    // 这条边界不为圣诞另写一套，复用 `Hand::gain` 的 `pushed_out`
    let full: Vec<CardId> = (0..15).map(CardId).collect();
    app.world_mut()
        .entity_mut(p0)
        .insert(Hand { cards: full.into() });
    assert!(run_until_phase(&mut app, TurnPhase::WaitRoll, 30));

    let hand = hand_of(&app, p0);
    assert_eq!(hand.len(), 15, "手牌上限不该被礼物撑破");
    assert_eq!(hand.first(), Some(&CardId(1)), "最前面的 0 号该被挤掉");
}

#[test]
fn the_same_gift_year_is_never_delivered_twice() {
    let mut app = boot_at(Date::new(2048, 12, 25), GameSeed::default());
    boot_to_wait_roll(&mut app);
    assert_eq!(hands(&mut app)[0].1.len(), 1);

    // 同一天再跑一次（模拟读档回到 12/25）：年份这一位挡住第二次发放
    app.world_mut()
        .run_system_once(deliver_calendar_events_system)
        .expect("送礼 system 应能独立跑一次");
    assert_eq!(hands(&mut app)[0].1.len(), 1, "同一年发了第二轮");
    assert_eq!(events(&app, "Gifts(").len(), 1);

    // 把「今年发过了」抹掉再跑，才会真发 —— 证明挡住的正是这一位，不是别的巧合
    app.world_mut().resource_mut::<GameSession>().last_gift_year = None;
    app.world_mut()
        .run_system_once(deliver_calendar_events_system)
        .expect("送礼 system 应能独立跑一次");
    assert_eq!(hands(&mut app)[0].1.len(), 2);
}

/// 四位数字红包的金额由 `pick_envelope` 子流按玩家序逐人抽；`0000` 是合法的空红包。
/// 这里用同一个种子独立算一遍，钉住「金额怎么来的」而不是钉某个具体数字。
#[test]
fn envelopes_use_four_digits_per_player_in_turn_order() {
    let seed = GameSeed(0xE2E2);
    let mut app = boot_at(Date::new(2048, 2, 14), seed);
    boot_to_wait_roll(&mut app);

    let mut rng = Rng::new(seed.0);
    let want: Vec<i64> = (0..2)
        .map(|_| (0..4).fold(0i64, |acc, _| acc * 10 + rng.pick_envelope_digit() as i64))
        .collect();
    let lines = events(&app, "Envelope(");
    assert_eq!(
        lines,
        vec![
            format!("Envelope(P0${})", want[0]),
            format!("Envelope(P1${})", want[1])
        ],
        "红包金额应逐人按四次数字抽"
    );
    for ((player, ..), amount) in roster(&mut app).into_iter().zip(&want) {
        assert_eq!(
            cash_of(&mut app, player),
            20_000 + amount,
            "金额 > 0 时要进现金（{player:?}）"
        );
    }
    assert_cash_audit(&mut app, 20_000);
}

/// 0 元红包：凭据照发、老虎机照播，但**现金流水一条都不许有**。
/// 找一个首抽就是 0000 的种子来钉这一格（概率 1/10000，搜索很便宜）。
#[test]
fn an_empty_envelope_leaves_no_cash_trail() {
    let seed = (0..300_000u64)
        .find(|seed| {
            let mut rng = Rng::new(*seed);
            (0..4).fold(0i64, |acc, _| acc * 10 + rng.pick_envelope_digit() as i64) == 0
        })
        .expect("30 万个种子里总有一个首抽 0000");
    let mut app = boot_at(Date::new(2048, 2, 14), GameSeed(seed));
    boot_to_wait_roll(&mut app);

    let p0 = roster(&mut app)[0].0;
    assert_eq!(events(&app, "Envelope(").first().unwrap(), "Envelope(P0$0)");
    assert_eq!(cash_of(&mut app, p0), 20_000, "空红包不该动现金");
    let cash_lines = app
        .world()
        .resource::<common::CashAudit>()
        .0
        .iter()
        .filter(|(player, ..)| *player == p0)
        .count();
    assert_eq!(cash_lines, 0, "0 元红包在现金审计里留下了一条假账");
}

#[test]
fn a_bankrupt_player_gets_no_envelope() {
    let mut app = boot_at(Date::new(2048, 2, 14), GameSeed(0xE2E2));
    assert!(run_until_world(&mut app));
    let p1 = roster(&mut app)[1].0;
    insert(&mut app, p1, Bankrupt);
    assert!(run_until_phase(&mut app, TurnPhase::WaitRoll, 30));

    let lines = events(&app, "Envelope(");
    assert_eq!(lines.len(), 1, "破产者不该有红包凭据：{lines:?}");
    assert!(lines[0].starts_with("Envelope(P0$"));
}

/// 表外年份（2053 起）的显式降级：农历节不放假，红包也不发。
#[test]
fn no_envelope_outside_the_holiday_table() {
    let mut app = boot_at(Date::new(2053, 2, 1), GameSeed::default());
    boot_to_wait_roll(&mut app);
    assert!(
        events(&app, "Envelope(").is_empty(),
        "2053 年春节不在表里，红包不该发"
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn the_same_seed_pays_the_same_envelopes() {
    let run = || {
        let mut app = boot_at(Date::new(2048, 2, 14), GameSeed(0xBEEF));
        boot_to_wait_roll(&mut app);
        events(&app, "Envelope(")
    };
    assert_eq!(run(), run(), "同种子两次跑出不同红包金额");
}
