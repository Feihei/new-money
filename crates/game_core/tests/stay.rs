//! P4④ 的验收：停留卡按住的是**人**，不是他的地、他的方向、他的钱包；而且它现在
//! 是一个**选择**而不是一次自动跳过 —— 轮到他时掷骰按钮翻成「停留」，点一下才生效。
//!
//! 这条卡最容易写错的四种可能都不在卡面上：
//!
//! - 它不该被合成进 `Detention`：被拘留的人要被送进死胡同格，停留的人**还在自己踩到的
//!   那一格上**，棋子不必位移、回来的凭据（`DetentionLifted`）也不该发；
//! - 它不该再走「判 3 = 跳 2」那半条：计数是**字面**的，写 1 就停 1 个回合；
//! - 被停留按住时掷骰必须被规则层拒掉（而不只是 UI 把按钮换了字）—— 否则 AI/恶人
//!   那边的「停留」意图就成了摆设；
//! - 拘留与停留同时挂着时，拘留那几回合不消耗停留的计数（他不在牌桌上）。
//!
//! 还有一条是它与 P4 的边界：拘留与停留只差「他在不在牌桌上」，而差别落在**收租**上
//! （P4② 给 `Detention` 加了那条判据）。`a_stayed_owner_still_collects_rent...` 钉的就是
//! 「那条判据不许顺手扩到 `StaySentence`」—— 人还在场上的人，地还得收。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    CardAim, Detention, DetentionPlace, Heading, StaySentence, Term, TurnPhase,
};

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

/// `who` 被停留按掉的高水位线（按到达顺序）。
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

    assert_eq!(stay_of(&app, p1), Some(1), "卡面上写 1 回合，身上就得是 1");
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
fn a_one_turn_stay_costs_him_exactly_one_turn_and_never_auto_skips() {
    // 字面口径（P4④）：停 1 回合就是**一个**回合，不是「判 1 = 跳 0」也不是旧的跳 2。
    // 回合序 P0/P1 交替：P1 该在自己那一格上按掉一次「停留」，然后照常掷骰
    let (mut app, p0, p1) = two_of_them(4, 3);
    hold_there(&mut app, p0, p1);
    play_turns(&mut app, 6);

    assert!(
        events(&app).contains(&"StayLift(P1)".to_string()),
        "按下唯一那一格就该摘掉计时并留一句：{:?}",
        events(&app)
    );
    assert!(
        stay_skips(&app, "P1").is_empty(),
        "只剩一格时不发「还剩几格」，那是给两张/多回合卡用的：{:?}",
        events(&app)
    );
    assert_eq!(stay_of(&app, p1), None, "用完了就要消失");
    assert!(
        !events(&app).iter().any(|e| e.starts_with("Lifted(P1@")),
        "他从没进过死胡同格，不该有一条出狱凭据：{:?}",
        events(&app)
    );
    // 六回合里 P1 有三次自己的回合：被按掉一次、另两次照常掷骰
    assert_eq!(
        rolls_for(&app, "P1"),
        2,
        "被停留的那一回合没有掷骰，其余两回合照常：{:?}",
        events(&app)
    );
}

#[test]
fn rolling_is_refused_while_staying_and_the_click_closes_the_turn() {
    // 两条判据各在一层：UI 把按钮翻成「停留」，而规则层**仍然**拒掉`RequestRoll`。
    // 只做前一半的话，AI 与 P12 的恶人没有那颗按钮，`StaySentence` 就成了死锁
    let (mut app, p0, p1) = two_of_them(4, 3);
    hold_there(&mut app, p0, p1);
    play_turns(&mut app, 1);
    assert_eq!(current(&app), p1, "这一格轮到被按住的人");
    assert_eq!(phase(&app), TurnPhase::WaitRoll);

    // 像不知道规则那样硬按掷骰：什么都不该发生
    roll(&mut app, p1);
    settle(&mut app, 2);
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "回合不该被推进");
    assert_eq!(stay_of(&app, p1), Some(1), "计数也不该被扣");
    assert!(
        !events(&app).iter().any(|e| e.starts_with("Roll(P1,")),
        "规则层必须拒掉被停留者的掷骰：{:?}",
        events(&app)
    );

    // 真正的出口：按「停留」
    stay(&mut app, p1);
    settle(&mut app, 2);
    assert_eq!(
        phase(&app),
        TurnPhase::TurnEnd,
        "按下去之后这一回合就该收尾"
    );
    assert_eq!(stay_of(&app, p1), None, "1 回合的卡用一次就清空");
    assert_eq!(tile_of(&app, p1), 0, "他一步都没走");
    assert!(
        !events(&app)
            .iter()
            .any(|e| e.starts_with("Step(P1,") || e.starts_with("Arrive(P1,")),
        "停留的那一回合不该有行走：{:?}",
        events(&app)
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
    insert(&mut app, p1, StaySentence { turns_left: 2 });
    // 空转几帧：现在站在 `WaitRoll` 里的是 P0，没有人为 P1 按下「停留」，
    // 而他的计数一格都没少 —— 扣计数的那件事只发生在**他自己的**回合
    settle(&mut app, 3);
    assert_eq!(stay_of(&app, p1), Some(2), "别人的回合不消耗他的计数");

    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 2, "0 号往前走两步是 2 号格，那块地是 L1");
    assert!(
        ledger(&app).contains(&"Rent(P0->P1@L1$500)".to_string()),
        "被按住的人照样收得到租：{:?}",
        ledger(&app)
    );
    assert_eq!(cash_of(&mut app, p1), 20_500, "租金要真的进他的钱包");
    // 轮到他了，但没人按下「停留」—— 计数原地不动，这正是「选择」与「自动跳过」的差别
    assert_eq!(
        stay_of(&app, p1),
        Some(2),
        "回合开始了但按钮还没按：{:?}",
        events(&app)
    );
    stay(&mut app, p1);
    settle(&mut app, 2);
    assert_eq!(
        stay_of(&app, p1),
        Some(1),
        "他的第一格按掉之后剩 1：{:?}",
        events(&app)
    );
    assert_eq!(
        stay_skips(&app, "P1"),
        vec!["StaySkip(P1,剩1)"],
        "收租那一格与他的第一格之间不该多出一个回合：{:?}",
        events(&app)
    );
}

#[test]
fn a_jail_skip_does_not_consume_the_stay_counter() {
    // 拘留那几回合他根本不在牌桌上，没有「停留」这颗按钮可按 —— 所以停留的计数
    // 一格都不该跟着走。拘留期满那一格他才进入 WaitRoll，从那一格起才开始按。
    let (mut app, p0, p1) = two_of_them(4, 3);
    insert(
        &mut app,
        p1,
        Detention {
            place: DetentionPlace::Jail,
            term: Term::Turns(2),
        },
    );
    hold_there(&mut app, p0, p1);
    play_turns(&mut app, 4);

    let ev = events(&app);
    assert_eq!(
        ev.iter()
            .filter(|e| e.starts_with("DetainSkip(P1@监狱,"))
            .count(),
        1,
        "服刑 2 天该跳过一格：{ev:?}"
    );
    assert!(
        ev.iter().any(|e| e.starts_with("Lifted(P1@监狱")),
        "第二格该放他出来：{ev:?}"
    );
    assert!(
        ev.contains(&"StayLift(P1)".to_string()),
        "出来后的那一格他按掉了停留（1 回合）：{ev:?}"
    );
    assert_eq!(stay_of(&app, p1), None, "1 回合的停留用完就没了");
    assert_eq!(
        rolls_for(&app, "P1"),
        0,
        "被拘留那一格与按停留那一格都不掷骰：{ev:?}"
    );
}

#[test]
fn two_one_turn_stays_on_the_same_head_do_not_add_up_to_two() {
    // 两张 1 回合 = 1 回合，不是 2。这张卡买的是「这一回合他不动」，而「不动」叠两层
    // 与叠一层在屏幕上没有区别 —— 能叠出 2 就说明它其实是两件事
    let (mut app, p0, p1) = two_of_them(4, 3);
    let card = item_index(&app, "item_stay");
    deal(&mut app, p0, card.0);
    deal(&mut app, p0, card.0);
    play_card(&mut app, p0, card.0, Some(CardAim::Player(p1)));
    play_card(&mut app, p0, card.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(stay_of(&app, p1), Some(1), "两张同长的卡 = 一张");
}

#[test]
fn a_shorter_stay_does_not_cut_a_longer_one_short() {
    // 取**较大**而不是覆盖：给一个已经停 4 回合的人再打一张 1，他该停 4 回合。
    // 写成覆盖的话，惩罚卡就有了「让人早走」的副作用
    let (mut app, p0, p1) = two_of_them(4, 3);
    insert(&mut app, p1, StaySentence { turns_left: 4 });
    hold_there(&mut app, p0, p1);

    assert_eq!(stay_of(&app, p1), Some(4), "短的那张不该把长的抵掉");
}

#[test]
fn a_forgive_card_is_not_even_asked_about_a_stay() {
    // P13 定稿的免疫口径（10-25 拍板）：免罪卡只挡「被关起来」那一族，停留不在里面 ——
    // 被人按住一回合是处境变了，不是一件灾落下来。
    //
    // P13b② 之后这条口径有**两句**证据，而不是一句：门连问都不问（`gate_asking` 为空），
    // 而那张卡照样留在他手里。只断言后一句的话，「门压根没运转」与「门问过又落下」
    // 在测试里是同一个形状 —— 空跑的断言等于没断言。
    let (mut app, p0, p1) = two_of_them(4, 3);
    let insulate = item_index(&app, "item_insulate");
    make_current(&mut app, p1);
    deal(&mut app, p1, insulate.0);
    settle(&mut app, 2);
    make_current(&mut app, p0);

    hold_there(&mut app, p0, p1);

    assert_eq!(stay_of(&app, p1), Some(1), "停留照样落地：免罪卡不吃这一族");
    assert_eq!(
        gate_asking(&app),
        None,
        "门不许对着一句「被按住」问人 —— 问了就是替玩家决定这张卡值不值"
    );
    assert_eq!(
        forgive_in_hand(&app, p1),
        1,
        "没问的那一句不该白吃一张免罪卡"
    );
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_stay)".to_string()),
        "流水里不该带「挡」：{:?}",
        ledger(&app)
    );

    // 再打一次还是照样生效，而那张卡还是没被碰 —— 两边各归 [`game_core::passive`] 那张表
    hold_there(&mut app, p0, p1);
    assert_eq!(stay_of(&app, p1), Some(1), "第二次也没被挡");
    assert_eq!(forgive_in_hand(&app, p1), 1, "两张停留卡都没吃到免罪卡");
}

#[test]
fn a_stay_aimed_at_himself_uses_up_this_very_turn() {
    // `CardTarget::Anybody` 允许选自己。P4④ 拍板之后方向反过来：打给自己那一张
    // 读的是**当前这一回合**（他已经站在 `WaitRoll` 里，按钮当场翻成「停留」）；
    // 打给别人读的是他的下一回合。写成「打给自己也留到下一回合」就是白送一回合
    let (mut app, p0, _p1) = two_of_them(4, 3);
    hold_there(&mut app, p0, p0);
    assert_eq!(stay_of(&app, p0), Some(1), "对自己的那一张该生效");

    // 这一回合还没结束，但掷骰已经被拒
    roll(&mut app, p0);
    settle(&mut app, 2);
    assert!(
        !events(&app).iter().any(|e| e.starts_with("Roll(P0,")),
        "被自己的停留卡按住，本回合不能掷骰：{:?}",
        events(&app)
    );

    stay(&mut app, p0);
    settle(&mut app, 2);
    assert_eq!(stay_of(&app, p0), None, "用在这一回合上了");
    assert_eq!(tile_of(&app, p0), 4, "他从 4 号格一步没走");
}
