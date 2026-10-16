//! 点券格：一次性拾取 + 按回合冷却刷新（设计口径见 `.agents/PLAN.md` §五）。
//!
//! 三档面额 10/30/50 **写在格子上而不是抽出来的**：随机抽会让三档的期望值全部相同，
//! 「这一段路值得走」的地图梯度当场失效。不确定感另有来源 —— 谁先抢到这一段、
//! 下一轮什么时候长回来，那是争夺而不是抽奖。
//!
//! 现金与点券是两套账：这一格发的钱只走 [`PointsChanged`]，一条 `CashChanged` 都不产生，
//! 所以 B3 的现金审计看不见它（PLAN §一 拍板「永不通兑」）。

use std::collections::HashMap;

use bevy::prelude::*;

use crate::board::TileIndex;
use crate::landing::{Landed, Landing, LandingDomain};
use crate::settle::Books;
use crate::turn::GameSession;

/// 已取走的点券箱：`格 → 哪一回合重新出现`。
///
/// 为什么是资源而不是道路格上的实体：AGENTS.md 硬约束第 4 条「道路格不建实体」，
/// 而这里每个格的全部状态就是一个整数。P5 的路障/地雷/定时炸弹是带生命周期的个体，
/// 那才是需要补一条 ADR 再谈 occupancy 的东西（PLAN §七）。
#[derive(Resource, Debug, Default, Clone, PartialEq, Eq)]
pub struct CouponBoxes {
    ready_at: HashMap<TileIndex, u32>,
}

impl CouponBoxes {
    /// 这一格现在能不能捡：没被捡过，或冷却已过。
    ///
    /// `turn` 是 `GameSession::turn_count`，数的是**全场合计**的回合数 —— 与
    /// 「几天后重新长出来」同一口径（一天就是一个回合，行情也照它 tick）。
    pub fn is_ready(&self, tile: TileIndex, turn: u32) -> bool {
        self.ready_at
            .get(&tile)
            .is_none_or(|&ready_at| turn >= ready_at)
    }

    /// 记一次取走：`turn + refresh` 之后重新出现。
    pub fn take(&mut self, tile: TileIndex, turn: u32, refresh: u32) {
        self.ready_at.insert(tile, turn + refresh);
    }

    /// 被取走过的格子数（只给日志与测试看，规则不读它）。
    pub fn taken(&self) -> usize {
        self.ready_at.len()
    }
}

/// 踩到点券箱：还在就捡走并记冷却；已经被谁抢了（或冷却没过）就什么都不发。
///
/// 刻意**不挂 `Pending`** —— 捡箱子没有「要不要」这个决策，弹一张收据只会把回合机
/// 停在没人回答的地方；到账由 `PointsChanged` 说话，HUD 与顶栏各读它一条。
pub fn settle_coupon_landing(
    landing: Res<Landing>,
    mut books: Books,
    mut boxes: ResMut<CouponBoxes>,
    session: Res<GameSession>,
) {
    let Some(Landed {
        player,
        tile,
        domain: LandingDomain::Coupon,
        ..
    }) = landing.0
    else {
        return;
    };

    // 先读数据再动手：`def` 挂着 `books.active` 的不可变借用，
    // 而下面既要把冷却写进资源、又要点券进账本
    let Some(def) = books.active.def.tile(tile.0 as i64) else {
        return;
    };
    let Some(amount) = def.data.coupon else {
        // `BoardDef::validate` 已在加载期拦下这种情况；走到这里说明校验漏了一条，
        // 但不该由落格结算来掀桌子
        warn!("点券格 {}({}) 没有面额，本回合不发", tile.0, def.name);
        return;
    };
    let refresh = def.data.refresh.unwrap_or(books.rules.coupon_refresh_turns);

    if !boxes.is_ready(tile, session.turn_count) {
        debug!(
            "点券箱 {} 已被取走，冷却到第 {} 回合",
            tile.0,
            session.turn_count + refresh
        );
        return;
    }
    boxes.take(tile, session.turn_count, refresh);
    books.award_points(player, amount);
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: TileIndex = TileIndex(3);
    const B: TileIndex = TileIndex(10);

    #[test]
    fn a_box_nobody_touched_is_ready_from_the_first_turn() {
        let boxes = CouponBoxes::default();
        assert!(boxes.is_ready(A, 0));
        assert_eq!(boxes.taken(), 0);
    }

    #[test]
    fn taking_a_box_locks_it_until_the_refresh_turn() {
        let mut boxes = CouponBoxes::default();
        boxes.take(A, 4, 6);
        for turn in 4..10 {
            assert!(!boxes.is_ready(A, turn), "第 {turn} 回合还不该长回来");
        }
        assert!(boxes.is_ready(A, 10), "第 4 + 6 = 10 回合起重新出现");
    }

    /// 冷却是一格一条记录。做成全局一个数的话，「谁先抢到这一段」就变成
    /// 「抢到一个等于全盘都空」—— 那不是争夺，是清场。
    #[test]
    fn the_cooldown_is_per_tile() {
        let mut boxes = CouponBoxes::default();
        boxes.take(A, 0, 6);
        assert!(!boxes.is_ready(A, 1));
        assert!(boxes.is_ready(B, 1), "别的格子不该跟着空");
        assert_eq!(boxes.taken(), 1);
    }
}
