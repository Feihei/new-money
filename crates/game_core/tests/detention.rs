//! P4① 的验收：坐牢、住院、出国是**同一件事的三个名字**，所以规则只有一处，
//! 而差别全部落在两样东西上 —— 人话（`label`）与棋盘上那个停车位（`branch`）。
//!
//! PLAN §四 给这一格的原话是「不要各建一个状态」。这条决定能验证的形状是：
//!
//! - 三个去处走的是同一条 `detain`、同一条 `Detention` 组件、同一条跳过/回来流水；
//! - 换去处**不新增任何一处结算分支** —— 于是「他在哪儿」改变不了「世界怎么对待他」，
//!   除了棋子停在哪一格；
//! - `Away` 是这一格最硬的检查点：那个地方**不在棋盘上**，所以没有死胡同格可停，
//!   他留在自己那一格 —— 如果模型是「三个组件三条分支」，这一支必然要现编一个格子。
//!
//! 收租权那一半是 P4②（`tests/rent_rights.rs`），这里只钉容器。

mod common;

use bevy::app::App;
use bevy::asset::Assets;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    ActiveBoard, BranchKind, CardKind, CardSet, DataHandles, Detention, DetentionPlace, TurnPhase,
};

/// prototype 上的三个格号：5 号是命运格（抽到就是这张卡），2 号挂监狱分支，9 号挂医院分支。
const FATE: u32 = 5;
const JAIL_JUNCTION: u32 = 2;
const HOSPITAL_JUNCTION: u32 = 9;

/// 命运池里某个 id 的下标（与 `item_index` 同一条理由：按名字拿卡，不靠数组位置）。
fn fate_index(app: &App, id: &str) -> u32 {
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().fate;
    sets.get(handle)
        .expect("命运牌没加载")
        .0
        .iter()
        .position(|c| c.id == id)
        .unwrap_or_else(|| panic!("fate_cards.json 里没有 {id}")) as u32
}

/// 装配到「P0 站在命运格前面一步、下一掷恰好 1 点、掷骰之前」，并把牌堆顶压成 `id`。
fn about_to_draw(id: &str) -> (App, Entity, u32) {
    let (mut app, p0) = ready_on(FATE - 1, 1, DecisionPolicy::DeclineAll);
    let at = fate_index(&app, id);
    stack_card(&mut app, CardKind::Fate, at);
    (app, p0, at)
}

/// 抽完那张命运牌（走完那一步并结算落格）。
fn draw_it(app: &mut App) {
    play_turns(app, 1);
}

#[test]
fn only_the_two_real_rooms_have_a_door_on_the_board() {
    // 这张映射表是「三个名字一件事」的全部差别所在：两条拓扑 + 三个人话词。
    // 新增第四种去处（P10 的旅馆、P11 的太空船）必须在这里表态
    assert_eq!(DetentionPlace::Jail.branch(), Some(BranchKind::Jail));
    assert_eq!(
        DetentionPlace::Hospital.branch(),
        Some(BranchKind::Hospital)
    );
    assert_eq!(
        DetentionPlace::Away.branch(),
        None,
        "出国不在棋盘上 —— 给它配一条死胡同格就是造一个假地方"
    );
    for place in [
        DetentionPlace::Jail,
        DetentionPlace::Hospital,
        DetentionPlace::Away,
    ] {
        assert!(!place.label().is_empty(), "{place:?} 说不出他在哪儿");
        assert!(!place.back_verb().is_empty(), "{place:?} 说不出他回来了");
    }
}

#[test]
fn a_trip_to_the_ditch_parks_him_at_the_hospital_junction() {
    let (mut app, p0, _) = about_to_draw("fate_007");
    draw_it(&mut app);

    assert_eq!(
        tile_of(&app, p0),
        HOSPITAL_JUNCTION,
        "逻辑位置停在医院那条分支的接入点，不是死胡同格本身"
    );
    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Hospital,
            turns_left: 3
        }),
        "掉水沟是住院，不是坐牢"
    );
    assert!(
        ledger(&app).contains(&"Detain(P0@医院,格9)".to_string()),
        "流水要带得出他在哪儿：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_hospital_stay_skips_two_turns_and_says_which_door_he_came_back_through() {
    // PLAN §四 P4 的验收原话：「有人住院时棋子停在医院死胡同格、回合被跳过、判 3 天＝跳过 2 个
    // 回合」。这里能验到的是后两条 + 「回来那一句说的是出院而不是出狱」；
    // 棋子**视觉上**停在死胡同格是表现层读 `end` 那个网格坐标的事，由真鼠标那张截图验
    let (mut app, p0, _) = about_to_draw("fate_007");
    draw_it(&mut app);
    // 六格 = 他(抽到住院) / 乙 / 他跳 / 乙 / 他跳 / 乙，尾巴上那几帧走进他的第三格
    play_turns(&mut app, 6);

    let ev = events(&app);
    assert_eq!(
        ev.iter()
            .filter(|e| e.starts_with("DetainSkip(P0@医院,"))
            .count(),
        2,
        "判 3 天＝跳过 2 个回合，与坐牢同一条口径：{ev:?}"
    );
    assert!(
        ev.contains(&"DetainSkip(P0@医院,剩2)".to_string())
            && ev.contains(&"DetainSkip(P0@医院,剩1)".to_string()),
        "剩余要逐格递减：{ev:?}"
    );
    assert!(
        ev.iter().any(|e| e.starts_with("Lifted(P0@医院,格9")),
        "回来那一句要带得出他从哪一格继续走：{ev:?}"
    );
    assert_eq!(detention_of(&app, p0), None, "回来之后组件必须消失");
    assert_eq!(phase(&app), TurnPhase::WaitRoll);
    assert_eq!(current(&app), p0, "第三格该轮到他掷骰");
    assert_eq!(tile_of(&app, p0), HOSPITAL_JUNCTION, "住院期间不该移动");
}

#[test]
fn a_trip_abroad_has_no_dead_end_to_park_him_at_so_he_stays_put() {
    // 这一条是「不要各建一个状态」的试金石：出国那个地方在棋盘上不存在，
    // 于是模型只能承认「接入点就是他此刻那一格」，回来时也从这格继续走。
    // 谁哪天为 `Away` 现编一个格子，这条会红
    let (mut app, p0, _) = about_to_draw("fate_008");
    draw_it(&mut app);

    assert_eq!(
        tile_of(&app, p0),
        FATE,
        "他站在命运格上被送出国，那格就是他回来的落点"
    );
    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Away,
            turns_left: 3
        })
    );
    assert!(
        ledger(&app).contains(&"Detain(P0@国外,格5)".to_string()),
        "出国也要有凭据，虽然它没有停车位：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_shorter_detention_keeps_both_the_old_place_and_the_old_count() {
    // 已经要被关 5 天的人再吃一张 3 天：他该待原来那个地方满 5 天。
    // 覆盖等于替人减刑 —— 与 P3d④c 停留卡同一条理由，所以那条规则在这里也成立
    let (mut app, p0, _) = about_to_draw("fate_001"); // 直接入狱 3 回合
    insert(
        &mut app,
        p0,
        Detention {
            place: DetentionPlace::Hospital,
            turns_left: 5,
        },
    );
    draw_it(&mut app);

    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Hospital,
            turns_left: 5
        }),
        "短的那张不该把他换个地方关得更久，也不该让他早出来"
    );
    assert_eq!(
        tile_of(&app, p0),
        FATE,
        "处境没变，棋子就不该被搬到监狱那条分支的接入点"
    );
}

#[test]
fn two_detentions_of_equal_length_move_him_to_the_newer_place() {
    // 「较长者赢」剩下唯一的任意格是**等长**。取后来者：他此刻被新的一张抓走了，
    // 天数不变。这条断言不证明哪个读法更好，它钉住的是「这里有个决定、别飘」
    let (mut app, p0, _) = about_to_draw("fate_001");
    insert(
        &mut app,
        p0,
        Detention {
            place: DetentionPlace::Hospital,
            turns_left: 3,
        },
    );
    draw_it(&mut app);

    assert_eq!(
        detention_of(&app, p0),
        Some(Detention {
            place: DetentionPlace::Jail,
            turns_left: 3
        })
    );
    assert_eq!(tile_of(&app, p0), JAIL_JUNCTION, "换了地方就该搬走");
}

#[test]
fn an_insulated_victim_shrugs_off_a_trip_abroad_too() {
    // 抵消判据挂在「事」那一族上（`AfflictDomain::Detention`），不是挂在「监狱」那一个名字上：
    // 免罪卡挡得住坐牢，就同样挡得住住院与出国
    let (mut app, p0, _) = about_to_draw("fate_008");
    let insulate = item_index(&app, "item_insulate");
    deal(&mut app, p0, insulate.0);
    play_card(&mut app, p0, insulate.0, None);
    settle(&mut app, 2);
    assert_eq!(shield_of(&app, p0), Some(1), "刚挂上两层？");

    draw_it(&mut app);

    assert_eq!(detention_of(&app, p0), None, "挡住了就没有出国这回事");
    assert_eq!(tile_of(&app, p0), FATE, "也没被搬到任何接入点");
    assert_eq!(
        shield_of(&app, p0),
        None,
        "那一层护身吃掉了这次厄运，就该空掉"
    );
}

#[test]
fn a_board_without_that_branch_sends_nobody_there() {
    // 数据把送去的地方写在卡上，棋盘却可能没有那条分支（自定义盘）。这时**什么都不改**：
    // 不猜一个格子、也不回退到起点。卡照样抽掉了（那是牌堆的事），但流水里不该有一条 Detain
    let (mut app, p0, _) = about_to_draw("fate_007");
    app.world_mut().resource_mut::<ActiveBoard>().def.tiles[HOSPITAL_JUNCTION as usize].branch =
        None;

    draw_it(&mut app);

    let led = ledger(&app);
    assert!(
        !led.iter().any(|l| l.starts_with("Detain(")),
        "没有那条分支就不该有这一次拘留：{led:?}"
    );
    assert_eq!(detention_of(&app, p0), None);
    assert_eq!(tile_of(&app, p0), FATE, "他没被搬到任何地方");
    assert!(
        led.iter().any(|l| l.starts_with("Card(P0:fate_007")),
        "牌是抽掉了的，只是这件事没落地：{led:?}"
    );
}

/// 一条流水的「形状」：事件名留着，载荷（谁、在哪个地方、剩几天）抹掉。
fn shape(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| match l.split_once('(') {
            Some((head, _)) => format!("{head}(…)"),
            None => l.clone(),
        })
        .collect()
}

#[test]
fn the_three_places_share_one_skip_rule_because_they_are_one_state() {
    // 这一条是整批用例的重点：**换去处不换规则**。三个去处各抽一张卡、各跑到同一格回合数，
    // 看到的流水形状必须完全一致。哪天有人给医院加一条「交钱就能买通」、或给出国加一条
    // 「回来重新掷骰」，这条会以「多一格／少一格」的方式红 —— 而不是靠读代码发现分支变多了
    let mut shapes: Vec<(String, Vec<String>)> = Vec::new();
    for id in ["fate_001", "fate_007", "fate_008"] {
        let (mut app, _, _) = about_to_draw(id);
        draw_it(&mut app);
        play_turns(&mut app, 5);
        let ev: Vec<String> = events(&app)
            .into_iter()
            .filter(|e| {
                e.starts_with("TurnStarted(")
                    || e.starts_with("DetainSkip(")
                    || e.starts_with("Lifted(")
            })
            .collect();
        assert_eq!(
            ev.iter().filter(|e| e.starts_with("DetainSkip(")).count(),
            2,
            "{id} 没跳过两个回合，形状比较就没有意义：{ev:?}"
        );
        shapes.push((id.to_string(), shape(&ev)));
    }
    assert_eq!(
        shapes[0].1, shapes[1].1,
        "坐牢与住院的回合流水形状不同：{shapes:?}"
    );
    assert_eq!(
        shapes[0].1, shapes[2].1,
        "坐牢与出国的回合流水形状不同：{shapes:?}"
    );
}
