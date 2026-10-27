//! P5 的验收：道路放置物、道具栏、载具与乌龟卡。
//!
//! 五条验收各有一组用例（PLAN §四 P5 的右列）：
//! 1. 放路障 → 下一个人被卡（含放置者自己）；
//! 2. 踩地雷 → 住院 3 天，放置者自己踩也炸；
//! 3. 定时炸弹附身 → 走满 38 步爆 → 3×3 全住院 5 天且那一圈建筑各降 1 级、所有权不动
//!    → 相遇时可传给别人；
//! 4. 开车撞飞恶犬（步行则被咬）；
//! 5. 身上有乌龟计时的人，掷 5 点也只前进 1 格、而 `DiceRolled.steps` 仍是 5。
//!
//! 另有 ADR-002 的三条结构义务：`place` 闸门拒绝一格第二件；开局清空/宝箱只在落脚触发；
//! 中断时不发 `PlayerArrived`、不结算脚下那格、`TokenPosition` 回到停下那一格。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    CardAim, DetentionPlace, Inventory, ItemKind, MoveCap, OccupancyAt, OccupancyKind, Term,
    TileKind, TokenPosition, TurnPhase,
};

/// 默认阵容里的另一名玩家。
fn other_of(app: &mut App, me: Entity) -> Entity {
    roster(app)
        .iter()
        .find(|(e, ..)| *e != me)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家")
}

/// 把一条路改成纯路面：验收要的是「走到第几步」，不是「这一格买了什么」。
fn pave(app: &mut App, tiles: std::ops::RangeInclusive<u32>) {
    for tile in tiles {
        retile(app, tile, TileKind::Road);
    }
}

/// 在 `player` 身上放一个已经点着的炸弹。爆心由接下来的走步决定。
fn carry_bomb(app: &mut App, player: Entity, steps_left: u32) -> Entity {
    app.world_mut()
        .spawn((
            game_core::prelude::Occupancy {
                kind: OccupancyKind::TimeBomb,
                at: OccupancyAt::Carried {
                    by: player,
                    steps_left,
                },
            },
            game_core::prelude::Gameplay,
        ))
        .id()
}

/// 背包里一共有几个道具（不看种类）。
fn item_total(app: &App, player: Entity) -> u32 {
    inventory_of(app, player).iter().map(|(_, n)| n).sum()
}

// ===== 1. 路障 =====

#[test]
fn a_roadblock_stops_the_one_who_placed_it_on_its_own_tile() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    assert_eq!(
        inventory_of(&app, p0).len(),
        1,
        "开局该送一个路障（PLAN §三）"
    );

    place_item(&mut app, p0, ItemKind::Roadblock, 1);
    settle(&mut app, 2);
    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|l| l.starts_with("Place("))
            .count(),
        1,
        "放置要留一条凭据：{:?}",
        ledger(&app)
    );
    assert!(inventory_of(&app, p0).is_empty(), "放下一个就该少一个");

    play_turns(&mut app, 1);

    let ev = events(&app);
    assert!(
        ev.contains(&"Roll(P0,5,0->5)".to_string()),
        "骰面仍是 5，`DiceRolled.to` 是没被截断的原终点：{ev:?}"
    );
    assert!(
        ev.contains(&"Walk(P0,5,0->1)".to_string()),
        "行走意图要取截断之后的终点：{ev:?}"
    );
    assert!(
        ev.contains(&"Arrive(P0,0->1)".to_string()),
        "路障格是正常到站，不是中断：{ev:?}"
    );
    assert_eq!(tile_of(&app, p0), 1, "人停在路障这一格，不是前一格");
    assert!(
        !ev.iter().any(|e| e.starts_with("Step(P0,2")),
        "被挡住之后不能再走：{ev:?}"
    );
    assert!(
        ledger(&app).contains(&"BuyOffer(P0,@0)".to_string()),
        "路障格的静态格型照常结算（这里是南京路的买地报价）：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_roadblock_stops_the_next_player_too() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    clear_occupancies(&mut app);

    place_item(&mut app, p0, ItemKind::Roadblock, 2);
    settle(&mut app, 2);
    // P0 这一回合不掷骰，直接轮到 P1 —— 骰子流的第一手仍是 seed 的 5
    make_current(&mut app, p1);
    roll(&mut app, p1);
    settle(&mut app, 4);

    assert_eq!(tile_of(&app, p1), 2, "别人放的路障照样挡人");
    let ev = events(&app);
    assert!(
        ev.contains(&"Arrive(P1,0->2)".to_string())
            && !ev.iter().any(|e| e.starts_with("Step(P1,3")),
        "P1 该停在 2 号格：{ev:?}"
    );
}

#[test]
fn one_tile_holds_one_thing_and_two_requests_in_one_frame_cannot_both_win() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    grant_item(&mut app, p0, ItemKind::Roadblock, 3);

    // 同一帧两条请求指着同一格：只有一条能成
    place_item(&mut app, p0, ItemKind::Roadblock, 4);
    place_item(&mut app, p0, ItemKind::Roadblock, 4);
    settle(&mut app, 2);

    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|l| l.starts_with("Place("))
            .count(),
        1,
        "同帧两条请求都该过了「查询里没有」那一关，闸门必须另有本帧已占格：{:?}",
        ledger(&app)
    );
    assert_eq!(
        occupancies_on(&mut app),
        vec![(4, OccupancyKind::Roadblock)],
        "一格只能有一件"
    );
    assert_eq!(item_total(&app, p0), 3, "被拒的那一件要还回来，不许白扣");

    // 第二件放另一格仍可以；放着人的格子不行；遥控骰子不是放置物
    place_item(&mut app, p0, ItemKind::Roadblock, 5);
    place_item(&mut app, p0, ItemKind::Roadblock, 0); // P0 自己站在 0 号
    place_item(&mut app, p0, ItemKind::RemoteDice, 6);
    settle(&mut app, 2);
    assert_eq!(
        occupancies_on(&mut app),
        vec![(4, OccupancyKind::Roadblock), (5, OccupancyKind::Roadblock)],
        "只有 5 号那一笔该成：{:?}",
        ledger(&app)
    );
    assert_eq!(item_total(&app, p0), 2, "第二批里只该扣掉 5 号那一件");
}

// ===== 2. 地雷 =====

#[test]
fn a_mine_sends_its_own_placer_to_the_hospital_for_three_days() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    grant_item(&mut app, p0, ItemKind::Mine, 1);
    place_item(&mut app, p0, ItemKind::Mine, 2);
    settle(&mut app, 2);
    assert_eq!(item_total(&app, p0), 1, "还剩开局那个路障");

    play_turns(&mut app, 1);

    let ev = events(&app);
    assert!(
        ev.contains(&"Halt(P0,格2,地雷)".to_string()),
        "走步要在循环里中断：{ev:?}"
    );
    assert!(
        !ev.iter().any(|e| e.starts_with("Step(P0,3")),
        "后续步数一步都不该走：{ev:?}"
    );
    assert!(
        !ev.iter().any(|e| e.starts_with("Arrive(P0,")),
        "中断不是到站：{ev:?}"
    );
    let held = detain_of(&app, p0).expect("踩雷该住院");
    assert_eq!(
        (held.place, held.term),
        (DetentionPlace::Hospital, Term::Turns(3)),
        "地雷 3 天"
    );
    assert_eq!(
        tile_of(&app, p0),
        junction_tile(&app, game_core::board::BranchKind::Hospital),
        "逻辑位置回到医院的接入点"
    );
    assert!(
        tiles_holding(&mut app, OccupancyKind::Mine).is_empty(),
        "地雷被踩掉，场上不该还留着"
    );
    assert!(
        !ev.iter().any(|e| e == "BuyOffer(P0,@1)"),
        "地雷格不结算静态格型：{ev:?}"
    );
}

// ===== 3. 定时炸弹 =====

#[test]
fn a_time_bomb_is_picked_up_and_the_pickup_step_counts_as_the_first() {
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::TimeBomb, 2);

    play_turns(&mut app, 1);

    let bombs = bombs(&mut app);
    assert_eq!(bombs.len(), 1);
    assert_eq!(
        bombs[0].1,
        OccupancyAt::Carried {
            by: p0,
            steps_left: game_core::prelude::BOMB_STEPS - 1
        },
        "拾取那一步算第 1 步（本阶段钉死的口径）"
    );
    assert_eq!(tile_of(&app, p0), 2, "拾取不打断这一趟行走");
    assert!(detain_of(&app, p0).is_none(), "刚捡起来还没爆");
}

#[test]
fn a_carried_bomb_moves_to_the_player_it_meets_and_keeps_its_counter() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    clear_occupancies(&mut app);
    pave(&mut app, 5..=5); // 终点是 5 号命运格：铺成路面，抽牌挪人不是本条要测的东西
    spawn_occupancy(&mut app, OccupancyKind::TimeBomb, 1);
    place(&mut app, p1, 3);

    play_turns(&mut app, 1);

    let bombs = bombs(&mut app);
    assert_eq!(bombs.len(), 1);
    let OccupancyAt::Carried { by, steps_left } = bombs[0].1 else {
        panic!("相遇之后炸弹该在一个人身上");
    };
    assert_eq!(by, p1, "经过另一名玩家所在格即转让");
    assert_eq!(
        steps_left,
        game_core::prelude::BOMB_STEPS - 3,
        "换手不重置步数：拾取那一步 + 后两步各扣一格"
    );
    assert!(
        ledger(&app).iter().any(|l| l == "Bomb(P0→P1@格3,剩35)"),
        "转让要留凭据：{:?}",
        ledger(&app)
    );
    assert!(
        detain_of(&app, p0).is_none() && detain_of(&app, p1).is_none(),
        "转让不是爆炸"
    );
}

#[test]
fn a_bomb_blast_hurts_the_ring_and_degrades_buildings_without_seizing_them() {
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    clear_occupancies(&mut app);
    // 爆心会落在 1 号格；3×3 覆盖南京路 (2,0) 与汉口路 (3,0)
    own(&mut app, 0, Some(0), 2, false);
    own(&mut app, 1, Some(1), 3, false);
    place(&mut app, p1, 2);
    set_mount(&mut app, p0, game_core::prelude::Mount::Car);
    carry_bomb(&mut app, p0, 1);

    // 用一颗骰子：seed 的第一手是 2，走到 1 号格就爆
    roll_with_dice(&mut app, p0, 1);
    settle(&mut app, 3);

    let ev = events(&app);
    assert!(
        ev.contains(&"Halt(P0,格1,定时炸弹)".to_string()),
        "第 1 步就爆：{ev:?}"
    );
    assert!(
        !ev.iter().any(|e| e.starts_with("Step(P0,2")),
        "爆炸之后剩余步数一步都不走：{ev:?}"
    );
    for player in [p0, p1] {
        let held = detain_of(&app, player).expect("3×3 里的人都要住院");
        assert_eq!(
            (held.place, held.term),
            (DetentionPlace::Hospital, Term::Turns(5))
        );
    }
    // 一圈建筑各降 1 级，所有权一动不动
    assert_eq!(
        plot_state(&mut app, 0),
        (Some(0), 1, false),
        "降层不夺地：南京路仍是 P0 的，2→1"
    );
    assert_eq!(
        plot_state(&mut app, 1),
        (Some(1), 2, false),
        "汉口路 3→2，还是 P1 的"
    );
    assert!(
        ledger(&app).contains(&"Mount(P0,步行)".to_string()),
        "炸弹把车炸了：{:?}",
        ledger(&app)
    );
    assert!(bombs(&mut app).is_empty(), "炸完的实体要消失");
}

#[test]
fn a_halt_does_not_trigger_a_landing_only_chest_underfoot() {
    // ADR-002 §7 的落脚口径：宝箱读 `PlayerArrived.to`，中断时没有 `PlayerArrived`，
    // 所以「脚下那一格」的宝箱不该被打开
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Chest, 2);
    carry_bomb(&mut app, p0, 1);

    play_turns(&mut app, 1);

    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Chest(")),
        "中断的那一格不该触发落脚效果：{:?}",
        ledger(&app)
    );
    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::Chest),
        vec![2],
        "宝箱还在原地"
    );
}

// ===== 4. 恶犬 =====

#[test]
fn a_dog_bites_a_walker_for_three_days_and_stays_on_the_road() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Dog, 3);

    play_turns(&mut app, 1);

    assert!(
        events(&app).contains(&"Halt(P0,格3,恶犬)".to_string()),
        "{:?}",
        events(&app)
    );
    let held = detain_of(&app, p0).expect("被咬该住院");
    assert_eq!(
        (held.place, held.term),
        (DetentionPlace::Hospital, Term::Turns(3))
    );
    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::Dog),
        vec![3],
        "步行的人被咬，狗还在"
    );
}

#[test]
fn a_car_runs_the_dog_over_and_keeps_going() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    pave(&mut app, 5..=5); // 终点是 5 号命运格：铺成路面，抽牌挪人不是本条要测的东西
    spawn_occupancy(&mut app, OccupancyKind::Dog, 2);
    set_mount(&mut app, p0, game_core::prelude::Mount::Car);

    roll_with_dice(&mut app, p0, 1); // 第一颗骰仍是 seed 的 5
    settle(&mut app, 6);
    play_turns(&mut app, 1);

    assert!(
        !events(&app).iter().any(|e| e.starts_with("Halt(P0,")),
        "有车的人不该被狗拦住：{:?}",
        events(&app)
    );
    assert_eq!(tile_of(&app, p0), 5, "撞飞之后照常走完");
    assert!(
        tiles_holding(&mut app, OccupancyKind::Dog).is_empty(),
        "狗被撞飞了"
    );
    assert!(detain_of(&app, p0).is_none());
    assert_eq!(mount_of(&app, p0), game_core::prelude::Mount::Car);
}

// ===== 5. 乌龟卡与骰子颗数 =====

#[test]
fn a_slowed_player_rolls_five_but_moves_only_one() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    pave(&mut app, 1..=6);
    insert(&mut app, p0, MoveCap { turns_left: 3 });

    play_turns(&mut app, 1);

    let ev = events(&app);
    assert!(
        ev.contains(&"Roll(P0,5,0->5)".to_string()) && ev.contains(&"Walk(P0,5,0->1)".to_string()),
        "骰面照记 5，实际只走 1：{ev:?}"
    );
    assert_eq!(tile_of(&app, p0), 1);
    assert_eq!(move_cap_of(&app, p0), Some(2), "影响一个回合扣一格");
    assert!(
        ledger(&app).contains(&"Cap(P0,5→1,剩2)".to_string()),
        "{:?}",
        ledger(&app)
    );
}

#[test]
fn the_turtle_counter_lasts_three_rolls_and_then_disappears() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    pave(&mut app, 1..=6);
    insert(&mut app, p0, MoveCap { turns_left: 3 });

    play_turns(&mut app, 6);

    assert_eq!(move_cap_of(&app, p0), None, "三个掷骰回合之后就没了");
    let caps: Vec<String> = ledger(&app)
        .into_iter()
        .filter(|l| l.starts_with("Cap(P0,"))
        .collect();
    assert_eq!(caps.len(), 3, "只该影响三次掷骰：{caps:?}");
    assert!(
        caps.iter().any(|l| l.contains("剩2"))
            && caps.iter().any(|l| l.contains("剩1"))
            && caps.iter().any(|l| l.contains("剩0")),
        "计数要逐格递减：{caps:?}"
    );
}

#[test]
fn the_turtle_card_stacks_by_max_and_is_not_shortened_by_a_second_one() {
    let (mut app, p0) = ready_on(4, 3, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    clear_occupancies(&mut app);

    let turtle = item_index(&app, "item_turtle");
    deal(&mut app, p0, turtle.0);
    play_card(&mut app, p0, turtle.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(move_cap_of(&app, p1), Some(3), "乌龟卡写 3 就挂 3");
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_turtle)".to_string()),
        "{:?}",
        ledger(&app)
    );

    // 再打一张短的不会把长的抵掉
    insert(&mut app, p1, MoveCap { turns_left: 5 });
    deal(&mut app, p0, turtle.0);
    play_card(&mut app, p0, turtle.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(move_cap_of(&app, p1), Some(5), "短的不覆盖长的");
}

#[test]
fn a_forgive_card_is_not_even_asked_about_a_turtle() {
    // P13 定稿的免疫口径（10-25 拍板）：免罪卡只挡「被关起来」那一族。走不远是这一趟
    // 的处境变了，不是一件灾落在他头上 —— 所以乌龟照挂，而玩家那张卡**一张都不该少**。
    // 两句证据：门连问都不问（`gate_asking` 为空）+ 卡还在手里。只断言后一句的话，
    // 「门没运转」与「门问过又落下」在测试里是同一个形状
    let (mut app, p0) = ready_on(4, 3, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    clear_occupancies(&mut app);

    let insulate = item_index(&app, "item_insulate");
    make_current(&mut app, p1);
    deal(&mut app, p1, insulate.0);
    settle(&mut app, 2);
    make_current(&mut app, p0);

    let turtle = item_index(&app, "item_turtle");
    deal(&mut app, p0, turtle.0);
    play_card(&mut app, p0, turtle.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);

    assert_eq!(
        move_cap_of(&app, p1),
        Some(3),
        "乌龟照样生效：免罪卡不吃这一族"
    );
    assert_eq!(
        gate_asking(&app),
        None,
        "门不许对着一句「走不远」问人 —— 问了就是替玩家决定这张卡值不值"
    );
    assert_eq!(
        forgive_in_hand(&app, p1),
        1,
        "没问的那一句不该白吃一张免罪卡"
    );
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_turtle)".to_string()),
        "流水里不该带「挡」：{:?}",
        ledger(&app)
    );
    assert!(
        !ledger(&app).iter().any(|l| l.contains("挡")),
        "没挡住却写了挡，等于凭空造一次免疫：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_car_rolls_three_dice_and_a_foot_walker_rolls_one() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    pave(&mut app, 1..=12);
    set_mount(&mut app, p0, game_core::prelude::Mount::Car);

    let seed = app.world().resource::<game_core::prelude::GameSeed>().0;
    let mut rng = game_core::prelude::Rng::new(seed);
    let want: u32 = (0..3).map(|_| rng.roll_die()).sum();

    roll_with_dice(&mut app, p0, 0); // 0 = 载具上限
    settle(&mut app, 6);
    play_turns(&mut app, 1);

    assert_eq!(
        tile_of(&app, p0),
        want % 13,
        "汽车该摇 3 颗骰子，合计 {want}"
    );
    assert!(
        events(&app)
            .iter()
            .any(|e| e.starts_with(&format!("Roll(P0,{want},0->"))),
        "{:?}",
        events(&app)
    );
}

#[test]
fn remote_dice_take_exactly_the_chosen_steps_and_are_spent() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    pave(&mut app, 1..=6);
    grant_item(&mut app, p0, ItemKind::RemoteDice, 1);

    use_remote_dice(&mut app, p0, 4);
    settle(&mut app, 2);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 4, "遥控骰子走恰好 4 步");
    assert!(
        inventory_of(&app, p0)
            .iter()
            .all(|(item, _)| *item != ItemKind::RemoteDice),
        "用掉就少一个"
    );
    assert!(
        ledger(&app).contains(&"Remote(P0,4步)".to_string()),
        "{:?}",
        ledger(&app)
    );
}

#[test]
fn remote_dice_are_still_capped_by_the_turtle() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    pave(&mut app, 1..=6);
    grant_item(&mut app, p0, ItemKind::RemoteDice, 1);
    insert(&mut app, p0, MoveCap { turns_left: 3 });

    use_remote_dice(&mut app, p0, 4);
    settle(&mut app, 2);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 1, "夹子对遥控骰子同样生效");
    assert!(
        ledger(&app).contains(&"Cap(P0,4→1,剩2)".to_string()),
        "{:?}",
        ledger(&app)
    );
}

// ===== 宝箱与清场 =====

#[test]
fn a_chest_pays_only_on_landing_not_when_walking_past() {
    let (mut app, p0) = ready_on(0, 5, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Chest, 1);
    let before = item_total(&app, p0);

    play_turns(&mut app, 1);

    assert_eq!(item_total(&app, p0), before, "路过不给");
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Chest(")),
        "{:?}",
        ledger(&app)
    );
    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::Chest),
        vec![1],
        "宝箱还在"
    );
}

#[test]
fn a_chest_on_the_destination_grants_an_item_and_disappears() {
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Chest, 2);
    let before = item_total(&app, p0);

    play_turns(&mut app, 1);

    assert_eq!(item_total(&app, p0), before + 1, "落脚才开");
    assert!(
        ledger(&app).iter().any(|l| l.starts_with("Chest(P0@格2:")),
        "{:?}",
        ledger(&app)
    );
    assert!(
        tiles_holding(&mut app, OccupancyKind::Chest).is_empty(),
        "开过的宝箱要消失"
    );
}

#[test]
fn the_starting_chests_are_two_and_all_on_the_ring() {
    let mut app = boot_default();
    boot_to_wait_roll(&mut app);
    let rows = occupancies_on(&mut app);
    let mut chests: Vec<u32> = rows
        .iter()
        .filter(|(_, kind)| *kind == OccupancyKind::Chest)
        .map(|(tile, _)| *tile)
        .collect();
    chests.sort();
    assert_eq!(
        chests.len(),
        game_core::prelude::STARTING_CHESTS,
        "开局该有两只宝箱：{rows:?}"
    );
    let unique: std::collections::HashSet<u32> = chests.iter().copied().collect();
    assert_eq!(
        unique.len(),
        chests.len(),
        "两只宝箱挤在同一格 —— 一格一件没挡住开局的两次刷出：{chests:?}"
    );
    // 「开局路上只有宝箱」这句在 P6 之后不再成立：装配那一步除了宝箱还放满了神。
    // 这里改成各数各的 —— 一格一件由两条一起占住同一批格子这件事才是真风险，
    // 而它由 `the_board_starts_with_the_complement_and_no_rival_pair` 与下面那条
    // 不重叠断言守着。
    let gods: usize = rows
        .iter()
        .filter(|(_, kind)| matches!(kind, OccupancyKind::God(_)))
        .count();
    assert_eq!(
        gods,
        game_core::prelude::STARTING_GODS,
        "开局路上也该放满这几位神：{rows:?}"
    );
    let len = app
        .world()
        .resource::<game_core::prelude::ActiveBoard>()
        .def
        .loop_len();
    assert!(
        rows.iter().all(|(tile, _)| *tile < len),
        "路上的东西只能落在主环上：{rows:?}"
    );
    // 10-26 之前这里还有一条「谁都不许落在 0 号格」，理由是「起点没人会落脚，撒在那儿等于
    // 少撒一个」。起点整格删掉之后那条豁免没有了对象，而它继续留着会变成一颗**偶发红**的
    // 断言 —— 0 号现在是一条普通路，抽到它的概率与其他格一样
}

#[test]
fn inventory_starts_with_a_roadblock_and_never_leaks_into_the_wallet() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let inv: &Inventory = app.world().entity(p0).get::<Inventory>().unwrap();
    assert_eq!(inv.count(ItemKind::Roadblock), 1);
    // 道具不是钱：拿到 / 用掉路障都不该动现金与点券
    let cash = cash_of(&mut app, p0);
    assert_eq!(cash, 20_000);
    assert_eq!(points_of(&app, p0), 0);
    assert_eq!(
        app.world()
            .entity(p0)
            .get::<TokenPosition>()
            .unwrap()
            .tile
            .0,
        0
    );
    assert_eq!(phase(&app), TurnPhase::WaitRoll);
}
