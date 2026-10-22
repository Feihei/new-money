//! 落格分派与决策闭环 —— Godot `turn_manager.gd` 的 `_land_and_end` + 六个决策方法。
//!
//! 拆成六段流水线，每段只做一件事（`Landed` 这一帧内按顺序跑完，跨帧只发生一次状态转移）：
//!
//! ```text
//! classify ─→ property ─┐
//!           ─→ bank      │
//!           ─→ market    ├─→ close（有 pending 就等回答，没有就收摊）
//!           ─→ card      │
//!           ─→ coupon ───┤
//!           ─→ shop ─────┘
//! ```
//!
//! 三个刻意的结构约束：
//! - **「踩到哪格是什么」只解释一次**（[`classify_landing_system`] → [`Landing`]），
//!   **「这格归谁结算」也只映射一次**（[`domain_of`]）。六个域的 system 各读各的，
//!   不会出现五份「这格是不是地产」的理解。
//! - **域的结算 system 不碰 `NextState`**：只有 [`close_landing_system`] 有权决定
//!   下一步去哪，判据是「有没有人还挂着待决事项」。表现层永远推不动回合这条红线
//!   （§4.6）在这里的另一面就是：任何单个结算 system 也推不动回合。
//! - **[`LandingDomain`] 是一张穷尽 `match` 表**（缺陷 #8 的后半条）：新增 `TileKind`
//!   而没回答「它归哪个域」，编译就停在 [`domain_of`]，不会像 Godot 的字符串 `match`
//!   那样静默落到 `default`。表里被点名成 [`LandingDomain::Unimplemented`] 的那些格
//!   踩上去会 `warn!` 一句 —— 「什么都不发生」从此是可被告知的，而不是靠玩家猜。

use bevy::prelude::*;

use crate::board::{BoardDef, GridPos, TileIndex, TileKind};
use crate::card::CardKind;
use crate::land::LandId;
use crate::msg::{Decision, DecisionChosen, LandBought, LandUpgraded};
use crate::player::{Detention, DetentionPlace, PlayerId, TokenPosition};
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
    /// 百货公司柜台：点券买卡 / 卖回（`shop::settle_shop_landing`）。
    Shop,
    /// 游乐场：踩上去开一局小游戏，成绩换点券（P7，`minigame::settle_arcade_landing`）。
    ///
    /// 「玩哪一款」不在这一格里 —— 那是格子数据（`TileData.game`，缺省＝随机一款）与
    /// [`Pending::Minigame`] 的事，而 `domain_of` 只是「格类型 → 谁结算」的一张表，
    /// 它读不到格子数据（`const fn`，且刻意不碰世界）。
    Arcade,
    /// 拘留场所的接入点：踩上去可以花点券保释关在里面的人（P4③，`bail::settle_bail_landing`）。
    /// 带的是**这个入口管哪一处** —— 监狱门口放不出医院里的人。
    Bail(DetentionPlace),
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
        TileKind::Shop => D::Shop,
        TileKind::Arcade => D::Arcade,
        // 拘留场所的**接入点**（P4③）：踩上去是保释入口，不是被抓 ——
        // 「没有任何踩到某格就被关的规则」是 10-20 拍板的另一半
        TileKind::Jail => D::Bail(DetentionPlace::Jail),
        TileKind::Hospital => D::Bail(DetentionPlace::Hospital),
        // 起点「什么都不发」是拍板过的规则（4 代没有过起点领薪水），不是没做完
        TileKind::Start | TileKind::Road => D::Nothing,
        // ↓ 待办清单。神明等 P6、乐透等 P8、中立与交通根本没有对应玩法（PLAN §三）。
        //   踩上去不再静默。
        TileKind::Lottery | TileKind::GodDeity | TileKind::Neutral | TileKind::Transport => {
            D::Unimplemented(kind)
        }
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
    /// 踩到百货格，柜台开着：买一张、卖回一张（9 折）随便办，直到玩家按「离开」。
    ///
    /// 与 `BankTeller` 是**同一类**（一个窗口而不是一个报价，交易走
    /// [`crate::msg::ShopOpRequested`] 逐笔进来），但不是同一条资格：银行路过就发 ATM 卡，
    /// 百货没有「路过」这一档 —— 逛店必须停下来。所以两者各占一个变体而不是
    /// `Pending::Counter { which }`：分开写，将来「柜台能办什么」那张表才不会被并错。
    Shop,
    /// 踩到拘留场所的接入点：可以花点券保释关在里面的人（P4③）。
    ///
    /// 与 `BankTeller`/`Shop` 一样是**窗口**（保释请求走 [`crate::msg::BailRequested`] 逐笔
    /// 进来，一次可以放不止一个人），所以对 `Accept`/`Decline` 不认，只认「离开」。
    /// 与 P3a「出牌不挂 Pending」相反：这里规则层确实要停下来问「要不要保」。
    Bail(DetentionPlace),
    /// 踩到游乐场：这一趟玩哪一款小游戏（P7）。
    ///
    /// 它也是**窗口**，但关掉它的那把钥匙与另外三个不同：柜台按「离开」就算办完，
    /// 而这一格的正经出口是 [`crate::msg::MinigameResult`]（玩完把成绩报上来，
    /// 由 `minigame::minigame_score_system` 换点券、清窗口、交还回合）。
    /// `Acknowledge` 在这里的意思是**空手离开**（不玩、或玩了不报成绩）—— 它必须存在，
    /// 否则一个不想玩的人会把回合机永久停在 `WaitDecision`，而长跑的自动玩家也过不去。
    ///
    /// 带的那一款是**这一趟定下来的**：格子数据写了就是它，没写则由 `Rng` 的小游戏子流抽
    /// （见 `minigame::settle_arcade_landing`）。挂在待决事项上而不是让 UI 自己去查格子，
    /// 是因为「随机抽到了哪一款」是这一局的事实，得跟着回合机走、也得进存档（P15）。
    Minigame(crate::minigame::GameKind),
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

/// `decision_system` 除了账本之外要读的那一份「这一帧落在哪」。
/// 打成参数包是因为 clippy 的参数上限（§六 坑 8 的同一处理）：这不是「需要知道很多事」，
/// 是「同一个问题的两半」—— 被回答的报价与它落的那一格。
#[derive(bevy::ecs::system::SystemParam)]
pub struct DecisionView<'w> {
    landing: Res<'w, Landing>,
}

/// 执行玩家的回答。只在 `WaitDecision` 里跑，且只认「被挂着的那位」的回答。
///
/// Godot 的六个方法（`confirm_buy`/`cancel_buy`/`confirm_upgrade`/`cancel_upgrade`/
/// `dismiss_rent_notice`/`dismiss_card`）在这里塌成一条消息 + 一份状态：
/// 语义由 `Pending` 决定，消息只带意图，所以「阶段不对却执行买地」不可能发生。
pub fn decision_system(
    mut books: Books,
    mut pending: ResMut<PendingDecision>,
    view: DecisionView<'_>,
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
        // 百货柜台与保释窗口同一条理由（`ShopOpRequested`/`BailRequested` 逐笔办），
        // 游乐场也在这组里（成绩走 `MinigameResult` 报上来）：四个窗口对 `Accept`/`Decline`
        // 一律不认，谁新增第五种窗口都要来这一格表态。
        // ⚠️ 游乐场那一格的 `Acknowledge` 语义与另外三个不同：柜台是「办完了」，
        //    它是「不玩了／空手离开」（成绩另有一条路）。见 `Pending::Minigame` 的注。
        (
            Pending::BankTeller | Pending::Shop | Pending::Bail(_) | Pending::Minigame(_),
            Decision::Acknowledge,
        ) => {}
        (
            Pending::BankTeller | Pending::Shop | Pending::Bail(_) | Pending::Minigame(_),
            Decision::Accept | Decision::Decline,
        ) => return,
        // 收据与卡面没有「可拒绝」的选项：钱已经扣了、效果已经结算了。
        // 任何回答都算「我看完了」—— 严格拒绝只会把 UI 的一次疏忽变成挂死。
        (Pending::RentNotice(_) | Pending::Card, _) => {}
        // 报价上答「已阅」= 没答，继续等（Godot 的 `_pending_decision != "buy"` 同义）
        (Pending::Buy(_) | Pending::Upgrade(_), Decision::Acknowledge) => return,
    }

    // 刚答完一个报价/窗口：如果这一格正是监狱或医院那条分支的**接入点**，且那里真关着人，
    // 就把保释窗口摆出来（P4③）。演示盘的接入点是地产格（汉口路／徐家汇），所以「走到
    // 监狱门口」表现为先买地/付租，再决定要不要放人 —— 与 4 代在图上走到那格是同一件事。
    // 保释窗口自己关掉时（`about` 已是 `Bail`）不再回头再问一次，否则会绕成死循环。
    if !matches!(about, Pending::Bail(_))
        && let Some(place) = view
            .landing
            .0
            .filter(|l| l.player == asked)
            .and_then(|l| crate::bail::branch_place(&books.active.def, l.tile))
        && crate::bail::someone_held(&books, place)
    {
        pending.offer(asked, Pending::Bail(place));
        return;
    }

    pending.clear();
    next.set(TurnPhase::TurnEnd);
}

/// 把人送进某一处「不在牌桌上」的状态（P4）。返回的是**他此刻真正被扣住的那一条**
/// 拘留（可能是原来那张更久的）+ 接入点 + 死胡同格，调用方拿这三样去发凭据。
///
/// 三个去处共用这一句，差别全在 [`DetentionPlace`] 那两条映射上：
/// - `Jail` / `Hospital` → 该条分支的接入点格 + 那条死胡同格的网格坐标；
/// - `Away`（出国、被绑架、住旅馆）→ **棋盘上没有那个地方**，所以他留在自己此刻所在
///   的那格，那格就是回来时继续走的地方（与「这张棋盘没有分支格」共用同一条退路）。
///
/// 查不到接入点（这张棋盘没有那条分支）时返回 `None` 且**什么都不改** —— 不猜一个格子、
/// 也不回退到起点（`card_effects.gd:76-81` 修过的坑）。
///
/// **更久的那一张赢**：已经要被关 3 天的人再吃一张 2 天，他该待原来那个地方满 3 天。
/// 写成覆盖就等于替人减刑 —— 与 P3d④c 停留卡同一条理由，所以那条规则在这里也成立，
/// 而「等长时后来者赢」是这条规则剩下唯一的任意格，由一条用例钉住。
///
/// 两条写都尽量**就地**（`Query<&mut>`），`Commands` 只用于「他身上本来没有这个组件」那一格：
/// 同一帧两张拘留卡若都走延迟写，第二条读不到第一条，就会各搬一次棋子、各发一份凭据。
pub fn detain(
    commands: &mut Commands,
    positions: &mut Query<&mut TokenPosition, With<PlayerId>>,
    held: &mut Query<&mut Detention, With<PlayerId>>,
    def: &BoardDef,
    player: Entity,
    place: DetentionPlace,
    turns: u32,
) -> Option<(Detention, TileIndex, Option<GridPos>)> {
    if let Ok(existing) = held.get_mut(player)
        && existing.turns_left > turns
    {
        let kept = *existing;
        // 他没换地方，所以接入点还是他现在站着的那格
        let junction = positions.get(player).ok()?.tile;
        let end = kept
            .place
            .branch()
            .and_then(|kind| def.branch_end(kind))
            .map(GridPos::of);
        return Some((kept, junction, end));
    }
    let mut pos = positions.get_mut(player).ok()?;
    let junction = match place.branch() {
        Some(kind) => TileIndex(def.branch_junction(kind)?.id),
        None => pos.tile,
    };
    let end = place
        .branch()
        .and_then(|kind| def.branch_end(kind))
        .map(GridPos::of);
    pos.tile = junction;
    let sentence = Detention {
        place,
        turns_left: turns,
    };
    match held.get_mut(player) {
        Ok(mut existing) => *existing = sentence,
        Err(_) => {
            commands.entity(player).insert(sentence);
        }
    }
    Some((sentence, junction, end))
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
    const ALL_KINDS: [TileKind; 16] = [
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
        TileKind::Shop,
        TileKind::Arcade,
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
            (TileKind::Shop, D::Shop),
            (TileKind::Arcade, D::Arcade),
            // P4③：拘留场所的接入点 = 保释入口（没人可保时安静收摊）
            (TileKind::Jail, D::Bail(DetentionPlace::Jail)),
            (TileKind::Hospital, D::Bail(DetentionPlace::Hospital)),
            // 拍板过「什么都不发」的两格：不是待办
            (TileKind::Start, D::Nothing),
            (TileKind::Road, D::Nothing),
            // 待办四格各自点名；将来某格实现了就从这一组挪走，
            // 于是「悄悄少了一个待办」也会在这里显形
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
            Pending::Shop,
            Pending::Bail(DetentionPlace::Jail),
            Pending::Minigame(crate::minigame::GameKind::PenguinDig),
        ] {
            let mut pending = PendingDecision::default();
            pending.offer(e, about);
            assert_eq!(pending.active(), Some((e, about)));
        }
    }
}
