//! P6d 的验收：两张神明符。
//!
//! **请神符**卖的是「不必走到那一格」，所以它的第一条用例不是「能不能上身」（那条 P6a 测过），
//! 而是**它请的是哪一位**：环上双向最短，而不是「他前面那位」。后面那位才是这张卡值钱的地方
//! —— 前面那位他下一回合本来就会踩到，为一个本来就会发生的事花 50 点券不叫玩法。
//!
//! **送神符**的主语是「（我身上）那位神」，所以它的用例全在**边界**上：身上没有神、
//! 那位神送不走（死神）、炸弹单独在而神不在、想拿去送别人的神。四条都该**卡不消耗**。
//! 「连带送走身上那颗炸弹」是这张卡的第二件事，它有自己的一条用例与一条反例。
//!
//! 两张符都不推进回合、都不花钱（钱是买它那次花的，走百货那条账），所以每条用例都顺手
//! 断一句「还在掷骰之前」—— 出牌是玩家在自己回合里的自由动作（P3a 那条不改）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::Gameplay;
use game_core::prelude::{CardAim, God, Occupancy, OccupancyAt, OccupancyKind, TurnPhase};

/// 装配到「P0 的回合、掷骰之前」。P0 站在 0 号格（起点），环长 13。
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

/// 把一张符发到手里、按下去、跑两帧。
fn play(app: &mut App, player: Entity, id: &str, aim: Option<CardAim>) {
    let card = item_index(app, id);
    deal(app, player, card.0);
    play_card(app, player, card.0, aim);
    settle(app, 2);
}

fn summon(app: &mut App, player: Entity) {
    play(app, player, "item_summon", None);
}

fn banish(app: &mut App, player: Entity) {
    play(app, player, "item_banish", None);
}

/// 清空路上与身上的神，再按 `(神, 格)` 摆好 —— 用例要的是「哪一格站着哪一位」，
/// 而装配会随机刷两位、补刷每回合再加一位。
fn board_gods(app: &mut App, rows: &[(God, u32)]) {
    clear_gods(app);
    for (god, tile) in rows {
        spawn_god(app, *god, *tile);
    }
}

/// 往某人身上塞一颗**已经带着**的定时炸弹。真实路径是踩到它（`walk.rs`），而那一掷
/// 由骰子说了算 —— 夹具只写「他带着」这一件事，不推进回合。
fn hand_a_bomb(app: &mut App, player: Entity) {
    app.world_mut().spawn((
        Occupancy {
            kind: OccupancyKind::TimeBomb,
            at: OccupancyAt::Carried {
                by: player,
                steps_left: 30,
            },
        },
        Gameplay,
    ));
}

fn held_god(app: &App, player: Entity) -> Option<God> {
    possession_of(app, player).map(|held| held.god)
}

/// 那张符还在手里吗（办不成时它必须**没被消耗**）。
fn holds(app: &App, player: Entity, id: &str) -> bool {
    let card = item_index(app, id);
    hand_of(app, player).contains(&card)
}

// ===== 请神符 =====

#[test]
fn the_summon_buys_the_god_behind_him_not_the_one_he_was_about_to_step_on() {
    // 从 0 号格看：2 号那位离 2 步、在他**前面**；环尾 18 号那位离 1 步、在他**后面**。
    // 请的是后面那位 —— 前面那位他下一回合本来就会踩到，那不必花一张卡
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[(God::Angel, 2), (God::Devil, 18)]);

    summon(&mut app, p0);

    assert_eq!(
        held_god(&app, p0),
        Some(God::Devil),
        "「离自己最近」读的是双向最短，不是行进方向"
    );
    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::God(God::Devil)),
        Vec::<u32>::new(),
        "请过来之后路上不该还站着一位恶魔"
    );
    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::God(God::Angel)),
        vec![2],
        "没被请的那位照站在原地"
    );
    assert_eq!(tile_of(&app, p0), 0, "请神不走路：他还站在起点");
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "出牌不吃掉他的掷骰");
    assert!(
        !holds(&app, p0, "item_summon"),
        "办成了这张卡就该少一张 —— 下面那批「卡留在手里」的用例全靠这一句才有牙"
    );
}

#[test]
fn a_summoned_god_does_the_thing_he_does_when_you_walk_into_him() {
    // 「请来的人与踩到的人是同一个人」：那一次性的一笔照做、那条链照挂。
    // 少做一半的话，这张卡就成了「打折的大财神」，而 50 点券那个价就说不通了
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[(God::BigRich, 12)]);
    let before = cash_of(&mut app, p0);

    summon(&mut app, p0);

    let gift = cash_of(&mut app, p0) - before;
    assert!(
        (1000..=9999).contains(&gift),
        "大财神那笔是四位数，实测 {gift}（{before} → {}）",
        cash_of(&mut app, p0)
    );
    assert_eq!(
        corrections_len(&app),
        1,
        "他那条「过路费不用付」的链也要一起挂上"
    );
    assert!(
        ledger(&app).iter().any(|line| line.starts_with("GodDeed")),
        "当场那笔要有凭据，顶栏与老虎机读的就是它"
    );
}

#[test]
fn summoning_replaces_the_god_already_on_him_and_keeps_only_the_new_chain() {
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[(God::BigFortune, 12)]);
    attach_god(&mut app, p0, God::SmallRich, None);
    assert_eq!(corrections_len(&app), 1, "前置：小财神那条减半链挂着");

    summon(&mut app, p0);

    assert_eq!(held_god(&app, p0), Some(God::BigFortune));
    assert_eq!(
        corrections_len(&app),
        1,
        "旧神那条得跟着旧神走 —— 剩的这一条是大福神的"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("GodOff") && line.contains("顶走")),
        "顶走要留一句怎么走的：{:?}",
        ledger(&app)
    );
}

#[test]
fn summoning_moves_him_off_the_road_but_not_out_of_play() {
    // 请神**不是消灭一位神**：他从路上挪到人身上，而「同一路不挤两位」那条对位规则
    // 读的是在场（路上 + 身上，`Presence::observe` 那两半）。所以小财神照样出不来，
    // 而 12 号那一格的槽位确实空了（ADR-002 一格一件）
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[(God::BigRich, 12)]);

    summon(&mut app, p0);

    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::God(God::BigRich)),
        Vec::<u32>::new(),
        "路上不该还剩着一位大财神"
    );
    assert!(
        !god_pool(&mut app).contains(&God::SmallRich),
        "他改站在人身上，对位那条挡照样成立：{:?}",
        gods_in_play(&mut app)
    );
    assert_eq!(gods_in_play(&mut app), vec![God::BigRich]);
}

#[test]
fn an_empty_road_leaves_the_charm_in_his_hand() {
    // 路上没有一位神时这张卡**办不成**，而办不成的卡不该被吃掉 ——
    // 判据分叉的形状从来不是钱少了，是「卡没了事没成」
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);

    summon(&mut app, p0);

    assert!(holds(&app, p0, "item_summon"), "卡要留在手里");
    assert_eq!(held_god(&app, p0), None, "没有对象就不该有附身");
    assert!(
        !ledger(&app)
            .iter()
            .any(|line| line.starts_with("Play(") && line.contains("item_summon")),
        "被拒的那一次不该有出牌流水：{:?}",
        ledger(&app)
    );
}

// ===== 送神符 =====

#[test]
fn banishing_takes_the_god_and_every_chain_he_brought() {
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);
    attach_god(&mut app, p0, God::BigDoom, None);
    assert_eq!(corrections_len(&app), 2, "前置：大衰神那两条挂着");

    banish(&mut app, p0);

    assert_eq!(held_god(&app, p0), None);
    assert_eq!(
        corrections_len(&app),
        0,
        "神走了而链还在，就是「七天」变成了「终身」"
    );
    let left = ledger(&app)
        .into_iter()
        .find(|line| line.starts_with("GodOff"))
        .expect("离场要有凭据");
    assert!(
        left.contains("送走"),
        "那句要说的是「送走」而不是「期满」：{left}"
    );
    assert!(
        !holds(&app, p0, "item_banish"),
        "办成了就该消耗 —— 下面那三条「卡留在手里」的反例全靠这一句才不是空跑"
    );
}

#[test]
fn the_banish_charm_also_sends_the_bomb_along() {
    // PLAN §三 神明行那句「送神符连带送走身上定时炸弹」
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);
    attach_god(&mut app, p0, God::Devil, None);
    hand_a_bomb(&mut app, p0);
    assert_eq!(bombs(&mut app).len(), 1, "前置：他真带着一颗");

    banish(&mut app, p0);

    assert_eq!(bombs(&mut app).len(), 0, "炸弹跟着神一起走");
    assert_eq!(held_god(&app, p0), None);
}

#[test]
fn a_bomb_without_a_god_is_not_a_reason_to_play_it() {
    // 这张卡的主语是神。允许「没神也能送炸弹」会开出一条隐藏的正确用法，
    // 而那件事从卡面上读不出来 —— 玩家该从文案就知道它落空
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);
    hand_a_bomb(&mut app, p0);

    banish(&mut app, p0);

    assert_eq!(bombs(&mut app).len(), 1, "神不在，炸弹也不该被顺手带走");
    assert!(holds(&app, p0, "item_banish"), "落空的符不该被消耗");
}

#[test]
fn death_refuses_this_charm_while_the_other_eleven_take_it() {
    // PLAN §三「送神符对死神无效」。他今天还不装配（上不了街、也没有召唤路径），
    // 所以这条只能从「硬挂上他」这一侧测 —— 而判据本来就该在表里，不在调用点旁
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);
    attach_god(&mut app, p0, God::Death, None);

    banish(&mut app, p0);

    assert_eq!(
        held_god(&app, p0),
        Some(God::Death),
        "死神不吃这一张：他缠上来就是十三天"
    );
    assert!(holds(&app, p0, "item_banish"), "送不走就不该把卡扣掉");
}

#[test]
fn the_charm_cannot_be_aimed_at_another_players_god() {
    // 「只能送自己的」这条决定在请求那一层的形状：这张卡没有 aim，
    // 所以带目标的请求是**错请求**（`aim_is_legal` 的 `SelfOnly × Some` 那一格挡掉）
    let (mut app, p0, p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);
    attach_god(&mut app, p1, God::BigPoor, None);

    banish_at(&mut app, p0, p1);

    assert_eq!(
        held_god(&app, p1),
        Some(God::BigPoor),
        "别人身上的神不是这张卡能单方面送走的东西"
    );
    assert_eq!(held_god(&app, p0), None, "他自己的身上本来也没神");
    assert!(holds(&app, p0, "item_banish"));
}

/// 带着一个目标按下送神符（面板不会这么发，这条测的是规则层认不认）。
fn banish_at(app: &mut App, player: Entity, target: Entity) {
    let card = item_index(app, "item_banish");
    deal(app, player, card.0);
    play_card(app, player, card.0, Some(CardAim::Player(target)));
    settle(app, 2);
}

#[test]
fn banishing_with_nothing_on_him_leaves_the_charm_in_his_hand() {
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[]);

    banish(&mut app, p0);

    assert!(holds(&app, p0, "item_banish"), "身上没有神，这张卡落空");
    assert_eq!(held_god(&app, p0), None);
}

#[test]
fn the_two_charms_never_touch_the_cash_or_the_points() {
    // 出牌本身不花钱（钱是**买**它那次花的，走百货那条点券账）。
    // 用天使与恶魔这两位：他们既不挂链也不当场给钱，所以差值必须是零
    let (mut app, p0, _p1) = two_players_before_the_roll();
    board_gods(&mut app, &[(God::Angel, 12)]);
    let (cash, points) = (cash_of(&mut app, p0), points_of(&app, p0));

    summon(&mut app, p0);
    assert_eq!((cash_of(&mut app, p0), points_of(&app, p0)), (cash, points));

    board_gods(&mut app, &[]);
    banish(&mut app, p0);
    assert_eq!(
        (cash_of(&mut app, p0), points_of(&app, p0)),
        (cash, points),
        "送神同样是一分钱都不动的一件事"
    );
}
