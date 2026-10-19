//! P3a 的验收：一局内**可持有、可打出、可被反制**。
//!
//! 对应 PLAN §四 P3 的第一块地基。这一批用例只证明「拿在手里并打出去」这条链是通的；
//! 货架（百货公司）要到 P3d 才有，所以卡一律由 [`deal`] 发 —— 但**发的是同一条公开入口**
//! （[`game_core::prelude::deal_to`]），于是上限与「挤掉最前面那张要留流水」这两条规则
//! 也在被测范围内，而不是只有单元测试覆盖。
//!
//! 出牌一律走 [`play_card`] → 真消息 → `play_card_system`，所以「不是当前回合玩家」
//! 「手里没有这张卡」「目标不合法」「挂着待决事项」四道门都是真走到的，
//! 不是在测试里绕过入口直接改组件。

mod common;

use bevy::prelude::*;
use common::*;
use game_core::prelude::{
    Bankrupt, CardAim, CardId, GameRules, GameSeed, GameSetup, Insulation, Pending,
    PendingDecision, TurnPhase,
};

/// 装配到「P0 的回合、掷骰之前」，返回 app 与两名玩家（按 `PlayerId` 认人：P0、P1）。
fn two_players_before_the_roll() -> (App, Entity, Entity) {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = roster(&mut app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家");
    (app, p0, p1)
}

/// 还在被拘留就返回剩余回合数。
fn turns_left(app: &App, player: Entity) -> Option<u32> {
    detention_of(app, player).map(|d| d.turns_left)
}

#[test]
fn a_card_sits_in_hand_doing_nothing_until_it_is_played() {
    // 「持有」与「生效」分开，是道具卡与事件池的唯一分别，也是这条链的前提：
    // 发到手那一刻不该有任何事发生在对手身上
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);

    assert_eq!(hand_of(&app, p0), vec![frame], "卡该按获得顺序躺在手里");
    assert!(hand_of(&app, p1).is_empty(), "另一只手是空的");
    assert_eq!(turns_left(&app, p1), None, "拿着陷害卡不等于谁坐牢");
    assert!(
        app.world().entity(p1).get::<Insulation>().is_none(),
        "发牌那一刻不该有护身"
    );
}

#[test]
fn playing_a_framing_card_locks_the_target_up_and_empties_the_slot() {
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), Some(5), "卡面写的是关 5 天");
    assert!(hand_of(&app, p0).is_empty(), "打出去就该把占的那格还回来");
    let log = ledger(&app);
    assert!(
        log.contains(&"Play(P0->P1:item_frame)".to_string()),
        "出牌要留流水：{log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("Detain(P1@")),
        "承受者那边也要有流水：{log:?}"
    );
    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "出牌不推进回合（§4.6 红线）"
    );
}

#[test]
fn an_insulated_victim_shrugs_off_the_frame_and_both_sides_keep_their_trail() {
    // PLAN §四 P3 验收的原文形状：A 用陷害打 B，B 有免罪卡则**完全抵消**并各留流水
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");

    // 免罪卡只能由本人在自己的回合打，所以先把轮转挪到 P1 身上
    make_current(&mut app, p1);
    deal(&mut app, p1, insulate.0);
    play_card(&mut app, p1, insulate.0, None);
    settle(&mut app, 2);
    assert_eq!(shield_of(&app, p1), Some(Some(1)), "一张免罪 = 一层护身");
    assert!(hand_of(&app, p1).is_empty(), "打过就没了");

    make_current(&mut app, p0);
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None, "抵消了就没有入狱这回事");
    assert_eq!(shield_of(&app, p1), None, "用掉的那一层要跟着收走");
    let log = ledger(&app);
    assert!(
        log.contains(&"Play(P1->P1:item_insulate)".to_string()),
        "B 那边要看得见自己打过卡：{log:?}"
    );
    assert!(
        log.contains(&"Play(P0->P1:item_frame挡1)".to_string()),
        "A 那边要看得见这次被挡了：{log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("Detain(")),
        "抵消掉的那次不该留下入狱流水：{log:?}"
    );
}

#[test]
fn two_shields_block_two_frames_and_the_third_one_lands() {
    // 一次性消耗品按次计：两张免罪卡 = 抵两次，而不是「这段时间免疫」
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");

    make_current(&mut app, p1);
    for _ in 0..2 {
        deal(&mut app, p1, insulate.0);
        play_card(&mut app, p1, insulate.0, None);
        settle(&mut app, 2);
    }
    assert_eq!(shield_of(&app, p1), Some(Some(2)), "护身是叠层不是刷新");

    make_current(&mut app, p0);
    for _ in 0..2 {
        deal(&mut app, p0, frame.0);
        play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
        settle(&mut app, 2);
        assert_eq!(turns_left(&app, p1), None, "两层还没用完");
    }
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(turns_left(&app, p1), Some(5), "第三张没人挡了");
    assert_eq!(shield_of(&app, p1), None, "两层都用掉了");
}

#[test]
fn the_refusals_are_all_silent_and_change_nothing() {
    // 四种「办不动」：不是当前回合玩家、手里没有这张卡、陷害自己、没带目标。
    // 面板把按钮亮成灰的用的就是同一组判据，所以这里断言的是「一条流水都不留」
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");

    deal(&mut app, p1, frame.0);
    play_card(&mut app, p1, frame.0, Some(CardAim::Player(p0)));
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p0)));
    play_card(&mut app, p0, frame.0, None);
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p0), None);
    assert_eq!(turns_left(&app, p1), None);
    assert_eq!(hand_of(&app, p1), vec![frame], "被拒的出牌不该扣卡");
    assert!(hand_of(&app, p0).is_empty());
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Play(")),
        "被拒的出牌不该留流水：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_card_cannot_be_played_while_a_question_is_open() {
    // 挂着待决事项（买地/升级/收据/卡面）时出牌窗口是关的：那时玩家该先回答那个问题。
    // 与证券下单同一条判据，所以这里用同一个 pending 把它挡下来。
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    app.world_mut()
        .resource_mut::<PendingDecision>()
        .offer(p1, Pending::Card);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None, "窗口关着就不该受理");
    assert_eq!(hand_of(&app, p0), vec![frame], "没受理就不该扣卡");
}

#[test]
fn a_gone_player_cannot_be_framed() {
    // 破产 = 已经离场。给一个不在场的人「关 5 天」既没意义也没法收场，
    // 所以 `aim_is_legal` 读的是 `is_bankrupt` 而不是「实体还在不在」
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    insert(&mut app, p1, Bankrupt);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None);
    assert_eq!(hand_of(&app, p0), vec![frame], "打不出去就不该把卡用掉");
}

#[test]
fn a_full_hand_pushes_out_the_oldest_and_says_which_one() {
    // 上限那条规则在 `Hand::gain` 里，而「挤掉了哪张」在 `CardGained.pushed_out` 里。
    // 这一条同时是 P3d 百货账目的前提：玩家买了卡却没拿到，只能靠流水自证。
    let mut app = boot(
        GameSetup::default(),
        GameRules {
            hand_limit: 2,
            ..GameRules::instant()
        },
        GameSeed::default(),
    );
    let p0 = boot_to_wait_roll(&mut app);
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");

    deal(&mut app, p0, frame.0);
    deal(&mut app, p0, insulate.0);
    assert_eq!(
        hand_of(&app, p0),
        vec![frame, insulate],
        "刚好装满，不该有人出局"
    );

    deal(&mut app, p0, frame.0);
    settle(&mut app, 1);
    assert_eq!(
        hand_of(&app, p0),
        vec![insulate, frame],
        "先进来的先出局：{:?}",
        hand_of(&app, p0)
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l.contains("Gain(P0:item_frame)挤掉item_frame")),
        "被挤掉的那张要写在流水上：{:?}",
        ledger(&app)
    );
}

#[test]
fn item_cards_are_not_a_deck_anybody_can_draw() {
    // 道具卡没有对应的格子（`domain_of` 不产出 `Card(Item)`），来源是货架。
    // 这条钉住「第三副池不是一副可抽的牌」—— 哪天有人手滑给道具卡开一格，
    // 抽卡入口会先在这里红，而不是在游戏里发出不该存在的卡
    let (mut app, _p0, _p1) = two_players_before_the_roll();
    assert_eq!(
        deck_kinds(&mut app),
        vec!["Chance".to_string(), "Fate".to_string()],
        "牌堆只有机会与命运两副"
    );
    assert_eq!(
        hand_owners(&mut app),
        2,
        "每个玩家都该有手牌组件（开局为空）"
    );
}

#[test]
fn a_card_id_outside_the_shelf_is_refused() {
    // 数据与请求之间没有编译期保证（UI 递来的是下标），所以这条越界必须安静拒绝，
    // 而不是 panic —— 对局跑到一半因为一个坏按钮崩掉是不可接受的
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, 9_999, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None);
    assert_eq!(hand_of(&app, p0), vec![CardId(0)]);
}
