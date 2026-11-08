//! P16b 的验收：两段过场（掷骰翻滚 / 摆放标志物）真的把回合机串起来，
//! 单击能跳过过场而不改任何规则，表现层的全局闸把掷骰与走步一起按住。
//!
//! 每一条只碰 core 的读数（`TurnPhase` / `BeatClock` / `WalkClock` / 流水），
//! 翻滚动画、标志物落下与音效本身在 `game_app` 那一侧，这里不测像素。
//!
//! 时钟的推进方式与 `tests/walk.rs` 同一条：headless 里 `Time` 不前进，
//! 所以直接改写 `BeatClock.elapsed` / `WalkClock.elapsed`（§4.6 认可的做法）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::Messages;
use common::*;
use game_core::prelude::{
    ActiveBoard, BeatClock, Decision, DecisionChosen, GameRules, GameSession, GameSetup, ItemKind,
    Pending, PerformanceHold, RequestSkip, TileKind, TurnPhase, WalkClock,
};

/// 瞬时行走 + 指定的两段过场：本文件绝大多数用例要的是「过场之外一切当帧落定」。
fn beats(dice: f32, place: f32) -> GameRules {
    GameRules {
        dice_beat: dice,
        place_beat: place,
        ..GameRules::instant()
    }
}

/// 定步行走 + 过场：测「闸门把棋子按在半格上」时用。
fn paced(dice: f32, place: f32) -> GameRules {
    GameRules {
        walk_step_interval: 0.18,
        dice_beat: dice,
        place_beat: place,
        ..GameRules::instant()
    }
}

fn set_beat(app: &mut App, secs: f32) {
    app.world_mut().resource_mut::<BeatClock>().elapsed = secs;
}

fn hold(app: &mut App, on: bool) {
    app.world_mut().resource_mut::<PerformanceHold>().0 = on;
}

fn skip(app: &mut App, player: Entity) {
    app.world_mut()
        .resource_mut::<Messages<RequestSkip>>()
        .write(RequestSkip { player });
}

fn answer(app: &mut App, player: Entity, action: Decision) {
    app.world_mut()
        .resource_mut::<Messages<DecisionChosen>>()
        .write(DecisionChosen { player, action });
}

fn steps(app: &App) -> usize {
    events(app)
        .iter()
        .filter(|e| e.starts_with("Step("))
        .count()
}

fn rolls(app: &App) -> usize {
    events(app)
        .iter()
        .filter(|e| e.starts_with("Roll("))
        .count()
}

/// 演示盘上第一块地带的那一格（格号, 地号）。用例不写死格号：插一格的成本不该落到这里。
fn first_property(app: &App) -> (u32, u32) {
    let board = app.world().resource::<ActiveBoard>();
    board
        .def
        .tiles
        .iter()
        .find_map(|t| t.land_id.map(|land| (t.id, land)))
        .expect("演示盘上有地块")
}

#[test]
fn the_dice_beat_holds_the_walk_until_the_clock_runs_out() {
    let mut app = boot(GameSetup::default(), beats(1.0, 0.0), seed_rolling_first(3));
    let p0 = boot_to_wait_roll(&mut app);
    // 3 号格是命运格（定序第一张恰好会把人抓走），换成路面 —— 这条测的是拍子，不是卡
    retile(&mut app, 3, TileKind::Road);
    clear_occupancies(&mut app);
    clear_board_gods(&mut app);

    roll(&mut app, p0);
    app.update(); // 掷骰帧：路径已定，状态转移还没生效
    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "转移下一帧才生效（坑 12）"
    );
    assert_eq!(tile_of(&app, p0), 3, "逻辑位置仍在掷骰帧落定（§4.6）");

    app.update(); // 过场生效帧
    assert_eq!(phase(&app), TurnPhase::Rolling);
    assert_eq!(
        steps(&app),
        0,
        "骰子还在翻，一步都不许迈：{:?}",
        events(&app)
    );

    // 台词在播（全局闸）时拍子钟不走 —— 过场不许在闸门后面偷偷放完。
    // ⚠️ 这里必须把读数抬到**超过时长**：写在 0.55 的话「没结束」与「闸门根本没起作用」
    // 长得一模一样（red-check ⑤ 第一版就是这么空跑的）
    hold(&mut app, true);
    set_beat(&mut app, 2.5);
    settle(&mut app, 2);
    assert_eq!(phase(&app), TurnPhase::Rolling, "闸门按住时过场不结束");
    hold(&mut app, false);

    // 钟走满：这一帧写转移，下一帧才进 Walking
    set_beat(&mut app, 1.0);
    app.update();
    assert_eq!(phase(&app), TurnPhase::Rolling, "转移下一帧才生效");
    assert_eq!(steps(&app), 0, "拍子没走完，一步都不许迈");

    app.update(); // Walking 生效帧：interval=0 一帧走完
    assert_eq!(phase(&app), TurnPhase::Walking);
    assert_eq!(steps(&app), 3, "走满骰点：{:?}", events(&app));

    app.update(); // Landed
    assert_eq!(phase(&app), TurnPhase::Landed);
    assert_eq!(tile_of(&app, p0), 3);
}

#[test]
fn a_single_click_skips_the_dice_beat_but_changes_no_rules() {
    let mut app = boot(GameSetup::default(), beats(5.0, 0.0), seed_rolling_first(3));
    let p0 = boot_to_wait_roll(&mut app);
    retile(&mut app, 3, TileKind::Road);
    clear_occupancies(&mut app);
    clear_board_gods(&mut app);

    roll(&mut app, p0);
    app.update();
    app.update();
    assert_eq!(phase(&app), TurnPhase::Rolling);

    // 别的玩家的单击不作数（只认当前回合玩家）
    let other = roster(&mut app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| *e)
        .expect("默认阵容有第二名玩家");
    skip(&mut app, other);
    app.update();
    assert_eq!(phase(&app), TurnPhase::Rolling, "不是他的回合，跳过不生效");

    skip(&mut app, p0);
    app.update(); // skip_beat 写转移
    assert_eq!(phase(&app), TurnPhase::Rolling, "转移下一帧才生效");
    app.update(); // Walking：interval=0 一帧走完
    assert_eq!(
        steps(&app),
        3,
        "跳过只是不等动画，路照走：{:?}",
        events(&app)
    );
    assert_eq!(rolls(&app), 1, "跳过不会重掷、也不会多掷");
    assert_eq!(tile_of(&app, p0), 3);
}

/// 站到「掷 1 点正好踩到第一块无主地」的位置，返回 (app, p0, 地号)。
fn ready_to_buy(rules: GameRules) -> (App, Entity, u32) {
    let mut app = boot(GameSetup::default(), rules, seed_rolling_first(1));
    app.insert_resource(DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    clear_occupancies(&mut app);
    clear_board_gods(&mut app);
    let (tile, land) = first_property(&app);
    stand_to(&mut app, p0, tile, 1);
    (app, p0, land)
}

#[test]
fn buying_earns_a_placement_beat_after_the_marker_is_up() {
    let (mut app, p0, land) = ready_to_buy(beats(0.0, 0.6));

    roll(&mut app, p0);
    assert!(
        run_until_phase(&mut app, TurnPhase::WaitDecision, 60),
        "踩到无主地应挂买地报价，实际停在 {:?}",
        phase(&app)
    );
    assert!(
        matches!(pending_of(&app), Some((_, Pending::Buy(_)))),
        "这一格该是一份买地报价：{:?}",
        pending_of(&app)
    );

    let before_turn = app.world().resource::<GameSession>().turn_count;
    answer(&mut app, p0, Decision::Accept);
    settle(&mut app, 2); // 回答帧 + 转移帧
    assert_eq!(phase(&app), TurnPhase::Placing);
    assert_eq!(
        plot_state(&mut app, land).0,
        Some(0),
        "钱与归属在答成那一帧就办完了 —— 过场只是让人看见它"
    );
    assert_eq!(
        app.world().resource::<GameSession>().turn_count,
        before_turn,
        "摆放没过场完，回合还没交还"
    );
    assert_eq!(current(&app), p0);

    // 钟没走满：还在这一拍
    settle(&mut app, 3);
    assert_eq!(phase(&app), TurnPhase::Placing);

    // 走满：交还回合、轮转到下一位
    set_beat(&mut app, 0.6);
    settle(&mut app, 4);
    assert_eq!(
        app.world().resource::<GameSession>().turn_count,
        before_turn + 1,
        "摆放完才算这一回合过去"
    );
    assert_ne!(current(&app), p0, "摆放完就该轮到下一位");
}

#[test]
fn declining_the_offer_skips_the_placement_beat() {
    let (mut app, p0, _land) = ready_to_buy(beats(0.0, 1.0));

    roll(&mut app, p0);
    assert!(run_until_phase(&mut app, TurnPhase::WaitDecision, 60));
    answer(&mut app, p0, Decision::Decline);

    // 答「不买」之后一帧也不该停在过场：连看四帧，谁都等不到下落动画
    for i in 0..4 {
        app.update();
        assert_ne!(
            phase(&app),
            TurnPhase::Placing,
            "没有东西摆下去，就没有摆放这一拍（第 {i} 帧）"
        );
    }
    assert_eq!(
        app.world().resource::<BeatClock>().exit,
        TurnPhase::TurnEnd,
        "拍子钟没被写过：出口留在默认值"
    );
}

#[test]
fn the_performance_hold_stops_the_turn_machine_cold() {
    let mut app = boot(GameSetup::default(), paced(0.0, 0.0), seed_rolling_first(2));
    let p0 = boot_to_wait_roll(&mut app);
    retile(&mut app, 2, TileKind::Road);
    clear_occupancies(&mut app);
    clear_board_gods(&mut app);

    // ① 台词在播时，掷骰不受理
    hold(&mut app, true);
    roll(&mut app, p0);
    settle(&mut app, 3);
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "闸门按着，回合不许动");
    assert_eq!(rolls(&app), 0, "闸门按着，骰子不该响：{:?}", events(&app));

    hold(&mut app, false);
    // 闸门期间那一条请求已经过期（消息只活两帧），所以重新发一次 ——
    // 这正是「被闸门挡掉的动作不会在松闸后幽灵般地补做」的读法
    roll(&mut app, p0);
    settle(&mut app, 3);
    assert_eq!(rolls(&app), 1, "松闸之后掷骰才发生");

    // ② 走到 Walking 之后：闸门让棋子停在半格上
    assert!(
        run_until_phase(&mut app, TurnPhase::Walking, 30),
        "掷骰后该进入 Walking，实际停在 {:?}",
        phase(&app)
    );
    app.world_mut().resource_mut::<WalkClock>().elapsed = 0.18;
    app.update();
    let walked = steps(&app);
    assert_eq!(walked, 1, "第一个定步时长走一格");

    hold(&mut app, true);
    app.world_mut().resource_mut::<WalkClock>().elapsed = 0.36;
    settle(&mut app, 3);
    assert_eq!(steps(&app), walked, "闸门按住时一步也不迈");

    hold(&mut app, false);
    app.update();
    assert_eq!(
        steps(&app),
        walked + 1,
        "松闸后从断点继续（时钟自己没被偷偷推进）"
    );
}

#[test]
fn remote_dice_goes_straight_to_walking_without_the_dice_beat() {
    let mut app = boot(GameSetup::default(), beats(5.0, 0.0), seed_rolling_first(2));
    let p0 = boot_to_wait_roll(&mut app);
    retile(&mut app, 2, TileKind::Road);
    clear_occupancies(&mut app);
    clear_board_gods(&mut app);
    grant_item(&mut app, p0, ItemKind::RemoteDice, 1);

    use_remote_dice(&mut app, p0, 2);
    app.update(); // 受理帧
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "转移下一帧才生效");

    // 之后的每一帧都不许出现 Rolling —— 遥控骰子没有「摇」这个过程
    let mut seen = Vec::new();
    for _ in 0..4 {
        app.update();
        seen.push(phase(&app));
    }
    assert!(
        !seen.contains(&TurnPhase::Rolling),
        "遥控骰子不该进掷骰过场：{seen:?}"
    );
    assert_eq!(tile_of(&app, p0), 2, "步数照走");
}
