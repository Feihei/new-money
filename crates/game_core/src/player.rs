//! 玩家实体：运行时状态是组件，开局配置是 `GameSetup`。

use bevy::prelude::*;

use crate::board::TileIndex;
use crate::rules::GameRules;
use crate::world::Gameplay;

use std::collections::VecDeque;

/// 玩家标识。与 `LandId`、`TileIndex` 互不通用 —— 这是缺陷 #1
/// （拿 `"L"+land_id` 字符串 key 去调 `get_tile(index:int)`，导致贷款上限恒为 0）
/// 在 Rust 里编译不过的结构性原因。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct PlayerName(pub String);

/// 归属色，sRGB 三分量。规则层刻意不用 bevy 的 `Color`：`default_app` 特性子集里
/// 根本不包含 `bevy_color`，而且颜色本就只该由表现层解释。
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct PlayerColor(pub [f32; 3]);

impl PlayerColor {
    pub const fn srgb(r: f32, g: f32, b: f32) -> Self {
        Self([r, g, b])
    }
}

/// 棋子配色，照搬 Godot `board.gd:33-38`（四名上限即 `DEV_PLAN.md` 的对局人数上限）。
pub const PLAYER_COLORS: [PlayerColor; 4] = [
    PlayerColor::srgb(1.0, 0.3, 0.3),
    PlayerColor::srgb(0.3, 0.5, 1.0),
    PlayerColor::srgb(0.3, 0.85, 0.4),
    PlayerColor::srgb(1.0, 0.75, 0.1),
];

/// 现金。加减钱的入口在 B3 的结算 system，那里要同时发 `CashChanged` 与处理不足额（缺陷 #3）。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wallet {
    pub cash: i64,
}

/// 银行账户：存款生息、贷款计息（`bank_system.gd` 的 deposit/loan 字段）。
/// 利息结算不经过现金 —— 存款利息进余额、贷款利息进负债，与 Godot 一致。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BankBalance {
    pub deposit: i64,
    pub loan: i64,
}

/// 逻辑位置：永远只在主环上。
///
/// 入狱时停在**接入点**而不是死胡同格本身（`player_data.gd:62-68` 的语义），
/// 死胡同格只是表现层把棋子挪过去的视觉停车位 —— 这条区分是「core 拥有逻辑位置、
/// render 只做插值」的前提，否则出狱后不知道该从哪格继续走。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPosition {
    pub tile: TileIndex,
}

/// 存在即服刑（取代 Godot 的 `status == "jailed"` 字符串比较，缺陷 #8）。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct JailSentence {
    pub turns_left: u32,
}

/// 一次行走的进度，挂在玩家实体上；`path` 为空即「没在走」。
///
/// 与 §4.6 原案（`WalkQueue{path, elapsed}`）的差异：耗时累计放在全局
/// [`WalkClock`](crate::walk::WalkClock)（行走是互斥的，同一时刻至多一人在走），
/// 队列里只留「绝对进度」—— `done` 记已发过 `TileStepped` 的步数，
/// 时钟换算出的应到步数减去它就是本帧要补的步数。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct WalkQueue {
    /// 还没走的格（不含出发点，按绕行方向）。空 = 已到站或没在走。
    pub path: VecDeque<TileIndex>,
    /// 本段行走已完成的步数。
    pub done: u32,
    /// 本段行走的出发点（`PlayerArrived` 的 `from`）。
    pub from: TileIndex,
}

impl Default for WalkQueue {
    fn default() -> Self {
        Self {
            path: VecDeque::new(),
            done: 0,
            from: START_TILE,
        }
    }
}

impl WalkQueue {
    /// 队列里的总步数（含已完成的）。
    pub fn total(&self) -> usize {
        self.done as usize + self.path.len()
    }
}

/// 存在即破产。破产玩家退出回合序，钱包与地产由结算 system 清空。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bankrupt;

/// 起始格：主环 0 号（prototype 里是「起点」）。Godot 端同样假定 `tile_index = 0`
/// （`game_state.gd:42`），棋盘校验保证主环非空。
pub const START_TILE: TileIndex = TileIndex(0);

/// 开局时一个玩家的配置项。 spawned 之后就不再读。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSetup {
    pub name: String,
    pub color: PlayerColor,
}

/// 对局配置：玩家名单。表现层/测试在开局前插入即可覆盖默认阵容。
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameSetup {
    pub players: Vec<PlayerSetup>,
}

impl Default for GameSetup {
    /// 默认阵容与 Godot `board.gd:341-344` 一致：阿土伯 vs 孙小美。
    fn default() -> Self {
        Self {
            players: vec![
                PlayerSetup {
                    name: "阿土伯".into(),
                    color: PLAYER_COLORS[0],
                },
                PlayerSetup {
                    name: "孙小美".into(),
                    color: PLAYER_COLORS[1],
                },
            ],
        }
    }
}

/// 铺出玩家实体，返回实体句柄（回合序 `TurnOrder` 用）。
pub fn spawn_players(commands: &mut Commands, setup: &GameSetup, rules: &GameRules) -> Vec<Entity> {
    setup
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            commands
                .spawn((
                    PlayerId(i as u32),
                    PlayerName(p.name.clone()),
                    p.color,
                    Wallet {
                        cash: rules.start_cash,
                    },
                    TokenPosition { tile: START_TILE },
                    WalkQueue::default(),
                    BankBalance::default(),
                    crate::stock::StockHoldings::default(),
                    Gameplay,
                ))
                .id()
        })
        .collect()
}
