//! 卡片格：踩到后从**道具牌堆**随机抽一张进手牌，不立刻结算卡面效果。
//!
//! 与命运格的分工在这条用例里钉死：命运抽的是「当场生效」的事件牌，卡片格抽的是
//! 百货公司卖的那批道具卡 —— 同一批 `CardId`，但入口是 `Hand` 而不是效果管线。

mod common;

use bevy::app::App;
use bevy::prelude::*;
use common::*;
use game_core::prelude::{CardKind, CardSet, DataHandles, Deck, Gameplay, Rng, TileKind};

/// 把 1 号格改成卡片格，并补一副道具牌堆（prototype 盘本身没有卡片格，开局不会发这副牌）。
fn boot_card_tile() -> (App, Entity) {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    retile(&mut app, 1, TileKind::Card);
    let cards = {
        let sets = app.world().resource::<Assets<CardSet>>();
        let handle = &app.world().resource::<DataHandles>().items;
        sets.get(handle).expect("道具牌没加载").clone()
    };
    app.world_mut().spawn((
        Deck::dealt(CardKind::Item, &cards, &mut Rng::new(5)),
        Gameplay,
    ));
    (app, p0)
}

#[test]
fn a_card_tile_puts_one_item_card_in_hand_and_does_not_resolve_it() {
    let (mut app, p0) = boot_card_tile();
    play_turns(&mut app, 1);

    let hand = hand_of(&app, p0);
    assert_eq!(hand.len(), 1, "卡片格只发一张，且是进手牌：{hand:?}");
    let log = ledger(&app);
    assert!(
        log.iter().any(|l| l.starts_with("Gain(P0:item_")),
        "进手牌要留一条 Gain 流水：{log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("Play(")),
        "获得不等于打出，卡片格不该替玩家出牌：{log:?}"
    );
}

#[test]
fn the_item_deck_is_spawned_for_a_board_that_has_card_tiles() {
    let (mut app, _) = boot_card_tile();
    assert_eq!(
        deck_kinds(&mut app),
        vec!["Fate".to_string(), "Item".to_string()],
        "卡片格需要道具牌堆；命运那副照旧在"
    );
}

#[test]
fn both_real_boards_actually_ship_a_card_tile_and_its_deck() {
    for board in ["data/boards/taiwan.json", "data/boards/china.json"] {
        let mut app = boot_on(
            board,
            GameSetup::default(),
            GameRules::instant(),
            GameSeed::default(),
        );
        assert!(run_until_world(&mut app), "{board} 没装配起来");
        assert_eq!(
            deck_kinds(&mut app),
            vec!["Fate".to_string(), "Item".to_string()],
            "{board} 有卡片格却没有道具牌堆"
        );
    }
}
