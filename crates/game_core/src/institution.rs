//! 机构停摆（P11c②）：新闻能让**一个规矩**停几天，而不是只能打人或者打地。
//!
//! 这是新闻作用域的第四档 —— PLAN §三 那行原文写的是「某类地产（股市/银行）」，
//! 落地时定的是**机构**而不是地产：那两条新闻改的不是哪一块地值多少，而是
//! 「这几天这条规矩还作不作数」（股市停牌、银行停止放款）。所以它既不属于人档
//! （没有承受者，挨不到任何一个人的钱包）也不属于地档（没有锚点，街上没有一块地会变），
//! [`crate::news::Family`] 因此长出第三族。
//!
//! **停摆的是「一项能力」而不是「一家门面」**：`Bank` 那一格停的是**放款**，
//! 存取款与还款照办 —— 银行关门会让还不上钱的人凭空多出一笔逾期，而「停止放款」
//! 那句话的本意是「不借新的」，不是「不许还钱」。这条差别由
//! [`crate::bank::bank_op_allowed`] 那三格分开写死（10-24 拍）。
//!
//! 天数走的是与地块状态同一把尺：**一天 = 一个回合结束**（见
//! [`crate::status::Timed`] 与 [`crate::market::market_tick_on_turn_end_system`]），
//! 由 [`age_suspensions_system`] 与 [`crate::status::expire_land_status_system`] 同批递减。
//! 状态挂在 [`crate::turn::GameSession`] 上而不是单独一个资源：它**必须进存档**（P15），
//! 而「开局归零」这件事由 `begin_game_system` 造一份新 `GameSession` 顺带解决 ——
//! 跨局残留那一族缺陷（§6.1 坑 22/25）在这里少一个入口。

use bevy::prelude::ResMut;
use serde::Deserialize;

/// 哪一家机构可以被停摆。两格各自对应一条**能力**，不是两栋楼。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Institution {
    /// 股市：停的是「下单受理」与「行情往前走」这两样。
    /// 与日历休市（周末∪节假日，[`crate::calendar::HolidaySet::is_market_closed`]）
    /// **并列而不是合并**：那条说的是「今天本来就不开盘」，这一条说的是「上面发了话」。
    StockMarket,
    /// 银行：停的是**放款**。存款、取款、还款照常。
    Bank,
}

impl Institution {
    /// 机构名。顶栏那句、灰按钮那句与 dump 那一行都用它。
    pub const fn label(self) -> &'static str {
        match self {
            Self::StockMarket => "股市",
            Self::Bank => "银行",
        }
    }

    /// 停的是哪件事的人话。**不许并进 [`Self::label`] 的调用点**：「银行 5 天」说不清
    /// 停的是什么，而玩家下一手要问的正是「那我还能不能借钱」。
    pub const fn halt_phrase(self) -> &'static str {
        match self {
            Self::StockMarket => "停牌",
            Self::Bank => "停止放款",
        }
    }
}

/// 两家机构各自的停摆剩余天数。`0` = 没停摆（不是「停摆 0 天」）。
///
/// 为什么是**两个数**而不是 `Vec<(Institution, u32)>`：这一族只有两格，写成一个表就要
/// 为「同一家出现两次」定一条优先级，而那条规则今天没有答案（地块那边同理，见
/// [`crate::status`] 模块头：单槽位的替换就是「后盖前」，不留优先级表）。
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Suspensions {
    stock_market: u32,
    bank: u32,
}

impl Suspensions {
    /// 还剩几天（`0` = 没停摆）。面板那句「（剩 N 天）」读它。
    pub fn days_left(self, what: Institution) -> u32 {
        match what {
            Institution::StockMarket => self.stock_market,
            Institution::Bank => self.bank,
        }
    }

    /// 这一家现在停着吗。规则层与灰按钮读的是这**同一个**判据。
    pub fn halted(self, what: Institution) -> bool {
        self.days_left(what) > 0
    }

    /// 停摆 N 天，返回**叠完之后**的剩余天数。
    ///
    /// 「更久的那一张赢」（与 [`crate::landing::detain`]、[`crate::player::StaySentence`]
    /// 同一条口径）：手上已有 5 天的人再挨一条 3 天，不该被替他们减刑成 3 天。
    /// 返回值不是冗余 —— 顶栏与浮层那句「剩 N 天」读的是世界上的那个数，
    /// 而条目上那个数在叠加那天是错的（`NewsAudience::Institution::days` 因此由
    /// 这一句的返回值喂，不由数据喂）。
    pub fn suspend(&mut self, what: Institution, days: u32) -> u32 {
        let slot = match what {
            Institution::StockMarket => &mut self.stock_market,
            Institution::Bank => &mut self.bank,
        };
        *slot = (*slot).max(days);
        *slot
    }

    /// 过一天：两家各减一格（`saturating_sub`，与 [`crate::status::Timed::age`] 同一条
    /// 防回绕纪律 —— 到期后没人减到 0 为止的话，面板会念出「剩 42 亿天」）。
    pub fn age(&mut self) {
        self.stock_market = self.stock_market.saturating_sub(1);
        self.bank = self.bank.saturating_sub(1);
    }
}

/// 回合末让机构停摆过一天。与 [`crate::status::expire_land_status_system`] 同一批
/// （`OnEnter(TurnPhase::TurnEnd)`）：**一天＝一个回合结束**这条口径只有一处定义，
/// 涨价那五天、查封那五天与停牌这几天才是同一把尺子。
///
/// ⚠️ 但它**不能与行情 tick 挤在同一个并行元组里**：那条也碰 `GameSession`（经 `Books`），
/// 同阶段并跑会在调度期直接撞成借用冲突（§6.1 坑 33）。而次序在这里**有语义**：
/// 必须在行情 tick **之后**过这一天，否则「停牌 1 天」会在当天 tick 之前就被减成 0，
/// 那一天照常开盘 —— 少挡的正是它本要挡的那一天（与 §6.1 坑 36 那条「排位置不是排次序」
/// 是同一件事的另一半：这里要的次序由 `chain` 的阶段边界给）。
pub fn age_suspensions_system(mut session: ResMut<crate::turn::GameSession>) {
    session.suspensions.age();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_longer_suspension_wins_and_the_shorter_one_is_not_a_pardon() {
        let mut s = Suspensions::default();
        assert_eq!(s.suspend(Institution::StockMarket, 5), 5);
        // 再吃一条 3 天：世界上的答案是 5，而这条新闻报出去的也必须是 5
        assert_eq!(
            s.suspend(Institution::StockMarket, 3),
            5,
            "短的那一条把停摆替人减了刑"
        );
        assert_eq!(s.suspend(Institution::StockMarket, 8), 8);
        assert!(!s.halted(Institution::Bank), "一家停摆不该连带另一家");
    }

    #[test]
    fn the_clock_counts_down_to_zero_and_stops_there() {
        let mut s = Suspensions::default();
        s.suspend(Institution::Bank, 2);
        s.age();
        assert!(s.halted(Institution::Bank), "第二天还在停");
        s.age();
        assert!(!s.halted(Institution::Bank), "过完第三天就该恢复");
        // 到期后继续减（漏摘/多跑一帧）不许回绕成天文数字
        s.age();
        assert_eq!(s.days_left(Institution::Bank), 0);
    }

    /// 顶栏与灰按钮念的是这两句。「银行 5 天」那种说法要红 —— 它答不出玩家那一问
    /// （「那我还能不能借钱」），而这一格的全部用途就是把「停的是哪项能力」说清。
    #[test]
    fn each_institution_says_which_capability_stopped() {
        assert_eq!(
            (
                Institution::StockMarket.label(),
                Institution::StockMarket.halt_phrase()
            ),
            ("股市", "停牌")
        );
        assert_eq!(
            (Institution::Bank.label(), Institution::Bank.halt_phrase()),
            ("银行", "停止放款")
        );
    }
}
