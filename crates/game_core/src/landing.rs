//! 落格分派与决策闭环 —— Godot `turn_manager.gd` 的 `_land_and_end` + 六个决策方法。
//!
//! 拆成六段流水线，每段只做一件事（`Landed` 这一帧内按顺序跑完，跨帧只发生一次状态转移）：
//!
//! ```text
//! classify ─→ property ─┐
//!           ─→ bank      │
//!           ─→ market    ├─→ close（有 pending 就等回答，没有就收摊）
//!           ─→ card      │
//!           ─→ coupon ───┘
//! ```
//!
//! 三个刻意的结构约束：
//! - **「踩到哪格是什么」只解释一次**（[`classify_landing_system`] → [`Landing`]），
//!   **「这格归谁结算」也只映射一次**（[`domain_of`]）。五个域的 system 各读各的，
//!   不会出现五份「这格是不是地产」的理解。
//! - **域的结算 system 不碰 `NextState`**：只有 [`close_landing_system`] 有权决定
//!   下一步去哪，判据是「有没有人还挂着待决事项」。表现层永远推不动回合这条红线
//!   （§4.6）在这里的另一面就是：任何单个结算 system 也推不动回合。
//! - **[`LandingDomain`] 是一张穷尽 `match` 表**（缺陷 #8 的后半条）：新增 `TileKind`
//!   而没回答「它归哪个域」，编译就停在 [`domain_of`]，不会像 Godot 的字符串 `match`
//!   那样静默落到 `default`。表里被点名成 [`LandingDomain::Unimplemented`] 的那些格
//!   踩上去会 `warn!` 一句 —— 「什么都不发生」从此是可被告知的，而不是靠玩家猜。

use bevy::prelude::*;

use crate::board::{TileIndex, TileKind};
use crate::card::CardKind;
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
    /// 该格的结算域，由 [`domain_of`] 从 `kind` 映射而来。各域 system 认这个而不是 `kind`：
    /// 「谁该管」只有一个落点，加格类型时才可能被迫回答它。
    pub domain: LandingDomain,
    /// 该格绑定的土地（只有 `TileKind::Property` 有，`BoardDef::validate` 保证）。
    pub land: Option<LandId>,
}

/// 落格归哪个结算域。与流水线里那条 chain 一一对应，`match` 的穷尽性由编译器守住。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LandingDomain {
    /// 无主报价 / 收租 / 升级报价（`property::settle_property_landing`）
    Property,
    /// 全员结息 + 开柜台（`bank::settle_bank_landing`）
    Bank,
    /// 证券格报到（`market::settle_market_landing`）
    Market,
    /// 抽一张卡并当场结算（`deck::settle_card_landing`）。带的就是该抽的那副牌 ——
    /// 于是「机会格抽机会牌」这条配对也只写一次，不在牌堆域里再 `match` 一遍格类型。
    Card(CardKind),
    /// 点券箱一次性拾取（`coupon::settle_coupon_landing`）
    Coupon,
    /// 按设计就是什么都不做：路面只是路面，起点刻意不发薪水（PLAN §一）。
    Nothing,
    /// 玩法已在 PLAN 登记、但结算还没到位的格。踩上去 `warn!`，别当成「没有这个格」。
    /// 每一项的归属阶段见 `.agents/PLAN.md` §三 的缺口对照表。
    Unimplemented(TileKind),
}

/// 「这格归谁结算」的唯一一张表。**这里不许出现 `_`**：
/// 新增 `TileKind` 时会在这里编译失败，逼出一次「它到底该谁管」的判断。
pub const fn domain_of(kind: TileKind) -> LandingDomain {
    use LandingDomain as D;
    match kind {
        TileKind::Property => D::Property,
        TileKind::Bank => D::Bank,
        TileKind::Stock => D::Market,
        TileKind::Chance => D::Card(CardKind::Chance),
        TileKind::Fate => D::Card(CardKind::Fate),
        TileKind::Coupon => D::Coupon,
        // 起点「什么都不发」是拍板过的规则（4 代没有过起点领薪水），不是没做完
        TileKind::Start | TileKind::Road => D::Nothing,
        // ↓ 待办清单。监狱/医院要等 P4 的 `Detention`、神明等 P6、乐透等 P8、
        //   中立与交通根本没有对应玩法（PLAN §三）。踩上去不再静默。
        TileKind::Jail
        | TileKind::Hospital
        | TileKind::Lottery
        | TileKind::GodDeity
        | TileKind::Neutral
        | TileKind::Transport => D::Unimplemented(kind),
    }
}

/// 「这一帧踩到了什么」的唯一一份事实。由流水线第一段写一次，后面各段只读。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Landing(pub Option<Landed>);

/// 流水线第一段：把当前玩家的逻辑位置翻译成 `(格类型, 结算域, 土地)`。
pub fn classify_landing_system(
    order: Res<TurnOrder>,
    positions: Query<&TokenPosition, With<PlayerId>>,
    active: Res<ActiveBoard>,
    mut landing: ResMut<Landing>,
) {
    landing.0 = None;
    let Some(player) = order.current() else {
        return;
    };
    let Ok(pos) = positions.get(player) else {
        return;
    };
    let Some(tile) = active.def.tile(pos.tile.0 as i64) else {
        return;
    };
    let domain = domain_of(tile.kind);
    if let LandingDomain::Unimplemented(kind) = domain {
        warn!(
            "格 {}({}) 是 {kind:?} 类型，但这一格还没有结算实现（待办见 .agents/PLAN.md §三）",
            tile.id, tile.name
        );
    }
    landing.0 = Some(Landed {
        player,
        tile: pos.tile,
        kind: tile.kind,
        domain,
        land: tile.land_id.map(LandId),
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
    /// 踩到银行格，柜台开着：存/取/借/还随便办，直到玩家按「办完」。
    ///
    /// 与另外四种不同，它**不是一个报价**，而是一个窗口 —— 钱的操作走
    /// [`crate::msg::BankOpRequested`] 逐笔进来，本条待决事项只负责「回合先别结束」。
    BankTeller,
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
        // 柜台：只有「办完」算回答。四种钱的操作走 `BankOpRequested` 逐笔进来办，
        // 这里若也认 `Accept`，就成了一条消息既办业务又关窗口 —— 两个语义挤一个动作，
        // 而面板上根本没有「接受」这个按钮可对上。
        (Pending::BankTeller, Decision::Acknowledge) => {}
        (Pending::BankTeller, Decision::Accept | Decision::Decline) => return,
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

    /// 分派表的全部条目。这张清单与 `domain_of` 一一对着写：新增 `TileKind` 时
    /// `domain_of` 会先编译失败，所以这里唯一可能过期的是「加了格类型没加进清单」——
    /// 下面那条 `每格都有归宿` 的用例把两种过期分开钉住了。
    const ALL_KINDS: [TileKind; 14] = [
        TileKind::Start,
        TileKind::Property,
        TileKind::Bank,
        TileKind::Stock,
        TileKind::Chance,
        TileKind::Fate,
        TileKind::Jail,
        TileKind::Hospital,
        TileKind::Lottery,
        TileKind::GodDeity,
        TileKind::Neutral,
        TileKind::Road,
        TileKind::Transport,
        TileKind::Coupon,
    ];

    #[test]
    fn every_kind_is_routed_to_exactly_one_domain() {
        use LandingDomain as D;
        let routes = [
            (TileKind::Property, D::Property),
            (TileKind::Bank, D::Bank),
            (TileKind::Stock, D::Market),
            (TileKind::Chance, D::Card(CardKind::Chance)),
            (TileKind::Fate, D::Card(CardKind::Fate)),
            (TileKind::Coupon, D::Coupon),
            // 拍板过「什么都不发」的两格：不是待办
            (TileKind::Start, D::Nothing),
            (TileKind::Road, D::Nothing),
            // 待办六格各自点名；将来某格实现了就从这一组挪走，
            // 于是「悄悄少了一个待办」也会在这里显形
            (TileKind::Jail, D::Unimplemented(TileKind::Jail)),
            (TileKind::Hospital, D::Unimplemented(TileKind::Hospital)),
            (TileKind::Lottery, D::Unimplemented(TileKind::Lottery)),
            (TileKind::GodDeity, D::Unimplemented(TileKind::GodDeity)),
            (TileKind::Neutral, D::Unimplemented(TileKind::Neutral)),
            (TileKind::Transport, D::Unimplemented(TileKind::Transport)),
        ];
        assert_eq!(
            routes.len(),
            ALL_KINDS.len(),
            "分派表与 `TileKind` 清单不同宽：加格类型时两边都要动"
        );
        for &(kind, want) in &routes {
            assert_eq!(domain_of(kind), want, "{kind:?} 的分派变了");
        }
        for kind in ALL_KINDS {
            assert!(
                routes.iter().any(|(k, _)| *k == kind),
                "{kind:?} 在分派表里没有归宿"
            );
        }
    }

    /// 结算域里不许出现「谁都不管」的洞：每个 `Unimplemented` 都得是刻意点名的。
    #[test]
    fn the_todo_list_is_explicit() {
        let todos: Vec<_> = ALL_KINDS
            .iter()
            .filter(|k| matches!(domain_of(**k), LandingDomain::Unimplemented(_)))
            .collect();
        assert_eq!(
            todos,
            vec![
                &TileKind::Jail,
                &TileKind::Hospital,
                &TileKind::Lottery,
                &TileKind::GodDeity,
                &TileKind::Neutral,
                &TileKind::Transport,
            ],
            "待办清单变了：那是 PLAN §三 的缺口对照表也跟着变的意思"
        );
    }

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
            Pending::BankTeller,
        ] {
            let mut pending = PendingDecision::default();
            pending.offer(e, about);
            assert_eq!(pending.active(), Some((e, about)));
        }
    }
}
