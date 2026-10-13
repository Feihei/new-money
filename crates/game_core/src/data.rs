//! 一局对局所需的数据资产句柄。
//!
//! 「读哪几个文件」属于规则层：表现层只负责把资产根指对（见 `game_app::asset_root`），
//! 于是 headless 测试与真实客户端走的是同一条加载路径。

use bevy::prelude::*;

use crate::board::BoardDef;
use crate::card::CardSet;
use crate::stock::StockSet;

pub const BOARD_PATH: &str = "data/boards/prototype.json";
pub const CHANCE_PATH: &str = "data/cards/chance_cards.json";
pub const FATE_PATH: &str = "data/cards/fate_cards.json";
pub const STOCKS_PATH: &str = "data/stocks/stocks.json";

#[derive(Resource, Debug, Clone)]
pub struct DataHandles {
    pub board: Handle<BoardDef>,
    pub chance: Handle<CardSet>,
    pub fate: Handle<CardSet>,
    pub stocks: Handle<StockSet>,
}

impl DataHandles {
    pub fn load(server: &AssetServer) -> Self {
        Self {
            board: server.load(BOARD_PATH),
            chance: server.load(CHANCE_PATH),
            fate: server.load(FATE_PATH),
            stocks: server.load(STOCKS_PATH),
        }
    }
}
