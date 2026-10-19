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
    ActiveBoard, Adjustment, AtmAccess, BankBalance, BankOp, BankOpDone, BankOpRequested,
    BankVisited, Books, BuildingLevel, BuyOffered, CardAim, CardDrawn, CardGained, CardId,
    CardKind, CardPlayed, CardSet, CashChanged, ChargeKind, ChargeWaived, Correction, Corrections,
    CouponBoxes, DamageTally, DataHandles, Decision, DecisionChosen, Deck, DiceRolled, GameOver,
    GameRules, GameSeed, GameSession, GameSetup, Hand, Insulation, JailSkipped, LandBought,
    LandDamage, LandId, LandStatus, LandUpgraded, Market, MarketClock, Mortgaged, Owner, Pending,
    PendingDecision, PlayCardRequested, PlayerArrived, PlayerBankrupt, PlayerId, PlayerJailed,
    PlayerReleased, PlayerTeleported, PlotArea, Points, PointsChanged, RentDue, RequestRoll, Rng,
    Scope, Shelf, ShopOpDone, ShopOpRequested, ShopStock, Source, StockId, StockPriceChanged,
    StockTradeRequested, StockTraded, StockVisited, TileIndex, TileStepped, Timed, TokenPosition,
    TradeSide, TurnEnded, TurnOrder, TurnPhase, TurnStarted, UpgradeOffered, WalkStarted, Wallet,
    deal_to,
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

/// 测试侧的「点手牌浮层上的一张卡」。与证券/银行同一条理由：走真消息，
/// 于是「掷骰前才受理」「手里没有这张卡」「目标不合法」三道门都被真走到。
#[derive(Resource, Default)]
pub struct Plays(pub Vec<PlayCardRequested>);

/// 测试侧的「点百货柜台上的买/卖按钮」。同样走队列 → 真消息，于是柜台那道窗口判据
/// （`Pending::Shop` 挂的是谁）被真走到，而不是被测试绕过。
#[derive(Resource, Default)]
pub struct ShopRequests(pub Vec<ShopOpRequested>);

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
        // 百货柜台同一条（P3d②）：13 格环上真有一格百货，长跑用例一定会踩到它。
        (Pending::BankTeller | Pending::Shop, _) => Decision::Acknowledge,
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

/// 点券流水。与 `CashAudit` 分成两张表就是「两套账」这件事的测试形态：
/// 点券的变动绝不能混进现金那一条去（PLAN §一 拍板永不通兑）。
#[derive(Resource, Default, Debug)]
pub struct PointsAudit(pub Vec<(Entity, i64, u32)>);

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
    /// 一次性修正生效那一笔（免费卡花掉了）。它**没有**对应的 `CashChanged`，
    /// 所以「钱一分没动而卡没了」这件事只有这一条流水能证明。
    waived: MessageReader<'w, 's, ChargeWaived>,
    bankrupts: MessageReader<'w, 's, PlayerBankrupt>,
    jailed: MessageReader<'w, 's, PlayerJailed>,
    teleported: MessageReader<'w, 's, PlayerTeleported>,
    cards: MessageReader<'w, 's, CardDrawn>,
    played: MessageReader<'w, 's, CardPlayed>,
    gained: MessageReader<'w, 's, CardGained>,
    bank: MessageReader<'w, 's, BankVisited>,
    bank_ops: MessageReader<'w, 's, BankOpDone>,
    /// 百货成交凭据（P3d②）。与 `gained` 分开记：买入那一次两件事各有一条流水，
    /// 「花了多少点券」与「卡进没进手牌」不是同一个断言。
    shop_ops: MessageReader<'w, 's, ShopOpDone>,
    stock_visit: MessageReader<'w, 's, StockVisited>,
    traded: MessageReader<'w, 's, StockTraded>,
    prices: MessageReader<'w, 's, StockPriceChanged>,
    cash: MessageReader<'w, 's, CashChanged>,
    points: MessageReader<'w, 's, PointsChanged>,
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
    mut points_audit: ResMut<PointsAudit>,
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
    for m in r.waived.read() {
        log.0.push(format!(
            "Waive({}${}·{:?})",
            who(&players, m.player),
            m.saved,
            m.via
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
    // 出牌流水把「谁打的」与「打在谁头上」都写出来 —— 这正是 `CardPlayed` 存在的意义，
    // 断言「A 陷害 B 但被 B 的免罪抵消」要能同时看见这两半
    for m in r.played.read() {
        let blocked = m
            .card
            .outcomes
            .iter()
            .filter(|o| matches!(o, game_core::prelude::EffectOutcome::Blocked { .. }))
            .count();
        log.0.push(format!(
            "Play({}->{}:{}{})",
            who(&players, m.player),
            who(&players, m.victim),
            m.card.id,
            if blocked > 0 {
                format!("挡{blocked}")
            } else {
                String::new()
            }
        ));
    }
    for m in r.gained.read() {
        log.0.push(format!(
            "Gain({}:{}){}",
            who(&players, m.player),
            m.card.id,
            match &m.pushed_out {
                Some(lost) => format!("挤掉{}", lost.id),
                None => String::new(),
            }
        ));
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
    for m in r.shop_ops.read() {
        log.0.push(format!(
            "Shop({}{}{}·{}点券)",
            who(&players, m.player),
            side_verb(m.side),
            m.card.id,
            m.points
        ));
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
    for m in r.points.read() {
        log.0.push(format!(
            "Points({}+{}={})",
            who(&players, m.player),
            m.delta,
            m.balance
        ));
        points_audit.0.push((m.player, m.delta, m.balance));
    }
}

/// 五组「测试侧的点击」与五条对应的消息写出。装一个参数包而不是十个函数参数，
/// 与生产代码同一条房规（§六 坑 8），而且这五对本来就是同一件事：把队列排空成消息。
#[derive(SystemParam)]
struct Pump<'w> {
    inputs: ResMut<'w, Inputs>,
    trades: ResMut<'w, Trades>,
    asks: ResMut<'w, BankRequests>,
    plays: ResMut<'w, Plays>,
    shops: ResMut<'w, ShopRequests>,
    rolls: MessageWriter<'w, RequestRoll>,
    trade_writes: MessageWriter<'w, StockTradeRequested>,
    op_writes: MessageWriter<'w, BankOpRequested>,
    card_writes: MessageWriter<'w, PlayCardRequested>,
    shop_writes: MessageWriter<'w, ShopOpRequested>,
}

fn pump_inputs(mut pump: Pump<'_>) {
    for player in pump.inputs.0.drain(..) {
        pump.rolls.write(RequestRoll { player });
    }
    for t in pump.trades.0.drain(..) {
        pump.trade_writes.write(t);
    }
    for a in pump.asks.0.drain(..) {
        pump.op_writes.write(a);
    }
    for c in pump.plays.0.drain(..) {
        pump.card_writes.write(c);
    }
    for s in pump.shops.0.drain(..) {
        pump.shop_writes.write(s);
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
        .init_resource::<Plays>()
        .init_resource::<ShopRequests>()
        .init_resource::<DecisionPolicy>()
        .init_resource::<Observed>()
        .init_resource::<Ledger>()
        .init_resource::<CashAudit>()
        .init_resource::<PointsAudit>()
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

/// 某位玩家身上的点券。与 `cash_of` 分开读，正是「两套账」在读数这一侧的形状。
pub fn points_of(app: &App, player: Entity) -> u32 {
    app.world()
        .entity(player)
        .get::<Points>()
        .expect("玩家应有点券组件")
        .0
}

/// 把某格改成点券箱（面额与冷却都要能单独试，prototype 十二格里只装得下一格）。
/// 与 `retile` 同一手法：棋盘是 resource 里的一份快照，改它不必动 JSON。
pub fn coupon_tile(app: &mut App, tile: usize, amount: u32, refresh: Option<u32>) {
    let mut board = app.world_mut().resource_mut::<ActiveBoard>();
    let t = &mut board.def.tiles[tile];
    t.kind = game_core::prelude::TileKind::Coupon;
    t.data.coupon = Some(amount);
    t.data.refresh = refresh;
}

/// 已被取走、还在冷却里的点券箱数量。
pub fn boxes_taken(app: &App) -> usize {
    app.world().resource::<CouponBoxes>().taken()
}

/// 把随机流退回「开局第一条」，下一次掷骰就又是 `seed_rolling_first` 那个点数。
///
/// 冷却类用例要的是「同一个人再次踩到同一格」，而为此真跑六个回合会把别的账
/// （利息、分红、行情）一起卷进来。行情流也跟着重放，但这一组用例不断价格。
pub fn rewind_dice(app: &mut App) {
    let seed = app.world().resource::<GameSeed>().0;
    app.insert_resource(Rng::new(seed));
}

/// 把轮转指针挪到某位玩家身上（与 `finance.rs` 里直接改 `AtmAccess` 同一类前置）。
pub fn make_current(app: &mut App, player: Entity) {
    let mut order = app.world_mut().resource_mut::<TurnOrder>();
    let at = order
        .order
        .iter()
        .position(|e| *e == player)
        .expect("玩家在回合序里");
    order.cursor = at;
}

/// 把「第几回合」的计数推到 `turn`。冷却读的就是这个数，等它不必真跑一局。
pub fn set_turn(app: &mut App, turn: u32) {
    app.world_mut().resource_mut::<GameSession>().turn_count = turn;
}

/// 把物价指数直接摆到 `index`。
///
/// P2 的验收要的是「同一笔 5000 租金在四种状态下」，而指数是**跑出来的**（回合末重算），
/// 要它等于 2.5 得先构造出一场资产翻 2.5 倍的对局 —— 那测的是指数公式而不是管线。
/// 所以这里直接摆值，公式另有 `price_index.rs::the_index_follows_the_money_supply` 钉住。
pub fn set_price_index(app: &mut App, index: f32) {
    app.world_mut().resource_mut::<GameSession>().price_index = index;
}

/// 把某块地的租金表整条改成同一个数。验收要「5000 这笔租金」，而 prototype 的 L0 是 60。
pub fn flat_rent(app: &mut App, land: u32, rent: i64) {
    let mut board = app.world_mut().resource_mut::<ActiveBoard>();
    let l = board
        .def
        .lands
        .iter_mut()
        .find(|l| l.id == land)
        .expect("prototype 里有这块地");
    l.rent = vec![rent; 6];
}

/// 往修正登记处塞一条修正 —— **P3 的卡与 P6 的神明将来就是这么接进来的**。
///
/// 测试侧用它而不是改管线代码，正是 P2 这条链的验收点：加一种修正不必碰 `charge.rs`。
pub fn register(app: &mut App, correction: Correction) {
    app.world_mut()
        .resource_mut::<Corrections>()
        .register(correction);
}

/// 常用的两条修正：整笔抵消（＝将来的免费卡）、乘系数（＝将来的大穷神加倍）。
pub fn cancel_for(app: &mut App, bearer: Entity) {
    register(
        app,
        Correction::always(bearer, Scope::Any, Adjustment::Cancel, Source::Card),
    );
}

pub fn scale_for(app: &mut App, bearer: Entity, by: f64, kind: ChargeKind) {
    register(
        app,
        Correction::always(
            bearer,
            Scope::Domain(game_core::prelude::charge_domain_of(kind)),
            Adjustment::Scale { by },
            Source::God,
        ),
    );
}

/// 登记表里现在挂着几条修正。免费卡那两条验收读的是「卡有没有被花掉」，
/// 而那张卡在管线里的唯一痕迹就是这一行。
pub fn corrections_len(app: &App) -> usize {
    app.world().resource::<Corrections>().len()
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

/// 一次建筑损害请求（[`run_damage`] 的载体）。
#[derive(Resource)]
pub struct DamageReq(Option<(LandId, LandDamage, PlotArea)>);

/// 跑完那一拳的产出，供测试断言。
#[derive(Resource, Default)]
pub struct DamageOut(Option<DamageTally>);

/// `Books::damage` 要 `Commands`（摘 `Mortgaged` 只能用 deferred 的移除），
/// 而 [`run_op`] 的闭包签名里没有它 —— 为它改那条签名会把所有既有用例一起卷进来，
/// 所以这里单开一条一次性的 system，走的仍然是同一个 `Books` 公开入口。
fn run_damage_system(
    mut commands: Commands,
    mut books: Books,
    mut req: ResMut<DamageReq>,
    mut out: ResMut<DamageOut>,
) {
    let Some((center, depth, area)) = req.0 else {
        return;
    };
    req.0 = None;
    out.0 = Some(books.damage(&mut commands, center, depth, area));
}

/// 对真棋盘打一次损害，返回命中统计。prototype 的六个地块：
/// 0 南京路(col2 row0, brown) / 1 汉口路(col3 row0, brown) / 2 外滩(col6 row2, **2×2**, orange) /
/// 3 徐家汇(col4 row5, red) / 4 王府井(col2 row5, yellow) / 5 西单(col0 row2, yellow)。
pub fn run_damage(app: &mut App, center: LandId, depth: LandDamage, area: PlotArea) -> DamageTally {
    app.insert_resource(DamageReq(Some((center, depth, area))));
    app.insert_resource(DamageOut::default());
    app.add_systems(Update, run_damage_system);
    app.update();
    app.world()
        .resource::<DamageOut>()
        .0
        .expect("跑了一帧就该有产出")
}

/// `(业主, 等级, 是否抵押中)` —— 损害阶梯的断言三样都要看。
pub fn plot_state(app: &mut App, land: u32) -> (Option<u32>, u8, bool) {
    let w = app.world_mut();
    let mut q = w.query::<(&LandId, &Owner, &BuildingLevel, Option<&Mortgaged>)>();
    q.iter(w)
        .find(|(id, _, _, _)| id.0 == land)
        .map(|(_, owner, level, mortgaged)| (owner.0.map(|p| p.0), level.0, mortgaged.is_some()))
        .expect("prototype 里有这块地")
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

/// 替 `player` 按一次手牌浮层上的卡（`index` 是 `item_cards.json` 里的下标）。
/// 同样走队列 → 真消息，于是出牌那三道门（窗口、在手、目标）都被真走到。
pub fn play_card(app: &mut App, player: Entity, index: u32, aim: Option<CardAim>) {
    app.world_mut()
        .resource_mut::<Plays>()
        .0
        .push(PlayCardRequested {
            player,
            card: CardId(index),
            aim,
        });
}

/// 手牌里的卡（按获得顺序）。`None` = 这个实体没有 `Hand` 组件。
pub fn hand_of(app: &App, player: Entity) -> Vec<CardId> {
    app.world()
        .entity(player)
        .get::<Hand>()
        .map(|h| h.cards.iter().copied().collect())
        .unwrap_or_default()
}

/// 替 `player` 按一次百货柜台上的买/卖按钮（`index` 是 `item_cards.json` 里的下标）。
/// 同样走队列 → 真消息，于是「站在柜台前的那个人才办得成」这条窗口判据被真走到。
pub fn shop_ask(app: &mut App, player: Entity, side: TradeSide, index: u32) {
    app.world_mut()
        .resource_mut::<ShopRequests>()
        .0
        .push(ShopOpRequested {
            player,
            side,
            card: CardId(index),
        });
}

/// 现在货架上摆着哪几张（按陈列顺序）。
pub fn shelf_of(app: &App) -> Vec<CardId> {
    app.world().resource::<ShopStock>().slots.clone()
}

/// 把货架压成指定的那几张。换货是抽签（`shop::restock`），而用例要断言的是「买这一张
/// 花多少点券、卖回去收多少」—— 与 [`stack_card`] 同一条手法：控制不了随机源就别碰运气。
pub fn set_shelf(app: &mut App, ids: &[u32]) {
    let mut stock = app.world_mut().resource_mut::<ShopStock>();
    stock.slots = ids.iter().map(|&at| CardId(at)).collect();
}

/// 把某位玩家的点券调到 `want`。与 [`set_cash`] 同一条理由：走 `Books` 而不是直改组件，
/// 「每一笔点券变动都留下流水」那条审计才不会因为测试的手 while 假性失败。
pub fn set_points(app: &mut App, player: Entity, want: u32) {
    let now = points_of(app, player);
    if want > now {
        let delta = want - now;
        run_op(app, move |books, _| {
            books.award_points(player, delta);
        });
    } else if want < now {
        let delta = now - want;
        run_op(app, move |books, _| {
            assert!(
                books.spend_points(player, delta),
                "点券调低被拒（余额 {now}，扣 {delta}）"
            );
        });
    }
}

/// 一次发牌请求（[`deal`] 的载体）。
#[derive(Resource)]
pub struct DealReq(Option<(Entity, CardId)>);

/// [`crate::hand::deal_to`] 要 `MessageWriter`，而 [`run_op`] 的闭包签名里没有它 ——
/// 同 [`run_damage`]，为它改那条签名会把所有既有用例一起卷进来，所以单开一条一次 system。
/// 走 `deal_to` 而不是直改 `Hand`：上限与「被挤掉要发流水」这条规则才是被测对象，
/// 绕过它发牌就等于把 P3d 的百货账目测了个寂寞。
fn run_deal_system(
    mut req: ResMut<DealReq>,
    mut hands: Query<&mut Hand, With<PlayerId>>,
    rules: Res<GameRules>,
    shelf: Shelf<'_>,
    mut gained: MessageWriter<CardGained>,
) {
    let Some((player, card)) = req.0.take() else {
        return;
    };
    let Ok(mut hand) = hands.get_mut(player) else {
        return;
    };
    deal_to(
        &mut hand,
        rules.hand_limit,
        card,
        player,
        &shelf,
        &mut gained,
    );
}

/// 把一张道具卡发进某人手里（`index` 是 `item_cards.json` 里的下标）。
/// 百货公司要到 P3d 才有，用例先从这里发牌 —— 但发的是同一条公开入口。
pub fn deal(app: &mut App, player: Entity, index: u32) {
    app.insert_resource(DealReq(Some((player, CardId(index)))));
    app.add_systems(Update, run_deal_system);
    app.update();
}

/// 某人身上还剩几层护身（`None` = 没有护身；`Some(None)` = 不限次的神明那一类）。
pub fn shield_of(app: &App, player: Entity) -> Option<Option<u32>> {
    app.world()
        .entity(player)
        .get::<Insulation>()
        .map(|i| i.charges)
}

/// 场上牌堆的种类名（按名字排序）。用来钉住「第三副池不是一副可抽的牌」——
/// `CardKind` 没有 `Ord`，而这里要的是一个可比对的稳定形状。
pub fn deck_kinds(app: &mut App) -> Vec<String> {
    let w = app.world_mut();
    let mut kinds = w
        .query::<&Deck>()
        .iter(w)
        .map(|d| format!("{:?}", d.kind))
        .collect::<Vec<_>>();
    kinds.sort();
    kinds
}

/// 有 `Hand` 组件的实体数。
pub fn hand_owners(app: &mut App) -> usize {
    let w = app.world_mut();
    w.query_filtered::<&Hand, With<PlayerId>>().iter(w).count()
}

/// `item_cards.json` 里某个 id 的下标 —— 用例按名字拿卡，不靠数组位置（改数据顺序
/// 不该让一批测试集体变红，而「第 0 张是陷害」这种断言本来就说不清是哪张）。
pub fn item_index(app: &App, id: &str) -> CardId {
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().items;
    let set = sets.get(handle).expect("道具牌没加载");
    set.0
        .iter()
        .position(|c| c.id == id)
        .map(|at| CardId(at as u32))
        .unwrap_or_else(|| panic!("item_cards.json 里没有 {id}"))
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

/// 点券那一侧的同一组断言。两条审计各跑各的、互不读对方的流水，
/// 这就是「点券与现金永不通兑」在测试里的形状：捡箱子的 +30 只可能出现在这张表上。
pub fn assert_points_audit(app: &mut App, start: u32) {
    let players: Vec<Entity> = roster(app).into_iter().map(|(e, ..)| e).collect();
    let audit = app.world().resource::<PointsAudit>().0.clone();
    for &player in &players {
        let sum: i64 = audit
            .iter()
            .filter(|(p, ..)| *p == player)
            .map(|(_, delta, _)| *delta)
            .sum();
        assert_eq!(
            points_of(app, player) as i64,
            start as i64 + sum,
            "玩家 {player:?} 的点券与起始 {start} + 流水 {sum} 对不上"
        );
    }
    let mut running: std::collections::HashMap<Entity, i64> =
        players.iter().map(|e| (*e, start as i64)).collect();
    for (player, delta, balance) in audit {
        let at = running.entry(player).or_insert(start as i64);
        *at += delta;
        assert_eq!(
            *at, balance as i64,
            "玩家 {player:?} 的点券流水余额 {balance} 与实际累计 {at} 不符"
        );
    }
}

/// 这块地现在挂着的限时状态（`(种类, 剩余天数)`）。`None` = 没挂。
pub fn status_of(app: &mut App, land: u32) -> Option<(LandStatus, u32)> {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &LandId, &Timed<LandStatus>)>();
    q.iter(w)
        .find(|(_, id, _)| id.0 == land)
        .map(|(_, _, timed)| (timed.kind, timed.days_left))
}

/// 这块地现在的租金。**走 `Books::rent_of`**，也就是踩上去要收的那同一个数 ——
/// 在测试里重算一遍垄断/涨价的乘法，证的就不是游戏而是测试自己了。
pub fn rent_of(app: &mut App, land: u32) -> i64 {
    let out = std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = out.clone();
    run_op(app, move |books, _| {
        *sink.lock().unwrap() = Some(books.rent_of(LandId(land)));
    });
    out.lock().unwrap().expect("跑了一帧就该有读数")
}
