//! P3d④c 的验收：停留卡按住的是**人**，不是他的地、他的方向、他的钱包。
//!
//! 这条卡落在 `begin_turn_system` 那一格里，所以它最容易写错的三种可能都不在卡面上：
//!
//! - 它不该被合成进 `Detention`：被拘留的人要被送进死胡同格，停留的人**还在自己踩到的
//!   那一格上**，棋子不必位移、回来的凭据（`DetentionLifted`）也不该发；
//! - 两张计时同时挂着时，一回合只跳一次、两个计数**各减一**（不是互相延长、也不是各跳一回）；
//! - 「判 3 = 跳 2」这条口径必须与拘留同一条，否则卡面那句「停 3 回合」在两种处境下
//!   是两种长度，玩家没法判断。
//!
//! 还有一条是它与 P4 的边界：拘留与停留只差「他在不在牌桌上」，而差别落在**收租**上
//! （P4② 给 `Detention` 加了那条判据）。`a_stayed_owner_still_collects_rent...` 钉的就是
//! 「那条判据不许顺手扩到 `StaySentence`」—— 人还在场上的人，地还得收。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{CardAim, Detention, DetentionPlace, Heading, StaySentence, TurnPhase};

/// 这位此刻朝哪边走。缺组件就 panic：`Heading` 是必挂的那一格（`spawn_players`）。
fn heading_of(app: &App, player: Entity) -> Heading {
    *app.world()
        .entity(player)
        .get::<Heading>()
        .expect("每个玩家都该挂着 Heading（spawn_players 漏挂了？）")
}

/// 装配到「P0 站在 `tile` 上、下一掷恰好 `steps` 点、掷骰之前」，并拿到另一名玩家。
/// 默认阵容两名玩家，`PlayerId` 就是 0 与 1 —— 流水里的 `P0`/`P1` 按这个读。
fn two_of_them(tile: u32, steps: u32) -> (App, Entity, Entity) {
    let (mut app, p0) = ready_on(tile, steps, DecisionPolicy::DeclineAll);
    let p1 = roster(&mut app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家");
    (app, p0, p1)
}

/// 把停留卡发到 `player` 手里并对 `aim` 打出去。
fn hold_there(app: &mut App, player: Entity, aim: Entity) {
    let card = item_index(app, "item_stay");
    deal(app, player, card.0);
    play_card(app, player, card.0, Some(CardAim::Player(aim)));
    settle(app, 2);
}

/// `who` 被停留卡跳过的高水位线（按到达顺序）。
fn stay_skips(app: &App, who: &str) -> Vec<String> {
    events(app)
        .into_iter()
        .filter(|e| e.starts_with(&format!("StaySkip({who},")))
        .collect()
}

/// `who` 掷骰的次数。
fn rolls_for(app: &App, who: &str) -> usize {
    events(app)
        .iter()
        .filter(|e| e.starts_with(&format!("Roll({who},")))
        .count()
}

#[test]
fn a_stay_card_holds_the_victim_on_his_own_tile_without_touching_his_wallet() {
    // 效果落成的形状只有「一个计数」。位置、方向、钱三样一样都不该跟着动 ——
    // 这三条读数就是这张卡与入狱卡的全部区别
    let (mut app, p0, p1) = two_of_them(4, 3);
    place(&mut app, p1, 7);
    hold_there(&mut app, p0, p1);

    assert_eq!(stay_of(&app, p1), Some(3), "卡面上写 3 回合，身上就得是 3");
    assert_eq!(tile_of(&app, p1), 7, "人还站在他踩到的那一格上");
    assert_eq!(
        heading_of(&app, p1),
        Heading::Forward,
        "停留不是转向：不该顺手把方向也翻了"
    );
    assert_eq!(cash_of(&mut app, p1), 20_000, "这里没有钱动");
    assert_eq!(
        stay_of(&app, p0),
        None,
        "出手的人自己没被打到，回合结束也不该顺手给他挂一个"
    );
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_stay)".to_string()),
        "流水要说清这是对谁用的：{:?}",
        ledger(&app)
    );
}

#[test]
fn judged_three_turns_he_is_skipped_twice_and_walks_again_on_the_third() {
    // 与服刑同一条口径（`turns_left == 1` 的那一回合当场可行动），所以卡面那句「停 3 回合」
    // 在这里读作「跳 2 个回合」。回合序是 P0、P1 交替，走 6 个回合正好覆盖 P1 的三格
    let (mut app, p0, p1) = two_of_them(4, 3);
    hold_there(&mut app, p0, p1);
    play_turns(&mut app, 6);

    assert_eq!(
        stay_skips(&app, "P1"),
        vec!["StaySkip(P1,剩2)", "StaySkip(P1,剩1)"],
        "剩余值要逐格递减、且只跳两格：{:?}",
        events(&app)
    );
    assert!(
        events(&app).contains(&"StayLift(P1)".to_string()),
        "第三格要有一句期满：{:?}",
        events(&app)
    );
    assert_eq!(stay_of(&app, p1), None, "期满之后组件必须消失");
    assert!(
        !events(&app).iter().any(|e| e.starts_with("Lifted(P1@")),
        "他从没进过死胡同格，不该有一条出狱凭据：{:?}",
        events(&app)
    );
    assert_eq!(
        rolls_for(&app, "P1"),
        1,
        "六回合里他只该在期满那一格掷一次骰"
    );
}

#[test]
fn every_skipped_turn_closes_right_after_the_notice_and_never_walks() {
    // 「一回合只跳一次」的结构形状：`StaySkip` 后面紧跟的就是这一回合的 `TurnEnd` ——
    // 中间没有掷骰、没有行走，也没有第二句跳过。谁哪天把两条计时写成各跳一回，
    // 这一条会先红，而屏幕上看到的正是「一个人平白无故消失两个回合」
    //
    // 只跑到第 5 回合（P0 的第三格）：P1 那时刚被放出，而脚手架没替他按骰子，
    // 于是「他一步没走」这条断言才不会被他自己期满后的那一步污染
    let (mut app, p0, p1) = two_of_them(4, 3);
    hold_there(&mut app, p0, p1);
    play_turns(&mut app, 5);

    let ev = events(&app);
    for (at, line) in ev.iter().enumerate() {
        let Some(rest) = line.strip_prefix("StaySkip(") else {
            continue;
        };
        let who = rest.split(',').next().expect("流水形状是 StaySkip(Pn,剩k)");
        let expected = format!("TurnEnd({who})");
        assert_eq!(
            ev.get(at + 1).map(String::as_str),
            Some(expected.as_str()),
            "跳过之后应当天就结束这一回合：{ev:?}"
        );
    }
    // 被按住的那两格里，他的棋子一步都没走
    assert_eq!(tile_of(&app, p1), 0, "起点到起点，中间不该有任何位移");
    assert!(
        !ev.iter()
            .any(|e| e.starts_with("Step(P1,") || e.starts_with("Arrive(P1,")),
        "跳过的那两回合不该有行走：{ev:?}"
    );
    assert!(
        !ev.iter().any(|e| e.starts_with("Roll(P1,")),
        "跳过的那两回合连骰子都不该掷：{ev:?}"
    );
    assert_eq!(
        (phase(&app), current(&app)),
        (TurnPhase::WaitRoll, p1),
        "期满那一格他的回合正常开着，只等按骰子：{ev:?}"
    );
}

#[test]
fn a_stayed_owner_still_collects_rent_because_he_is_still_at_the_table() {
    // 这条是它与 P4 的边界所在：停留的人**在场**，名下的地照样收租。
    // 前置用 `insert` 直接挂计数而不是再打一张卡：这一条要测的是「挂着的那几回合
    // 世界怎么对待他」，卡怎么打出去已经有别的用例钉住了
    let (mut app, p0, p1) = two_of_them(0, 2);
    own(&mut app, 1, Some(1), 1, false);
    flat_rent(&mut app, 1, 500);
    insert(&mut app, p1, StaySentence { turns_left: 3 });
    // 空转几帧：现在站在 `WaitRoll` 里的是 P0，没有人为 P1 按下骰子，
    // 而他的计数一格都没少 —— 扣计数的那件事只发生在**他自己的**回合开始
    settle(&mut app, 3);
    assert_eq!(stay_of(&app, p1), Some(3), "别人的回合不消耗他的计数");

    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 2, "0 号往前走两步是 2 号格，那块地是 L1");
    assert!(
        ledger(&app).contains(&"Rent(P0->P1@L1$500)".to_string()),
        "被按住的人照样收得到租：{:?}",
        ledger(&app)
    );
    assert_eq!(cash_of(&mut app, p1), 20_500, "租金要真的进他的钱包");
    // 尾巴上多跑的那几帧已经把他自己的第一格扣掉了（3→2），而那一格只扣一次
    assert_eq!(
        stay_of(&app, p1),
        Some(2),
        "他的第一格跳过之后剩 2：{:?}",
        events(&app)
    );
    assert_eq!(
        stay_skips(&app, "P1"),
        vec!["StaySkip(P1,剩2)"],
        "收租那一格与他的第一格之间不该多出一个回合：{:?}",
        events(&app)
    );
}

#[test]
fn a_stay_and_a_jail_on_the_same_head_count_down_separately_and_skip_once() {
    // 同时挂着两张计时：P1 那一格是「服 2 + 停 3」。第一格两条各减一且只跳一次，
    // 第二格服刑已经扣到 1（不再占用）而停留还在占用 —— 关键是那一格**不该提前放出狱凭据**，
    // 他还在被停留按住，此时发 `DetentionLifted` 等于说「他可以走了」
    let (mut app, p0, p1) = two_of_them(4, 3);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            turns_left: 2,
        },
    );
    hold_there(&mut app, p0, p1);
    play_turns(&mut app, 6);

    let ev = events(&app);
    assert_eq!(
        ev.iter()
            .filter(|e| e.starts_with("DetainSkip(P1@监狱,"))
            .count(),
        1,
        "服刑那一格只占用一回合，之后就让位给停留：{ev:?}"
    );
    assert_eq!(
        stay_skips(&app, "P1"),
        vec!["StaySkip(P1,剩2)", "StaySkip(P1,剩1)"],
        "两条计时各扣各的：{ev:?}"
    );
    // 「一回合只跳一次」在这一格里长这样：两条 Skip 是**相邻**两行，中间没有第二次
    // TurnStarted —— 也就是同一个回合开始同时扣了两格，而不是各开一个回合各扣一格
    let jail_at = ev
        .iter()
        .position(|e| e == "DetainSkip(P1@监狱,剩1)")
        .expect("该有一条服刑跳过");
    assert_eq!(
        ev.get(jail_at + 1).map(String::as_str),
        Some("StaySkip(P1,剩2)"),
        "两条跳过发生在同一个回合：{ev:?}"
    );
    assert_eq!(
        ev.iter().filter(|e| e.as_str() == "TurnEnd(P1)").count(),
        3,
        "他一共只有三个回合：跳两格 + 期满那一格正常走：{ev:?}"
    );
    assert_eq!(
        rolls_for(&app, "P1"),
        1,
        "三个回合里只有期满那一格掷了骰：{ev:?}"
    );
    let released_at = ev
        .iter()
        .position(|e| e.starts_with("Lifted(P1@监狱"))
        .expect("服刑期满要有一条出狱凭据");
    let last_skip_at = ev
        .iter()
        .rposition(|e| e.starts_with("StaySkip(P1,剩1)"))
        .expect("停留该跳过两格");
    assert!(
        released_at > last_skip_at,
        "出狱凭据不能早于最后一次被停留按住：{ev:?}"
    );
    assert_eq!(
        ev.get(released_at + 1).map(String::as_str),
        Some("StayLift(P1)"),
        "两张计时在同一格一起期满：{ev:?}"
    );
}

#[test]
fn two_stay_cards_on_the_same_head_do_not_add_up_to_six_turns() {
    // 两张 3 回合 = 3 回合，不是 6。这张卡买的是「那几回合他不动」，而「不动」叠两层
    // 与叠一层在屏幕上没有区别 —— 能叠出 6 就说明它其实是两件事
    let (mut app, p0, p1) = two_of_them(4, 3);
    let card = item_index(&app, "item_stay");
    deal(&mut app, p0, card.0);
    deal(&mut app, p0, card.0);
    play_card(&mut app, p0, card.0, Some(CardAim::Player(p1)));
    play_card(&mut app, p0, card.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(stay_of(&app, p1), Some(3), "两张同长的卡 = 一张");
}

#[test]
fn a_shorter_stay_does_not_cut_a_longer_one_short() {
    // 取**较大**而不是覆盖：给一个已经停 4 回合的人再打一张 3，他该停 4 回合。
    // 写成覆盖的话，惩罚卡就有了「让人早走」的副作用
    let (mut app, p0, p1) = two_of_them(4, 3);
    insert(&mut app, p1, StaySentence { turns_left: 4 });
    hold_there(&mut app, p0, p1);

    assert_eq!(stay_of(&app, p1), Some(4), "短的那张不该把长的抵掉");
}

#[test]
fn an_insulated_victim_is_not_stayed_and_both_sides_still_keep_a_trail() {
    // 与入狱、转向同一条反制形状：免罪挡的是**事**（`AfflictDomain::Stay`），
    // 挡掉了就没有这回事 —— 不留半个计数，但两边各看得见一次
    let (mut app, p0, p1) = two_of_them(4, 3);
    let insulate = item_index(&app, "item_insulate");
    make_current(&mut app, p1);
    deal(&mut app, p1, insulate.0);
    play_card(&mut app, p1, insulate.0, None);
    settle(&mut app, 2);
    make_current(&mut app, p0);

    hold_there(&mut app, p0, p1);

    assert_eq!(stay_of(&app, p1), None, "抵消了就没有停留这回事");
    assert_eq!(shield_of(&app, p1), None, "用掉的那一层要跟着收走");
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_stay挡1)".to_string()),
        "被挡掉也要留痕：{:?}",
        ledger(&app)
    );

    // 而下一张照样生效：挡掉的是那一件**事**，不是这个人以后的所有事
    hold_there(&mut app, p0, p1);
    assert_eq!(stay_of(&app, p1), Some(3), "护身只吃了一层");
}

#[test]
fn a_stay_aimed_at_himself_holds_his_next_turn_but_not_this_one() {
    // `CardTarget::Anybody` 允许选自己。打给自己的那一刻他正站在 `WaitRoll` 里，
    // 这一回合**照旧走完**（棋子照动、落格照结算）—— 计数是从下一次回合开始才扣的。
    // 谁哪天把这条写成「当场冻结本回合」，骰子已经按下去的那一步就凭空消失了
    let (mut app, p0, _p1) = two_of_them(4, 3);
    hold_there(&mut app, p0, p0);
    assert_eq!(stay_of(&app, p0), Some(3), "对自己的那一张该生效");

    play_turns(&mut app, 1);
    let ev = events(&app);
    assert!(
        ev.iter().any(|e| e.starts_with("Roll(P0,")),
        "本回合该照旧掷骰：{ev:?}"
    );
    assert_eq!(
        tile_of(&app, p0),
        7,
        "从 4 号往前走三步，停留卡没踩住这一次移动"
    );
    assert_eq!(
        stay_of(&app, p0),
        Some(3),
        "他自己刚走完的这一格不该扣它，那是下一次回合开始的事"
    );
}
