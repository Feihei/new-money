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

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, Detention, DetentionPlace, Pending, PendingDecision, PlayerDetained, PlayerId,
    Points, TokenPosition, TurnOrder, TurnPhase, detain,
};

pub struct DevPlugin;

/// 与 `NM_DUMP` 同一接线形状：主入口只看这一句，装不装由本模块自己判。
pub fn install_if_requested(app: &mut App) {
    let forced = ForceDetention::from_env();
    let bail = ForceBail::from_env();
    if forced.0.is_none() && bail.0.is_none() {
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

impl Plugin for DevPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Injected>()
            .insert_resource(ForceDetention::from_env())
            .insert_resource(ForceBail::from_env())
            // 回菜单时清掉「已经注入过」：`NM_DETAIN` 配 `NM_MENULOOP` 要能在第二局再注入一次，
            // 而留在 true 就等于「新局什么都没有」，看着像注入失效（跨局残留那类坑的反面）
            .add_systems(OnEnter(crate::scene::Scene::Menu), clear_injected)
            // 两道门都要：只挡 `TurnPhase::WaitRoll` 会在「回菜单」那一帧抢跑 —— 那时场景已经
            // 切走、`TurnPhase` 还没停，于是把正要 despawn 的旧实体扣住，本局的注入名额白白用掉
            // （第一次冒烟丢的就是第二局）。只挡场景则会在 `Boot` 态空转，那时还没有玩家可扣
            .add_systems(
                Update,
                (force_bail, force_detention.after(force_bail))
                    .run_if(in_state(crate::scene::Scene::Board))
                    .run_if(in_state(TurnPhase::WaitRoll)),
            );
    }
}

/// 「本局已经注入过一次」。**一局一次**：注入的是「回合开始那一刻之后」的状态，
/// 再来一次就等于凭空多关一个人，或把刚放出来的人再抓回去。
#[derive(Resource, Default)]
struct Injected(bool);

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
    let (Some(place), false) = (force.0, done.0) else {
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
        done.0 = true;
        warn!("{} 在这张棋盘上没有那条分支，什么都没注入", place.label());
        return;
    };
    done.0 = true;
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
    let (Some(place), false) = (force.0, done.0) else {
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
        done.0 = true;
        warn!("{} 在这张棋盘上没有那条分支，什么都没注入", place.label());
        return;
    };
    done.0 = true;
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
