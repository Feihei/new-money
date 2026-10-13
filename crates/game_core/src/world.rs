//! 对局世界：棋盘静态定义 + 由它派生的实体索引，以及开局装配流程。

use std::collections::HashMap;

use bevy::prelude::*;

use crate::board::BoardDef;
use crate::data::DataHandles;
use crate::land::{self, LandId};
use crate::player::{self, GameSetup};
use crate::rules::GameRules;

/// 取代 Godot 的 `GameState.board`（`game_state.gd:24`）。
///
/// `def` 是加载后的一次性快照而不是 `Handle`：规则 system 直接 `Res<ActiveBoard>` 就能拿拓扑，
/// 不必每处再解一遍 `Assets<BoardDef>`，headless 测试也只插这一个 resource 就能跑。
///
/// 只有**土地**建实体（要挂归属/等级/抵押这些运行时状态）；**道路格不建实体** —— 它没有任何
/// 可变状态，`BoardDef` 的拓扑查询就是它的接口，再复制一份 component 只会多出第二个真相来源。
/// 表现层为了拾取/渲染自己 spawn 的网格实体归 `game_app` 管，与规则层无关。
#[derive(Resource, Debug)]
pub struct ActiveBoard {
    pub def: BoardDef,
    pub lands: HashMap<LandId, Entity>,
}

/// 棋盘资产就位后一次性装配世界：校验 → spawn 土地实体 → spawn 玩家实体 → 插入 `ActiveBoard`。
///
/// 挂在 `Update` 上并用 `Option<Res<ActiveBoard>>` 自守门（0.20 没有 `resource_missing` 运行条件，
/// 而 `Option<Res<T>>` 的写法语义更直白）。B2b 引入 `TurnPhase` 后改挂 `OnEnter(Boot)`。
pub fn begin_game_system(
    mut commands: Commands,
    active: Option<Res<ActiveBoard>>,
    data: Res<DataHandles>,
    boards: Res<Assets<BoardDef>>,
    rules: Res<GameRules>,
    setup: Res<GameSetup>,
) {
    if active.is_some() {
        return;
    }
    let Some(def) = boards.get(&data.board).cloned() else {
        return;
    };
    if let Err(errs) = def.validate() {
        panic!("棋盘校验失败:\n  {}", errs.join("\n  "));
    }

    let lands = land::spawn_lands(&mut commands, &def);
    let players = player::spawn_players(&mut commands, &setup, &rules);
    info!(
        "世界装配完成：棋盘「{}」{} 主环格 / {} 土地实体 / {} 玩家（起始现金 {}）",
        def.board_name,
        def.loop_len(),
        lands.len(),
        players.len(),
        rules.start_cash
    );

    commands.insert_resource(ActiveBoard { def, lands });
}
