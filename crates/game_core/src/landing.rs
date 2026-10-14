//! 落格分派与决策闭环 —— Godot `turn_manager.gd` 的 `_land_and_end` + 六个决策方法。
//!
//! 拆成六段流水线，每段只做一件事（`Landed` 这一帧内按顺序跑完，跨帧只发生一次状态转移）：
//!
//! ```text
//! classify ─→ property ─┐
//!           ─→ bank     ├─→ close（有 pending 就等回答，没有就收摊）
//!           ─→ market   │
//!           ─→ card ────┘
//! ```
//!
//! 三个刻意的结构约束：
//! - **「踩到哪格是什么」只解释一次**（[`classify_landing_system`] → [`Landing`]）。
//!   四个域的 system 各读各的，不会出现四份「这格是不是地产」的理解。
//! - **域的结算 system 不碰 `NextState`**：只有 [`close_landing_system`] 有权决定
//!   下一步去哪，判据是「有没有人还挂着待决事项」。表现层永远推不动回合这条红线
//!   （§4.6）在这里的另一面就是：任何单个结算 system 也推不动回合。
//! - **`TileKind` 用 `match` 穷尽**（缺陷 #8）：新增格类型时漏写分支直接编译失败，
//!   不像 Godot 的字符串 `match` 那样静默落到 `default`。

use bevy::prelude::*;

use crate::board::{TileIndex, TileKind};
use crate::land::LandId;
use crate::msg::{Decision, DecisionChosen, LandBought, LandUpgraded};
use crate::player::{JailSentence, PlayerId, TokenPosition};
use crate::settle::Books;
use crate::turn::{TurnOrder, TurnPhase};
use crate::world::ActiveBoard;

/// 本帧落格的结算目标。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Landed {
    pub player: Entity,
    pub tile: TileIndex,
    pub kind: TileKind,
    /// 该格绑定的土地（只有 `TileKind::Property` 有，`BoardDef::validate` 保证）。
    pub land: Option<LandId>,
}

/// 「这一帧踩到了什么」的唯一一份事实。由流水线第一段写一次，后面各段只读。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Landing(pub Option<Landed>);

/// 流水线第一段：把当前玩家的逻辑位置翻译成 `(格类型, 土地)`。
pub fn classify_landing_system(
    order: Res<TurnOrder>,
    positions: Query<&TokenPosition, With<PlayerId>>,
    active: Res<ActiveBoard>,
    mut landing: ResMut<Landing>,
) {
    landing.0 = order.current().and_then(|player| {
        let pos = positions.get(player).ok()?;
        let tile = active.def.tile(pos.tile.0 as i64)?;
        Some(Landed {
            player,
            tile: pos.tile,
            kind: tile.kind,
            land: tile.land_id.map(LandId),
        })
    });
}

/// 待决事项的种类。带上 `LandId` 是让 UI 知道报价关于哪块地（`Card` 没有对象）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pending {
    /// 无主地：买还是不买
    Buy(LandId),
    /// 自己的垄断地：升还是不升
    Upgrade(LandId),
    /// 租金已扣，只需玩家看一眼收据
    RentNotice(LandId),
    /// 卡面已结算，只需玩家读完
    Card,
}

/// 系统状态：现在正等谁的什么回答。
///
/// 取代 Godot 的三个 `_pending_*` 字段（`turn_manager.gd:50-52`：一个 `int`、
/// 一个 `Resource`、一个 `String`，其中 `_pending_player` 还持有对象引用）。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct PendingDecision {
    pub player: Option<Entity>,
    pub about: Option<Pending>,
}

impl PendingDecision {
    pub fn offer(&mut self, player: Entity, about: Pending) {
        self.player = Some(player);
        self.about = Some(about);
    }

    pub fn active(&self) -> Option<(Entity, Pending)> {
        match (self.player, self.about) {
            (Some(p), Some(about)) => Some((p, about)),
            _ => None,
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

/// 流水线最后一段：有人被挂着等回答就停在 `WaitDecision`，否则收摊。
pub fn close_landing_system(pending: Res<PendingDecision>, mut next: ResMut<NextState<TurnPhase>>) {
    next.set(if pending.active().is_some() {
        TurnPhase::WaitDecision
    } else {
        TurnPhase::TurnEnd
    });
}

/// 执行玩家的回答。只在 `WaitDecision` 里跑，且只认「被挂着的那位」的回答。
///
/// Godot 的六个方法（`confirm_buy`/`cancel_buy`/`confirm_upgrade`/`cancel_upgrade`/
/// `dismiss_rent_notice`/`dismiss_card`）在这里塌成一条消息 + 一份状态：
/// 语义由 `Pending` 决定，消息只带意图，所以「阶段不对却执行买地」不可能发生。
pub fn decision_system(
    mut books: Books,
    mut pending: ResMut<PendingDecision>,
    mut answers: MessageReader<DecisionChosen>,
    mut bought: MessageWriter<LandBought>,
    mut upgraded: MessageWriter<LandUpgraded>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let Some((asked, about)) = pending.active() else {
        return;
    };
    // 全部读完再决定：`any()`/`find()` 会短路，没读完的回答下一帧又被读出来，
    // 于是同一个报价可能被答两次
    let mut answer = None;
    for a in answers.read() {
        if a.player == asked {
            answer = Some(a.action);
        }
    }
    let Some(action) = answer else {
        return;
    };

    match (about, action) {
        (Pending::Buy(land), Decision::Accept) => {
            // 钱不够时 `buy` 返回 None —— 与 Godot 的 `confirm_buy` 一样：报价已经给过，
            // 付不起就什么也不发生，回合照常结束（是否该亮「买得起」由 UI 判）
            if let Some(price) = books.buy(asked, land) {
                bought.write(LandBought {
                    player: asked,
                    land,
                    price,
                });
            }
        }
        (Pending::Upgrade(land), Decision::Accept) => {
            if let Some(new_level) = books.upgrade(asked, land) {
                upgraded.write(LandUpgraded {
                    player: asked,
                    land,
                    new_level,
                });
            }
        }
        (Pending::Buy(_) | Pending::Upgrade(_), Decision::Decline) => {}
        // 收据与卡面没有「可拒绝」的选项：钱已经扣了、效果已经结算了。
        // 任何回答都算「我看完了」—— 严格拒绝只会把 UI 的一次疏忽变成挂死。
        (Pending::RentNotice(_) | Pending::Card, _) => {}
        // 报价上答「已阅」= 没答，继续等（Godot 的 `_pending_decision != "buy"` 同义）
        (Pending::Buy(_) | Pending::Upgrade(_), Decision::Acknowledge) => return,
    }

    pending.clear();
    next.set(TurnPhase::TurnEnd);
}

/// 把玩家挂进「服刑」状态：逻辑位置停在**接入点**（出狱后从这格继续沿环前进），
/// 死胡同格只作为视觉停车位发给表现层（`player_data.gd:62-68` 的语义）。
///
/// `JailSentence` 走 `Commands`（下一条 system 才生效，回合还早），位置改是即时的：
/// 本帧后面的 system 看到的就该是新格号。
pub fn jail_player(
    commands: &mut Commands,
    positions: &mut Query<&mut TokenPosition, With<PlayerId>>,
    player: Entity,
    junction: TileIndex,
    turns: u32,
) {
    if let Ok(mut pos) = positions.get_mut(player) {
        pos.tile = junction;
    }
    commands
        .entity(player)
        .insert(JailSentence { turns_left: turns });
}

/// 瞬移：只改逻辑位置，位移动画是表现层的事。
pub fn teleport(
    positions: &mut Query<&mut TokenPosition, With<PlayerId>>,
    player: Entity,
    tile: TileIndex,
) {
    if let Ok(mut pos) = positions.get_mut(player) {
        pos.tile = tile;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_offers_and_clears() {
        let mut pending = PendingDecision::default();
        assert_eq!(pending.active(), None);
        let e = Entity::from_raw_u32(7).expect("索引不该撞上保留值");
        pending.offer(e, Pending::Card);
        assert_eq!(pending.active(), Some((e, Pending::Card)));
        pending.clear();
        assert_eq!(pending.active(), None);
    }

    /// 报价与收据的区别写死在 match 里：收据答什么都算过，报价答「已阅」不算回答。
    /// 这里只验证类型层面「一个待决事项带得出玩家与种类」。
    #[test]
    fn pending_carries_what_it_offered() {
        let e = Entity::from_raw_u32(3).expect("索引不该撞上保留值");
        for about in [
            Pending::Buy(LandId(2)),
            Pending::Upgrade(LandId(2)),
            Pending::RentNotice(LandId(2)),
            Pending::Card,
        ] {
            let mut pending = PendingDecision::default();
            pending.offer(e, about);
            assert_eq!(pending.active(), Some((e, about)));
        }
    }
}
