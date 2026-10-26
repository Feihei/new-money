//! 道路放置物（P5）：ADR-002 的落地。
//!
//! **一句话**：occupancy 是挂在主环格坐标上的运行时实体；拓扑真相仍然只在
//! `BoardDef`；道路格本身依旧不建实体。所以这个模块里没有第二种 `TileKind`，
//! 也没有 `HashMap<TileIndex, …>` —— 一格有没有东西，问组件。
//!
//! 四条在实现上最容易被写歪的口径：
//! - **一格一件**：放置、宝箱刷出、P6 神明换格、P12 小偷搬动都走 [`place`] 这一道闸门。
//!   同帧两条请求不能都读到「空」——`Commands::spawn` 是延迟写，所以系统里额外维护一个
//!   **本帧已占格**集合（[`place_item_system`] 的 `Local`）；它不是第二份索引，
//!   每次调用开头就清空，只覆盖「这一帧还没落地的 spawn」。
//! - **经过 vs 落脚**：[`trigger_of`] 是穷尽表。路障/地雷/炸弹/恶犬在**走步循环里**
//!   触发；宝箱在 `PlayerArrived` 之后触发（落脚才算）。
//! - **走步循环只做与移动有关的事**：路障截断发生在掷骰入队时；地雷/炸弹的中断发生在
//!   循环内部；住院、拆房、毁车这些后果由 [`resolve_halt_system`] 在停下后应用。
//! - **中断不是到站**：发 [`WalkHalted`]，不发 `PlayerArrived`，也不结算脚下那一格。

use std::collections::HashSet;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::{BoardDef, TileIndex};
use crate::damage::LandDamage;
use crate::items::ItemKind;
use crate::landing::{PendingDecision, detain};
use crate::msg::{
    ChestOpened, ItemPlaced, PlaceItemRequested, PlayerArrived, PlayerDetained, WalkHalted,
};
use crate::player::{DetentionPlace, PlayerId, TokenPosition, Walker};
use crate::plot;
use crate::rng::Rng;
use crate::settle::Books;
use crate::turn::{TurnOrder, TurnPhase};
use crate::world::{ActiveBoard, Gameplay};

/// 定时炸弹走满多少步爆。ADR-002 §8 把「拾取那一步算不算第 1 步」留给 P5 的测试钉死：
/// 这里选「拾取即第 1 步」，所以拾取后显示 37、再走 37 步爆炸（合计 38 步）。
pub const BOMB_STEPS: u32 = 38;
/// 炸弹爆炸后的住院天数（ADR-002 §5）。
pub const BOMB_HOSPITAL_DAYS: u32 = 5;
/// 地雷与恶犬的住院天数（PLAN §三「医院」行）。
pub const MINE_HOSPITAL_DAYS: u32 = 3;
/// 开局在路上撒几个宝箱。4 代是随机刷；这里固定两个，够玩家在一局里摸到道具，
/// 又不至于把整条路变成宝箱传送带。
pub const STARTING_CHESTS: usize = 2;

/// 一种道路放置物。**新增一种必须同时回答三件事**（ADR-002 维护提醒）：
/// 占不占格、经过还是落脚触发、会不会被 P12 小偷搬走。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum OccupancyKind {
    /// 路障：经过即停，不消耗
    Roadblock,
    /// 地雷：经过即炸，消耗
    Mine,
    /// 定时炸弹：经过拾取，之后每走一步计数
    TimeBomb,
    /// 恶犬：经过被咬（住院 3 天）；载具撞飞它（P6 负责刷出）
    Dog,
    /// 宝箱：落脚拾取，给一个随机道具
    Chest,
    /// 神：落脚即附身（P6）。带变体而不是每种神一个 `OccupancyKind` ——
    /// 「占不占格」「经过还是落脚」这十三位的答案是同一个，逐位再写一遍就是十三行废话。
    /// 它们**各不相同**的那三样（管几天、对位是谁、上不上街）在
    /// [`crate::gods::God`] 的三张穷尽表里。
    God(crate::gods::God),
}

impl OccupancyKind {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Roadblock => "路障",
            Self::Mine => "地雷",
            Self::TimeBomb => "定时炸弹",
            Self::Dog => "恶犬",
            Self::Chest => "宝箱",
            Self::God(god) => god.label(),
        }
    }
}

/// 挂在实体上的位置。**一个状态字段而不是两个可并存的组件**：位置是排他的，
/// 让它只能表示一个值，就写不出「既在地上又在人身上」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccupancyAt {
    OnTile(TileIndex),
    Carried { by: Entity, steps_left: u32 },
}

/// 一个道路放置物。携带 [`Gameplay`] 标记，回主菜单时随其它核心实体一起清。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Occupancy {
    pub kind: OccupancyKind,
    pub at: OccupancyAt,
}

impl Occupancy {
    pub const fn on_tile(kind: OccupancyKind, tile: TileIndex) -> Self {
        Self {
            kind,
            at: OccupancyAt::OnTile(tile),
        }
    }

    pub fn tile(&self) -> Option<TileIndex> {
        match self.at {
            OccupancyAt::OnTile(tile) => Some(tile),
            OccupancyAt::Carried { .. } => None,
        }
    }
}

/// 什么时候触发。**穷尽表，不许出现 `_`** —— 新增种类必须当场表态。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Trigger {
    /// 走进这一格就触发（路障、地雷、炸弹拾取、恶犬）
    Pass,
    /// 走完骰子停在这一格才触发（宝箱、P6 的神明）
    Land,
}

pub const fn trigger_of(kind: OccupancyKind) -> Trigger {
    match kind {
        OccupancyKind::Roadblock
        | OccupancyKind::Mine
        | OccupancyKind::TimeBomb
        | OccupancyKind::Dog => Trigger::Pass,
        // 神与宝箱同一条：走完骰子停在那一格才算。路过一位财神不给祝福，
        // 这条口径与 ADR-002 §5 那张表一致，而 P6 的漫游「走到人那格不算附身」也靠它
        OccupancyKind::Chest | OccupancyKind::God(_) => Trigger::Land,
    }
}

// ===== 放置 =====

/// 一次放置办不成的原因。UI 的灰按钮与规则层的拒绝共用这一张表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaceDeny {
    /// 不是当前回合玩家、或不在掷骰前
    NotNow,
    /// 身上没有这个道具
    NotHeld,
    /// 遥控骰子不是放置物（它改的是这一回合走几步）
    NotPlaceable,
    /// 格号越界（主环只有 `0..loop_len`）
    NoSuchTile,
    /// 这一格已经有东西了（ADR-002 的一格一件）
    Occupied,
    /// 这一格上有玩家棋子（玩家主动放置的合法性；读档与 P6 换格不走这一条）
    SomebodyThere,
}

impl PlaceDeny {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotNow => "掷骰前才能放",
            Self::NotHeld => "身上没有这个道具",
            Self::NotPlaceable => "这个道具不能放在路上",
            Self::NoSuchTile => "这一格不在主环上",
            Self::Occupied => "这一格已经有东西了",
            Self::SomebodyThere => "这一格上有人",
        }
    }
}

/// 纯判据：目标格能不能放。窗口与主语由 system 先判，这里只回答「这一笔」。
/// UI 与规则层都调它，所以灰按钮上的理由与真正被拒的原因不可能分叉。
pub fn place_denied(
    item: ItemKind,
    held: u32,
    tile_exists: bool,
    occupied: bool,
    somebody_there: bool,
) -> Result<(), PlaceDeny> {
    if item.occupancy().is_none() {
        return Err(PlaceDeny::NotPlaceable);
    }
    if held == 0 {
        return Err(PlaceDeny::NotHeld);
    }
    if !tile_exists {
        return Err(PlaceDeny::NoSuchTile);
    }
    if occupied {
        return Err(PlaceDeny::Occupied);
    }
    if somebody_there {
        return Err(PlaceDeny::SomebodyThere);
    }
    Ok(())
}

/// 往一格上放东西的**唯一闸门**。返回 `None` = 这一格已被占（含本帧刚 spawn、
/// 还没被查询看见的那一件）。
///
/// P6 的神明刷出、P12 小偷搬动将来也走这里 —— 「一格一件」只在这一处判。
pub fn place(
    commands: &mut Commands,
    occupancies: impl Iterator<Item = (Entity, OccupancyAt)>,
    claimed: &mut HashSet<TileIndex>,
    kind: OccupancyKind,
    tile: TileIndex,
) -> Option<Entity> {
    let taken = claimed.contains(&tile)
        || occupancies
            .into_iter()
            .any(|(_, at)| at == OccupancyAt::OnTile(tile));
    if taken {
        return None;
    }
    claimed.insert(tile);
    Some(
        commands
            .spawn((Occupancy::on_tile(kind, tile), Gameplay))
            .id(),
    )
}

/// 放道具要读的世界。打包成参数包是仓库房规（§六 坑 8）：这几样一起回答
/// 「轮到谁、身上有没有、这一格空不空、有没有人」。
#[derive(SystemParam)]
pub struct PlaceDesk<'w, 's> {
    commands: Commands<'w, 's>,
    order: Res<'w, TurnOrder>,
    phase: Res<'w, State<TurnPhase>>,
    pending: Res<'w, PendingDecision>,
    active: Res<'w, ActiveBoard>,
    inventories: Query<'w, 's, &'static mut crate::items::Inventory, With<PlayerId>>,
    positions: Query<'w, 's, &'static TokenPosition, With<PlayerId>>,
    occupancies: Query<'w, 's, (Entity, &'static Occupancy)>,
    requests: MessageReader<'w, 's, PlaceItemRequested>,
    placed: MessageWriter<'w, ItemPlaced>,
}

/// 把一件放置类道具放到指定主环格。只在 `WaitRoll` 且没有待决事项时受理，
/// 主语只认当前回合玩家（与出牌同一条窗口）。
pub fn place_item_system(mut desk: PlaceDesk<'_, '_>, mut claimed: Local<HashSet<TileIndex>>) {
    if *desk.phase.get() != TurnPhase::WaitRoll || desk.pending.active().is_some() {
        // 消息仍要读完：留着不读会在下一帧被再读一次（窗口恰好打开那一帧就多放一件）
        for _ in desk.requests.read() {}
        return;
    }
    let Some(current) = desk.order.current() else {
        for _ in desk.requests.read() {}
        return;
    };
    // Local 的生命周期跨帧，所以每次调用开头清空 —— 它只表示「这一帧已经 spawn 的格」，
    // 不是一份长期索引（ADR-002 §3 禁止的是后者）
    claimed.clear();

    for request in desk.requests.read() {
        if request.player != current {
            debug!("放置被拒：{:?} 不是当前回合玩家", request.player);
            continue;
        }
        let held = desk
            .inventories
            .get(current)
            .map(|inv| inv.count(request.item))
            .unwrap_or(0);
        let exists = (request.tile.0 as usize) < desk.active.def.loop_len() as usize;
        let occupied = claimed.contains(&request.tile)
            || desk
                .occupancies
                .iter()
                .any(|(_, occ)| occ.at == OccupancyAt::OnTile(request.tile));
        let somebody_there = desk.positions.iter().any(|p| p.tile == request.tile);
        if let Err(why) = place_denied(request.item, held, exists, occupied, somebody_there) {
            debug!(
                "放置被拒：{:?} 想把{}放到格{} —— {}",
                request.player,
                request.item.label(),
                request.tile.0,
                why.label()
            );
            continue;
        }
        let Some(kind) = request.item.occupancy() else {
            continue;
        };
        let Ok(mut inventory) = desk.inventories.get_mut(current) else {
            continue;
        };
        if !inventory.spend(request.item, 1) {
            continue;
        }
        let at = desk.occupancies.iter().map(|(e, occ)| (e, occ.at));
        let Some(_entity) = place(&mut desk.commands, at, &mut claimed, kind, request.tile) else {
            // 上面刚查过；走到这里只可能是同一帧另一条请求抢先占了它。
            // 道具要还回去 —— 「扣了道具没放成」是查不动的那类投诉。
            if let Ok(mut inventory) = desk.inventories.get_mut(current) {
                inventory.gain(request.item, 1);
            }
            continue;
        };
        desk.placed.write(ItemPlaced {
            player: current,
            item: request.item,
            tile: request.tile,
        });
    }
}

// ===== 开局宝箱 =====

/// 开局在环上撒 [`STARTING_CHESTS`] 个宝箱（不含起点）。位置走**专用随机子流**，
/// 所以「这一局宝箱在哪儿」不会挪动骰子、牌堆与行情。
pub fn spawn_starting_chests(
    commands: &mut Commands,
    def: &BoardDef,
    rng: &mut Rng,
) -> Vec<TileIndex> {
    let n = def.loop_len() as usize;
    if n <= 1 {
        return Vec::new();
    }
    let want = STARTING_CHESTS.min(n - 1);
    let mut picked: Vec<usize> = Vec::new();
    while picked.len() < want {
        // 候选是 1..n（排除 0 号起点），用部分 Fisher-Yates 不放回
        let span = (n - 1) as u32;
        let at = 1 + rng.pick_spawn(0..=span - 1) as usize;
        if !picked.contains(&at) {
            picked.push(at);
        }
    }
    for &at in &picked {
        commands.spawn((
            Occupancy::on_tile(OccupancyKind::Chest, TileIndex(at as u32)),
            Gameplay,
        ));
    }
    picked.into_iter().map(|at| TileIndex(at as u32)).collect()
}

// ===== 落脚触发 =====

/// 落脚触发的 core 侧 reader（ADR-002 §5/§7）。
///
/// **读的是 `PlayerArrived.to`，不是最后一个 `TileStepped`**：中断时两者不是同一格，
/// 而中断的那一格本来就不该触发落脚效果。宝箱是这条 reader 的第一个用户；
/// P6 的神明落地也接这里。
///
/// 小偷（P12c）落脚在宝箱上走的也是这一条 —— 但他不是「开箱」，是「连箱搬走」：
/// despawn 之后掉落照抽（与玩家同一条掉落子流，箱子不该认人），东西发
/// `ItemThieved` 给 [`crate::villain::thief_deliver_system`] 送进雇主道具栏。
/// 其余三种恶人**不开箱也不动箱** —— 偷窃是小偷的职业行为，宝箱留在街上等它的主人。
// 参数都是同一条 reader 的输入输出（拆一半就得借两次 `Rng`），与 `settle_service`
// 同一条处理：`allow` 而不是造一个只用一次的参数包
#[allow(clippy::too_many_arguments)]
pub fn settle_landing_occupancy_system(
    mut commands: Commands,
    mut arrivals: MessageReader<PlayerArrived>,
    occupancies: Query<(Entity, &Occupancy)>,
    villains: Query<&crate::villain::Villain>,
    mut inventories: Query<&mut crate::items::Inventory, With<PlayerId>>,
    mut rng: ResMut<Rng>,
    mut opened: MessageWriter<ChestOpened>,
    mut thieved: MessageWriter<crate::msg::ItemThieved>,
) {
    for arrival in arrivals.read() {
        let Some((entity, _)) = occupancies.iter().find(|(_, occ)| {
            occ.kind == OccupancyKind::Chest
                && trigger_of(occ.kind) == Trigger::Land
                && occ.at == OccupancyAt::OnTile(arrival.to)
        }) else {
            continue;
        };
        let item = ItemKind::ALL[rng.pick_loot(0..=ItemKind::ALL.len() as u32 - 1) as usize];
        // 踩到的是小偷 → 连箱搬走（P12c）：拆箱 + 赃物走雇主那一头。
        // 拆箱必须在这里 —— 小偷没有 `PlayerId`，下面的道具栏查询查不到他；
        // 把 despawn 放到那一步之后，小偷就变成了「踩到宝箱连箱子都搬不动的人」
        if villains
            .get(arrival.player)
            .is_ok_and(|v| v.kind == crate::villain::VillainKind::Thief)
        {
            commands.entity(entity).despawn();
            thieved.write(crate::msg::ItemThieved {
                thief: arrival.player,
                item,
                tile: arrival.to,
            });
            continue;
        }
        // 其余三种恶人在这里自然落空 —— 箱子留着等它的主人，「偷窃」只有小偷干得出来
        let Ok(mut inventory) = inventories.get_mut(arrival.player) else {
            continue;
        };
        commands.entity(entity).despawn();
        inventory.gain(item, 1);
        opened.write(ChestOpened {
            player: arrival.player,
            tile: arrival.to,
            item,
        });
    }
}

// ===== 中断 =====

/// 这一趟行走为什么停下。ADR-002 §5 的经过触发表。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HaltCause {
    Mine,
    TimeBomb,
    Dog,
}

impl HaltCause {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mine => "地雷",
            Self::TimeBomb => "定时炸弹",
            Self::Dog => "恶犬",
        }
    }
}

/// 最后一次中断。走步循环写、`resolve_halt_system` 取走 —— 与消息分开是因为
/// 后果结算要读 `Books`/`Commands`，而走步循环不该背上那一整包。
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct WalkHalt {
    pub last: Option<WalkHalted>,
}

/// 停下之后应用后果：住院、炸弹的 3×3 降层与全圈住院。
///
/// **不结算脚下那一格**（不买地、不收租、不抽卡）：中断不是到站，那条分派根本不该跑。
/// 这里也不碰 `TokenPosition` 之外的位置状态 —— 走步循环已经把逻辑位置改回停下的那一格。
// 参数都是「把一件停摆结算完」的那一套工具，与 `settle_service` 同一条处理
#[allow(clippy::too_many_arguments)]
pub fn resolve_halt_system(
    mut commands: Commands,
    mut halt: ResMut<WalkHalt>,
    mut books: Books<'_, '_>,
    mut positions: Query<&mut TokenPosition, With<Walker>>,
    mut mounts: Query<&mut crate::player::Mount, With<Walker>>,
    villains: Query<&crate::villain::Villain>,
    movers: Query<(Entity, Option<&crate::player::Bankrupt>), With<Walker>>,
    mut detained: MessageWriter<PlayerDetained>,
    mut mounted: MessageWriter<crate::msg::MountChanged>,
) {
    let Some(stop) = halt.last.take() else {
        return;
    };
    let def = &books.active.def;
    // 地雷与炸弹都会毁车（PLAN §三「载具」行）。恶犬不毁 —— 有车的人把它撞飞。
    if matches!(stop.cause, HaltCause::Mine | HaltCause::TimeBomb) {
        lose_mount(&mut mounts, &mut mounted, stop.player);
    }
    match stop.cause {
        HaltCause::Mine | HaltCause::Dog => {
            detain_and_report(
                &mut commands,
                &mut positions,
                &mut books,
                &villains,
                stop.player,
                DetentionPlace::Hospital,
                MINE_HOSPITAL_DAYS,
                &mut detained,
            );
        }
        HaltCause::TimeBomb => {
            // 3×3 的命中集合先算完：`def` 是 `books.active` 的借用，不能与 `damage_lands`
            // 的可变借用同时活着
            let lands = plot::lands_around_tile(def, stop.tile, 3);
            // 爆心 3×3 里的**每一个还没出局的行动者**（P12 起含受雇的恶人：他从雷区走出来
            // 却没事，等于告诉玩家「恶人不怕炸弹」，而那不是任何一条规则说过的话）
            let victims: Vec<Entity> = movers
                .iter()
                .filter(|(entity, bankrupt)| {
                    bankrupt.is_none()
                        && positions
                            .get(*entity)
                            .map(|p| within_chebyshev(def, stop.tile, p.tile, 1))
                            .unwrap_or(false)
                })
                .map(|(entity, _)| entity)
                .collect();
            // 3×3 降 1 级、保所有权（PLAN §七 拍板）
            books.damage_lands(&lands, LandDamage::Degrade { levels: 1 });
            // 含携带者。已经在拘留中的人交给 `detain`
            // 的「更久的那一张赢」，这里不另写分支。
            for victim in victims {
                detain_and_report(
                    &mut commands,
                    &mut positions,
                    &mut books,
                    &villains,
                    victim,
                    DetentionPlace::Hospital,
                    BOMB_HOSPITAL_DAYS,
                    &mut detained,
                );
            }
        }
    }
}

/// 毁载具：回到步行并留一条凭据。已经步行的人不发 —— 那条流水说的是「车没了」。
fn lose_mount(
    mounts: &mut Query<&mut crate::player::Mount, With<Walker>>,
    changed: &mut MessageWriter<crate::msg::MountChanged>,
    player: Entity,
) {
    let Ok(mut mount) = mounts.get_mut(player) else {
        return;
    };
    if !mount.is_vehicle() {
        return;
    }
    *mount = crate::player::Mount::Foot;
    changed.write(crate::msg::MountChanged {
        player,
        mount: *mount,
    });
}

/// 中断的后果结算完之后收尾。单独一段是因为 `resolve_halt_system` 不该顺手推回合
/// （与落格那条「域 system 不碰 `NextState`」同一条分工）。
pub fn finish_halt_system(mut next: ResMut<NextState<TurnPhase>>) {
    next.set(TurnPhase::TurnEnd);
}

/// 把一个人送进医院并留凭据。抽出来是因为炸弹的 3×3 要对一圈人各做一次，
/// 而「接入点 + 死胡同格 + 更久的那张赢」只能由 `landing::detain` 那一处算。
///
/// `days` 是**玩家**那一份的住院天数；挨这一下的是恶人时它换成「回笼等保释」，
/// 换算只发生在 [`crate::villain::custody_of`] 那一处（P12）。
// 与调用方 `resolve_halt_system` 同一条处理：这一把工具拆散了只会让两处各借一遍
#[allow(clippy::too_many_arguments)]
fn detain_and_report(
    commands: &mut Commands,
    positions: &mut Query<&mut TokenPosition, With<Walker>>,
    books: &mut Books<'_, '_>,
    villains: &Query<&crate::villain::Villain>,
    player: Entity,
    place: DetentionPlace,
    days: u32,
    detained: &mut MessageWriter<PlayerDetained>,
) {
    let term = crate::villain::custody_of(villains.get(player).ok(), days);
    let Some((held, junction, end)) = detain(
        commands,
        positions,
        &mut books.detained,
        &books.active.def,
        player,
        place,
        term,
    ) else {
        return;
    };
    detained.write(PlayerDetained {
        player,
        place: held.place,
        junction,
        end,
    });
}

/// 两格在网格上的切比雪夫距离是否 ≤ `radius`。炸弹的 3×3 按格算，不按地块算
/// （ADR-002 §8：`Books::damage` 只吃地块集合，所以「以格为中心圈哪些地」在 `plot` 里另算）。
pub fn within_chebyshev(def: &BoardDef, a: TileIndex, b: TileIndex, radius: u32) -> bool {
    let (Some(a), Some(b)) = (def.tile(a.0 as i64), def.tile(b.0 as i64)) else {
        return false;
    };
    a.col.abs_diff(b.col) <= radius && a.row.abs_diff(b.row) <= radius
}

/// 这一格上有没有「经过即停」的东西。走步循环用它判路障截断与地雷/恶犬。
pub fn pass_kind_at(
    occupancies: &[(Entity, Occupancy)],
    tile: TileIndex,
) -> Option<(Entity, OccupancyKind)> {
    occupancies
        .iter()
        .find(|(_, occ)| {
            occ.at == OccupancyAt::OnTile(tile) && trigger_of(occ.kind) == Trigger::Pass
        })
        .map(|(e, occ)| (*e, occ.kind))
}

/// 路径里第一个路障的**位置**（含该格）。掷骰入队时用它截断 `walk_path` 的结果 ——
/// `walk_path` 本身仍是纯拓扑函数，不认识 occupancy（ADR-002 §6）。
///
/// 返回的是下标而不是格子：调用方要 `truncate(at + 1)`，保留路障格本身。
pub fn first_roadblock(
    occupancies: &Query<(Entity, &Occupancy)>,
    path: &[TileIndex],
) -> Option<usize> {
    path.iter().position(|tile| {
        occupancies.iter().any(|(_, occ)| {
            occ.kind == OccupancyKind::Roadblock && occ.at == OccupancyAt::OnTile(*tile)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::BoardDef;

    const PROTOTYPE: &str = include_str!("../../../assets/data/boards/prototype.json");

    fn board() -> BoardDef {
        BoardDef::from_json(PROTOTYPE).expect("prototype.json 应能解析")
    }

    #[test]
    fn every_kind_states_when_it_triggers() {
        use OccupancyKind as K;
        let rows = [
            (K::Roadblock, Trigger::Pass),
            (K::Mine, Trigger::Pass),
            (K::TimeBomb, Trigger::Pass),
            (K::Dog, Trigger::Pass),
            (K::Chest, Trigger::Land),
            // 神取一位代表整族：`trigger_of` 对十三位给的是同一个答案，
            // 而「路过不给」这条正是 P6 漫游不触发附身的那个判据
            (K::God(crate::gods::God::BigRich), Trigger::Land),
            (K::God(crate::gods::God::Death), Trigger::Land),
        ];
        for (kind, want) in rows {
            assert_eq!(trigger_of(kind), want, "{kind:?} 的触发口径变了");
            assert!(!kind.label().is_empty());
        }
    }

    #[test]
    fn placement_refuses_every_kind_of_bad_target_with_its_own_words() {
        let mine = ItemKind::Mine;
        assert_eq!(
            place_denied(ItemKind::RemoteDice, 1, true, false, false),
            Err(PlaceDeny::NotPlaceable)
        );
        assert_eq!(
            place_denied(mine, 0, true, false, false),
            Err(PlaceDeny::NotHeld)
        );
        assert_eq!(
            place_denied(mine, 1, false, false, false),
            Err(PlaceDeny::NoSuchTile)
        );
        assert_eq!(
            place_denied(mine, 1, true, true, false),
            Err(PlaceDeny::Occupied),
            "一格一件"
        );
        assert_eq!(
            place_denied(mine, 1, true, false, true),
            Err(PlaceDeny::SomebodyThere),
            "玩家主动放置不许放在有人站着的格子上"
        );
        assert_eq!(place_denied(mine, 1, true, false, false), Ok(()));
        let words: Vec<&str> = [
            PlaceDeny::NotNow,
            PlaceDeny::NotHeld,
            PlaceDeny::NotPlaceable,
            PlaceDeny::NoSuchTile,
            PlaceDeny::Occupied,
            PlaceDeny::SomebodyThere,
        ]
        .iter()
        .map(|d| d.label())
        .collect();
        assert!(words.iter().all(|w| !w.is_empty()));
        let mut uniq = words.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), words.len(), "两种拒绝说了同一句话: {words:?}");
    }

    #[test]
    fn chebyshev_box_is_a_grid_not_a_ring_distance() {
        let b = board();
        // 6 号 (5,2) 与 7 号 (5,3) 相邻；与 2 号 (3,1) 距离 2，不在 3×3 里
        assert!(within_chebyshev(&b, TileIndex(6), TileIndex(7), 1));
        assert!(within_chebyshev(&b, TileIndex(6), TileIndex(6), 1));
        assert!(!within_chebyshev(&b, TileIndex(6), TileIndex(2), 1));
        // 环上的距离与此无关：18 号 (0,1) 与 0 号 (1,1) 相邻，13 号 (1,4) 不相邻
        assert!(within_chebyshev(&b, TileIndex(18), TileIndex(0), 1));
        assert!(!within_chebyshev(&b, TileIndex(18), TileIndex(13), 1));
    }
}
