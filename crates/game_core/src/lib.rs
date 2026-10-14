//! 《暴富》规则层入口。
pub mod board;
pub mod card;
pub mod data;
pub mod land;
pub mod loader;
pub mod msg;
pub mod player;
pub mod rng;
pub mod rules;
pub mod stock;
pub mod turn;
pub mod world;

pub mod prelude {
    pub use crate::CoreGamePlugin;
    pub use crate::board::{BoardDef, LandDef, TileDef, TileIndex, TileKind};
    pub use crate::card::{CardDef, CardEffect, CardKind, CardSet};
    pub use crate::data::{BOARD_PATH, CHANCE_PATH, DataHandles, FATE_PATH, STOCKS_PATH};
    pub use crate::land::{LandId, Owner};
    pub use crate::loader::JsonAssetLoader;
    pub use crate::msg::{
        DiceRolled, GameOver, JailSkipped, PlayerReleased, RequestRoll, TurnEnded, TurnStarted,
    };
    pub use crate::player::{
        Bankrupt, GameSetup, JailSentence, PLAYER_COLORS, PlayerColor, PlayerId, PlayerName,
        PlayerSetup, START_TILE, TokenPosition, Wallet,
    };
    pub use crate::rng::{DIE_FACES, GameSeed, Rng};
    pub use crate::rules::GameRules;
    pub use crate::stock::{StockDef, StockSet};
    pub use crate::turn::{GameSession, TurnOrder, TurnPhase};
    pub use crate::world::{ActiveBoard, begin_game_system};
}

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use prelude::{
    BoardDef, CardSet, DiceRolled, GameOver, GameRules, GameSeed, GameSetup, JailSkipped,
    JsonAssetLoader, PlayerReleased, RequestRoll, StockSet, TurnEnded, TurnPhase, TurnStarted,
    begin_game_system,
};
use turn::{
    begin_turn_system, end_turn_system, game_over_system, resolve_landing_system, roll_dice_system,
};

/// 规则层 Plugin：注册数据资产与其 JSON loader、对局 resource、回合消息与状态机。
/// 只链接 bevy 的无渲染子集（`default_app`），因此 `cargo test -p game_core` 不需要窗口。
pub struct CoreGamePlugin;

impl Plugin for CoreGamePlugin {
    fn build(&self, app: &mut App) {
        // 规则层用了 `States`，就得保证 `StateTransition` 调度存在。真实客户端经 `DefaultPlugins`
        // 已有；headless 测试里没有，所以自己补 —— 但 `StatesPlugin` 的 `insert_after` 不幂等，
        // 加两遍会让状态转移每帧跑两次，因此必须先查。
        if !app.is_plugin_added::<StatesPlugin>() {
            app.add_plugins(StatesPlugin);
        }

        app.init_asset::<BoardDef>()
            .init_asset::<CardSet>()
            .init_asset::<StockSet>()
            .register_asset_loader(JsonAssetLoader::<BoardDef>::default())
            .register_asset_loader(JsonAssetLoader::<CardSet>::default())
            .register_asset_loader(JsonAssetLoader::<StockSet>::default())
            // 对局配置有默认值（prototype 双人阵容 + 可复现种子），
            // 表现层要换阵容/换种子就在 Startup 前 insert_resource 覆盖
            .init_resource::<GameRules>()
            .init_resource::<GameSetup>()
            .init_resource::<GameSeed>()
            .add_message::<TurnStarted>()
            .add_message::<RequestRoll>()
            .add_message::<DiceRolled>()
            .add_message::<TurnEnded>()
            .add_message::<PlayerReleased>()
            .add_message::<JailSkipped>()
            .add_message::<GameOver>()
            .init_state::<TurnPhase>()
            // 装配世界：初始态的 OnEnter 不会触发，所以用 in_state(Boot)
            .add_systems(Update, begin_game_system.run_if(in_state(TurnPhase::Boot)))
            .add_systems(OnEnter(TurnPhase::TurnStart), begin_turn_system)
            .add_systems(
                Update,
                roll_dice_system.run_if(in_state(TurnPhase::WaitRoll)),
            )
            .add_systems(OnEnter(TurnPhase::Landed), resolve_landing_system)
            .add_systems(OnEnter(TurnPhase::TurnEnd), end_turn_system)
            .add_systems(OnEnter(TurnPhase::Over), game_over_system);
    }
}
