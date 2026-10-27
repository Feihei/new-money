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
    Bankrupt, CardAim, CardId, GameRules, GameSeed, GameSetup, Pending, PendingDecision, TurnPhase,
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
    detention_of(app, player).and_then(|d| d.term.turns())
}

#[test]
fn a_card_sits_in_hand_doing_nothing_until_it_is_played() {
    // 「持有」与「生效」分开，是道具卡与事件池的唯一分别，也是这条链的前提：
    // 发到手那一刻不该有任何事发生在对手身上
    let (mut app, p0, p1) = two_players_before_the_roll();
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    let insulate = item_index(&app, "item_insulate");
    deal(&mut app, p1, insulate.0);

    assert_eq!(hand_of(&app, p0), vec![frame], "卡该按获得顺序躺在手里");
    assert_eq!(
        hand_of(&app, p1),
        vec![insulate],
        "另一只手只躺着那张被动卡"
    );
    assert_eq!(turns_left(&app, p1), None, "拿着陷害卡不等于谁坐牢");
    assert!(
        gate_asking(&app).is_none() && gate_depth(&app) == 0,
        "P13b②：被动卡不是护身组件 —— 没有事件就没有一问，手里那张只是资格"
    );
    assert_eq!(
        forgive_in_hand(&app, p1),
        1,
        "而那张资格照样数得出来（门问的就是这一位）"
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
fn a_forgive_card_in_hand_blocks_the_frame_and_both_sides_keep_their_trail() {
    // PLAN §四 P3 验收的原文形状：A 用陷害打 B，B 有免罪卡则**完全抵消**并各留流水。
    //
    // P13b② 改的是「有免罪卡」这四个字的含义：它不再需要先打出去挂上一层，
    // **躺在手里就是资格**，而门在花掉那一刻从 `Hand` 里摘走那一张。于是这条用例
    // 多断言两样以前没有的东西：这一问挂在 **B** 身上（不是出牌的 A，PLAN §五 第 2 条
    // 「决策主体能在回合内跳动」），以及答完之后门是空的（没留下第二个没人答的窗口）。
    let (mut app, p0, p1) = two_players_before_the_roll();
    // 这道门要人工答：自动玩家对 `Pending::Passive` 一律答「不用」（与拍卖那格同一条理由），
    // 而这条用例测的就是「用」那一侧
    app.insert_resource(DecisionPolicy::Hold);
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");

    deal(&mut app, p1, insulate.0);
    make_current(&mut app, p0);
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(
        pending_of(&app),
        Some((p1, Pending::Passive)),
        "该被问的是挨打的那位，不是出牌的那位"
    );
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::JailTrip)),
        "门问的是「免罪」那一问，而那一行是坐牢"
    );
    assert_eq!(turns_left(&app, p1), None, "问还没答完，人先不该进去");

    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None, "抵消了就没有入狱这回事");
    assert_eq!(forgive_in_hand(&app, p1), 0, "用掉的那一张要从手里消失");
    assert_eq!(gate_asking(&app), None, "答完的门该是空的");
    let log = ledger(&app);
    assert!(
        log.contains(&"Play(P0->P1:item_frame挡卡)".to_string()),
        "A 那边要看得见这次被挡了：{log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("Used(")),
        "B 那边要看得见自己花掉了一张卡：{log:?}"
    );
    assert!(
        !log.iter().any(|l| l.starts_with("Detain(")),
        "抵消掉的那次不该留下入狱流水：{log:?}"
    );
}

#[test]
fn declining_the_gate_costs_no_card_and_the_frame_lands() {
    // 验收⑤：放弃 = 事件照原样落，且**不消耗卡**（P3d④a ⑤ 那条纪律：卡没了事没成＝缺陷）。
    // 这一条与上面那条是一对 —— 只测「用」的那一侧，测试自己就分不清
    // 「门没花卡」与「门根本没问」这两种世界
    let (mut app, p0, p1) = two_players_before_the_roll();
    app.insert_resource(DecisionPolicy::Hold);
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");

    deal(&mut app, p1, insulate.0);
    make_current(&mut app, p0);
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::JailTrip))
    );

    answer_gate(&mut app, p1, PassiveMove::Decline);
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), Some(5), "放弃之后那一下照原样落下来");
    assert_eq!(forgive_in_hand(&app, p1), 1, "说「不用」不该花掉任何一张卡");
    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "门答完把回合交还给出牌那一格"
    );
}

#[test]
fn two_forgive_cards_are_asked_once_and_only_one_is_spent() {
    // 验收⑨（PLAN §五 第 7 条）：两张同类卡在门上是**同一问**。
    // 按张数循环的实现在这里红 —— 它会弹两次窗，而玩家两次点的是同一句话。
    let (mut app, p0, p1) = two_players_before_the_roll();
    app.insert_resource(DecisionPolicy::Hold);
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");

    deal(&mut app, p1, insulate.0);
    deal(&mut app, p1, insulate.0);
    assert_eq!(forgive_in_hand(&app, p1), 2, "前置：两张都在手里");
    make_current(&mut app, p0);
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    // 一次调用只挂出一问：`gate_depth` 是 1，而那一句问完门就空了
    assert_eq!(gate_depth(&app), 1, "两张卡不该问两遍");
    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None, "第一张挡掉了那一下");
    assert_eq!(forgive_in_hand(&app, p1), 1, "一张抵一次，另一张留着");
    assert_eq!(gate_asking(&app), None, "答完就收，不留第二问");

    // 留着的那一张对**下一次**还有答案 —— 这才是「按次」而不是「按天免疫」
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::JailTrip)),
        "第二张照样要被问"
    );
    answer_gate(&mut app, p1, PassiveMove::Decline);
    settle(&mut app, 2);
    assert_eq!(turns_left(&app, p1), Some(5), "说不用就没人挡得住这一张");
    assert_eq!(forgive_in_hand(&app, p1), 1, "不用那一问，两张都留着");
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

/// P13b 第 6 条：**这一族从此没有「出牌」这个动作**。
///
/// 嫁祸卡躺在手里就是躺在手里 —— 规则层拒掉它，而且**卡不消耗**（P3d④a 那条纪律：
/// 判据分叉的形状从来不是「钱少了」，是「卡没了事没成」）。它带不带目标都同样拒：
/// 「被动卡不占 `CardTarget`」这件事在数据上是 `target` 缺省成 `SelfOnly`，
/// 所以面板若哪天错发一颗带目标的按钮，规则层照样挡得住。
#[test]
fn a_passive_card_cannot_be_played_and_stays_in_hand() {
    let (mut app, p0, p1) = two_players_before_the_roll();
    let shift = item_index(&app, "item_shift");
    deal(&mut app, p0, shift.0);
    let cash = cash_of(&mut app, p0);

    play_card(&mut app, p0, shift.0, Some(CardAim::Player(p1)));
    play_card(&mut app, p0, shift.0, None);
    settle(&mut app, 2);

    assert_eq!(
        hand_of(&app, p0),
        vec![shift],
        "被拒的被动卡一张都没该花掉 —— 事件还没落下来，它连被问都没被问"
    );
    assert_eq!(cash_of(&mut app, p0), cash, "这一趟没有任何事发生");
}
