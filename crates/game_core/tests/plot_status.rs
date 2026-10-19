//! P3c 的验收：地块上的限时状态**真的改到了钱**，而且会自己消失。
//!
//! 这一批用例刻意从「踩上去实际付了多少」那一侧断言（`Rent(P0->P1@L0$…)` 那条流水），
//! 而不是只查组件：涨价/查封的全部意义都在别人掏钱的那一刻，组件挂上了而钱没变，
//! 或者钱变了而流水里说不出为什么，都是同一类没做到。
//! 纯算术那半边（垄断 × 涨价的先后、查封挡不挡加盖）在 `economy.rs` 的单测里，
//! 两边各证一段 —— 这里再重复一遍乘法就只是在测测试了。

mod common;

use bevy::prelude::*;
use common::*;
use game_core::prelude::{CardAim, LandId, LandStatus};

/// brown 街（南京路 = 地块 0、汉口路 = 地块 1）整条归 P1，P0 下一步就踩到 0 号。
///
/// 两块都给同一个人：凑成垄断，于是流水里那个数同时含「×2 垄断」与「×2 涨价」——
/// 涨价卡到底乘在哪一步，从这条流水就读得出来（顺序在 `economy::calculate_rent`）。
fn brown_street_ready_to_be_hiked() -> (App, Entity) {
    // 1 号格 = 地块 0 南京路（同 tests/damage.rs 的走位）
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 0, Some(1), 1, false);
    own(&mut app, 1, Some(1), 1, false);
    (app, p0)
}

#[test]
fn a_hiked_street_doubles_the_rent_somebody_actually_pays() {
    let (mut app, p0) = brown_street_ready_to_be_hiked();
    let hike = item_index(&app, "item_hike");
    deal(&mut app, p0, hike.0);
    play_card(&mut app, p0, hike.0, Some(CardAim::Land(LandId(0))));
    settle(&mut app, 2);

    // 整条街都盖到了 —— 「街」是范围，不是只打锚点那块
    assert_eq!(status_of(&mut app, 0), Some((LandStatus::PriceHike, 5)));
    assert_eq!(
        status_of(&mut app, 1),
        Some((LandStatus::PriceHike, 5)),
        "涨价卡写的是整街，同组的另一块也得算进去"
    );
    // 承受者是业主（P1），不是按按钮的人
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_hike)".to_string()),
        "出牌流水要说清办的是谁的地：{:?}",
        ledger(&app)
    );

    play_turns(&mut app, 1);
    let led = ledger(&app);
    assert!(
        led.contains(&"Rent(P0->P1@L0$240)".to_string()),
        "垄断 120 再涨价 = 240，实际付的就是这个数：{led:?}"
    );
    assert_eq!(
        cash_of(&mut app, p0),
        20_000 - 240,
        "涨价的钱真的从踩上来的人口袋里出去"
    );
}

#[test]
fn a_sealed_street_collects_nothing_and_replaces_the_hike() {
    let (mut app, p0) = brown_street_ready_to_be_hiked();
    let hike = item_index(&app, "item_hike");
    let seal = item_index(&app, "item_seal");
    deal(&mut app, p0, hike.0);
    deal(&mut app, p0, seal.0);
    play_card(&mut app, p0, hike.0, Some(CardAim::Land(LandId(0))));
    settle(&mut app, 2);
    assert_eq!(status_of(&mut app, 0), Some((LandStatus::PriceHike, 5)));

    // PLAN §三 那对「抵消对」：查封盖在同一条街上，单槽位直接替换
    play_card(&mut app, p0, seal.0, Some(CardAim::Land(LandId(1))));
    settle(&mut app, 2);
    assert_eq!(
        status_of(&mut app, 0),
        Some((LandStatus::Sealed, 5)),
        "查封要抵掉涨价 —— 同一块地上不许同时挂着两种，租金就说不清"
    );
    assert_eq!(status_of(&mut app, 1), Some((LandStatus::Sealed, 5)));

    play_turns(&mut app, 1);
    let led = ledger(&app);
    assert!(
        !led.iter().any(|e| e.starts_with("Rent(P0->P1@L0")),
        "封条贴着的那一趟什么都不该收，也不该有收据：{led:?}"
    );
    assert_eq!(cash_of(&mut app, p0), 20_000, "P0 一分没付");
}

#[test]
fn one_turn_end_burns_exactly_one_day_off_the_stamp() {
    // 「5 天」= 五个回合结束。与日历、行情 tick 同一把尺子（`status.rs` 模块头那条口径）
    let (mut app, p0) = brown_street_ready_to_be_hiked();
    let hike = item_index(&app, "item_hike");
    deal(&mut app, p0, hike.0);
    play_card(&mut app, p0, hike.0, Some(CardAim::Land(LandId(0))));
    settle(&mut app, 2);
    assert_eq!(status_of(&mut app, 0), Some((LandStatus::PriceHike, 5)));

    play_turns(&mut app, 1);
    assert_eq!(
        status_of(&mut app, 0),
        Some((LandStatus::PriceHike, 4)),
        "跑完一个回合，天数该且只该减一"
    );
}

#[test]
fn the_stamp_falls_off_by_itself_and_the_rent_goes_back() {
    let (mut app, p0) = brown_street_ready_to_be_hiked();
    let hike = item_index(&app, "item_hike");
    deal(&mut app, p0, hike.0);
    play_card(&mut app, p0, hike.0, Some(CardAim::Land(LandId(0))));
    settle(&mut app, 2);

    // 五回合 = 五天。第六回合起，这块地读出来的租金就是原价 —— 不再断言「有人踩上去付了多少」，
    // 因为骰子不保证还会有人踩回来（`rent_of` 就是那一脚真踩下来时要收的数）
    play_turns(&mut app, 5);
    assert_eq!(
        status_of(&mut app, 0),
        None,
        "到期就该自己消失，没人记得去摘它就不算做到"
    );
    assert_eq!(rent_of(&mut app, 0), 120, "涨价过期后回到垄断原价 120");
}

#[test]
fn an_unowned_lot_cannot_be_targeted() {
    // 地块 3（徐家汇）无主。查封一块无主地没有意义：没有业主可办，
    // 而 UI 也只列有主的地（同一句判据，见 `aim_is_legal`）
    let (mut app, p0) = brown_street_ready_to_be_hiked();
    let hike = item_index(&app, "item_hike");
    deal(&mut app, p0, hike.0);
    play_card(&mut app, p0, hike.0, Some(CardAim::Land(LandId(3))));
    settle(&mut app, 2);

    assert_eq!(status_of(&mut app, 3), None, "无主地不该被盖上限时状态");
    assert!(
        !hand_of(&app, p0).is_empty(),
        "被拒的出牌不许把卡吃掉：{:?}",
        hand_of(&app, p0)
    );
}
