//! 《暴富》规则层入口。
//!
//! 实体分四类（B3 起）：土地（`LandId`+`Owner`+`BuildingLevel`+可选 `Mortgaged`）、
//! 玩家（`PlayerId`+`Wallet`+`Points`+`Hand`+`TokenPosition`+`BankBalance`+`StockHoldings`
//! +可选 `Detention`/`StaySentence`/`MoveCap`/`Possession`/`Bankrupt`）、股票（`StockId`+`StockPrice`+`PriceHistory`）、
//! 牌堆（`Deck`，机会/命运各一）。道路格**不建实体**，相机/灯光/UI 在 `game_app`。

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimePlugin;

pub mod afflict;
pub mod ai;
pub mod auction;
pub mod bail;
pub mod bank;
pub mod board;
pub mod calendar;
pub mod calendar_fx;
pub mod card;
pub mod charge;
pub mod company;
pub mod coupon;
pub mod damage;
pub mod data;
pub mod deck;
pub mod economy;
pub mod gods;
pub mod hand;
pub mod institution;
pub mod items;
pub mod land;
pub mod landing;
pub mod loader;
pub mod lottery;
pub mod magic;
pub mod market;
pub mod minigame;
pub mod mortgage;
pub mod msg;
pub mod news;
pub mod occupancy;
pub mod passive;
pub mod player;
pub mod plot;
pub mod property;
pub mod rng;
pub mod rules;
pub mod service;
pub mod settle;
pub mod shop;
pub mod status;
pub mod stock;
pub mod turn;
pub mod villain;
pub mod walk;
pub mod world;

pub mod prelude {
    pub use crate::afflict::{Afflict, Affliction};
    pub use crate::ai::{
        AiCadence, AiClock, AiHand, AiMove, AiProfile, Controller, ai_answer_system,
        ai_roll_system, decide,
    };
    pub use crate::auction::{
        Auction, AuctionDesk, BidDeny, BidFacts, Gavel, LiquidationPlan, Listing, Lot, OpenDeny,
        auction_hold_system, auction_move_system, bid_allowed, bidders, can_open, close_if_done,
        open, record_raise, record_withdraw, seat_order, step_of,
    };
    pub use crate::bail::{
        BailDeny, BailFacts, bail_op_allowed, bail_op_system, branch_place,
        settle_bail_landing_system, someone_held,
    };
    pub use crate::bank::{
        AtmAccess, apply_month_end_interest_system, bank_op_allowed, bank_op_system,
        financing_system, grant_atm_access_system, settle_bank_landing,
    };
    pub use crate::board::{
        BoardDef, BranchDef, BranchKind, BuildingKind, FeeDef, GridPos, LandDef, LandType, TileDef,
        TileIndex, TileKind,
    };
    pub use crate::calendar::{
        Date, EPOCH_YEAR, EpochOverride, HolidaySet, LunarYear, date_of, days_in_month,
        days_until_dividend, is_dividend_day, is_leap_year, is_month_end,
    };
    pub use crate::calendar_fx::deliver_calendar_events_system;
    pub use crate::card::{
        CardDef, CardEffect, CardKind, CardSet, CardTarget, LevyBasis, PassiveCard,
    };
    pub use crate::charge::{
        Adjustment, Charge, ChargeDomain, ChargeKind, Correction, Corrections, Scope, Source,
        charge_domain_of, scales_with_price, stage, transferable,
    };
    pub use crate::company::{
        CompanyLedgers, LedgerEntry, MonthLedger, chairman_of, insurance_claim_system,
        pay_company_dividends, remote_build_system, settle_company_landing, tick_insurance_system,
    };
    pub use crate::coupon::{CouponBoxes, settle_coupon_landing};
    pub use crate::damage::{DamageTally, LandDamage};
    pub use crate::data::{
        BOARD_PATH, CHANCE_PATH, DataHandles, FATE_PATH, ITEMS_PATH, STOCKS_PATH,
    };
    pub use crate::deck::{
        CardId, Deck, DeckOrder, Shelf, apply_effects, card_of, effect_steps, order_of,
        settle_card_landing,
    };
    pub use crate::economy::LandView;
    pub use crate::gods::{
        BestowKit, God, GodDesk, MAX_ON_BOARD, NO_GOD_ON_THE_ROAD, NO_GOD_TO_BANISH, Possession,
        Presence, Rival, STARTING_GODS, WalkEffect, acquire_denied_word, age_possession, attach,
        banishable, cards_lost_per_turn, denies_acquire, detach, doom_days, doom_multiplier,
        drain_cards, extra_levels, god_refuses_banish, grant_effects, land_deity_succession_system,
        money_corrections, nearest_god, presence_of, resists, revoke_effects,
        settle_god_landing_system, settle_land_deity_landing_system, spawn_starting_gods,
        takes_landing, top_up_gods_system, walk_effect, walk_effect_system, wander_gods_system,
    };
    pub use crate::hand::{
        CardAim, Hand, PASSIVE_WORD, PlayWindow, card_notice, deal_to, is_passive, play_card_system,
    };
    pub use crate::institution::{Institution, Suspensions, age_suspensions_system};
    pub use crate::items::{Inventory, ItemKind};
    pub use crate::land::{BuildingLevel, Facility, LandId, Mortgaged, Owner};
    pub use crate::landing::{
        Landed, Landing, LandingDomain, Mover, Pending, PendingDecision, classify_landing_system,
        close_landing_system, decision_system, detain, domain_of, teleport,
    };
    pub use crate::loader::JsonAssetLoader;
    pub use crate::lottery::{
        Jackpot, LotteryDeny, LotteryTickets, accrue_jackpot_interest_system, buy_allowed,
        draw_lottery_system, lottery_op_system, settle_lottery_landing,
    };
    pub use crate::magic::{
        MagicFilter, MagicRitual, MagicSpell, filter_targets, magic_filter_system,
        magic_spell_system, settle_magic_landing,
    };
    pub use crate::market::{
        Halt, Market, MarketClock, market_halt, market_tick_on_turn_end_system,
        settle_market_landing, trade_request_system,
    };
    pub use crate::minigame::{
        ArcadeDeal, GameKind, Payout, Payouts, minigame_score_system, settle_arcade_landing,
    };
    pub use crate::mortgage::{
        MortgageDeny, MortgageFacts, counter_plots, mortgage_op_allowed, mortgage_op_system,
        read_facts,
    };
    pub use crate::msg::{
        AuctionFailed, AuctionMove, AuctionStarted, AuctionWon, BailRequested, BankOp, BankOpDone,
        BankOpRequested, BankVisited, BidMove, BidPlaced, BidWithdrawn, BlameShifted, Blocker,
        BombTransferred, BuildOffered, BuildRequested, BuyOffered, BuyTicketRequested, CardDrawn,
        CardGained, CardLost, CardNotice, CardPlayed, CardSnatched, CashChanged, ChairmanTook,
        ChestOpened, CompanyCharged, CompanyPayout, Decision, DecisionChosen, DepositsRobbed,
        DetentionLifted, DetentionSkipped, DiceRolled, EffectOutcome, FacilityBuilt,
        FinancingDrawRequested, FinancingDrawn, FinancingRecalled, GameOver, GiftGiven, GiftsGiven,
        GodAppeared, GodDeed, GodExit, GodLeft, GodPossessed, IndemnityPaid, ItemPlaced,
        ItemThieved, JackpotChanged, JackpotWhy, LandBought, LandTaken, LandUpgraded,
        LotteryBought, LotteryDrawn, LotteryWon, MagicBlocked, MagicCast, MagicFilterPicked,
        MagicOffered, MagicSpellPicked, MagicTargets, MinigameResult, MinigameScore, MortgageDone,
        MortgageOp, MortgageRequested, MountChanged, MoveCapped, NewsAudience, NewsFired, NewsHit,
        NewsNotice, PassiveAnswer, PassiveMove, PassiveUsed, PlaceItemRequested, PlayCardRequested,
        PlayerArrived, PlayerBailed, PlayerBankrupt, PlayerDetained, PlayerTeleported,
        PointsChanged, PointsStolen, PolicyBought, RedEnvelope, RemoteBuildOffered, RemoteDiceUsed,
        RemoteUpgradeRequested, RentDue, RentRerouted, RequestRemoteDice, RequestRoll, RequestStay,
        ServiceCharged, ShopOpDone, ShopOpRequested, StayLifted, StaySkipped, StockPriceChanged,
        StockTradeRequested, StockTraded, StockVisited, TileStepped, TollExtorted, TradeSide,
        TurnEnded, TurnStarted, UpgradeOffered, VillainHired, WalkHalted, WalkStarted,
    };
    pub use crate::news::{
        FieldFilter, NewsDef, NewsId, NewsScope, NewsSet, NewsSink, PlayerPick, find_id, fire_news,
        news_of, roll_span, settle_news_landing_system,
    };
    pub use crate::occupancy::{
        BOMB_HOSPITAL_DAYS, BOMB_STEPS, HaltCause, MINE_HOSPITAL_DAYS, Occupancy, OccupancyAt,
        OccupancyKind, PlaceDeny, STARTING_CHESTS, Trigger, WalkHalt, finish_halt_system,
        first_roadblock, place, place_denied, place_item_system, resolve_halt_system,
        settle_landing_occupancy_system, spawn_starting_chests, trigger_of, within_chebyshev,
    };
    pub use crate::passive::{
        Affair, Ask, Gate, GateDesk, Job, PassiveEvent, Step, Tail, affair_of_card,
        blame_candidates, gate_system, submit, trip_of,
    };
    pub use crate::player::{
        BankBalance, Bankrupt, Detention, DetentionPlace, GameSetup, Gender, Heading, Insured,
        Mount, MoveCap, PLAYER_COLORS, PlayerColor, PlayerId, PlayerName, PlayerSetup, Points,
        StaySentence, Term, TokenPosition, WalkQueue, Walker, Wallet,
    };
    pub use crate::plot::{PlotArea, area_of, is_in_area};
    pub use crate::property::{
        BuildDeny, build_op_allowed, build_op_system, settle_property_landing,
    };
    pub use crate::rng::{DIE_FACES, GameSeed, Rng};
    pub use crate::rules::GameRules;
    pub use crate::service::{FeePrototype, ServiceBill, assess as assess_service};
    pub use crate::settle::{Books, InsufficientFunds};
    pub use crate::shop::{
        ShopDeny, ShopStock, buyback, chairman_freebie_system, restock, settle_shop_landing,
        shop_op_allowed, shop_op_system,
    };
    pub use crate::status::{LandStatus, Timed, expire_land_status_system};
    pub use crate::stock::{PriceHistory, StockDef, StockHoldings, StockId, StockPrice, StockSet};
    pub use crate::turn::{GameSession, LastRoll, Paused, TurnOrder, TurnPhase, not_paused};
    pub use crate::villain::{
        Hired, Villain, VillainKind, custody_of, has_door, hoodlum_toll_system, on_hired_system,
        robber_snatch_system, spawn_caged_villains, thief_deliver_system, villain_rotation_system,
        villain_turn_system,
    };
    pub use crate::walk::{
        WalkClock, WalkPlan, advance_walk_clock_system, plan_walk, walk_step_system,
    };
    pub use crate::world::{ActiveBoard, CoreSet, Gameplay, begin_game_system};
}

use bank::apply_month_end_interest_system;
use prelude::{
    ActiveBoard, ArcadeDeal, AtmAccess, AuctionFailed, AuctionMove, AuctionStarted, AuctionWon,
    BailRequested, BankOpDone, BankOpRequested, BankVisited, BidPlaced, BidWithdrawn, BlameShifted,
    BoardDef, BombTransferred, BuildOffered, BuildRequested, BuyOffered, BuyTicketRequested,
    CardDrawn, CardGained, CardLost, CardPlayed, CardSet, CardSnatched, CashChanged, ChairmanTook,
    ChestOpened, CompanyCharged, CompanyPayout, Corrections, CouponBoxes, DecisionChosen,
    DepositsRobbed, DetentionLifted, DetentionSkipped, DiceRolled, EpochOverride, FacilityBuilt,
    FinancingDrawRequested, FinancingDrawn, FinancingRecalled, GameOver, GameRules, GameSeed,
    GameSetup, GiftsGiven, GodAppeared, GodDeed, GodLeft, GodPossessed, HolidaySet, IndemnityPaid,
    ItemPlaced, ItemThieved, JackpotChanged, JsonAssetLoader, LandBought, LandTaken, LandUpgraded,
    Landing, LotteryBought, LotteryDrawn, LotteryWon, MagicBlocked, MagicCast, MagicFilterPicked,
    MagicOffered, MagicSpellPicked, MagicTargets, MarketClock, MinigameResult, MinigameScore,
    MortgageDone, MortgageRequested, MountChanged, MoveCapped, NewsFired, PassiveAnswer,
    PassiveUsed, Paused, PendingDecision, PlaceItemRequested, PlayCardRequested, PlayerArrived,
    PlayerBailed, PlayerBankrupt, PlayerDetained, PlayerTeleported, PointsChanged, PointsStolen,
    PolicyBought, RedEnvelope, RemoteBuildOffered, RemoteDiceUsed, RemoteUpgradeRequested, RentDue,
    RentRerouted, RequestRemoteDice, RequestRoll, RequestStay, ServiceCharged, ShopOpDone,
    ShopOpRequested, ShopStock, StayLifted, StaySkipped, StockPriceChanged, StockSet,
    StockTradeRequested, StockTraded, StockVisited, TileStepped, TollExtorted, TurnEnded,
    TurnPhase, TurnStarted, UpgradeOffered, VillainHired, WalkClock, WalkHalt, WalkHalted,
    WalkStarted, accrue_jackpot_interest_system, advance_walk_clock_system, age_suspensions_system,
    auction_hold_system, auction_move_system, bail_op_system, bank_op_system, begin_game_system,
    build_op_system, decision_system, deliver_calendar_events_system, draw_lottery_system,
    expire_land_status_system, financing_system, finish_halt_system, gate_system,
    grant_atm_access_system, hoodlum_toll_system, insurance_claim_system,
    land_deity_succession_system, lottery_op_system, magic_filter_system, magic_spell_system,
    market_tick_on_turn_end_system, minigame_score_system, mortgage_op_system, on_hired_system,
    place_item_system, play_card_system, remote_build_system, resolve_halt_system,
    robber_snatch_system, settle_arcade_landing, settle_god_landing_system,
    settle_landing_occupancy_system, shop_op_system, thief_deliver_system, tick_insurance_system,
    top_up_gods_system, trade_request_system, villain_rotation_system, villain_turn_system,
    walk_effect_system, walk_step_system, wander_gods_system,
};
use turn::{
    LastRoll, begin_turn_system, end_turn_system, game_over_system, not_paused, remote_dice_system,
    roll_dice_system, take_stay_system, update_price_index_system,
};
use world::CoreSet;

/// 规则层 Plugin：注册数据资产与其 JSON loader、对局 resource、回合消息与状态机。
/// 只链接 bevy 的无渲染特性子集（`default_app` + `bevy_time`），因此 `cargo test -p game_core`
/// 不需要窗口。
pub struct CoreGamePlugin;

impl Plugin for CoreGamePlugin {
    fn build(&self, app: &mut App) {
        // 规则层用了 `States`，就得保证 `StateTransition` 调度存在。真实客户端经 `DefaultPlugins`
        // 已有；headless 测试里没有，所以自己补 —— 但 `StatesPlugin` 的 `insert_after` 不幂等，
        // 加两遍会让状态转移每帧跑两次，因此必须先查。
        if !app.is_plugin_added::<StatesPlugin>() {
            app.add_plugins(StatesPlugin);
        }
        // 行走时钟要读 `Res<Time>`（`advance_walk_clock_system`）；同理补齐且防重复
        if !app.is_plugin_added::<TimePlugin>() {
            app.add_plugins(TimePlugin);
        }

        app.init_asset::<BoardDef>()
            .init_asset::<CardSet>()
            .init_asset::<StockSet>()
            .init_asset::<HolidaySet>()
            .init_asset::<crate::news::NewsSet>()
            .register_asset_loader(JsonAssetLoader::<BoardDef>::default())
            .register_asset_loader(JsonAssetLoader::<CardSet>::default())
            .register_asset_loader(JsonAssetLoader::<StockSet>::default())
            .register_asset_loader(JsonAssetLoader::<HolidaySet>::default())
            .register_asset_loader(JsonAssetLoader::<crate::news::NewsSet>::default())
            // 对局配置有默认值（prototype 双人阵容 + 可复现种子），
            // 表现层要换阵容/换种子就在 Startup 前 insert_resource 覆盖
            .init_resource::<GameRules>()
            .init_resource::<GameSetup>()
            .init_resource::<GameSeed>()
            .init_resource::<EpochOverride>()
            .init_resource::<HolidaySet>()
            .init_resource::<Paused>()
            .init_resource::<crate::magic::MagicRitual>()
            .init_resource::<WalkClock>()
            .init_resource::<MarketClock>()
            // AI 的那两条钟（P14）。`AiCadence` 是**配置**：core 的默认是两个 0，真实客户端
            // 在启动时换成 0.5..=1.5（无头测试里 `Time::elapsed_secs()` 不前进，任何正数都会
            // 把 AI 钉死在第一帧不动，而那个症状与「AI 没装配」一模一样）。
            // `AiClock` 是**这一局的状态**，所以装配时归零而不是只 init（见 `world.rs`）。
            .init_resource::<crate::ai::AiCadence>()
            .init_resource::<crate::ai::AiClock>()
            .init_resource::<Landing>()
            .init_resource::<PendingDecision>()
            .init_resource::<AtmAccess>()
            .init_resource::<CouponBoxes>()
            .init_resource::<Corrections>()
            .init_resource::<ShopStock>()
            .init_resource::<ArcadeDeal>()
            .init_resource::<lottery::Jackpot>()
            .init_resource::<auction::Auction>()
            // 被动卡那道门（P13b②）。它带 `Books`，所以装配前的那些帧与拍卖同一条处理
            // （门禁在 system 上，资源本身常驻 —— `Default` 就是一条空队列）
            .init_resource::<passive::Gate>()
            .init_resource::<company::CompanyLedgers>()
            .init_resource::<LastRoll>()
            .init_resource::<WalkHalt>()
            // ===== 流程与移动 =====
            .add_message::<TurnStarted>()
            .add_message::<RequestRoll>()
            .add_message::<RequestStay>()
            .add_message::<DiceRolled>()
            .add_message::<WalkStarted>()
            .add_message::<TileStepped>()
            .add_message::<PlayerArrived>()
            .add_message::<TurnEnded>()
            .add_message::<DetentionLifted>()
            .add_message::<PlayerDetained>()
            .add_message::<PlayerTeleported>()
            .add_message::<DetentionSkipped>()
            // 停留卡的两条（P3d④c）。`begin_turn_system` 的 `TurnWriters` 现在带着它们的
            // writer —— 漏注册不是少一行文案，而是那个 SystemParam 建不出来、
            // **回合机整个不跑**（同一个坑见上面那条注）
            .add_message::<StaySkipped>()
            .add_message::<StayLifted>()
            .add_message::<GameOver>()
            // ===== 地产与经济（B3）=====
            .add_message::<CashChanged>()
            .add_message::<PointsChanged>()
            .add_message::<PlayerBankrupt>()
            .add_message::<BuyOffered>()
            .add_message::<UpgradeOffered>()
            .add_message::<LandBought>()
            // 土地公的过户（P6c）。与 `LandBought` 分两条：那条说「有人付了这笔钱」，这条没有
            .add_message::<LandTaken>()
            .add_message::<LandUpgraded>()
            .add_message::<BuildOffered>()
            .add_message::<BuildRequested>()
            .add_message::<FacilityBuilt>()
            .add_message::<ServiceCharged>()
            // 公司账的两条凭据（P10d①）：收费（进损益账）与分红（出损益账）
            .add_message::<CompanyCharged>()
            .add_message::<CompanyPayout>()
            // P10d② 的五条：投保与赔付（保险格）、白拿一件（百货董事长）、
            // 远程建设窗口（日产）、融资（银行保险董事长）。漏注册的症状同上 ——
            // 带着 writer 的参数包建不出来，整条链不跑
            .add_message::<PolicyBought>()
            .add_message::<IndemnityPaid>()
            .add_message::<ChairmanTook>()
            .add_message::<RemoteBuildOffered>()
            .add_message::<RemoteUpgradeRequested>()
            .add_message::<FinancingDrawn>()
            .add_message::<FinancingRecalled>()
            .add_message::<FinancingDrawRequested>()
            .add_message::<RentDue>()
            .add_message::<BankVisited>()
            .add_message::<BankOpRequested>()
            .add_message::<BankOpDone>()
            .add_message::<StockVisited>()
            .add_message::<StockTradeRequested>()
            .add_message::<StockTraded>()
            .add_message::<StockPriceChanged>()
            // ===== 卡牌与决策（B3）=====
            .add_message::<CardDrawn>()
            .add_message::<DecisionChosen>()
            // ===== 手牌（P3a）=====
            .add_message::<PlayCardRequested>()
            .add_message::<CardPlayed>()
            .add_message::<CardGained>()
            // ===== 百货公司（P3d②）=====
            // 漏一条就是 `Books` 那批参数整局不跑（症状见 ARCHITECTURE §六 的消息表注）
            .add_message::<ShopOpRequested>()
            .add_message::<ShopOpDone>()
            // ===== 游乐场（P7）=====
            // 成绩报上来那一条与结算完那一条。漏注册第一条的症状不是「少一行流水」，
            // 而是 `minigame_score_system` 的参数校验直接失败 —— 回合机整个不跑
            .add_message::<MinigameResult>()
            .add_message::<MinigameScore>()
            // ===== 乐透（P8）=====
            // 一条请求 + 三条凭据 + 一条奖池流水。漏任何一条不是「少一行字」，
            // 而是 `lottery_op_system` / `draw_lottery_system` 的参数建不出来 —— 整批 system 不跑
            .add_message::<BuyTicketRequested>()
            .add_message::<LotteryBought>()
            .add_message::<LotteryDrawn>()
            .add_message::<LotteryWon>()
            .add_message::<JackpotChanged>()
            // ===== 拍卖（P9）=====
            // 一条意图（`AuctionMove`，两个动作共用它）+ 一条开场 + 四条结果。
            // 漏注册任何一条都不是「少一行字」，而是相应 system 的参数建不出来 ——
            // 整条拍卖机不跑，回合停在「等着问下一个人」的地方
            .add_message::<AuctionMove>()
            .add_message::<AuctionStarted>()
            .add_message::<BidPlaced>()
            .add_message::<BidWithdrawn>()
            .add_message::<AuctionWon>()
            .add_message::<AuctionFailed>()
            // ===== 日历事件（P2b）=====
            .add_message::<GiftsGiven>()
            .add_message::<RedEnvelope>()
            // ===== 新闻（P11）=====
            // 漏这一行的症状与上面每一条同形：`NewsSink` 的 writer 建不出来，
            // 落格那条链在踩到新闻中心的一帧整批不跑
            .add_message::<NewsFired>()
            // ===== 保释（P4③）=====
            .add_message::<BailRequested>()
            .add_message::<PlayerBailed>()
            // ===== 四大恶人（P12b–d）=====
            .add_message::<VillainHired>()
            .add_message::<DepositsRobbed>()
            .add_message::<PointsStolen>()
            .add_message::<ItemThieved>()
            .add_message::<CardSnatched>()
            .add_message::<TollExtorted>()
            .add_message::<RentRerouted>()
            // ===== 魔法屋（P13）=====
            .add_message::<MagicOffered>()
            .add_message::<MagicFilterPicked>()
            .add_message::<MagicTargets>()
            .add_message::<MagicSpellPicked>()
            .add_message::<MagicCast>()
            .add_message::<MagicBlocked>()
            // ===== 抵押与赎回（P3d③）=====
            .add_message::<MortgageRequested>()
            .add_message::<MortgageDone>()
            // ===== 道路放置物与道具（P5）=====
            .add_message::<PlaceItemRequested>()
            .add_message::<ItemPlaced>()
            .add_message::<WalkHalted>()
            .add_message::<BombTransferred>()
            .add_message::<ChestOpened>()
            .add_message::<RequestRemoteDice>()
            .add_message::<RemoteDiceUsed>()
            .add_message::<MoveCapped>()
            .add_message::<MountChanged>()
            // ===== 神明（P6）=====
            // `GodExit` 不注册：它只是 `GodLeft` 的一个字段，没有独立的读者
            .add_message::<GodAppeared>()
            .add_message::<GodPossessed>()
            .add_message::<GodLeft>()
            // 上身那一刻的一次性效果（P6b）。`GodDesk` 带着它的 writer，
            // 漏注册 = 那个参数包建不出来 = 踩到神的那一刻整个 `Landed` 链不跑
            .add_message::<GodDeed>()
            // 衰神每回合弄走的那张（P6b②）。它挂在 `TurnWriters` 上 ——
            // 漏这一行不是少一条流水，是**回合机整个不跑**（本文件上面那条坑注）
            .add_message::<CardLost>()
            // ===== 被动卡那道门（P13b②）=====
            // 三条都要注册：`PassiveAnswer` 是人的那一句（`gate_system` 的 reader），
            // 另两条是门答完发的凭据，挂在 `GateDesk` 的 writer 上 —— 漏哪一条，
            // 那一刻被问到的那个窗口就整段不跑（与上面 `GodDeed` 同一个形状）
            .add_message::<PassiveAnswer>()
            .add_message::<PassiveUsed>()
            .add_message::<BlameShifted>()
            .init_state::<TurnPhase>()
            // 装配世界：初始态的 OnEnter 不会触发，所以用 in_state(Boot)
            .add_systems(
                Update,
                begin_game_system
                    .in_set(CoreSet::Boot)
                    .run_if(in_state(TurnPhase::Boot)),
            )
            // 日历事件排在回合开始之前：12/25 的礼物与正月初一的红包要在
            // 「那一天的第一个回合」动手之前发完，且不挂 `Pending`（不挡回合机）
            .add_systems(
                OnEnter(TurnPhase::TurnStart),
                // 保单效期扣表排在回合开始之后：begin_turn 可能刚把这位按成「被跳过」，
                // 而被拘留的回合照旧是一天 —— 保单与拘留同一把尺（company.rs 的注）
                (
                    deliver_calendar_events_system,
                    begin_turn_system,
                    tick_insurance_system,
                )
                    .chain(),
            )
            // 保险赔付（P10d②）：`PlayerDetained` 的读者之一，消息下一帧才投递（坑 12），
            // 拘留那一帧与赔付差一帧没有判据在乎。带 `Books`，所以要 `resource_exists`
            .add_systems(
                Update,
                insurance_claim_system.run_if(resource_exists::<ActiveBoard>),
            )
            // 土地公期满 → 恶犬接位（P6）。挂在 `Update` 而不是 `OnEnter(TurnStart)`：
            // 它是**消息的后果**，而消息要到下一帧才投递 —— 放进回合开始那一拍会让恶犬
            // 晚整整一回合才出现在路上，而那件事没有任何判据需要它等。
            // 它带 `Res<ActiveBoard>`（要找环长），而装配之前那个资源还不存在 ——
            // 漏掉这道门就是每帧一次参数校验失败（与本文件上面 `bank_op_system` 那条注同族）
            .add_systems(
                Update,
                land_deity_succession_system.run_if(resource_exists::<ActiveBoard>),
            )
            .add_systems(
                Update,
                (
                    roll_dice_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    // 停留（P4④）：与掷骰同一个窗口、同一条资格，只认当前回合玩家
                    take_stay_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    // 恶人的回合（P12a）：与玩家、AI 发**同一条意图**（`RequestRoll`/`RequestStay`），
                    // 所以它与上面两条同一个窗口、同一个闸门。它排在两条之后没有语义 ——
                    // 消息要到下一帧才投递（坑 12），这里的位置只是为了读的时候一眼看出
                    // 「掷骰、停留、恶人自发意图」是同一个窗口的三个入口
                    villain_turn_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    walk_step_system
                        .run_if(in_state(TurnPhase::Walking))
                        .run_if(not_paused),
                    // 成绩结算（P7）与 `decision_system` 同一条窗口、同一份账本（都带 `Books`），
                    // 所以显式 `chain` 而不是并排放进元组：两条并行会直接撞成调度期借用冲突。
                    // 彩票柜台也在这条链上（P8）：它同样带 `Books`，并行会撞成调度期借用冲突。
                    //
                    // **`lottery_op_system` 必须排在 `decision_system` 之前**，这一条不是风格：
                    // 自动玩家（`NM_AUTOROLL`）在同一帧里既发「买一注」又发「离开」，而
                    // `decision_system` 会把窗口关掉、`lottery_op_system` 认的就是「窗口还开着吗」——
                    // 反过来写，那一注会被它自己的「离开」挡在门外，而症状是奖池永远为 0。
                    // 真人分两帧点，两种顺序都不受影响，所以只有长跑读得出这个差别。
                    (lottery_op_system, decision_system, minigame_score_system)
                        .chain()
                        .run_if(in_state(TurnPhase::WaitDecision))
                        .run_if(not_paused),
                    // 拍卖机（P9）+ 出牌与被动卡那道门（P3d、P13b②）—— 三条合成**一条链**。
                    //
                    // 拍卖要排在 `decision_system` **之后**是它自己要求的：反过来的那一帧，
                    // `decision_system` 会因为「上一个窗口被回答了」把回合推向 `TurnEnd`，
                    // 而拍卖机还在等下一个人喊 —— 症状是拍卖停在半空，面板上谁都不该被问、
                    // 却谁都不动。
                    //
                    // 它自己那一组内部也是链：`auction_move_system` 办完动作、落下槌，
                    // `auction_hold_system` 才知道该散场还是该接着开下一块 —— 一处清算有三块地，
                    // 「散场同帧接着开下一场」是常态而不是边界。
                    //
                    // P13b② 把出牌与门并进同一条链，理由是拍卖那一句的延长线：**这三条都会
                    // 在同一帧里写回合机**（`NextState<TurnPhase>`），而元组元素之间的顺序
                    // 根本没有保证（§6.1 坑 36 —— 把 system 挪个位置不是一次有效破坏）。
                    // 一张拍卖卡在 `WaitRoll` 打出，而它若同时把一件灾落到别人头上，门在同帧
                    // 就要把回合押住；反过来拍卖正开场时，交回合的应当是拍卖而不是门。
                    // 谁先写、谁后写不能靠赌。
                    //
                    // 出牌排第一：它要么自己当场把这一件办完（`submit` 里的 `pump`），要么只是
                    // 排队，门在它之后跑，于是这一帧新塞进去的信封不必等下一帧。
                    //
                    // 这道门与上面那组不同：**不能**整条 gate 在 `WaitDecision` 上 —— 出牌只在
                    // 掷骰前、门只在有人该答一句时动，而拍卖必须在任何阶段都转得起来（破产发生
                    // 在落格结算里）。所以条件留各条 system 自己身上，组级只留 `not_paused` 与
                    // `resource_exists::<ActiveBoard>`（三条都带 `Books`，装配之前那个资源还没有，
                    // 与 `land_deity_succession_system` 同一条理由）。
                    (
                        play_card_system.run_if(in_state(TurnPhase::WaitRoll)),
                        auction_move_system,
                        auction_hold_system,
                        gate_system.run_if(in_state(TurnPhase::WaitDecision)),
                    )
                        .chain()
                        .run_if(not_paused)
                        .run_if(resource_exists::<ActiveBoard>),
                    // 证券交易的窗口就是「还没掷骰」这一段，与掷骰同态、同闸门
                    trade_request_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    // 放置道具与掷骰同一个窗口：路障要挡的是「接下来这几步」，所以
                    // 必须在我自己走之前放下去（ADR-002 §4）
                    place_item_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    // 遥控骰子：与掷骰互斥地决定这一回合走几格。谁先受理由消息定，
                    // 同一帧两条时规则层各认各的窗口，不改回合状态
                    remote_dice_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                ),
            )
            // ATM 资格与银行操作不挂状态：玩家可能在行走途中、也可能在落格之后按。
            // 谁能办哪一种由 `bank_op_system` 自己判（AtmAccess / 柜台），
            // 阶段门禁挡不住「乙替甲办」这类越权，认人才是有效判据。
            // `grant_atm_access_system` 反过来不闸暂停：它只是记账，而暂停期间根本没有新
            // 的 `TileStepped` 产生 —— 给它加闸门只会让暂停那一帧的路上银行格被漏记。
            // `bank_op_system` 要额外等棋盘就位：它带 `Books`（里面是 `Res<ActiveBoard>`），
            // 而装配前的那些帧还没有这个资源 —— 别的 `Books` 用户都躲在状态门禁后面，
            // 这一只没有，所以漏一条就是每帧一次「Resource does not exist」。
            .add_systems(Update, grant_atm_access_system)
            // 天使与恶魔：同一条 `TileStepped` 的 core 侧 reader，注册形状照上面那条 ATM
            // （不设阶段闸门，理由见 `walk_effect_system` 的注）。它带 `Books`，
            // 所以要 `resource_exists` 挡装配前与回菜单之后的那些帧。
            // 恶人行为的三条 reader（P12b/c）与它同住这一组：`hoodlum_toll_system`
            // 也带 `Books`（两条对 `&mut Wallet` 的访问冲突是调度期硬检查的），
            // 所以这一组显式 `chain` —— 勒索与神明的脚步互不相干，chain 只是让
            // 「同一帧读到同一条 TileStepped」的两条 reader 都跑得到。
            // `robber_snatch_system`/`thief_deliver_system` 不带 `Books`
            // （前者动的是手牌与随机流，后者是道具栏），本来可以并行，
            // 但它们读的消息与上面两条同源，串成一条链读起来一眼看出「走路的事在这里」。
            // `on_hired_system` 读的是 `VillainHired`（保释窗口发的，下一帧才到），
            // 与这条链先后无语义，放在一起是因为它同样是「恶人的事」。
            .add_systems(
                Update,
                (
                    walk_effect_system.run_if(resource_exists::<ActiveBoard>),
                    hoodlum_toll_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // `robber_snatch_system` 读 `Rng`（装配时才插入）、`thief_deliver_system`
                    // 读道具栏 —— 门禁与上面两条同款：装配前世界不存在，恶人也不存在
                    robber_snatch_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    thief_deliver_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    on_hired_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    bank_op_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // 与银行同一条窗口（都带 `Books`，里面是 `&mut Points`），所以显式
                    // `chain` 而不是并排放进一个元组：两条并行会直接撞成调度期借用冲突。
                    // 谁先办都不改结果 —— 它们读的是同一个「此刻站在哪个柜台前」。
                    shop_op_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // 抵押柜台与银行柜台是同一个窗口、同一批账（都带 `Books`），
                    // 所以三条一起 `chain`。它与 `bank_op_system` 的先后不改结果：
                    // 两者读的都是「此刻站在哪个柜台前」，而谁都不动回合状态。
                    mortgage_op_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // 保释与银行/百货/抵押同一条窗口形状（都带 `Books` 里的 `&mut Points`），
                    // 所以一起 `chain`：谁先办都不改结果，只是不能并行借同一份账本
                    bail_op_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // 建设柜台（P10c）：与上面四条同一族（都带 `Books`，都读「此刻站在哪个
                    // 柜台前」，都不动回合状态）。办完一条建造窗口照开，玩家按「先不建」
                    // 才由 `decision_system` 交还回合 —— 与柜台「办完才关窗」同一形
                    build_op_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // 融资三件套（P10d②）与远程升级柜台（P10d②）：同一条 chain 的尾巴。
                    // 融资必须排在 `bank_op_system` **之后**（储备透支是取款写下的，
                    // 兜底要读到取款之后的储备）；远程升级与建设柜台同形（窗口照开、
                    // 请求逐笔办，`decision_system` 对它只认「先不建」那一句交还）
                    financing_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    remote_build_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    // 魔法屋的两步回答（P13）：条件与法术各一条意图消息，规则层判窗口
                    // 与主语。两条都带 CardSink（里面是 Books），所以接在同一条 chain 的尾部
                    // —— 与保释/柜台那族同一条纪律：谁先办都不改结果，只是不能并行借同一份账本。
                    magic_filter_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                    magic_spell_system
                        .run_if(resource_exists::<ActiveBoard>)
                        .run_if(not_paused),
                )
                    .chain(),
            )
            // 时钟在 First 推进、Update 消费：同帧内先累计、后定步
            .add_systems(
                First,
                advance_walk_clock_system
                    .run_if(in_state(TurnPhase::Walking))
                    .run_if(not_paused),
            )
            // 落格流水线：解释一次格子 → 六个域各自结算 → 由 pending 决定去哪一阶段。
            // 顺序写成显式 chain：域的先后决定同帧消息的次序，靠隐式冲突排序说出来不算数。
            // 哪个格归哪个域，只在 `landing::domain_of` 那张穷尽表里说一次。
            .add_systems(
                OnEnter(TurnPhase::Landed),
                // 14 条是 Bevy 元组 chain 的上限（15 起 IntoIterator 没了）：前段是
                // 「先解释、再按格域当场算钱」的老骨干，后段是「窗口类」—— 分界正好
                // 也是「报价/扣款」与「挂窗口」的分界；收尾那条在下一个 add_systems 接链尾
                (
                    landing::classify_landing_system,
                    // 落脚触发的放置物（宝箱）。读 `PlayerArrived.to`，所以中断那一格
                    // 不会误触发（ADR-002 §7）；它与下面各域互不相交，排第一只是先给出道具
                    settle_landing_occupancy_system,
                    // 落脚踩到路上的神 → 上身（P6）。排在宝箱那条之后而不是合并进去：
                    // 两者都读 `PlayerArrived`，但读者各有游标、互不吃消息，而附身要做的事
                    // （顶掉旧神、挂修正、腾对位）比「给一件道具」重一个数量级
                    settle_god_landing_system,
                    property::settle_property_landing,
                    // 土地公：踩到的地直接归他。**排在地产结算之后**是那句「过路费照付原主」
                    // 的实现 —— 先照原样把这一趟的租收掉，再过户（理由见那个函数的注）
                    gods::settle_land_deity_landing_system,
                    bank::settle_bank_landing,
                    market::settle_market_landing,
                    deck::settle_card_landing,
                    coupon::settle_coupon_landing,
                    shop::settle_shop_landing,
                    // 游乐场（P7）：定下这一趟玩哪一款、发一手布局，再把窗口挂上。
                    // 排在百货之后、保释之前 —— 它与前六个域互不相交（读的是格子数据与
                    // 随机流），而保释那一条要在所有结算之后才问「要不要放人」
                    settle_arcade_landing,
                    // 彩票中心（P8）：挂上买券窗口。与游乐场同一类「窗口而不是报价」，
                    // 所以也排在保释之前
                    lottery::settle_lottery_landing,
                    // 公司门面（P10d①）：服务费当场结算进损益账、B 型送人出国。
                    // 排在彩票之后、保释之前 —— 与游乐场/彩票同一类「当场办完的事」，
                    // 而保释那一条要在所有结算之后才问「要不要放人」
                    company::settle_company_landing,
                    // 拘留场所接入点：踩上去开保释窗口（P4③）。排在最后一块结算之后 ——
                    // 演示盘的接入点同时是地产格，先办完买地/付租再问要不要放人
                    bail::settle_bail_landing_system,
                )
                    .chain(),
            )
            // 收尾那条单独挂：上面的 14 元组已顶满 Bevy 元组 chain 的上限（15 起
            // IntoIterator 就没了），而它必须等全部域结算完才能关这一趟 —— 用
            // `.after` 显式接在链尾，与写在同一个元组里是同一条顺序约束
            .add_systems(
                OnEnter(TurnPhase::Landed),
                landing::close_landing_system.after(bail::settle_bail_landing_system),
            )
            // 新闻中心（P11）：抽一条、当场播完并结算。**排在全部域结算之后、关这一趟之前**
            // 是因为新闻动的正是上面那些域管着的东西（钱、归属、身上的计时）——
            // 它必须读到「这一趟已经结完」的世界，否则同一帧里地产租金与新闻课税会各按
            // 一半的账本走。14 元组已顶满上限，与上面两条同一手法用 `.after`/`.before` 接链
            // 魔法屋（P13）：开两步窗口的第一步，同样排在全部域结算之后 ——
            // 它要读的是「这一趟已经结完」的世界（脚下地、名下账）
            .add_systems(
                OnEnter(TurnPhase::Landed),
                magic::settle_magic_landing
                    .after(bail::settle_bail_landing_system)
                    .before(landing::close_landing_system),
            )
            .add_systems(
                OnEnter(TurnPhase::Landed),
                news::settle_news_landing_system
                    .after(magic::settle_magic_landing)
                    .before(landing::close_landing_system),
            )
            // 董事长白拿一件（P10d②）：接在 `settle_shop_landing` 之后 —— 货架刚
            // 重抽完、`Pending::Shop` 刚挂上，白拿拿的才是「这一趟的货」。14 元组
            // 已顶满上限，所以同白拿收尾那条一样用 `.after` 显式接链
            .add_systems(
                OnEnter(TurnPhase::Landed),
                shop::chairman_freebie_system
                    .after(shop::settle_shop_landing)
                    .before(landing::close_landing_system),
            )
            // 走步被放置物打断：先应用后果（住院/拆房），再收尾。中断那一格不走
            // 落格分派 —— 它不是到站（ADR-002 §7）
            .add_systems(
                OnEnter(TurnPhase::Halted),
                (resolve_halt_system, finish_halt_system).chain(),
            )
            // 回合结束 = 一天过去：行情 tick 与回合轮转同批、互不冲突（动的组件不相交），
            // 所以不排顺序 —— 谁先跑都不改结果。后面两跳都读「今天几号」，而 tick 就是
            // 在行情那一步推进的，所以必须排在它后面：
            // 月末结息（改余额）→ 物价指数（分子里有余额）。
            .add_systems(
                OnEnter(TurnPhase::TurnEnd),
                (
                    (
                        end_turn_system,
                        market_tick_on_turn_end_system,
                        // 限时状态（涨价 / 查封）与日历同一个节拍过一天：「5 天」必须是
                        // 五天，不能是「五个行情 tick 加半个回合」。动的组件与上面两条
                        // 都不相交，所以放进这组并行元组里，谁先跑都不改结果
                        expire_land_status_system,
                    ),
                    // 机构停摆过一天（P11c②）。**单独一个阶段**，两个理由叠在一起：
                    // ① 它碰 `GameSession`，而上面那条行情 tick 经 `Books` 也碰 ——
                    // 同阶段并跑会在调度期撞成借用冲突（§6.1 坑 33）；
                    // ② 次序有语义：必须在 tick **之后**减，否则「停牌 1 天」在当天
                    // tick 之前就成了 0，那一天照常开盘 —— 少挡的正是它要挡的那一天
                    age_suspensions_system,
                    // 恶人与轮转的那道同步（P12a）。**必须排在 `end_turn_system` 之后**：
                    // 那条刚把游标推到下一位，这一条才摘掉用完回合的那一位 —— 反过来写，
                    // 摘除会把游标一起往前带，症状是有人的回合凭空被跳过一次。
                    // 与坑 36/37 同一条纪律：这种顺序只能由 `chain` 的元素位置说出来。
                    villain_rotation_system,
                    // 神明过一天：先各自挪一格，再补刷到名额。这两条都要 `&mut Rng`，
                    // 而上面那一组里的行情 tick 也要 —— 所以它们自己成一阶段，
                    // 不能塞进那个并行元组（调度期直接撞成借用冲突）
                    (wander_gods_system, top_up_gods_system).chain(),
                    apply_month_end_interest_system,
                    // 乐透那两条都读「今天几号」，所以必须在行情 tick 之后（与结息同一条理由）；
                    // 而发奖是**给某个人的钱包加钱**，会进 `total_assets` 的分子，
                    // 所以又必须排在物价指数之前 —— 与月末结息同一个位置、同一条纪律
                    accrue_jackpot_interest_system,
                    draw_lottery_system,
                    update_price_index_system,
                )
                    .chain(),
            )
            .add_systems(OnEnter(TurnPhase::Over), game_over_system)
            // ===== AI 玩家（P14a）=====
            // 两条都挂在 **PreUpdate**，而且不是「排在 `decision_system` 之前」那种隐式顺序：
            // 与测试夹具那份 `auto_answer_system` 同一条理由（§6.1 坑 36）—— 并行元组里
            // 谁先跑没人保证，于是「这一帧发的回答当帧读不读得到」变成帧相关的。写在
            // `PreUpdate` 之后，任何 `Update` 里的读者都在**同一帧**读得到，而拍卖与门那
            // 两台机器每帧只吃一个动作，这条纪律不是可选的。
            //
            // `not_paused` 那道门是给 AI 的：少了它，暂停菜单开着的时候电脑照样掷骰、
            // 照样答门，人类那一侧全停而 AI 那一侧全走 —— 玩家会以为游戏自己动起来了。
            .add_systems(
                PreUpdate,
                (
                    ai::ai_roll_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    ai::ai_answer_system
                        .run_if(in_state(TurnPhase::WaitDecision))
                        .run_if(not_paused),
                ),
            );
    }
}
