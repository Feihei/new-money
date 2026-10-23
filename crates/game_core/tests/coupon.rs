//! P1 的验收：点券格踩上去发一次、冷却期间谁都不发、到期再发，且这笔账不进现金。
//!
//! 落格一律走「把玩家摆在 `箱子号 - 点数` 那格 + 掷出点数的种子」这条真路径
//! （`ready_on`），于是被测的是分派表与结算 system，不是手搓的组件。
//! prototype 的 14 号格就是点券箱（面额 30，冷却走 `GameRules` 缺省 6 回合；
//! P10d② 重排后它是 13 号，P11 的新闻中心插在 13 号那一格之前、它顺移到 14）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::NextState;
use common::*;
use game_core::prelude::{ActiveBoard, GameSession, TileKind, TurnPhase};

/// prototype 上的点券箱（`assets/data/boards/prototype.json` 的 14 号格）。
const BOX: u32 = 14;

fn land(app: &mut App) {
    play_turns(app, turn_count(app) + 1);
}

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

/// p0 从 13 号（新闻中心）掷 1 点，正好踩上 14 号箱子。
fn step_onto_the_box() -> (App, Entity) {
    let (app, p0) = ready_on(BOX - 1, 1, DecisionPolicy::DeclineAll);
    assert_eq!(tile_of(&app, p0), BOX - 1, "前置：把人摆在箱子前一格");
    (app, p0)
}

#[test]
fn a_coupon_box_pays_points_and_never_cash() {
    let (mut app, p0) = step_onto_the_box();
    land(&mut app);

    assert_eq!(points_of(&app, p0), 30, "面额写在格子上");
    assert_eq!(
        cash_of(&mut app, p0),
        20_000,
        "两套账：点券一格都不该挪动现金"
    );
    assert!(
        ledger(&app).contains(&"Points(P0+30=30)".to_string()),
        "到账要留流水：{:?}",
        ledger(&app)
    );
    assert_eq!(pending_of(&app), None, "捡箱子没有「要不要」，不该挂住回合");
    assert_eq!(turn_count(&app), 1, "回合照常收得掉");
    assert_cash_audit(&mut app, 20_000);
    assert_points_audit(&mut app, 0);
}

#[test]
fn a_box_still_cooling_pays_nobody() {
    let (mut app, p0) = step_onto_the_box();
    land(&mut app);
    assert_eq!(boxes_taken(&app), 1, "前置：刚被取走");

    // 同一个人再来一次（骰子流退回开局、轮转挪回 p0）：冷却没到就一分钱不发
    rewind_dice(&mut app);
    make_current(&mut app, p0);
    place(&mut app, p0, BOX - 1);
    land(&mut app);

    assert_eq!(points_of(&app, p0), 30, "二次踩同一格不该再发一次");
    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|line| line.starts_with("Points("))
            .count(),
        1,
        "不发就不该留流水：{:?}",
        ledger(&app)
    );
    assert_points_audit(&mut app, 0);
}

#[test]
fn the_box_pays_again_once_the_cooldown_lapses() {
    let (mut app, p0) = step_onto_the_box();
    land(&mut app);
    assert_eq!(boxes_taken(&app), 1);

    // 冷却读的就是 `GameSession::turn_count`，把它推过 6 回合即「时间过去了」；
    // 真跑六个回合会把利息、分红、行情一起卷进来，那些不是这一条要守的东西
    set_turn(&mut app, 6);
    rewind_dice(&mut app);
    make_current(&mut app, p0);
    place(&mut app, p0, BOX - 1);
    land(&mut app);

    assert_eq!(points_of(&app, p0), 60, "冷却到期该重新长出来");
    assert_eq!(turn_count(&app), 7, "落格时正好是第 6 回合之后");
    assert_points_audit(&mut app, 0);
}

/// 三档 10/30/50 由数据决定，不是代码里的三张表；随机抽会让三档期望值相同（PLAN §五）。
#[test]
fn the_three_face_values_come_from_the_tile_data() {
    for amount in [10, 30, 50] {
        let (mut app, p0) = step_onto_the_box();
        coupon_tile(&mut app, BOX as usize, amount, None);
        land(&mut app);
        assert_eq!(
            points_of(&app, p0),
            amount,
            "面额 {amount} 没按格子上的数字到账"
        );
    }
}

/// 冷却是一格一条记录：抢到一个不该把全盘都掏空。
#[test]
fn the_cooldown_is_per_tile() {
    let (mut app, p0) = step_onto_the_box();
    coupon_tile(&mut app, 3, 10, None); // 3 号保险公司格改成另一颗 10 点的箱子
    land(&mut app);
    assert_eq!(points_of(&app, p0), 30);

    rewind_dice(&mut app);
    make_current(&mut app, p0);
    place(&mut app, p0, 2);
    land(&mut app);

    assert_eq!(points_of(&app, p0), 40, "13 号还在冷却，3 号那颗该照发");
    assert_eq!(boxes_taken(&app), 2);
    assert_points_audit(&mut app, 0);
}

/// 分派表里被点名成「还没有实现」的格：不发钱、不挂决策，但回合必须收得掉。
/// 这一条守的是缺陷 #8 的另一半 —— 静默可以是「什么都不做」，不能是「卡住」。
///
/// 用的是**神明格**而不是彩票格：彩票中心在 P8 有了结算（踩上去开买券窗口），
/// 拿它当「什么都不做」的那一格，这条就悄悄变成在测「窗口会被自动答完」——
/// 那是 `common` 夹具的行为，不是分派表待办那一半的行为。
#[test]
fn an_unimplemented_tile_still_closes_the_turn() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    retile(&mut app, 1, TileKind::GodDeity);
    land(&mut app);

    assert_eq!(points_of(&app, p0), 0);
    assert_eq!(cash_of(&mut app, p0), 20_000);
    assert_eq!(pending_of(&app), None);
    assert_eq!(turn_count(&app), 1, "没有结算的格子也该把回合结束掉");
}

/// 重开一局必须把冷却表清空，否则第二局一开局整盘都是空的
/// （跨局残留那一族缺陷的根，见 ARCHITECTURE §6.1 坑 22/25）。
#[test]
fn a_rebooted_world_refills_the_boxes() {
    let (mut app, p0) = step_onto_the_box();
    land(&mut app);
    assert_eq!(points_of(&app, p0), 30, "前置：刚被取走一格");
    assert_eq!(boxes_taken(&app), 1);

    // 只走 core 这一侧的重开：装配 system 负责把「这一局」的运行时状态换成新的，
    // 表现层的清场（`scene.rs` 的 `teardown_match`）是另一件事
    app.world_mut().remove_resource::<ActiveBoard>();
    app.world_mut()
        .resource_mut::<NextState<TurnPhase>>()
        .set(TurnPhase::Boot);
    assert!(run_until_world(&mut app), "世界没能重装");
    assert_eq!(boxes_taken(&app), 0, "新局开局即整盘冷却中");
}
