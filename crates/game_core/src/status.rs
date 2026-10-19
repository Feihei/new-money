//! 限时状态容器（P3c）。PLAN §三「抵消对」那行的缺口原文是「地块与股票上没有『限时状态』
//! 这种容器」—— 缺的不是某条规则，是**放规则的那个格子**：涨价五天、查封五天、将来的
//! 涨停跌停三天，全都是「一个种类 + 一个倒计时 + 到点自己消失」。
//!
//! 于是这里只有一个泛型组件 [`Timed<T>`] 与一条递减规则，宿主各自决定自己那张穷尽表
//! （地块的是 [`LandStatus`]）。为什么不做成一个 `Vec<Status>` 挂在实体上：
//!
//! - **同一块地同时挂着「涨价」与「查封」是说不清的**（租金到底翻倍还是零？）。
//!   单槽位让「查封抵掉涨价」这件事在数据结构上就是**替换**，与 4 代那张卡读出来的
//!   「抵消对」完全一致，也不需要一条「谁压过谁」的优先级表 —— 优先级表是会被漏填的那种东西。
//! - 倒计时只有一份，回合末一次递减，不会有「清了一个忘了清另一个」的残留。
//!
//! 寿命由**宿主**管：`Timed` 只回答「还剩几天」「今天过完没了没」，谁插的组件谁负责在
//! 到期时摘掉（见 [`expire_land_status_system`]）。这与 [`crate::charge::Corrections`]
//! 那条「没有撤销接口是刻意的」是同一个分工。

use bevy::prelude::*;
use serde::Deserialize;

/// 一个会自己消失的状态：种类 + 剩余天数。
///
/// 「天」在这套代码里等于**一个回合结束**（与 [`crate::world::Gameplay`] 上的日历同一个
/// 节拍，见 `market::market_tick_on_turn_end_system`），所以 `days_left` 只在回合末那一批
/// system 里减一次；一回合里不管发生多少笔结算都不掉天数。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timed<T> {
    pub kind: T,
    pub days_left: u32,
}

impl<T: Copy> Timed<T> {
    pub fn new(kind: T, days: u32) -> Self {
        Self {
            kind,
            days_left: days,
        }
    }

    /// 过一天。返回 `true` = 今天过完就到期，**调用方负责把组件摘掉**。
    ///
    /// `saturating_sub`：到期后如果没人摘（漏了一条 system），这里也不会减成负数把
    /// 「剩 4294967295 天」显示到面板上 —— 那类回绕错误读起来像数据错了，不像少跑了一条规则。
    pub fn age(&mut self) -> bool {
        self.days_left = self.days_left.saturating_sub(1);
        self.days_left == 0
    }
}

/// 地块上的限时状态。**不许出现第四种**：新增一种要同时决定它怎么改租金、
/// 挡不挡加盖、被谁抵消（这三问各有一张穷尽表在等着它）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandStatus {
    /// 涨价：租金翻倍（4 代的「涨价卡」，路面发红）。
    PriceHike,
    /// 查封：不收租、也不能加盖（法院贴条子，路面发蓝）。与 `PriceHike` 互为抵消对 ——
    /// 单槽位的替换就是那对关系，见模块头。
    Sealed,
}

impl LandStatus {
    /// 这一种状态动的是什么，给调试面板与顶栏流水说人话用。
    pub fn label(&self) -> &'static str {
        match self {
            LandStatus::PriceHike => "涨价",
            LandStatus::Sealed => "查封",
        }
    }
}

/// 回合末让地块上的限时状态过一天。
///
/// 挂在 `OnEnter(TurnPhase::TurnEnd)` 那一组里（与行情 tick 同批）：**一天 = 一个回合结束**
/// 这条口径只有一处定义，涨价的五天与派息的日子才是同一把尺子。
/// 到期即摘组件 —— 于是「这块地现在什么状态」永远只有一个读法：组件在就是在。
pub fn expire_land_status_system(
    mut commands: Commands,
    mut statuses: Query<(Entity, &mut Timed<LandStatus>)>,
) {
    for (entity, mut status) in &mut statuses {
        if status.age() {
            commands.entity(entity).remove::<Timed<LandStatus>>();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_last_day_expires_and_the_counter_never_wraps() {
        let mut hike = Timed::new(LandStatus::PriceHike, 2);
        assert!(!hike.age(), "第二天还在");
        assert!(hike.age(), "过完第三天就是最后一天");
        assert_eq!(hike.days_left, 0);
        // 到期后没人摘（漏一条 system）也不许回绕成「还剩 42 亿天」
        assert!(hike.age());
        assert_eq!(hike.days_left, 0);
    }

    #[test]
    fn every_status_has_a_word_for_the_trail() {
        // 顶栏流水与 dump 面板都说的是这一句。穷尽匹配（没有 `_`）保证第四种状态
        // 出生时必须当场给它一个人话名字，而不是在 UI 里显示成 `PriceHike`
        assert_eq!(LandStatus::PriceHike.label(), "涨价");
        assert_eq!(LandStatus::Sealed.label(), "查封");
    }
}
