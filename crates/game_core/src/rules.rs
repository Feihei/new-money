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
    /// 每月哪一天派息（1 起）。被 `calendar::is_dividend_day` 解释；
    /// P2b 起是 **15 号**（月中那天＝乐透开奖 + 公司分红，月末另结储金息）。
    ///
    /// 一天 == 一个回合结束（行情在那里 tick 一次），所以「一个月」不再是固定回合数：
    /// 月长由真实公历给出（28–31、闰年 2/29），见 [`crate::calendar`]。
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
    /// 百货公司一次摆几个货位（PLAN §三「每次进入货架不同」）。
    /// 不是「把整副货架摆出来」：货位少，「这一趟买哪一张」才是个选择，而 P5 的道具池一长大，
    /// 全摆出来的面板就成了一页清单。抽签不放回，所以这三个货位互不重复。
    pub shop_slots: u32,
    /// 百货回收折扣的**分子**（PLAN §三「回收打 9 折」→ 90）。
    /// 写成整数分子而不是 `0.9: f32`：点券是整数账，`cost × 90 / 100` 的向下取整是规则的一部分
    /// （20 点券的卡卖回 18，不是 18.0 再想办法），用浮点乘就变成两次舍入。
    pub shop_buyback: u32,
    /// 抵押到手 = 地块账面价值 × 该分子 / 100（P3d③）。
    ///
    /// **按账面而不是只按地价**（`economy::estate_value`：地价 + 已花的加盖费）：加盖花掉的
    /// 钱否则就在抵押那一刻蒸发了 —— 而抵押中的地不能加盖（`can_upgrade` 那条），所以这笔
    /// 数一旦定下就冻结，赎回时不会与它脱钩。
    ///
    /// 50 这个数与 [`max_loan_ratio`] 是一对：可贷额度是地价的 200%，而抵押只给账面的 50%，
    /// 所以**先借后押**是显然的顺序 —— 抵押是给「额度已经借满、或不想再背利息」的人留的
    /// 最后一道变现口。把它调高过 `max_loan_ratio` 之前先想清楚这条。
    pub mortgage_percent: u32,
    /// 赎回 = 抵押到手那笔 × 该分子 / 100（P3d③）。110 = 一成手续费。
    ///
    /// 手续费是**必须的**，不是平衡调味：抵押不计息（它的代价是失去收租权、垄断资格与可贷
    /// 额度），若赎回又分文不加，「缺钱就抵押、宽裕就赎回」成了一台免费的现金泵，
    /// 而它每转一圈都在给下一回合的租金定价（`total_assets` 读的就是这些地块）。
    pub redeem_percent: u32,
    /// 保释一次要花的点券（P4③）。**不动现金**：别的玩家走到拘留场所的接入点，
    /// 花这笔钱把里面的人放出来；与 P12 保释四大恶人（300）同一条判据、同一个界面。
    ///
    /// 30 是 10-20 拍板时给的单子（监狱与医院同价）—— 4 代原文查不到，所以这是设计值。
    pub bail_points: u32,
    /// 大穷神上身当场拿走现金的**分母**（P6b）：`现金 / 4`。
    ///
    /// 写成分母而不是百分比，是因为这条天生是「按比例抢」而不是「抢一个固定数」——
    /// 神明金额本来豁免物价指数（`scales_with_price(God) == false`，PLAN §三 点名），
    /// 一个固定数在物价 5 倍时就成了白送；而按比例抢的数额自动跟着钱包走，不必再乘一遍。
    ///
    /// 4 是设计值（4 代只说「损失现金」，没给数）。读它的那一处夹成**至少 1**：
    /// 这个数一旦从存档或设置里读进来，`0` 就是一次当场崩掉的除法，而它换来的只是「不抢钱」。
    pub poor_cash_divisor: u32,
    /// 三种小游戏的成绩→点券表（P7）。**三种各配「满分」与「每分几点券」，放在同一个
    /// 结构体里**，理由与 [`bail_points`] 那条同族：这三个游戏的平衡是这个功能唯一真正
    /// 难的地方，参数散在三个玩法文件里就没法横向比较（「踩到哪一格」不该是抽一张
    /// 大小不同的奖）。表本身的数与理由见 [`crate::minigame::Payouts`]。
    pub minigames: crate::minigame::Payouts,
    /// 乐透的号码池大小（1 起，含）。开奖从 `1..=lottery_numbers` 抽一个。
    ///
    /// 36 是 4 代口径（PLAN §三「1–36 选 1 号」）。它同时决定了**中奖概率**，所以它与
    /// [`ticket_price`]、[`jackpot_rate`] 是一对三的平衡三元组：号码越多，同样一注的期望
    /// 回报越低，而奖池滚得越久。改这一个数之前先想那两个。
    pub lottery_numbers: u32,
    /// 一注乐透券的价钱（现金，**不是点券**）。4 代口径：1000 元/注。
    ///
    /// 全额进池（[`crate::lottery::Jackpot`] 的唯一常规来源），规则层不抽头 —— 一旦抽头，
    /// 「奖池为什么是这个数」就多了一个只有代码知道的第三方，而 PLAN §四 P8 那条验收要的
    /// 恰恰是「每笔进池的钱都能从流水追溯」。
    pub ticket_price: i64,
    /// 奖池的**月息**（与 [`deposit_rate`] 同一条日子：月末那一天结）。
    ///
    /// 这是奖池的第二条来源，也是「本月没人猜中」时池子仍然会长大的那件事。它比储金月息
    /// （0.10）低一档：池子不该比把钱正经存进银行更划算，否则「买券等息」成了无风险的
    /// 存钱姿势，而乐透本该是一个负期望的赌局。
    pub jackpot_rate: f32,
    /// 一处破产清算**拍几块地**（10-22 拍板：从他名下地产里随机抽三处逐项拍卖，其余归无主）。
    ///
    /// 它是「抽多少个」而不是「抽哪几个」：哪几个由 [`crate::rng::Rng::pick_auction`] 决定，
    /// 而块数是一个玩法判断 —— 全拍会把一局拖成一场清仓展销，一块又让「破产」在玩家眼里
    /// 等于「什么也没发生」。真棋盘的地多了，这个数要重看，所以它住在这里而不是 `auction.rs`。
    pub liquidation_lots: u32,
    /// 拍卖一档加多少：起拍价的百分之几（`auction::step_of` 还夹一个 50 元的下限）。
    ///
    /// 与 [`liquidation_lots`] 一起是拍卖的两个数：一个决定拍几场，一个决定每场喊多久。
    /// 台阶太窄（1%）会让一场拍卖变成几十次点击，太宽（25%）则第一个敢喊的人就吓退全场。
    pub auction_step_percent: u32,
}

impl Default for GameRules {
    fn default() -> Self {
        Self {
            start_cash: 20_000,
            walk_step_interval: 0.18,
            max_building_level: 5,
            monopoly_rent_multiplier: 2,
            hike_rent_multiplier: 2,
            deposit_rate: 0.10,
            loan_rate: 0.15,
            max_loan_ratio: 2.0,
            stock_base_drift: 0.002,
            dividend_day: 15,
            dividend_rate: 0.015,
            max_price_history: 50,
            coupon_refresh_turns: 6,
            hand_limit: 15,
            shop_slots: 3,
            shop_buyback: 90,
            mortgage_percent: 50,
            redeem_percent: 110,
            bail_points: 30,
            poor_cash_divisor: 4,
            minigames: crate::minigame::Payouts::default(),
            lottery_numbers: 36,
            ticket_price: 1_000,
            jackpot_rate: 0.05,
            liquidation_lots: 3,
            auction_step_percent: 10,
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
