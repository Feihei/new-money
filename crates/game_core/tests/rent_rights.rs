//! P4② 的验收：人在里面，名下的地**收不到租** —— 而他的地还是他的。
//!
//! PLAN §四 P4 那行的原话是「服刑期间丧失收租权（租金判定读 `Detention`）」。这一半比
//! 容器那一半更容易写歪，因为它有两种**相反方向**的错法：
//!
//! - 判据只挂在「监狱」那一个名字上 → 住院与出国照样收租。P4① 刚把三处去处合成一个状态，
//!   所以这里三个去处各跑一遍同一条断言：换去处不换判据；
//! - 反过来把「他不在场上」当成「他的地也没了」→ 物价指数在有人进监狱的那一刻掉一截，
//!   而那是**下一回合每一笔租金的乘数**。「谁被关着」不该替全场房租定价（P3d③ 的抵押是
//!   同一条理由，那条记一笔等额负债；这一条什么都不记 —— 收租权是会回来的）。
//!
//! 边界的另一侧由 `tests/stay.rs::a_stayed_owner_still_collects_rent...` 钉着：被停留按住
//! 的人还在场上，地照收。那条判据**不许**顺手扩到 `StaySentence`。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    CardAim, CardKind, Detention, DetentionPlace, GameSession, Term, TurnPhase,
};

/// 地块 1（汉口路，2 号格）归 P1 且 flat 成 500，P0 站在起点、下一掷两步正好踩上去。
/// 与 `tests/stay.rs` 同一套前置 —— 那一条测「照样收得到」，这一条测「收不到」。
fn p1s_lot_a_step_away() -> (App, Entity, Entity) {
    let (mut app, p0, p1) = two_of_them(0, 2);
    own(&mut app, 1, Some(1), 1, false);
    flat_rent(&mut app, 1, 500);
    (app, p0, p1)
}

/// 装配到「P0 站在 `tile` 上、下一掷恰好 `steps` 点」，并拿到另一名玩家。
fn two_of_them(tile: u32, steps: u32) -> (App, Entity, Entity) {
    let (mut app, p0) = ready_on(tile, steps, DecisionPolicy::DeclineAll);
    let p1 = roster(&mut app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家");
    (app, p0, p1)
}

/// 命运池里某个 id 的下标（照 `tests/detention.rs` 的写法：按名字拿卡，不靠数组位置）。
fn fate_index(app: &App, id: &str) -> u32 {
    use bevy::asset::Assets;
    use game_core::prelude::{CardSet, DataHandles};
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().fate;
    sets.get(handle)
        .expect("命运牌没加载")
        .0
        .iter()
        .position(|c| c.id == id)
        .unwrap_or_else(|| panic!("fate_cards.json 里没有 {id}")) as u32
}

/// 这一格有没有留下一条收租流水（`Rent(P0->P1@L1$500)` 那个形状）。
fn rent_lines(app: &App, land: &str) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|l| l.starts_with(&format!("Rent(P0->P1@{land}")))
        .collect()
}

/// **三个去处共用一条判据**。这是 P4①「一个状态三个名字」在钱上的推论：如果收租权只认
/// 监狱那一个名字，P4① 那条合并就只是把三个组件换成了一个组件加三条隐藏分支。
#[test]
fn a_locked_owner_collects_nothing_whatever_the_room_is() {
    for place in [
        DetentionPlace::Jail,
        DetentionPlace::Hospital,
        DetentionPlace::Away,
    ] {
        let (mut app, p0, p1) = p1s_lot_a_step_away();
        insert(
            &mut app,
            p1,
            Detention {
                place,
                term: Term::Turns(3),
            },
        );

        play_turns(&mut app, 1);

        assert_eq!(tile_of(&app, p0), 2, "P0 确实踩到了那块地");
        assert!(
            rent_lines(&app, "L1").is_empty(),
            "{place:?} 里的人收不到租，也不该有收据：{:?}",
            ledger(&app)
        );
        assert_eq!(
            cash_of(&mut app, p0),
            20_000,
            "{place:?}：踩上来的人一分没付"
        );
        assert_eq!(
            cash_of(&mut app, p1),
            20_000,
            "{place:?}：这里没有钱动，所以也不该有进项"
        );
        // 报价侧与成交侧读的是同一个 0（`LandView.owner_detained` 那一格）
        assert_eq!(rent_of(&mut app, 1), 0, "{place:?}：报价也得是 0");
    }
}

/// 真卡那一侧：陷害卡把人关进去（走 `detain`，也就是 P4① 那条正路），他的地就收不到租。
/// 上面那条用的是 `insert`，所以这一条补的是「写的那一半与读的那一半接得上」。
#[test]
fn a_card_that_parks_him_in_a_room_is_the_same_one_that_stops_his_rent() {
    let (mut app, p0, p1) = two_of_them(0, 2);
    own(&mut app, 1, Some(1), 1, false);
    flat_rent(&mut app, 1, 500);
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(
        detention_of(&app, p1),
        Some(Detention {
            place: DetentionPlace::Jail,
            term: Term::Turns(5)
        }),
        "卡面判 5 天"
    );
    assert_eq!(
        tile_of(&app, p1),
        2,
        "他被搬去监狱那条分支的接入点（P4① 那一半，这里要它还成立）"
    );

    play_turns(&mut app, 1);

    assert!(
        rent_lines(&app, "L1").is_empty(),
        "被真卡关进去的人，地照样收不到：{:?}",
        ledger(&app)
    );
    assert_eq!(cash_of(&mut app, p0), 20_000);
}

/// PLAN §四 P4 验收原话的那一半：**住院**的人，他的地收不到租。
/// 上面两条的前置是 `insert` 与陷害卡，这一条走的是命运「掉进水沟」+ 医院那条死胡同，
/// 也就是验收里那个场景本身：他被水沟送进医院，别人踩他的地不付钱。
#[test]
fn a_player_the_ditch_sent_to_the_hospital_collects_no_rent() {
    const FATE: u32 = 5;
    // P0 站在 4 号格上，下一掷 1 点正好踩进 5 号命运格（P10d② 重排后命运从 3 号顺移过来）
    let (mut app, p0, p1) = two_of_them(FATE - 1, 1);
    let at = fate_index(&app, "fate_007");
    stack_card(&mut app, CardKind::Fate, at);
    own(&mut app, 1, Some(0), 1, false);
    flat_rent(&mut app, 1, 500);

    play_turns(&mut app, 1);

    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Hospital,
            term: Term::Turns(3)
        }),
        "掉水沟是住院"
    );
    assert_eq!(
        tile_of(&app, p0),
        junction_tile(&app, game_core::board::BranchKind::Hospital),
        "他人已经在医院那条分支的接入点上"
    );

    // 让 P1 从 1 号格走一步踩到 2 号格（地块 1，住院那位名下的地）
    rewind_dice(&mut app);
    make_current(&mut app, p1);
    place(&mut app, p1, 1);
    // 掐**差值**而不是掐绝对余额：环 15 之后这一局里小财神真的附到了 P0 身上，他进场那一
    // 刻就从 P1 钱包里抽走一笔（`GodDeed`），20 000 那个数不再干净。这条用例问的从来只有
    // 「踩到住院那位的要不要付钱」，神的动静发生在快照之前，就不该混进断言里。
    let before = (cash_of(&mut app, p1), cash_of(&mut app, p0));
    let next = app.world().resource::<GameSession>().turn_count + 1;
    play_turns(&mut app, next);

    assert_eq!(tile_of(&app, p1), 2, "P1 确实踩上去了");
    let led = ledger(&app);
    assert!(
        !led.iter().any(|l| l.starts_with("Rent(P1->P0@L1")),
        "住院那位名下的地这一趟不收钱：{led:?}"
    );
    assert_eq!(cash_of(&mut app, p1), before.0, "踩的人一分没付");
    assert_eq!(cash_of(&mut app, p0), before.1, "住院的人一分没进");
    assert_eq!(rent_of(&mut app, 1), 0);
}

/// 跑完两个回合之后的物价指数。`lock` 决定要不要在开跑前把 P1 关起来。
///
/// 为什么要 A/B 两条而不是一条「指数 == 1.0」：`own()` 是**凭空塞一块地进去**（不是买来的），
/// 所以全场资产本来就比开局那一刻多出一截，指数必然离开 1.0 —— 那与拘留无关，第一次跑这条
/// 时读到的 1.0389 就是这么来的。要问的问题从来不是「指数是不是 1.0」，而是
/// 「**关不关他**，指数是不是同一个数」。
fn index_after_two_turns(lock: bool) -> f32 {
    let (mut app, _, p1) = p1s_lot_a_step_away();
    if lock {
        insert(
            &mut app,
            p1,
            Detention {
                place: DetentionPlace::Hospital,
                term: Term::Turns(3),
            },
        );
    }
    play_turns(&mut app, 2);
    app.world().resource::<GameSession>().price_index
}

/// 剥夺的是**这几回合**，不是所有权：他的地仍然算他的资产，物价指数也不许因此挪动。
///
/// 后半条才是这条用例存在的理由。物价指数每回合末按「全场资产 ÷ 初始资产」重算，而它是
/// 下一回合每一笔租金的乘数 —— 如果「他不在场上」顺手把地也从账上摘掉，那么有人进一次
/// 监狱，全场的房租就跟着降一截，出狱再涨回来。那件事在画面上完全不可见。
#[test]
fn locking_him_out_does_not_reprice_the_whole_board() {
    let (mut app, _, p1) = p1s_lot_a_step_away();
    let before = total_assets(&mut app, 1.0);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Hospital,
            term: Term::Turns(3),
        },
    );

    assert_eq!(
        total_assets(&mut app, 1.0),
        before,
        "他的地还是他的资产：关起来不等于没收"
    );
    assert_eq!(
        index_after_two_turns(true),
        index_after_two_turns(false),
        "有人住院不该替全场房租定价"
    );
}

/// 回来那一格：期满之后他的地照收租。判 3 = 跳 2 的口径（P4① 钉过）在这里的读数是
/// 「第 6 回合开始放出人」，所以跑到第 6 格再问报价。
#[test]
fn the_rent_comes_back_the_turn_he_steps_out() {
    let (mut app, _, p1) = p1s_lot_a_step_away();
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            term: Term::Turns(3),
        },
    );
    assert_eq!(rent_of(&mut app, 1), 0, "关着的时候报价是 0");

    play_turns(&mut app, 6);

    assert_eq!(
        detention_of(&app, p1),
        None,
        "期满之后组件必须消失（回合机那一半由 tests/detention.rs 钉）"
    );
    assert_eq!(
        rent_of(&mut app, 1),
        500,
        "回来之后他的地照收租 —— 剥夺的是那几回合，不是这块地的租金表"
    );
    assert_eq!(phase(&app), TurnPhase::WaitRoll);
}
