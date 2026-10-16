//! 回合状态机。对应 Godot 的 `turn_manager.gd`（492 行、三处 `await`、六处手写阶段守卫）。
//!
//! 源实现里「现在是哪个阶段」是 `var _phase` + 每个回调开头一句 `if _phase != X: return`；
//! 这里 `TurnPhase` 是 `States`，守卫由 `run_if(in_state(..))` 声明式给出，
//! 于是漏写守卫这类错误不复存在。`await token.arrived` 那种「逻辑等渲染」则被拆成
//! `Walking` 阶段（见 §4.6 的所有权红线）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::bank::AtmAccess;
use crate::economy;
use crate::market::Market;
use crate::msg::{
    DiceRolled, GameOver, JailSkipped, PlayerReleased, RequestRoll, TurnEnded, TurnStarted,
    WalkStarted,
};
use crate::player::{Bankrupt, JailSentence, PlayerId, TokenPosition, WalkQueue};
use crate::rng::Rng;
use crate::settle::Books;
use crate::walk::WalkClock;
use crate::world::ActiveBoard;

/// 回合阶段。`Boot` 是初始态：资产到位前什么都不做。
///
/// ⚠️ 初始态的 `OnEnter` **不会**自动触发（Bevy 的 `init_state` 不产生转移），
/// 所以 `begin_game_system` 挂在 `in_state(Boot)` 上而不是 `OnEnter(Boot)`。
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TurnPhase {
    #[default]
    Boot,
    TurnStart,
    WaitRoll,
    /// 逐格行走：`WalkClock` 驱动 `WalkQueue`，每格一条 `TileStepped`
    /// （`walk_step_system`，见 §4.6 的所有权红线）。`walk_step_interval = 0`
    /// 时一帧走完，与 B2b 的瞬时落格行为等价。
    Walking,
    /// 落格结算的分派点。B3 在此接地产/卡牌/银行，目前所有格子都无结算。
    Landed,
    /// 等待玩家决策（买地/升级/确认租金/读卡）。生产者随 B3 的买地流程出现。
    WaitDecision,
    TurnEnd,
    Over,
}

/// 回合轮转顺序。破产玩家留在表里但被跳过（对应 `_advance_and_start` 的 while 循环）。
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct TurnOrder {
    pub order: Vec<Entity>,
    pub cursor: usize,
}

impl TurnOrder {
    pub fn new(order: Vec<Entity>) -> Self {
        Self { order, cursor: 0 }
    }

    pub fn current(&self) -> Option<Entity> {
        self.order.get(self.cursor).copied()
    }

    /// 保证 `current()` 落在可行动玩家上（必要时前进），返回该玩家。
    /// 全员不可行动（或阵容为空）时返回 `None`。
    pub fn current_active(&mut self, is_active: impl Fn(Entity) -> bool) -> Option<Entity> {
        let n = self.order.len();
        for i in 0..n {
            let idx = (self.cursor + i) % n;
            if is_active(self.order[idx]) {
                self.cursor = idx;
                return Some(self.order[idx]);
            }
        }
        None
    }

    /// 前进到下一名可行动玩家（`i = 1..=n`，不含自己），返回该玩家。
    pub fn advance_to_active(&mut self, is_active: impl Fn(Entity) -> bool) -> Option<Entity> {
        let n = self.order.len();
        for i in 1..=n {
            let idx = (self.cursor + i) % n;
            if is_active(self.order[idx]) {
                self.cursor = idx;
                return Some(self.order[idx]);
            }
        }
        None
    }
}

/// 对局元信息。取代 `game_state.gd` 的 `game_id`/`turn_count`。
///
/// `game_id` 不再保留：它只被用在一条错误的 `game_saved.emit`（缺陷 #11）里，
/// 而取时间戳本身就会让对局不可复现。存档需要标识时从 `seed` 推。
///
/// 只 `PartialEq` 不 `Eq`：`price_index` 是浮点，`Eq` 在 `NaN` 上不自反。
/// 存档比较走逐字段 `==`，不需要把整条资源塞进 `HashMap` 的键。
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameSession {
    pub turn_count: u32,
    pub seed: u64,
    /// 物价指数（P2）。开局 1.0，每个回合末按「全员现资产总和 ÷ 全员初始总和」重算，
    /// 不设上限（PC 原版口径，PLAN §一）。哪些钱受它缩放是 `charge::scales_with_price`
    /// 那张穷尽表说了算 —— 股价与神明金额是点名豁免的。
    pub price_index: f32,
}

/// 暂停总闸（`pause_menu.gd` 的 `get_tree().paused`）。规则层只读，写的永远是输入侧。
///
/// 这不违反 §4.6 的红线：红线管的是「表现层替逻辑做决定」，而暂停与掷骰同属玩家意图 ——
/// 它说的是「现在不许推进」，不是「推进到下一步」。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Paused(pub bool);

/// `run_if` 条件：没按下暂停才让回合机的 system 跑。
pub fn not_paused(paused: Res<Paused>) -> bool {
    !paused.0
}

/// `begin_turn_system` 的出口消息打包（四个 `MessageWriter` 参数比挂 `#[allow]` 好读）。
#[derive(SystemParam)]
pub struct TurnWriters<'w> {
    turn_started: MessageWriter<'w, TurnStarted>,
    released: MessageWriter<'w, PlayerReleased>,
    skipped: MessageWriter<'w, JailSkipped>,
    over: MessageWriter<'w, GameOver>,
}

/// `roll_dice_system` 的输入打包：请求流水 + 当前轮转 + 棋盘拓扑
/// （§4.5 表格里「读」的那三样；reader 带两个生命周期，见坑 13）。
#[derive(SystemParam)]
pub struct RollInput<'w, 's> {
    requests: MessageReader<'w, 's, RequestRoll>,
    order: Res<'w, TurnOrder>,
    active: Res<'w, ActiveBoard>,
}

/// 回合开始的唯一入口：跳过破产者 → 处理服刑 → 否则等掷骰。
///
/// 服刑的递减规则与 `player_data.gd:72-80` 一致：`turns_left` 每次回合开始减一，
/// 减到 0 的那一回合**当场出狱并可掷骰**（即判 3 回合 = 跳过 2 个回合）。
pub fn begin_turn_system(
    mut commands: Commands,
    mut order: ResMut<TurnOrder>,
    players: Query<(Entity, Option<&Bankrupt>, &TokenPosition), With<PlayerId>>,
    jailed: Query<Option<&JailSentence>, With<PlayerId>>,
    mut atm: ResMut<AtmAccess>,
    mut writers: TurnWriters,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let alive = |e: Entity| {
        players
            .get(e)
            .map(|(_, bankrupt, _)| bankrupt.is_none())
            .unwrap_or(false)
    };

    let Some(player) = order.current_active(alive) else {
        // 阵容为空，或全员已破产（正常流程下 end_turn_system 会先一步进 Over）
        writers.over.write(GameOver { winner: None });
        next.set(TurnPhase::Over);
        return;
    };
    // ATM 资格活到「持有人自己的下一回合开始」为止，不是任何人的回合开始都清。
    // 路过那一瞬间正是回合机最赶的时候（行走未完、落格已结算、下一位紧接着开始），
    // 只给那一帧的窗口等于按不到；放到下一次行动之前，玩家才有真正能点的一段 ——
    // 与证券面板「掷骰前可操作」是同一个窗口。
    if atm.0 == Some(player) {
        atm.0 = None;
    }
    let Ok((_, _, pos)) = players.get(player) else {
        return;
    };

    // 回合开始即扣一格，所以 turns_left==1 表示「最后一格服刑完，本回合可行动」
    let jail_left = jailed.get(player).ok().flatten().map(|s| s.turns_left);

    match jail_left {
        Some(left) if left > 1 => {
            commands.entity(player).insert(JailSentence {
                turns_left: left - 1,
            });
            // Godot `turn_manager.gd:188-189`：先让 UI 显示「谁在服刑」，再结束回合
            writers.turn_started.write(TurnStarted { player });
            writers.skipped.write(JailSkipped {
                player,
                turns_left: left - 1,
            });
            next.set(TurnPhase::TurnEnd);
        }
        Some(_) => {
            commands.entity(player).remove::<JailSentence>();
            writers.released.write(PlayerReleased {
                player,
                junction: pos.tile,
            });
            writers.turn_started.write(TurnStarted { player });
            next.set(TurnPhase::WaitRoll);
        }
        None => {
            writers.turn_started.write(TurnStarted { player });
            next.set(TurnPhase::WaitRoll);
        }
    }
}

/// 掷骰并定下本次移动。只在 `WaitRoll` 里跑，且只认「当前回合玩家」的请求 ——
/// 这是缺陷 #7 的修复点：Godot 的点数由表现层 `board.gd:704` 发出，逻辑因此离不开渲染、
/// headless 根本掷不了骰。
///
/// 逻辑位置（`TokenPosition`）在掷骰的一刻就落定到 `to`；此后 `Walking` 态只是把
/// 这次移动按 `GameRules.walk_step_interval` 展开成逐格时间线（`TileStepped`），
/// 表现层拿它做插值 —— 任何等待动画的 `await` 都不存在了。
pub fn roll_dice_system(
    mut input: RollInput,
    mut rng: ResMut<Rng>,
    mut clock: ResMut<WalkClock>,
    mut walkers: Query<(&mut TokenPosition, &mut WalkQueue), With<PlayerId>>,
    mut rolled: MessageWriter<DiceRolled>,
    mut started: MessageWriter<WalkStarted>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let Some(current) = input.order.current() else {
        return;
    };
    // 全部读完再判断：`any()` 会短路，没读完的请求下一帧又被读出来 → 多掷一次
    let mut asked = false;
    for r in input.requests.read() {
        asked |= r.player == current;
    }
    if !asked {
        return;
    }
    let Ok((mut pos, mut queue)) = walkers.get_mut(current) else {
        return;
    };

    let from = pos.tile;
    let steps = rng.roll_die();
    let path = input.active.def.walk_path(from.0 as i64, steps as i64);
    let to = path.last().copied().unwrap_or(from);
    pos.tile = to;
    *queue = WalkQueue {
        path: path.into(),
        done: 0,
        from,
    };
    clock.elapsed = 0.0;
    rolled.write(DiceRolled {
        player: current,
        steps,
        from,
        to,
    });
    started.write(WalkStarted {
        player: current,
        from,
        to,
        steps,
    });
    // 步数为 0（理论不可达：骰子 ≥1）也必须能出 Walking，否则状态机会卡死
    next.set(if queue.path.is_empty() {
        TurnPhase::Landed
    } else {
        TurnPhase::Walking
    });
}

/// 回合收尾：发 `TurnEnded`、回合计数、判终局，否则轮转下一位。
pub fn end_turn_system(
    mut order: ResMut<TurnOrder>,
    mut session: ResMut<GameSession>,
    players: Query<(Entity, Option<&Bankrupt>), With<PlayerId>>,
    mut ended: MessageWriter<TurnEnded>,
    mut over: MessageWriter<GameOver>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let Some(player) = order.current() else {
        return;
    };
    ended.write(TurnEnded { player });
    session.turn_count += 1;

    let living: Vec<Entity> = players
        .iter()
        .filter(|(_, bankrupt)| bankrupt.is_none())
        .map(|(e, _)| e)
        .collect();

    // 只剩一人（或零人）→ 终局。对应 `_check_game_over`
    if living.len() <= 1 {
        over.write(GameOver {
            winner: living.first().copied(),
        });
        next.set(TurnPhase::Over);
        return;
    }

    order.advance_to_active(|e| living.contains(&e));
    next.set(TurnPhase::TurnStart);
}

/// 每个回合结束重算物价指数（P2③）。
///
/// 排在 `TurnEnd` 的**最后**，因为分子里的股票市值要等行情 tick 完才是今天的数 ——
/// 跑在 `market_tick_on_turn_end_system` 之前会拿到昨天的价，那一帧的指数就说不清是哪天的。
///
/// 一天一个数、整场共用：它是世界的状态，不是某个玩家的状态，所以不进修正登记处
/// （那里每条都要有个 bearer），而是管线第一步直接读 [`GameSession::price_index`]。
pub fn update_price_index_system(mut books: Books, market: Market) {
    let previous = books.session.price_index;
    let total = books.total_assets(previous, |id| market.price(id).unwrap_or(0.0));
    let initial = books.initial_assets();
    let index = economy::price_index(total, initial);
    books.session.price_index = index;
    debug!("物价指数 {previous:.3} → {index:.3}（现资产 {total} / 初始 {initial}）");
}

/// 终局只负责记账与日志；胜负细节在 `GameOver` 消息里，UI 自己读。
pub fn game_over_system(session: Res<GameSession>) {
    info!(
        "对局结束：{} 回合（种子 {:#018x}）",
        session.turn_count, session.seed
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一批假 `Entity`：测试里只比较句柄，不碰世界。
    fn ents(n: usize) -> Vec<Entity> {
        (0..n as u32)
            .map(|i| Entity::from_raw_u32(i).expect("索引不该撞上 Entity 的保留值"))
            .collect()
    }

    #[test]
    fn current_active_falls_back_to_the_first_alive_one() {
        let e = ents(3);
        let mut order = TurnOrder::new(e.clone());
        // 0 号破产：应停在 1 号，且幂等
        assert_eq!(order.current_active(|x| x != e[0]), Some(e[1]));
        assert_eq!(order.current_active(|x| x != e[0]), Some(e[1]));
    }

    #[test]
    fn advance_skips_bankrupt_runs() {
        let e = ents(4);
        let mut order = TurnOrder::new(e.clone());
        let alive = |x: Entity| x != e[1] && x != e[2];
        assert_eq!(order.advance_to_active(alive), Some(e[3]));
        assert_eq!(order.advance_to_active(alive), Some(e[0]), "应绕回");
        assert_eq!(order.current(), Some(e[0]));
    }

    #[test]
    fn advance_reports_none_when_nobody_is_left() {
        let e = ents(3);
        let mut order = TurnOrder::new(e.clone());
        assert_eq!(order.advance_to_active(|_| false), None);
        assert_eq!(order.current_active(|_| false), None);
        assert_eq!(TurnOrder::new(vec![]).current_active(|_| true), None);
    }
}
