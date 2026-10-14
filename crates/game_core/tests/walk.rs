//! B2c 定步时行走的 App 级测试。
//!
//! 覆盖 §十 B2c 行的三条验收：
//! 1. 绕回起点经过 —— 主环步进 `i → (i+1)%N`，起点是「路过」不是「停驻」；
//! 2. 分支死路不进入主环 —— 行走路径恒为主环下标，途经挂分支的接入点不误入死胡同；
//! 3. `walk_step_interval = 0` 时与 B2b 的瞬时落格行为等价 —— 一帧走完，不依赖时钟。
//!
//! 另有 interval > 0 时「每个定步时长恰好一格」的节奏测试（直接改写 `WalkClock`，§4.6 认可）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    GameRules, GameSeed, GameSetup, Rng, TileIndex, TokenPosition, TurnPhase, WalkClock,
};

/// 找一颗「第一掷恰好是 face」的种子，让测试不依赖对默认种子点数的记忆。
fn seed_with_first_roll(face: u32) -> GameSeed {
    for s in 0..10_000u64 {
        if Rng::new(s).roll_die() == face {
            return GameSeed(s);
        }
    }
    panic!("一万颗种子里居然没掷出 {face}，骰子分布可疑");
}

/// 把玩家的逻辑位置搬到任意主环格（`WaitRoll` 态没有系统会写它，直接改安全）。
fn place(app: &mut App, player: Entity, tile: u32) {
    app.world_mut()
        .entity_mut(player)
        .get_mut::<TokenPosition>()
        .expect("玩家应有位置")
        .tile = TileIndex(tile);
}

/// 从流水里抽某位玩家的 Step 序列（踏入的格号，按行走顺序）。
fn steps_of(ev: &[String], who: &str) -> Vec<u32> {
    let prefix = format!("Step({who},");
    ev.iter()
        .filter_map(|e| e.strip_prefix(&prefix))
        .map(|s| {
            s.trim_end_matches(')')
                .parse()
                .unwrap_or_else(|_| panic!("流水里的 Step 不合法：{s}（整条流水 {ev:?}）"))
        })
        .collect()
}

fn set_clock(app: &mut App, secs: f32) {
    app.world_mut().resource_mut::<WalkClock>().elapsed = secs;
}

#[test]
fn walking_wraps_past_the_start_tile() {
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_with_first_roll(2),
    );
    let p0 = boot_to_wait_roll(&mut app);
    place(&mut app, p0, 11);

    play_turns(&mut app, 1);

    let ev = events(&app);
    assert!(
        ev.contains(&"Roll(P0,2,11->1)".to_string())
            && ev.contains(&"Walk(P0,2,11->1)".to_string()),
        "掷骰与行走意图都该带上回绕后的终点：{ev:?}"
    );
    assert_eq!(
        steps_of(&ev, "P0"),
        vec![0, 1],
        "从 11 号走 2 步：先踏过 0 号起点，再落到 1 号 —— 「绕回起点经过」：{ev:?}"
    );
    assert!(
        ev.contains(&"Arrive(P0,11->1)".to_string()),
        "到站消息要带完整行程：{ev:?}"
    );
    assert_eq!(tile_of(&app, p0), 1);
}

#[test]
fn walking_crosses_a_branch_junction_without_diving_in() {
    // prototype 的 2 号格是监狱分支接入点（见 board.rs 的 branches_expose_junction_and_dead_end）。
    // 从 1 号走 3 步必须恰好踏 2→3→4：分支死胡同不顶掉任何一步，也不会被当成环上一格绕回主环
    // （两个方向一起挡住 —— 死胡同格没有 TileIndex，结构上进不了 Step 流水）。
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_with_first_roll(3),
    );
    let p0 = boot_to_wait_roll(&mut app);
    place(&mut app, p0, 1);

    play_turns(&mut app, 1);

    let ev = events(&app);
    assert_eq!(
        steps_of(&ev, "P0"),
        vec![2, 3, 4],
        "穿过接入点应继续沿主环走，步数不因分支而增减：{ev:?}"
    );
    assert!(
        ev.contains(&"Roll(P0,3,1->4)".to_string()) && ev.contains(&"Arrive(P0,1->4)".to_string()),
        "行程两端都应落在主环：{ev:?}"
    );
    assert_eq!(tile_of(&app, p0), 4);
}

#[test]
fn zero_interval_arrives_in_one_frame_like_b2b() {
    // interval=0：掷骰帧落定逻辑位置，进入 Walking 的那一帧一步走完 ——
    // 与 B2b 的瞬时落格逐字等价（既有 turn_flow 用例全绿也依赖这一点）。
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_with_first_roll(2),
    );
    let p0 = boot_to_wait_roll(&mut app);

    roll(&mut app, p0);
    app.update(); // 掷骰帧

    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "状态转移要到下一帧才生效（坑 12）"
    );
    assert_eq!(
        tile_of(&app, p0),
        2,
        "逻辑位置应在掷骰帧就落定（§4.5 的表：TokenPosition 由 roll_dice 写）"
    );
    assert!(
        !events(&app).iter().any(|e| e.starts_with("Step(")),
        "掷骰帧不该已经迈步：{:?}",
        events(&app)
    );

    app.update(); // Walking 生效帧：一帧走完

    assert_eq!(phase(&app), TurnPhase::Walking);
    assert_eq!(
        steps_of(&events(&app), "P0"),
        vec![1, 2],
        "interval=0 应在这一帧补齐全部步数"
    );
    assert!(
        events(&app).contains(&"Arrive(P0,0->2)".to_string()),
        "同帧到站：{:?}",
        events(&app)
    );

    app.update(); // → Landed
    assert_eq!(phase(&app), TurnPhase::Landed);
    let clock = app.world().resource::<WalkClock>().elapsed;
    assert!(
        clock < 0.18,
        "到站不该依赖时钟推进（实际累计 {clock}s）—— 否则就不是「跳过动画」了"
    );
}

#[test]
fn paced_walking_steps_once_per_interval() {
    // interval=0.18：每累计一个定步时长恰好一格。headless 里虚拟时间的真实增量
    // 是微秒级的噪声，所以测试直接改写 WalkClock（§4.6 认可的做法）来精确控拍。
    let mut app = boot(
        GameSetup::default(),
        GameRules {
            start_cash: 20_000,
            walk_step_interval: 0.18,
        },
        seed_with_first_roll(3),
    );
    let p0 = boot_to_wait_roll(&mut app);

    roll(&mut app, p0);
    app.update(); // 掷骰帧：定路径、清零时钟

    let stepped = |app: &App| steps_of(&events(app), "P0").len();
    set_clock(&mut app, 0.0);
    app.update(); // Walking 生效帧
    assert_eq!(phase(&app), TurnPhase::Walking);
    assert_eq!(stepped(&app), 0, "时钟没走就不该迈步");

    set_clock(&mut app, 0.18);
    app.update();
    assert_eq!(stepped(&app), 1, "每过一个 interval 恰好一格");

    set_clock(&mut app, 0.36);
    app.update();
    assert_eq!(stepped(&app), 2);

    set_clock(&mut app, 0.54);
    app.update();
    assert_eq!(stepped(&app), 3, "走满骰点即到站");
    assert!(
        events(&app).contains(&"Arrive(P0,0->3)".to_string()),
        "到站消息：{:?}",
        events(&app)
    );

    // 到站后时钟再大也不会多走：队列已空，状态已离开 Walking
    set_clock(&mut app, 9.99);
    app.update();
    assert_eq!(stepped(&app), 3);
    assert_eq!(phase(&app), TurnPhase::Landed);
    assert_eq!(tile_of(&app, p0), 3);
}
