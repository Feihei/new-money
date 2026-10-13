//! 《暴富》规则层入口。
pub mod board;
pub mod card;
pub mod loader;
pub mod stock;

pub mod prelude {
    pub use crate::CoreGamePlugin;
    pub use crate::board::{BoardDef, LandDef, TileDef, TileKind};
    pub use crate::card::{CardDef, CardEffect, CardKind, CardSet};
    pub use crate::loader::JsonAssetLoader;
    pub use crate::stock::{StockDef, StockSet};
}

use bevy::prelude::*;
use prelude::{BoardDef, CardSet, JsonAssetLoader, StockSet};

/// 规则层 Plugin：注册数据资产与其 JSON loader，以及后续的 Message / States / Resource。
/// 只链接 bevy 的无渲染子集（`default_app`），因此 `cargo test -p game_core` 不需要窗口。
pub struct CoreGamePlugin;

impl Plugin for CoreGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<BoardDef>()
            .init_asset::<CardSet>()
            .init_asset::<StockSet>()
            .register_asset_loader(JsonAssetLoader::<BoardDef>::default())
            .register_asset_loader(JsonAssetLoader::<CardSet>::default())
            .register_asset_loader(JsonAssetLoader::<StockSet>::default());
    }
}
