//! 一局对局所需的数据资产句柄。
//!
//! 「读哪几个文件」属于规则层：表现层只负责把资产根指对（见 `game_app::asset_root`），
//! 于是 headless 测试与真实客户端走的是同一条加载路径。

use bevy::prelude::*;

use crate::board::BoardDef;
use crate::calendar::HolidaySet;
use crate::card::CardSet;
use crate::character::CharacterSet;
use crate::news::NewsSet;
use crate::stock::StockSet;

/// 玩家看到的那张盘（P2c）：台湾 16 条街。演示盘 `prototype.json` 仍留在仓库里，
/// 规则层集成测试把它当**固定夹具**显式加载（`DataHandles::load_with_board`）——
/// 那些用例的格号/地号/地名是按演示盘写的，换默认盘不等于也要把它们重写一遍。
pub const BOARD_PATH: &str = "data/boards/taiwan.json";
pub const FATE_PATH: &str = "data/cards/fate_cards.json";
/// 可持有的道具卡（P3a）。它没有对应的格子，来源是百货公司（P3d），
/// 所以**不在** `domain_of` 那张分派表上 —— 加这一行不是为了让人踩它。
pub const ITEMS_PATH: &str = "data/cards/item_cards.json";
pub const STOCKS_PATH: &str = "data/stocks/stocks.json";
/// 节假日表（P2b）：农历四节查表、固定公历四条由代码算。表覆盖 2048–2052。
pub const HOLIDAYS_PATH: &str = "data/calendar/holidays.json";
/// 新闻池（P11）。与两副牌**不同目录**是刻意的：它不是「一副要抽穿的牌」，
/// 抽取算法也不同（全池随机 + 同场不连重，见 `news::draw`）—— 放同一个目录会让人
/// 以为它归 `deck::order_of` 那张穷尽表管。
pub const NEWS_PATH: &str = "data/news/news_items.json";
/// 首发十二人的档案（P2d）。主菜单的对局设置页读它选人；规则层只在开局时
/// 收下选好的 `GameSetup`，不直接读这份表。
pub const CHARACTERS_PATH: &str = "data/characters.json";

#[derive(Resource, Debug, Clone)]
pub struct DataHandles {
    pub board: Handle<BoardDef>,
    pub fate: Handle<CardSet>,
    pub items: Handle<CardSet>,
    pub stocks: Handle<StockSet>,
    pub holidays: Handle<HolidaySet>,
    pub news: Handle<NewsSet>,
    pub characters: Handle<CharacterSet>,
}

impl DataHandles {
    pub fn load(server: &AssetServer) -> Self {
        Self::load_with_board(server, BOARD_PATH)
    }

    /// 换一张棋盘（测试夹具 / 将来多盘选择）。其余资产与默认路径同一套。
    pub fn load_with_board(server: &AssetServer, board: &'static str) -> Self {
        Self {
            board: server.load(board),
            fate: server.load(FATE_PATH),
            items: server.load(ITEMS_PATH),
            stocks: server.load(STOCKS_PATH),
            holidays: server.load(HOLIDAYS_PATH),
            news: server.load(NEWS_PATH),
            characters: server.load(CHARACTERS_PATH),
        }
    }
}
