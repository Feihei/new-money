//! 定步时行走：把「掷出 N 点」展开成逐格推进。
//!
//! 对应 Godot `player_token.gd:99-112` 的逐格 0.18s + `turn_manager.gd` 里
//! `await token.arrived`（逻辑等渲染）的拆解：core 拥有逻辑位置与步时
//! （§4.6 的所有权红线），`Walking` 态由 [`walk_step_system`] 驱动 ——
//! 它按 [`WalkClock`] 累计的时间换算「此刻该走到第几步」，逐格发
//! `TileStepped`，步数耗尽发 `PlayerArrived` 并交棒给落格结算。
//!
//! P5 起走步循环还负责**道路放置物**（ADR-002 §7）：每一步读一次 occupancy，
//! 地雷/恶犬/定时炸弹的触发会在这里清空剩余路径并停住。这里只做「与还能不能走有关」
//! 的状态改动（炸弹步数、携带者、消耗地雷）；住院与拆房这些后果由
//! `occupancy::resolve_halt_system` 在 `Halted` 阶段应用。
//!
//! 表现层（B4 的 `TokenVisual`）只订阅消息做视觉插值，**永远不向 core
//! 发「推进回合」的消息**；想让棋子停走或加速，改 [`WalkClock`] 或
//! `GameRules.walk_step_interval` 即可（后者正是 B7 设置项的挂钩点）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::{BoardDef, TileIndex};
use crate::msg::{BombTransferred, PlayerArrived, TileStepped, WalkHalted};
use crate::occupancy::{
    BOMB_STEPS, HaltCause, Occupancy, OccupancyAt, OccupancyKind, Trigger, WalkHalt,
    first_roadblock, trigger_of,
};
use crate::player::{Heading, Mount, TokenPosition, WalkQueue, Walker};
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

/// 「这一回合到底走几格」的唯一一份算术：乌龟夹步 → 方向 → 拓扑路径 → 路障截断。
///
/// 掷骰与遥控骰子两条入口都走这里，所以夹子与截断不会各修一半。`rolled` 永远是
/// **骰面（或遥控骰子选定的点数）**，`WalkPlan::moved` 才是实际走的格数 ——
/// `DiceRolled.steps` 保持前者（P3d④b 与 ADR-002 §7.1）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkPlan {
    /// 骰子（或遥控骰子）给出的点数，不因夹步/路障而改变。
    pub rolled: u32,
    /// 实际要走的格数（乌龟卡夹到 1）。
    pub moved: u32,
    /// 不截断时本该落到的格（`DiceRolled.to` 用它）。
    pub rolled_to: TileIndex,
    /// 截断路障之后的实际路径。
    pub path: Vec<TileIndex>,
    /// 实际终点（`WalkStarted.to`、`TokenPosition.tile` 用它）。
    pub to: TileIndex,
    /// 乌龟计时扣完这一回合之后还剩几回合；`None` = 身上本来没有夹步。
    pub cap_left: Option<u32>,
}

pub fn plan_walk(
    def: &BoardDef,
    from: TileIndex,
    heading: Heading,
    rolled: u32,
    cap: Option<u32>,
    occupancies: &Query<(Entity, &Occupancy)>,
    pass_roadblocks: bool,
) -> WalkPlan {
    let moved = match cap {
        Some(_) => rolled.min(1),
        None => rolled,
    };
    let rolled_path = def.walk_path(from.0 as i64, rolled as i64 * heading.sign());
    let rolled_to = rolled_path.last().copied().unwrap_or(from);
    let mut path = if moved == rolled {
        rolled_path
    } else {
        def.walk_path(from.0 as i64, moved as i64 * heading.sign())
    };
    // 路障截断对小偷豁免（P12c）：PLAN §三 那句「唯一能穿路障」落在这一个布尔上，
    // 而不是让两个调用点各写一遍判据。他穿过去的那一格在走步循环里被偷走 ——
    // 「穿」与「偷」是同一次经过的两句话
    if !pass_roadblocks && let Some(at) = first_roadblock(occupancies, &path) {
        path.truncate(at + 1);
    }
    let to = path.last().copied().unwrap_or(from);
    WalkPlan {
        rolled,
        moved,
        rolled_to,
        path,
        to,
        cap_left: cap.map(|n| n.saturating_sub(1)),
    }
}

/// 走一步要读写的世界。打包成参数包是仓库房规（§六 坑 8）：这些字段一起回答
/// 「谁在走、这一格有什么、停下了该怎么记」。
#[derive(SystemParam)]
pub struct WalkDesk<'w, 's> {
    clock: Res<'w, WalkClock>,
    rules: Res<'w, GameRules>,
    /// 玩家名字那一栏只用来给炸弹转让排序？不需要 —— 这里只带位置与载具。
    walkers: Query<
        'w,
        's,
        (
            Entity,
            &'static mut WalkQueue,
            &'static mut TokenPosition,
            &'static Mount,
        ),
        With<Walker>,
    >,
    /// 一步一拍地读；放置物全场个位数，线性扫比再挂索引划算（ADR-002 §3 明确不要第二份索引）。
    occupancies: Query<'w, 's, (Entity, &'static mut Occupancy)>,
    /// 「相遇」要判**另一个行动者**此刻在哪一格（P12 起含受雇的恶人：他站在路上就是一块
    /// 会挡路的肉，炸弹经过他那一格就该转让）。
    ///
    /// **`Without<Detention>`**：被关着的人不在路上。他的逻辑位置停在接入点（那是
    /// 「放出来之后从哪格继续走」的读数，见 `player::TokenPosition`），但他人其实在那条
    /// 死胡同尽头 —— 让他站在门口接住一颗经过的炸弹，说的是「路过监狱门口的人替囚犯扛雷」，
    /// 而那不是任何一条规则。P12 之前这条过滤不改变任何结果（那时场上只有正在走路的人
    /// 与没被关的人）；恶人一上场，接入点上就会常年站着几位囚犯，所以这一格必须显式回答。
    ///
    /// 只要 `Entity`：位置从上面那条 `walkers` 借读，因为 `TokenPosition` 在那里已经是
    /// `&mut` —— 同一条 system 里并存两条读它的查询会撞成借用冲突（§6.1 坑 33 同族，
    /// 而这条在旧代码里是靠「拿 `PlayerId` 再回查 `walkers`」绕开的，P12 只是把那个
    /// 「谁」从玩家换成了行动者）。排一次序（按实体）：同格站着两个人时「转让给谁」要稳定。
    movers: Query<'w, 's, Entity, (With<Walker>, Without<crate::player::Detention>)>,
    commands: Commands<'w, 's>,
    stepped: MessageWriter<'w, TileStepped>,
    arrived: MessageWriter<'w, PlayerArrived>,
    halted: MessageWriter<'w, WalkHalted>,
    /// 炸弹经手人变更（ADR-002 §8）。与 `halt` 一样是走步循环写下的现场记录。
    bomb_transferred: MessageWriter<'w, BombTransferred>,
    /// 走步循环只记「为什么停」，后果由 `occupancy::resolve_halt_system` 应用 ——
    /// 与消息分开是因为那一侧要读 `Books`。
    halt: ResMut<'w, WalkHalt>,
    next: ResMut<'w, NextState<TurnPhase>>,
    /// 他是哪一种行动者（P12c）。**只在这一处读**：小偷对地雷/路障的「偷」必须与
    /// 「踩到即炸」同帧裁定 —— 消息是帧末投递的，等一条 reader 赶到，人已经回笼了。
    /// 走步循环因此认识「小偷」这一个词（不认识他的雇主、更不认识道具栏）：
    /// 偷走的东西给谁，由 `villain::thief_deliver_system` 凭消息里的 `Hired` 回答。
    villains: Query<'w, 's, &'static crate::villain::Villain>,
    thieved: MessageWriter<'w, crate::msg::ItemThieved>,
}

/// 逐格推进：消费 [`WalkClock`]，把应到而未到的步数补齐（每步一条
/// `TileStepped`），队列走空即发 `PlayerArrived` 并转入 `Landed`。
///
/// **中断不是到站**（ADR-002 §7）：踩到地雷/恶犬，或身上的定时炸弹走满 38 步时，
/// 清空剩余路径、把逻辑位置改回停下的那一格、发 [`WalkHalted`] 并转入 `Halted`；
/// 这一格**不结算静态格型**，也**不发** `PlayerArrived`。
pub fn walk_step_system(mut desk: WalkDesk<'_, '_>) {
    // 相遇快照：这一帧谁站在哪儿。行走互斥，所以除正在走的人以外不会有人移动；
    // 正在走的人自己那一格每步更新，不影响「别人在哪」。
    // 排一次序（按实体，也就是按生成号）：同格站着两个人时「转让给谁」要稳定，
    // 而 archetype 的迭代序在 0.20 里不被保证
    let mut snapshot: Vec<(Entity, crate::board::TileIndex)> = desk
        .movers
        .iter()
        .filter_map(|entity| {
            desk.walkers
                .get(entity)
                .ok()
                .map(|(_, _, pos, _)| (entity, pos.tile))
        })
        .collect();
    snapshot.sort();

    let due_rules = *desk.rules;
    for (entity, mut queue, mut pos, mount) in desk.walkers.iter_mut() {
        if queue.path.is_empty() {
            continue;
        }
        let due = steps_due(
            desk.clock.elapsed,
            due_rules.walk_step_interval,
            queue.total(),
        );
        let mut last = queue.from;
        let mut stopped = false;
        // 小偷的两个专属行为都在这一趟循环里裁（P12c）：穿路障不截断（掷骰入队时已豁免，
        // 见 `plan_walk`）与「偷走路上的雷/路障」。偷必须先于「踩到即炸」的裁定 ——
        // 同一帧里他先下手，雷就炸不到他
        let thief = desk
            .villains
            .get(entity)
            .is_ok_and(|v| v.kind == crate::villain::VillainKind::Thief);
        while (queue.done as usize) < due {
            let tile = queue.path.pop_front().expect("done < total 保证队列非空");
            last = tile;
            queue.done += 1;
            desk.stepped.write(TileStepped {
                player: entity,
                tile,
            });

            // ===== 这一格有什么 =====
            let mut mine = None;
            let mut dog = None;
            let mut bomb = None;
            let mut roadblock = None;
            // 先拾取：OnTile 的炸弹改成 Carried，然后同一步就开始计数
            for (occ_entity, occ) in desk.occupancies.iter() {
                if occ.at != OccupancyAt::OnTile(tile) || trigger_of(occ.kind) != Trigger::Pass {
                    continue;
                }
                match occ.kind {
                    OccupancyKind::Mine => mine = Some(occ_entity),
                    OccupancyKind::Dog => dog = Some(occ_entity),
                    OccupancyKind::TimeBomb => bomb = Some(occ_entity),
                    // 路障在掷骰入队时已经截断；走到这里只可能是路径被别的东西改了。
                    // 它不中断也不消耗 —— 静态格照常结算（ADR-002 §5/§6）。
                    // 小偷例外（P12c）：他路过顺手偷走，发消息给雇主收货
                    OccupancyKind::Roadblock => {
                        if thief {
                            roadblock = Some(occ_entity);
                        }
                    }
                    OccupancyKind::Chest => {}
                    // 神：落脚触发，上面那道 `trigger_of` 已经把它挡在外面。
                    // 这一格不是可达分支，是**这张表要求的表态** —— 谁哪天把神改成 Pass
                    // 触发，这里会立刻变成「走路时被附身」那条没人见过的路径，
                    // 而那时必须重新读一遍这段循环。
                    OccupancyKind::God(_) => {}
                }
            }
            if let Some(bomb_entity) = bomb
                && let Ok((_, mut occ)) = desk.occupancies.get_mut(bomb_entity)
            {
                occ.at = OccupancyAt::Carried {
                    by: entity,
                    steps_left: BOMB_STEPS,
                };
            }

            // 计数：只数走路进入的格。拾取那一步算第 1 步（P5 测试钉死的口径）
            let mut exploded = false;
            let carried: Vec<(Entity, u32)> = desk
                .occupancies
                .iter()
                .filter_map(|(occ_entity, occ)| match occ.at {
                    OccupancyAt::Carried { by, steps_left } if by == entity => {
                        Some((occ_entity, steps_left))
                    }
                    _ => None,
                })
                .collect();
            for (bomb_entity, steps_left) in carried {
                if steps_left > 1 {
                    if let Ok((_, mut occ)) = desk.occupancies.get_mut(bomb_entity)
                        && let OccupancyAt::Carried { by, steps_left } = occ.at
                    {
                        occ.at = OccupancyAt::Carried {
                            by,
                            steps_left: steps_left - 1,
                        };
                    }
                } else {
                    // 第 38 步：爆。实体 despawn，后果由 `resolve_halt_system` 应用
                    desk.commands.entity(bomb_entity).despawn();
                    exploded = true;
                }
            }

            // 相遇：路径经过另一名玩家当前所在格即转让（ADR-002 §8）。
            // 先爆炸后转让 —— 已经爆掉的炸弹不该在新主人手上再响一次。
            if !exploded
                && let Some((target, _)) = snapshot
                    .iter()
                    .find(|(other, at)| *other != entity && *at == tile)
            {
                let carried: Vec<(Entity, u32)> = desk
                    .occupancies
                    .iter()
                    .filter_map(|(occ_entity, occ)| match occ.at {
                        OccupancyAt::Carried { by, steps_left } if by == entity => {
                            Some((occ_entity, steps_left))
                        }
                        _ => None,
                    })
                    .collect();
                for (bomb_entity, steps_left) in carried {
                    if let Ok((_, mut occ)) = desk.occupancies.get_mut(bomb_entity) {
                        occ.at = OccupancyAt::Carried {
                            by: *target,
                            steps_left,
                        };
                    }
                    desk.bomb_transferred.write(BombTransferred {
                        from: entity,
                        to: *target,
                        tile,
                        steps_left,
                    });
                }
            }

            // 地雷与恶犬：走进去就中断。载具有两种命运 —— 踩雷被毁，
            // 撞狗把狗撞飞（PLAN §三「载具」行）。
            // 小偷先下手（P12c）：他踩到的雷不发火、顺手搬走 —— 拆掉的东西炸不到人，
            // 这一步也不停。恶犬不是放置物（它是活的），照咬。
            if let Some(mine_entity) = mine {
                if thief {
                    desk.commands.entity(mine_entity).despawn();
                    desk.thieved.write(crate::msg::ItemThieved {
                        thief: entity,
                        item: crate::items::ItemKind::Mine,
                        tile,
                    });
                } else {
                    desk.commands.entity(mine_entity).despawn();
                }
            }
            if thief && let Some(roadblock_entity) = roadblock {
                desk.commands.entity(roadblock_entity).despawn();
                desk.thieved.write(crate::msg::ItemThieved {
                    thief: entity,
                    item: crate::items::ItemKind::Roadblock,
                    tile,
                });
            }
            if let Some(dog_entity) = dog
                && mount.is_vehicle()
            {
                desk.commands.entity(dog_entity).despawn();
            }

            // 优先级：炸弹爆炸 > 地雷 > 恶犬（同一格最多一件，但携带中的炸弹不占格）。
            // 小偷偷走的那颗雷不算「踩到」（P12c）：东西都没了，人照常往前走
            let mine_hit = mine.is_some() && !thief;
            let dog_hit = dog.is_some();
            let cause = if exploded {
                Some(HaltCause::TimeBomb)
            } else if mine_hit {
                Some(HaltCause::Mine)
            } else if dog_hit && !mount.is_vehicle() {
                Some(HaltCause::Dog)
            } else {
                None
            };

            if let Some(cause) = cause {
                // 逻辑位置必须改回停下的这一格：B2c 的「掷骰即落定终点」只在走完全程时成立
                pos.tile = tile;
                queue.path.clear();
                desk.halt.last = Some(WalkHalted {
                    player: entity,
                    tile,
                    cause,
                });
                desk.halted.write(WalkHalted {
                    player: entity,
                    tile,
                    cause,
                });
                desk.next.set(TurnPhase::Halted);
                stopped = true;
                break;
            }
        }
        if !stopped && queue.path.is_empty() {
            desk.arrived.write(PlayerArrived {
                player: entity,
                from: queue.from,
                to: last,
            });
            desk.next.set(TurnPhase::Landed);
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
