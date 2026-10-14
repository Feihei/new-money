//! 回合状态机与骰子的 App 级测试。
//!
//! 断言方式是 §八 定的模板：Godot 的「信号发过没有」改成"把消息转存进 `Observed` resource
//! 后比对整条流水"，于是回合顺序、每回合恰好一条 `TurnStarted`（缺陷 #2 的回归）
//! 这类性质都能一次看住。

mod common;

use bevy::app::App;
use common::*;
use game_core::prelude::{
    Bankrupt, GameRules, GameSeed, GameSession, GameSetup, JailSentence, PLAYER_COLORS,
    PlayerSetup, Rng, TurnPhase,
};

/// 独立掷 `n` 次，用来核对「对局里的点数确实只来自种子」。
fn rolls_for(seed: GameSeed, n: usize) -> Vec<u32> {
    let mut rng = Rng::new(seed.0);
    (0..n).map(|_| rng.roll_die()).collect()
}

fn trio() -> GameSetup {
    GameSetup {
        players: ["甲", "乙", "丙"]
            .iter()
            .enumerate()
            .map(|(i, name)| PlayerSetup {
                name: (*name).to_string(),
                color: PLAYER_COLORS[i],
            })
            .collect(),
    }
}

fn started_log(app: &App) -> Vec<String> {
    events(app)
        .into_iter()
        .filter(|e| e.starts_with("TurnStarted("))
        .collect()
}

/// 缺陷 #2 的回归网：`TurnStarted(X)` 之后必须先见到 `TurnEnd(X)` 才能再开任何回合。
/// 服刑跳过也走这条（`TurnStarted → JailSkip → TurnEnd`），所以同样被覆盖。
fn assert_turns_balanced(app: &App) {
    let ev = events(app);
    let mut open: Option<&str> = None;
    for e in &ev {
        if let Some(p) = e
            .strip_prefix("TurnStarted(")
            .map(|s| s.trim_end_matches(')'))
        {
            assert!(open.is_none(), "回合 {:?} 未结束就又开了 {p}：{ev:?}", open);
            open = Some(p);
        } else if let Some(p) = e.strip_prefix("TurnEnd(").map(|s| s.trim_end_matches(')')) {
            assert_eq!(open, Some(p), "TurnEnd({p}) 没有对应的 TurnStarted：{ev:?}");
            open = None;
        }
    }
}

#[test]
fn one_turn_cycles_the_fsm() {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    assert_eq!(
        events(&app),
        vec!["TurnStarted(P0)".to_string()],
        "每回合只该有一条 TurnStarted（缺陷 #2）"
    );

    let steps = rolls_for(GameSeed::default(), 1)[0];
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), steps, "落格点数应等于种子掷出的点数");
    // B2c 起，掷骰后的移动是一条逐格时间线：Roll 定意图 → Walk 开始 →
    // 每格一条 Step → Arrive 到站。interval=0 时全部压在一帧内，流水仍确定。
    let mut expected = vec![
        "TurnStarted(P0)".to_string(),
        format!("Roll(P0,{steps},0->{steps})"),
        format!("Walk(P0,{steps},0->{steps})"),
    ];
    expected.extend((1..=steps).map(|k| format!("Step(P0,{k})")));
    expected.push(format!("Arrive(P0,0->{steps})"));
    expected.push("TurnEnd(P0)".to_string());
    expected.push("TurnStarted(P1)".to_string());
    assert_eq!(events(&app), expected, "整条回合流水的顺序与内容都必须确定");
    assert_eq!(app.world().resource::<GameSession>().turn_count, 1);
    assert_eq!(phase(&app), TurnPhase::WaitRoll);
    assert_eq!(current(&app), roster(&mut app)[1].0, "该轮到 P1");
}

#[test]
fn same_seed_replays_the_same_match() {
    let play = |seed: GameSeed| {
        let mut app = boot(GameSetup::default(), GameRules::instant(), seed);
        assert!(run_until_world(&mut app));
        play_turns(&mut app, 6);
        events(&app)
            .into_iter()
            .filter(|e| e.starts_with("Roll("))
            .collect::<Vec<_>>()
    };

    let a = play(GameSeed(1234));
    assert_eq!(a.len(), 6, "六回合应恰好六次掷骰：{a:?}");
    assert_eq!(a, play(GameSeed(1234)), "同种子必须复现同一条掷骰序列");
    assert_ne!(
        a,
        play(GameSeed(4321)),
        "换种子却得到相同序列，说明骰子没读种子"
    );
}

#[test]
fn requests_from_somebody_else_are_ignored() {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = roster(&mut app)[1].0;

    roll(&mut app, p1);
    settle(&mut app, 12);

    assert_eq!(phase(&app), TurnPhase::WaitRoll, "别人的请求不该推进阶段");
    assert_eq!(current(&app), p0);
    assert!(
        !events(&app).iter().any(|e| e.starts_with("Roll(")),
        "非当前玩家的请求被接受了: {:?}",
        events(&app)
    );
}

#[test]
fn bankrupt_players_are_skipped() {
    let mut app = boot(trio(), GameRules::instant(), GameSeed::default());
    boot_to_wait_roll(&mut app);
    let p2 = roster(&mut app)[2].0;
    insert(&mut app, p2, Bankrupt);

    play_turns(&mut app, 2);

    assert_eq!(
        started_log(&app),
        vec![
            "TurnStarted(P0)".to_string(),
            "TurnStarted(P1)".to_string(),
            "TurnStarted(P0)".to_string()
        ],
        "破产的 P2 应被整轮跳过：{:?}",
        events(&app)
    );
    assert_turns_balanced(&app);
}

#[test]
fn jail_skips_two_turns_then_releases() {
    // 判 3 格：跳过 2 个回合，第 3 次回合开始当场出狱并可掷骰（player_data.gd:72-80 的规则）
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let p1 = roster(&mut app)[1].0;
    insert(&mut app, p1, JailSentence { turns_left: 3 });

    // 五格 = P0 掷 / P1 跳过 / P0 掷 / P1 跳过 / P0 掷，之后轮到 P1（第三格 → 出狱）
    play_turns(&mut app, 5);

    let ev = events(&app);
    assert_eq!(
        ev.iter().filter(|e| e.starts_with("JailSkip(P1,")).count(),
        2,
        "应恰好跳过两格：{ev:?}"
    );
    assert!(
        ev.contains(&"JailSkip(P1,剩2)".to_string())
            && ev.contains(&"JailSkip(P1,剩1)".to_string()),
        "剩余格数应逐格递减：{ev:?}"
    );
    assert!(
        ev.iter().any(|e| e.starts_with("Released(P1@")),
        "第三格应出狱：{ev:?}"
    );
    assert_eq!(phase(&app), TurnPhase::WaitRoll);
    assert_eq!(current(&app), p1, "出狱后该轮到 P1 掷骰");
    assert!(
        app.world().entity(p1).get::<JailSentence>().is_none(),
        "出狱后服刑组件必须消失"
    );
    assert_eq!(tile_of(&app, p1), 0, "服刑期间不该移动");
    assert_turns_balanced(&app);
}

#[test]
fn game_over_when_one_player_remains() {
    let mut app = boot(trio(), GameRules::instant(), GameSeed::default());
    boot_to_wait_roll(&mut app);
    let rows = roster(&mut app);
    for i in [1, 2] {
        insert(&mut app, rows[i].0, Bankrupt);
    }

    let cur = current(&app);
    roll(&mut app, cur);
    assert!(
        run_until_phase(&mut app, TurnPhase::Over, 60),
        "只剩一人应进 Over，实际 {:?}",
        phase(&app)
    );
    settle(&mut app, 2);

    assert!(
        events(&app).contains(&"GameOver(P0)".to_string()),
        "终局消息要带胜者：{:?}",
        events(&app)
    );
    assert_eq!(app.world().resource::<GameSession>().turn_count, 1);
}

#[test]
fn empty_roster_ends_the_match_instead_of_hanging() {
    let mut app = boot(
        GameSetup { players: vec![] },
        GameRules::instant(),
        GameSeed::default(),
    );
    assert!(run_until_world(&mut app));
    assert!(
        run_until_phase(&mut app, TurnPhase::Over, 30),
        "空阵容应立即终局，实际 {:?}",
        phase(&app)
    );
    assert!(events(&app).contains(&"GameOver(none)".to_string()));
}

#[test]
fn twenty_turns_run_without_panic() {
    // 长跑守状态机本身。B3 起「回合数 == 掷骰数」不再恒成立：抽到入狱卡的那一回合
    // 只有 `JailSkip` 没有 `Roll`，所以不变量改成「每回合要么掷骰要么服刑跳过」。
    let mut app = boot(trio(), GameRules::instant(), GameSeed(777));
    assert!(run_until_world(&mut app));

    play_turns(&mut app, 20);

    let ev = events(&app);
    let turns = app.world().resource::<GameSession>().turn_count;
    let rolls = ev.iter().filter(|e| e.starts_with("Roll(")).count();
    let skipped = ev.iter().filter(|e| e.starts_with("JailSkip(")).count();
    let ended = ev.iter().filter(|e| e.starts_with("TurnEnd(")).count();
    assert_eq!(turns, 20, "回合计数该正好走完 20 格");
    assert_eq!(
        rolls + skipped,
        turns as usize,
        "{rolls} 次掷骰 + {skipped} 次服刑跳过 应等于 {turns} 个回合：{ev:?}"
    );
    assert_eq!(ended, turns as usize, "每个回合都要有一条 TurnEnd");
    assert_turns_balanced(&app);
}
