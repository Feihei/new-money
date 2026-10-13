//! 《暴富》规则层入口。
pub mod board;

pub mod prelude {
    pub use crate::CoreGamePlugin;
    pub use crate::board::{BoardDef, LandDef, TileDef, TileKind};
}

use bevy::prelude::*;

/// 规则层 Plugin：注册 Message / States / Resource，spawn 棋盘与玩家实体。
/// 不依赖任何渲染 feature，因此 `cargo test -p game_core` 无需窗口。
pub struct CoreGamePlugin;

impl Plugin for CoreGamePlugin {
    fn build(&self, _app: &mut App) {}
}
