//! 一局对局所需的数据资产句柄。
//!
//! 「读哪几个文件」属于规则层：表现层只负责把资产根指对（见 `game_app::asset_root`），
//! 于是 headless 测试与真实客户端走的是同一条加载路径。

use bevy::prelude::*;

use crate::board::BoardDef;
use crate::calendar::HolidaySet;
use crate::card::CardSet;
use crate::stock::StockSet;

pub const BOARD_PATH: &str = "data/boards/prototype.json";
pub const CHANCE_PATH: &str = "data/cards/chance_cards.json";
pub const FATE_PATH: &str = "data/cards/fate_cards.json";
/// 可持有的道具卡（P3a）。它没有对应的格子，来源是百货公司（P3d），
/// 所以**不在** `domain_of` 那张分派表上 —— 加这一行不是为了让人踩它。
pub const ITEMS_PATH: &str = "data/cards/item_cards.json";
pub const STOCKS_PATH: &str = "data/stocks/stocks.json";
/// 节假日表（P2b）：农历四节查表、固定公历四条由代码算。表覆盖 2048–2052。
pub const HOLIDAYS_PATH: &str = "data/calendar/holidays.json";

#[derive(Resource, Debug, Clone)]
pub struct DataHandles {
    pub board: Handle<BoardDef>,
    pub chance: Handle<CardSet>,
    pub fate: Handle<CardSet>,
    pub items: Handle<CardSet>,
    pub stocks: Handle<StockSet>,
    pub holidays: Handle<HolidaySet>,
}

impl DataHandles {
    pub fn load(server: &AssetServer) -> Self {
        Self {
            board: server.load(BOARD_PATH),
            chance: server.load(CHANCE_PATH),
            fate: server.load(FATE_PATH),
            items: server.load(ITEMS_PATH),
            stocks: server.load(STOCKS_PATH),
            holidays: server.load(HOLIDAYS_PATH),
        }
    }
}
