//! 对局规则常量。
//!
//! Godot 端这些值散落在四个文件里且互相不一致 —— 缺陷 #12：起始资金 20000 在
//! `player_data.gd:13`、`player_data.gd:98`、`game_state.gd:41`、`turn_manager.gd:39`
//! 各写了一遍，而 `START_CASH` 常量虽然存在却没被 `reset()` 使用。这里收成唯一来源。

use bevy::prelude::*;

/// 数值规则的单一来源。凡是「改了就影响平衡」的量都放这里，不放进具体 system。
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct GameRules {
    /// 开局现金，以及任何「重置对局」时的现金值。
    pub start_cash: i64,
    /// 逐格行走的定步时长（秒），对应 Godot `player_token.gd:17` 的 0.18s。
    /// `0` = 跳过动画，一帧走完 —— headless 测试与「动画速度」设置都归到这条路径
    /// （B2c 验收：interval=0 时与 B2b 的瞬时落格行为等价）。
    pub walk_step_interval: f32,
    /// 建筑等级上限（`property_system.gd:113` 的 `level >= 6`）。
    pub max_building_level: u8,
    /// 垄断同色组后的租金倍数（`property_system.gd:75-77` 的 `base_rent *= 2`）。
    pub monopoly_rent_multiplier: i64,
    /// 存款利率（每回合），`bank_system.gd:8` 的 3%。
    pub deposit_rate: f32,
    /// 贷款利率（每回合），`bank_system.gd:9` 的 8%。
    pub loan_rate: f32,
    /// 最多可贷 = 未抵押地产总价 × 该系数，`bank_system.gd:10`。
    pub max_loan_ratio: f32,
    /// 股价每 tick 的基础漂移，`stock_market.gd:14`。
    pub stock_base_drift: f32,
    /// 每多少个 tick 分红一次，`stock_market.gd:15`。
    pub dividend_interval: u32,
    /// 每股股息 = 股价 × 该比例，`stock_market.gd:16`。
    pub dividend_rate: f32,
    /// 每只股保留的历史价格条数，`stock_market.gd:17`。
    pub max_price_history: usize,
}

impl Default for GameRules {
    fn default() -> Self {
        Self {
            start_cash: 20_000,
            walk_step_interval: 0.18,
            max_building_level: 6,
            monopoly_rent_multiplier: 2,
            deposit_rate: 0.03,
            loan_rate: 0.08,
            max_loan_ratio: 2.0,
            stock_base_drift: 0.002,
            dividend_interval: 5,
            dividend_rate: 0.015,
            max_price_history: 50,
        }
    }
}

impl GameRules {
    /// 瞬时落格：行走不占帧（无头测试的默认形态 —— 那里没有真实时间流逝，
    /// 定步时钟永远推不动，必须退化成 B2b 的即时行为）。
    pub fn instant() -> Self {
        Self {
            walk_step_interval: 0.0,
            ..Self::default()
        }
    }
}
