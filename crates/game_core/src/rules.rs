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
    /// 涨价卡期间的租金倍数（PLAN §三「抵消对」那行的「整街 5 天加倍」）。
    /// 与 [`monopoly_rent_multiplier`] 分开两个数：它们是两个规则，会各自被改
    /// （将来「涨价不叠加垄断」这种话一说出口，共用一个数就得先拆开）。
    pub hike_rent_multiplier: i64,
    /// 存款利率（**每月**），月末那一天发。P2④ 之前它是「每回合 3%」并且挂在踩银行格上，
    /// 一天一回合的口径下等于日复利 3%（一个月 12 天就涨四成），那不是我想要的银行。
    pub deposit_rate: f32,
    /// 贷款利率（**每月**）。必须高于 [`deposit_rate`]：两个数反过来，「借钱存进银行」
    /// 就是无风险套利，玩家五分钟就能发现，而规则层没有任何东西挡得住它。
    pub loan_rate: f32,
    /// 最多可贷 = 未抵押地产总价 × 该系数，`bank_system.gd:10`。
    pub max_loan_ratio: f32,
    /// 股价每 tick 的基础漂移，`stock_market.gd:14`。
    pub stock_base_drift: f32,
    /// 一个月多少天。一天 == 一个回合结束（行情就在那里 tick 一次），所以这条同时是
    /// 「多少回合一个月」。B5b 起分红不再挂在 tick 计数上，而是挂在日历的固定日 ——
    /// 「每月 1 号派息」是玩家能记住的规则，「每 5 次 tick」不是。
    pub days_per_month: u32,
    /// 一周多少天。周末（每周最后两天）**股市与银行都休**（PLAN §三「日期系统」行）。
    /// 与 `days_per_month` 不整除是刻意的：月长 12、周长按 7 走，周末在月内逐月漂移，
    /// 于是「这个月踩银行格能不能办业务」不是一眼能背出来的表 —— 玩家要抬头看日历。
    pub days_per_week: u32,
    /// 每月哪一天派息（1 起）。被 `economy::date_of` 的日历解释。
    pub dividend_day: u32,
    /// 每股股息 = 股价 × 该比例，`stock_market.gd:16`。
    pub dividend_rate: f32,
    /// 每只股保留的历史价格条数，`stock_market.gd:17`。
    pub max_price_history: usize,
    /// 点券箱被取走后隔多少回合重新出现。格子上写了 `data.refresh` 就以格子为准，
    /// 这条只是缺省值（`4 代路上宝箱是一次性的`，「什么时候再长出来」是我们的参数）。
    pub coupon_refresh_turns: u32,
    /// 手牌上限（PLAN §三「持有与上限」行：4 代是 30 种卡、同时最多 15 张）。
    /// 满了不是「不让买」而是**挤掉最前面那张**，所以这条由 [`crate::hand::Hand::gain`] 读，
    /// 那里才看得见「被挤掉的是哪张」这条流水要写什么。
    pub hand_limit: u32,
}

impl Default for GameRules {
    fn default() -> Self {
        Self {
            start_cash: 20_000,
            walk_step_interval: 0.18,
            max_building_level: 6,
            monopoly_rent_multiplier: 2,
            hike_rent_multiplier: 2,
            deposit_rate: 0.10,
            loan_rate: 0.15,
            max_loan_ratio: 2.0,
            stock_base_drift: 0.002,
            days_per_month: 12,
            days_per_week: 7,
            dividend_day: 1,
            dividend_rate: 0.015,
            max_price_history: 50,
            coupon_refresh_turns: 6,
            hand_limit: 15,
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
