//! P4③ 的验收：保释**不动现金**，是别的玩家走到拘留场所的接入点时花点券放人。
//!
//! 这条规则在演示盘上的入口是 2 号（汉口路，挂监狱分支）与 7 号（徐家汇，挂医院分支）：
//! 踩上去先照常结算地产，然后如果那条分支里真关着人，才把保释窗口摆出来。所以本文件
//! 的每个用例都是「真走到那一格 → 真答地产报价 → 看窗口」这条路径，而不是直接挂 Pending。
//!
//! 四件事各有一条钉子：
//! - 点券扣 30、现金分文不动（两套账；与 P4② 的收租权那条互不牵连）；
//! - 人从死胡同里被放出来之后，下一回合照常在接入点掷骰；
//! - 监狱门口放不出医院里的人；
//! - 同一帧两条请求只收一次钱（`Commands` 的移除是延迟的，这条坑 P3d③ 付过学费）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::Messages;
use common::*;
use game_core::prelude::{
    Decision, DecisionChosen, Detention, DetentionPlace, GameRules, GameSetup, PLAYER_COLORS,
    Pending, PlayerSetup, TurnPhase,
};

/// prototype 的两条分支接入点。
const JAIL_JUNCTION: u32 = 2;
const HOSPITAL_JUNCTION: u32 = 7;

fn other(app: &mut App, player: Entity) -> Entity {
    roster(app)
        .iter()
        .find(|(e, ..)| *e != player)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家")
}

fn answer(app: &mut App, player: Entity, action: Decision) {
    app.world_mut()
        .resource_mut::<Messages<DecisionChosen>>()
        .write(DecisionChosen { player, action });
    settle(app, 3);
}

/// 把 P0 送到 `junction` 上（从前一格掷一步），并让窗口前的报价停住。
/// `P0` 的点券先调到 100 —— 保释的价目与「不够的时候会怎样」各有用例。
fn walk_onto(app: &mut App, p0: Entity) -> Entity {
    assert_eq!(current(app), p0, "该轮到要走路的那位");
    roll(app, p0);
    let mut at_decision = false;
    for _ in 0..80 {
        app.update();
        if phase(app) == TurnPhase::WaitDecision {
            at_decision = true;
            break;
        }
    }
    assert!(at_decision, "踩到分支接入点之后该停下来等报价");
    p0
}

#[test]
fn bailing_someone_costs_points_and_never_cash() {
    let (mut app, p0) = ready_on(JAIL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 5,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    set_points(&mut app, p0, 100);

    let p0 = walk_onto(&mut app, p0);
    assert!(
        matches!(pending_of(&app), Some((_, Pending::Buy(_)))),
        "第 2 格是地产，先照常报价：{:?}",
        pending_of(&app)
    );
    answer(&mut app, p0, Decision::Decline);

    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Bail(DetentionPlace::Jail))),
        "地产报价答完，监狱门口才长出保释窗口"
    );

    bail_ask(&mut app, p0, p1);
    settle(&mut app, 3);

    assert_eq!(points_of(&app, p0), 70, "30 点券从保释者身上扣");
    assert_eq!(cash_of(&mut app, p0), 20_000, "保释不动现金");
    assert_eq!(cash_of(&mut app, p1), 20_000, "被保的人也不动现金");
    assert_eq!(detention_of(&app, p1), None, "人已经出来了");
    assert_eq!(tile_of(&app, p1), JAIL_JUNCTION, "逻辑位置一直是接入点");
    assert!(
        ledger(&app).contains(&"Bail(P1←P0@监狱,30点券)".to_string()),
        "流水要说清谁替谁付了多少：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, 20_000);
    assert_points_audit(&mut app, 0);
    // 窗口留着：同一处关着好几个人时可以一个个放
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Bail(DetentionPlace::Jail))),
        "放走一个人不该顺手关掉窗口"
    );
}

#[test]
fn the_released_player_rolls_normally_on_his_next_turn() {
    let (mut app, p0) = ready_on(JAIL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 5,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    set_points(&mut app, p0, 100);
    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);
    bail_ask(&mut app, p0, p1);
    settle(&mut app, 3);
    // 关掉窗口，让回合正常收尾
    answer(&mut app, p0, Decision::Acknowledge);
    assert_eq!(pending_of(&app), None, "「离开」之后窗口该收掉");

    // 跑到他自己的下一回合：没有被拘留跳过，直接进入 WaitRoll
    for _ in 0..200 {
        app.update();
        if current(&app) == p1 && phase(&app) == TurnPhase::WaitRoll {
            break;
        }
    }
    assert_eq!(current(&app), p1);
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "他该正常等掷骰");
    assert_eq!(tile_of(&app, p1), JAIL_JUNCTION, "从接入点继续走");
    assert!(
        !events(&app).iter().any(|e| e.starts_with("DetainSkip(P1")),
        "保释之后不该再有一次「跳过」：{:?}",
        events(&app)
    );
    roll(&mut app, p1);
    settle(&mut app, 3);
    assert!(
        events(&app).iter().any(|e| e.starts_with("Roll(P1,")),
        "他该真的掷出骰子：{:?}",
        events(&app)
    );
}

#[test]
fn the_jail_gate_cannot_release_a_patient_from_the_hospital() {
    // 三名玩家：乙关在监狱（所以窗口真的开着），丙关在医院 —— 窗口管的是监狱，
    // 不能把丙也放出来。只放一个医院的病人在场的话窗口根本不会开，这条判据就没被走到
    let mut setup = GameSetup::default();
    setup.players.push(PlayerSetup {
        name: "王小二".into(),
        color: PLAYER_COLORS[2],
    });
    let mut app = boot(setup, GameRules::instant(), seed_rolling_first(1));
    app.insert_resource(DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    place(&mut app, p0, JAIL_JUNCTION - 1);
    let rows = roster(&mut app);
    let p1 = rows[1].0;
    let p2 = rows[2].0;
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 5,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    insert(
        &mut app,
        p2,
        Detention {
            place: DetentionPlace::Hospital,
            turns_left: 3,
        },
    );
    place(&mut app, p2, HOSPITAL_JUNCTION);
    set_points(&mut app, p0, 100);

    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Bail(DetentionPlace::Jail))),
        "乙关在监狱，窗口该开"
    );

    bail_ask(&mut app, p0, p2);
    settle(&mut app, 3);
    assert_eq!(points_of(&app, p0), 100, "没办成就不该扣钱");
    assert_eq!(
        detention_of(&app, p2).map(|d| d.place),
        Some(DetentionPlace::Hospital),
        "医院里的人不该从监狱门口被放出来"
    );
    assert!(
        detention_of(&app, p1).is_some(),
        "另一条请求被拒不该顺手把监狱里的人也放了"
    );
}

#[test]
fn the_hospital_gate_is_its_own_window() {
    let (mut app, p0) = ready_on(HOSPITAL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Hospital,
            turns_left: 3,
        },
    );
    place(&mut app, p1, HOSPITAL_JUNCTION);
    set_points(&mut app, p0, 100);
    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Bail(DetentionPlace::Hospital))),
        "徐家汇是医院的接入点"
    );
    bail_ask(&mut app, p0, p1);
    settle(&mut app, 3);
    assert_eq!(detention_of(&app, p1), None);
    assert_eq!(points_of(&app, p0), 70);
}

#[test]
fn not_enough_points_means_nothing_happens() {
    let (mut app, p0) = ready_on(JAIL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 5,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    set_points(&mut app, p0, 29);
    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);
    bail_ask(&mut app, p0, p1);
    settle(&mut app, 3);

    assert_eq!(points_of(&app, p0), 29, "一分都不该扣");
    assert_eq!(
        detention_of(&app, p1).map(|d| d.turns_left),
        Some(5),
        "人还得关着"
    );
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Bail(")),
        "没办成不该有成交凭据：{:?}",
        ledger(&app)
    );
}

#[test]
fn only_the_player_at_the_window_can_bail() {
    let (mut app, p0) = ready_on(JAIL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 5,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    set_points(&mut app, p0, 100);
    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);
    assert!(matches!(pending_of(&app), Some((_, Pending::Bail(_)))));

    // 两个方向都试：别人发、别人当目标（乙自己还在牢里）
    bail_ask(&mut app, p1, p1);
    bail_ask(&mut app, p1, p0);
    settle(&mut app, 3);
    assert_eq!(points_of(&app, p0), 100);
    assert_eq!(points_of(&app, p1), 0);
    assert!(detention_of(&app, p1).is_some(), "乙不该把自己保出来");
}

#[test]
fn two_requests_for_the_same_head_in_one_frame_charge_once() {
    let (mut app, p0) = ready_on(JAIL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 5,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    set_points(&mut app, p0, 100);
    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);

    bail_ask(&mut app, p0, p1);
    bail_ask(&mut app, p0, p1);
    settle(&mut app, 3);

    assert_eq!(points_of(&app, p0), 70, "同一帧两条请求只该扣一次");
    let paid = ledger(&app)
        .iter()
        .filter(|l| l.starts_with("Bail(P1←P0"))
        .count();
    assert_eq!(paid, 1, "成交凭据也只该有一条：{:?}", ledger(&app));
}

#[test]
fn a_ring_tile_typed_jail_opens_the_same_window_without_a_property_quote() {
    // 真棋盘可以把 `type: jail` 直接放在主环上（演示盘把它挂在 2 号地产上）。
    // 这条钉的是那条入口：落格分派直接进 `LandingDomain::Bail`，不必先答地产报价
    let (mut app, p0) = ready_on(JAIL_JUNCTION, 1, DecisionPolicy::Hold);
    let p1 = other(&mut app, p0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 3,
        },
    );
    place(&mut app, p1, JAIL_JUNCTION);
    set_points(&mut app, p0, 100);
    // 下一格（3 号）改成主环上的监狱格；P0 正站在 2 号，掷 1 点就踩上去
    retile(&mut app, 3, game_core::prelude::TileKind::Jail);

    let p0 = walk_onto(&mut app, p0);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Bail(DetentionPlace::Jail))),
        "主环上的监狱格该直接开保释窗口"
    );
    bail_ask(&mut app, p0, p1);
    settle(&mut app, 3);
    assert_eq!(detention_of(&app, p1), None);
    assert_eq!(points_of(&app, p0), 70);
}

#[test]
fn nobody_to_bail_means_no_window_at_all() {
    // 没人关着的时候，接入点就与路面一样直接收摊 —— 不能弹一个空窗口把回合停住
    let (mut app, p0) = ready_on(JAIL_JUNCTION - 1, 1, DecisionPolicy::Hold);
    set_points(&mut app, p0, 100);
    let p0 = walk_onto(&mut app, p0);
    answer(&mut app, p0, Decision::Decline);
    assert_eq!(pending_of(&app), None, "没人可保却摆了一个窗口");
}
