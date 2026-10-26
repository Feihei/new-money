//! App 级集成测试：真资产 + 真 `AssetServer`，走与 `game_app` 相同的装配路径。
//! 不需要窗口：`game_core` 只链接 bevy 的 `default_app` 特性子集。

mod common;

use bevy::prelude::*;
use common::*;
use game_core::CoreGamePlugin;
use game_core::prelude::{
    ActiveBoard, Bankrupt, DataHandles, Detention, GameRules, GameSeed, GameSetup, LandId, Owner,
    PLAYER_COLORS, PlayerColor, PlayerId, PlayerName, PlayerSetup, TokenPosition, TurnPhase,
    Wallet,
};

fn snapshot(app: &mut App) -> Vec<(u32, String, i64, u32)> {
    let w = app.world_mut();
    let mut q = w.query::<(&PlayerId, &PlayerName, &Wallet, &TokenPosition)>();
    let mut rows: Vec<(u32, String, i64, u32)> = q
        .iter(w)
        .map(|(id, name, cash, pos)| (id.0, name.0.clone(), cash.cash, pos.tile.0))
        .collect();
    rows.sort();
    rows
}

fn land_rows(app: &mut App) -> Vec<(u32, bool)> {
    let w = app.world_mut();
    let mut q = w.query::<(&LandId, &Owner)>();
    let mut rows: Vec<(u32, bool)> = q
        .iter(w)
        .map(|(id, owner)| (id.0, owner.0.is_none()))
        .collect();
    rows.sort();
    rows
}

/// 玩家身上「开局不该有」的标记数。
///
/// **按 `PlayerId` 收而不是全场数**（P12）：四位恶人开局就关在笼里，那是这套玩法的起点
/// 而不是跨局残留。这条用例守的是「玩家没有被莫名关着／破产」，把恶人算进来就变成
/// 一条永远为红的假警报 —— 而假警报的下一步就是有人来把断言删掉。
fn status_markers(app: &mut App) -> usize {
    let w = app.world_mut();
    let mut jq = w.query_filtered::<&Detention, With<PlayerId>>();
    let mut bq = w.query_filtered::<&Bankrupt, With<PlayerId>>();
    jq.iter(w).count() + bq.iter(w).count()
}

#[test]
fn world_boots_from_real_assets() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app), "棋盘资产没能在时限内装配成世界");

    let rows = snapshot(&mut app);
    assert_eq!(
        rows.iter()
            .map(|(id, name, cash, _)| (*id, name.clone(), *cash))
            .collect::<Vec<_>>(),
        vec![
            (0, "姬百年".to_string(), 20_000),
            (1, "范一文".to_string(), 20_000),
        ],
        "玩家阵容与现金应与 `GameSetup::default()` 一致（阵容两位出自 CHARACTERS.md §五）"
    );
    // 落位是抽的（10-26 起这张棋盘没有起点），这里只问「抽到的那一格在不在环上」——
    // 「同种子同落位」与「落位不触发结算」由下面的 `the_opening_deal_...` 那条钉
    let ring = app.world().resource::<ActiveBoard>().def.loop_len();
    assert!(
        rows.iter().all(|(_, _, _, tile)| *tile < ring),
        "开局落位必须落在主环上：{rows:?} / 环长 {ring}"
    );

    // 六块土地全部成实体，且开局无主（缺陷 #1 的前提：归属是组件，不是字符串 key 字典）
    let lands = land_rows(&mut app);
    assert_eq!(lands.len(), 6, "土地实体数量应与 prototype.json 一致");
    assert!(
        lands.iter().all(|(_, unowned)| *unowned),
        "开局所有土地都应无主: {lands:?}"
    );
    assert_eq!(
        app.world().resource::<ActiveBoard>().lands.len(),
        lands.len(),
        "ActiveBoard 索引与实体数量不符"
    );
    assert_eq!(status_markers(&mut app), 0, "开局不该有拘留/破产标记");
}

/// 开局落位（10-26 实报：4 代没有起点格，玩家开局随机落位，而**落下不触发格子事件**，
/// 只有第一次掷骰之后落到的那一格才算数）。
///
/// 三条各守一种翻法：
/// - 同一种子两次装配给出同一份落位 —— 截图核对与回放的前提；
/// - 换一种子就换一份 —— 有人把 `pick_start` 接回常量时，只有这一条会响；
/// - 落位那一帧没有任何走步/落脚凭据，而同一份前置掷一次骰之后就有一条。
///   最后这条是否定式，所以它必须带牙齿：先证明正面那一次真的发生过
///   （§八 那条纪律，本项目已经在这一点上翻过五次）。
#[test]
fn the_opening_deal_is_seeded_and_settles_nothing() {
    // 这条**不用** `boot_to_wait_roll`：那个夹具会把每人摆到 0 号格，而这里要读的正是
    // 装配自己抽出来的那一份落位
    fn deal(seed: u64) -> (App, Vec<u32>) {
        let mut app = boot(GameSetup::default(), GameRules::instant(), GameSeed(seed));
        assert!(run_until_world(&mut app), "棋盘资产没能在时限内装配成世界");
        let tiles = roster(&mut app)
            .into_iter()
            .map(|(_, _, _, tile)| tile)
            .collect();
        (app, tiles)
    }

    let (mut app, first) = deal(GameSeed::default().0);
    let (_, again) = deal(GameSeed::default().0);
    assert_eq!(first, again, "同一种子必须给出同一份落位");
    let (_, shifted) = deal(7_777);
    assert_ne!(
        first, shifted,
        "换种子就该换落位。写死一格的话这条会红 —— 而它正是这次改动最可能犯的反向错误"
    );

    let ring = app.world().resource::<ActiveBoard>().def.loop_len();
    assert!(
        first.iter().all(|t| *t < ring),
        "落位只能落在主环上：{first:?} / 环长 {ring}"
    );

    assert!(
        run_until_phase(&mut app, TurnPhase::WaitRoll, 30),
        "状态机没能进入 WaitRoll"
    );
    settle(&mut app, 2);
    let before = events(&app);
    assert!(
        !before
            .iter()
            .any(|e| e.starts_with("Step(") || e.starts_with("Arrive(")),
        "落位不该产生走步或落脚凭据（那会当场弹出买地报价、点券、命运抽卡）：{before:?}"
    );

    let p0 = current(&app);
    roll(&mut app, p0);
    settle(&mut app, 8);
    let after = events(&app);
    assert!(
        after.iter().any(|e| e.starts_with("Arrive(")),
        "掷骰之后落脚凭据确实会发出来 —— 上面那句否定式断言的存在理由：{after:?}"
    );
}

#[test]
fn world_boots_exactly_once() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    let before = (snapshot(&mut app).len(), land_rows(&mut app).len());

    // begin_game_system 每帧都跑，靠 in_state(Boot) + Option<Res<ActiveBoard>> 自守门；
    // 没有守门就会重复铺实体
    for _ in 0..10 {
        app.update();
    }
    assert_eq!(
        (snapshot(&mut app).len(), land_rows(&mut app).len()),
        before,
        "世界被重复装配了"
    );
}

#[test]
fn setup_and_rules_are_the_only_sources() {
    // 阵容与起始现金都由 resource 驱动：换值不必动任何 system（缺陷 #12 的验收）
    let setup = GameSetup {
        players: (0..3)
            .map(|i| PlayerSetup {
                name: ["甲", "乙", "丙"][i].to_string(),
                color: PLAYER_COLORS[i],
                gender: game_core::player::Gender::Male,
            })
            .collect(),
    };
    let mut app = boot(
        setup,
        GameRules {
            start_cash: 8_888,
            walk_step_interval: 0.0,
            ..Default::default()
        },
        Default::default(),
    );
    assert!(run_until_world(&mut app));

    // 阵容与起始现金都由 resource 驱动：换值不必动任何 system（缺陷 #12 的验收）。
    // 落位那一列不进断言 —— 这条测的不是位置，而它在装配期抽的那一格本来也不该被钉成 0
    //（位置那两条在 `the_opening_deal_is_seeded_and_settles_nothing`）
    assert_eq!(
        snapshot(&mut app)
            .into_iter()
            .map(|(id, name, cash, _)| (id, name, cash))
            .collect::<Vec<_>>(),
        vec![
            (0, "甲".to_string(), 8_888),
            (1, "乙".to_string(), 8_888),
            (2, "丙".to_string(), 8_888),
        ]
    );

    let w = app.world_mut();
    let mut cq = w.query::<&PlayerColor>();
    let colors: Vec<_> = cq.iter(w).map(|c| c.0).collect();
    assert_eq!(
        colors,
        vec![PLAYER_COLORS[0].0, PLAYER_COLORS[1].0, PLAYER_COLORS[2].0],
        "配色应照搬 board.gd:33-38 的顺序"
    );
}

#[test]
fn world_waits_when_board_asset_is_missing() {
    // 规则层的行为是「等」，不是「猜一个棋盘」。响的部分由 game_app 的 expect_ready 负责
    use std::time::Duration;

    let mut app = App::new();
    app.add_plugins(bevy::app::TaskPoolPlugin::default())
        .add_plugins(bevy::asset::AssetPlugin {
            file_path: asset_root().display().to_string(),
            watch_for_changes_override: Some(false),
            ..default()
        })
        .add_plugins(CoreGamePlugin)
        .add_systems(
            Startup,
            |mut commands: Commands, server: Res<AssetServer>| {
                commands.insert_resource(DataHandles {
                    board: server.load("data/boards/does_not_exist.json"),
                    ..DataHandles::load(&server)
                });
            },
        );

    for _ in 0..60 {
        app.update();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(
        !app.world().contains_resource::<ActiveBoard>(),
        "棋盘缺失时不该装配出世界"
    );
}
