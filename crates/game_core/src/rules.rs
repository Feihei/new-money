//! 对局规则常量。
//!
//! Godot 端这些值散落在四个文件里且互相不一致 —— 缺陷 #12：起始资金 20000 在
//! `player_data.gd:13`、`player_data.gd:98`、`game_state.gd:41`、`turn_manager.gd:39`
//! 各写了一遍，而 `START_CASH` 常量虽然存在却没被 `reset()` 使用。这里收成唯一来源。

use bevy::prelude::*;

/// 数值规则的单一来源。凡是「改了就影响平衡」的量都放这里，不放进具体 system。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameRules {
    /// 开局现金，以及任何「重置对局」时的现金值。
    pub start_cash: i64,
}

impl Default for GameRules {
    fn default() -> Self {
        Self { start_cash: 20_000 }
    }
}
