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
    ActiveBoard, ArcadeDeal, Auction, BankBalance, BuildingLevel, BuyTicketRequested, Detention,
    DetentionPlace, GameKind, GameRules, GameSession, Gavel, God, GodDesk, Hired, Inventory,
    ItemKind, LandId, Listing, Mount, MountChanged, NewsSink, Occupancy, OccupancyKind, Owner,
    Pending, PendingDecision, PlayerDetained, PlayerId, Points, RemoteBuildOffered, Rng, Term,
    TileIndex, TokenPosition, TurnOrder, TurnPhase, Villain, VillainHired, VillainKind, Walker,
    detain, find_id, fire_news, place,
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
    let lottery = ForceLottery::from_env();
    let auction = ForceAuction::from_env();
    let house = ForceHouse::from_env();
    let rbuild = ForceRbuild::from_env();
    let news = ForceNews::from_env();
    let villain = ForceVillain::from_env();
    let magic = ForceMagic::from_env();
    let spell = ForceSpell::from_env();
    if forced.0.is_none()
        && bail.0.is_none()
        && !items.0
        && mount.0.is_none()
        && places.0.is_empty()
        && god.0.is_none()
        && minigame.0.is_none()
        && lottery.0.is_none()
        && auction.0.is_none()
        && house.0.is_none()
        && !rbuild.0
        && news.0.is_none()
        && villain.0.is_none()
        // 魔法屋那两步各一条：漏掉任何一条的症状都不是「旋钮不灵」，而是
        // **整个 DevPlugin 没装** —— 于是 `NM_MAGIC=girls` 拍出来是「今天没人踩到魔法屋」，
        // 与本模块上面那条「拼错的旋钮值不崩也不红」是同一族坑（P13 第一次冒烟就踩到了）
        && magic.0.is_none()
        && spell.0.is_none()
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
    if let Some(wanted) = lottery.0 {
        info!(
            "开发注入已开启（NM_LOTTERY）：当前玩家被摆到彩票柜台前，买下一注 {}",
            match wanted {
                LotteryWanted::Number(n) => n.to_string(),
                LotteryWanted::Auto => "本期开奖号（auto）".to_string(),
            }
        );
    }
    if let Some(land) = auction.0 {
        info!(
            "开发注入已开启（NM_AUCTION）：地块 {land} 将被挂上拍卖台（配 NM_PLOTS=<n> 让它有主人）"
        );
    }
    if let Some((land, level)) = house.0 {
        info!(
            "开发注入已开启（NM_HOUSE）：地块 {land} 将被盖上 Lv{level}（归属与钱都不动，配 NM_DECISION=buy 拍那块地的报价）"
        );
    }
    if rbuild.0 {
        info!(
            "开发注入已开启（NM_RBUILD）：当前玩家面前挂出远程建设窗口（配 NM_PLOTS=<n> 让他有地可升）"
        );
    }
    if let Some(wanted) = news.0.as_deref() {
        info!(
            "开发注入已开启（NM_NEWS={wanted}）：当前玩家的回合里立刻播那一条新闻（不踩格、不推进回合）"
        );
    }
    if let Some(kind) = villain.0 {
        info!(
            "开发注入已开启（NM_VILLAIN）：{} 将被当前玩家保释出场（点券补到够，出场行为随消息分派）",
            kind.label()
        );
    }
    if let Some(wanted) = magic.0 {
        info!(
            "开发注入已开启（NM_MAGIC）：当前玩家面前摆出魔法屋{}",
            match wanted {
                MagicWanted::Menu => "第一步（十一道条件）".to_string(),
                MagicWanted::Filter(filter) => format!("并替他答了条件「{}」", filter.label()),
            }
        );
    }
    if let Some(wanted) = spell.0 {
        info!(
            "开发注入已开启（NM_SPELL）：第二步窗口一挂上就替他求一道「{}」",
            wanted.0.label()
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

/// `NM_VILLAIN=<robber|thief|hoodlum|spy>`：把那一位恶人**保释给当前玩家**。
///
/// 保释的正经入口要 300 点券（点券箱攒很久）还要有人走到监狱门口，长跑里几乎不会发生
/// —— 而恶人的全部行为都发生在「出场之后」，不注入就一张也拍不到。这里走的是
/// `bail_op_system` 成交那一下的同一组写（摘 `Detention`、挂 `Hired`、插进轮转、发
/// `VillainHired`），出场行为由 `villain::on_hired_system` 下一帧照常分派 ——
/// 注入是走后门进同一扇门，不是开另一扇。点券补到够保释价，与 `NM_BAIL` 同一条理由。
#[derive(Resource)]
struct ForceVillain(Option<VillainKind>);

impl ForceVillain {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_VILLAIN") else {
            return Self(None);
        };
        match VillainKind::from_key(raw.trim()) {
            Some(kind) => Self(Some(kind)),
            None => {
                warn!("NM_VILLAIN={raw} 不认识，应为 robber|thief|hoodlum|spy");
                Self(None)
            }
        }
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
            .insert_resource(ForceLottery::from_env())
            .insert_resource(ForceAuction::from_env())
            .insert_resource(ForceHouse::from_env())
            .insert_resource(ForceRbuild::from_env())
            .insert_resource(ForceNews::from_env())
            .insert_resource(ForceVillain::from_env())
            .insert_resource(ForceMagic::from_env())
            .insert_resource(ForceSpell::from_env())
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
                    force_lottery,
                    force_auction,
                    force_house,
                    force_rbuild,
                    // 新闻排在盖楼之后：`NM_HOUSE=0:3 NM_NEWS=news_014` 那一张要拍的是
                    // 「楼被征收拆平」，而两条旋钮在同一个并行元组里时次序不保证（坑 36）——
                    // 抢在前面的那一版拍出来是一块本来就没楼的空地，看着像注入失效
                    force_news.after(force_house),
                    force_villain,
                    // 魔法屋第一步与保释/彩票同一族门（`WaitRoll` 时才有「当前玩家」可摆）
                    force_magic,
                )
                    .run_if(in_state(crate::scene::Scene::Board))
                    .run_if(in_state(TurnPhase::WaitRoll)),
            )
            // 第二步那颗按钮点出去**之后**相位已经是 `WaitDecision`，上面那道门永远关着，
            // 所以 `NM_SPELL` 只能挂在没有相位门禁的那一组里，由系统自己问「窗口是不是
            // 魔法屋的第二步」。一局一次的位还是它自己的，不会每帧重复施法。
            // ⚠️ 但**装配前那几帧仍然要挡**：它读 `TurnOrder`，而那是世界装配时才插进来的资源
            // —— 只挡场景会在开局第一帧就把 SystemParam 校验撞成 panic（坑 22 的那一族，
            // 第一次冒烟就是这么死的：症状不是「旋钮没生效」而是整局起不来）
            .add_systems(
                Update,
                force_spell
                    .run_if(in_state(crate::scene::Scene::Board))
                    .run_if(resource_exists::<game_core::prelude::TurnOrder>),
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
    lottery: bool,
    auction: bool,
    house: bool,
    rbuild: bool,
    news: bool,
    villain: bool,
    /// 魔法屋第一步（`NM_MAGIC`）与第二步（`NM_SPELL`）各一位：它们是两个旋钮、
    /// 两次回答，共用一位的话后给的那个永远轮不到（上面那段「每个旋钮一位」的同一条理由）
    magic: bool,
    spell: bool,
}

fn clear_injected(mut done: ResMut<Injected>) {
    *done = Injected::default();
}

/// `detain` 要的那三样写打包成一个参数包（八个散装参数会撞 clippy 的 `too_many_arguments`，
/// 与 P3d③ 那次「打包而不是 `#[allow]`」同一条处理）。
#[derive(SystemParam)]
struct DetainWriters<'w, 's> {
    commands: Commands<'w, 's>,
    // `With<Walker>` 而不是 `With<PlayerId>`：`detain` 的签名按行动者那一族收（P12），
    // 而这两个注入今天拍的都是玩家
    positions: Query<'w, 's, &'static mut TokenPosition, With<Walker>>,
    held: Query<'w, 's, &'static mut Detention, With<Walker>>,
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
        Term::Turns(DAYS),
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
    rules: Res<'w, GameRules>,
    /// 为了让「保释」那颗按钮在截图里是亮的，注入时把当前玩家的点券补到够保一次
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
        Term::Turns(DAYS),
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
    // 而注入场景不经过那一格。只补到够保一次，别顺手把余额写成一个大数。
    // P12a 之后同一排候选里还站着四位 300 的恶人，所以这一句补到「最贵的那一行也按得动」——
    // 少这一格，`NM_BAIL=jail` 的截图就只会给出四行灰着的「点券不够」，恶人那条路径上不了屏
    if let Ok(mut points) = scene.points.get_mut(current) {
        points.0 = points.0.max(30).max(scene.rules.villain_bail_points);
    }
    next.set(TurnPhase::WaitDecision);
}

/// `NM_VILLAIN` 的场景：那一位恶人被当前玩家保释出场，出场行为下一帧照常分派。
///
/// 与 `force_bail` 的差别在**主语**：那条摆窗口等人点，这条直接办成交 ——
/// 因为要拍的不是窗口，是恶人「在环上替人办事」的世界。存一笔钱进每位玩家的账户
/// 是同一份摆拍合同：强盗抢的是存款，存款是 0 的图只会拍出「抢走了 0」，
/// 那张图什么也说不清（与 `NM_BAIL` 补点券那一条同一个理由）。
// 参数是「把一次保释办成」的全套（轮转、恶人、两位的钱、消息），与注入一族同一条处理
#[allow(clippy::too_many_arguments)]
fn force_villain(
    force: Res<ForceVillain>,
    mut done: ResMut<Injected>,
    mut order: ResMut<TurnOrder>,
    villains: Query<(Entity, &Villain)>,
    mut players: Query<(Entity, &mut BankBalance), With<PlayerId>>,
    mut points: Query<&mut Points, With<PlayerId>>,
    rules: Res<GameRules>,
    mut commands: Commands,
    mut hired: MessageWriter<VillainHired>,
) {
    let (Some(wanted), false) = (force.0, done.villain) else {
        return;
    };
    let Some(current) = order.current() else {
        return;
    };
    done.villain = true;
    // 全场各存一笔（摆拍用，见上面的注）：出场那一刀才有东西可抢
    for (_, mut bank) in players.iter_mut() {
        bank.deposit += 10_000;
    }
    let Some((villain, held)) = villains.iter().find(|(_, v)| v.kind == wanted) else {
        warn!("场上找不到 {:?}（世界还没装配到恶人？）", wanted);
        return;
    };
    commands.entity(villain).remove::<Detention>();
    commands.entity(villain).insert(Hired { employer: current });
    if order.insert_after(villain, current) {
        debug!("注入：{:?} 接在当前玩家之后进入轮转", wanted);
    }
    // 点券补到够保释价 —— 这一笔没有走 spend_points（它说的是「已经付过了」，
    // 与 NM_BAIL 补到能按那颗按钮是同一个口径），但恶人行与 HUD 要读得出「为什么出得来」
    if let Ok(mut points) = points.get_mut(current) {
        points.0 = points.0.max(rules.villain_bail_points);
    }
    hired.write(VillainHired {
        villain,
        employer: current,
        kind: wanted,
    });
    info!(
        "注入：{} 被当前玩家保释出场（全场各存 10,000，好让出场行为拍得到）",
        held.label()
    );
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

/// `NM_LOTTERY` 要买的那一注：点名一个号，或 `auto` = 买「这一局第一次开奖会开出的那个号」。
///
/// `auto` 能从外面算，靠的是分流买来的一条性质：乐透那条子流只有 `draw_lottery_system`
/// 会取，而开局装配一个数都不取它 —— 所以「第一个号是什么」是种子的纯函数，用同一颗种子
/// 新建一个临时 `Rng` 取一次就读到了（`game_core/tests/lottery.rs` 用的是同一条性质）。
/// 这里**不动对局里那条流**，只是提前读出它下一步要给什么数。
///
/// 反过来不做的事是给 `Jackpot` 加一个「强制开奖号」的位：那等于让规则层多认一条
/// 生产环境永远不会设的分支。`EpochOverride` 与之不同 —— 开局锚点本来就是这一局的事实，
/// 而「这个月开几号」按设计只能是随机流上的一个数。
///
/// 只对**第一次**开奖有效（第二次要再取一次流，而这里不取）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum LotteryWanted {
    Number(u8),
    Auto,
}

impl LotteryWanted {
    /// `NM_LOTTERY` 的那个值怎么读。单开一个纯函数而不是写在 `from_env` 里：
    /// 读环境变量这件事没法在测试里安全地摆（与 `GameKind::parse` 同一条理由）。
    fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim();
        if raw == "auto" {
            return Some(Self::Auto);
        }
        raw.parse::<u8>().ok().map(Self::Number)
    }
}

/// `NM_LOTTERY=<号|auto>`：把当前玩家摆到彩票柜台前，并替他买下一注。`None` = 没开。
#[derive(Resource)]
struct ForceLottery(Option<LotteryWanted>);

impl ForceLottery {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_LOTTERY") else {
            return Self(None);
        };
        match LotteryWanted::parse(&raw) {
            Some(wanted) => Self(Some(wanted)),
            None => {
                warn!("NM_LOTTERY={raw} 不认识，应为一注的号码（1..=号码池）或 auto");
                Self(None)
            }
        }
    }
}

/// 这一注要写的两样 + 要读的四样。打包而不是散装（§六 坑 8），而 `next` 留在函数参数上：
/// 它是 `NextState`，装进参数包要把两个生命周期写死在一起，读起来比八个散装参数更难懂。
#[derive(SystemParam)]
struct LotteryDesk<'w> {
    order: Res<'w, TurnOrder>,
    rules: Res<'w, GameRules>,
    session: Option<Res<'w, GameSession>>,
    pending: ResMut<'w, PendingDecision>,
    requests: MessageWriter<'w, BuyTicketRequested>,
    done: ResMut<'w, Injected>,
}

fn force_lottery(
    force: Res<ForceLottery>,
    mut desk: LotteryDesk<'_>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let (Some(wanted), false) = (force.0, desk.done.lottery) else {
        return;
    };
    let Some(player) = desk.order.current() else {
        return;
    };
    // 号码池被配成 0 时夹成 1，与规则层 `draw_lottery_system` 同一条处理
    let numbers = desk.rules.lottery_numbers.max(1);
    let number = match wanted {
        LotteryWanted::Number(n) => n,
        LotteryWanted::Auto => {
            let seed = desk.session.map(|s| s.seed).unwrap_or(0);
            let mut probe = Rng::new(seed);
            probe.pick_lottery(1..=numbers) as u8
        }
    };
    desk.done.lottery = true;
    desk.pending.offer(player, Pending::Lottery);
    next.set(TurnPhase::WaitDecision);
    if number == 0 || u32::from(number) > numbers {
        // 窗口照样摆出来：拍不到「这一注买不成时面板长什么样」也是没做完
        warn!("NM_LOTTERY={number} 不在 1..={numbers} 的号码池里，这一注不买");
    } else {
        // 走真消息：柜台前那个人买成这一注，判据与扣钱都由 `lottery_op_system` 办
        desk.requests.write(BuyTicketRequested { player, number });
        info!("注入：{player:?} 站在彩票柜台前，买下一注 {number} 号");
    }
}

// ===== P13：魔法屋那两步 =====

/// `NM_MAGIC` 的那个值。`menu` = 只把第一步那一屏摆出来（11 条条件各自筛出谁），
/// 其余是**一条条件**——于是第二步那一屏（12 道法术）开出来。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum MagicWanted {
    Menu,
    Filter(game_core::prelude::MagicFilter),
}

impl MagicWanted {
    /// 读法写成纯函数（与 [`LotteryWanted::parse`] 同一条理由：环境变量本身没法在测试里摆）。
    /// 认不出来时由调用方 `warn!` 并列出可写的值 —— 猜一个条件是注入工具最坏的错误。
    fn parse(raw: &str) -> Option<Self> {
        use game_core::prelude::MagicFilter as F;
        // 大小写先抹平再比：`NM_MAGIC=GIRLS` 与 `NM_MAGIC=girls` 该是同一个旋钮，
        // 而认不出来时工具只会「注入没发生」，截图上看不出任何异常（`nm_god_*` 踩过的那格）
        let raw = raw.trim().to_ascii_lowercase();
        if raw == "menu" || raw.is_empty() {
            return Some(Self::Menu);
        }
        Some(Self::Filter(match raw.as_str() {
            "walker" | "走路" => F::Walker,
            "deposit" | "存款" => F::TopDeposit,
            "girls" | "女生" => F::AllGirls,
            "boys" | "男生" => F::AllBoys,
            "bike" | "机车" => F::OnBike,
            "car" | "汽车" => F::OnCar,
            "points" | "点券" => F::TopPoints,
            "lands" | "土地" => F::TopLands,
            "cash" | "现金" => F::TopCash,
            "possessed" | "附身" => F::Possessed,
            "wealth" | "财产" => F::TopWealth,
            _ => return None,
        }))
    }
}

/// `NM_SPELL` 的那个值（第二步的那一道法术）。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct SpellWanted(game_core::prelude::MagicSpell);

impl SpellWanted {
    fn parse(raw: &str) -> Option<Self> {
        use game_core::prelude::MagicSpell as S;
        // 同 `MagicWanted::parse`：大小写先抹平，中文那半不受影响
        let raw = raw.trim().to_ascii_lowercase();
        Some(Self(match raw.as_str() {
            "card" | "得卡" => S::GainCard,
            "build" | "加盖" => S::BuildHere,
            "bank" | "存钱" => S::BankCash,
            "fate" | "命运" => S::DrawFate,
            "sellcards" | "卖卡" => S::SellCards,
            "sellitems" | "卖道具" => S::SellItems,
            "jail" | "坐牢" => S::Jail,
            "hospital" | "住院" => S::Hospital,
            "stay" | "停留" => S::Stay,
            "demolish" | "拆除" => S::Demolish,
            "turn" | "掉头" => S::TurnBack,
            "auction" | "拍卖" => S::AuctionHere,
            _ => return None,
        }))
    }
}

/// `NM_MAGIC=<menu|条件>`：把当前玩家摆到女巫那里。`menu` 停在第一步，
/// 给一条条件就替他答第一步（截图拍的是**第二步**那一屏：条件 + 名单 + 十二道法术）。
///
/// **别配 `NM_AUTOROLL`**（坑 37 的形状，实测两种结果）：注入与自动玩家在同一帧各写一次
/// `NextState`（一个 `WaitDecision`、一个 `Moving`），谁赢由调度次序决定 —— 而次序会变
/// （`NM_DUMP` 加不加就足以翻面）。输的那一版拍到的是「他滚完路、踩到别人家地、买地模态
/// 把女巫的窗口顶掉了」，看着像旋钮失效。正经配方是 `NM_SCENE=board` 单独跑：回合机自己
/// 会停在 `WaitRoll`，那一格没有人滚，窗口就一直挂着。
#[derive(Resource)]
struct ForceMagic(Option<MagicWanted>);

impl ForceMagic {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_MAGIC") else {
            return Self(None);
        };
        match MagicWanted::parse(&raw) {
            Some(wanted) => Self(Some(wanted)),
            None => {
                warn!(
                    "NM_MAGIC={raw} 不认识，应为 menu 或 \
                     walker|deposit|girls|boys|bike|car|points|lands|cash|possessed|wealth"
                );
                Self(None)
            }
        }
    }
}

/// `NM_SPELL=<法术>`：第二步窗口挂着时替他点那道法术。
///
/// 为什么是**第二条**旋钮而不是 `NM_MAGIC=girls:jail` 一条：两步之间隔着一次「筛人」，
/// 名单由规则层算完才开得出第二步窗口，一条消息说不完那个中间状态（与真玩家的两次点击
/// 同一形状）。它不认 `WaitRoll` 那道门——发出去的那一刻相位本来就是 `WaitDecision`。
#[derive(Resource)]
struct ForceSpell(Option<SpellWanted>);

impl ForceSpell {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_SPELL") else {
            return Self(None);
        };
        match SpellWanted::parse(&raw) {
            Some(wanted) => Self(Some(wanted)),
            None => {
                warn!(
                    "NM_SPELL={raw} 不认识，应为 card|build|bank|fate|sellcards|sellitems|\
                     jail|hospital|stay|demolish|turn|auction"
                );
                Self(None)
            }
        }
    }
}

/// 两步各自要写的消息 + 要读的窗口。打包（§六 坑 8）。
#[derive(SystemParam)]
struct MagicInject<'w> {
    order: Res<'w, TurnOrder>,
    pending: ResMut<'w, PendingDecision>,
    filters: MessageWriter<'w, game_core::prelude::MagicFilterPicked>,
    spells: MessageWriter<'w, game_core::prelude::MagicSpellPicked>,
    done: ResMut<'w, Injected>,
}

fn force_magic(
    force: Res<ForceMagic>,
    mut desk: MagicInject<'_>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let (Some(wanted), false) = (force.0, desk.done.magic) else {
        return;
    };
    let Some(player) = desk.order.current() else {
        return;
    };
    desk.done.magic = true;
    desk.pending.offer(player, Pending::Magic);
    next.set(TurnPhase::WaitDecision);
    if let MagicWanted::Filter(filter) = wanted {
        // 走真消息：筛谁、开不开得出第二步，全由 `magic_filter_system` 判（空名单那条
        // 直接收摊 —— 注入替玩家答一条筛不出人的条件，拍到的就该是「女巫什么都没说」）
        desk.filters
            .write(game_core::prelude::MagicFilterPicked { player, filter });
        info!(
            "注入：{player:?} 站在魔法屋里，条件挑了「{}」",
            filter.label()
        );
    } else {
        info!("注入：{player:?} 站在魔法屋里，停在第一步");
    }
}

fn force_spell(force: Res<ForceSpell>, mut desk: MagicInject<'_>) {
    let (Some(wanted), false) = (force.0, desk.done.spell) else {
        return;
    };
    // 主语与条件都取自窗口：第一步还没答完时这里什么都没有，等下一帧
    let Some((player, Pending::MagicSpell(_))) = desk.pending.active() else {
        return;
    };
    desk.done.spell = true;
    desk.spells.write(game_core::prelude::MagicSpellPicked {
        player,
        spell: wanted.0,
    });
    info!("注入：{player:?} 求了一道「{}」", wanted.0.label());
}

/// `NM_AUCTION=<地块号>`：把那块地挂上拍卖台。`None` = 没开。
///
/// 它写的不是窗口而是**那张愿望单**（`Auction::requested`）—— 与拍卖卡落下的那一槌同一格。
/// 开场、排队、把窗口挂到「此刻该出价的那一位」身上，全部由规则层那两条 system 自己做，
/// 于是这个旋钮走的是真路径：面板与 `NM_AUTOROLL` 跑出来的那一屏是同一块面板。
///
/// 为什么非要有它：拍卖的正经入口有两个（拍卖卡在手 + 有人破产），一个要抽到那张卡，
/// 一个要输掉一局 —— 两者都押不住截图时刻（同 `NM_DETAIN` 那一条理由，第四次付学费）。
///
/// 开槌的人取**座位序里第一个不是那块地业主的玩家**：使用者不参与拍卖（10-22 拍板），
/// 而这样一轮到的第一位正好是当前回合玩家（`NM_PLOTS` 把地划给他），鼠标冒烟就点得动那颗
/// 「加到 N」。反过来若让当前玩家开槌，他站在自己的地前面，那条 `OwnLand` 会当场把这一槌挡掉。
#[derive(Resource)]
struct ForceAuction(Option<u32>);

impl ForceAuction {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_AUCTION") else {
            return Self(None);
        };
        match raw.trim().parse::<u32>() {
            Ok(land) => Self(Some(land)),
            Err(_) => {
                warn!("NM_AUCTION={raw} 不认识，应为地块编号（prototype 是 0..=5）");
                Self(None)
            }
        }
    }
}

/// 那一槌要写的两样 + 要读的三样。`Books` 刻意**不**进来：它怀里揣着一条
/// `ResMut<Auction>`，与这里的 `auction` 参数会在参数建立期撞成同资源两条可变借用
/// （`auction::AuctionDesk` 上面那段纪律，本旋钮正是那句话的第二个用户）。
#[derive(SystemParam)]
struct HammerWriters<'w, 's> {
    active: Res<'w, ActiveBoard>,
    order: Res<'w, TurnOrder>,
    owners: Query<'w, 's, &'static Owner>,
    identities: Query<'w, 's, &'static PlayerId>,
    auction: ResMut<'w, Auction>,
    done: ResMut<'w, Injected>,
}
fn force_auction(force: Res<ForceAuction>, mut desk: HammerWriters<'_, '_>) {
    let (Some(land), false) = (force.0, desk.done.auction) else {
        return;
    };
    let id = LandId(land);
    // 注入名额**一次就消耗**，无论这一槌落不落得下：旋钮值是启动时定的，落不下的话
    // 每一帧都重试就变成每一帧一句 `warn!`（长跑日志被刷满那条，同 `NM_LOTTERY` 的号码池）
    desk.done.auction = true;
    let Some(&plot) = desk.active.lands.get(&id) else {
        warn!("NM_AUCTION={land} 这块地不在这一局的盘上");
        return;
    };
    let owner = desk.owners.get(plot).ok().and_then(|held| held.0);
    let Some(owner) = owner else {
        warn!("NM_AUCTION={land} 这块地还没有主人 —— 拍卖开不了场（配 NM_PLOTS=<n> 先划几块地）");
        return;
    };
    let Some(founder) = desk
        .order
        .order
        .iter()
        .find(|e| desk.identities.get(**e).ok().copied() != Some(owner))
        .copied()
    else {
        warn!("NM_AUCTION={land} 场上没有不是这块地主人的玩家，没人能敲这一槌");
        return;
    };
    desk.auction.requested = Some(Listing {
        land: id,
        gavel: Gavel::Card(founder),
    });
    info!("注入：地块 {land} 挂上拍卖台，开槌的是 {founder:?}（他不参与竞价）");
}

/// `NM_HOUSE=<地块号>:<层数>`：给那块地盖上指定层数，**不动归属、不动钱**。
///
/// 为什么要有它（10-22 实报之后新增的那种状态）：「无主而立着楼」现在是真状态 ——
/// 破产剩下的那些地就是这一种，而它的两个来源都押不住截图时刻：破产要输掉一局，
/// 天使那几层要连踩几次同一格且都由骰子说了算。买地模态上那个「买价」（地价 + 已盖的每一层）
/// 是这一阶段唯一要肉眼核对的报价，拍不到就等于没验（`NM_DETAIN` 那条理由，第五次付学费）。
///
/// 它写的是**世界状态**而不是窗口，且**就地写而不是 `Commands`**：延迟到下一次同步点才落地，
/// 而 `sync_modal` 可能就在本帧把面板建好 —— 那会拍出一张写着旧价的图（签名里带层数那一格
/// 已经补上了这道防线，但注入自己也不该晚一帧）。与 `NM_PLOTS` 同一条纪律：这是前置，
/// 不是一笔交易，所以它不发 `LandBought`、不动现金，硬套交易入口会让现金审计多出一笔
/// 没花出去的钱。
#[derive(Resource)]
struct ForceHouse(Option<(u32, u8)>);

impl ForceHouse {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_HOUSE") else {
            return Self(None);
        };
        match Self::parse(&raw) {
            Some(land) => Self(Some(land)),
            None => {
                warn!("NM_HOUSE={raw} 不认识，应为 地块号:层数（如 3:3）");
                Self(None)
            }
        }
    }

    /// 拆成纯函数是为了能被测到（与 [`ForceGod::parse`]／[`LotteryWanted::parse`] 同一条理由）：
    /// 一个拼错的旋钮值不崩也不红，只会「注入没发生」，而那张截图看着像「今天没有楼」。
    fn parse(raw: &str) -> Option<(u32, u8)> {
        let (land, level) = raw.trim().split_once(':')?;
        Some((
            land.trim().parse::<u32>().ok()?,
            level.trim().parse::<u8>().ok()?,
        ))
    }
}

fn force_house(
    force: Res<ForceHouse>,
    active: Res<ActiveBoard>,
    mut plots: Query<&mut BuildingLevel, With<LandId>>,
    mut done: ResMut<Injected>,
) {
    let (Some((land, level)), false) = (force.0, done.house) else {
        return;
    };
    // 名额一次就消耗，无论这块地在不在盘上：旋钮值是启动时定的，落不下还每帧重试就成了
    // 每帧一句 `warn!`（长跑日志被刷满那一条，同 `NM_AUCTION`／`NM_LOTTERY`）
    done.house = true;
    let Some(&plot) = active.lands.get(&LandId(land)) else {
        warn!("NM_HOUSE={land} 这块地不在这一局的盘上");
        return;
    };
    let Ok(mut now) = plots.get_mut(plot) else {
        warn!("NM_HOUSE={land} 这块地没有建筑等级组件，什么都没注入");
        return;
    };
    now.0 = level;
    info!("注入：地块 {land} 现在是 Lv{level}（归属与钱都没动）");
}

/// `NM_RBUILD=1`：把远程建设窗口挂到当前玩家面前。`false` = 没开。
///
/// 与 `NM_MINIGAME` 同一条处理：写的是**规则层那个窗口**（`Pending::RemoteBuild`），
/// 不是 UI 侧的开关。发 [`RemoteBuildOffered`] 是走真路径 —— 它是踩到日产建设格
/// 的那条凭据，冒烟截图里的窗口与真玩出来的那一块必须是同一块。
///
/// 为什么非要有它：这个窗口的正经入口是踩 4 号建设格，而那一步押不住截图时刻
/// （`NM_DETAIN` 那条理由的第六次学费）；配 `NM_PLOTS=<n>` 才有「他名下的地」可升。
#[derive(Resource)]
struct ForceRbuild(bool);

impl ForceRbuild {
    fn from_env() -> Self {
        Self(std::env::var("NM_RBUILD").as_deref() == Ok("1"))
    }
}

/// `NM_NEWS=<新闻 id 或池子里的序号>`：立刻播那一条新闻。`None` = 没开。
///
/// 它调的是 core 自己导出的 [`fire_news`]，与 `NM_DETAIN` 调 [`detain`] 同一条分工：
/// **不推进回合、不替随机流取数**。刻意不写「强制下一条」那种位（`NM_LOTTERY` 也不这么干）：
/// 那等于给规则层加一条生产环境永远不读的分支。
///
/// 为什么非要有它：抽哪一条是随机流上的事，而**每一天的新闻效果长得都不一样**（天灾拆街、
/// 全场课税、点名罚款）。要让「外星人攻打地球」那一屏拍得出来，靠 `NM_SEED` 得连试很多颗种子；
/// 而 P11 的验收①（触发者金额不变、被点名者金额变）与验收②（全场每个人都留流水）
/// 各自钉的正是**特定那一条**。进不了截图的机制等于没做完（本模块头那条老账）。
#[derive(Resource)]
struct ForceNews(Option<String>);

impl ForceNews {
    fn from_env() -> Self {
        let Ok(raw) = std::env::var("NM_NEWS") else {
            return Self(None);
        };
        Self(Self::parse(&raw))
    }

    /// 纯函数那半：只认「去掉空白之后非空」这一条。写错 id 不在这里响，
    /// 要到池子那一侧才知道（`find_id`），因为合法集合是数据给的而不是代码给的。
    fn parse(raw: &str) -> Option<String> {
        let wanted = raw.trim();
        (!wanted.is_empty()).then(|| wanted.to_string())
    }
}

fn force_news(
    order: Res<TurnOrder>,
    mut sink: NewsSink,
    force: Res<ForceNews>,
    mut done: ResMut<Injected>,
) {
    let (Some(wanted), false) = (force.0.as_deref(), done.news) else {
        return;
    };
    let Some(player) = order.current() else {
        return;
    };
    done.news = true;
    let Some(set) = sink.pool() else {
        warn!("NM_NEWS={wanted} 没能播：新闻池还没就位");
        return;
    };
    let Some(id) = find_id(set, wanted) else {
        warn!(
            "NM_NEWS={wanted} 不在池子里（写新闻 id 如 news_001，或序号 0..{}）",
            set.0.len().saturating_sub(1)
        );
        return;
    };
    fire_news(&mut sink, player, id);
}

fn force_rbuild(
    force: Res<ForceRbuild>,
    order: Res<TurnOrder>,
    mut pending: ResMut<PendingDecision>,
    mut offered: MessageWriter<RemoteBuildOffered>,
    mut done: ResMut<Injected>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let (true, false) = (force.0, done.rbuild) else {
        return;
    };
    let Some(player) = order.current() else {
        return;
    };
    done.rbuild = true;
    offered.write(RemoteBuildOffered { player });
    pending.offer(player, Pending::RemoteBuild);
    // `WaitDecision` 是那个窗口在规则里的真实阶段（与 `force_minigame` 同一条）
    next.set(TurnPhase::WaitDecision);
    info!("注入：当前玩家面前挂出远程建设窗口");
}

#[cfg(test)]
mod tests {
    use super::{ForceGod, ForceNews};
    use game_core::prelude::{GameKind, God};

    /// `NM_NEWS` 只认「去掉空白非空」这一条：`NM_NEWS=` 那种给法要的是**不注入**，
    /// 而不是一句「不在池子里」的 warn —— 空串是「我没填」，填错才是「我填错了」
    #[test]
    fn nm_news_takes_a_name_and_nothing_else() {
        assert_eq!(ForceNews::parse("news_002"), Some("news_002".to_string()));
        assert_eq!(ForceNews::parse(" 1 "), Some("1".to_string()));
        assert_eq!(ForceNews::parse(""), None);
        assert_eq!(ForceNews::parse("   "), None);
    }

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

    /// `NM_HOUSE` 的那个串要认「地块号:层数」，而拼错时必须被说不认识而不是静默什么都不做：
    /// 这个旋钮存在的意义就是「拍得到那块带楼的无主地」，静默失败会让那张图看着像
    /// 「今天没有楼」，而同一条坑在 `nm_god_...`／`nm_minigame_...` 上各付过一次学费。
    /// 层数 0 是**合法**的字面（拆平那块地，拍 Lv0 的对照图用），挡的是读不出来的那一类。
    #[test]
    fn nm_house_takes_a_plot_and_a_storey_count() {
        use super::ForceHouse;
        assert_eq!(ForceHouse::parse("3:3"), Some((3, 3)));
        assert_eq!(
            ForceHouse::parse(" 5 : 1 "),
            Some((5, 1)),
            "环境变量里带空格是常事"
        );
        assert_eq!(ForceHouse::parse("0:0"), Some((0, 0)), "拆平也是一种注入");
        assert_eq!(ForceHouse::parse("3"), None, "少了层数就不知道该盖几层");
        assert_eq!(ForceHouse::parse("3:"), None);
        assert_eq!(ForceHouse::parse("西单:3"), None, "地块号不认名字");
        assert_eq!(ForceHouse::parse("3:三层"), None);
        assert_eq!(ForceHouse::parse(""), None);
    }

    /// `NM_LOTTERY` 两种写法都要认：点名的号，与 `auto`（买「这一局第一次会开出的那个号」）。
    /// `auto` 是冒烟脚本唯一用得上的那种 —— 开奖号由种子决定，而没人能在 shell 里跑
    /// ChaCha8 把它算出来。
    #[test]
    fn nm_lottery_takes_a_number_or_auto() {
        use super::LotteryWanted;
        assert_eq!(LotteryWanted::parse("7"), Some(LotteryWanted::Number(7)));
        assert_eq!(
            LotteryWanted::parse(" auto "),
            Some(LotteryWanted::Auto),
            "环境变量里带空格是常事"
        );
        assert_eq!(LotteryWanted::parse("0"), Some(LotteryWanted::Number(0)));
        // 0 与超出号码池的那一种是**合法的字面**、非法的下注 —— 挡它的是 `force_lottery`
        // 里那句 warn，不是解析层（解析层管不了规则里的 `lottery_numbers`）
        assert_eq!(LotteryWanted::parse("99"), Some(LotteryWanted::Number(99)));
        assert_eq!(LotteryWanted::parse(""), None);
        assert_eq!(LotteryWanted::parse("256"), None, "u8 装不下就该说不认识");
    }

    /// `NM_MAGIC` 认的十一个条件必须真是 `MagicFilter::ALL` 那十一格，一格不漏。
    ///
    /// 这一条的价值在后半句：拼错的旋钮值不崩也不红，只会「注入没发生」，而那张截图
    /// 看起来只是「今天没人踩到魔法屋」—— 与 `nm_god_...`／`nm_minigame_...` 同一种坑，
    /// 而那两次各付过学费。用 `ALL` 的条数反证「表与穷尽表同源」，加第十二条条件时这条会红。
    #[test]
    fn nm_magic_takes_every_filter_and_menu() {
        use super::MagicWanted;
        use game_core::prelude::MagicFilter as F;
        for (raw, want) in [
            ("walker", F::Walker),
            ("deposit", F::TopDeposit),
            ("girls", F::AllGirls),
            ("boys", F::AllBoys),
            ("bike", F::OnBike),
            ("car", F::OnCar),
            ("points", F::TopPoints),
            ("lands", F::TopLands),
            ("cash", F::TopCash),
            ("possessed", F::Possessed),
            ("wealth", F::TopWealth),
        ] {
            assert_eq!(
                MagicWanted::parse(raw),
                Some(MagicWanted::Filter(want)),
                "旋钮值 {raw}"
            );
        }
        assert_eq!(
            MagicWanted::parse("GIRLS"),
            Some(MagicWanted::Filter(F::AllGirls)),
            "大小写不分是环境变量的常态"
        );
        assert_eq!(MagicWanted::parse("menu"), Some(MagicWanted::Menu));
        assert_eq!(
            MagicWanted::parse(""),
            Some(MagicWanted::Menu),
            "只写 `NM_MAGIC=` 的意思是摆第一步"
        );
        assert_eq!(
            MagicWanted::parse("rich"),
            None,
            "认不出来要说「不认识」，不许猜一条条件"
        );
    }

    /// `NM_SPELL` 认的十二道法术必须真是 `MagicSpell::ALL` 那十二格。同上一条的理由。
    #[test]
    fn nm_spell_takes_every_spell() {
        use super::SpellWanted;
        use game_core::prelude::MagicSpell as S;
        for (raw, want) in [
            ("card", S::GainCard),
            ("build", S::BuildHere),
            ("bank", S::BankCash),
            ("fate", S::DrawFate),
            ("sellcards", S::SellCards),
            ("sellitems", S::SellItems),
            ("jail", S::Jail),
            ("hospital", S::Hospital),
            ("stay", S::Stay),
            ("demolish", S::Demolish),
            ("turn", S::TurnBack),
            ("auction", S::AuctionHere),
        ] {
            assert_eq!(
                SpellWanted::parse(raw).map(|w| w.0),
                Some(want),
                "旋钮值 {raw}"
            );
        }
        assert_eq!(SpellWanted::parse("wish"), None);
        assert_eq!(SpellWanted::parse(""), None, "空值不是「随便挑一道」");
    }
}
