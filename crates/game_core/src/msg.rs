//! 规则层对外广播「发生了什么」，对应 Godot 的 `event_bus.gd`（24 个信号）。
//!
//! 与信号的实质差别：`EventBus.emit()` 是同步递归调用（结算里再触发别的事件会嵌套执行，
//! 这正是 `DEV_PLAN.md:458` 担心的隐患）；消息是**帧末批量投递** —— 本帧写的消息
//! 下一帧才读得到（缓冲在 `First` 交换），天然无重入，因果顺序由 schedule 顺序显式表达。
//!
//! 玩家一律用 `Entity` 指代而不是 `PlayerId`：接收方要拿它查组件，`Entity` 是可直接使用的句柄；
//! `PlayerId` 只在需要跨存档稳定标识时才用（见 §九）。

use bevy::prelude::*;

use crate::board::TileIndex;

/// 某位玩家的回合开始了。阶段推进的唯一入口是 `OnEnter(TurnPhase::TurnStart)`，
/// 所以每回合恰好一条 —— Godot 端 `_advance_and_start` 与 `_start_turn` 各发一次（缺陷 #2）
/// 在这个结构下没有落点。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStarted {
    pub player: Entity,
}

/// 请求掷骰。只有「当前回合玩家」的请求会被响应，UI 出错也不会替别人掷骰。
///
/// 发送者只会是表现层或 AI；规则层从不因为收到它就「推进」到什么 —— 见 §4.6 的红线。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRoll {
    pub player: Entity,
}

/// 骰子结果。`from`/`to` 一起带上，省掉监听方再去查一次移动前的位置。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiceRolled {
    pub player: Entity,
    pub steps: u32,
    pub from: TileIndex,
    pub to: TileIndex,
}

/// 行走开始：`roll_dice_system` 定好路径后发，携带本次移动的完整意图。
/// 表现层由此开始逐格动画；规则层的逻辑位置此刻已指向 `to`
/// （§4.6 红线：core 拥有逻辑位置，渲染只做插值）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkStarted {
    pub player: Entity,
    pub from: TileIndex,
    pub to: TileIndex,
    pub steps: u32,
}

/// 棋子踏入一格（每步一条，`tile` 是踏入的格）。
///
/// 路径恒为主环下标：分支死胡同没有 `TileIndex`，结构上进不来
/// （ADR-001 行为规则 4，见 `BoardDef::walk_path`）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileStepped {
    pub player: Entity,
    pub tile: TileIndex,
}

/// 走完骰子点数、落定在 `to`。落格结算（B3 起 `resolve_landing_system` 的分派）自此开始。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerArrived {
    pub player: Entity,
    pub from: TileIndex,
    pub to: TileIndex,
}

/// 某位玩家的回合结束。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnEnded {
    pub player: Entity,
}

/// 服刑期满自动出狱。逻辑位置一直是监狱对应的**接入点**（`player_data.gd:62-68` 的语义），
/// 所以这里带的是「从哪格继续沿环前进」，不是死胡同格。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerReleased {
    pub player: Entity,
    pub junction: TileIndex,
}

/// 仍在服刑，本回合跳过。`turns_left` 是已经扣掉本回合之后的剩余值。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct JailSkipped {
    pub player: Entity,
    pub turns_left: u32,
}

/// 对局结束。`winner` 为 `None` 表示无人存活。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOver {
    pub winner: Option<Entity>,
}
