//! 集成测试共用脚手架：真资产 + 真 `AssetServer` 的无窗口 App。
//!
//! 每个测试文件只用到其中一部分，故整体标 `dead_code` 允许。
//!
//! B3 起对局会停在 `WaitDecision` 等玩家回答（买地/升级/收据/卡面），而无头测试没有 UI，
//! 所以这里放了一个「代替 B5 对话框」的自动应答 system（[`DecisionPolicy`]）。
//! 它走的是与真实玩家完全相同的入口 —— `DecisionChosen` 消息，不是直接改组件。

#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use bevy::app::TaskPoolPlugin;
use bevy::asset::AssetPlugin;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::CoreGamePlugin;
use game_core::prelude::{
    ActiveBoard, AtmAccess, BankBalance, BankOp, BankOpDone, BankOpRequested, BankVisited, Books,
    BuildingLevel, BuyOffered, CardDrawn, CardId, CardKind, CashChanged, DataHandles, Decision,
    DecisionChosen, Deck, DiceRolled, GameOver, GameRules, GameSeed, GameSession, GameSetup,
    JailSkipped, LandBought, LandId, LandUpgraded, Market, MarketClock, Mortgaged, Owner, Pending,
    PendingDecision, PlayerArrived, PlayerBankrupt, PlayerId, PlayerJailed, PlayerReleased,
    PlayerTeleported, RentDue, RequestRoll, Rng, StockId, StockPriceChanged, StockTradeRequested,
    StockTraded, StockVisited, TileIndex, TileStepped, TokenPosition, TradeSide, TurnEnded,
    TurnOrder, TurnPhase, TurnStarted, UpgradeOffered, WalkStarted, Wallet,
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

/// 测试侧的「点证券面板的买/卖按钮」。同理经一条泵 system 转成消息，
/// 于是「阶段不对就不受理」这条门禁被真的走了一遍，而不是被测试绕过。
#[derive(Resource, Default)]
pub struct Trades(pub Vec<StockTradeRequested>);

/// 测试侧的「点柜台/ATM 的四个按钮」。
#[derive(Resource, Default)]
pub struct BankRequests(pub Vec<BankOpRequested>);

/// 自动应答策略。默认 `DeclineAll`：不买不升，于是整局不产生归属、不收租金，
/// B2 那批「整条回合流水逐字比对」的断言才不会因为经济上线而改变语义。
/// 要测经济就换成 `AcceptAll`（买得起就买、能升就升）。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum DecisionPolicy {
    /// 默认值：不买不升，于是整局不产生归属、不收租金，
    /// B2 那批「整条回合流水逐字比对」的断言不会因为经济上线而改变语义。
    #[default]
    DeclineAll,
    AcceptAll,
    /// 什么都不答，回合停在 `WaitDecision`。要看「窗口开着的时候能办什么」的用例用它 ——
    /// 柜台与买地报价都是先看再答，自动答掉就观察不到了。
    Hold,
}

/// 代替 B5 的对话框：挂着待决事项就按策略答一次。
///
/// 只在 `WaitDecision` 里跑，且只答 `PendingDecision` 里那位 —— 与真实 UI 的权限一样。
fn auto_answer_system(
    policy: Res<DecisionPolicy>,
    pending: Res<PendingDecision>,
    mut answers: MessageWriter<DecisionChosen>,
) {
    if *policy == DecisionPolicy::Hold {
        return;
    }
    let Some((player, about)) = pending.active() else {
        return;
    };
    let action = match (about, *policy) {
        (Pending::Buy(_) | Pending::Upgrade(_), DecisionPolicy::DeclineAll) => Decision::Decline,
        (Pending::Buy(_) | Pending::Upgrade(_), DecisionPolicy::AcceptAll) => Decision::Accept,
        // 收据与卡面无论如何都算「看完了」
        (Pending::RentNotice(_) | Pending::Card, _) => Decision::Acknowledge,
        // 柜台只认「办完」。这里必须显式答一句，否则每一个踩到银行格的回合都会
        // 停在 `WaitDecision`，把所有长跑用例整片挂死 —— 编译器逼这一条写出来。
        (Pending::BankTeller, _) => Decision::Acknowledge,
        // `Hold` 在函数开头就返回了，这一支只为穷尽存在
        (Pending::Buy(_) | Pending::Upgrade(_), DecisionPolicy::Hold) => return,
    };
    answers.write(DecisionChosen { player, action });
}

/// 按到达顺序记录的消息流水。跨类型的相对顺序只在「同一帧内多条消息」时有歧义
/// （消息本身是帧末批量投递的），而断言用的序列都留了一帧余量。
#[derive(Resource, Default, Debug)]
pub struct Observed(pub Vec<String>);

/// 经济/卡牌侧的消息流水，与 `Observed` 分开记：
/// 回合流水的逐字断言属于 B2，经济断言属于 B3，混在一张表里改一个就得改另一个。
#[derive(Resource, Default, Debug)]
pub struct Ledger(pub Vec<String>);

/// 现金流水（`(玩家, delta, 结算后余额)`）。B3 的审计断言用它：
/// `余额 == 起始现金 + Σ delta`，任何「改了钱包没留流水」都会在这里显形。
#[derive(Resource, Default, Debug)]
pub struct CashAudit(pub Vec<(Entity, i64, i64)>);

/// `MessageReader` 打包：流程域的 reader 们（0.20 的 reader 带 `'w`/`'s` 两个生命周期，见坑 13）。
///
/// 字段顺序 = `collect` 的记录顺序 = 同帧多条消息在 `Observed` 里的相对顺序，
/// 按「回合开始 → 出狱/服刑 → 掷骰 → 行走 → 到站 → 回合结束」的因果排列。
#[derive(SystemParam)]
struct FlowReaders<'w, 's> {
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

/// 经济/卡牌域的 reader 们。
#[derive(SystemParam)]
struct BookReaders<'w, 's> {
    offers: MessageReader<'w, 's, BuyOffered>,
    upgrade_offers: MessageReader<'w, 's, UpgradeOffered>,
    bought: MessageReader<'w, 's, LandBought>,
    upgraded: MessageReader<'w, 's, LandUpgraded>,
    rents: MessageReader<'w, 's, RentDue>,
    bankrupts: MessageReader<'w, 's, PlayerBankrupt>,
    jailed: MessageReader<'w, 's, PlayerJailed>,
    teleported: MessageReader<'w, 's, PlayerTeleported>,
    cards: MessageReader<'w, 's, CardDrawn>,
    bank: MessageReader<'w, 's, BankVisited>,
    bank_ops: MessageReader<'w, 's, BankOpDone>,
    stock_visit: MessageReader<'w, 's, StockVisited>,
    traded: MessageReader<'w, 's, StockTraded>,
    prices: MessageReader<'w, 's, StockPriceChanged>,
    cash: MessageReader<'w, 's, CashChanged>,
}

fn who(players: &Query<&PlayerId>, e: Entity) -> String {
    match players.get(e) {
        Ok(id) => format!("P{}", id.0),
        Err(_) => "?".into(),
    }
}

/// 地块显示成 `L3` 而不是 `LandId(3)`，流水读起来像人话。
fn land(id: LandId) -> String {
    format!("L{}", id.0)
}

/// 银行操作的中文短动词。流水要能被断言读出来，`Deposit(1000)` 那种一屏看不出方向。
fn op_verb(op: BankOp) -> &'static str {
    match op {
        BankOp::Deposit => "存",
        BankOp::Withdraw => "取",
        BankOp::Borrow => "借",
        BankOp::Repay => "还",
    }
}

fn side_verb(side: game_core::prelude::TradeSide) -> &'static str {
    match side {
        game_core::prelude::TradeSide::Buy => "买",
        game_core::prelude::TradeSide::Sell => "卖",
    }
}

fn collect_flow(mut log: ResMut<Observed>, mut r: FlowReaders, players: Query<&PlayerId>) {
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

fn collect_books(
    mut log: ResMut<Ledger>,
    mut audit: ResMut<CashAudit>,
    mut r: BookReaders,
    players: Query<&PlayerId>,
) {
    for m in r.offers.read() {
        log.0.push(format!(
            "BuyOffer({},@{})",
            who(&players, m.player),
            m.land.0
        ));
    }
    for m in r.upgrade_offers.read() {
        log.0.push(format!(
            "UpgradeOffer({},@{}${})",
            who(&players, m.player),
            m.land.0,
            m.cost
        ));
    }
    for m in r.bought.read() {
        log.0.push(format!(
            "Bought({}@{}${})",
            who(&players, m.player),
            m.land.0,
            m.price
        ));
    }
    for m in r.upgraded.read() {
        log.0.push(format!(
            "Upgraded({}@{}→{})",
            who(&players, m.player),
            m.land.0,
            m.new_level
        ));
    }
    for m in r.rents.read() {
        log.0.push(format!(
            "Rent({}->{}@L{}${})",
            who(&players, m.payer),
            who(&players, m.owner),
            m.land.0,
            m.amount
        ));
    }
    for m in r.bankrupts.read() {
        log.0.push(format!(
            "Bankrupt({}欠{})",
            who(&players, m.player),
            m.unpaid
        ));
    }
    for m in r.jailed.read() {
        log.0.push(format!(
            "Jailed({}@{})",
            who(&players, m.player),
            m.junction.0
        ));
    }
    for m in r.teleported.read() {
        log.0.push(format!(
            "Teleport({}→{})",
            who(&players, m.player),
            m.tile.0
        ));
    }
    for m in r.cards.read() {
        log.0
            .push(format!("Card({}:{})", who(&players, m.player), m.card.id));
    }
    for m in r.bank.read() {
        log.0.push(format!(
            "Bank({},存{}贷{})",
            who(&players, m.player),
            m.deposit,
            m.loan
        ));
    }
    for m in r.bank_ops.read() {
        log.0.push(format!(
            "BankOp({}{}{})",
            who(&players, m.player),
            op_verb(m.op),
            m.amount
        ));
    }
    for m in r.stock_visit.read() {
        log.0
            .push(format!("StockVisit({})", who(&players, m.player)));
    }
    for m in r.traded.read() {
        log.0.push(format!(
            "Trade({}{}{}{}股@{:.2})",
            who(&players, m.player),
            side_verb(m.side),
            m.stock.0,
            m.shares,
            m.price
        ));
    }
    for m in r.prices.read() {
        log.0
            .push(format!("Price({}:{:.2}→{:.2})", m.stock.0, m.old, m.new));
    }
    for m in r.cash.read() {
        audit.0.push((m.player, m.delta, m.balance));
    }
}

fn pump_inputs(
    mut inputs: ResMut<Inputs>,
    mut trades: ResMut<Trades>,
    mut asks: ResMut<BankRequests>,
    mut rolls: MessageWriter<RequestRoll>,
    mut trade_writes: MessageWriter<StockTradeRequested>,
    mut op_writes: MessageWriter<BankOpRequested>,
) {
    for player in inputs.0.drain(..) {
        rolls.write(RequestRoll { player });
    }
    for t in trades.0.drain(..) {
        trade_writes.write(t);
    }
    for a in asks.0.drain(..) {
        op_writes.write(a);
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
        .init_resource::<Trades>()
        .init_resource::<BankRequests>()
        .init_resource::<DecisionPolicy>()
        .init_resource::<Observed>()
        .init_resource::<Ledger>()
        .init_resource::<CashAudit>()
        .add_systems(
            Startup,
            |mut commands: Commands, server: Res<AssetServer>| {
                commands.insert_resource(DataHandles::load(&server));
            },
        )
        .add_systems(PreUpdate, pump_inputs)
        // 自动应答排在 `decision_system` 之前写入即可：消息下一帧才被读，顺序不敏感
        .add_systems(
            Update,
            auto_answer_system.run_if(in_state(TurnPhase::WaitDecision)),
        )
        .add_systems(PostUpdate, (collect_flow, collect_books));
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
/// B3 起回合中途可能停在 `WaitDecision`，那一段由 `DecisionPolicy` 自动应答，
/// 所以这里的判据仍是 `GameSession.turn_count`（见坑 12）。
///
/// 结尾固定多跑 6 帧：状态机每步恰好占一帧，跑满就能把最后一批消息刷进流水，
/// 于是断言看到的序列是确定的。
pub fn play_turns(app: &mut App, want: u32) {
    let mut pending = false;
    for _ in 0..(want as usize + 5) * 120 {
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

pub fn ledger(app: &App) -> Vec<String> {
    app.world().resource::<Ledger>().0.clone()
}

pub fn set_policy(app: &mut App, policy: DecisionPolicy) {
    app.insert_resource(policy);
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

/// `(地块编号, 业主 PlayerId, 等级)` 全表，按编号排序 —— 经济断言的主视图。
pub fn holdings(app: &mut App) -> Vec<(u32, Option<u32>, u8)> {
    let w = app.world_mut();
    let mut q = w.query::<(&LandId, &Owner, &BuildingLevel)>();
    let mut rows: Vec<(u32, Option<u32>, u8)> = q
        .iter(w)
        .map(|(id, owner, level)| (id.0, owner.0.map(|p| p.0), level.0))
        .collect();
    rows.sort();
    rows
}

pub fn cash_of(app: &mut App, player: Entity) -> i64 {
    app.world()
        .entity(player)
        .get::<Wallet>()
        .expect("玩家应有钱包")
        .cash
}

// ===== 直接驱动结算层 API =====
///
/// 存款/取款/借贷/买卖这些动作在 B3 还没有生产者（银行面板是 B5、AI 是 B6），
/// 但它们是 `Books`/`Market` 上的公开方法。测试用一次性 system 去调它们，
/// 走的就是将来 UI 要走的同一条路径 —— 而不是在测试里手搓世界访问绕过这层接口。
/// 一次结算层调用的形状。单独命名出来是因为把 `Box<dyn FnOnce(...)>` 直接写在字段上
/// 会撞 clippy 的 `type_complexity`，而这条签名本身就是有信息量的东西。
type Op = dyn FnOnce(&mut Books, &mut Market) + Send + Sync;

#[derive(Resource)]
pub struct Ops(Option<Box<Op>>);

fn run_op_system(mut ops: ResMut<Ops>, mut books: Books, mut market: Market) {
    if let Some(f) = ops.0.take() {
        f(&mut books, &mut market);
    }
}

pub fn run_op<F>(app: &mut App, f: F)
where
    F: FnOnce(&mut Books, &mut Market) + Send + Sync + 'static,
{
    app.insert_resource(Ops(Some(Box::new(f))));
    // 只在真要跑这一次操作时挂上来：`Books` 需要 `ActiveBoard` 已就位，
    // 提前注册会让装配前的空帧 panic
    app.add_systems(Update, run_op_system);
    app.update();
}

/// 造一颗「第一掷恰好是 face」的种子。对局里的骰子流与独立掷骰同序
/// （分流的意义，见 `rng.rs`），所以这条在 B3 之后仍然可靠。
pub fn seed_rolling_first(face: u32) -> GameSeed {
    for s in 0..10_000u64 {
        if Rng::new(s).roll_die() == face {
            return GameSeed(s);
        }
    }
    panic!("一万颗种子里居然没掷出 {face}，骰子分布可疑");
}

/// 装配到「0 号玩家站在 `tile` 上、下一步就掷骰」的状态，但不替测试跑回合 ——
/// 落格结算类用例的标准前置：控制「踩到哪一格」而不是碰运气等种子。
/// 预置归属/现金要在拿到 app 之后做，然后自己 `play_turns(app, 1)`。
pub fn ready_on(tile: u32, steps: u32, policy: DecisionPolicy) -> (App, Entity) {
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(steps),
    );
    app.insert_resource(policy);
    let p0 = boot_to_wait_roll(&mut app);
    place(&mut app, p0, tile);
    (app, p0)
}

/// 某只股的现价（测试用独立读数，不去重跑一遍 `Market`）。
pub fn price_of(app: &mut App, id: &str) -> f32 {
    let w = app.world_mut();
    let mut q = w.query::<(
        &game_core::prelude::StockId,
        &game_core::prelude::StockPrice,
    )>();
    q.iter(w)
        .find(|(s, _)| s.0 == id)
        .map(|(_, p)| p.0)
        .expect("stocks.json 里有这只股")
}

/// 某位玩家对某只股的持股数。
pub fn shares_of(app: &mut App, player: Entity, id: &str) -> u32 {
    app.world()
        .entity(player)
        .get::<game_core::prelude::StockHoldings>()
        .expect("玩家应有持仓组件")
        .0
        .get(&game_core::prelude::StockId(id.to_string()))
        .copied()
        .unwrap_or(0)
}

/// 把某位玩家的现金调到 `cash`（预置「付不起」这类边界）。
///
/// 刻意走 `credit`/`debit` 而不是直改组件：现金审计那条不变量是
/// 「余额 == 起始现金 + Σ 流水」，绕过账本去调现金会让审计在别的用例上假性失败。
pub fn set_cash(app: &mut App, player: Entity, cash: i64) {
    let now = cash_of(app, player);
    if cash > now {
        run_op(app, move |books, _| books.credit(player, cash - now));
    } else if cash < now {
        run_op(app, move |books, _| {
            assert!(
                books.debit(player, now - cash).is_ok(),
                "调低现金不该被拒（余额 {now}，扣 {}）",
                now - cash
            )
        });
    }
}

/// 把某格改成别的类型（prototype 没有证券格，行情那组用例只能就地改棋盘）。
pub fn retile(app: &mut App, tile: u32, kind: game_core::prelude::TileKind) {
    app.world_mut().resource_mut::<ActiveBoard>().def.tiles[tile as usize].kind = kind;
}

/// 把玩家的逻辑位置搬到任意主环格（`WaitRoll` 态没有系统会写它，直接改安全）。
pub fn place(app: &mut App, player: Entity, tile: u32) {
    app.world_mut()
        .entity_mut(player)
        .get_mut::<TokenPosition>()
        .expect("玩家应有位置")
        .tile = TileIndex(tile);
}

/// 预置地块状态：归属（按 `PlayerId`）、等级、是否抵押。
/// 租金/垄断/升级类用例的前置 —— 比让玩家真去买地更能聚焦在被测的性质上。
pub fn own(app: &mut App, land: u32, owner: Option<u32>, level: u8, mortgaged: bool) {
    let pid = owner.map(PlayerId);
    let target = {
        let w = app.world_mut();
        let mut q = w.query::<(Entity, &LandId)>();
        q.iter(w)
            .find(|(_, id)| id.0 == land)
            .map(|(e, _)| e)
            .expect("prototype 里有这块地")
    };
    let w = app.world_mut();
    let mut e = w.entity_mut(target);
    if mortgaged {
        e.insert((Owner(pid), BuildingLevel(level), Mortgaged));
    } else {
        e.insert((Owner(pid), BuildingLevel(level)))
            .remove::<Mortgaged>();
    }
}

/// 把牌堆顶压成指定的那张卡（`CardId` 是 `*_cards.json` 里的下标）。
/// 抽卡结果由种子决定，而用例要测的是「这张卡的效果」，所以直接控制发牌。
pub fn stack_card(app: &mut App, kind: CardKind, index: u32) {
    let w = app.world_mut();
    let mut q = w.query::<&mut Deck>();
    for mut deck in q.iter_mut(w) {
        if deck.kind == kind {
            deck.draw_pile = vec![CardId(index)];
            deck.discard.clear();
            return;
        }
    }
    panic!("牌堆没装配（{kind:?}）");
}

pub fn bank_of(app: &mut App, player: Entity) -> (i64, i64) {
    let b = app
        .world()
        .entity(player)
        .get::<BankBalance>()
        .expect("玩家应有银行账户");
    (b.deposit, b.loan)
}

/// 替 `player` 按一次证券面板的买/卖按钮（`id` 用 `stocks.json` 里的字符串）。
/// 走 `Trades` 队列而非直塞消息：这样「只在掷骰前受理」那条门禁是真的被走到的。
pub fn trade(app: &mut App, player: Entity, id: &str, side: TradeSide, shares: u32) {
    app.world_mut()
        .resource_mut::<Trades>()
        .0
        .push(StockTradeRequested {
            player,
            stock: StockId(id.to_string()),
            side,
            shares,
        });
}

/// 替 `player` 按一次柜台/ATM 面板上的金额按钮。
pub fn bank_ask(app: &mut App, player: Entity, op: BankOp, amount: i64) {
    app.world_mut()
        .resource_mut::<BankRequests>()
        .0
        .push(BankOpRequested { player, op, amount });
}

/// 本回合谁有 ATM 资格（`None` = 没人路过银行格）。
pub fn atm_holder(app: &App) -> Option<Entity> {
    app.world().resource::<AtmAccess>().0
}

/// 现在挂着「谁的什么待决事项」。
pub fn pending_of(app: &App) -> Option<(Entity, Pending)> {
    app.world().resource::<PendingDecision>().active()
}

/// 行情钟走到第几天（一个 tick 就是一天）。
pub fn tick_of(app: &App) -> u32 {
    app.world().resource::<MarketClock>().tick
}

/// 现金审计：每位玩家的 `Σ delta` 与钱包余额逐一对账。
///
/// 这是 B3 的「现金流守恒」断言的实际形态：银行与证券市场是**外部对手方**
/// （存款利息凭空进余额、股息凭空进现金），所以全局现金不守恒；
/// 能守恒的是「每一次变动都留下了流水」这一条 —— 它同样抓得住漏发消息、
/// 双重扣款、静默改钱包这三类错误。
pub fn assert_cash_audit(app: &mut App, start_cash: i64) {
    let rows = roster(app);
    let audit = app.world().resource::<CashAudit>().0.clone();
    for &(entity, _, cash, _) in &rows {
        let sum: i64 = audit
            .iter()
            .filter(|(p, _, _)| *p == entity)
            .map(|(_, delta, _)| *delta)
            .sum();
        assert_eq!(
            cash,
            start_cash + sum,
            "玩家 {entity:?} 余额 {cash} 与起始 {start_cash} + 流水 {sum} 对不上"
        );
    }
    // 每条流水的 `balance` 必须是当时该玩家的真实余额（顺序对了才对得上）
    let mut running: std::collections::HashMap<Entity, i64> =
        rows.iter().map(|(e, _, _, _)| (*e, start_cash)).collect();
    for (player, delta, balance) in audit {
        let at = running.entry(player).or_insert(start_cash);
        *at += delta;
        assert_eq!(
            *at, balance,
            "玩家 {player:?} 的流水余额 {balance} 与实际累计 {at} 不符（有变动没发 `CashChanged`）"
        );
    }
}

/// 银行/市场之外的对手方资金不需要对账，但现金为负是硬伤：任何时刻都不该出现。
pub fn assert_no_negative_cash(app: &mut App) {
    for (entity, _, cash, _) in roster(app) {
        assert!(cash >= 0, "玩家 {entity:?} 现金为负：{cash}");
    }
}
