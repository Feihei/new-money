//! P15 存档：一局对局的快照（[`GameSnapshot`]）与「什么时候能存」。
//!
//! **存的是真相，不是影子**：`Books`、`LandView`、董事长、公司当月净额、租金与地价都是现算的，
//! 快照里一格都没有它们。反过来，凡是「下一帧有人会读」的组件与资源都在这里，漏一样就会
//! 表现为「读档之后某个数悄悄变了」—— 所以清单按**区块**维护，加组件时去对应的 `SeatState`
//! ／`LandState` 里加一格，而 `capture` 会把它读出来。
//!
//! **可存点只有一个**（PLAN §四・十六）：`WaitRoll`、槽里没人等着答、门与拍卖都空着、没人在走、
//! 当前这位是人类。这一帧里三台「过程机器」（`Gate`／`Auction`／`MagicRitual`）天然是空的，
//! 于是它们不必序列化；而电脑半回合里的 `AiClock`（墙钟）与 `AiErrands`（节流）也被
//! 「当前这位是人类」挡在门外。
//!
//! **随机流存水位不存种子**：`Rng` 是 20 条互不相干的流，种子只能把整局退回开局。
//!
//! 这一节只做「捕获」；读档装配（把快照装回一个世界）在 P15b，与 `world::begin_game_system`
//! 共用同一条铺局链。

use std::collections::HashMap;

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

use crate::ai::Controller;
use crate::auction::Auction;
use crate::bank::AtmAccess;
use crate::board::BuildingKind;
use crate::charge::{Adjustment, ChargeDomain, Correction, Corrections, Scope, Source};
use crate::company::{CompanyLedgers, MonthLedger};
use crate::coupon::CouponBoxes;
use crate::deck::Deck;
use crate::gods::Possession;
use crate::hand::Hand;
use crate::items::Inventory;
use crate::land::{BuildingLevel, Facility, LandId, Mortgaged, Owner, RentLedger};
use crate::landing::PendingDecision;
use crate::lottery::{Jackpot, LotteryTickets};
use crate::market::MarketClock;
use crate::occupancy::{Occupancy, OccupancyAt, OccupancyKind};
use crate::passive::Gate;
use crate::player::{
    BankBalance, Bankrupt, Detention, Gender, Heading, Insured, Mount, MoveCap, PlayerColor,
    PlayerId, PlayerName, Points, StaySentence, TokenPosition, WalkQueue, Wallet,
};
use crate::rng::{Rng, RngPositions};
use crate::rules::GameRules;
use crate::shop::ShopStock;
use crate::status::{LandStatus, Timed};
use crate::stock::{PriceHistory, StockHoldings, StockId, StockPrice};
use crate::turn::{GameSession, LastRoll, TurnOrder, TurnPhase};
use crate::villain::{Hired, Villain, VillainKind};
use crate::world::{ActiveBoard, Gameplay};

/// 存档格式版本。加字段＋给 `#[serde(default)]` 时递增；`migrate` 负责把旧版本搬上来。
pub const SAVE_VERSION: u32 = 1;

/// 一个行动者在快照里的**稳定身份**。
///
/// 世界里的 `Entity` 是句柄（带世代、重开一局全部作废），快照里只能写身份：
/// 玩家是 `PlayerId`（`Owner` 本来就用它），恶人是他那一位（`VillainKind` 唯一）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SeatRef {
    Player(PlayerId),
    Villain(VillainKind),
}

/// 一局对局的完整快照。字段按区块排，读的时候按区块读。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GameSnapshot {
    pub version: u32,
    /// 整份规则。规则是这一局的一部分：利率、物价门槛、抵押成数换一份，同一局的账就对不上。
    pub rules: GameRules,
    pub session: GameSession,
    pub market: MarketClock,
    /// 按 `PlayerId` 排序。
    pub players: Vec<PlayerState>,
    /// 按 [`VillainKind::ALL`] 的顺序。
    pub villains: Vec<VillainState>,
    /// 按 `LandId` 排序。
    pub lands: Vec<LandState>,
    /// 按 `StockId` 排序。
    pub stocks: Vec<StockState>,
    /// 机会／命运两副，按 [`CardKind`](crate::card::CardKind) 的顺序。
    pub decks: Vec<Deck>,
    pub occupancy: Vec<OccupancyState>,
    pub order: OrderState,
    /// 按登记顺序。`bearer` 已翻译成 [`SeatRef`]。
    pub corrections: Vec<CorrectionState>,
    pub atm: Option<PlayerId>,
    /// 按格子号排序（`HashMap` 的迭代序不能进存档，见 [`CouponBoxes::entries`]）。
    pub coupon_boxes: Vec<(crate::board::TileIndex, u32)>,
    pub shop: ShopStock,
    pub jackpot: Jackpot,
    /// 公司当月账页，顺序＝录入顺序。
    pub ledgers: Vec<(String, MonthLedger)>,
    pub rng: RngPositions,
}

/// 一位玩家的全部随身状态。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlayerState {
    pub id: PlayerId,
    pub name: PlayerName,
    pub color: PlayerColor,
    pub gender: Gender,
    pub controller: Controller,
    pub wallet: Wallet,
    pub points: Points,
    pub bank: BankBalance,
    pub hand: Hand,
    pub inventory: Inventory,
    /// 排序过的持仓（`HashMap` 的迭代序不进存档）。
    pub holdings: Vec<(StockId, u32)>,
    pub tickets: LotteryTickets,
    pub tile: TokenPosition,
    pub heading: Heading,
    pub mount: Mount,
    pub detention: Option<Detention>,
    pub stay: Option<StaySentence>,
    pub move_cap: Option<MoveCap>,
    pub insured: Option<Insured>,
    pub possession: Option<Possession>,
    pub bankrupt: bool,
}

/// 一位恶人的状态。四人开局都在笼里（`Detention{ UntilBailed }`）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VillainState {
    pub kind: VillainKind,
    pub name: PlayerName,
    pub tile: TokenPosition,
    pub heading: Heading,
    pub mount: Mount,
    pub detention: Detention,
    /// 受谁雇用。`Some` 即「在环上走」—— 与 [`Hired`] 存在即受雇同一条判据。
    pub hired_by: Option<PlayerId>,
}

/// 一块地的状态。地价／租金表不存：它们是 `BoardDef` 的静态配置。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LandState {
    pub id: LandId,
    pub owner: Option<PlayerId>,
    pub level: BuildingLevel,
    pub mortgaged: Mortgaged,
    pub rent_ledger: RentLedger,
    pub facility: Option<BuildingKind>,
    /// 限时状态：一格只挂一个（`status::Timed`），所以是 `Option` 而不是 `Vec`。
    pub status: Option<(LandStatus, u32)>,
}

/// 一只股票的现价与历史。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StockState {
    pub id: StockId,
    pub price: StockPrice,
    pub history: PriceHistory,
}

/// 一个道路放置物。位置是排他的（ADR-002），所以是一格而不是两个字段。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OccupancyState {
    pub kind: OccupancyKind,
    pub place: OccupancyPlace,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OccupancyPlace {
    OnTile(crate::board::TileIndex),
    /// 定时炸弹在谁身上、还剩几步。
    Carried {
        by: SeatRef,
        steps_left: u32,
    },
}

/// 轮转表：`Vec<SeatRef>` ＋ 游标。**不重建** —— `insert_after` 的位置是玩法事实
/// （谁排在谁后面），从「玩家＋受雇恶人」再推一遍会丢掉它。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OrderState {
    pub order: Vec<SeatRef>,
    pub cursor: usize,
}

/// 一条常开修正。`Scope::Domains` 拿的是 `&'static` 切片，存成自己的形状。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrectionState {
    pub bearer: SeatRef,
    pub scope: ScopeState,
    pub adjustment: Adjustment,
    pub source: Source,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ScopeState {
    Any,
    Domain(ChargeDomain),
    Domains(Vec<ChargeDomain>),
}

/// 读档能把 `Domains` 换回 `&'static` 的**全集**：代码里写下来的那几张表。
///
/// 新增一条 `Scope::domains(&[...])` 时把它加进来；漏了不会错账（内容仍相等），
/// 只是那一条会退回一条泄漏出来的切片并 `warn!` 一句 —— 见 [`scope_from_state`]。
static KNOWN_DOMAIN_LISTS: &[&[ChargeDomain]] = &[&[ChargeDomain::Rent, ChargeDomain::Penalty]];

impl From<Scope> for ScopeState {
    fn from(scope: Scope) -> Self {
        match scope {
            Scope::Any => Self::Any,
            Scope::Domain(domain) => Self::Domain(domain),
            Scope::Domains(list) => Self::Domains(list.to_vec()),
        }
    }
}

fn scope_from_state(state: ScopeState) -> Scope {
    match state {
        ScopeState::Any => Scope::Any,
        ScopeState::Domain(domain) => Scope::Domain(domain),
        ScopeState::Domains(list) => match KNOWN_DOMAIN_LISTS
            .iter()
            .find(|known| **known == list.as_slice())
        {
            Some(known) => Scope::Domains(known),
            None => {
                warn!("存档里有一条没登记的域列表 {list:?}，退回一块泄漏出来的切片");
                Scope::Domains(Box::leak(list.into_boxed_slice()))
            }
        },
    }
}

/// 请存一次。表现层在按钮那一刻置 `true`，捕获 system 取走。
///
/// 为什么是资源而不是消息：捕获是独占 system（一次拿全 `&mut World`，不跟 `Books`/`AiHand`
/// 那类参数包打架），而独占 system 读消息得自己开游标；一个布尔"许愿"与
/// [`Auction::requested`](crate::auction::Auction) 同一种形状。
#[derive(Resource, Debug, Default)]
pub struct SaveRequest {
    pub pending: bool,
}

/// 捕获结果。表现层取走它落盘（core 不碰文件路径，那是 `game_app` 的活）。
#[derive(Resource, Default)]
pub struct SnapshotTaken(pub Option<GameSnapshot>);

/// 「此刻能不能存」的读数：`None` = 正是一个可存点，`Some(why)` = 灰按钮要念的那句话。
///
/// 每帧由 [`judge_system`] 重算。与 [`capture_system`] 走同一份事实（[`save_point`]）——
/// 灰按钮的文案与「按下去被拒」的那句话出自同一次判断，否则会出现「亮着、按下去说不能存」。
#[derive(Resource, Debug, Default)]
pub struct SaveDenied(pub Option<SaveDeny>);

/// 存不了的理由。灰按钮的文案与规则层的拒绝读同一张表（同 `ShopDeny`/`LotteryDeny`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveDeny {
    /// 还没进对局（主菜单）。
    NotInMatch,
    /// 不是「轮到你、还没掷骰」那一刻。
    NotYourTurn,
    /// 有人被挂着等他回答。
    Waiting,
    /// 被动卡那道门正在问人。
    GateAsking,
    /// 拍卖还没散场。
    AuctionRunning,
    /// 棋子还在走。
    Walking,
}

impl SaveDeny {
    /// 全表。灰按钮那条用例逐条打它 —— `label` 的穷尽表已经会逼人给新拒因写文案，
    /// 这一行把那句话也逼进测试（漏了这里，新拒因就只在「真的被拒」时才第一次上屏）。
    pub const ALL: [Self; 6] = [
        Self::NotInMatch,
        Self::NotYourTurn,
        Self::Waiting,
        Self::GateAsking,
        Self::AuctionRunning,
        Self::Walking,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Self::NotInMatch => "现在没有对局",
            Self::NotYourTurn => "等你的回合、掷骰之前才能存",
            Self::Waiting => "先把眼前这个窗口答完",
            Self::GateAsking => "有人正要答一张被动卡",
            Self::AuctionRunning => "拍卖还没散场",
            Self::Walking => "棋子还在走",
        }
    }
}

/// 判断可存点的全部事实。抽成结构体是为了让这张判据表能被单测逐条打 ——
/// 世界里的五种形状要摆起来各有各的前置，而判据本身只有这六个布尔。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SaveFacts {
    pub phase: TurnPhase,
    pub in_match: bool,
    pub waiting: bool,
    pub gate: bool,
    pub auction: bool,
    pub walking: bool,
    pub human: bool,
}

/// 可存点判据。**顺序有意义**：先说「现在根本不能存」，再说「还没轮到你」。
pub fn judge(facts: SaveFacts) -> Result<(), SaveDeny> {
    if !facts.in_match || facts.phase == TurnPhase::Boot {
        return Err(SaveDeny::NotInMatch);
    }
    if facts.phase != TurnPhase::WaitRoll || !facts.human {
        return Err(SaveDeny::NotYourTurn);
    }
    if facts.walking {
        return Err(SaveDeny::Walking);
    }
    if facts.waiting {
        return Err(SaveDeny::Waiting);
    }
    if facts.gate {
        return Err(SaveDeny::GateAsking);
    }
    if facts.auction {
        return Err(SaveDeny::AuctionRunning);
    }
    Ok(())
}

/// 捕获 system：排在 `Last`，独占 world。请存一次只捕获一份。
pub fn capture_system(world: &mut World) {
    let Some(request) = world.get_resource::<SaveRequest>() else {
        return;
    };
    if !request.pending {
        return;
    }
    world.resource_mut::<SaveRequest>().pending = false;
    match capture_match(world) {
        Ok(snapshot) => {
            world.resource_mut::<SnapshotTaken>().0 = Some(snapshot);
            world.resource_mut::<SaveDenied>().0 = None;
        }
        Err(why) => {
            info!("存档被拒：{}", why.label());
            world.resource_mut::<SaveDenied>().0 = Some(why);
        }
    }
}

/// 此刻能不能存。判据是六样事实，[`judge`] 只认这六个布尔 —— 分开两半是为了让
/// 「读世界」与「拍板」各自能单独验。
///
/// [`capture_match`] 与 [`judge_system`] 都从这里读，于是暂停菜单那颗灰按钮的文案
/// 与「按下去被拒」的那句话永远出自同一份事实。
fn save_point(world: &mut World) -> Result<(), SaveDeny> {
    let in_match = world.get_resource::<crate::world::ActiveBoard>().is_some();
    let phase = world
        .get_resource::<State<TurnPhase>>()
        .map(|state| *state.get())
        .unwrap_or(TurnPhase::Boot);
    let waiting = world
        .get_resource::<PendingDecision>()
        .is_some_and(|pending| pending.active().is_some());
    let gate = world.get_resource::<Gate>().is_some_and(Gate::holds_turn);
    let auction = world
        .get_resource::<Auction>()
        .is_some_and(Auction::holds_turn);
    let walking = {
        let mut query = world.query::<&WalkQueue>();
        query.iter(world).any(|queue| !queue.path.is_empty())
    };
    let human = world
        .get_resource::<TurnOrder>()
        .and_then(TurnOrder::current)
        .is_some_and(|seat| matches!(world.get::<Controller>(seat), Some(Controller::Human)));
    judge(SaveFacts {
        phase,
        in_match,
        waiting,
        gate,
        auction,
        walking,
        human,
    })
}

/// 每帧把「此刻能不能存」抄进 [`SaveDenied`]（P15b）。
///
/// 独占 system 而不是九参数包：五样事实散在 `State`／资源／两条查询里，而
/// [`capture_system`] 已经是同一形状的本家 —— 两处读世界的写法一样，
/// 「按钮说能存、按下去说不能」这种分叉才没有缝可钻。
pub fn judge_system(world: &mut World) {
    let why = save_point(world).err();
    let mut denied = world.resource_mut::<SaveDenied>();
    if denied.0 != why {
        denied.0 = why;
    }
}

/// 把此刻的世界拍成一张快照。**不可存时返回理由**，一个字都不写。
pub fn capture_match(world: &mut World) -> Result<GameSnapshot, SaveDeny> {
    save_point(world)?;

    // 身份表：`Entity → SeatRef`。下面四处（轮转、修正、ATM、炸弹在谁身上）都靠它。
    let mut seat_of: HashMap<Entity, SeatRef> = HashMap::new();
    let mut player_entities: Vec<(Entity, PlayerId)> = {
        let mut query = world.query::<(Entity, &PlayerId)>();
        query.iter(world).map(|(e, id)| (e, *id)).collect()
    };
    player_entities.sort_by_key(|(_, id)| id.0);
    let mut villain_entities: Vec<(Entity, VillainKind)> = {
        let mut query = world.query::<(Entity, &Villain)>();
        query
            .iter(world)
            .map(|(e, villain)| (e, villain.kind))
            .collect()
    };
    villain_entities.sort_by_key(|(_, kind)| villain_rank(*kind));
    for (entity, id) in &player_entities {
        seat_of.insert(*entity, SeatRef::Player(*id));
    }
    for (entity, kind) in &villain_entities {
        seat_of.insert(*entity, SeatRef::Villain(*kind));
    }

    let players: Vec<PlayerState> = player_entities
        .iter()
        .map(|(entity, _)| read_player(&*world, *entity))
        .collect();
    let villains: Vec<VillainState> = villain_entities
        .iter()
        .map(|(entity, kind)| read_villain(&*world, *entity, *kind))
        .collect();

    let mut lands: Vec<LandState> = {
        let mut query = world.query::<(
            &LandId,
            &Owner,
            &BuildingLevel,
            &Mortgaged,
            &RentLedger,
            Option<&Facility>,
            Option<&Timed<LandStatus>>,
        )>();
        query
            .iter(world)
            .map(
                |(id, owner, level, mortgaged, ledger, facility, status)| LandState {
                    id: *id,
                    owner: owner.0,
                    level: *level,
                    mortgaged: *mortgaged,
                    rent_ledger: *ledger,
                    facility: facility.map(|facility| facility.0),
                    status: status.map(|status| (status.kind, status.days_left)),
                },
            )
            .collect()
    };
    lands.sort_by_key(|land| land.id.0);

    let mut stocks: Vec<StockState> = {
        let mut query = world.query::<(&StockId, &StockPrice, &PriceHistory)>();
        query
            .iter(world)
            .map(|(id, price, history)| StockState {
                id: id.clone(),
                price: *price,
                history: history.clone(),
            })
            .collect()
    };
    stocks.sort_by(|a, b| a.id.0.cmp(&b.id.0));

    let mut decks: Vec<Deck> = {
        let mut query = world.query::<&Deck>();
        query.iter(world).cloned().collect()
    };
    decks.sort_by_key(|deck| deck.kind as u8);

    let mut occupancy: Vec<OccupancyState> = {
        let mut query = world.query::<&Occupancy>();
        query
            .iter(world)
            .filter_map(|occupancy| {
                let place = match occupancy.at {
                    OccupancyAt::OnTile(tile) => OccupancyPlace::OnTile(tile),
                    OccupancyAt::Carried { by, steps_left } => match seat_of.get(&by) {
                        Some(seat) => OccupancyPlace::Carried {
                            by: *seat,
                            steps_left,
                        },
                        None => {
                            // 携带者只在整局收场时 despawn，而可存点上世界正在跑 —— 真撞见
                            // 说明别处把一个还挂着的炸弹连同携带者一起删了。**不替它编一格**：
                            // 编成 0 号格会在读档后凭空多一颗炸弹，那比漏掉更难查。
                            warn!("放置物挂在一个已离场的行动者身上，这一件不进存档");
                            return None;
                        }
                    },
                };
                Some(OccupancyState {
                    kind: occupancy.kind,
                    place,
                })
            })
            .collect()
    };
    occupancy.sort_by(|a, b| {
        (occupancy_rank(a.kind), place_rank(&a.place))
            .cmp(&(occupancy_rank(b.kind), place_rank(&b.place)))
    });

    let order = {
        let order = world.resource::<TurnOrder>();
        OrderState {
            order: order
                .order
                .iter()
                .filter_map(|entity| seat_of.get(entity).copied())
                .collect(),
            cursor: order.cursor,
        }
    };

    let corrections = world
        .resource::<Corrections>()
        .entries()
        .iter()
        .filter_map(|correction| {
            let bearer = seat_of.get(&correction.bearer)?;
            Some(CorrectionState {
                bearer: *bearer,
                scope: correction.scope.into(),
                adjustment: correction.adjustment,
                source: correction.source,
            })
        })
        .collect();

    let atm = world
        .resource::<AtmAccess>()
        .0
        .and_then(|entity| seat_of.get(&entity).copied())
        .and_then(|seat| match seat {
            SeatRef::Player(id) => Some(id),
            // ATM 卡只会发给玩家（`Books::solvent` 那一侧），恶人拿到是数据错误
            SeatRef::Villain(_) => None,
        });

    Ok(GameSnapshot {
        version: SAVE_VERSION,
        rules: *world.resource::<GameRules>(),
        session: world.resource::<GameSession>().clone(),
        market: *world.resource::<MarketClock>(),
        players,
        villains,
        lands,
        stocks,
        decks,
        occupancy,
        order,
        corrections,
        atm,
        coupon_boxes: world.resource::<CouponBoxes>().entries(),
        shop: world.resource::<ShopStock>().clone(),
        jackpot: *world.resource::<Jackpot>(),
        ledgers: world.resource::<CompanyLedgers>().months().to_vec(),
        rng: world.resource::<Rng>().positions(),
    })
}

/// 玩家的那一半：每个必需组件缺一块都是装配缺陷，所以 `expect` 得开口说话
/// （静默丢一位玩家的钱包，读档后才会以「钱少了」的形式回到玩家身上）。
fn read_player(world: &World, entity: Entity) -> PlayerState {
    let id = *world.get::<PlayerId>(entity).expect("玩家缺 PlayerId");
    PlayerState {
        id,
        name: world.get::<PlayerName>(entity).expect("玩家缺名字").clone(),
        color: *world.get::<PlayerColor>(entity).expect("玩家缺颜色"),
        gender: *world.get::<Gender>(entity).expect("玩家缺性别"),
        controller: *world.get::<Controller>(entity).expect("玩家缺座位归属"),
        wallet: *world.get::<Wallet>(entity).expect("玩家缺钱包"),
        points: *world.get::<Points>(entity).expect("玩家缺点券"),
        bank: *world.get::<BankBalance>(entity).expect("玩家缺银行"),
        hand: world.get::<Hand>(entity).expect("玩家缺手牌").clone(),
        inventory: world.get::<Inventory>(entity).expect("玩家缺道具").clone(),
        holdings: {
            let holdings = world.get::<StockHoldings>(entity).expect("玩家缺持仓");
            let mut list: Vec<(StockId, u32)> =
                holdings.0.iter().map(|(id, n)| (id.clone(), *n)).collect();
            list.sort_by(|a, b| a.0.0.cmp(&b.0.0));
            list
        },
        tickets: world
            .get::<LotteryTickets>(entity)
            .expect("玩家缺持号")
            .clone(),
        tile: *world.get::<TokenPosition>(entity).expect("玩家缺位置"),
        heading: *world.get::<Heading>(entity).expect("玩家缺朝向"),
        mount: *world.get::<Mount>(entity).expect("玩家缺载具"),
        detention: world.get::<Detention>(entity).copied(),
        stay: world.get::<StaySentence>(entity).copied(),
        move_cap: world.get::<MoveCap>(entity).copied(),
        insured: world.get::<Insured>(entity).cloned(),
        possession: world.get::<Possession>(entity).copied(),
        bankrupt: world.get::<Bankrupt>(entity).is_some(),
    }
}

fn read_villain(world: &World, entity: Entity, kind: VillainKind) -> VillainState {
    VillainState {
        kind,
        name: world.get::<PlayerName>(entity).expect("恶人缺名字").clone(),
        tile: *world.get::<TokenPosition>(entity).expect("恶人缺位置"),
        heading: *world.get::<Heading>(entity).expect("恶人缺朝向"),
        mount: *world.get::<Mount>(entity).expect("恶人缺载具"),
        detention: *world.get::<Detention>(entity).expect("恶人缺拘留状态"),
        hired_by: world
            .get::<Hired>(entity)
            .and_then(|hired| world.get::<PlayerId>(hired.employer))
            .copied(),
    }
}

fn villain_rank(kind: VillainKind) -> usize {
    VillainKind::ALL
        .iter()
        .position(|known| *known == kind)
        .unwrap_or(usize::MAX)
}

fn occupancy_rank(kind: OccupancyKind) -> usize {
    match kind {
        OccupancyKind::Roadblock => 0,
        OccupancyKind::Mine => 1,
        OccupancyKind::TimeBomb => 2,
        OccupancyKind::Dog => 3,
        OccupancyKind::Chest => 4,
        OccupancyKind::God(god) => 5 + gods_rank(god),
    }
}

fn gods_rank(god: crate::gods::God) -> usize {
    crate::gods::God::ALL
        .iter()
        .position(|known| *known == god)
        .unwrap_or(usize::MAX)
}

fn place_rank(place: &OccupancyPlace) -> (u8, u32, u32) {
    match place {
        OccupancyPlace::OnTile(tile) => (0, tile.0, 0),
        OccupancyPlace::Carried { by, steps_left } => {
            let seat = match by {
                SeatRef::Player(id) => (1, id.0, 0),
                SeatRef::Villain(kind) => (2, villain_rank(*kind) as u32, 0),
            };
            (seat.0, seat.1, *steps_left)
        }
    }
}

/// 把快照里的 `ScopeState` 换回运行期的 [`Scope`]（读档侧用）。
pub(crate) fn scope_of(state: &ScopeState) -> Scope {
    scope_from_state(state.clone())
}

// ===== 读档装配（P15b）=====

/// 开局走哪条路。表现层点「读档」时插入 [`StartMode::Resume`]，`begin_game_system` 取走它。
///
/// 它是一次性的**许愿**而不是常驻配置：取走之后回到 `Fresh`，否则下一局「新游戏」
/// 会把上一次选的档再读一遍。
#[derive(Resource, Debug, Default)]
pub enum StartMode {
    /// 新局：按 `GameSetup` / `GameSeed` 铺一局。
    #[default]
    Fresh,
    /// 读档：按快照铺一局，落点 `WaitRoll`（不重跑 `TurnStart`）。
    Resume(Box<GameSnapshot>),
}

/// 从快照铺一局。**两遍装配**：先按稳定键铺壳（`LandId`／`PlayerId`／`VillainKind`／
/// 股票 id／牌堆），再把状态灌进去、并把 `Entity` 引用翻译成新句柄。
///
/// 落点由调用方设成 `TurnPhase::WaitRoll`；这里**不跑** `TurnStart` —— 那一格会扣停留与
/// 乌龟计数、把人从机场放回来，而存档点的语义正是「他还没开始这一回合」。
pub fn resume_match(
    commands: &mut Commands,
    def: crate::board::BoardDef,
    stocks: &crate::stock::StockSet,
    holidays: crate::calendar::HolidaySet,
    news: crate::news::NewsSet,
    snapshot: GameSnapshot,
) {
    // 存档与数据必须同一张棋盘、同一份股票表：对不上时下面按 id 回填会**静默**漏掉整块地，
    // 而那正是「读档之后地没了」这类最难查的错。所以这里当场 panic，把话说清楚 —
    // 今天只有一张盘，真撞见就是拿错档或换了数据，不是运行期能兜的事。
    {
        let mut stored: Vec<u32> = snapshot.lands.iter().map(|land| land.id.0).collect();
        let mut board: Vec<u32> = def.lands.iter().map(|land| land.id).collect();
        stored.sort_unstable();
        board.sort_unstable();
        assert_eq!(
            stored, board,
            "存档与当前棋盘的 LandId 对不上 —— 这份档不是在当前棋盘上存的"
        );
        let mut stored: Vec<&str> = snapshot
            .stocks
            .iter()
            .map(|stock| stock.id.0.as_str())
            .collect();
        let mut known: Vec<&str> = stocks.0.iter().map(|stock| stock.id.as_str()).collect();
        stored.sort_unstable();
        known.sort_unstable();
        assert_eq!(stored, known, "存档与当前股票表对不上");
    }

    // ---- 第一遍：壳 ----
    let lands = crate::land::spawn_lands(commands, &def);
    let mut players_by_id: HashMap<PlayerId, Entity> = HashMap::new();
    for state in &snapshot.players {
        let entity = crate::player::spawn_body(
            commands,
            state.id,
            state.name.clone(),
            state.color,
            state.gender,
            state.controller,
        );
        players_by_id.insert(state.id, entity);
    }
    for stock in &snapshot.stocks {
        commands.spawn((
            stock.id.clone(),
            stock.price,
            stock.history.clone(),
            Gameplay,
        ));
    }
    for deck in &snapshot.decks {
        commands.spawn((deck.clone(), Gameplay));
    }
    // 恶人开局的那个壳（四人、全在笼里）原样复用，再按存档改状态
    let caged = crate::villain::spawn_caged_villains(commands, &def);
    let villains_by_kind: HashMap<VillainKind, Entity> = VillainKind::ALL
        .iter()
        .copied()
        .zip(caged.iter().copied())
        .collect();

    let seat_entity = |seat: &SeatRef| -> Entity {
        match seat {
            SeatRef::Player(id) => *players_by_id
                .get(id)
                .unwrap_or_else(|| panic!("存档里的 P{} 不在名单上", id.0)),
            SeatRef::Villain(kind) => *villains_by_kind
                .get(kind)
                .unwrap_or_else(|| panic!("存档里的恶人「{}」不存在", kind.label())),
        }
    };

    // ---- 第二遍：状态 ----
    for state in &snapshot.players {
        let mut player = commands.entity(seat_entity(&SeatRef::Player(state.id)));
        player.insert((
            state.wallet,
            state.points,
            state.bank,
            state.hand.clone(),
            state.inventory.clone(),
            state.tickets.clone(),
            state.tile,
            state.heading,
            state.mount,
        ));
        player.insert(StockHoldings(state.holdings.iter().cloned().collect()));
        if let Some(detention) = state.detention {
            player.insert(detention);
        }
        if let Some(stay) = state.stay {
            player.insert(stay);
        }
        if let Some(move_cap) = state.move_cap {
            player.insert(move_cap);
        }
        if let Some(insured) = state.insured.clone() {
            player.insert(insured);
        }
        if let Some(possession) = state.possession {
            player.insert(possession);
        }
        if state.bankrupt {
            player.insert(Bankrupt);
        }
    }
    for state in &snapshot.villains {
        let mut villain = commands.entity(seat_entity(&SeatRef::Villain(state.kind)));
        villain.insert((
            state.name.clone(),
            state.tile,
            state.heading,
            state.mount,
            state.detention,
        ));
        if let Some(employer) = state.hired_by {
            let boss = *players_by_id
                .get(&employer)
                .unwrap_or_else(|| panic!("存档里恶人的雇主 P{} 不在名单上", employer.0));
            villain.insert(Hired { employer: boss });
        }
    }
    for state in &snapshot.lands {
        let entity = *lands
            .get(&state.id)
            .unwrap_or_else(|| panic!("存档里的地块 L{} 在棋盘上不存在", state.id.0));
        let mut land = commands.entity(entity);
        land.insert((
            Owner(state.owner),
            state.level,
            state.mortgaged,
            state.rent_ledger,
        ));
        // 设施与限时状态**两个方向都要写**：棋盘数据可能预建了设施，而存档里那一格已经
        // 被拆掉了 —— 只写 `Some` 的那一半会让读档后的建筑比存档多一座
        match state.facility {
            Some(kind) => {
                land.insert(Facility(kind));
            }
            None => {
                land.remove::<Facility>();
            }
        }
        match state.status {
            Some((kind, days)) => {
                land.insert(Timed::new(kind, days));
            }
            None => {
                land.remove::<Timed<LandStatus>>();
            }
        }
    }
    for state in &snapshot.occupancy {
        let at = match state.place {
            OccupancyPlace::OnTile(tile) => OccupancyAt::OnTile(tile),
            OccupancyPlace::Carried { by, steps_left } => OccupancyAt::Carried {
                by: seat_entity(&by),
                steps_left,
            },
        };
        commands.spawn((
            Occupancy {
                kind: state.kind,
                at,
            },
            Gameplay,
        ));
    }

    let order = TurnOrder {
        order: snapshot.order.order.iter().map(seat_entity).collect(),
        cursor: if snapshot.order.order.is_empty() {
            0
        } else {
            snapshot.order.cursor.min(snapshot.order.order.len() - 1)
        },
    };
    let corrections = snapshot
        .corrections
        .iter()
        .map(|correction| Correction {
            bearer: seat_entity(&correction.bearer),
            scope: scope_of(&correction.scope),
            adjustment: correction.adjustment,
            source: correction.source,
        })
        .collect::<Vec<_>>();
    let atm = snapshot.atm.map(|id| {
        *players_by_id
            .get(&id)
            .unwrap_or_else(|| panic!("存档里的 ATM 持有人 P{} 不在名单上", id.0))
    });

    // ---- 资源 ----
    let mut rng = Rng::new(snapshot.session.seed);
    rng.set_positions(snapshot.rng);
    let summary = (
        snapshot.session.turn_count,
        snapshot.session.epoch,
        snapshot.players.len(),
        snapshot.lands.len(),
    );
    commands.insert_resource(ActiveBoard { def, lands });
    commands.insert_resource(order);
    commands.insert_resource(snapshot.session.clone());
    commands.insert_resource(snapshot.rules);
    commands.insert_resource(snapshot.market);
    commands.insert_resource(rng);
    commands.insert_resource(holidays);
    commands.insert_resource(news);
    commands.insert_resource(Corrections::from_items(corrections));
    commands.insert_resource(AtmAccess(atm));
    commands.insert_resource(CouponBoxes::from_entries(snapshot.coupon_boxes.clone()));
    commands.insert_resource(snapshot.shop.clone());
    commands.insert_resource(snapshot.jackpot);
    commands.insert_resource(CompanyLedgers::from_months(snapshot.ledgers.clone()));
    // 可存点上这几台「过程机器」必空（判据在 `judge`）。仍然显式清一遍：
    // 读档走的是「回主菜单 → 再开局」，任何一处漏清都会以「上一局的窗口/拍卖还在」回来
    commands.insert_resource(Gate::default());
    commands.insert_resource(Auction::default());
    commands.insert_resource(crate::magic::MagicRitual::default());
    commands.insert_resource(LastRoll::default());
    commands.insert_resource(crate::landing::Landing::default());
    commands.insert_resource(PendingDecision::default());
    commands.insert_resource(crate::walk::WalkClock::default());
    commands.insert_resource(crate::occupancy::WalkHalt::default());
    commands.insert_resource(crate::minigame::ArcadeDeal::default());
    // 墙钟量不进快照（P14a 那句）：读档后「立刻可以想」，而不是回到存档那一刻的延迟
    commands.insert_resource(crate::ai::AiClock::default());
    commands.insert_resource(crate::ai::AiErrands::default());

    info!(
        "读档完成：第 {} 回合 · {} · {} 玩家 / {} 块地 / 20 条随机流已摆回",
        summary.0, summary.1, summary.2, summary.3
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(phase: TurnPhase) -> SaveFacts {
        SaveFacts {
            phase,
            in_match: true,
            waiting: false,
            gate: false,
            auction: false,
            walking: false,
            human: true,
        }
    }

    /// 可存点判据：五种「正在办事」的形状各拒一条，且理由说得出口。
    /// 这张表是 UI 灰按钮与规则层共用的，所以逐条钉死而不是只断一个 `is_err`。
    #[test]
    fn every_busy_shape_is_refused_with_its_own_words() {
        assert_eq!(judge(facts(TurnPhase::WaitRoll)), Ok(()));
        assert_eq!(judge(facts(TurnPhase::Boot)), Err(SaveDeny::NotInMatch));
        assert_eq!(judge(facts(TurnPhase::Walking)), Err(SaveDeny::NotYourTurn));
        assert_eq!(
            judge(SaveFacts {
                human: false,
                ..facts(TurnPhase::WaitRoll)
            }),
            Err(SaveDeny::NotYourTurn),
            "电脑那一半回合里不许存：它的墙钟与节流不进快照"
        );
        assert_eq!(
            judge(SaveFacts {
                walking: true,
                ..facts(TurnPhase::WaitRoll)
            }),
            Err(SaveDeny::Walking)
        );
        assert_eq!(
            judge(SaveFacts {
                waiting: true,
                ..facts(TurnPhase::WaitRoll)
            }),
            Err(SaveDeny::Waiting)
        );
        assert_eq!(
            judge(SaveFacts {
                gate: true,
                ..facts(TurnPhase::WaitRoll)
            }),
            Err(SaveDeny::GateAsking)
        );
        assert_eq!(
            judge(SaveFacts {
                auction: true,
                ..facts(TurnPhase::WaitRoll)
            }),
            Err(SaveDeny::AuctionRunning)
        );
        // 每一条理由都有一句人话，且互不重复 —— 灰按钮上写同一句话等于没写
        let labels = [
            SaveDeny::NotInMatch,
            SaveDeny::NotYourTurn,
            SaveDeny::Waiting,
            SaveDeny::GateAsking,
            SaveDeny::AuctionRunning,
            SaveDeny::Walking,
        ]
        .map(SaveDeny::label);
        let mut unique: Vec<&str> = labels.to_vec();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "两条拒绝理由说了同一句话");
    }

    /// `Scope::Domains` 那条往返：内容相同的切片读回来必须**仍是那个静态切片**
    /// （不是一块新泄漏出来的），否则每读一次档就漏一次。
    #[test]
    fn the_known_domain_list_round_trips_to_the_static_slice() {
        let scope = Scope::domains(&[ChargeDomain::Rent, ChargeDomain::Penalty]);
        let state: ScopeState = scope.into();
        assert_eq!(scope_of(&state), scope);
        match scope_of(&state) {
            Scope::Domains(list) => assert!(
                std::ptr::eq(list, KNOWN_DOMAIN_LISTS[0]),
                "读档没换回那张静态表，而是另给了一块切片"
            ),
            other => panic!("换回来的不是 Domains：{other:?}"),
        }
    }
}
