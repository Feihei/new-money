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
    ActiveBoard, Adjustment, ArcadeDeal, AtmAccess, Auction, AuctionFailed, AuctionMove,
    AuctionStarted, AuctionWon, BailRequested, BankBalance, BankOp, BankOpDone, BankOpRequested,
    BankVisited, BidMove, BidPlaced, BidWithdrawn, BombTransferred, Books, BuildOffered,
    BuildRequested, BuildingLevel, BuyOffered, BuyTicketRequested, CardAim, CardDrawn, CardGained,
    CardId, CardKind, CardLost, CardPlayed, CardSet, CashChanged, ChairmanTook, ChargeKind,
    ChargeWaived, ChestOpened, CompanyCharged, CompanyLedgers, CompanyPayout, Correction,
    Corrections, CouponBoxes, DamageTally, DataHandles, Date, Decision, DecisionChosen, Deck,
    Detention, DetentionLifted, DetentionSkipped, DiceRolled, EpochOverride, FacilityBuilt,
    FinancingDrawRequested, FinancingDrawn, FinancingRecalled, GameKind, GameOver, GameRules,
    GameSeed, GameSession, GameSetup, Gameplay, GiftsGiven, God, GodAppeared, GodDeed, GodLeft,
    GodPossessed, Hand, IndemnityPaid, Insulation, Insured, Inventory, ItemKind, ItemPlaced,
    Jackpot, JackpotChanged, JackpotWhy, LandBought, LandDamage, LandId, LandStatus, LandTaken,
    LandUpgraded, LandView, LiquidationPlan, Lot, LotteryBought, LotteryDrawn, LotteryTickets,
    LotteryWon, Market, MarketClock, MinigameResult, MinigameScore, MortgageDone, MortgageOp,
    MortgageRequested, Mortgaged, Mount, MountChanged, MoveCap, MoveCapped, Occupancy, OccupancyAt,
    OccupancyKind, Owner, Pending, PendingDecision, PlaceItemRequested, PlayCardRequested,
    PlayerArrived, PlayerBailed, PlayerBankrupt, PlayerDetained, PlayerId, PlayerTeleported,
    PlotArea, Points, PointsChanged, PolicyBought, Possession, Presence, RedEnvelope,
    RemoteBuildOffered, RemoteDiceUsed, RemoteUpgradeRequested, RentDue, RequestRemoteDice,
    RequestRoll, RequestStay, Rng, Scope, ServiceCharged, Shelf, ShopOpDone, ShopOpRequested,
    ShopStock, Source, StayLifted, StaySentence, StaySkipped, StockId, StockPriceChanged,
    StockTradeRequested, StockTraded, StockVisited, TileIndex, TileStepped, Timed, TokenPosition,
    TradeSide, TurnEnded, TurnOrder, TurnPhase, TurnStarted, UpgradeOffered, WalkHalted,
    WalkStarted, Wallet, deal_to, grant_effects,
};

pub fn asset_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets")
        .canonicalize()
        .expect("资产根应存在")
}

/// 测试侧的「点击掷骰按钮」：`PreUpdate` 把它排空成 `RequestRoll` 消息。
/// 不直接塞消息是为了让测试走与真实 UI 完全相同的入口。
/// 第二个数是这一掷用几颗骰子（`0` = 载具上限，见 `RequestRoll.dice`）。
#[derive(Resource, Default)]
pub struct Inputs(pub Vec<(Entity, u32)>);
/// 测试侧的「点那颗已经翻成『停留』的按钮」（P4④）：排空成 `RequestStay`。
/// 与 [`Inputs`] 分成两条队列，是因为它们现在是两个意图 —— 测试要能分别按。
#[derive(Resource, Default)]
pub struct StayRequests(pub Vec<Entity>);

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

/// 测试侧的「点银行柜台上的抵押/赎回按钮」。同样走队列 → 真消息，于是那条窗口判据
/// （`Pending::BankTeller` 挂的是谁、银行今天开不开门）被真走到，而不是被测试绕过。
#[derive(Resource, Default)]
pub struct MortgageRequests(pub Vec<MortgageRequested>);

/// 测试侧的「点保释窗口里的那一颗『保释 某人』」（P4③）。同样走队列 → 真消息，
/// 于是「窗口挂的是谁」「那个人关在不在这一处」都被真走到。
#[derive(Resource, Default)]
pub struct BailRequests(pub Vec<BailRequested>);

/// 测试侧的「把道具放到某一格」（P5）。与其它队列同一条理由：走真消息，
/// 于是「掷骰前」「主语是当前玩家」「这一格空不空」三道门都被真走到。
#[derive(Resource, Default)]
pub struct PlaceRequests(pub Vec<PlaceItemRequested>);

/// 测试侧的「用遥控骰子定步数」（P5）。同样走队列 → 真消息。
#[derive(Resource, Default)]
pub struct RemoteDiceRequests(pub Vec<RequestRemoteDice>);

/// 测试侧的「一局小游戏打完，把成绩报上去」（P7）。同样走队列 → 真消息，于是
/// 「窗口挂的是谁」「玩的是不是这一款」「`awarded` 那一格认不认」都被真走到 ——
/// 直接塞一条 `MinigameScore` 会绕过整张表，而那张表正是这个阶段唯一难的地方。
#[derive(Resource, Default)]
pub struct Scores(pub Vec<MinigameResult>);

/// 测试侧的「点彩票柜台上那颗号按钮」（P8）。同样走队列 → 真消息，于是
/// 「站在柜台前的人才买得成」「号码在不在池里」「同号第二张要拒」「钱不够要拒」
/// 四道门都被真走到，而不是被测试绕过。
#[derive(Resource, Default)]
pub struct TicketRequests(pub Vec<BuyTicketRequested>);

/// 测试侧的「按下拍卖面板上那两颗按钮之一」（P9）。同样走队列 → 真消息，于是
/// 「不是他的回合要拒」「加不起这一档要拒」「放弃永远办得成」三道门都被真走到，
/// 而不是被测试绕过。
#[derive(Resource, Default)]
pub struct AuctionRequests(pub Vec<AuctionMove>);

/// 测试侧的「点建设窗口里的那颗『建一座购物中心』」（P10c）。同样走队列 → 真消息，
/// 于是「窗口挂的是谁」「这块地归不归他」「一格一设施」「现钱够不够」四道门都被真走到，
/// 而不是被测试绕过。
#[derive(Resource, Default)]
pub struct BuildRequests(pub Vec<BuildRequested>);

/// 测试侧的「在日产建设的窗口里点一颗『升级某地块』」（P10d②）。同样走队列 → 真消息，
/// 于是「窗口挂着谁」「只升自己的地」「董事长那两层怎么免」都被真走到。
#[derive(Resource, Default)]
pub struct RemoteBuildRequests(pub Vec<RemoteUpgradeRequested>);

/// 测试侧的「在银行柜台按一下融资」（P10d②）。同样走队列 → 真消息，于是
/// 「发起人是不是银行保险的董事长」「余额不足按实挪」两条判据都被真走到。
#[derive(Resource, Default)]
pub struct FinancingRequests(pub Vec<FinancingDrawRequested>);

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
    rules: Res<GameRules>,
    pending: Res<PendingDecision>,
    mut answers: MessageWriter<DecisionChosen>,
    mut scores: MessageWriter<MinigameResult>,
    mut moves: MessageWriter<AuctionMove>,
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
        // 收据与卡面无论如何都算「看完了」（P10c 起服务收据也在这一族）
        (
            Pending::RentNotice(_)
            | Pending::ServiceNotice(_)
            | Pending::CompanyNotice
            | Pending::Card,
            _,
        ) => Decision::Acknowledge,
        // 建设窗口（P10c）只认「先不建」。它**不建** —— 与彩票那格「不买券」同一条理由：
        // 建一座设施要 2200，长跑用例的现金断言钉的是「租金与地价」那几笔，
        // 多一笔建设费会让几十条既有断言一起红。建造那条路由 `tests/commercial.rs` 自己走
        (Pending::Build(_), _) => Decision::Acknowledge,
        // 远程建设窗口（P10d②）同一条：自动玩家**不升**任何地 —— 日产格一踩到就开窗，
        // 长跑不自动关掉会整片挂在 `WaitDecision`，而升级那条路由 `tests/remote_build.rs` 自己走
        (Pending::RemoteBuild, _) => Decision::Acknowledge,
        // 柜台只认「办完」。这里必须显式答一句，否则每一个踩到银行格的回合都会
        // 停在 `WaitDecision`，把所有长跑用例整片挂死 —— 编译器逼这一条写出来。
        // 百货柜台同一条（P3d②）：13 格环上真有一格百货，长跑用例一定会踩到它。
        // 百货柜台与保释窗口同一条（P3d②/P4③）：环上真有一格百货，接入点真能被踩到 ——
        // 长跑用例不自动关掉它们就会整片停在 `WaitDecision`
        // 彩票柜台同一条（P8）：环上真有一格彩票中心，不自动关掉就会整片挂死。
        // 它**不买券** —— 买券要花钱，而长跑用例的现金断言钉的是「租金与地价」那几笔，
        // 多一笔买券钱会让几十条既有断言一起红。买券那条路由 `tests/lottery.rs` 自己走
        (Pending::BankTeller | Pending::Shop | Pending::Bail(_) | Pending::Lottery, _) => {
            Decision::Acknowledge
        }
        // 游乐场那一格（P7）**不是一句回答而是一份成绩**：自动玩家打出「满分的一半」。
        // 报 0 分也能过关口，但那样一来长跑就永远走不到「成绩换点券」那条路上，
        // 而这一格是点券的第二个来源（第一个是路上的箱子）
        (Pending::Minigame(game), _) => {
            scores.write(MinigameResult {
                player,
                game,
                score: rules.minigames.of(game).perfect / 2,
                awarded: true,
            });
            return;
        }
        // 拍卖（P9）同样**不是一句回答**：这台机器的窗口对三种 `Decision` 一律不认
        // （理由写在 `decision_system` 那一格上），要办的事走 `AuctionMove`。
        // 自动玩家一律放弃 —— 与彩票那一格「不买券」同一条理由：长跑用例的现金断言钉的是
        // 「租金与地价」那几笔，多一笔拍地的钱会让几十条既有断言一起红；而**必须**答这一句，
        // 否则任一处破产触发的清算会把整片长跑停在「等着问下一个人」的地方。
        // 真拍地那条路由 `tests/auction.rs` 自己走（它把策略换成 `Hold`）。
        (Pending::Auction, _) => {
            moves.write(AuctionMove {
                player,
                what: BidMove::Withdraw,
            });
            return;
        }
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

/// 奖池流水 `(delta, 原因, 结算后池子)`。第三张账 —— 池子不是任何人的钱包，
/// `CashAudit` 与 `PointsAudit` 都读不到它，而 PLAN §四 P8 的验收原话是
/// 「每笔进池的钱都能从流水追溯」，所以它自己得有一张能被 Σ 的表：
/// `池子 == Σ delta`，任何「改了池子没留流水」都会在这里显形。
#[derive(Resource, Default, Debug)]
pub struct JackpotAudit(pub Vec<(i64, game_core::prelude::JackpotWhy, i64)>);

/// `MessageReader` 打包：流程域的 reader 们（0.20 的 reader 带 `'w`/`'s` 两个生命周期，见坑 13）。
///
/// 字段顺序 = `collect` 的记录顺序 = 同帧多条消息在 `Observed` 里的相对顺序，
/// 按「回合开始 → 出狱/服刑 → 掷骰 → 行走 → 到站 → 回合结束」的因果排列。
#[derive(SystemParam)]
struct FlowReaders<'w, 's> {
    turn_started: MessageReader<'w, 's, TurnStarted>,
    lifted: MessageReader<'w, 's, DetentionLifted>,
    detention_skipped: MessageReader<'w, 's, DetentionSkipped>,
    // 停留的两条（P3d④c）。排在服刑之后：同帧两条各写各的，这里记录顺序就是断言要读的相对顺序
    stay_skipped: MessageReader<'w, 's, StaySkipped>,
    stay_lifted: MessageReader<'w, 's, StayLifted>,
    dice_rolled: MessageReader<'w, 's, DiceRolled>,
    walk_started: MessageReader<'w, 's, WalkStarted>,
    tile_stepped: MessageReader<'w, 's, TileStepped>,
    player_arrived: MessageReader<'w, 's, PlayerArrived>,
    /// 走步被放置物打断（P5）。它与 `Arrive` 互斥，所以同一张表里读得出来
    walk_halted: MessageReader<'w, 's, WalkHalted>,
    turn_ended: MessageReader<'w, 's, TurnEnded>,
    game_over: MessageReader<'w, 's, GameOver>,
}

/// 经济/卡牌域的 reader 们。
#[derive(SystemParam)]
struct BookReaders<'w, 's> {
    offers: MessageReader<'w, 's, BuyOffered>,
    upgrade_offers: MessageReader<'w, 's, UpgradeOffered>,
    bought: MessageReader<'w, 's, LandBought>,
    /// 土地公的直接占有（P6c）。与 `bought` 分开记：那条说「有人付了这笔钱」，
    /// 这一条没有付款，而两者的主语甚至可以是同一个人（他先付租、后拿地）
    lands_taken: MessageReader<'w, 's, LandTaken>,
    upgraded: MessageReader<'w, 's, LandUpgraded>,
    rents: MessageReader<'w, 's, RentDue>,
    /// 一次性修正生效那一笔（免费卡花掉了）。它**没有**对应的 `CashChanged`，
    /// 所以「钱一分没动而卡没了」这件事只有这一条流水能证明。
    waived: MessageReader<'w, 's, ChargeWaived>,
    bankrupts: MessageReader<'w, 's, PlayerBankrupt>,
    detained: MessageReader<'w, 's, PlayerDetained>,
    teleported: MessageReader<'w, 's, PlayerTeleported>,
    /// 保释成交（P4③）。它**不是** `Lifted`：那条说的是期满，这条说的是有人替付了账
    bailed: MessageReader<'w, 's, PlayerBailed>,
    cards: MessageReader<'w, 's, CardDrawn>,
    played: MessageReader<'w, 's, CardPlayed>,
    gained: MessageReader<'w, 's, CardGained>,
    bank: MessageReader<'w, 's, BankVisited>,
    bank_ops: MessageReader<'w, 's, BankOpDone>,
    /// 百货成交凭据（P3d②）。与 `gained` 分开记：买入那一次两件事各有一条流水，
    /// 「花了多少点券」与「卡进没进手牌」不是同一个断言。
    shop_ops: MessageReader<'w, 's, ShopOpDone>,
    /// 游乐场那一份成绩的凭据（P7）。它排在 `shop_ops` 之后：两者都是「点券进账」的来源，
    /// 而这一条带着「哪一款、几分」—— 断言要能读出成绩换点券那张表算出来的数
    scores: MessageReader<'w, 's, MinigameScore>,
    /// 抵押柜台的成交凭据（P3d③）。与 `bank_ops` 分开记：那四条是「一个人 + 一个金额」，
    /// 这两件是「一个人 + 一块地」，而断言要的正是「哪块地、过手多少」。
    mortgages: MessageReader<'w, 's, MortgageDone>,
    stock_visit: MessageReader<'w, 's, StockVisited>,
    traded: MessageReader<'w, 's, StockTraded>,
    prices: MessageReader<'w, 's, StockPriceChanged>,
    cash: MessageReader<'w, 's, CashChanged>,
    points: MessageReader<'w, 's, PointsChanged>,
    /// 日历事件（P2b）：礼物一条带全桌，红包一人一条（0 元也有一条）
    gifts: MessageReader<'w, 's, GiftsGiven>,
    envelopes: MessageReader<'w, 's, RedEnvelope>,
    // ===== 道路放置物与道具（P5）=====
    items_placed: MessageReader<'w, 's, ItemPlaced>,
    bombs: MessageReader<'w, 's, BombTransferred>,
    chests: MessageReader<'w, 's, ChestOpened>,
    remote_used: MessageReader<'w, 's, RemoteDiceUsed>,
    capped: MessageReader<'w, 's, MoveCapped>,
    mount_changed: MessageReader<'w, 's, MountChanged>,
    // ===== 神明（P6）=====
    gods_appeared: MessageReader<'w, 's, GodAppeared>,
    gods_possessed: MessageReader<'w, 's, GodPossessed>,
    /// 上身那一刻做完的那件事（给钱/收钱/抢钱/送卡）。它排在 `gods_possessed` 之后：
    /// 记录顺序就是因果顺序，「先换神、后动手」在同一段流水里读得出来。
    gods_deeds: MessageReader<'w, 's, GodDeed>,
    gods_left: MessageReader<'w, 's, GodLeft>,
    /// 衰神每回合弄走的卡（P6b②）。它与 `CardPlayed` 分开读：那条说出牌人，这条说的是
    /// 「我明明有卡，它去哪了」—— 而后者才是这条用例要能回答的那句话。
    cards_lost: MessageReader<'w, 's, CardLost>,
    // ===== 乐透（P8）=====
    tickets: MessageReader<'w, 's, LotteryBought>,
    /// 池子动的每一步。它同时喂 `Ledger`（要说得出为什么动的）与 [`JackpotAudit`]
    /// （要 Σ 得平），与 `cash`/`points` 那两条完全同形 —— 一条消息只能有一个 reader
    /// 拿到这一帧的读取权，分成两个参数包反而是多余的。
    pot: MessageReader<'w, 's, JackpotChanged>,
    draws: MessageReader<'w, 's, LotteryDrawn>,
    wins: MessageReader<'w, 's, LotteryWon>,
    // ===== 拍卖（P9）=====
    auctions_started: MessageReader<'w, 's, AuctionStarted>,
    bids: MessageReader<'w, 's, BidPlaced>,
    passes: MessageReader<'w, 's, BidWithdrawn>,
    sold: MessageReader<'w, 's, AuctionWon>,
    unsold: MessageReader<'w, 's, AuctionFailed>,
    // ===== 商业用地（P10c）=====
    /// 建设报价。与 `upgrade_offers` 分开记：那一条说「往上盖一级」，这一条说
    /// 「立起一座设施」，断言「自己的空商业地不开升级窗口、开建设窗口」要能分开读。
    build_offers: MessageReader<'w, 's, BuildOffered>,
    /// 服务费收据（购物中心摇几点/旅馆住几天/油费几步）。与 `rents` 分开记：
    /// 商业地的这笔钱不查租金表，混进一张表「收租」与「收费」就分不清了。
    services: MessageReader<'w, 's, ServiceCharged>,
    /// 建成凭据。与 `upgraded` 分开记的理由同上。
    built: MessageReader<'w, 's, FacilityBuilt>,
    // ===== 公司企业（P10d①）=====
    /// 公司服务费收据。与 `services` 分开记：那一条的钱进了地主的钱包，
    /// 这一条的钱进了公司的当月损益账 —— 「谁收的」答案不同。
    company_charged: MessageReader<'w, 's, CompanyCharged>,
    /// 公司分红凭据（正负共用一条：亏损月的「扣」也是分红）。
    company_payouts: MessageReader<'w, 's, CompanyPayout>,
    // ===== 保险与董事长特权（P10d②）=====
    /// 投保凭据。与 `company_charged` 分开记：那一条说「保费多少」，这一条说
    /// 「保单生效了、管几天」—— 免单不发保单要用后者单独钉。
    policies: MessageReader<'w, 's, PolicyBought>,
    /// 保险赔付。与 `company_charged` 同理：赔付进了玩家钱包、公司账记负数。
    indemnities: MessageReader<'w, 's, IndemnityPaid>,
    /// 董事长白拿凭据。与 `shop_ops` 分开记：那一条有「实付点券」，这一条**没有钱动**。
    freebies: MessageReader<'w, 's, ChairmanTook>,
    /// 远程建设窗口报到。
    remote_offered: MessageReader<'w, 's, RemoteBuildOffered>,
    /// 融资成交（挪用与兜底共用一条）。
    fin_drawn: MessageReader<'w, 's, FinancingDrawn>,
    /// 融资收回（身份丢了当场清算）。
    fin_recalled: MessageReader<'w, 's, FinancingRecalled>,
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
    for m in r.lifted.read() {
        log.0.push(format!(
            "Lifted({}@{},格{})",
            who(&players, m.player),
            m.place.label(),
            m.junction.0
        ));
    }
    for m in r.detention_skipped.read() {
        log.0.push(format!(
            "DetainSkip({}@{},剩{})",
            who(&players, m.player),
            m.place.label(),
            m.turns_left
        ));
    }
    for m in r.stay_skipped.read() {
        log.0.push(format!(
            "StaySkip({},剩{})",
            who(&players, m.player),
            m.turns_left
        ));
    }
    for m in r.stay_lifted.read() {
        log.0.push(format!("StayLift({})", who(&players, m.player)));
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
    for m in r.walk_halted.read() {
        log.0.push(format!(
            "Halt({},格{},{})",
            who(&players, m.player),
            m.tile.0,
            m.cause.label()
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
    mut pot_audit: ResMut<JackpotAudit>,
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
    for m in r.lands_taken.read() {
        let from = m
            .from
            .map_or_else(|| "无主".to_string(), |old| who(&players, old));
        log.0.push(format!(
            "Take({}:L{}←{from})",
            who(&players, m.player),
            m.land.0
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
    for m in r.detained.read() {
        log.0.push(format!(
            "Detain({}@{},格{})",
            who(&players, m.player),
            m.place.label(),
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
    for m in r.gifts.read() {
        log.0
            .push(format!("Gifts({}年,{}张)", m.year, m.gifts.len()));
    }
    for m in r.envelopes.read() {
        log.0.push(format!(
            "Envelope({}${})",
            who(&players, m.player),
            m.amount
        ));
    }
    for m in r.bailed.read() {
        log.0.push(format!(
            "Bail({}←{}@{},{}点券)",
            who(&players, m.player),
            who(&players, m.by),
            m.place.label(),
            m.points
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
    // 小游戏成绩（P7）。与 `points` 分开记：那条说「点券余额变成多少」，这条说的是
    // 「哪一款、打了几分、换成几点券」—— 而「表算错了」与「账记错了」是两个不同的缺陷
    for m in r.scores.read() {
        log.0.push(format!(
            "Arcade({}·{}·{}分→{})",
            who(&players, m.player),
            m.game.label(),
            m.score,
            if m.awarded {
                format!("{}点券", m.points)
            } else {
                "练习不计".to_string()
            }
        ));
    }
    for m in r.mortgages.read() {
        log.0.push(format!(
            "Mortgage({}{}{}·{})",
            who(&players, m.player),
            match m.op {
                MortgageOp::Take => "押",
                MortgageOp::Redeem => "赎",
            },
            land(m.land),
            m.amount
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
    for m in r.items_placed.read() {
        log.0.push(format!(
            "Place({}:{}→格{})",
            who(&players, m.player),
            m.item.label(),
            m.tile.0
        ));
    }
    for m in r.bombs.read() {
        log.0.push(format!(
            "Bomb({}→{}@格{},剩{})",
            who(&players, m.from),
            who(&players, m.to),
            m.tile.0,
            m.steps_left
        ));
    }
    for m in r.chests.read() {
        log.0.push(format!(
            "Chest({}@格{}:{})",
            who(&players, m.player),
            m.tile.0,
            m.item.label()
        ));
    }
    for m in r.remote_used.read() {
        log.0
            .push(format!("Remote({},{}步)", who(&players, m.player), m.steps));
    }
    for m in r.capped.read() {
        log.0.push(format!(
            "Cap({},{}→{},剩{})",
            who(&players, m.player),
            m.rolled,
            m.moved,
            m.turns_left
        ));
    }
    for m in r.mount_changed.read() {
        log.0.push(format!(
            "Mount({},{})",
            who(&players, m.player),
            m.mount.label()
        ));
    }
    // ===== 神明（P6）=====
    for m in r.gods_appeared.read() {
        log.0
            .push(format!("GodAppear({}@格{})", m.god.label(), m.tile.0));
    }
    for m in r.gods_possessed.read() {
        log.0.push(format!(
            "GodOn({}:{},{}天{})",
            who(&players, m.player),
            m.god.label(),
            m.days,
            match m.replaced {
                Some(old) => format!("顶走{}", old.label()),
                None => String::new(),
            }
        ));
    }
    for m in r.gods_deeds.read() {
        let cards = m
            .cards
            .iter()
            .map(|card| card.title.as_str())
            .collect::<Vec<_>>()
            .join("/");
        log.0.push(format!(
            "GodDeed({}:{},{:+}{})",
            who(&players, m.player),
            m.god.label(),
            m.cash,
            if cards.is_empty() {
                String::new()
            } else {
                format!(",送卡{cards}")
            }
        ));
    }
    for m in r.gods_left.read() {
        log.0.push(format!(
            "GodOff({}:{},{})",
            who(&players, m.player),
            m.god.label(),
            m.reason.label()
        ));
    }
    for m in r.cards_lost.read() {
        log.0.push(format!(
            "Lose({}:{}·{})",
            who(&players, m.player),
            m.card.title,
            m.by.label()
        ));
    }
    // ===== 乐透（P8）=====
    for m in r.tickets.read() {
        log.0.push(format!(
            "Ticket({},{}号,{})",
            who(&players, m.player),
            m.number,
            m.price
        ));
    }
    for m in r.pot.read() {
        log.0
            .push(format!("Pot({}{:+}→{})", m.why.label(), m.delta, m.balance));
        pot_audit.0.push((m.delta, m.why, m.balance));
    }
    for m in r.draws.read() {
        log.0.push(format!(
            "Draw({}号,{}人,各{},池{})",
            m.number, m.winners, m.each, m.pot
        ));
    }
    for m in r.wins.read() {
        log.0.push(format!(
            "Win({}:{}号,+{})",
            who(&players, m.player),
            m.number,
            m.amount
        ));
    }
    // ===== 拍卖（P9）=====
    // 五条各一行，因为它们是**四个不同的问题**：开了哪一场（起拍与台阶）、谁喊到哪、
    // 谁退出、成交给谁、有没有流标。「拍卖办成了」那句断言要能逐问回答，而不是只看最后一条。
    for m in r.auctions_started.read() {
        let founder =
            m.by.map_or_else(|| "bank".to_string(), |e| who(&players, e));
        let from = m
            .seller
            .map_or_else(|| "-".to_string(), |e| who(&players, e));
        log.0.push(format!(
            "Auction({}:by {founder},from {from},start {},step {},{}人)",
            land(m.land),
            m.opening,
            m.step,
            m.bidders
        ));
    }
    for m in r.bids.read() {
        log.0.push(format!(
            "Bid({},{},{})",
            who(&players, m.player),
            land(m.land),
            m.amount
        ));
    }
    for m in r.passes.read() {
        log.0.push(format!(
            "Pass({}:{})",
            who(&players, m.player),
            land(m.land)
        ));
    }
    for m in r.sold.read() {
        let founder =
            m.by.map_or_else(|| "bank".to_string(), |e| who(&players, e));
        log.0.push(format!(
            "Sold({}:{},{},to {founder})",
            who(&players, m.player),
            land(m.land),
            m.price
        ));
    }
    for m in r.unsold.read() {
        log.0.push(format!("Unsold({},{})", land(m.land), m.price));
    }
    // ===== 商业用地（P10c）=====
    for m in r.build_offers.read() {
        log.0.push(format!(
            "BuildOffer({},@{}${})",
            who(&players, m.player),
            m.land.0,
            m.cost
        ));
    }
    for m in r.services.read() {
        log.0.push(format!(
            "Service({}->{}@L{}${},摇{},住{})",
            who(&players, m.payer),
            who(&players, m.owner),
            m.land.0,
            m.amount,
            m.roll,
            m.stay_days
        ));
    }
    for m in r.built.read() {
        log.0.push(format!(
            "Built({}@{}:{:?}${})",
            who(&players, m.player),
            m.land.0,
            m.kind,
            m.cost
        ));
    }
    // ===== 公司企业（P10d①）=====
    for m in r.company_charged.read() {
        log.0.push(format!(
            "Company({}@{}${},摇{},住{}天,董事长{})",
            who(&players, m.payer),
            m.stock,
            m.amount,
            m.roll,
            m.stay_days,
            match m.chairman {
                Some(c) => who(&players, c),
                None => "无".into(),
            }
        ));
    }
    for m in r.company_payouts.read() {
        log.0.push(format!(
            "Dividend({}@{}:每股{}×{}股,当月{})",
            who(&players, m.player),
            m.stock,
            m.per_share,
            m.shares,
            m.profit
        ));
    }
    // ===== 保险与董事长特权（P10d②）=====
    for m in r.policies.read() {
        log.0.push(format!(
            "Policy({}@{},{}天${})",
            who(&players, m.player),
            m.stock,
            m.days,
            m.premium
        ));
    }
    for m in r.indemnities.read() {
        log.0.push(format!(
            "Indemnity({}@{},{}天${})",
            who(&players, m.player),
            m.stock,
            m.days,
            m.amount
        ));
    }
    for m in r.freebies.read() {
        log.0.push(format!(
            "Freebie({}@{},{})",
            who(&players, m.player),
            m.stock,
            m.item
        ));
    }
    for m in r.remote_offered.read() {
        log.0
            .push(format!("RemoteOffered({})", who(&players, m.player)));
    }
    for m in r.fin_drawn.read() {
        log.0.push(format!(
            "FinDraw({}${})",
            who(&players, m.chairman),
            m.amount
        ));
    }
    for m in r.fin_recalled.read() {
        log.0.push(format!(
            "FinRecall({}${})",
            who(&players, m.player),
            m.amount
        ));
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

/// 每一组「测试侧的点击」队列与它对应的消息写出。装一个参数包而不是二十几个函数参数，
/// 与生产代码同一条房规（§六 坑 8），而且这些对子本来就是同一件事：把队列排空成消息。
#[derive(SystemParam)]
struct Pump<'w> {
    inputs: ResMut<'w, Inputs>,
    stay_inputs: ResMut<'w, StayRequests>,
    trades: ResMut<'w, Trades>,
    asks: ResMut<'w, BankRequests>,
    plays: ResMut<'w, Plays>,
    shops: ResMut<'w, ShopRequests>,
    mortgages: ResMut<'w, MortgageRequests>,
    bails: ResMut<'w, BailRequests>,
    places: ResMut<'w, PlaceRequests>,
    remote: ResMut<'w, RemoteDiceRequests>,
    scores: ResMut<'w, Scores>,
    tickets: ResMut<'w, TicketRequests>,
    auctions: ResMut<'w, AuctionRequests>,
    builds: ResMut<'w, BuildRequests>,
    remote_builds: ResMut<'w, RemoteBuildRequests>,
    financings: ResMut<'w, FinancingRequests>,
    rolls: MessageWriter<'w, RequestRoll>,
    stay_writes: MessageWriter<'w, RequestStay>,
    trade_writes: MessageWriter<'w, StockTradeRequested>,
    op_writes: MessageWriter<'w, BankOpRequested>,
    card_writes: MessageWriter<'w, PlayCardRequested>,
    shop_writes: MessageWriter<'w, ShopOpRequested>,
    mortgage_writes: MessageWriter<'w, MortgageRequested>,
    bail_writes: MessageWriter<'w, BailRequested>,
    place_writes: MessageWriter<'w, PlaceItemRequested>,
    remote_writes: MessageWriter<'w, RequestRemoteDice>,
    score_writes: MessageWriter<'w, MinigameResult>,
    ticket_writes: MessageWriter<'w, BuyTicketRequested>,
    auction_writes: MessageWriter<'w, AuctionMove>,
    build_writes: MessageWriter<'w, BuildRequested>,
    remote_build_writes: MessageWriter<'w, RemoteUpgradeRequested>,
    financing_writes: MessageWriter<'w, FinancingDrawRequested>,
}

fn pump_inputs(mut pump: Pump<'_>) {
    for (player, dice) in pump.inputs.0.drain(..) {
        pump.rolls.write(RequestRoll { player, dice });
    }
    for player in pump.stay_inputs.0.drain(..) {
        pump.stay_writes.write(RequestStay { player });
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
    for m in pump.mortgages.0.drain(..) {
        pump.mortgage_writes.write(m);
    }
    for b in pump.bails.0.drain(..) {
        pump.bail_writes.write(b);
    }
    for p in pump.places.0.drain(..) {
        pump.place_writes.write(p);
    }
    for r in pump.remote.0.drain(..) {
        pump.remote_writes.write(r);
    }
    for s in pump.scores.0.drain(..) {
        pump.score_writes.write(s);
    }
    for t in pump.tickets.0.drain(..) {
        pump.ticket_writes.write(t);
    }
    for a in pump.auctions.0.drain(..) {
        pump.auction_writes.write(a);
    }
    for b in pump.builds.0.drain(..) {
        pump.build_writes.write(b);
    }
    for r in pump.remote_builds.0.drain(..) {
        pump.remote_build_writes.write(r);
    }
    for f in pump.financings.0.drain(..) {
        pump.financing_writes.write(f);
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
        .init_resource::<StayRequests>()
        .init_resource::<Trades>()
        .init_resource::<BankRequests>()
        .init_resource::<Plays>()
        .init_resource::<ShopRequests>()
        .init_resource::<MortgageRequests>()
        .init_resource::<BailRequests>()
        .init_resource::<PlaceRequests>()
        .init_resource::<RemoteDiceRequests>()
        .init_resource::<Scores>()
        .init_resource::<TicketRequests>()
        .init_resource::<AuctionRequests>()
        .init_resource::<BuildRequests>()
        .init_resource::<RemoteBuildRequests>()
        .init_resource::<FinancingRequests>()
        .init_resource::<DecisionPolicy>()
        .init_resource::<Observed>()
        .init_resource::<Ledger>()
        .init_resource::<CashAudit>()
        .init_resource::<PointsAudit>()
        .init_resource::<JackpotAudit>()
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
                // 被停留按住的人那颗按钮已经翻成「停留」（P4④）：脚手架要像真人一样按它，
                // 否则 `roll_dice_system` 会拒掉、回合机原地空转到超时
                if app.world().entity(cur).get::<StaySentence>().is_some() {
                    stay(app, cur);
                } else {
                    roll(app, cur);
                }
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

/// 替 `player` 按下掷骰按钮（用载具上限的颗数）。
pub fn roll(app: &mut App, player: Entity) {
    roll_with_dice(app, player, 0);
}

/// 替 `player` 按下「摇 N 颗骰子」那颗按钮。`0` = 载具上限。
pub fn roll_with_dice(app: &mut App, player: Entity, dice: u32) {
    app.world_mut()
        .resource_mut::<Inputs>()
        .0
        .push((player, dice));
}

/// 替 `player` 按下那颗「停留」按钮（P4④）。与 [`roll`] 走同一类入口 ——
/// 消息队列排空成 `RequestStay`，而不是直接改组件。
pub fn stay(app: &mut App, player: Entity) {
    app.world_mut()
        .resource_mut::<StayRequests>()
        .0
        .push(player);
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

/// 把某格改成游乐场（P7）。`game` 为 `None` 就是数据里那个「这一趟随机一款」的缺省形状，
/// 于是「抽签走小游戏子流」与「写死了就照写的玩」两条路各有用例可走。
/// 与 `coupon_tile` 同一手法：棋盘是资源里的一份快照，改它不必动 JSON。
pub fn arcade_tile(app: &mut App, tile: usize, game: Option<GameKind>) {
    let mut board = app.world_mut().resource_mut::<ActiveBoard>();
    let t = &mut board.def.tiles[tile];
    t.kind = game_core::prelude::TileKind::Arcade;
    t.data.game = game;
}

/// 替玩家把一份成绩报上去（`awarded = true`：正经对局里那一趟）。
/// 走 `Scores` 队列 → 真消息，所以「窗口挂的是谁」「玩的是不是这一款」两道门都被真走到。
pub fn report_score(app: &mut App, player: Entity, game: GameKind, score: u32) {
    report_score_awarded(app, player, game, score, true);
}

/// 同一件事，但把 `awarded` 那一格显式翻成 `false`（练习局那条路）。
/// 单开一个函数而不是给上面那个加默认参数：这一格是 PLAN §四 P7 点名的验收，
/// 用例里写着 `awarded: false` 才读得出「我在测那个开关」。
pub fn report_score_awarded(
    app: &mut App,
    player: Entity,
    game: GameKind,
    score: u32,
    awarded: bool,
) {
    app.world_mut()
        .resource_mut::<Scores>()
        .0
        .push(MinigameResult {
            player,
            game,
            score,
            awarded,
        });
}

/// 这一趟游乐场发下来的那一手布局种子（`ArcadeDeal`）。
pub fn deal_of(app: &App) -> u64 {
    app.world().resource::<ArcadeDeal>().seed
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

/// P10d①：把行情钟推一天（含公司分红）。`tick_at_turn_end` 的参数比 `run_op` 的
/// 两件套多出损益账与 `Commands`，不值得为它改所有既有用例的闭包签名 —— 单开一条。
///
/// 与 `run_op` 同一条一次性纪律：add 过的 system 在后续每一帧都还在跑，靠旗标
/// 让它空转 —— 否则循环里调 N 次就挂 N 条 tick，行情钟一帧被推 N 天。
#[derive(Resource, Default)]
pub struct MarketTickReq(pub bool);

pub fn run_market_tick(app: &mut App) {
    app.insert_resource(MarketTickReq(true));
    app.add_systems(Update, run_market_tick_system);
    app.update();
}

fn run_market_tick_system(
    mut req: ResMut<MarketTickReq>,
    mut commands: Commands,
    mut books: Books,
    mut market: Market,
    mut ledgers: ResMut<CompanyLedgers>,
) {
    if !req.0 {
        return;
    }
    req.0 = false;
    market.tick_at_turn_end(&mut books, &mut ledgers, &mut commands);
}

/// 一次建筑损害请求（[`run_damage`] 的载体）。
#[derive(Resource)]
pub struct DamageReq(Option<(LandId, LandDamage, PlotArea)>);

/// 跑完那一拳的产出，供测试断言。
#[derive(Resource, Default)]
pub struct DamageOut(Option<DamageTally>);

/// 损害要写回一个资源，而 [`run_op`] 的闭包只拿到 `Books` 与 `Market`、碰不到
/// `ResMut<DamageOut>` —— 为它改那条签名会把所有既有用例一起卷进来，所以这里单开一条
/// 一次性的 system，走的仍然是同一个 `Books` 公开入口。
fn run_damage_system(mut books: Books, mut req: ResMut<DamageReq>, mut out: ResMut<DamageOut>) {
    let Some((center, depth, area)) = req.0 else {
        return;
    };
    req.0 = None;
    out.0 = Some(books.damage(center, depth, area));
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
    let mut q = w.query::<(&LandId, &Owner, &BuildingLevel, &Mortgaged)>();
    q.iter(w)
        .find(|(id, _, _, _)| id.0 == land)
        .map(|(_, owner, level, mortgage)| (owner.0.map(|p| p.0), level.0, mortgage.held()))
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

/// 造第 `n` 颗（0 起）「第一掷恰好是 face」的种子。
///
/// [`seed_rolling_first`] 只给第一颗，而「这一趟随机一款」那条路（P7）要**跨几个种子**
/// 才看得出它真在抽 —— 一颗种子上「抽到的恰好是打气球」与「写死了打气球」长得一模一样。
pub fn nth_seed_rolling_first(face: u32, n: usize) -> GameSeed {
    let mut found = 0usize;
    for s in 0..200_000u64 {
        if Rng::new(s).roll_die() == face {
            if found == n {
                return GameSeed(s);
            }
            found += 1;
        }
    }
    panic!("二十万颗种子里只有 {found} 颗第一掷是 {face}，取不到第 {n} 颗");
}

/// 装配到「0 号玩家站在 `tile` 上、下一步就掷骰」的状态，但不替测试跑回合 ——
/// 落格结算类用例的标准前置：控制「踩到哪一格」而不是碰运气等种子。
/// 预置归属/现金要在拿到 app 之后做，然后自己 `play_turns(app, 1)`。
pub fn ready_on(tile: u32, steps: u32, policy: DecisionPolicy) -> (App, Entity) {
    ready_on_seeded(tile, policy, seed_rolling_first(steps))
}

/// 与 [`ready_on`] 同一件事，但种子由调用方给。随机类用例要的就是这一格：
/// 同一份前置、换一颗种子，看世界是不是真的跟着变。
pub fn ready_on_seeded(tile: u32, policy: DecisionPolicy, seed: GameSeed) -> (App, Entity) {
    let mut app = boot(GameSetup::default(), GameRules::instant(), seed);
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

/// 直接把某位玩家对某只股的持仓摆到 `shares`（预置「谁当董事长」这类前置）。
///
/// 持仓不是现金：没有「余额 == Σ流水」那类审计押着它，走 `trade` 反而要把现金、
/// 现价、封顶三条不相干的量搅进前置 —— 董事长判定要的是精确的股数对比。
pub fn set_shares(app: &mut App, player: Entity, id: &str, shares: u32) {
    app.world_mut()
        .entity_mut(player)
        .get_mut::<game_core::prelude::StockHoldings>()
        .expect("玩家应有持仓组件")
        .0
        .insert(game_core::prelude::StockId(id.to_string()), shares);
}

/// 往某家公司的当月账上添一笔（预置「分红日之前攒下了多少利润」这类前置）。
/// 日期取当前行情钟所在的那一天 —— 损益账按自然月分页，跨月入账会被翻页吞掉。
pub fn add_company_entry(app: &mut App, stock: &str, amount: i64, why: &'static str) {
    let tick = app
        .world()
        .resource::<game_core::prelude::MarketClock>()
        .tick;
    let epoch = app
        .world()
        .resource::<game_core::prelude::GameSession>()
        .epoch;
    let date = game_core::prelude::date_of(tick, epoch);
    app.world_mut()
        .resource_mut::<CompanyLedgers>()
        .current_mut(stock, date)
        .entries
        .push(game_core::prelude::LedgerEntry { date, amount, why });
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

// ===== 道路放置物与道具（P5）=====

/// 清空场上所有放置物。开局会随机撒宝箱，而 P5 的用例要精确控制「哪一格有什么」。
pub fn clear_occupancies(app: &mut App) {
    let victims: Vec<Entity> = {
        let world = app.world_mut();
        let mut q = world.query_filtered::<Entity, With<Occupancy>>();
        q.iter(world).collect()
    };
    for entity in victims {
        app.world_mut().entity_mut(entity).despawn();
    }
}

/// 在某一格放一个放置物（测试侧夹具，不走 `place_item_system` —— 那条测的是「玩家放置」）。
pub fn spawn_occupancy(app: &mut App, kind: OccupancyKind, tile: u32) -> Entity {
    app.world_mut()
        .spawn((Occupancy::on_tile(kind, TileIndex(tile)), Gameplay))
        .id()
}

/// 场上所有「在地上」的放置物，按格号排。携带中的炸弹不在里面（它不占格）。
pub fn occupancies_on(app: &mut App) -> Vec<(u32, OccupancyKind)> {
    let world = app.world_mut();
    let mut q = world.query::<&Occupancy>();
    let mut rows: Vec<(u32, OccupancyKind)> = q
        .iter(world)
        .filter_map(|occ| occ.tile().map(|t| (t.0, occ.kind)))
        .collect();
    rows.sort();
    rows
}

/// 某一种放置物占着哪几格。
///
/// P5 那批用例原本写的是 `occupancies_on(..) == vec![(格, 我要的那一件)]` —— 那时路上不会有
/// 别的东西。**P6 之后这个整表断言不再是它想说的东西**：补刷每回合都往路上放神，
/// 于是「我的地雷没了」会红在「路上还站着一位财神」上。按种类筛，断言才只说它要说的那件事。
pub fn tiles_holding(app: &mut App, kind: OccupancyKind) -> Vec<u32> {
    let mut tiles: Vec<u32> = occupancies_on(app)
        .into_iter()
        .filter(|(_, other)| *other == kind)
        .map(|(tile, _)| tile)
        .collect();
    tiles.sort();
    tiles
}

/// 场上的定时炸弹（含携带中的），返回 `(实体, 位置)`。
pub fn bombs(app: &mut App) -> Vec<(Entity, OccupancyAt)> {
    let world = app.world_mut();
    let mut q = world.query::<(Entity, &Occupancy)>();
    q.iter(world)
        .filter(|(_, occ)| occ.kind == OccupancyKind::TimeBomb)
        .map(|(e, occ)| (e, occ.at))
        .collect()
}

// ===== 神明（P6）=====

/// 把场上的神清干净：路上那几位的实体 despawn，身上的 `Possession` 摘掉。
///
/// 与 [`clear_occupancies`] 分成两条是刻意的：装配会随机刷两位神，而 P6 的用例要的正是
/// 「哪一格站着哪一位、谁身上挂着哪一天数」。只清路上那一半会留下身上的旧附身，
/// 于是「一人一神」那条断言先被自己的前置污染 —— 这是 P5 那次「点券 0 的拒绝理由」
/// 同一种假用例，别再犯第二遍。
pub fn clear_gods(app: &mut App) {
    let on_board: Vec<Entity> = {
        let world = app.world_mut();
        let mut q = world.query_filtered::<Entity, With<Occupancy>>();
        q.iter(world).collect()
    };
    for entity in on_board {
        app.world_mut().entity_mut(entity).despawn();
    }
    let players: Vec<Entity> = {
        let world = app.world_mut();
        let mut q = world.query_filtered::<Entity, With<PlayerId>>();
        q.iter(world).collect()
    };
    for player in players {
        app.world_mut().entity_mut(player).remove::<Possession>();
        // P6b 之后「神留下的痕迹」不止那一个组件：修正登记处里还挂着一整条链。
        // 只清组件会得到一个「神已经走了、租金还是双倍」的世界，而夹具里没有任何东西
        // 说得出一格为什么是这个数。撤销按 `(bearer, Source::God)` 键，所以同一张卡
        // 挂的免费修正照旧留得住。
        let mut corrections = app.world_mut().resource_mut::<Corrections>();
        corrections.release(player, Source::God);
    }
}

/// 只清**路上**那几位神，人身上的留着。
///
/// 「谁会被抽出来」这类用例要的是每回合真的去抽一次签，而人身上的那位正是抽签要避开的那一半 ——
/// 用 [`clear_gods`] 把两边一起清掉，等于把对位规则要测的那半个世界也清掉了。
pub fn clear_board_gods(app: &mut App) {
    let victims: Vec<Entity> = {
        let world = app.world_mut();
        let mut q = world.query::<(Entity, &Occupancy)>();
        q.iter(world)
            .filter(|(_, occ)| matches!(occ.kind, OccupancyKind::God(_)))
            .map(|(e, _)| e)
            .collect()
    };
    for entity in victims {
        app.world_mut().entity_mut(entity).despawn();
    }
}
/// 在某一格放一位神（夹具，不走那条随机刷出 —— 刷出本身另有用例）。
pub fn spawn_god(app: &mut App, god: God, tile: u32) -> Entity {
    spawn_occupancy(app, OccupancyKind::God(god), tile)
}

/// 路上有哪几位神、各站在哪一格（按格号排）。
pub fn gods_on_board(app: &mut App) -> Vec<(u32, God)> {
    let mut rows: Vec<(u32, God)> = occupancies_on(app)
        .into_iter()
        .filter_map(|(tile, kind)| match kind {
            OccupancyKind::God(god) => Some((tile, god)),
            _ => None,
        })
        .collect();
    rows.sort();
    rows
}

/// 某人身上那位（`None` = 没被附身）。天数一并带出来：验收③断言的就是那个数。
pub fn possession_of(app: &App, player: Entity) -> Option<Possession> {
    app.world().entity(player).get::<Possession>().copied()
}

/// 直接给某人挂上一位神，并把祂那条修正链登记好。`days` = `None` 用这位神的默认天数。
///
/// 到期与「拘留期间照老」两类用例的前置：附身路径本身另有用例，让它们各自陪跑七回合
/// 只会把断言变慢，不会把它变严。
///
/// **登记那半句不能省**：真实路径（[`game_core::prelude::attach`]）把「挂组件」与「挂修正」
/// 绑成一句，夹具只写组件就会测出一个世界上不存在的状态 —— 身上的神不花钱、也不收钱，
/// 而「租金翻了三倍」的用例会红在错误的那一行上。一次性效果（给钱/抢钱/送卡）**不做**：
/// 它要 `Books`/`Shelf`/`Rng` 那一摞，而夹具要的正是「只有状态、没有那一次」，
/// 给钱那条由真落格路径覆盖。
pub fn attach_god(app: &mut App, player: Entity, god: God, days: Option<u32>) {
    app.world_mut().entity_mut(player).insert(Possession {
        god,
        days_left: days.unwrap_or_else(|| god.days()),
    });
    let mut corrections = app.world_mut().resource_mut::<Corrections>();
    grant_effects(&mut corrections, god, player);
}

/// 现在还能刷出谁（对位那条规则的可读入口：它读的是世界，不是测试自己算的表）。
pub fn god_pool(app: &mut App) -> Vec<God> {
    let world = app.world_mut();
    let mut gods = world.query::<&Occupancy>();
    let mut attached = world.query::<&Possession>();
    let on_board = gods.iter(world).map(|occ| occ.kind);
    let worn = attached.iter(world).map(|held| held.god);
    Presence::observe(on_board, worn).pool()
}

/// 在场的所有位（路上 + 身上），排好序。对位不同场那条断言读的是这个，
/// 而不是池 —— 「现在能刷谁」是将来，「谁已经在了」是现在。
pub fn gods_in_play(app: &mut App) -> Vec<God> {
    let world = app.world_mut();
    let mut on_board = world.query::<&Occupancy>();
    let mut worn = world.query::<&Possession>();
    let mut rows: Vec<God> = on_board
        .iter(world)
        .filter_map(|occ| match occ.kind {
            OccupancyKind::God(god) => Some(god),
            _ => None,
        })
        .chain(worn.iter(world).map(|held| held.god))
        .collect();
    rows.sort();
    rows
}

/// 某位玩家的道具栏清单（按枚举顺序）。与 [`hand_of`] 分开读，正是「道具不是卡」的形状。
pub fn inventory_of(app: &App, player: Entity) -> Vec<(ItemKind, u32)> {
    app.world()
        .entity(player)
        .get::<Inventory>()
        .map(Inventory::entries)
        .unwrap_or_default()
}

/// 往道具栏里塞 `n` 个（夹具）。真正「用掉」仍走消息入口。
pub fn grant_item(app: &mut App, player: Entity, item: ItemKind, n: u32) {
    app.world_mut()
        .entity_mut(player)
        .get_mut::<Inventory>()
        .expect("玩家应有道具栏")
        .gain(item, n);
}

/// 替 `player` 把一件道具放到 `tile`（走 `PlaceRequests` → 真消息）。
pub fn place_item(app: &mut App, player: Entity, item: ItemKind, tile: u32) {
    app.world_mut()
        .resource_mut::<PlaceRequests>()
        .0
        .push(PlaceItemRequested {
            player,
            item,
            tile: TileIndex(tile),
        });
}

/// 替 `player` 用一颗遥控骰子定步数（走 `RemoteDiceRequests` → 真消息）。
pub fn use_remote_dice(app: &mut App, player: Entity, steps: u32) {
    app.world_mut()
        .resource_mut::<RemoteDiceRequests>()
        .0
        .push(RequestRemoteDice { player, steps });
}

/// 换载具（夹具）。
pub fn set_mount(app: &mut App, player: Entity, mount: Mount) {
    *app.world_mut()
        .entity_mut(player)
        .get_mut::<Mount>()
        .expect("玩家应有 Mount 组件") = mount;
}

pub fn mount_of(app: &App, player: Entity) -> Mount {
    *app.world()
        .entity(player)
        .get::<Mount>()
        .expect("玩家应有 Mount 组件")
}

pub fn move_cap_of(app: &App, player: Entity) -> Option<u32> {
    app.world()
        .entity(player)
        .get::<MoveCap>()
        .map(|c| c.turns_left)
}

/// 某位玩家此刻的 `Detention`（P5 的验收要读「他住院了几天、在哪一处」）。
pub fn detain_of(app: &App, player: Entity) -> Option<Detention> {
    app.world().entity(player).get::<Detention>().copied()
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
    // 本金走同一条纯函数算，不在这里另写一个数 —— 「这块地押着多少」的真相只有一处
    let proceeds = if mortgaged {
        let w = app.world();
        let def = &w.resource::<ActiveBoard>().def;
        let view = LandView::new(def, LandId(land), pid, level, true, None, false);
        game_core::economy::mortgage_cashout(def, &view, w.resource::<GameRules>())
    } else {
        0
    };
    let w = app.world_mut();
    w.entity_mut(target)
        .insert((Owner(pid), BuildingLevel(level), Mortgaged { proceeds }));
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

/// 读某人的融资债（P10d② 三件套的第三本账：不计息、不与 `loan` 混）。
pub fn financing_of(app: &App, player: Entity) -> i64 {
    app.world()
        .entity(player)
        .get::<BankBalance>()
        .expect("玩家应有银行账户")
        .financing
}

/// 读某人的保单（P10d②）。效期与赔付单价都读组件，流水只活两三帧。
pub fn policy_of(app: &App, player: Entity) -> Option<Insured> {
    app.world().entity(player).get::<Insured>().cloned()
}

/// 替 `player` 在日产建设的窗口里点一颗「升级某地块」（P10d②）。
/// 走队列 → 真消息：窗口挂着谁、那块地归不归他，两道门都被真走到。
pub fn remote_build_ask(app: &mut App, player: Entity, land: LandId) {
    app.world_mut()
        .resource_mut::<RemoteBuildRequests>()
        .0
        .push(RemoteUpgradeRequested { player, land });
}

/// 替 `player` 在银行柜台按一下融资（P10d②）：要从 `of_player` 的存款挪 `amount`。
pub fn financing_ask(app: &mut App, player: Entity, of_player: Entity, amount: i64) {
    app.world_mut()
        .resource_mut::<FinancingRequests>()
        .0
        .push(FinancingDrawRequested {
            player,
            of_player,
            amount,
        });
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

/// 替 `player` 按一次柜台上的抵押/赎回按钮（`land` 是地块编号，prototype 里 0..=5）。
/// 同样走队列 → 真消息，于是「站在银行柜台前的那个人才办得成」与休市两条都被真走到。
pub fn mortgage_ask(app: &mut App, player: Entity, op: MortgageOp, land: u32) {
    app.world_mut()
        .resource_mut::<MortgageRequests>()
        .0
        .push(MortgageRequested {
            player,
            op,
            land: LandId(land),
        });
}

/// 替 `player` 按一次保释窗口里的那颗按钮。与 [`mortgage_ask`]/[`shop_ask`] 同一条：
/// 走队列 → 真消息，「窗口挂的是谁」与「那个人关在不在这一处」都被真走到。
pub fn bail_ask(app: &mut App, player: Entity, target: Entity) {
    app.world_mut()
        .resource_mut::<BailRequests>()
        .0
        .push(BailRequested { player, target });
}

/// 替 `player` 按一次彩票柜台上那颗号按钮（P8）。与 [`bail_ask`]/[`shop_ask`] 同一条：
/// 走队列 → 真消息，于是「站在柜台前的人才买得成」「号码在不在池里」「同号第二张要拒」
/// 「钱不够要拒」四道门都被真走到。
pub fn lottery_ask(app: &mut App, player: Entity, number: u8) {
    app.world_mut()
        .resource_mut::<TicketRequests>()
        .0
        .push(BuyTicketRequested { player, number });
}

/// 按下拍卖面板上的一颗按钮（P9）。`what` 是「加价」还是「放弃」。
///
/// 它后面跟一次 `settle`：这台机器的窗口挂在「此刻该出价的那一位」身上，而一次动作会把
/// 轮转指针推给下一个人 —— 不等到下一帧，测试读不到新的那位。
pub fn auction_ask(app: &mut App, player: Entity, what: BidMove) {
    app.world_mut()
        .resource_mut::<AuctionRequests>()
        .0
        .push(AuctionMove { player, what });
    settle(app, 2);
}

/// 拍卖机现在的状态（P9）。断言「这一场排在第几块」「谁还在名单上」读的是这一份，
/// 而不是面板上的字符串。
pub fn auction_of(app: &App) -> Auction {
    app.world().resource::<Auction>().clone()
}

/// 正在进行的这一场。`None` = 机器空着。
pub fn lot_of(app: &App) -> Option<Lot> {
    auction_of(app).lot
}

/// 死者留下的清算料（P9）。用它验「料记下了而没有漏拍、也没有多拍」——
/// 那块数（三）是 `GameRules::liquidation_lots`，写死在断言里就会在改数时红得没道理。
pub fn liquidation_of(app: &App) -> Vec<LiquidationPlan> {
    auction_of(app).liquidation.clone()
}

/// 替 `player` 按下建设窗口里的那颗「建一座购物中心」（P10c）。
/// 走 `BuildRequests` 队列 → 真消息，与其它柜台同一条纪律。
pub fn build_ask(app: &mut App, player: Entity, land: LandId, kind: BuildingKind) {
    app.world_mut()
        .resource_mut::<BuildRequests>()
        .0
        .push(BuildRequested { player, land, kind });
    settle(app, 2);
}

/// 某块地上立着什么设施（P10c）。`None` ＝ 空地（包括空商业地）。
pub fn facility_of(app: &mut App, land: u32) -> Option<BuildingKind> {
    let w = app.world_mut();
    let mut q = w.query::<(&LandId, Option<&Facility>)>();
    q.iter(w)
        .find(|(id, _)| id.0 == land)
        .and_then(|(_, f)| f.map(|f| f.0))
}

use game_core::board::BuildingKind;
use game_core::land::Facility;

/// 奖池现在有多少钱。
pub fn pot_of(app: &App) -> i64 {
    app.world().resource::<Jackpot>().pot
}

/// 最近开出的那个号（`None` = 这一局还没开过奖）。
pub fn last_draw_of(app: &App) -> Option<u8> {
    app.world().resource::<Jackpot>().last_draw
}

/// 某人名下的彩票号（升序，因为 [`LotteryTickets::add`] 就排好序 —— 面板直接念它）。
pub fn tickets_of(app: &App, player: Entity) -> Vec<u8> {
    app.world()
        .entity(player)
        .get::<LotteryTickets>()
        .map(|tickets| tickets.numbers.clone())
        .unwrap_or_default()
}

/// 奖池流水 `(delta, 原因, 结算后池子)` —— 第三张账的测试形态。
pub fn pot_audit(app: &App) -> Vec<(i64, JackpotWhy, i64)> {
    app.world().resource::<JackpotAudit>().0.clone()
}

/// 池子的账必须平：`Σ delta == 现在的池子`，而且每一条流水都说得出为什么动的。
///
/// 与 [`assert_cash_audit`] 同一条纪律，但**分开两条**：池子不是玩家，它的钱不该
/// 出现在任何人的现金审计里（`CashChanged` 与 `JackpotChanged` 是两套账的第三套）。
pub fn assert_pot_audit(app: &mut App, start: i64) {
    let rows = app.world_mut().resource::<JackpotAudit>().0.clone();
    let sum: i64 = rows.iter().map(|(delta, ..)| delta).sum();
    let pot = app.world().resource::<Jackpot>().pot;
    assert_eq!(
        pot,
        start + sum,
        "奖池与流水对不上账：池子 {pot}，Σ delta = {sum}，流水 {rows:?}"
    );
    for (delta, why, balance) in &rows {
        assert_ne!(*delta, 0, "{why:?} 写了一条 0 元流水：池子没说它为什么没动");
        assert!(!why.label().is_empty(), "{why:?} 的文案是空的");
        assert!(*balance >= 0, "池子出现了负数：{rows:?}");
    }
}
/// 这块地押着多少本金（`None` = 没押）。读的是组件，所以它同时是「`Mortgaged` 到底写没写进去」
/// 那条断言的探针。走 `ActiveBoard` 的索引拿实体，不必为一次读数开一条 query。
pub fn mortgage_of(app: &App, land: u32) -> Option<i64> {
    let w = app.world();
    let entity = *w.resource::<ActiveBoard>().lands.get(&LandId(land))?;
    w.entity(entity)
        .get::<Mortgaged>()
        .and_then(|m| m.held().then_some(m.proceeds))
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

/// 某人身上还剩几层**卡给的**护身（`None` = 没有护身组件）。
///
/// 神给的那一半不在这里（P6b②：它现读 `Possession`，见 [`possession_of`]）—— 两半共用一个
/// 读数的话，「免疫期间卡上的层数没被吃掉」这件事就测不出来。
pub fn shield_of(app: &App, player: Entity) -> Option<u32> {
    app.world()
        .entity(player)
        .get::<Insulation>()
        .map(|i| i.charges)
}

/// 某人现在被拘留在哪儿、还剩几回合（`None` = 人在场上）。
/// 给的是整个组件而不是回合数：P4 的用例断言的正是「他在哪儿」这一半。
pub fn detention_of(app: &App, player: Entity) -> Option<Detention> {
    app.world().entity(player).get::<Detention>().copied()
}

/// 某人现在的停留剩余回合（`None` = 没挂着停留卡）。
/// 读组件而不是读流水，是因为「扣到 1 的那一回合当场可行动」这条规则要看的正是**组件还在不在**。
pub fn stay_of(app: &App, player: Entity) -> Option<u32> {
    app.world()
        .entity(player)
        .get::<StaySentence>()
        .map(|s| s.turns_left)
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

/// 把日历拨到第 `tick` 天。与 `NM_DAY` 同一条理由：哪天是周末由骰子走到第几回合决定，
/// 用例要钉的就是「休市那天办不办得成」，不能等运气。
/// **代价**同那个旋钮：拨完那天行情与结息都不再往前走，它是前置不是时光机。
pub fn set_tick(app: &mut App, tick: u32) {
    app.world_mut().resource_mut::<MarketClock>().tick = tick;
}

/// 钉死历法锚点（`NM_EPOCH` 的测试版）。必须在世界装配之前调用 —— 装配时它被读一次，
/// 之后 `GameSession.epoch` 就是那份值。
pub fn pin_epoch(app: &mut App, date: Date) {
    app.insert_resource(EpochOverride(Some(date)));
}

/// 「今天」是哪天：会话锚点 + 行情钟。所有断言都从这里读，不自己再算一遍。
pub fn today(app: &App) -> Date {
    let session = app.world().resource::<GameSession>();
    game_core::prelude::date_of(app.world().resource::<MarketClock>().tick, session.epoch)
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

/// 某人**最后一条**现金流水 `(delta, 结算后余额)`，`None` = 他从头到尾一分没动。
///
/// 「钱真的到账了」这句话在测试里的形状之一。凭据消息（`LotteryWon`/`RentDue`/`BankOpDone`…）
/// 与 `CashChanged` 是两条消息，只断言凭据会放过「发了凭据没加钱」，而 `assert_cash_audit`
/// 也抓不住它 —— 那种世界里钱包与流水都一动没动，照样对得平。
///
/// 但它只在该玩家**之后没有别的现金变动**时才说得出话：长跑用例里命运牌与神还会陆续动他的
/// 钱包，那时候要问的是「有没有这一笔」（[`paid_him`]），不是「最后一笔是不是它」。
pub fn last_cash_move(app: &App, player: Entity) -> Option<(i64, i64)> {
    app.world()
        .resource::<CashAudit>()
        .0
        .iter()
        .rev()
        .find(|(who, ..)| *who == player)
        .map(|(_, delta, balance)| (*delta, *balance))
}

/// 他的现金流水里有没有一笔正好是 `amount`。
///
/// 用在「跑了几十个回合，路上还有神与命运牌在动他的钱包」的场合：那时「最后一笔」
/// 说的是运气，而「发奖那一步真的给某个人加了这么多钱」才是这条用例要问的事。
/// 它与 `Win(...)` 那条凭据、与 [`assert_pot_audit`] 的池子对账是**三条不同的**证据，
/// 合起来才说得出「奖金到账」这句人话。
pub fn paid_him(app: &App, player: Entity, amount: i64) -> bool {
    app.world()
        .resource::<CashAudit>()
        .0
        .iter()
        .any(|(who, delta, _)| *who == player && *delta == amount)
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

/// 直接给某块地盖一个限时状态。与 [`own`] 同一条理由：要测的是「查封中的地办不办得成」，
/// 而不是「查封怎么盖上去」（那条由 `tests/plot_status.rs` 从真卡那一侧钉）。
pub fn stamp(app: &mut App, land: u32, status: LandStatus, days: u32) {
    let target = {
        let w = app.world_mut();
        let mut q = w.query::<(Entity, &LandId)>();
        q.iter(w)
            .find(|(_, id)| id.0 == land)
            .map(|(e, _)| e)
            .expect("prototype 里有这块地")
    };
    app.world_mut()
        .entity_mut(target)
        .insert(Timed::new(status, days));
}

/// 全场现资产总和（物价指数的分子）。**走 `Books::total_assets`**，所以回合末那次重算
/// 读的就是这个数；股票持仓按 0 计（这批用例里没人买过股票）。
pub fn total_assets(app: &mut App, index: f32) -> i64 {
    let out = std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = out.clone();
    run_op(app, move |books, _| {
        *sink.lock().unwrap() = Some(books.total_assets(index, |_| 0.0));
    });
    out.lock().unwrap().expect("跑了一帧就该有读数")
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
