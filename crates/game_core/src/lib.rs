//! 《暴富》规则层入口。
//!
//! 实体分四类（B3 起）：土地（`LandId`+`Owner`+`BuildingLevel`+可选 `Mortgaged`）、
//! 玩家（`PlayerId`+`Wallet`+`TokenPosition`+`BankBalance`+`StockHoldings`+可选
//! `JailSentence`/`Bankrupt`）、股票（`StockId`+`StockPrice`+`PriceHistory`）、
//! 牌堆（`Deck`，机会/命运各一）。道路格**不建实体**，相机/灯光/UI 在 `game_app`。

use bevy::prelude::*;
use bevy::state::app::StatesPlugin;
use bevy::time::TimePlugin;

pub mod bank;
pub mod board;
pub mod card;
pub mod data;
pub mod deck;
pub mod economy;
pub mod land;
pub mod landing;
pub mod loader;
pub mod market;
pub mod msg;
pub mod player;
pub mod property;
pub mod rng;
pub mod rules;
pub mod settle;
pub mod stock;
pub mod turn;
pub mod walk;
pub mod world;

pub mod prelude {
    pub use crate::bank::settle_bank_landing;
    pub use crate::board::{
        BoardDef, BranchDef, BranchKind, GridPos, LandDef, LandType, TileDef, TileIndex, TileKind,
    };
    pub use crate::card::{CardDef, CardEffect, CardKind, CardSet};
    pub use crate::data::{BOARD_PATH, CHANCE_PATH, DataHandles, FATE_PATH, STOCKS_PATH};
    pub use crate::deck::{CardId, Deck, card_of, settle_card_landing};
    pub use crate::economy::LandView;
    pub use crate::land::{BuildingLevel, LandId, Mortgaged, Owner};
    pub use crate::landing::{
        Landed, Landing, Pending, PendingDecision, classify_landing_system, close_landing_system,
        decision_system, jail_player, teleport,
    };
    pub use crate::loader::JsonAssetLoader;
    pub use crate::market::{Market, MarketClock, settle_market_landing};
    pub use crate::msg::{
        BankVisited, BuyOffered, CardDrawn, CardNotice, CashChanged, Decision, DecisionChosen,
        DiceRolled, EffectOutcome, GameOver, JailSkipped, LandBought, LandUpgraded, PlayerArrived,
        PlayerBankrupt, PlayerJailed, PlayerReleased, PlayerTeleported, RentDue, RequestRoll,
        StockPriceChanged, StockTraded, StockVisited, TileStepped, TradeSide, TurnEnded,
        TurnStarted, UpgradeOffered, WalkStarted,
    };
    pub use crate::player::{
        BankBalance, Bankrupt, GameSetup, JailSentence, PLAYER_COLORS, PlayerColor, PlayerId,
        PlayerName, PlayerSetup, START_TILE, TokenPosition, WalkQueue, Wallet,
    };
    pub use crate::property::settle_property_landing;
    pub use crate::rng::{DIE_FACES, GameSeed, Rng};
    pub use crate::rules::GameRules;
    pub use crate::settle::{Books, InsufficientFunds};
    pub use crate::stock::{PriceHistory, StockDef, StockHoldings, StockId, StockPrice, StockSet};
    pub use crate::turn::{GameSession, TurnOrder, TurnPhase};
    pub use crate::walk::{WalkClock, advance_walk_clock_system, walk_step_system};
    pub use crate::world::{ActiveBoard, begin_game_system};
}

use prelude::{
    BankVisited, BoardDef, BuyOffered, CardDrawn, CardSet, CashChanged, DecisionChosen, DiceRolled,
    GameOver, GameRules, GameSeed, GameSetup, JailSkipped, JsonAssetLoader, LandBought,
    LandUpgraded, Landing, MarketClock, PendingDecision, PlayerArrived, PlayerBankrupt,
    PlayerJailed, PlayerReleased, PlayerTeleported, RentDue, RequestRoll, StockPriceChanged,
    StockSet, StockTraded, StockVisited, TileStepped, TurnEnded, TurnPhase, TurnStarted,
    UpgradeOffered, WalkClock, WalkStarted, advance_walk_clock_system, begin_game_system,
    decision_system, walk_step_system,
};
use turn::{begin_turn_system, end_turn_system, game_over_system, roll_dice_system};

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
            .init_resource::<WalkClock>()
            .init_resource::<MarketClock>()
            .init_resource::<Landing>()
            .init_resource::<PendingDecision>()
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
            .add_message::<GameOver>()
            // ===== 地产与经济（B3）=====
            .add_message::<CashChanged>()
            .add_message::<PlayerBankrupt>()
            .add_message::<BuyOffered>()
            .add_message::<UpgradeOffered>()
            .add_message::<LandBought>()
            .add_message::<LandUpgraded>()
            .add_message::<RentDue>()
            .add_message::<BankVisited>()
            .add_message::<StockVisited>()
            .add_message::<StockTraded>()
            .add_message::<StockPriceChanged>()
            // ===== 卡牌与决策（B3）=====
            .add_message::<CardDrawn>()
            .add_message::<DecisionChosen>()
            .init_state::<TurnPhase>()
            // 装配世界：初始态的 OnEnter 不会触发，所以用 in_state(Boot)
            .add_systems(Update, begin_game_system.run_if(in_state(TurnPhase::Boot)))
            .add_systems(OnEnter(TurnPhase::TurnStart), begin_turn_system)
            .add_systems(
                Update,
                (
                    roll_dice_system.run_if(in_state(TurnPhase::WaitRoll)),
                    walk_step_system.run_if(in_state(TurnPhase::Walking)),
                    decision_system.run_if(in_state(TurnPhase::WaitDecision)),
                ),
            )
            // 时钟在 First 推进、Update 消费：同帧内先累计、后定步
            .add_systems(
                First,
                advance_walk_clock_system.run_if(in_state(TurnPhase::Walking)),
            )
            // 落格流水线：解释一次格子 → 四个域各自结算 → 由 pending 决定去哪一阶段。
            // 顺序写成显式 chain：域的先后决定同帧消息的次序，靠隐式冲突排序说出来不算数。
            .add_systems(
                OnEnter(TurnPhase::Landed),
                (
                    landing::classify_landing_system,
                    property::settle_property_landing,
                    bank::settle_bank_landing,
                    market::settle_market_landing,
                    deck::settle_card_landing,
                    landing::close_landing_system,
                )
                    .chain(),
            )
            .add_systems(OnEnter(TurnPhase::TurnEnd), end_turn_system)
            .add_systems(OnEnter(TurnPhase::Over), game_over_system);
    }
}
