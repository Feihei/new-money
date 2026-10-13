//! 《暴富》规则层入口。
pub mod board;
pub mod card;
pub mod data;
pub mod land;
pub mod loader;
pub mod player;
pub mod rules;
pub mod stock;
pub mod world;

pub mod prelude {
    pub use crate::CoreGamePlugin;
    pub use crate::board::{BoardDef, LandDef, TileDef, TileIndex, TileKind};
    pub use crate::card::{CardDef, CardEffect, CardKind, CardSet};
    pub use crate::data::{BOARD_PATH, CHANCE_PATH, DataHandles, FATE_PATH, STOCKS_PATH};
    pub use crate::land::{LandId, Owner};
    pub use crate::loader::JsonAssetLoader;
    pub use crate::player::{
        Bankrupt, GameSetup, JailSentence, PLAYER_COLORS, PlayerColor, PlayerId, PlayerName,
        PlayerSetup, START_TILE, TokenPosition, Wallet,
    };
    pub use crate::rules::GameRules;
    pub use crate::stock::{StockDef, StockSet};
    pub use crate::world::{ActiveBoard, begin_game_system};
}

use bevy::prelude::*;
use prelude::{
    BoardDef, CardSet, GameRules, GameSetup, JsonAssetLoader, StockSet, begin_game_system,
};

/// 规则层 Plugin：注册数据资产与其 JSON loader、对局 resource，以及开局装配 system。
/// 只链接 bevy 的无渲染子集（`default_app`），因此 `cargo test -p game_core` 不需要窗口。
pub struct CoreGamePlugin;

impl Plugin for CoreGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<BoardDef>()
            .init_asset::<CardSet>()
            .init_asset::<StockSet>()
            .register_asset_loader(JsonAssetLoader::<BoardDef>::default())
            .register_asset_loader(JsonAssetLoader::<CardSet>::default())
            .register_asset_loader(JsonAssetLoader::<StockSet>::default())
            // 对局配置有默认值（prototype 双人阵容），表现层要换阵容就在 Startup 前 insert_resource 覆盖
            .init_resource::<GameRules>()
            .init_resource::<GameSetup>()
            // 棋盘资产到位后装配世界；表现层的 spawn 流程必须排在它后面
            .add_systems(Update, begin_game_system);
    }
}
