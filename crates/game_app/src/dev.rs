//! 开发用的状态注入。默认不装 —— 给了对应环境变量才接线。
//!
//! 与 [`crate::autopilot`] 分家的理由：自动玩家做的是**人用按钮就能做出来的事**（掷骰、
//! 答模态），这里做的是「没有那只手能做出来」的事。P4 的验收原话是「有人住院时棋子停在
//! 医院死胡同格」，而住院那张是命运池的**第 7 张** —— 靠掷骰撞它要先连撞七次命运格，
//! 截图时刻根本押不住。同一条理由前面已经付过三次学费（`NM_PICK_TILE`、`NM_DAY`、
//! `NM_DECISION`）：**进不了截图的状态等于没做完**。
//!
//! 它只写「他被这一处扣住了」这一件事，不推进回合、不替任何人做决定（分层红线：core 拥有
//! 逻辑位置与步时，这里只是从表现层按一次 core 自己导出的 [`detain`]，走的正是命运牌那条路）。

use std::collections::HashSet;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, ArcadeDeal, Detention, DetentionPlace, GameKind, GameSession, God, GodDesk,
    Inventory, ItemKind, Mount, MountChanged, Occupancy, OccupancyKind, Pending, PendingDecision,
    PlayerDetained, PlayerId, Points, TileIndex, TokenPosition, TurnOrder, TurnPhase, detain,
    place,
};

pub struct DevPlugin;

/// 与 `NM_DUMP` 同一接线形状：主入口只看这一句，装不装由本模块自己判。
pub fn install_if_requested(app: &mut App) {
    let forced = ForceDetention::from_env();
    let bail = ForceBail::from_env();
    let items = ForceItems::from_env();
    let mount = ForceMount::from_env();
    let places = ForcePlace::from_env();
    let god = ForceGod::from_env();
    let minigame = ForceMinigame::from_env();
    if forced.0.is_none()
        && bail.0.is_none()
        && !items.0
        && mount.0.is_none()
        && places.0.is_empty()
        && god.0.is_none()
        && minigame.0.is_none()
    {
        return;
    }
    if let Some(place) = forced.0 {
        info!(
            "开发注入已开启（NM_DETAIN）：当前玩家将被关进{}，判 {DAYS} 回合",
            place.label()
        );
    }
    if let Some(place) = bail.0 {
        info!(
            "开发注入已开启（NM_BAIL）：另一位玩家将被关进{}，当前玩家面前挂出保释窗口",
            place.label()
        );
    }
    if items.0 {
        info!("开发注入已开启（NM_ITEMS）：当前玩家各得一个道具");
    }
    if let Some(mount) = mount.0 {
        info!(
            "开发注入已开启（NM_MOUNT）：当前玩家的载具是{}",
            mount.label()
        );
    }
    if !places.0.is_empty() {
        info!(
            "开发注入已开启（NM_PLACE）：预放 {} 件道路放置物",
            places.0.len()
        );
    }
    if let Some(god) = god.0 {
        info!(
            "开发注入已开启（NM_GOD）：当前玩家将被{}附身（{} 天）",
            god.label(),
            god.days()
        );
    }
    if let Some(game) = minigame.0 {
        info!(
            "开发注入已开启（NM_MINIGAME）：当前玩家面前挂出「{}」那一屏",
            game.label()
        );
    }
    app.add_plugins(DevPlugin);
}

/// 回合数写死 3：验收要看的是「判 3 = 跳过 2」这一条口径在三个去处都成立，
/// 而那个数本身是卡面上的事，已由 `tests/detention.rs` 钉住。
const DAYS: u32 = 3;

/// `NM_DETAIN=<jail|hospital|away>` 要注入的那一处。`None` = 没开。
#[derive(Resource)]
struct ForceDetention(Option<DetentionPlace>);

impl ForceDetention {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_DETAIN") else {
            return Self(None);
        };
        let place = match raw.trim() {
            "jail" => DetentionPlace::Jail,
            "hospital" => DetentionPlace::Hospital,
            "away" => DetentionPlace::Away,
            other => {
                warn!("NM_DETAIN={other} 不认识，应为 jail|hospital|away");
                return Self(None);
            }
        };
        Self(Some(place))
    }
}

/// `NM_BAIL=<jail|hospital>`：把**另一位**玩家关进去，并在当前玩家面前挂出保释窗口。
///
/// 与 `NM_DETAIN` 同一条理由：保释窗口的正经入口是「别人恰好踩到接入点」，而那一掷
/// 由骰子说了算 —— 截图脚本押不住。它只写「有人被关着 + 窗口摆出来」这两件事，
/// 不替任何人按按钮；`WaitDecision` 是那个窗口在规则里的真实阶段。
#[derive(Resource)]
struct ForceBail(Option<DetentionPlace>);

impl ForceBail {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_BAIL") else {
            return Self(None);
        };
        let place = match raw.trim() {
            "jail" => DetentionPlace::Jail,
            "hospital" => DetentionPlace::Hospital,
            other => {
                warn!("NM_BAIL={other} 不认识，应为 jail|hospital");
                return Self(None);
            }
        };
        Self(Some(place))
    }
}

/// `NM_ITEMS=1`：给当前玩家各发一个放置类道具与一颗遥控骰子。
///
/// 道具的正经来源是宝箱（落脚才开）与 P10 的研究所，两者都不由截图脚本控制。
/// 面板的「非鼠标入口」是 `NM_ITEMS`，而面板要有东西可放，就得先有道具。
#[derive(Resource)]
struct ForceItems(bool);

impl ForceItems {
    fn from_env() -> Self {
        Self(std::env::var_os("NM_ITEMS").is_some())
    }
}

/// `NM_MOUNT=foot|bike|car`：把当前玩家的载具换掉（车行 P10 才有）。
///
/// 载具唯一可见的后果是「掷骰面板能选 1..=3 颗」与「踩雷毁车」，两者都没有别的
/// 不靠运气的入口 —— 不注入就拍不到。
#[derive(Resource)]
struct ForceMount(Option<Mount>);

impl ForceMount {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_MOUNT") else {
            return Self(None);
        };
        let mount = match raw.trim() {
            "foot" => Mount::Foot,
            "bike" => Mount::Bike,
            "car" => Mount::Car,
            other => {
                warn!("NM_MOUNT={other} 不认识，应为 foot|bike|car");
                return Self(None);
            }
        };
        Self(Some(mount))
    }
}

/// `NM_PLACE=roadblock@5,mine@6`：开局在路上预放几件东西。
///
/// 路障要挡的人得下一回合才走到，地雷要有人踩上去才弹住院 —— 两件事都依赖骰子。
/// 这个旋钮只放东西，不推进回合；放不进去（格号越界/已占）会逐条 warn。
#[derive(Resource)]
struct ForcePlace(Vec<(OccupancyKind, TileIndex)>);

impl ForcePlace {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_PLACE") else {
            return Self(Vec::new());
        };
        let mut out = Vec::new();
        for part in raw.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            let Some((kind, tile)) = part.split_once('@') else {
                warn!("NM_PLACE 的 {part:?} 不是 kind@tile 形状，跳过");
                continue;
            };
            let kind = match kind {
                "roadblock" => OccupancyKind::Roadblock,
                "mine" => OccupancyKind::Mine,
                "bomb" => OccupancyKind::TimeBomb,
                "dog" => OccupancyKind::Dog,
                "chest" => OccupancyKind::Chest,
                other => {
                    warn!("NM_PLACE 的 {other:?} 不是 roadblock|mine|bomb|dog|chest，跳过");
                    continue;
                }
            };
            match tile.parse::<u32>() {
                Ok(n) => out.push((kind, TileIndex(n))),
                Err(_) => warn!("NM_PLACE 的格号 {tile:?} 不是整数，跳过"),
            }
        }
        Self(out)
    }
}

/// `NM_GOD=天使` / `NM_GOD=Angel`：把一位神直接附到当前玩家身上。
///
/// 附身的正经入口是「踩到路上那一位」，而路上刷谁由随机子流决定、谁踩到由骰子决定。
/// P6c 那三条效果吃的是**步数与落脚格**（「所经」要走过好几格），靠运气拍不到一张能说清
/// 问题的图 —— 与 `NM_DETAIN`/`NM_MOUNT` 同一条理由：进不了截图的状态等于没做完。
///
/// 中文名与英文名都收，因为 [`God::ALL`] 就是那张穷尽表，不必再维护一份 slug 对照。
#[derive(Resource)]
struct ForceGod(Option<God>);

impl ForceGod {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_GOD") else {
            return Self(None);
        };
        match Self::parse(raw.trim()) {
            Some(god) => Self(Some(god)),
            None => {
                warn!("NM_GOD={raw} 不认识，应为 12 位神名（大财神…死神）或它的英文名");
                Self(None)
            }
        }
    }

    /// 认中文名也认英文名（`Angel`／`angel` 都行）。单独成一句是为了能被测到：
    /// 一个拼错的旋钮值不会崩、也不会红，只会「注入没发生」，而那张截图看起来只是
    /// 「今天没有神」—— 与 `ui/hand.rs` 那条「样例卡的 id 不在货架上，NM_HAND 拍不到东西」同一种坑。
    fn parse(raw: &str) -> Option<God> {
        God::ALL
            .into_iter()
            .find(|g| g.label() == raw || format!("{g:?}").eq_ignore_ascii_case(raw))
    }
}

/// `NM_MINIGAME=pop_balloons|catch_treasure|penguin_dig`（中文名也认）。
///
/// 游乐场那一屏的正经入口是「踩到 13 号格」，而演示盘那一格在环尾，掷 1～2 点才踩得到 ——
/// 三款里抽到哪一款又由随机子流说了算。三个玩法各要一张能看清局面的截图，靠骰子押不住
/// （与 `NM_DETAIN`/`NM_GOD` 同一条理由：**进不了截图的状态等于没做完**）。
///
/// 它写的是**规则层那个窗口**（`Pending::Minigame` + 一手布局种子），不是 UI 侧的开关 ——
/// 于是这一屏走的正是踩到那一格的同一条路，`sync_arcade` 不需要为旋钮单开一个入口。
#[derive(Resource)]
struct ForceMinigame(Option<GameKind>);

impl ForceMinigame {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_MINIGAME") else {
            return Self(None);
        };
        match GameKind::parse(raw.trim()) {
            Some(game) => Self(Some(game)),
            None => {
                warn!(
                    "NM_MINIGAME={raw} 不认识，应为 pop_balloons|catch_treasure|penguin_dig（或中文名）"
                );
                Self(None)
            }
        }
    }
}

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Injected>()
            .insert_resource(ForceDetention::from_env())
            .insert_resource(ForceBail::from_env())
            .insert_resource(ForceItems::from_env())
            .insert_resource(ForceMount::from_env())
            .insert_resource(ForcePlace::from_env())
            .insert_resource(ForceGod::from_env())
            .insert_resource(ForceMinigame::from_env())
            // 回菜单时清掉「已经注入过」：`NM_DETAIN` 配 `NM_MENULOOP` 要能在第二局再注入一次，
            // 而留在 true 就等于「新局什么都没有」，看着像注入失效（跨局残留那类坑的反面）
            .add_systems(OnEnter(crate::scene::Scene::Menu), clear_injected)
            // 两道门都要：只挡 `TurnPhase::WaitRoll` 会在「回菜单」那一帧抢跑 —— 那时场景已经
            // 切走、`TurnPhase` 还没停，于是把正要 despawn 的旧实体扣住，本局的注入名额白白用掉
            // （第一次冒烟丢的就是第二局）。只挡场景则会在 `Boot` 态空转，那时还没有玩家可扣
            .add_systems(
                Update,
                (
                    force_bail,
                    force_detention.after(force_bail),
                    force_items,
                    force_mount,
                    force_place,
                    force_god,
                    force_minigame,
                )
                    .run_if(in_state(crate::scene::Scene::Board))
                    .run_if(in_state(TurnPhase::WaitRoll)),
            );
    }
}

/// 「本局已经注入过一次」。**一局一次**：注入的是「回合开始那一刻之后」的状态，
/// 再来一次就等于凭空多关一个人、把刚放出来的人再抓回去，或在同一格叠第二件。
///
/// 每个旋钮一位而不是共用一位：`NM_ITEMS=1` 与 `NM_MOUNT=car` 常常一起给，
/// 共用一位会让后跑的那个永远轮不到。
#[derive(Resource, Default)]
struct Injected {
    detention: bool,
    bail: bool,
    items: bool,
    mount: bool,
    place: bool,
    god: bool,
    minigame: bool,
}

fn clear_injected(mut done: ResMut<Injected>) {
    *done = Injected::default();
}

/// `detain` 要的那三样写打包成一个参数包（八个散装参数会撞 clippy 的 `too_many_arguments`，
/// 与 P3d③ 那次「打包而不是 `#[allow]`」同一条处理）。
#[derive(SystemParam)]
struct DetainWriters<'w, 's> {
    commands: Commands<'w, 's>,
    positions: Query<'w, 's, &'static mut TokenPosition, With<PlayerId>>,
    held: Query<'w, 's, &'static mut Detention, With<PlayerId>>,
}

/// 把当前玩家送进那一处，并发**同一条凭据** —— 渲染层读的是 `PlayerDetained` 里那个
/// 死胡同格坐标，只插组件的话棋子会留在原地，而「留在原地」恰恰是 `away` 那一支的语义，
/// 于是截图就看不出三种去处的差别了。
fn force_detention(
    force: Res<ForceDetention>,
    bail: Res<ForceBail>,
    active: Res<ActiveBoard>,
    order: Res<TurnOrder>,
    mut writers: DetainWriters,
    mut detained: MessageWriter<PlayerDetained>,
    mut done: ResMut<Injected>,
) {
    let (Some(place), false) = (force.0, done.detention) else {
        return;
    };
    // 两个旋钮同时给时 `NM_BAIL` 优先：它的场景更强（窗口 + 一个被关在别处的人）
    if bail.0.is_some() {
        return;
    }
    // 棋盘还没装配好（菜单里、或开局那一帧之前）就等着：`current()` 为空 = 还没有人可扣
    let Some(player) = order.current() else {
        return;
    };
    let Some((sentence, junction, end)) = detain(
        &mut writers.commands,
        &mut writers.positions,
        &mut writers.held,
        &active.def,
        player,
        place,
        DAYS,
    ) else {
        done.detention = true;
        warn!("{} 在这张棋盘上没有那条分支，什么都没注入", place.label());
        return;
    };
    done.detention = true;
    info!(
        "注入：当前玩家被关进{}（判 {DAYS} 回合），接入点格{}",
        sentence.place.label(),
        junction.0
    );
    detained.write(PlayerDetained {
        player,
        place: sentence.place,
        junction,
        end,
    });
}

/// `force_bail` 要读的两样棋盘事实。打包是房规（§六 坑 8）：它俩一起回答
/// 「现在轮到谁、这张盘上那条分支在哪儿」。
#[derive(SystemParam)]
struct BailScene<'w, 's> {
    active: Res<'w, ActiveBoard>,
    order: Res<'w, TurnOrder>,
    /// 为了让「保释」那颗按钮在截图里是亮的，注入时把当前玩家的点券补到 100
    points: Query<'w, 's, &'static mut Points, With<PlayerId>>,
}

/// `NM_BAIL` 的场景：另一位玩家被关进 `place`，窗口挂在当前玩家面前。
///
/// 不替任何人按按钮 —— 玩家仍然要自己点「保释 某人」或「离开」。把阶段切到
/// `WaitDecision` 是那个窗口在规则里的真实位置（`Pending::Bail` 有回答才能收）。
fn force_bail(
    force: Res<ForceBail>,
    mut scene: BailScene<'_, '_>,
    mut writers: DetainWriters,
    mut pending: ResMut<PendingDecision>,
    mut detained: MessageWriter<PlayerDetained>,
    mut done: ResMut<Injected>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let (Some(place), false) = (force.0, done.bail) else {
        return;
    };
    let Some(current) = scene.order.current() else {
        return;
    };
    let Some(target) = scene.order.order.iter().copied().find(|e| *e != current) else {
        return;
    };
    let Some((sentence, junction, end)) = detain(
        &mut writers.commands,
        &mut writers.positions,
        &mut writers.held,
        &scene.active.def,
        target,
        place,
        DAYS,
    ) else {
        done.bail = true;
        warn!("{} 在这张棋盘上没有那条分支，什么都没注入", place.label());
        return;
    };
    done.bail = true;
    info!(
        "注入：另一位玩家被关进{}（判 {DAYS} 回合），当前玩家面前挂出保释窗口",
        sentence.place.label()
    );
    detained.write(PlayerDetained {
        player: target,
        place: sentence.place,
        junction,
        end,
    });
    pending.offer(current, Pending::Bail(sentence.place));
    // 截图要看到一颗**亮着**的「保释」按钮：正经对局里 30 点券要靠点券箱攒，
    // 而注入场景不经过那一格。只补到够保一次，别顺手把余额写成一个大数
    if let Ok(mut points) = scene.points.get_mut(current) {
        points.0 = points.0.max(100);
    }
    next.set(TurnPhase::WaitDecision);
}

/// `NM_ITEMS=1` 的那一半：把四种道具各发一个给当前玩家。
/// 走 `Inventory::gain` 而不是直接改容器 —— 堆叠规则只住在它里面。
fn force_items(
    force: Res<ForceItems>,
    order: Res<TurnOrder>,
    mut inventories: Query<&mut Inventory, With<PlayerId>>,
    mut done: ResMut<Injected>,
) {
    if !force.0 || done.items {
        return;
    }
    let Some(current) = order.current() else {
        return;
    };
    done.items = true;
    if let Ok(mut inventory) = inventories.get_mut(current) {
        for kind in ItemKind::ALL {
            inventory.gain(kind, 1);
        }
        info!("注入：当前玩家得到四种道具各一个");
    }
}

/// `NM_MOUNT=car`：把当前玩家的载具换掉并留一条凭据（顶栏会播报）。
fn force_mount(
    force: Res<ForceMount>,
    order: Res<TurnOrder>,
    mut mounts: Query<&mut Mount, With<PlayerId>>,
    mut changed: MessageWriter<MountChanged>,
    mut done: ResMut<Injected>,
) {
    let (Some(wanted), false) = (force.0, done.mount) else {
        return;
    };
    let Some(current) = order.current() else {
        return;
    };
    done.mount = true;
    if let Ok(mut mount) = mounts.get_mut(current) {
        *mount = wanted;
        changed.write(MountChanged {
            player: current,
            mount: wanted,
        });
        info!("注入：当前玩家的载具换成{}", wanted.label());
    }
}

/// `NM_PLACE=roadblock@5,mine@6`：预放道路放置物。走 `occupancy::place` 的唯一闸门，
/// 所以「一格一件」与 `NM_PLACE` 自己两行撞同一格这两件事都被它挡住并 warn。
fn force_place(
    force: Res<ForcePlace>,
    active: Res<ActiveBoard>,
    mut commands: Commands,
    occupancies: Query<(Entity, &Occupancy)>,
    mut done: ResMut<Injected>,
) {
    if force.0.is_empty() || done.place {
        return;
    }
    done.place = true;
    let mut claimed = HashSet::new();
    for (kind, tile) in &force.0 {
        if (tile.0 as usize) >= active.def.loop_len() as usize {
            warn!("NM_PLACE：{} 号格不在主环上，跳过", tile.0);
            continue;
        }
        let at = occupancies.iter().map(|(entity, occ)| (entity, occ.at));
        if place(&mut commands, at, &mut claimed, *kind, *tile).is_none() {
            warn!("NM_PLACE：{} 号格已经有一件东西，跳过", tile.0);
            continue;
        }
        info!("注入：{} 放在 {} 号格", kind.label(), tile.0);
    }
}

/// `NM_GOD=天使`：把一位神附到当前玩家身上。
///
/// 走 [`attach`] 而不是自己插组件 —— 那是规则里唯一的附身出口，四件事必须同时成立
/// （顶掉旧神、挂修正链、做完一次性效果、发 `GodPossessed`）。插个裸组件会得到
/// 「身上多一位、账与房子都不动」，而那正是这条函数被合成一句的理由。
fn force_god(
    force: Res<ForceGod>,
    order: Res<TurnOrder>,
    positions: Query<&TokenPosition, With<PlayerId>>,
    mut desk: GodDesk<'_, '_>,
    mut done: ResMut<Injected>,
) {
    let (Some(god), false) = (force.0, done.god) else {
        return;
    };
    let Some(player) = order.current() else {
        return;
    };
    done.god = true;
    let at = positions
        .get(player)
        .map(|pos| pos.tile)
        .unwrap_or(TileIndex(0));
    desk.attach(player, god, at);
    info!(
        "注入：当前玩家被{}附身（{} 天），他从 {} 号格起走",
        god.label(),
        god.days(),
        at.0
    );
}

/// `NM_MINIGAME` 的场景：窗口挂到当前玩家面前，一手布局种子一起发下去。
///
/// 种子取本局那个（`GameSession::seed`）而不是随手摇一个：配 `NM_SEED` 时同一张截图
/// 要能重拍出一模一样的一手 —— 挖宝埋在哪几格是这一屏唯一由随机决定的东西。
fn force_minigame(
    force: Res<ForceMinigame>,
    order: Res<TurnOrder>,
    session: Option<Res<GameSession>>,
    mut pending: ResMut<PendingDecision>,
    mut deal: ResMut<ArcadeDeal>,
    mut done: ResMut<Injected>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let (Some(game), false) = (force.0, done.minigame) else {
        return;
    };
    let Some(player) = order.current() else {
        return;
    };
    done.minigame = true;
    deal.seed = session.map(|s| s.seed).unwrap_or(0);
    pending.offer(player, Pending::Minigame(game));
    // `WaitDecision` 是那个窗口在规则里的真实阶段（与 `force_bail` 同一条）
    next.set(TurnPhase::WaitDecision);
    info!("注入：当前玩家面前挂出「{}」那一屏", game.label());
}

#[cfg(test)]
mod tests {
    use super::ForceGod;
    use game_core::prelude::{GameKind, God};

    /// 每一位神都要能被两种写法点名 —— 这张穷尽表是旋钮唯一的名字来源，
    /// 谁把它换成手写 slug 对照，加一位神时就会漏掉一位（漏掉的那一位拍不到，
    /// 而症状是「今天没有神」，不是报错）
    #[test]
    fn nm_god_accepts_both_name_forms_for_every_god() {
        for god in God::ALL {
            assert_eq!(
                ForceGod::parse(god.label()),
                Some(god),
                "中文名 {}",
                god.label()
            );
            let english = format!("{god:?}");
            assert_eq!(ForceGod::parse(&english), Some(god), "英文名 {english}");
            assert_eq!(
                ForceGod::parse(&english.to_lowercase()),
                Some(god),
                "小写 {english}"
            );
        }
    }

    /// 名字撞车 = 旋钮分不出要给哪一位（`find` 永远给表里排前面的那个，且不报错）
    #[test]
    fn no_two_gods_share_a_label() {
        let mut labels: Vec<&str> = God::ALL.iter().map(|god| god.label()).collect();
        labels.sort_unstable();
        let every = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), every, "有两位神同名：{labels:?}");
    }

    #[test]
    fn an_unknown_name_injects_nothing() {
        assert_eq!(ForceGod::parse("土地婆"), None);
        assert_eq!(ForceGod::parse(""), None);
    }

    /// `NM_MINIGAME` 认的三个值必须真是那三款的数据名（`pop_balloons` 是写进
    /// `prototype.json` 的那个串）。拼错的旋钮值不崩也不红，只会「注入没发生」，
    /// 而那张截图看起来只是「这一格今天不是游乐场」—— 与 `nm_god_...` 同一种坑。
    #[test]
    fn nm_minigame_accepts_the_three_data_names() {
        for (raw, want) in [
            ("pop_balloons", GameKind::PopBalloons),
            ("catch_treasure", GameKind::CatchTreasure),
            ("penguin_dig", GameKind::PenguinDig),
        ] {
            assert_eq!(GameKind::parse(raw), Some(want), "旋钮值 {raw}");
        }
        assert_eq!(GameKind::parse("balloons"), None);
        assert_eq!(GameKind::parse(""), None);
    }
}
