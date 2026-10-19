//! 《暴富》规则层入口。
//!
//! 实体分四类（B3 起）：土地（`LandId`+`Owner`+`BuildingLevel`+可选 `Mortgaged`）、
//! 玩家（`PlayerId`+`Wallet`+`Points`+`Hand`+`TokenPosition`+`BankBalance`+`StockHoldings`
//! +可选 `JailSentence`/`Insulation`/`Bankrupt`）、股票（`StockId`+`StockPrice`+`PriceHistory`）、
//! 牌堆（`Deck`，机会/命运各一）。道路格**不建实体**，相机/灯光/UI 在 `game_app`。

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimePlugin;

pub mod afflict;
pub mod bank;
pub mod board;
pub mod card;
pub mod charge;
pub mod coupon;
pub mod damage;
pub mod data;
pub mod deck;
pub mod economy;
pub mod hand;
pub mod land;
pub mod landing;
pub mod loader;
pub mod market;
pub mod mortgage;
pub mod msg;
pub mod player;
pub mod plot;
pub mod property;
pub mod rng;
pub mod rules;
pub mod settle;
pub mod shop;
pub mod status;
pub mod stock;
pub mod turn;
pub mod walk;
pub mod world;

pub mod prelude {
    pub use crate::afflict::{AfflictDomain, Affliction, Insulation};
    pub use crate::bank::{
        AtmAccess, apply_month_end_interest_system, bank_op_allowed, bank_op_system,
        grant_atm_access_system, settle_bank_landing,
    };
    pub use crate::board::{
        BoardDef, BranchDef, BranchKind, GridPos, LandDef, LandType, TileDef, TileIndex, TileKind,
    };
    pub use crate::card::{CardDef, CardEffect, CardKind, CardSet, CardTarget};
    pub use crate::charge::{
        Adjustment, Charge, ChargeDomain, ChargeKind, Correction, Corrections, Scope, Source,
        charge_domain_of, scales_with_price, stage, transferable,
    };
    pub use crate::coupon::{CouponBoxes, settle_coupon_landing};
    pub use crate::damage::{DamageTally, LandDamage};
    pub use crate::data::{
        BOARD_PATH, CHANCE_PATH, DataHandles, FATE_PATH, ITEMS_PATH, STOCKS_PATH,
    };
    pub use crate::deck::{
        CardId, Deck, DeckOrder, Shelf, apply_card, card_of, order_of, settle_card_landing,
    };
    pub use crate::economy::LandView;
    pub use crate::hand::{CardAim, Hand, PlayWindow, card_notice, deal_to, play_card_system};
    pub use crate::land::{BuildingLevel, LandId, Mortgaged, Owner};
    pub use crate::landing::{
        Landed, Landing, LandingDomain, Pending, PendingDecision, classify_landing_system,
        close_landing_system, decision_system, domain_of, jail_player, teleport,
    };
    pub use crate::loader::JsonAssetLoader;
    pub use crate::market::{
        Market, MarketClock, market_tick_on_turn_end_system, settle_market_landing,
        trade_request_system,
    };
    pub use crate::mortgage::{
        MortgageDeny, MortgageFacts, counter_plots, mortgage_op_allowed, mortgage_op_system,
        read_facts,
    };
    pub use crate::msg::{
        BankOp, BankOpDone, BankOpRequested, BankVisited, BuyOffered, CardDrawn, CardGained,
        CardNotice, CardPlayed, CashChanged, ChargeWaived, Decision, DecisionChosen, DiceRolled,
        EffectOutcome, GameOver, JailSkipped, LandBought, LandUpgraded, MortgageDone, MortgageOp,
        MortgageRequested, PlayCardRequested, PlayerArrived, PlayerBankrupt, PlayerJailed,
        PlayerReleased, PlayerTeleported, PointsChanged, RentDue, RequestRoll, ShopOpDone,
        ShopOpRequested, StayLifted, StaySkipped, StockPriceChanged, StockTradeRequested,
        StockTraded, StockVisited, TileStepped, TradeSide, TurnEnded, TurnStarted, UpgradeOffered,
        WalkStarted,
    };
    pub use crate::player::{
        BankBalance, Bankrupt, GameSetup, Heading, JailSentence, PLAYER_COLORS, PlayerColor,
        PlayerId, PlayerName, PlayerSetup, Points, START_TILE, StaySentence, TokenPosition,
        WalkQueue, Wallet,
    };
    pub use crate::plot::{PlotArea, area_of, is_in_area};
    pub use crate::property::settle_property_landing;
    pub use crate::rng::{DIE_FACES, GameSeed, Rng};
    pub use crate::rules::GameRules;
    pub use crate::settle::{Books, InsufficientFunds};
    pub use crate::shop::{
        ShopDeny, ShopStock, buyback, restock, settle_shop_landing, shop_op_allowed, shop_op_system,
    };
    pub use crate::status::{LandStatus, Timed, expire_land_status_system};
    pub use crate::stock::{PriceHistory, StockDef, StockHoldings, StockId, StockPrice, StockSet};
    pub use crate::turn::{GameSession, Paused, TurnOrder, TurnPhase, not_paused};
    pub use crate::walk::{WalkClock, advance_walk_clock_system, walk_step_system};
    pub use crate::world::{ActiveBoard, CoreSet, Gameplay, begin_game_system};
}

use bank::apply_month_end_interest_system;
use prelude::{
    ActiveBoard, AtmAccess, BankOpDone, BankOpRequested, BankVisited, BoardDef, BuyOffered,
    CardDrawn, CardGained, CardPlayed, CardSet, CashChanged, ChargeWaived, Corrections,
    CouponBoxes, DecisionChosen, DiceRolled, GameOver, GameRules, GameSeed, GameSetup, JailSkipped,
    JsonAssetLoader, LandBought, LandUpgraded, Landing, MarketClock, MortgageDone,
    MortgageRequested, Paused, PendingDecision, PlayCardRequested, PlayerArrived, PlayerBankrupt,
    PlayerJailed, PlayerReleased, PlayerTeleported, PointsChanged, RentDue, RequestRoll,
    ShopOpDone, ShopOpRequested, ShopStock, StayLifted, StaySkipped, StockPriceChanged, StockSet,
    StockTradeRequested, StockTraded, StockVisited, TileStepped, TurnEnded, TurnPhase, TurnStarted,
    UpgradeOffered, WalkClock, WalkStarted, advance_walk_clock_system, bank_op_system,
    begin_game_system, decision_system, expire_land_status_system, grant_atm_access_system,
    market_tick_on_turn_end_system, mortgage_op_system, play_card_system, shop_op_system,
    trade_request_system, walk_step_system,
};
use turn::{
    begin_turn_system, end_turn_system, game_over_system, not_paused, roll_dice_system,
    update_price_index_system,
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
            .register_asset_loader(JsonAssetLoader::<BoardDef>::default())
            .register_asset_loader(JsonAssetLoader::<CardSet>::default())
            .register_asset_loader(JsonAssetLoader::<StockSet>::default())
            // 对局配置有默认值（prototype 双人阵容 + 可复现种子），
            // 表现层要换阵容/换种子就在 Startup 前 insert_resource 覆盖
            .init_resource::<GameRules>()
            .init_resource::<GameSetup>()
            .init_resource::<GameSeed>()
            .init_resource::<Paused>()
            .init_resource::<WalkClock>()
            .init_resource::<MarketClock>()
            .init_resource::<Landing>()
            .init_resource::<PendingDecision>()
            .init_resource::<AtmAccess>()
            .init_resource::<CouponBoxes>()
            .init_resource::<Corrections>()
            .init_resource::<ShopStock>()
            // ===== 流程与移动 =====
            .add_message::<TurnStarted>()
            .add_message::<RequestRoll>()
            .add_message::<DiceRolled>()
            .add_message::<WalkStarted>()
            .add_message::<TileStepped>()
            .add_message::<PlayerArrived>()
            .add_message::<TurnEnded>()
            .add_message::<PlayerReleased>()
            .add_message::<PlayerJailed>()
            .add_message::<PlayerTeleported>()
            .add_message::<JailSkipped>()
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
            .add_message::<LandUpgraded>()
            .add_message::<RentDue>()
            // 一次性修正生效那一笔（免费卡花掉了）。不注册的话 `Books` 的 writer
            // 参数校验直接失败 —— 而 `Books` 被每个 system 用着，症状是整局不跑
            .add_message::<ChargeWaived>()
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
            // ===== 抵押与赎回（P3d③）=====
            .add_message::<MortgageRequested>()
            .add_message::<MortgageDone>()
            .init_state::<TurnPhase>()
            // 装配世界：初始态的 OnEnter 不会触发，所以用 in_state(Boot)
            .add_systems(
                Update,
                begin_game_system
                    .in_set(CoreSet::Boot)
                    .run_if(in_state(TurnPhase::Boot)),
            )
            .add_systems(OnEnter(TurnPhase::TurnStart), begin_turn_system)
            .add_systems(
                Update,
                (
                    roll_dice_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    walk_step_system
                        .run_if(in_state(TurnPhase::Walking))
                        .run_if(not_paused),
                    decision_system
                        .run_if(in_state(TurnPhase::WaitDecision))
                        .run_if(not_paused),
                    // 证券交易的窗口就是「还没掷骰」这一段，与掷骰同态、同闸门
                    trade_request_system
                        .run_if(in_state(TurnPhase::WaitRoll))
                        .run_if(not_paused),
                    // 出牌与下单共用这一条窗口（见 `hand::play_card_system`）：
                    // 「本回合要怎么走」在我还没走之前决定完
                    play_card_system
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
                (
                    landing::classify_landing_system,
                    property::settle_property_landing,
                    bank::settle_bank_landing,
                    market::settle_market_landing,
                    deck::settle_card_landing,
                    coupon::settle_coupon_landing,
                    shop::settle_shop_landing,
                    landing::close_landing_system,
                )
                    .chain(),
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
                    apply_month_end_interest_system,
                    update_price_index_system,
                )
                    .chain(),
            )
            .add_systems(OnEnter(TurnPhase::Over), game_over_system);
    }
}
