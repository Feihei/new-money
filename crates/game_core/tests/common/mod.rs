//! 集成测试共用脚手架：真资产 + 真 `AssetServer` 的无窗口 App。
//!
//! 每个测试文件只用到其中一部分，故整体标 `dead_code` 允许。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::app::TaskPoolPlugin;
use bevy::asset::AssetPlugin;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::CoreGamePlugin;
use game_core::prelude::{
    ActiveBoard, DataHandles, DiceRolled, GameOver, GameRules, GameSeed, GameSession, GameSetup,
    JailSkipped, PlayerArrived, PlayerId, PlayerReleased, RequestRoll, TileStepped, TokenPosition,
    TurnEnded, TurnOrder, TurnPhase, TurnStarted, WalkStarted, Wallet,
};

pub fn asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .canonicalize()
        .expect("资产根应存在")
}

/// 测试侧的「点击掷骰按钮」：`PreUpdate` 把它排空成 `RequestRoll` 消息。
/// 不直接塞消息是为了让测试走与真实 UI 完全相同的入口。
#[derive(Resource, Default)]
pub struct Inputs(pub Vec<Entity>);

/// 按到达顺序记录的消息流水。跨类型的相对顺序只在「同一帧内多条消息」时有歧义
/// （消息本身是帧末批量投递的），而断言用的序列都留了一帧余量。
#[derive(Resource, Default, Debug)]
pub struct Observed(pub Vec<String>);

/// `MessageReader` 打包：九个 reader 参数比给系统挂 `#[allow]` 好读。
/// （0.20 的 reader 带 `'w`/`'s` 两个生命周期，与 Bevy 自带的 `EventReader` 同形）
///
/// 字段顺序 = `collect` 的记录顺序 = 同帧多条消息在 `Observed` 里的相对顺序，
/// 按「回合开始 → 出狱/服刑 → 掷骰 → 行走 → 到站 → 回合结束」的因果排列。
#[derive(SystemParam)]
struct AllReaders<'w, 's> {
    turn_started: MessageReader<'w, 's, TurnStarted>,
    released: MessageReader<'w, 's, PlayerReleased>,
    jail_skipped: MessageReader<'w, 's, JailSkipped>,
    dice_rolled: MessageReader<'w, 's, DiceRolled>,
    walk_started: MessageReader<'w, 's, WalkStarted>,
    tile_stepped: MessageReader<'w, 's, TileStepped>,
    player_arrived: MessageReader<'w, 's, PlayerArrived>,
    turn_ended: MessageReader<'w, 's, TurnEnded>,
    game_over: MessageReader<'w, 's, GameOver>,
}

fn who(players: &Query<&PlayerId>, e: Entity) -> String {
    match players.get(e) {
        Ok(id) => format!("P{}", id.0),
        Err(_) => "?".into(),
    }
}

fn collect(mut log: ResMut<Observed>, mut r: AllReaders, players: Query<&PlayerId>) {
    for m in r.turn_started.read() {
        log.0
            .push(format!("TurnStarted({})", who(&players, m.player)));
    }
    for m in r.released.read() {
        log.0.push(format!(
            "Released({}@{})",
            who(&players, m.player),
            m.junction.0
        ));
    }
    for m in r.jail_skipped.read() {
        log.0.push(format!(
            "JailSkip({},剩{})",
            who(&players, m.player),
            m.turns_left
        ));
    }
    for m in r.dice_rolled.read() {
        log.0.push(format!(
            "Roll({},{},{}->{})",
            who(&players, m.player),
            m.steps,
            m.from.0,
            m.to.0
        ));
    }
    for m in r.walk_started.read() {
        log.0.push(format!(
            "Walk({},{},{}->{})",
            who(&players, m.player),
            m.steps,
            m.from.0,
            m.to.0
        ));
    }
    for m in r.tile_stepped.read() {
        log.0
            .push(format!("Step({},{})", who(&players, m.player), m.tile.0));
    }
    for m in r.player_arrived.read() {
        log.0.push(format!(
            "Arrive({},{}->{})",
            who(&players, m.player),
            m.from.0,
            m.to.0
        ));
    }
    for m in r.turn_ended.read() {
        log.0.push(format!("TurnEnd({})", who(&players, m.player)));
    }
    for m in r.game_over.read() {
        let winner = match m.winner {
            Some(w) => who(&players, w),
            None => "none".into(),
        };
        log.0.push(format!("GameOver({winner})"));
    }
}

fn pump_inputs(mut inputs: ResMut<Inputs>, mut rolls: MessageWriter<RequestRoll>) {
    for player in inputs.0.drain(..) {
        rolls.write(RequestRoll { player });
    }
}

pub fn boot(setup: GameSetup, rules: GameRules, seed: GameSeed) -> App {
    let mut app = App::new();
    // 任务池（尤其 IoTaskPool）必须由测试自己初始化：AssetServer 的读盘跑在它上面
    app.add_plugins(TaskPoolPlugin::default())
        .add_plugins(AssetPlugin {
            file_path: asset_root().display().to_string(),
            watch_for_changes_override: Some(false),
            ..default()
        })
        .add_plugins(CoreGamePlugin)
        .insert_resource(setup)
        .insert_resource(rules)
        .insert_resource(seed)
        .init_resource::<Inputs>()
        .init_resource::<Observed>()
        .add_systems(
            Startup,
            |mut commands: Commands, server: Res<AssetServer>| {
                commands.insert_resource(DataHandles::load(&server));
            },
        )
        .add_systems(PreUpdate, pump_inputs)
        .add_systems(PostUpdate, collect);
    app
}

pub fn boot_default() -> App {
    // 无头 App 没有真实时间流逝，行走时钟永远推不动 —— 测试一律用瞬时落格
    // （interval=0 与 B2b 行为等价，这正是 B2c 的验收项之一）
    boot(
        GameSetup::default(),
        GameRules::instant(),
        GameSeed::default(),
    )
}

/// 步进直到世界装配完成（资产加载是异步的）。
pub fn run_until_world(app: &mut App) -> bool {
    for _ in 0..500 {
        app.update();
        if app.world().contains_resource::<ActiveBoard>() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    false
}

/// 走到 `WaitRoll` 并确认世界已装配 —— 绝大多数用例的前置。
///
/// 末尾多跑两帧：消息是帧末批量投递的，`WaitRoll` 当帧写的 `TurnStarted` 要下一帧才读得到。
pub fn boot_to_wait_roll(app: &mut App) -> Entity {
    assert!(run_until_world(app), "棋盘资产没能在时限内装配成世界");
    assert!(
        run_until_phase(app, TurnPhase::WaitRoll, 30),
        "状态机没能进入 WaitRoll，实际停在 {:?}",
        phase(app)
    );
    settle(app, 2);
    current(app)
}

/// 空跑几帧，让已写入的消息进入流水。
pub fn settle(app: &mut App, frames: usize) {
    for _ in 0..frames {
        app.update();
    }
}

/// 自动打完 `want` 个回合：每次进入 `WaitRoll` 就替当前玩家按一次掷骰。
///
/// `pending` 是必需的 —— 从按下到真正掷出要跨几帧（消息帧末投递），期间状态仍是
/// `WaitRoll`，不加守卫会一次排进好几个请求。同理不能拿 `run_until_phase(WaitRoll)`
/// 当「本回合结束」的判据：请求发出后阶段还没动，会立刻返回真。
///
/// 结尾固定多跑 6 帧：状态机每步恰好占一帧，跑满就能把最后一批消息刷进流水，
/// 于是断言看到的序列是确定的。
pub fn play_turns(app: &mut App, want: u32) {
    let mut pending = false;
    for _ in 0..(want as usize + 5) * 60 {
        if app.world().resource::<GameSession>().turn_count >= want {
            break;
        }
        match phase(app) {
            TurnPhase::WaitRoll if !pending => {
                let cur = current(app);
                roll(app, cur);
                pending = true;
            }
            TurnPhase::Over => break,
            _ => pending = false,
        }
        app.update();
    }
    settle(app, 6);
}

pub fn phase(app: &App) -> TurnPhase {
    *app.world().resource::<State<TurnPhase>>().get()
}

/// 步进直到状态机进入 `want`，超过 `frames` 帧则返回 false。
pub fn run_until_phase(app: &mut App, want: TurnPhase, frames: usize) -> bool {
    for _ in 0..frames {
        if phase(app) == want {
            return true;
        }
        app.update();
    }
    phase(app) == want
}

pub fn events(app: &App) -> Vec<String> {
    app.world().resource::<Observed>().0.clone()
}

/// 按 `PlayerId` 排序的 (实体, 编号, 现金, 所在格)。
pub fn roster(app: &mut App) -> Vec<(Entity, u32, i64, u32)> {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &PlayerId, &Wallet, &TokenPosition)>();
    let mut rows: Vec<(Entity, u32, i64, u32)> = q
        .iter(w)
        .map(|(e, id, cash, pos)| (e, id.0, cash.cash, pos.tile.0))
        .collect();
    rows.sort_by_key(|(_, id, _, _)| *id);
    rows
}

pub fn current(app: &App) -> Entity {
    app.world()
        .resource::<TurnOrder>()
        .current()
        .expect("有当前玩家")
}

/// 替 `player` 按下掷骰按钮。
pub fn roll(app: &mut App, player: Entity) {
    app.world_mut().resource_mut::<Inputs>().0.push(player);
}

pub fn tile_of(app: &App, player: Entity) -> u32 {
    app.world()
        .entity(player)
        .get::<TokenPosition>()
        .expect("玩家应有位置")
        .tile
        .0
}

pub fn insert<P: bevy::prelude::Bundle>(app: &mut App, player: Entity, bundle: P) {
    app.world_mut().entity_mut(player).insert(bundle);
}
