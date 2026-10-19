//! App 级集成测试：真资产 + 真 `AssetServer`，走与 `game_app` 相同的装配路径。
//! 不需要窗口：`game_core` 只链接 bevy 的 `default_app` 特性子集。

mod common;

use bevy::prelude::*;
use common::*;
use game_core::CoreGamePlugin;
use game_core::prelude::{
    ActiveBoard, Bankrupt, DataHandles, Detention, GameRules, GameSetup, LandId, Owner,
    PLAYER_COLORS, PlayerColor, PlayerId, PlayerName, PlayerSetup, START_TILE, TokenPosition,
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

fn status_markers(app: &mut App) -> usize {
    let w = app.world_mut();
    let mut jq = w.query::<&Detention>();
    let mut bq = w.query::<&Bankrupt>();
    jq.iter(w).count() + bq.iter(w).count()
}

#[test]
fn world_boots_from_real_assets() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app), "棋盘资产没能在时限内装配成世界");

    let start = START_TILE.0;
    assert_eq!(
        snapshot(&mut app),
        vec![
            (0, "阿土伯".to_string(), 20_000, start),
            (1, "孙小美".to_string(), 20_000, start),
        ],
        "玩家阵容/现金/起始格应与 Godot 端一致"
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

    assert_eq!(
        snapshot(&mut app),
        vec![
            (0, "甲".to_string(), 8_888, 0),
            (1, "乙".to_string(), 8_888, 0),
            (2, "丙".to_string(), 8_888, 0),
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
