//! 定步时行走：把「掷出 N 点」展开成逐格推进。
//!
//! 对应 Godot `player_token.gd:99-112` 的逐格 0.18s + `turn_manager.gd` 里
//! `await token.arrived`（逻辑等渲染）的拆解：core 拥有逻辑位置与步时
//! （§4.6 的所有权红线），`Walking` 态由 [`walk_step_system`] 驱动 ——
//! 它按 [`WalkClock`] 累计的时间换算「此刻该走到第几步」，逐格发
//! `TileStepped`，步数耗尽发 `PlayerArrived` 并交棒给落格结算。
//!
//! 表现层（B4 的 `TokenVisual`）只订阅消息做视觉插值，**永远不向 core
//! 发「推进回合」的消息**；想让棋子停走或加速，改 [`WalkClock`] 或
//! `GameRules.walk_step_interval` 即可（后者正是 B7 设置项的挂钩点）。

use bevy::prelude::*;

use crate::msg::{PlayerArrived, TileStepped};
use crate::player::{PlayerId, WalkQueue};
use crate::rules::GameRules;
use crate::turn::TurnPhase;

/// 行走时钟：本段行走已累计的秒数。
///
/// 只在 `Walking` 态由 [`advance_walk_clock_system`] 按虚拟时间推进
/// （行走互斥，同一时刻至多一人在走，全局一个就够）；掷骰的一刻清零。
/// 测试要精确控制节奏时直接改写 `elapsed` —— 这正是 §4.6 认可的做法。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Default)]
pub struct WalkClock {
    pub elapsed: f32,
}

/// 时钟换算应到步数：`floor(elapsed / interval)`，封顶 `total`；
/// `interval <= 0` 视为「跳过动画」，一步到位。纯函数，便于表驱动测试。
fn steps_due(elapsed: f32, interval: f32, total: usize) -> usize {
    if interval <= 0.0 {
        return total;
    }
    ((elapsed / interval).floor() as usize).min(total)
}

/// 行走时钟的推进者：挂在 `First`、只在 `Walking` 态跑，把帧间真实 Δt
/// 累计给 [`WalkClock`]。真实客户端经 `CoreGamePlugin` 自动就位。
pub fn advance_walk_clock_system(time: Res<Time>, mut clock: ResMut<WalkClock>) {
    clock.elapsed += time.delta_secs();
}

/// 逐格推进：消费 [`WalkClock`]，把应到而未到的步数补齐（每步一条
/// `TileStepped`），队列走空即发 `PlayerArrived` 并转入 `Landed`。
///
/// 只处理 `path` 非空的队列 —— B2c 里行走互斥（当前回合玩家才可能被
/// `roll_dice_system` 填队列），未来若出现并行行走，这里按「各自推进、
/// 先到站者先交棒」处理即可。
pub fn walk_step_system(
    clock: Res<WalkClock>,
    rules: Res<GameRules>,
    mut walkers: Query<(Entity, &mut WalkQueue), With<PlayerId>>,
    mut stepped: MessageWriter<TileStepped>,
    mut arrived: MessageWriter<PlayerArrived>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    for (entity, mut queue) in walkers.iter_mut() {
        if queue.path.is_empty() {
            continue;
        }
        let due = steps_due(clock.elapsed, rules.walk_step_interval, queue.total());
        let mut last = queue.from;
        while (queue.done as usize) < due {
            let tile = queue.path.pop_front().expect("done < total 保证队列非空");
            last = tile;
            queue.done += 1;
            stepped.write(TileStepped {
                player: entity,
                tile,
            });
        }
        if queue.path.is_empty() {
            arrived.write(PlayerArrived {
                player: entity,
                from: queue.from,
                to: last,
            });
            next.set(TurnPhase::Landed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn due_tracks_the_clock_one_step_per_interval() {
        // 0.18s 一步：时钟没走不动，走满一个 interval 恰好一步
        assert_eq!(steps_due(0.0, 0.18, 6), 0);
        assert_eq!(steps_due(0.17, 0.18, 6), 0);
        assert_eq!(steps_due(0.18, 0.18, 6), 1);
        assert_eq!(steps_due(0.36, 0.18, 6), 2);
        assert_eq!(steps_due(9.99, 0.18, 6), 6, "应封顶在总步数");
    }

    #[test]
    fn zero_interval_means_instant_arrival() {
        assert_eq!(steps_due(0.0, 0.0, 4), 4);
        assert_eq!(steps_due(0.0, -1.0, 4), 4, "负值同样按跳过动画处理");
    }
}
