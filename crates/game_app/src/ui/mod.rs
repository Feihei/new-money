//! UI 控件层：按钮、面板、遮罩与点击派发。
//!
//! 0.20 的 `bevy_ui::Button` 与 `Interaction` 已弃用（prelude 里带着
//! `#[expect(deprecated, reason = "Should be removed after 0.20 is released…")]`），
//! 用它们会直接吃 `-D warnings`。点击一律走 picking 的指针事件：
//! 交互节点挂一个 [`UiAction`]，[`dispatch_ui_actions`] 读 `PointerClick` 消息派发。
//!
//! 命中测试给出的是**最深的**节点 —— 按到的是按钮里的文字而不是按钮本身，
//! 所以两个入口都要沿 `ChildOf` 往上找第一个带 `UiAction` 的祖先
//! （[`action_of`]）。顺带这也让遮罩真的能挡住下面：遮罩是最深命中节点、没有动作，
//! 事件就停在那里，不会穿到顶栏按钮上。

pub mod calendar_fx;
pub mod dialog;
pub mod finance;
pub mod hand;
pub mod items;
pub mod menu;
pub mod minigame;
pub mod pause;
pub mod player_hud;
pub mod theme;
pub mod tile_info;
pub mod top_bar;

use bevy::{app::AppExit, ecs::system::SystemParam, prelude::*};
use game_core::prelude::{
    ActiveBoard, AtmAccess, AuctionMove, BailRequested, BankOp, BankOpRequested, BidMove,
    BuildRequested, BuildingKind, BuyTicketRequested, CardAim, CardId, Decision, DecisionChosen,
    GameKind, GameSession, HolidaySet, ItemKind, LandId, MarketClock, MinigameResult, MortgageOp,
    MortgageRequested, Paused, Pending, PendingDecision, PlaceItemRequested, PlayCardRequested,
    RequestRemoteDice, RequestRoll, RequestStay, ShopOpRequested, StaySentence, StockId,
    StockTradeRequested, TileIndex, TradeSide, TurnOrder, TurnPhase, bank_op_allowed, date_of,
};

use theme::{col, layout, size};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<dialog::RecentCard>()
            // 最近一笔服务费（P10c）。与 `RecentCard` 同一条：面板弹在收据上，
            // 而 `Pending::ServiceNotice` 只说「哪块地」，说不出摇了几点、住了几天 ——
            // 那些数只活在这条消息里
            .init_resource::<dialog::RecentService>()
            // 最近一笔公司到访（P10d①）。与 `RecentService` 同一条：收据的读数只在消息里
            .init_resource::<dialog::RecentCompany>()
            // `NM_PAUSE=1` 开局即暂停：没有键盘可按，暂停面板否则无法被截图核对
            .insert_resource(pause::PauseMenuOpen::from_env())
            // `NM_PICK_TILE=列:行` 开局即选中那格：详情卡否则也进不了截图
            .insert_resource(tile_info::SelectedTile::from_env())
            // `NM_DECISION=buy|upgrade|rent|card|bank` 直接挂出某个模态（见 [`dialog::ForceModal`]）
            .insert_resource(dialog::ForceModal::from_env())
            // `NM_STOCK=1` / `NM_ATM=1` / `NM_HAND=1` / `NM_ITEMS=1`：四块浮层的非鼠标
            // 入口，理由同 `NM_PAUSE`
            .insert_resource(finance::StockPanelOpen::from_env())
            .insert_resource(finance::AtmPanelOpen::from_env())
            .insert_resource(hand::HandPanelOpen::from_env())
            .insert_resource(items::ItemsPanelOpen::from_env())
            // 游乐场那四条（P7）常驻而不是 `insert_resource(from_env())`：这一屏**两个场景
            // 都要用**（对局里踩到那一格、主菜单上练习），所以「开机时定」那一套不适用 ——
            // 旋钮 `NM_MINIGAME` 走的是 core 的窗口（见 `dev.rs`），不从 UI 侧开门
            .init_resource::<minigame::Arcade>()
            .init_resource::<minigame::Practice>()
            .init_resource::<minigame::PracticeRounds>()
            .init_resource::<minigame::ArcadeChoiceOpen>()
            .add_systems(
                Update,
                (
                    (top_bar::spawn_top_bar, player_hud::spawn_player_hud).in_set(UiSet::Boot),
                    // 集合成员**逐个标**：`sync_stock_panel` 的参数包 `StockView<'w, 's>`
                    // 带两个生命周期，而元组整体 `.in_set(...)` 在这种元素上会退化到
                    // `ObserverSystemExt` 的重载解析然后崩（§6.1 坑 14）。
                    // 门禁仍然在集合上（见下面 `configure_sets`），这里只是标成员。
                    (
                        pause::toggle_pause_on_escape.in_set(UiSet::Live),
                        top_bar::refresh_top_bar.in_set(UiSet::Live),
                        top_bar::hint_from_events.in_set(UiSet::Live),
                        player_hud::refresh_player_hud.in_set(UiSet::Live),
                        dialog::remember_drawn_card.in_set(UiSet::Live),
                        dialog::remember_service.in_set(UiSet::Live),
                        dialog::remember_company.in_set(UiSet::Live),
                        dialog::force_decision.in_set(UiSet::Live),
                        dialog::sync_modal.in_set(UiSet::Live),
                        finance::sync_atm_panel.in_set(UiSet::Live),
                        finance::sync_stock_panel.in_set(UiSet::Live),
                        hand::sync_hand_panel.in_set(UiSet::Live),
                        items::sync_items_panel.in_set(UiSet::Live),
                        pause::sync_pause_menu.in_set(UiSet::Live),
                        tile_info::pick_tile.in_set(UiSet::Live),
                        tile_info::sync_tile_card.in_set(UiSet::Live),
                    ),
                    // 派发与悬停**不**受 `ActiveBoard` 门禁：主菜单上没有 ActiveBoard，
                    // 而「新游戏」「退出」恰恰只能在那里派发。曾把它们挂进 Live 集
                    // （`resource_exists::<ActiveBoard>`），后果是菜单按钮点了没反应、
                    // 只能点窗口关闭——且 `NM_MENULOOP` 直改状态绕过了派发，B5 验收测不到。
                    // 依赖的资源全部常驻（见 `Gate`/`UiState`），菜单一帧不缺。
                    //
                    // 游乐场那三条同理，而且理由更硬：**练习局就开在主菜单上**（P7），
                    // 那里连棋盘都没有。排在派发之前，所以「这一帧刚建好的那一屏」
                    // 当帧就能被点到。
                    minigame::sync_choice,
                    minigame::sync_arcade,
                    minigame::step_arcade,
                    dispatch_ui_actions,
                    hover_feedback,
                )
                    .chain(),
            );
        // 守门挂在集合上而不是逐个 system 上：元组整体 `.run_if(...)` 在参数含双生命周期
        // `SystemParam`（`Books`、`Notices`）时会退化成重载解析失败（§六 坑 9）
        app.configure_sets(
            Update,
            (
                UiSet::Boot.run_if(resource_added::<ActiveBoard>),
                UiSet::Live.run_if(resource_exists::<ActiveBoard>),
            ),
        );
        // `NM_ATM=1`：把 ATM 卡直接发到当前回合玩家手里。浮层要有不靠鼠标的入口，
        // 而这张卡的正常来源是「路过银行格」—— 那一掷不由截图脚本控制。
        if std::env::var_os("NM_ATM").is_some() {
            app.add_systems(Update, finance::dev_grant_atm);
        }
        // `NM_HAND=1`：开局即把手牌浮层打开并发五种形状的样例卡。道具卡的正经来源是百货
        // 柜台（P3d②），而「踩到哪一格」与「点券够不够」都不由截图脚本控制 —— 这个旋钮
        // 一次拍齐五种按钮排。只发一次，见 `hand::dev_deal_sample`。
        if std::env::var_os("NM_HAND").is_some() {
            app.add_systems(
                Update,
                hand::dev_deal_sample.run_if(resource_exists::<ActiveBoard>),
            );
        }
        // `NM_SHOP=1`：百货柜台的非鼠标入口。这块面板的正经入口是**踩到那一格**，
        // 而那一掷由骰子说了算；它还要货架与点券两样前提，所以比 `NM_DECISION` 多一段
        // （见 `dialog::dev_open_shop`）。
        if std::env::var_os("NM_SHOP").is_some() {
            app.add_systems(
                Update,
                dialog::dev_open_shop.run_if(resource_exists::<ActiveBoard>),
            );
        }
        // `NM_PLOTS=<n>`：把前 n 块地划给当前回合玩家。银行柜台那一节「你的地产」要有地块
        // 才拍得到，而「名下有地」在正经玩法里同样要靠骰子踩中无主地（判据：每个界面都得有
        // 一条不靠鼠标的入口）。与 `NM_DECISION=bank` 配用就是柜台那一屏。
        if std::env::var_os("NM_PLOTS").is_some() {
            app.add_systems(
                Update,
                dialog::dev_grant_plots.run_if(resource_exists::<ActiveBoard>),
            );
        }
        // `NM_BUILD=1`：建设窗口的非鼠标入口（P10c）。正经入口是**踩到自己的空商业地**，
        // 而演示盘那块商业地（外滩）一开局就带着预建的购物中心 —— 这扇窗口在没有旋钮的
        // 世界里根本不会出现（判据同 `NM_SHOP`：每个界面都得有一条不靠鼠标的入口）。
        if std::env::var_os("NM_BUILD").is_some() {
            app.add_systems(
                Update,
                dialog::dev_open_build.run_if(resource_exists::<ActiveBoard>),
            );
        }
        // `NM_SERVICE=1`：服务费收据的非鼠标入口（P10c）。正经入口是踩到别人的设施，
        // 而这一屏只活两三帧（自动玩家下一帧就答掉），截图撞不上 —— 判据同 `NM_SHOP`。
        if std::env::var_os("NM_SERVICE").is_some() {
            app.add_systems(
                Update,
                dialog::dev_open_service.run_if(resource_exists::<ActiveBoard>),
            );
        }
        // `NM_COMPANY=1`：公司到访收据的非鼠标入口（P10d①）。判据同 `NM_SERVICE`。
        if std::env::var_os("NM_COMPANY").is_some() {
            app.add_systems(
                Update,
                dialog::dev_open_company.run_if(resource_exists::<ActiveBoard>),
            );
        }
        // `NM_DAY=<tick>`：把日历钉在第 n 天。P2④ 之后「今天休不休市」是面板上十二颗按钮
        // 的着色判据，而哪天是周六由骰子走到哪一回合决定 —— 同一条理由，给它一条不靠运气的入口。
        // 每帧都钉：装配世界时 `begin_game_system` 会把 `MarketClock` 重置成默认值，
        // 只钉一次会在那一帧被抹掉。代价是这局日历不再往前走的（行情、派息、结息都停在
        // 那一天）—— 它是截图旋钮，不是时光机。
        if let Some(tick) = std::env::var("NM_DAY").ok().and_then(|s| s.parse().ok()) {
            app.add_systems(Update, pin_day.run_if(resource_exists::<ActiveBoard>));
            app.insert_resource(PinnedDay(tick));
        }
        // 日历事件的表现层（P2b）：`NM_AUTOROLL` 下它自己会拒绝安装
        calendar_fx::install(app);
    }
}

#[derive(Resource)]
struct PinnedDay(u32);

fn pin_day(mut clock: ResMut<MarketClock>, pin: Res<PinnedDay>) {
    clock.tick = pin.0;
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum UiSet {
    /// 棋盘就位那一刻装配一次
    Boot,
    /// 逐帧刷新，只在对局内才有意义（门是 `ActiveBoard`）。
    /// 派发与悬停不在此集内 —— 它们无条件跑，排在 Live 之后（见 `build`）。
    Live,
}

/// 挂在可点击节点上。语义是「按下去要做的事」，不携带状态 —— 能不能做由派发方判断。
///
/// 只 `Clone` 不 `Copy`：`Trade` 要带 `StockId`（里面是字符串）。命中测试每个点击只走一次，
/// 这点拷贝不值得为它把股票 id 换成索引 —— 换成索引就得在派发处再解回股票，
/// 而「按的是哪只股」在面板上本来就以 id 为准。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub enum UiAction {
    NewGame,
    Quit,
    BackToMenu,
    /// 掷骰。只有当前回合玩家、且没有待决事项时才生效（见 dispatch）
    Roll,
    /// 回答当前待决事项。「回答关于谁」由 `PendingDecision` 决定，按钮不背这个信息
    Decide(Decision),
    /// 关闭点格详情卡（新增交互，见 [`tile_info`]）
    CloseInfo,
    /// 开合暂停菜单（ESC 键与「继续」「暂停」按钮同一个动作）
    TogglePause,
    /// 开合 ATM 浮层（B5b：路过银行格解锁）
    ToggleAtm,
    /// 开合证券浮层（B5b：常驻入口，每回合掷骰前可交易）
    ToggleStock,
    /// 开合手牌浮层（P3b：常驻入口，与证券同一 —— 「能不能看」与「能不能出牌」是两件事）
    ToggleHand,
    /// 开合道具浮层（P5）：身上带着什么、能放到前方哪一格
    ToggleItems,
    /// 把一件道具放到主环的某一格。**目标烤在按钮里**（「5 南京路」本身就是一整个意图），
    /// 与 [`Self::PlayCard`] 同一条形状
    PlaceItem {
        item: ItemKind,
        tile: TileIndex,
    },
    /// 用一颗遥控骰子指定这一回合走几步（1..=6）
    UseRemoteDice {
        steps: u32,
    },
    /// 带颗数掷骰（P5 的载具）：`0` = 用载具上限，与 [`UiAction::Roll`] 同一条窗口。
    /// 单开一个变体而不是把 `Roll` 改成带字段，是因为顶栏那颗按钮不关心颗数
    RollWith(u32),
    /// 办一笔银行操作，金额在建按钮那一刻就烤成定值（面板跟着钱包重建，「全部」才是准的）
    BankOp {
        op: BankOp,
        amount: i64,
    },
    /// 买卖股票
    Trade {
        stock: StockId,
        side: TradeSide,
        shares: u32,
    },
    /// 打出手里的一张卡。**目标烤在按钮里**（那颗「陷害 → 孙小美」本身就是一整个意图），
    /// 所以规则层不必再挂一个「选目标」的待决事项 —— 与金额按钮同一个形状。
    PlayCard {
        card: CardId,
        aim: Option<CardAim>,
    },
    /// 在百货柜台买或卖一张卡（P3d②）。按钮上写的数额只是给玩家看的（「买 30」），
    /// 实际扣/付多少仍由 `shop::shop_op_system` 从卡面算 —— 与 [`UiAction::BankOp`] 同一条：
    /// 请求带意图，账由规则层算。
    ShopOp {
        side: TradeSide,
        card: CardId,
    },
    /// 在银行柜台抵押或赎回一块自己的地（P3d③）。与 [`UiAction::ShopOp`] 同一条：
    /// 按钮上写的数额只是给玩家看的，到手多少、赎回多少仍由规则层从账面与本金算。
    /// 主语也不在按钮里 —— 派发时取「此刻柜台前站着的那个人」。
    MortgageOp {
        land: LandId,
        op: MortgageOp,
    },
    /// 在拘留所门口保释一个人（P4③）。按钮上写的是「保释 阿土伯」，主语仍是
    /// 「此刻站在窗口前的那个人」—— 与银行/百货那两颗同一条形状。
    Bail {
        target: Entity,
    },
    /// 在自己的空商业地上立起一类设施（P10c）。**类别烤在按钮里**（那颗「建购物中心」
    /// 本身就是一整个意图），而多少钱仍由规则层算 —— 与 [`Self::BankOp`] 同一条分工。
    /// 主语也不在按钮里（派发时取建设窗口挂着的那位）。
    Build {
        land: LandId,
        kind: BuildingKind,
    },
    /// 在彩票柜台买一注（P8）。**号烤在按钮里**（那颗「17」本身就是一整个意图），
    /// 而一注多少钱仍由规则层从 `GameRules::ticket_price` 算 —— 与 [`UiAction::ShopOp`]
    /// 同一条分工：请求带意图，账由规则层算。主语也不在按钮里（派发时取柜台前那个人）。
    BuyTicket(u8),
    /// 在拍卖台上做这一个动作（P9）。**金额不在按钮里**：它由
    /// [`game_core::auction::Lot::next_amount`] 从「当前最高价 + 一档」现算，
    /// 面板与规则层读的是同一个表达式 —— 少这一格约束，就会出现「写着 6,840、扣掉 6,270」。
    ///
    /// 主语也不在按钮里（派发时取「此刻被问的那一位」），而那一位在拍卖过程中**每帧都可能换人**，
    /// 这是它与其余六种窗口的差别。
    Bid(BidMove),
    /// 点掉第 `slot` 只气球（P7 打气球）。槽位在建屏那一刻就烤进按钮里，
    /// 而「这一下算不算」仍由那一局的状态说了算（收工了、槽位越界都不算）
    Pop {
        slot: u8,
    },
    /// 往第 `cell` 格下铲（P7 企鹅挖宝）。铲过的格子再点不算，那格自己也会挂 `Disabled`
    Dig {
        cell: u8,
    },
    /// 提前收工：把现在这个分数送去做结算。它**不提交成绩**，结算屏上那颗才算
    ArcadeQuit,
    /// 收下成绩：报一份 `MinigameResult` 给规则层。练习局没有可报的（见 `minigame::result_of`）
    ArcadeCollect,
    /// 练习局「再来一局」。只把开过几手加一，重建由 `sync_arcade` 的签名比较做
    ArcadeReplay,
    /// 练习局「离开」：清掉 `Practice`，这一屏随签名变化被收走
    ArcadeLeave,
    /// 开合主菜单那块「选一款练习」
    ToggleArcadeChoice,
    /// 在主菜单上开始一局练习。练习不给点券 —— 而这不是「菜单不发消息」，
    /// 是结算屏上写出来的那半句（理由见 `minigame` 模块头）
    Practice(GameKind),
}

/// 存在即禁用：着色变灰，且点击不派发。对应 Godot 的 `button.disabled = true`。
#[derive(Component, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disabled;

/// 按钮的常态与悬停色。
#[derive(Component, Debug, Clone, Copy)]
pub struct Palette {
    pub normal: Color,
    pub hover: Color,
}

/// 主菜单的根节点标记，`OnEnter(Scene::Board)` 按它整块收走。
#[derive(Component)]
pub struct MenuPiece;

/// 对局内的 UI（顶栏、HUD、弹层）根节点标记，回菜单时整块收走。
#[derive(Component)]
pub struct BoardUi;

/// 掷骰此刻可用吗。派发方与顶栏着色共用一份判据，免得出现「看着能点、按下去没反应」。
///
/// 规则层还会再认一次「是不是当前玩家」，所以这里放行错人也不会真的掷出骰子
/// （`roll_dice_system` 的守门）。
pub(crate) fn can_roll(phase: TurnPhase, paused: &Paused, pending: &PendingDecision) -> bool {
    phase == TurnPhase::WaitRoll && !paused.0 && pending.active().is_none()
}

/// 证券交易的窗口与掷骰同一条：`WaitRoll`、没暂停、没被模态占住。
/// 「本回合怎么安排钱」属于掷骰之前的决定，走完路再买就变成事后诸葛。
/// 判据直接复用 [`can_roll`]，两条从此不会各自漂移。
///
/// `open` 是「今天股市开门吗」（见 [`is_open_day`]）：周末那两天规则层
/// `market::trade_request_system` 一律不受理，这里不挡就是「按下去没声音」。
pub(crate) fn can_trade(
    phase: TurnPhase,
    paused: &Paused,
    pending: &PendingDecision,
    market_open: bool,
) -> bool {
    market_open && can_roll(phase, paused, pending)
}

/// 出牌的窗口也是同一条：`WaitRoll`、没暂停、没被模态占住。规则层
/// [`game_core::hand::PlayWindow::opener`] 判的是同一件事（它多认一次主语），
/// 这里只管「该不该亮」。
///
/// 单独一个函数而不是就地用 [`can_roll`]：PLAN §三 记着 4 代的出牌时机分三类
/// （随时／走到别人地盘时／股市交易时），P3c 给地块卡开那个口子时改的是这一处，
/// 而不是让「掷骰」那条判据跟着一起变宽。
pub(crate) fn can_play(phase: TurnPhase, paused: &Paused, pending: &PendingDecision) -> bool {
    can_roll(phase, paused, pending)
}

/// 柜台此刻开着的话，是关于谁的。
pub(crate) fn counter_of(pending: &PendingDecision) -> Option<Entity> {
    match pending.active() {
        Some((player, Pending::BankTeller)) => Some(player),
        _ => None,
    }
}

/// 百货柜台此刻开着的话，是关于谁的（P3d②）。与 [`counter_of`] 分开两个函数而不是
/// 合成一个「任何窗口」：两种窗口的**资格**不同 —— 银行还有一张路过就发的 ATM 卡，
/// 百货只有踩上去那一刻。并成一个函数，第一条被放宽的就是那条不该放宽的。
pub(crate) fn shop_customer(pending: &PendingDecision) -> Option<Entity> {
    match pending.active() {
        Some((player, Pending::Shop)) => Some(player),
        _ => None,
    }
}

/// 彩票柜台前站着谁（P8）。与 [`shop_customer`] 同一条：窗口就是全部的判据，
/// 这一格没有 ATM 那种「路过就长期有效」的资格。
pub(crate) fn lottery_customer(pending: &PendingDecision) -> Option<Entity> {
    match pending.active() {
        Some((player, Pending::Lottery)) => Some(player),
        _ => None,
    }
}

/// 拍卖台此刻问的是谁（P9）。与 [`lottery_customer`] 同一条：窗口就是全部的判据。
///
/// 但这一格**每帧都可能换人**（轮转），而柜台那五种从头到尾站着同一个人 —— 这不是把两个
/// 函数并成一个的理由，恰恰是分开写的理由：派发拿到的必须是「此刻挂着窗口的那位」，
/// 而不是「这一局开始时踩到那一格的那位」。
pub(crate) fn auction_customer(pending: &PendingDecision) -> Option<Entity> {
    match pending.active() {
        Some((player, Pending::Auction)) => Some(player),
        _ => None,
    }
}

/// 今天股市开盘吗。**只反问股市**（P2b）：周末 ∪ 节假日休市，银行全年无休。
/// 规则层 `market::trade_request_system` 读的是同一个 `HolidaySet::is_market_closed`；
/// UI 只负责在玩家按下之前把按钮灰掉并写明为什么。
pub(crate) fn is_market_open(
    clock: &MarketClock,
    session: &GameSession,
    holidays: &HolidaySet,
) -> bool {
    !holidays.is_market_closed(date_of(clock.tick, session.epoch))
}

/// 此刻能不能办这一笔银行操作。面板着色与派发共用这一句（同 `can_roll` 的理由）。
///
/// 与规则层同构但不完全等价：这里多挡一条「挂着别的模态时不办」。买地/升级/收据/卡面
/// 的内容是弹出那一刻烤死的，中途改钱包会让它显示过期数字 —— 那些遮罩本来也压住了浮层。
/// 真正的权限仍由 `bank::bank_op_system` 判，这里只管「该不该亮」。
pub(crate) fn can_bank_op(
    op: BankOp,
    paused: &Paused,
    pending: &PendingDecision,
    atm: &AtmAccess,
) -> bool {
    if paused.0 {
        return false;
    }
    let counter = counter_of(pending).is_some();
    let other_modal_up = pending.active().is_some() && !counter;
    !other_modal_up && bank_op_allowed(op, atm.0.is_some() || counter, counter)
}

/// 这笔操作的**主语**是谁：柜台优先（那就是柜台的客人），否则是 ATM 卡的持有人。
/// 按钮不携带玩家句柄 —— 带错了就是「甲的点击花掉乙的钱」，由状态说了算更安全。
pub(crate) fn bank_op_subject(pending: &PendingDecision, atm: &AtmAccess) -> Option<Entity> {
    counter_of(pending).or(atm.0)
}

/// 回主菜单时清掉 UI 侧**跨局存活**的记忆。挂在 `OnEnter(Scene::Menu)` 的链上。
///
/// 面板实体本身不用管：它们都带 [`BoardUi`]，由 `despawn_board_view` 整块收走，
/// 而「现在挂着什么」这类键已经改挂在实体上（见 [`dialog::ModalRoot`]、[`tile_info::TileView`]、
/// [`finance::FinanceStamp`]），随实体一起消失。剩下这些是纯资源，不清的后果各不一样，
/// 且都**不会报错**：
/// - `Paused` 留着 true → 下一局开局即冻结，画面看着完全正常；
/// - `PauseMenuOpen` 留着 true → 下一局第一帧就把面板盖回来；
/// - `SelectedTile` 留着 → 下一局凭空显示上一局那格的详情卡；
/// - `RecentCard` 留着 → 下一局第一次抽到「无内容」的卡时，显示的是上一局那张的文案；
/// - `AtmPanelOpen`/`StockPanelOpen` 留着 → 下一局第一帧就浮着两块上一局的账面板。
/// - `Arcade`/`Practice` 留着 → 下一局第一帧就摆着上一局那一屏游乐场（P7）。它比其余几条
///   更值得清：这一屏在**两个场景**都能出现，而练习那一半根本没有棋盘可依附。
///
/// 三个「开机时定」的开关一律复原到 `from_env()` 而不是 `default()`：开机的菜单入场
/// 就在这一条链上（`NM_SCENE=board` 时它先跑一次），清成 `default()` 会把开发旋钮
/// 自己抹掉 —— 结果是 `NM_ATM`/`NM_STOCK`/`NM_PAUSE` 三张截图永远拍不到东西，
/// 而「每个界面都得有一条不靠鼠标的入口」这条判据就形同虚设。真人没设环境变量，
/// `from_env()` 就是 `default()`，行为不变。游乐场那四条**没有** `from_env()`：
/// `NM_MINIGAME` 走的是 core 的窗口（`dev.rs`），不从 UI 侧开门，所以清成 `default()`。
pub(crate) fn reset_ui_state(mut commands: Commands, mut kept: UiMemory) {
    kept.recent_card.0 = None;
    kept.recent_service.0 = None;
    kept.selected.0 = None;
    *kept.atm = finance::AtmPanelOpen::from_env();
    *kept.stock = finance::StockPanelOpen::from_env();
    *kept.hand_open = hand::HandPanelOpen::from_env();
    *kept.items_open = items::ItemsPanelOpen::from_env();
    kept.arcade.0 = None;
    kept.practice.0 = None;
    kept.practice_rounds.0 = 0;
    kept.arcade_choice.0 = false;
    commands.insert_resource(pause::PauseMenuOpen::from_env());
    commands.insert_resource(Paused::default());
}

/// 回菜单要清掉的那十格 UI 记忆。打包是房规（§六 坑 8）：这一条清的东西随每块浮层增加，
/// 十一位散装参数撞的是 `too_many_arguments`，而它们要说的其实只有一件事 ——
/// 「哪些键活过了场景切换」。加一块浮层就在这里加一格，漏了的后果见上面那张清单。
#[derive(SystemParam)]
pub(crate) struct UiMemory<'w> {
    recent_card: ResMut<'w, dialog::RecentCard>,
    recent_service: ResMut<'w, dialog::RecentService>,
    selected: ResMut<'w, tile_info::SelectedTile>,
    atm: ResMut<'w, finance::AtmPanelOpen>,
    stock: ResMut<'w, finance::StockPanelOpen>,
    hand_open: ResMut<'w, hand::HandPanelOpen>,
    items_open: ResMut<'w, items::ItemsPanelOpen>,
    arcade: ResMut<'w, minigame::Arcade>,
    practice: ResMut<'w, minigame::Practice>,
    practice_rounds: ResMut<'w, minigame::PracticeRounds>,
    arcade_choice: ResMut<'w, minigame::ArcadeChoiceOpen>,
}

/// 一个文本节点需要的三件套。包一层的理由是漏掉 `TextFont` 不会报错，
/// 只会退回 Fira Mono 子集把中文变成豆腐块（§六 坑 2）。
pub fn label(
    font: &Handle<Font>,
    text: impl Into<String>,
    px: f32,
    color: Color,
) -> (Text, TextFont, TextColor) {
    (
        Text::new(text),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(px),
            ..default()
        },
        TextColor(color),
    )
}

/// 按钮配色：主（绿）与次（灰）。数值来自 dialog 的 `_style_button`。
pub const PRIMARY: (Color, Color) = (col::BTN_PRIMARY, col::BTN_PRIMARY_HOVER);
pub const SECONDARY: (Color, Color) = (col::BTN_SECONDARY, col::BTN_SECONDARY_HOVER);

/// 造一个按钮要说的全部信息。收成一个结构是因为参数一多就撞 `too_many_arguments`，
/// 而房规是「打包参数而不是挂 `#[allow]`」（§六 坑 8）。
pub struct ButtonSpec<'a> {
    pub text: &'a str,
    pub size: [f32; 2],
    pub font: f32,
    pub palette: (Color, Color),
    pub action: UiAction,
    /// 灰掉且不可点（Godot 的 `button.disabled`）
    pub disabled: bool,
}

impl<'a> ButtonSpec<'a> {
    pub fn new(text: &'a str, size: [f32; 2], palette: (Color, Color), action: UiAction) -> Self {
        Self {
            text,
            size,
            font: theme::size::BODY,
            palette,
            action,
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 一个定尺寸的圆角按钮：盒子 + 居中文本 + 动作。挂到 `parent` 下。
pub fn button(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, spec: ButtonSpec) -> Entity {
    let (normal, hover) = if spec.disabled {
        (col::BTN_DISABLED, col::BTN_DISABLED)
    } else {
        spec.palette
    };
    let edge = if spec.disabled {
        col::BTN_BORDER_DISABLED
    } else {
        col::BTN_BORDER
    };
    let mut spawned = parent.spawn((
        Node {
            width: Val::Px(spec.size[0]),
            height: Val::Px(spec.size[1]),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(layout::BTN_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
            ..default()
        },
        BackgroundColor(normal),
        border_color(edge),
        spec.action,
        Palette { normal, hover },
    ));
    if spec.disabled {
        spawned.insert(Disabled);
    }
    spawned
        .with_children(|b| {
            b.spawn(label(
                font,
                spec.text,
                spec.font,
                if spec.disabled {
                    col::BTN_TEXT_DISABLED
                } else {
                    Color::WHITE
                },
            ));
        })
        .id()
}

/// Godot 的 `HSeparator`：一条 2px 高的填充节点（没有 `BackgroundColor` 的 `Node`
/// 什么都画不出来，别以为它会自动有线）。
pub fn separator() -> (Node, BackgroundColor) {
    (
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(2.0),
            ..default()
        },
        BackgroundColor(col::BTN_BORDER),
    )
}

/// 建筑等级怎么说成人话。**这是内部编号与屏幕编号唯一的翻译点**，五块屏共用它。
///
/// 规则层那个数从 1 起算：`BuildingLevel(1)` 是**空地**（买地即 1，`rent[0]` 就是空地租金，
/// `settle::buy` 把无主的 Lv0 钳到 1），`BuildingLevel(2)` 才是盖了第一层楼。
/// 于是「Lv1」在屏幕上读起来像「有一层楼」，而它的意思是「一块空地」—— 10-22 就为这一格
/// 争过一轮（「地价 + 升级费 × 等级」到底是 `× L` 还是 `× (L−1)`，答案是同一个数被数了两遍：
/// 花钱盖的楼只有 `L−1` 层，见 `economy::estate_value`）。
///
/// 所以玩家面前不出现裸的内部编号：空地就叫空地（它不是一级建筑，也不该显示成 Lv0），
/// 往上才是建筑 Lv1、Lv2…。内部编号只在规则层、测试流水与注释里活着。
pub fn building_label(level: u8) -> String {
    if level <= 1 {
        "空地".to_string()
    } else {
        format!("建筑 Lv{}", level - 1)
    }
}

/// 一行「键 值」信息。`key_w` 是键列的固定宽（Godot 的 `custom_minimum_size.x`），
/// 值列 `flex_grow` 吃掉余下宽度，长文案就在列内换行。
pub fn info_row(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    key: &str,
    val: &str,
    val_color: Color,
    key_w: f32,
) {
    let (key_text, key_font, key_color) = label(font, key, size::ROW, col::KEY);
    let (val_text, val_font, val_ink) = label(font, val, size::ROW, val_color);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Node {
                    min_width: Val::Px(key_w),
                    ..default()
                },
                key_text,
                key_font,
                key_color,
            ));
            row.spawn((
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
                val_text,
                val_font,
                val_ink,
            ));
        });
}

/// 四边同色的边框。0.20 的 `BorderColor` 是四个字段而不是 `all()`，收在这里免得散开。
pub fn border_color(c: Color) -> BorderColor {
    BorderColor {
        top: c,
        right: c,
        bottom: c,
        left: c,
    }
}

/// 弹层面板：绝对定位到「屏幕中心 + 左上角偏移」，复刻 Godot 的 `PRESET_CENTER` + `position`。
///
/// 用 `Percent(50)` 定左上角、再用 `UiTransform::translation` 挪像素，是因为面板高度由内容撑开，
/// 拿不到 Godot 那个「已知高度的一半」—— translation 是布局之后的位移，不参与重排。
pub fn panel(
    width: f32,
    offset: [f32; 3],
    accent: Color,
) -> (Node, UiTransform, BackgroundColor, BorderColor) {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(50.0),
            width: Val::Px(width),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(layout::PANEL_PAD_X), Val::Px(layout::PANEL_PAD_Y)),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        UiTransform::from_translation(Val2::new(Val::Px(offset[1]), Val::Px(offset[2]))),
        BackgroundColor(col::PANEL),
        border_color(accent),
    )
}

/// 全屏遮罩。`z` 决定它压在哪些 UI 之上（同层靠 spawn 顺序，跨层靠 `ZIndex`）。
pub fn overlay(z: i32) -> (Node, BackgroundColor, ZIndex) {
    (
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(col::OVERLAY),
        ZIndex(z),
    )
}

/// 派发要读的世界状态。三个参数包是房规换来的：这个 system 需要知道的确实不少
/// （点到了什么、能不能做、要往哪发），但参数超过七个不挂 `#[allow]` 而是收拢（§六 坑 8）。
#[derive(SystemParam)]
struct Gate<'w, 's> {
    pending: Res<'w, PendingDecision>,
    order: Option<Res<'w, TurnOrder>>,
    phase: Res<'w, State<TurnPhase>>,
    paused: Res<'w, Paused>,
    /// ATM 卡是谁的。`init_resource` 常驻，菜单态也在（所以派发不受门禁，见 `build`）
    atm: Res<'w, AtmAccess>,
    /// 股市休市要挡在派发这一层：面板的灰色只是提示，按下去仍然会走到这里。
    /// 银行不看日历（P2b：全年无休），所以它不再读这两样里的任何一个数。
    clock: Res<'w, MarketClock>,
    session: Option<Res<'w, GameSession>>,
    holidays: Res<'w, HolidaySet>,
    /// 身上挂着停留计时的人那一颗「掷骰」是「停留」（P4④）。派发读它，不是为了替规则
    /// 层判资格 —— 那边 `roll_dice_system` 仍会拒掉被按住的掷骰，这里只决定发哪个意图。
    stays: Query<'w, 's, &'static StaySentence>,
}

/// 命中测试给出的实体那一侧：动作、禁用标记、父链。
#[derive(SystemParam)]
struct Widgets<'w, 's> {
    actions: Query<'w, 's, &'static UiAction>,
    disabled: Query<'w, 's, &'static Disabled>,
    parents: Query<'w, 's, &'static ChildOf>,
}

/// 派发的出口。`AppExit` 在 0.20 也是消息，所以能和另几个收在一起；
/// `MessageWriter` 只带一个生命周期（`MessageReader` 才带两个，§六 坑 13）。
#[derive(SystemParam)]
struct Sends<'w> {
    rolls: MessageWriter<'w, RequestRoll>,
    stays: MessageWriter<'w, RequestStay>,
    answers: MessageWriter<'w, DecisionChosen>,
    ops: MessageWriter<'w, BankOpRequested>,
    trades: MessageWriter<'w, StockTradeRequested>,
    cards: MessageWriter<'w, PlayCardRequested>,
    shops: MessageWriter<'w, ShopOpRequested>,
    mortgages: MessageWriter<'w, MortgageRequested>,
    bails: MessageWriter<'w, BailRequested>,
    builds: MessageWriter<'w, BuildRequested>,
    places: MessageWriter<'w, PlaceItemRequested>,
    remotes: MessageWriter<'w, RequestRemoteDice>,
    scores: MessageWriter<'w, MinigameResult>,
    tickets: MessageWriter<'w, BuyTicketRequested>,
    /// 拍卖台上的两个动作共用一条消息（P9）：「加一档」与「放弃」是同一件事的两半 ——
    /// 都是此刻被问的那一位对一个**正在进行**的场作出的反应
    bids: MessageWriter<'w, AuctionMove>,
    exit: MessageWriter<'w, AppExit>,
}

/// 只动 UI 自己状态的那几个开关。
#[derive(SystemParam)]
struct UiState<'w> {
    toggle: ResMut<'w, pause::PauseMenuOpen>,
    selected: ResMut<'w, tile_info::SelectedTile>,
    atm: ResMut<'w, finance::AtmPanelOpen>,
    stock: ResMut<'w, finance::StockPanelOpen>,
    hand: ResMut<'w, hand::HandPanelOpen>,
    items: ResMut<'w, items::ItemsPanelOpen>,
    /// 游乐场那四格（P7）。`arcade` 是「现在开着的那一局」——派发要往里写分数与铲数，
    /// 所以它是 `ResMut` 而不是只有 `sync_arcade` 能碰的私有状态
    arcade: ResMut<'w, minigame::Arcade>,
    practice: ResMut<'w, minigame::Practice>,
    practice_rounds: ResMut<'w, minigame::PracticeRounds>,
    arcade_choice: ResMut<'w, minigame::ArcadeChoiceOpen>,
}

/// 掷骰意图的唯一出口。`dice == 0` = 载具满载，与 [`UiAction::Roll`] 相同；
/// 被停留按住的人一律改发 [`RequestStay`] —— 规则层也会拒掉他的掷骰，
/// 但派发侧发对意图，AI 与恶人那条路才有对应的人类入口（P4④）。
fn send_roll(sends: &mut Sends<'_>, gate: &Gate<'_, '_>, dice: u32) {
    if !can_roll(*gate.phase.get(), &gate.paused, &gate.pending) {
        return;
    }
    let Some(player) = gate.order.as_ref().and_then(|o| o.current()) else {
        return;
    };
    if gate.stays.get(player).is_ok() {
        sends.stays.write(RequestStay { player });
    } else {
        sends.rolls.write(RequestRoll { player, dice });
    }
}

/// 读 `PointerClick`，沿父链找到动作后派发。
///
/// 「能不能做」全部在这里判一次，而不是散到各按钮的构造代码里：按钮只声明自己代表什么。
/// 这也意味着弹层开着时掷骰必然无效 —— 判据是 `PendingDecision`，不是「有没有遮罩」。
/// 规则层还会再判一次（认主语、认窗口），所以这里放行错了也只是白点一下。
fn dispatch_ui_actions(
    mut clicks: MessageReader<PointerClick>,
    gate: Gate<'_, '_>,
    widgets: Widgets,
    mut next_scene: ResMut<NextState<crate::scene::Scene>>,
    mut sends: Sends,
    mut ui: UiState,
) {
    for click in clicks.read() {
        if click.button != PointerButton::Primary {
            continue;
        }
        let Some(entity) = action_owner(&widgets.actions, &widgets.parents, click.entity) else {
            continue;
        };
        if widgets.disabled.get(entity).is_ok() {
            continue;
        }
        let Ok(action) = widgets.actions.get(entity) else {
            continue;
        };
        // 休市只挡股市（P2b）。银行派发不看这一条 —— 柜台全年开
        let market_open = gate
            .session
            .as_ref()
            .is_some_and(|session| is_market_open(&gate.clock, session, &gate.holidays));
        match action {
            UiAction::NewGame => next_scene.set(crate::scene::Scene::Board),
            UiAction::BackToMenu => next_scene.set(crate::scene::Scene::Menu),
            UiAction::Quit => {
                sends.exit.write(AppExit::Success);
            }
            // 掷骰：等轮到人、没暂停、且没有待决事项（判据与顶栏着色同源）。
            // 被停留按住的人（P4④）发的是同一个意图位的「停留」—— 规则层的
            // `roll_dice_system` 会拒掉他的掷骰，所以这里不把两个意图混成一个请求。
            UiAction::Roll => send_roll(&mut sends, &gate, 0),
            // 载具颗数由道具面板给（P5）：与顶栏那颗同一条窗口，也同一条「停留优先」
            UiAction::RollWith(dice) => send_roll(&mut sends, &gate, *dice),
            // 放道具与出牌同一条窗口（「本回合怎么走」在我走之前定完）。目标格合法性
            // 由规则层的 `place_denied` 再判一次 —— 这里只挡窗口与主语
            UiAction::PlaceItem { item, tile } => {
                if !can_play(*gate.phase.get(), &gate.paused, &gate.pending) {
                    continue;
                }
                let Some(player) = gate.order.as_ref().and_then(|o| o.current()) else {
                    continue;
                };
                sends.places.write(PlaceItemRequested {
                    player,
                    item: *item,
                    tile: *tile,
                });
            }
            // 遥控骰子：与掷骰互斥地决定这一回合走几格，窗口也同一条
            UiAction::UseRemoteDice { steps } => {
                if !can_play(*gate.phase.get(), &gate.paused, &gate.pending) {
                    continue;
                }
                let Some(player) = gate.order.as_ref().and_then(|o| o.current()) else {
                    continue;
                };
                sends.remotes.write(RequestRemoteDice {
                    player,
                    steps: *steps,
                });
            }
            // 回答待决事项：主语取自 `PendingDecision`，不是「当前玩家」——
            // 两者在「A 踩了 B 的地、收据挂在 A 名下」这类场合就是不同的人
            UiAction::Decide(answer) => {
                if let Some((player, _)) = gate.pending.active() {
                    sends.answers.write(DecisionChosen {
                        player,
                        action: *answer,
                    });
                }
            }
            // 银行操作：主语是柜台的客人，或 ATM 卡的持有人（两者不会同时挂着别人）
            UiAction::BankOp { op, amount } => {
                if !can_bank_op(*op, &gate.paused, &gate.pending, &gate.atm) {
                    continue;
                }
                if let Some(player) = bank_op_subject(&gate.pending, &gate.atm) {
                    sends.ops.write(BankOpRequested {
                        player,
                        op: *op,
                        amount: *amount,
                    });
                }
            }
            // 股票：只有当前回合玩家在掷骰前能下单，与规则层同一条窗口（休市日没有）
            UiAction::Trade {
                stock,
                side,
                shares,
            } => {
                if !can_trade(*gate.phase.get(), &gate.paused, &gate.pending, market_open) {
                    continue;
                }
                let Some(player) = gate.order.as_ref().and_then(|o| o.current()) else {
                    continue;
                };
                sends.trades.write(StockTradeRequested {
                    player,
                    stock: stock.clone(),
                    side: *side,
                    shares: *shares,
                });
            }
            UiAction::CloseInfo => ui.selected.0 = None,
            UiAction::TogglePause => ui.toggle.flip(),
            UiAction::ToggleAtm => ui.atm.0 = !ui.atm.0,
            UiAction::ToggleStock => ui.stock.0 = !ui.stock.0,
            UiAction::ToggleHand => ui.hand.0 = !ui.hand.0,
            UiAction::ToggleItems => ui.items.0 = !ui.items.0,
            // 出牌：主语仍然取当前回合玩家，而不是按钮里带的谁 —— 手牌面板只画得下
            // 「这个人的手」，规则层 `play_card_system` 还会再认一次主语与窗口
            UiAction::PlayCard { card, aim } => {
                if !can_play(*gate.phase.get(), &gate.paused, &gate.pending) {
                    continue;
                }
                let Some(player) = gate.order.as_ref().and_then(|o| o.current()) else {
                    continue;
                };
                sends.cards.write(PlayCardRequested {
                    player,
                    card: *card,
                    aim: *aim,
                });
            }
            // 百货买卖：主语是**站在柜台前的那个人**，不是「当前回合玩家」也不是按钮里带的谁。
            // 柜台既然开着，回合就停在 `WaitDecision`，所以这一支不必再问阶段；
            // 而「这一笔办不办得动」（有没有货、点券够不够）由面板灰按钮 + 规则层那张表判，
            // 这里只挡窗口 —— 与 `can_bank_op` 同一条分工。
            UiAction::ShopOp { side, card } => {
                if gate.paused.0 {
                    continue;
                }
                let Some(player) = shop_customer(&gate.pending) else {
                    continue;
                };
                sends.shops.write(ShopOpRequested {
                    player,
                    side: *side,
                    card: *card,
                });
            }
            // 买一注走的是**彩票柜台**那个人（P8），与百货同一条形状：暂停挡一下，
            // 其余判据（号码在不在池里、同号第二张、现金够不够）全交给规则层那张
            // `buy_allowed` —— 面板的灰按钮读的也是同一张表。
            UiAction::BuyTicket(number) => {
                if gate.paused.0 {
                    continue;
                }
                let Some(player) = lottery_customer(&gate.pending) else {
                    continue;
                };
                sends.tickets.write(BuyTicketRequested {
                    player,
                    number: *number,
                });
            }
            // 拍卖台上的两颗按钮（P9）。主语取自「此刻挂着窗口的那一位」，而这一格在
            // 一场拍卖里会换人 —— 金额不在这里（由规则层从「最高价 + 一档」算），
            // 「轮不轮到他」「加不加得起」也全交给规则层那张 `bid_allowed`。
            // 这一支不认 `gate.paused`：暂停时那颗按钮本来就点不着（上面 `Disabled` 那一格
            // 与遮罩的命中测试都挡着），而拍卖是规则层在转的一台机器，不是一个可以「先想想」
            // 的报价 —— 暂停期间回合机整个停住，窗口也不会被回答。
            UiAction::Bid(what) => {
                if gate.paused.0 {
                    continue;
                }
                let Some(player) = auction_customer(&gate.pending) else {
                    continue;
                };
                sends.bids.write(AuctionMove {
                    player,
                    what: *what,
                });
            }
            // 抵押与赎回走的是**银行柜台**那个人（`counter_of`），与四种 `BankOp` 同一个窗口。
            // 休市与「这块地办不办得动」都不在这里判：前者 `can_bank_op` 那条路径已经管不到
            // 这一支，所以由规则层的 `mortgage_op_allowed` 挡（它读同一份日历），
            // 后者由面板的灰按钮 + 同一张表挡。
            UiAction::MortgageOp { land, op } => {
                if gate.paused.0 {
                    continue;
                }
                let Some(player) = counter_of(&gate.pending) else {
                    continue;
                };
                sends.mortgages.write(MortgageRequested {
                    player,
                    op: *op,
                    land: *land,
                });
            }
            // 保释：主语是保释窗口挂着的那位（P4③）。「那个人关不关在这里」「点券够不够」
            // 都由规则层的 `bail_op_allowed` 判；这里只认窗口在不在、按的是谁。
            UiAction::Bail { target } => {
                if gate.paused.0 {
                    continue;
                }
                let Some((player, Pending::Bail(_))) = gate.pending.active() else {
                    continue;
                };
                sends.bails.write(BailRequested {
                    player,
                    target: *target,
                });
            }
            // 建设窗口（P10c）：主语是窗口挂着的那位，地块必须正是窗口问的那块 ——
            // 与保释那一格同一条：按钮只说「建哪一类」，主语与对象都由状态说。
            // 「建不建得成」全交给规则层的 `build_op_allowed`（按钮那颗灰是同源结果）。
            UiAction::Build { land, kind } => {
                if gate.paused.0 {
                    continue;
                }
                let Some((player, Pending::Build(window))) = gate.pending.active() else {
                    continue;
                };
                if window != *land {
                    continue;
                }
                sends.builds.write(BuildRequested {
                    player,
                    land: *land,
                    kind: *kind,
                });
            }
            // ===== 游乐场（P7）=====
            // 玩法内的四颗一律先看暂停：暂停菜单压在 `ZIndex(6)`、游乐场在 5，
            // 遮罩本来就点不穿；这一道挡的是「ESC 那一帧的点击已经排进队列」那种时序。
            UiAction::Pop { slot } => {
                if gate.paused.0 {
                    continue;
                }
                minigame::pop(&mut ui.arcade, *slot);
            }
            UiAction::Dig { cell } => {
                if gate.paused.0 {
                    continue;
                }
                minigame::dig(&mut ui.arcade, *cell);
            }
            UiAction::ArcadeQuit => {
                if gate.paused.0 {
                    continue;
                }
                minigame::quit_early(&mut ui.arcade);
            }
            // 「收下」是唯一一处**从表现层往规则层报数**的地方：分数在这里数出来，
            // 点券由 core 那张表算（`minigame_score_system`）。练习局 `result_of` 返回
            // `None`，所以这一支在菜单上什么都不发 —— 而不是发一份 `awarded: false`：
            // 菜单里没有 `ActiveBoard`，`Books` 建不出来，那条消息会躺到下一局的第一个
            // 窗口打开时被读走（跨局残留，§6.1 坑 22/25 的一个新变种）
            UiAction::ArcadeCollect => {
                if gate.paused.0 {
                    continue;
                }
                if let Some(result) = minigame::result_of(&ui.arcade) {
                    sends.scores.write(result);
                }
            }
            // 「再来一局」只加一那个计数：`sync_arcade` 拿它当签名的一部分，
            // 于是连点两次会重开两手（并进 `Practice` 里就做不到 —— 那会被判成同一局）
            UiAction::ArcadeReplay => ui.practice_rounds.0 += 1,
            UiAction::ArcadeLeave => ui.practice.0 = None,
            UiAction::ToggleArcadeChoice => ui.arcade_choice.0 = !ui.arcade_choice.0,
            UiAction::Practice(game) => {
                ui.practice.0 = Some(*game);
                ui.arcade_choice.0 = false;
            }
        }
    }
}

/// 沿父链找第一个带 [`UiAction`] 的祖先，返回那个祖先实体。
///
/// 返回实体而不是动作本身：`UiAction` 从 `Copy` 降级成 `Clone`（`Trade` 带着股票 id），
/// 而派发方本来就要再读一次组件去 `match`，把引用传回去比在这儿拷一份更直。
fn action_owner(
    actions: &Query<&UiAction>,
    parents: &Query<&ChildOf>,
    hit: Entity,
) -> Option<Entity> {
    let mut current = Some(hit);
    while let Some(entity) = current {
        if actions.get(entity).is_ok() {
            return Some(entity);
        }
        current = parents.get(entity).ok().map(|p| p.parent());
    }
    None
}

/// 悬停换色。同样走父链，否则鼠标移到按钮文字上就会「掉色」。
///
/// **顺序要紧：先清算 Out、再应用 Over。** picking 每帧只 hover 最深的一个节点，
/// 指针在按钮里从空白处挪到文字上，同一帧会成对出现 `Out(按钮)` + `Over(文字)`；
/// 若按「先进后出」处理，后到的 Out 会把刚点亮的高亮立刻浇灭 —— 悬停高亮一闪即逝，
/// 稳态是原色（§6.1 坑 24）。先出后进，同帧「离开旧 + 进入新」指向同一按钮时，
/// 最终停在 hover 色；彻底离开时只有 Out，自然复原。
fn hover_feedback(
    mut overs: MessageReader<PointerOver>,
    mut outs: MessageReader<PointerOut>,
    actions: Query<&UiAction>,
    parents: Query<&ChildOf>,
    mut buttons: Query<(&mut BackgroundColor, &Palette, Option<&Disabled>)>,
) {
    let mut repaint = |event_entity: Entity, hover: bool| {
        let Some(entity) = action_owner(&actions, &parents, event_entity) else {
            return;
        };
        if let Ok((mut bg, palette, disabled)) = buttons.get_mut(entity)
            && disabled.is_none()
        {
            bg.0 = if hover { palette.hover } else { palette.normal };
        }
    };
    for e in outs.read() {
        repaint(e.entity, false);
    }
    for e in overs.read() {
        repaint(e.entity, true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::message::Messages;
    use bevy::picking::{
        backend::HitData,
        pointer::{Location, PointerId},
    };
    use bevy::state::app::StatesPlugin;
    use bevy::window::WindowRef;
    use game_core::prelude::{
        Date, Decision, GameKind, GameRules, ItemKind, LandId, Pending, PlayerId, StaySentence,
        TileIndex, TurnOrder,
    };
    use std::time::Duration;

    /// 内部编号 → 屏幕编号那一条映射，本身要钉住：它错位一格，五块屏就同时全都错，
    /// 而症状是「一块空地写着有一层楼」这种没人会去查的话。
    /// 尤其钉 0 与 1 都算空地（无主的 Lv0 与刚买下的 Lv1 在玩家眼里是同一件事），
    /// 以及顶格那一档（`max_building_level` 默认 6 → 建筑 Lv5，租金表恰好六行）。
    #[test]
    fn the_level_label_never_shows_a_bare_plot_as_a_storey() {
        assert_eq!(building_label(0), "空地", "无主空地");
        assert_eq!(building_label(1), "空地", "买地即内部 Lv1，而它没有一层楼");
        assert_eq!(building_label(2), "建筑 Lv1");
        assert_eq!(
            building_label(6),
            "建筑 Lv5",
            "满级那一档少说一层就是全场读错"
        );
    }

    /// 造一条落在 `entity` 上的左键点击。这些字段平时由 picking 后端填，
    /// 测试里没有窗口也没有指针，只能手拼一份最小可信的。
    fn make_click(entity: Entity) -> PointerClick {
        PointerClick {
            entity,
            pointer: Pointer::new(PointerId::Mouse, test_location()),
            button: PointerButton::Primary,
            hit: test_hit(),
            duration: Duration::ZERO,
            count: 1,
        }
    }

    /// 测试用的指针位置（无窗口，字段由 `WindowRef::normalize` 兜出一份合法值）
    fn test_location() -> Location {
        Location {
            target: NormalizedRenderTarget::Window(
                WindowRef::Entity(Entity::PLACEHOLDER)
                    .normalize(None)
                    .expect("占位窗口应当能归一化"),
            ),
            position: Vec2::ZERO,
        }
    }

    fn test_hit() -> HitData {
        HitData {
            camera: Entity::PLACEHOLDER,
            depth: 0.0,
            position: None,
            normal: None,
            extra: None,
        }
    }

    /// 悬停消息与点击同构，共用一份最小可信字段
    fn make_over(entity: Entity) -> PointerOver {
        PointerOver {
            entity,
            pointer: Pointer::new(PointerId::Mouse, test_location()),
            hit: test_hit(),
        }
    }

    fn make_out(entity: Entity) -> PointerOut {
        PointerOut {
            entity,
            pointer: Pointer::new(PointerId::Mouse, test_location()),
            hit: test_hit(),
        }
    }

    /// 派发写出去的消息留在原地，由这条探针收下来供断言。
    #[derive(Resource, Default)]
    struct Seen {
        rolls: Vec<Entity>,
        /// 掷骰请求里带的颗数（P5 载具）。单开一栏而不是改 `rolls` 的类型：那会让
        /// 上面七八条旧断言都跟着改，而它们关心的是「谁发的」
        dice: Vec<(Entity, u32)>,
        stays: Vec<Entity>,
        answers: Vec<(Entity, Decision)>,
        ops: Vec<(Entity, BankOp, i64)>,
        trades: Vec<(Entity, String, TradeSide, u32)>,
        plays: Vec<(Entity, CardId, Option<CardAim>)>,
        shops: Vec<(Entity, TradeSide, CardId)>,
        mortgages: Vec<(Entity, LandId, MortgageOp)>,
        places: Vec<(Entity, ItemKind, TileIndex)>,
        remotes: Vec<(Entity, u32)>,
        /// 彩票柜台买的那一注（P8）：`(谁, 号)`
        tickets: Vec<(Entity, u8)>,
        /// 拍卖台上按下的那一个动作（P9）：`(谁, 加价还是放弃)`。
        /// 主语必须是「此刻挂着窗口的那一位」，而这一位在拍卖过程中会换人 —— 所以这一栏
        /// 记的是「谁发的」而不是「谁该发」
        bids: Vec<(Entity, BidMove)>,
        /// 建设窗口按下的那一类（P10c）：`(谁, 哪块地, 建什么)`。地块也记下来，
        /// 因为按钮里烤着它，而「窗口问的是另一块地」必须被挡掉
        builds: Vec<(Entity, LandId, BuildingKind)>,
    }

    /// 派发出口的读者们。装成参数包而不是八个函数参数：这仓库遇到 `too_many_arguments`
    /// 不挂 `#[allow]` 而是打包（§六 坑 8），而这里加一颗按钮就多一个读者是**预期内的长**，
    /// 不是「这个 system 真的要知道八件事」那种需要被警告吃掉的信号。
    #[derive(SystemParam)]
    struct Readers<'w, 's> {
        rolls: MessageReader<'w, 's, RequestRoll>,
        stays: MessageReader<'w, 's, RequestStay>,
        answers: MessageReader<'w, 's, DecisionChosen>,
        ops: MessageReader<'w, 's, BankOpRequested>,
        trades: MessageReader<'w, 's, StockTradeRequested>,
        plays: MessageReader<'w, 's, PlayCardRequested>,
        shops: MessageReader<'w, 's, ShopOpRequested>,
        mortgages: MessageReader<'w, 's, MortgageRequested>,
        places: MessageReader<'w, 's, PlaceItemRequested>,
        remotes: MessageReader<'w, 's, RequestRemoteDice>,
        tickets: MessageReader<'w, 's, BuyTicketRequested>,
        bids: MessageReader<'w, 's, AuctionMove>,
        builds: MessageReader<'w, 's, BuildRequested>,
    }

    fn collect(mut seen: ResMut<Seen>, mut r: Readers<'_, '_>) {
        for roll in r.rolls.read() {
            seen.rolls.push(roll.player);
            seen.dice.push((roll.player, roll.dice));
        }
        seen.stays.extend(r.stays.read().map(|x| x.player));
        seen.answers
            .extend(r.answers.read().map(|a| (a.player, a.action)));
        seen.ops
            .extend(r.ops.read().map(|o| (o.player, o.op, o.amount)));
        seen.trades.extend(
            r.trades
                .read()
                .map(|t| (t.player, t.stock.0.clone(), t.side, t.shares)),
        );
        seen.plays
            .extend(r.plays.read().map(|p| (p.player, p.card, p.aim)));
        seen.shops
            .extend(r.shops.read().map(|s| (s.player, s.side, s.card)));
        seen.mortgages
            .extend(r.mortgages.read().map(|m| (m.player, m.land, m.op)));
        seen.places
            .extend(r.places.read().map(|p| (p.player, p.item, p.tile)));
        seen.remotes
            .extend(r.remotes.read().map(|p| (p.player, p.steps)));
        seen.tickets
            .extend(r.tickets.read().map(|t| (t.player, t.number)));
        seen.bids.extend(r.bids.read().map(|b| (b.player, b.what)));
        seen.builds
            .extend(r.builds.read().map(|b| (b.player, b.land, b.kind)));
    }

    /// 一套只装配到「能跑 UI 派发」的对局骨架：两名玩家、一颗掷骰按钮（文字是它的子节点）、
    /// 一颗决策按钮、一颗灰掉的按钮、一层没有动作的遮罩，外加详情卡、暂停、
    /// B5b 的银行操作与股票买卖、P3b 的出牌、P3d② 的百货买卖，与三个浮层开关各一颗。
    struct Harness {
        app: App,
        /// `TurnOrder` 的当前玩家
        player: Entity,
        /// 另一名玩家：用来验证「决策的主语是待决玩家，不是当前玩家」
        other: Entity,
        roll_label: Entity,
        decide: Entity,
        disabled_roll: Entity,
        bare: Entity,
        close: Entity,
        pause: Entity,
        deposit: Entity,
        buy: Entity,
        frame: Entity,
        insulate: Entity,
        /// 百货柜台上的一颗「买」按钮（P3d②）
        shop_buy: Entity,
        /// 彩票柜台上的一颗「17 号」（P8）。与 `shop_buy` 同一条形状：号烤在按钮里，
        /// 主语由 `PendingDecision` 给
        ticket_seventeen: Entity,
        /// 拍卖台上那两颗（P9）：「加到 N」与「放弃这一场」。它们**不带金额**，
        /// 因为那个数由规则层从「最高价 + 一档」现算 —— 与上面那颗「17 号」相反的形状
        bid_raise: Entity,
        bid_pass: Entity,
        /// 银行柜台上的一颗「押」按钮（P3d③）
        mortgage_take: Entity,
        /// 建设窗口上的「建购物中心」（P10c）：地块与类别都烤在按钮里
        build_mall: Entity,
        /// 道具面板上的两颗（P5）：放一个路障到 5 号格、用遥控骰子走 3 步
        place_rock: Entity,
        remote_three: Entity,
        /// 带颗数掷骰（载具 2 颗）
        roll_two: Entity,
        toggle_atm: Entity,
        toggle_stock: Entity,
        toggle_hand: Entity,
        toggle_items: Entity,
    }

    impl Harness {
        fn new(phase: TurnPhase, paused: bool, pending: PendingDecision) -> Self {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_plugins(StatesPlugin)
                .init_state::<TurnPhase>()
                .init_state::<crate::scene::Scene>()
                .add_message::<PointerClick>()
                .add_message::<RequestRoll>()
                .add_message::<RequestStay>()
                .add_message::<DecisionChosen>()
                .add_message::<BankOpRequested>()
                .add_message::<StockTradeRequested>()
                .add_message::<PlayCardRequested>()
                .add_message::<ShopOpRequested>()
                .add_message::<MortgageRequested>()
                .add_message::<BailRequested>()
                .add_message::<BuildRequested>()
                .add_message::<PlaceItemRequested>()
                .add_message::<RequestRemoteDice>()
                .add_message::<MinigameResult>()
                .add_message::<BuyTicketRequested>()
                // P9：拍卖台那两颗按钮共用这一条出口（漏了它 = 派发这个 system 建不出来，
                // 于是上面那一整片按钮全死，而不是只死拍卖那两颗）
                .add_message::<AuctionMove>()
                .add_message::<AppExit>()
                .init_resource::<Seen>()
                .init_resource::<tile_info::SelectedTile>()
                .init_resource::<pause::PauseMenuOpen>()
                .init_resource::<finance::AtmPanelOpen>()
                .init_resource::<finance::StockPanelOpen>()
                .init_resource::<hand::HandPanelOpen>()
                .init_resource::<items::ItemsPanelOpen>()
                .init_resource::<minigame::Arcade>()
                .init_resource::<minigame::Practice>()
                .init_resource::<minigame::PracticeRounds>()
                .init_resource::<minigame::ArcadeChoiceOpen>()
                .init_resource::<AtmAccess>()
                .insert_resource(Paused(paused))
                // 休市判据要读日历、锚点与节假日表，三者都由 core 常驻。
                // 锚点钉 2048-01-06（周一），于是 tick 5 = 1 月 11 日（周六）是休市日
                .insert_resource(MarketClock::default())
                .insert_resource(GameRules::default())
                .insert_resource(HolidaySet::default())
                .insert_resource(GameSession {
                    turn_count: 0,
                    seed: 0,
                    price_index: 1.0,
                    epoch: Date::new(2048, 1, 6),
                    last_gift_year: None,
                    last_envelope_year: None,
                })
                .insert_resource(pending)
                .add_systems(Update, (dispatch_ui_actions, collect).chain());

            let player = app.world_mut().spawn(PlayerId(0)).id();
            let other = app.world_mut().spawn(PlayerId(1)).id();
            app.world_mut()
                .insert_resource(TurnOrder::new(vec![player, other]));
            app.world_mut()
                .resource_mut::<NextState<TurnPhase>>()
                .set(phase);

            let roll = app.world_mut().spawn(UiAction::Roll).id();
            let roll_label = app.world_mut().spawn(ChildOf(roll)).id();
            let decide = app
                .world_mut()
                .spawn(UiAction::Decide(Decision::Accept))
                .id();
            let disabled_roll = app.world_mut().spawn((UiAction::Roll, Disabled)).id();
            let bare = app.world_mut().spawn(Node::default()).id();
            let close = app.world_mut().spawn(UiAction::CloseInfo).id();
            let pause = app.world_mut().spawn(UiAction::TogglePause).id();
            let deposit = app
                .world_mut()
                .spawn(UiAction::BankOp {
                    op: BankOp::Deposit,
                    amount: 1_000,
                })
                .id();
            let buy = app
                .world_mut()
                .spawn(UiAction::Trade {
                    stock: StockId("stock_001".to_string()),
                    side: TradeSide::Buy,
                    shares: 10,
                })
                .id();
            // 出牌两颗：一颗带目标（陷害 → 另一名玩家），一颗不带（免罪给自己）
            let frame = app
                .world_mut()
                .spawn(UiAction::PlayCard {
                    card: CardId(0),
                    aim: Some(CardAim::Player(other)),
                })
                .id();
            let insulate = app
                .world_mut()
                .spawn(UiAction::PlayCard {
                    card: CardId(1),
                    aim: None,
                })
                .id();
            // 百货那颗「买 30」：主语不写在按钮里，由 `PendingDecision` 给（同 ATM）
            let shop_buy = app
                .world_mut()
                .spawn(UiAction::ShopOp {
                    side: TradeSide::Buy,
                    card: CardId(0),
                })
                .id();
            // 彩票那颗「17 号」（P8）：与百货同一形 —— 号在按钮里，主语在 `PendingDecision` 里
            let ticket_seventeen = app.world_mut().spawn(UiAction::BuyTicket(17)).id();
            // 拍卖台那两颗（P9）：动作在按钮里，金额与主语都不在
            let bid_raise = app.world_mut().spawn(UiAction::Bid(BidMove::Raise)).id();
            let bid_pass = app.world_mut().spawn(UiAction::Bid(BidMove::Withdraw)).id();
            let toggle_atm = app.world_mut().spawn(UiAction::ToggleAtm).id();
            // 柜台那颗「押 750」：与百货同一形 —— 主语在 `PendingDecision` 里，金额在规则层算
            let mortgage_take = app
                .world_mut()
                .spawn(UiAction::MortgageOp {
                    land: LandId(0),
                    op: MortgageOp::Take,
                })
                .id();
            let toggle_stock = app.world_mut().spawn(UiAction::ToggleStock).id();
            // 建设窗口那颗「建购物中心」（P10c）：与抵押同一形 —— 意图在按钮里，主语在窗口里
            let build_mall = app
                .world_mut()
                .spawn(UiAction::Build {
                    land: LandId(2),
                    kind: BuildingKind::Mall,
                })
                .id();
            let toggle_hand = app.world_mut().spawn(UiAction::ToggleHand).id();
            let toggle_items = app.world_mut().spawn(UiAction::ToggleItems).id();
            let place_rock = app
                .world_mut()
                .spawn(UiAction::PlaceItem {
                    item: ItemKind::Roadblock,
                    tile: TileIndex(5),
                })
                .id();
            let remote_three = app
                .world_mut()
                .spawn(UiAction::UseRemoteDice { steps: 3 })
                .id();
            let roll_two = app.world_mut().spawn(UiAction::RollWith(2)).id();

            // 先把阶段落实，再点击才不会落在 `Boot` 上
            app.update();
            Self {
                app,
                player,
                other,
                roll_label,
                decide,
                disabled_roll,
                bare,
                close,
                pause,
                deposit,
                buy,
                frame,
                insulate,
                shop_buy,
                ticket_seventeen,
                bid_raise,
                bid_pass,
                mortgage_take,
                build_mall,
                place_rock,
                remote_three,
                roll_two,
                toggle_atm,
                toggle_stock,
                toggle_hand,
                toggle_items,
            }
        }

        /// 把 ATM 卡发到某人手里。
        fn set_atm(&mut self, holder: Entity) {
            self.app.world_mut().resource_mut::<AtmAccess>().0 = Some(holder);
        }

        fn offer(&mut self, player: Entity, about: Pending) {
            self.app
                .world_mut()
                .resource_mut::<PendingDecision>()
                .offer(player, about);
        }

        /// 把日历拨到第 `tick` 天（一个 tick = 一天 = 一次回合结束）。
        fn set_tick(&mut self, tick: u32) {
            self.app.world_mut().resource_mut::<MarketClock>().tick = tick;
        }

        /// 点一格实体，跑两帧收消息。
        fn click(&mut self, target: Entity) {
            self.app
                .world_mut()
                .resource_mut::<Messages<PointerClick>>()
                .write(make_click(target));
            self.app.update();
            self.app.update();
        }

        fn seen(&self) -> &Seen {
            self.app.world().resource::<Seen>()
        }
    }

    #[test]
    fn clicking_the_text_inside_the_roll_button_rolls() {
        // 命中测试报的是**最深的**节点，也就是按钮里的文字，所以父链必须走通
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.roll_label);
        assert_eq!(h.seen().rolls, vec![h.player]);
    }

    #[test]
    fn a_pending_decision_blocks_the_roll() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = h.other;
        h.app
            .world_mut()
            .resource_mut::<PendingDecision>()
            .offer(other, Pending::Buy(LandId(3)));
        h.click(h.roll_label);
        assert!(h.seen().rolls.is_empty(), "还有待决事项就掷骰了");
    }

    #[test]
    fn pausing_blocks_the_roll() {
        let mut h = Harness::new(TurnPhase::WaitRoll, true, PendingDecision::default());
        h.click(h.roll_label);
        assert!(h.seen().rolls.is_empty(), "暂停时不该放行掷骰");
    }

    #[test]
    fn a_staying_player_sends_the_stay_intent_from_the_same_button() {
        // P4④：按钮还是那一颗，判据是「当前这位身上有没有停留计时」。
        // 规则层也会拒掉被停留者的掷骰，但派发侧必须发对意图 —— 否则 AI/恶人那边的
        // `RequestStay` 就没有人类这条入口对应
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.app
            .world_mut()
            .entity_mut(h.player)
            .insert(StaySentence { turns_left: 1 });
        h.click(h.roll_label);
        assert!(h.seen().rolls.is_empty(), "被按住的人不该发掷骰");
        assert_eq!(h.seen().stays, vec![h.player]);
    }

    /// 放道具与出牌同一条窗口、同一个主语（P5）。按钮里烤的是「哪件、哪一格」，
    /// 「这一格能不能放」由规则层再判 —— 派发只负责把意图送对。
    #[test]
    fn a_place_request_names_the_current_player_and_needs_the_roll_window() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let me = h.player;
        h.click(h.place_rock);
        assert_eq!(
            h.seen().places,
            vec![(me, ItemKind::Roadblock, TileIndex(5))],
            "掷骰前那颗「5 号格」该派发，且主语是当前玩家"
        );

        let mut paused = Harness::new(TurnPhase::WaitRoll, true, PendingDecision::default());
        paused.click(paused.place_rock);
        assert!(paused.seen().places.is_empty(), "暂停时不该放道具");

        let mut after = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = after.other;
        after.offer(other, Pending::RentNotice(LandId(3)));
        after.click(after.place_rock);
        assert!(
            after.seen().places.is_empty(),
            "回合卡在决策上，还能往路上放东西？"
        );
    }

    /// 遥控骰子与掷骰互斥地决定这一回合走几格，窗口与主语同一条。
    #[test]
    fn a_remote_dice_request_carries_its_steps_and_the_current_player() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let me = h.player;
        h.click(h.remote_three);
        assert_eq!(h.seen().remotes, vec![(me, 3)]);

        let mut later = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = later.other;
        later.offer(other, Pending::RentNotice(LandId(3)));
        later.click(later.remote_three);
        assert!(later.seen().remotes.is_empty(), "走完路还能遥控骰子？");
    }

    /// 载具颗数（P5）：`RollWith(2)` 送的是同一个 `RequestRoll`，只是 `dice = 2`；
    /// 被停留按住时与顶栏那颗一样改发「停留」，颗数因此不该漏出去。
    #[test]
    fn a_roll_with_dice_count_still_yields_to_the_stay_intent() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let me = h.player;
        h.click(h.roll_two);
        assert_eq!(h.seen().rolls, vec![me]);
        assert_eq!(h.seen().dice, vec![(me, 2)], "颗数没有送到规则层");

        let mut staying = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        staying
            .app
            .world_mut()
            .entity_mut(staying.player)
            .insert(StaySentence { turns_left: 1 });
        staying.click(staying.roll_two);
        assert!(staying.seen().rolls.is_empty(), "被停留按住还掷骰");
        assert_eq!(staying.seen().stays, vec![staying.player]);
    }

    #[test]
    fn a_disabled_button_does_not_dispatch() {
        // 阶段是对的、判据也是通的，唯一拦路的是 `Disabled` —— 顶栏着色与派发共用它
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.disabled_roll);
        assert!(h.seen().rolls.is_empty());
    }

    #[test]
    fn an_entity_without_an_action_is_dropped() {
        // 遮罩就是这一类：命中停在它身上，事件不会穿到顶栏按钮上
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.bare);
        assert!(h.seen().rolls.is_empty());
    }

    #[test]
    fn deciding_answers_for_the_pending_player_not_the_current_one() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = h.other;
        h.app
            .world_mut()
            .resource_mut::<PendingDecision>()
            .offer(other, Pending::RentNotice(LandId(3)));
        h.click(h.decide);
        assert_eq!(h.seen().answers, vec![(other, Decision::Accept)]);
    }

    #[test]
    fn the_ui_only_actions_write_ui_state() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.app
            .world_mut()
            .resource_mut::<tile_info::SelectedTile>()
            .0 = Some((1, 1));

        h.click(h.close);
        assert_eq!(h.app.world().resource::<tile_info::SelectedTile>().0, None);

        h.click(h.pause);
        assert!(h.app.world().resource::<pause::PauseMenuOpen>().0);
    }

    /// ATM 认人不认回合：卡是甲的，就该由甲办，哪怕现在轮到乙行动。
    /// 这条如果写成「主语 = 当前回合玩家」，路过银行那一格的人反而永远花不到自己的卡 ——
    /// 那正是这条路最初设想失败的地方。
    #[test]
    fn an_atm_op_names_the_card_holder_not_the_current_player() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let holder = h.other;
        h.set_atm(holder);
        h.click(h.deposit);
        assert_eq!(h.seen().ops, vec![(holder, BankOp::Deposit, 1_000)]);
    }

    /// 没卡又没柜台 = 一点都没发生。UI 不该替规则层「先记一笔再说」。
    #[test]
    fn a_bank_op_with_no_window_is_dropped() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.deposit);
        assert!(h.seen().ops.is_empty(), "两个窗口都不在还派发出操作");
    }

    /// 柜台与卡分属两人时（同帧交接的那一瞬间真会出现），主语归柜台。
    /// 柜台是规则层主动挂上来的「正在等你」，卡只是地理事实。
    #[test]
    fn the_counter_outranks_the_card_for_the_subject() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let customer = h.player;
        let holder = h.other;
        h.set_atm(holder);
        h.offer(customer, Pending::BankTeller);
        h.click(h.deposit);
        assert_eq!(h.seen().ops, vec![(customer, BankOp::Deposit, 1_000)]);
    }

    /// 交易只在掷骰之前。走完路再下单 = 事后补决策，规则层的窗口判据也是这么写的。
    #[test]
    fn a_trade_only_goes_through_before_the_roll() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let me = h.player;
        h.click(h.buy);
        assert_eq!(
            h.seen().trades,
            vec![(me, "stock_001".to_string(), TradeSide::Buy, 10)],
            "掷骰前那颗按钮该下单，且主语是当前玩家"
        );

        let mut later = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = later.other;
        later.offer(other, Pending::RentNotice(LandId(3)));
        later.click(later.buy);
        assert!(
            later.seen().trades.is_empty(),
            "已经掷过骰、回合卡在决策上，还能下单？"
        );
    }

    /// 暂停也要挡住交易：判据与掷骰同源（`can_trade` 就是 `can_roll`）。
    #[test]
    fn pausing_blocks_a_trade_request() {
        let mut h = Harness::new(TurnPhase::WaitRoll, true, PendingDecision::default());
        h.click(h.buy);
        assert!(h.seen().trades.is_empty(), "暂停时不该放行下单");
    }

    /// P2b：休市只挡股市、银行全年无休。派发这一层必须跟着分叉 ——
    /// 若把银行也挡下，周末走进柜台会成为「按不动且没有理由」那类故障（§6.1 坑 27）。
    #[test]
    fn a_closed_market_drops_trades_but_never_bank_ops() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let holder = h.player;
        h.set_atm(holder);
        h.click(h.deposit);
        h.click(h.buy);
        assert_eq!(h.seen().ops, vec![(holder, BankOp::Deposit, 1_000)]);
        assert_eq!(h.seen().trades.len(), 1, "工作日这两笔都该办成");

        // 锚点 2048-01-06（周一）→ tick 5 = 1 月 11 日（周六）
        h.set_tick(5);
        h.click(h.deposit);
        h.click(h.buy);
        let seen = h.seen();
        assert_eq!(seen.ops.len(), 2, "银行全年无休，周末存款该照办");
        assert_eq!(seen.trades.len(), 1, "休市那天还受理了一笔下单");
    }

    #[test]
    fn bank_ops_are_never_gated_by_the_calendar_but_trades_are() {
        let paused = Paused(false);
        let pending = PendingDecision::default();
        let atm = AtmAccess(Some(Entity::from_raw_u32(7).expect("7 不是保留值")));
        // 银行四个窗口与日历无关：有卡/柜台就亮
        for op in [
            BankOp::Deposit,
            BankOp::Withdraw,
            BankOp::Borrow,
            BankOp::Repay,
        ] {
            let needs_counter = matches!(op, BankOp::Borrow | BankOp::Repay);
            assert_eq!(
                can_bank_op(op, &paused, &pending, &atm),
                !needs_counter,
                "{op:?} 的窗口判据漂了"
            );
        }
        // 股市反过来：休市就不亮
        assert!(can_trade(TurnPhase::WaitRoll, &paused, &pending, true));
        assert!(!can_trade(TurnPhase::WaitRoll, &paused, &pending, false));
    }

    #[test]
    fn the_floating_panels_toggle_from_their_own_buttons() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.toggle_atm);
        assert!(h.app.world().resource::<finance::AtmPanelOpen>().0);
        h.click(h.toggle_stock);
        assert!(h.app.world().resource::<finance::StockPanelOpen>().0);
        h.click(h.toggle_hand);
        assert!(h.app.world().resource::<hand::HandPanelOpen>().0);
        h.click(h.toggle_items);
        assert!(h.app.world().resource::<items::ItemsPanelOpen>().0);
        h.click(h.toggle_atm);
        assert!(!h.app.world().resource::<finance::AtmPanelOpen>().0);
    }

    /// 出牌与下单共用同一条窗口，所以派发的判据也必须一致：**主语是当前回合玩家**，
    /// 而目标由按钮带着走（那颗「→ 孙小美」本身就是一整个意图，规则层不必再挂一个
    /// 「选目标」的待决事项）。
    #[test]
    fn a_play_card_request_carries_the_button_aim_and_names_the_current_player() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        let me = h.player;
        let foe = h.other;
        h.click(h.frame);
        assert_eq!(
            h.seen().plays,
            vec![(me, CardId(0), Some(CardAim::Player(foe)))],
            "掷骰前那颗带目标的按钮该派发，且主语是当前玩家"
        );

        // 给自己的卡不带目标：`aim` 必须是 `None`，规则层才不会把它当成一次非法目标拒掉
        h.click(h.insulate);
        let seen = h.seen();
        assert_eq!(seen.plays.len(), 2);
        assert_eq!(seen.plays[1], (me, CardId(1), None));
    }

    /// 走完路就不能出牌了 —— 出牌是「本回合打算怎么走」的一部分，与下单同一个时机。
    #[test]
    fn a_card_cannot_be_played_after_the_roll() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = h.other;
        h.offer(other, Pending::RentNotice(LandId(3)));
        h.click(h.frame);
        assert!(
            h.seen().plays.is_empty(),
            "回合卡在决策上，还能替当前玩家出牌？"
        );
    }

    #[test]
    fn pausing_blocks_a_card_request() {
        let mut h = Harness::new(TurnPhase::WaitRoll, true, PendingDecision::default());
        h.click(h.frame);
        assert!(h.seen().plays.is_empty(), "暂停时不该放行出牌");
    }

    /// 百货的买卖只认「站在柜台前的那个人」：主语取自 `PendingDecision`，与 ATM 同形 ——
    /// 柜台开着的人可能不是当前回合玩家（同帧交接那一瞬），所以按钮不带句柄更安全。
    #[test]
    fn a_shop_op_names_the_counter_customer_and_needs_a_counter() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        h.click(h.shop_buy);
        assert!(h.seen().shops.is_empty(), "没有柜台也派发了买卖请求");

        let customer = h.player;
        h.offer(customer, Pending::Shop);
        h.click(h.shop_buy);
        assert_eq!(
            h.seen().shops,
            vec![(customer, TradeSide::Buy, CardId(0))],
            "柜台开着的那位才是这笔买卖的主语"
        );

        // 暂停挡住一切：柜台虽然还挂着，人先把游戏按停了就不该再花钱
        let mut held = Harness::new(TurnPhase::WaitDecision, true, PendingDecision::default());
        let shopper = held.player;
        held.offer(shopper, Pending::Shop);
        held.click(held.shop_buy);
        assert!(held.seen().shops.is_empty(), "暂停时还派发了买卖请求");
    }

    /// 彩票柜台（P8）与百货同一条形状，但它守的是另一件事：**两个柜台共用一套派发**，
    /// 要是 `lottery_customer` 错写成了 `shop_customer`，在百货柜台里按下一注就会
    /// 花掉彩票的钱 —— 而两边的面板看着都正常，只有账对不上。
    #[test]
    fn a_ticket_names_the_lottery_customer_not_the_shop_one() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        h.click(h.ticket_seventeen);
        assert!(
            h.seen().tickets.is_empty(),
            "没有柜台也派发了买券请求 —— 凭空就能买一注"
        );

        let customer = h.player;
        h.offer(customer, Pending::Lottery);
        h.click(h.ticket_seventeen);
        assert_eq!(
            h.seen().tickets,
            vec![(customer, 17)],
            "柜台开着的那位才是这一注的主语"
        );

        // 百货柜台开着的时候按彩票那颗：不该办成（两条 customer 是两张不同的窗口）
        let mut other_window =
            Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let shopper = other_window.player;
        other_window.offer(shopper, Pending::Shop);
        other_window.click(other_window.ticket_seventeen);
        assert!(
            other_window.seen().tickets.is_empty(),
            "站在百货柜台前的人不该能买彩票"
        );

        // 暂停挡住一切：与百货同一条
        let mut paused = Harness::new(TurnPhase::WaitDecision, true, PendingDecision::default());
        let buyer = paused.player;
        paused.offer(buyer, Pending::Lottery);
        paused.click(paused.ticket_seventeen);
        assert!(paused.seen().tickets.is_empty(), "暂停时还派发了买券请求");
    }

    /// 拍卖台那两颗按钮（P9）只认「此刻挂着窗口的那一位」。
    ///
    /// 它是这场验收里最便宜的一条牙：**拍卖与彩票柜台、百货柜台共用同一套派发**，
    /// 而 `auction_customer` 若错写成 `lottery_customer`，症状不是报错而是「在彩票柜台里
    /// 喊了一声价」—— 面板看着都正常，钱却从拍卖那台机器上走过。反过来（拍卖窗口里买彩票）
    /// 也一样要挡住，所以两类窗口各验一遍。
    #[test]
    fn a_bid_names_the_bidder_and_needs_a_lot_on_the_block() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        h.click(h.bid_raise);
        assert!(
            h.seen().bids.is_empty(),
            "没有拍卖也派发了出价 —— 凭空就能拍下一块地"
        );

        // 窗口挂在**另一个人**身上：主语取窗口，不取当前回合玩家。
        // 这一条与柜台那几处相反 —— 柜台前的人通常就是当前玩家，而拍卖一圈一圈问不同的人，
        // 「当前玩家」与「被问的人」在一场拍卖里**注定会分开**，取错就是替别人喊价
        let bidder = h.other;
        h.offer(bidder, Pending::Auction);
        h.click(h.bid_raise);
        assert_eq!(
            h.seen().bids,
            vec![(bidder, BidMove::Raise)],
            "被问的那一位才是这一声的主语，不是当前回合玩家"
        );
        h.click(h.bid_pass);
        assert_eq!(
            h.seen().bids.last(),
            Some(&(bidder, BidMove::Withdraw)),
            "「放弃」走的是同一条消息"
        );

        // 彩票柜台开着时按拍卖那颗：不该办成（两张不同的窗口）
        let mut counter = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let buyer = counter.player;
        counter.offer(buyer, Pending::Lottery);
        counter.click(counter.bid_raise);
        assert!(
            counter.seen().bids.is_empty(),
            "站在彩票柜台前的人不该能喊价"
        );

        // 暂停挡住一切：与柜台同一条
        let mut paused = Harness::new(TurnPhase::WaitDecision, true, PendingDecision::default());
        let stuck = paused.other;
        paused.offer(stuck, Pending::Auction);
        paused.click(paused.bid_raise);
        assert!(paused.seen().bids.is_empty(), "暂停时还派发了出价");
    }

    /// 抵押与赎回走的是**银行**那个人（`counter_of`）。它与上面那条同形，但守的是另一件事：
    /// 两个柜台共用一个 `Pending` 家族，派发要是错用了 `shop_customer`，面板上按「押」
    /// 就会在百货柜台里办成银行的事。
    #[test]
    fn a_mortgage_op_names_the_bank_customer_not_the_shop_one() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        h.click(h.mortgage_take);
        assert!(
            h.seen().mortgages.is_empty(),
            "没有柜台也派发了抵押请求 —— 凭空就能变现一块地"
        );

        let customer = h.player;
        h.offer(customer, Pending::BankTeller);
        h.click(h.mortgage_take);
        assert_eq!(
            h.seen().mortgages,
            vec![(customer, LandId(0), MortgageOp::Take)],
            "柜台开着的那位才是这笔抵押的主语"
        );

        // 百货柜台开着不算银行柜台：同一颗按钮，换个窗口就该被挡下
        let mut shop = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let shopper = shop.player;
        shop.offer(shopper, Pending::Shop);
        shop.click(shop.mortgage_take);
        assert!(
            shop.seen().mortgages.is_empty(),
            "站在百货柜台里就把地押给了银行"
        );

        // 暂停挡住一切（与买卖那一条同判据）
        let mut held = Harness::new(TurnPhase::WaitDecision, true, PendingDecision::default());
        let banker = held.player;
        held.offer(banker, Pending::BankTeller);
        held.click(held.mortgage_take);
        assert!(held.seen().mortgages.is_empty(), "暂停时还派发了抵押请求");
    }

    /// 建设窗口那颗「建购物中心」（P10c）走的是与保释/抵押同一条 M 形：
    /// **类别与地块烤在按钮里，主语由窗口给**。三件要验的事各是一种错法：
    /// 没有窗口就建（凭空在别人的地上盖楼）、窗口问的是另一块地（按钮里的地块作数）、
    /// 暂停时还建。判据的第三层（够不够钱、押没押、查封没查封）不在这里 ——
    /// 那是 `Books::build_probe` 的事，`tests/commercial.rs` 有正面断言。
    #[test]
    fn a_build_request_names_the_window_owner_and_the_lot_the_button_carries() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        h.click(h.build_mall);
        assert!(
            h.seen().builds.is_empty(),
            "没有建设窗口也派发了建造请求 —— 凭空给自己盖了座楼"
        );

        let owner = h.player;
        h.offer(owner, Pending::Build(LandId(2)));
        h.click(h.build_mall);
        assert_eq!(
            h.seen().builds,
            vec![(owner, LandId(2), BuildingKind::Mall)],
            "窗口挂着的那位才是这一笔建造的主语"
        );

        // 窗口问的是另一块地：按钮里那颗地块不认账（否则同一颗按钮能盖到全盘）
        let mut elsewhere =
            Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = elsewhere.player;
        elsewhere.offer(other, Pending::Build(LandId(3)));
        elsewhere.click(elsewhere.build_mall);
        assert!(
            elsewhere.seen().builds.is_empty(),
            "窗口问的是 L3、按钮写的是 L2，两处对不上就该整笔丢掉"
        );

        // 暂停挡住（与抵押同判据）
        let mut held = Harness::new(TurnPhase::WaitDecision, true, PendingDecision::default());
        let paused_one = held.player;
        held.offer(paused_one, Pending::Build(LandId(2)));
        held.click(held.build_mall);
        assert!(held.seen().builds.is_empty(), "暂停时还派发了建造请求");
    }

    /// 银行柜台与百货柜台是**两个**窗口，所以 `counter_of` 与 `shop_customer` 不能合成
    /// 一个「任何窗口」：并起来的第一条被放宽的就是「站在银行里能买百货」这种不该存在的口子。
    #[test]
    fn the_two_counters_do_not_open_each_other() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let banker = h.player;
        h.set_atm(banker);
        h.offer(banker, Pending::BankTeller);
        h.click(h.shop_buy);
        assert!(h.seen().shops.is_empty(), "在银行柜台里就能买百货？");

        // 反过来同样：百货柜台开着时银行那八颗一律不亮（`can_bank_op` 的 `other_modal_up`）
        let mut other = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let shopper = other.player;
        other.set_atm(shopper);
        other.offer(shopper, Pending::Shop);
        other.click(other.deposit);
        assert!(other.seen().ops.is_empty(), "百货柜台里还能办银行存款");
    }

    /// 窗口判据与掷骰**同一条**，所以要一致到边界上每一个组合都一样 —— 这不是重复
    /// `can_roll` 的测试，而是钉住「出牌窗口 = 掷骰窗口」这个断言本身（P3c 给地块卡
    /// 开新时机时，会有一条不同的组合在这里红出来，那就是要重新拍板的时候）。
    #[test]
    fn the_play_window_is_exactly_the_roll_window() {
        let paused_on = Paused(true);
        let paused_off = Paused(false);
        let mut open = PendingDecision::default();
        open.offer(
            Entity::from_raw_u32(9).expect("9 不是保留值"),
            Pending::Card,
        );

        for phase in [
            TurnPhase::Boot,
            TurnPhase::TurnStart,
            TurnPhase::WaitRoll,
            TurnPhase::Walking,
            TurnPhase::Halted,
            TurnPhase::Landed,
            TurnPhase::WaitDecision,
            TurnPhase::TurnEnd,
            TurnPhase::Over,
        ] {
            for paused in [&paused_off, &paused_on] {
                for pending in [PendingDecision::default(), open] {
                    assert_eq!(
                        can_play(phase, paused, &pending),
                        can_roll(phase, paused, &pending),
                        "{phase:?} / paused={} / pending={:?} 两条窗口判据漂了",
                        paused.0,
                        pending.active().map(|(_, about)| about)
                    );
                }
            }
        }
    }

    /// 真实接线的 headless 应用：UiPlugin + ScenePlugin 都装上，补齐 `CoreGamePlugin`
    /// 平时注册的那批资源，没有棋盘、没有窗口。
    ///
    /// 为什么要走真接线而不是手搓 system：坑 23 那类错误是「挂错集合/挂错门禁」，
    /// 只有按生产环境的同一份接线才暴露得出来。
    fn wired() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            // `step_arcade` 要读键盘（接财宝的 ← →）。`MinimalPlugins` 不带 `InputPlugin`，
            // 少了它不是「键盘那半条不灵」而是这条 system 整个建不出来 —— 与消息同一条坑
            .add_plugins(bevy::input::InputPlugin)
            .add_plugins(StatesPlugin)
            .add_plugins(crate::scene::ScenePlugin)
            .add_plugins(UiPlugin)
            // `dispatch_ui_actions` 依赖的资源平时由 CoreGamePlugin 注册；
            // headless 下按同一路径补齐（都常驻，与菜单态一致）
            .init_state::<TurnPhase>()
            .insert_resource(Paused::default())
            .insert_resource(PendingDecision::default())
            .init_resource::<AtmAccess>()
            // P7：`sync_arcade` 读那一手布局种子（对局里由 core 发下来）
            .init_resource::<game_core::prelude::ArcadeDeal>()
            // P2④ 起派发要读日历判休市。这两条在真实应用里由 `CoreGamePlugin`
            // `init_resource` 常驻，菜单态也在 —— 少了它们崩的是测试，多的那条路径崩的会是玩家
            .init_resource::<MarketClock>()
            .init_resource::<GameRules>()
            .init_resource::<HolidaySet>()
            .add_message::<PointerClick>()
            .add_message::<RequestRoll>()
            .add_message::<RequestStay>()
            .add_message::<DecisionChosen>()
            .add_message::<BankOpRequested>()
            .add_message::<StockTradeRequested>()
            .add_message::<PlayCardRequested>()
            // P3d②：派发多了百货那一个出口。漏一条不是「那个按钮不灵」而是
            // `dispatch_ui_actions` **整条不跑**（参数校验失败，症状是所有按钮都死）
            .add_message::<ShopOpRequested>()
            // P3d③ 同一条坑，两遍：`MortgageRequested` 是派发的出口，`MortgageDone` 是
            // 顶栏提示行读的 —— 少任何一条，崩的都是「一片按钮没反应」而不是这一个
            .add_message::<MortgageRequested>()
            .add_message::<game_core::prelude::MortgageDone>()
            // P4③ 的两个出口：请求由派发写、成交由顶栏提示行读
            .add_message::<BailRequested>()
            .add_message::<game_core::prelude::PlayerBailed>()
            // P10c：建设请求与它带来的两条凭据（顶栏那两行播报读的就是它们）
            .add_message::<BuildRequested>()
            .add_message::<game_core::prelude::FacilityBuilt>()
            .add_message::<game_core::prelude::ServiceCharged>()
            .add_message::<game_core::prelude::CompanyCharged>()
            // P5：道具与遥控骰子的请求，以及顶栏提示行要读的那一批成交/中断凭据。
            // 漏任何一条不是「那行字不出现」，而是 `dispatch_ui_actions` 或
            // `hint_from_events` 整个 SystemParam 建不出来 —— 一片按钮全死
            .add_message::<PlaceItemRequested>()
            .add_message::<RequestRemoteDice>()
            .add_message::<game_core::prelude::ItemPlaced>()
            .add_message::<game_core::prelude::WalkHalted>()
            .add_message::<game_core::prelude::BombTransferred>()
            .add_message::<game_core::prelude::ChestOpened>()
            .add_message::<game_core::prelude::RemoteDiceUsed>()
            .add_message::<game_core::prelude::MoveCapped>()
            .add_message::<game_core::prelude::MountChanged>()
            // P7：游乐场报成绩那一个出口（漏了它同样是「一片按钮全死」）
            .add_message::<MinigameResult>()
            .add_message::<BuyTicketRequested>()
            .add_message::<AuctionMove>()
            .add_message::<game_core::prelude::CompanyCharged>()
            // P2b：日历事件的表现层读这两条（礼物一屏、红包老虎机）
            .add_message::<game_core::prelude::GiftsGiven>()
            .add_message::<game_core::prelude::RedEnvelope>()
            .add_message::<AppExit>()
            // hover_feedback 读的两条 picking 消息，真实应用由 picking 后端注册
            .add_message::<PointerOver>()
            .add_message::<PointerOut>()
            // `spawn_menu` 要 `Res<DevFont>`（0.20 缺资源不再静默跳过而是 panic）；
            // 占位句柄即可 —— headless 不渲染，只求菜单装配系统跑得过去
            .insert_resource(crate::render::DevFont(Handle::default()));
        app
    }

    /// 回归：派发曾被挂进 `UiSet::Live`（门是 `resource_exists::<ActiveBoard>`），
    /// 而主菜单上没有 ActiveBoard —— 「新游戏」「退出」点了没反应，只能点窗口关闭。
    /// B5 验收测不到它：`NM_MENULOOP` 直改 Scene 状态，绕过了派发系统。
    ///
    /// 所以这里必须走**真实 Plugin 接线**（不是上面手搓 dispatch 的 Harness）：
    /// 菜单态、无 ActiveBoard，点一颗带动作的按钮，必须能切进对局。
    #[test]
    fn the_menu_dispatches_without_an_active_board() {
        let mut app = wired();

        // picking 后端 headless 不跑，按钮点击手拼一份最小可信的消息
        let button = app.world_mut().spawn(UiAction::NewGame).id();
        app.world_mut()
            .resource_mut::<Messages<PointerClick>>()
            .write(make_click(button));
        app.update();
        app.update();

        assert_eq!(
            app.world().resource::<State<crate::scene::Scene>>().get(),
            &crate::scene::Scene::Board,
            "菜单点击没被派发 —— dispatch 是否又被挂回了 ActiveBoard 门后？"
        );
    }

    /// 坑 22 那条判据（「换一局时它归谁清」）对三块浮层同样成立：
    /// 面板实体随 `BoardUi` 被收走，而「开着吗」存在资源里。留着不清，
    /// 下一局第一帧就浮着上一局的 ATM、行情与手牌 —— 不报错，只是数字是别人的。
    #[test]
    fn going_back_to_the_menu_closes_all_floating_panels() {
        let mut app = wired();
        // 进对局、把三块面板都打开（真实点击走的也是这三个开关）
        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Board);
        app.update();
        app.insert_resource(finance::AtmPanelOpen(true));
        app.insert_resource(finance::StockPanelOpen(true));
        app.insert_resource(hand::HandPanelOpen(true));
        app.insert_resource(items::ItemsPanelOpen(true));

        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Menu);
        for _ in 0..3 {
            app.update();
        }

        assert!(
            !app.world().resource::<finance::AtmPanelOpen>().0,
            "回主菜单没关 ATM 浮层"
        );
        assert!(
            !app.world().resource::<finance::StockPanelOpen>().0,
            "回主菜单没关证券浮层"
        );
        assert!(
            !app.world().resource::<hand::HandPanelOpen>().0,
            "回主菜单没关手牌浮层"
        );
        assert!(
            !app.world().resource::<items::ItemsPanelOpen>().0,
            "回主菜单没关道具浮层"
        );
    }

    /// 回归：真鼠标点「返回主菜单」会让进程直接崩掉（exit 101）。清场在同一帧里既
    /// `remove_resource` 又把 `NextState<TurnPhase>` 设成 `Boot`，可命令是立刻生效、
    /// 状态转移要到下一帧 —— 于是那一帧的后半程 `TurnPhase` 还写着 `WaitRoll`，
    /// `roll_dice_system` 照跑，却拿不到它的 `TurnOrder`（Bevy 0.20 把参数校验失败
    /// 直接 panic）。`NM_MENULOOP` 挡不住它：从菜单出发第一步只是自转，那半程没走过。
    ///
    /// 这里断言的是**顺序**而不是崩溃本身（这个 harness 没装 core 的 system，崩不了）：
    /// 回合机停稳之前资源必须还在，停稳之后才收走。
    #[test]
    fn the_match_resources_outlive_the_turn_machine() {
        let mut app = wired();
        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Board);
        app.update();
        app.insert_resource(TurnOrder::new(Vec::new()));
        app.world_mut()
            .resource_mut::<NextState<TurnPhase>>()
            .set(TurnPhase::WaitRoll);
        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Menu);

        let mut frames = 0;
        while app.world().contains_resource::<TurnOrder>() && frames < 5 {
            app.update();
            frames += 1;
        }

        assert_eq!(
            app.world().resource::<State<TurnPhase>>().get(),
            &TurnPhase::Boot,
            "跑了 {frames} 帧回合机还没停到 Boot，清场条件根本不成立"
        );
        assert!(
            !app.world().contains_resource::<TurnOrder>(),
            "回合机已停在 Boot，清场却还没动手"
        );
    }

    /// 回归：picking 每帧只 hover **最深的一个**节点，指针在按钮里从空白处挪到
    /// 文字上，同一帧会成对出现 `Out(按钮)` + `Over(文字)`。清算必须**先出后进** ——
    /// 若先 Over 后 Out，后到的 Out 会把刚点亮的高亮立刻浇灭：悬停高亮一闪即逝，
    /// 稳态是原色（用户实报，§6.1 坑 24）。光标恰恰最爱停在文字上，所以必现。
    #[test]
    fn out_and_over_of_the_same_button_in_one_frame_stays_hovered() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StatesPlugin)
            .init_state::<TurnPhase>()
            .insert_resource(Paused::default())
            .insert_resource(PendingDecision::default())
            .add_message::<PointerOver>()
            .add_message::<PointerOut>()
            .add_systems(Update, hover_feedback);

        let normal = Color::srgb(0.2, 0.2, 0.2);
        let hover = Color::srgb(0.9, 0.1, 0.1);
        let button = app
            .world_mut()
            .spawn((
                UiAction::Roll,
                BackgroundColor(normal),
                Palette { normal, hover },
            ))
            .id();
        // 命中测试报的是最深的节点：按钮里的文字
        let text = app.world_mut().spawn(ChildOf(button)).id();

        // 空白处 → 文字：同帧 Out(按钮) + Over(文字)，最终必须停在 hover 色
        app.world_mut()
            .resource_mut::<Messages<PointerOut>>()
            .write(make_out(button));
        app.world_mut()
            .resource_mut::<Messages<PointerOver>>()
            .write(make_over(text));
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(button).unwrap().0,
            hover,
            "同帧「离开旧 + 进入新」指向同一按钮，Out 不得覆盖 Over"
        );

        // 文字 → 空白处：对称方向，同样停在 hover 色
        app.world_mut()
            .resource_mut::<Messages<PointerOut>>()
            .write(make_out(text));
        app.world_mut()
            .resource_mut::<Messages<PointerOver>>()
            .write(make_over(button));
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(button).unwrap().0,
            hover,
            "反方向的按钮↔文字切换同样要保持 hover 色"
        );

        // 彻底离开：只有 Out，复原 normal
        app.world_mut()
            .resource_mut::<Messages<PointerOut>>()
            .write(make_out(text));
        app.update();
        assert_eq!(
            app.world().get::<BackgroundColor>(button).unwrap().0,
            normal,
            "光标离开后应复原"
        );
    }

    /// 按动作找出那颗按钮。测试里没有 picking 后端，只能自己找实体再手拼点击。
    fn find_action(app: &mut App, want: UiAction) -> Option<Entity> {
        let mut query = app.world_mut().query::<(Entity, &UiAction)>();
        query
            .iter(app.world())
            .find(|(_, action)| **action == want)
            .map(|(entity, _)| entity)
    }

    fn action_entity(app: &mut App, want: UiAction) -> Entity {
        find_action(app, want.clone()).unwrap_or_else(|| panic!("这一屏上没有 {want:?} 那颗按钮"))
    }

    fn click_real(app: &mut App, target: Entity) {
        app.world_mut()
            .resource_mut::<Messages<PointerClick>>()
            .write(make_click(target));
        app.update();
    }

    /// 派发写出去的成绩。用 `drain` 而不是 `iter_current_update_messages`：消息是在
    /// `app.update()` **里面**写的，等它返回时那一帧的缓冲已经换过去了，而这里要的正是
    /// 「刚才那一下点击报出了什么」。收走也无妨 —— 每颗按钮只点一次
    /// 跑过第一帧的接线。`request_initial_scene` 是 `Startup`，它会把场景钉到菜单 ——
    /// 所以任何「我要在棋盘上」的注入都得等那一帧跑完，否则它被覆盖掉，
    /// 而症状只是「这一屏没建出来」
    fn settled_app() -> App {
        let mut app = wired();
        app.update();
        app
    }

    fn scores_seen(app: &mut App) -> Vec<MinigameResult> {
        app.world_mut()
            .resource_mut::<Messages<MinigameResult>>()
            .drain()
            .collect()
    }

    /// 从窗口到成绩那一条完整的路（P7）：`Pending::Minigame` 建屏 → 「提前收工」→
    /// 「收下」报一份 `MinigameResult`，主语是**窗口挂着的那个人**，分数是这一局真数出来的。
    ///
    /// 走真实接线（`wired()`）而不是手搓 system：坑 23 那类「挂错门禁」只有在
    /// `sync_arcade` 与派发同一份调度里才暴露得出来。挖宝那一局用它是因为它**不看秒数** ——
    /// 限时两款要等 25 秒才收工，测试里等不起。
    #[test]
    fn the_collect_button_reports_the_round_it_was_built_from() {
        let mut app = settled_app();
        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Board);
        app.update();

        let me = app.world_mut().spawn(PlayerId(0)).id();
        app.world_mut()
            .resource_mut::<PendingDecision>()
            .offer(me, Pending::Minigame(GameKind::PenguinDig));
        app.update();

        let quit = action_entity(&mut app, UiAction::ArcadeQuit);
        click_real(&mut app, quit);
        app.update();
        let collect = action_entity(&mut app, UiAction::ArcadeCollect);
        click_real(&mut app, collect);

        let scores = scores_seen(&mut app);
        assert_eq!(scores.len(), 1, "「收下」没报成绩：{scores:?}");
        assert_eq!(scores[0].player, me, "报的人不是窗口挂着的那位");
        assert_eq!(scores[0].game, GameKind::PenguinDig);
        assert!(scores[0].awarded, "对局里那一手必须计入点券");
        assert_eq!(
            scores[0].score, 0,
            "一铲没下就报出了 {} 分 —— 分数不是从这一局数出来的",
            scores[0].score
        );
    }

    /// 练习局**根本没有「收下」那颗按钮** —— 于是「练习不给点券」不是靠某处返回 `None`
    /// 兜住的，而是这一屏上没有报成绩的入口。它换上来的是「再来一局」与「离开」两颗。
    ///
    /// 这条比「发了消息但没人读」强得多：菜单里没有 `ActiveBoard`，而点券的唯一入口
    /// `Books` 要它，硬发的那条消息会躺在队列里，等到下一局窗口第一次打开时被读走
    /// （跨局残留，§6.1 坑 22/25 的一个新变种）。
    #[test]
    fn a_practice_round_has_no_collect_button_at_all() {
        let mut app = settled_app();
        // 注入必须在菜单入场之后：`reset_ui_state` 挂在 `OnEnter(Scene::Menu)` 上，
        // 而开机那一次入场也走它（见它自己的文档注释）
        app.insert_resource(minigame::Practice(Some(GameKind::PopBalloons)));
        app.update();

        let quit = action_entity(&mut app, UiAction::ArcadeQuit);
        action_entity(&mut app, UiAction::ArcadeReplay);
        action_entity(&mut app, UiAction::ArcadeLeave);
        assert!(
            find_action(&mut app, UiAction::ArcadeCollect).is_none(),
            "练习局上摆着一颗「收下」"
        );

        // 收工之后也不长出来：结算屏那两颗是建屏那一刻就摆好的，不是收工时换的
        click_real(&mut app, quit);
        app.update();
        assert!(
            find_action(&mut app, UiAction::ArcadeCollect).is_none(),
            "练习局的结算屏上长出了一颗「收下」"
        );
        assert!(
            scores_seen(&mut app).is_empty(),
            "练习局报出了成绩 —— 它会在下一局的第一个窗口被读走"
        );
    }

    /// 回主菜单必须把游乐场那一屏一起收走（P7）。它比其余浮层更值得钉：这一屏在**两个
    /// 场景**都能出现，而练习那一半根本没有棋盘可依附 —— 留着不清，下一局第一帧就摆着
    /// 上一局那一手气球，不报错，只是分数是别人的（§6.1 坑 22 的同一家族）。
    #[test]
    fn going_back_to_the_menu_closes_the_arcade_too() {
        let mut app = settled_app();
        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Board);
        app.update();
        let me = app.world_mut().spawn(PlayerId(0)).id();
        app.world_mut()
            .resource_mut::<PendingDecision>()
            .offer(me, Pending::Minigame(GameKind::PopBalloons));
        app.update();
        assert!(
            app.world().resource::<minigame::Arcade>().0.is_some(),
            "前置：窗口挂着时这一屏该在"
        );

        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Menu);
        for _ in 0..3 {
            app.update();
        }

        assert!(
            app.world().resource::<minigame::Arcade>().0.is_none(),
            "回主菜单没清掉上一局那一屏"
        );
        let mut panels_query = app
            .world_mut()
            .query_filtered::<Entity, With<minigame::ArcadePanel>>();
        let panels = panels_query.iter(app.world()).count();
        assert_eq!(panels, 0, "上一局那块游乐场面板还挂在场景里");
    }

    /// 菜单那一块的三颗按钮只动 UI 自己的状态。「再来一局」加的是那个**当签名用**的计数
    /// （并进 `Practice` 里就做不到：连点两次会被判成同一局，而那正是玩家刚按下去的那颗）
    #[test]
    fn the_practice_buttons_move_ui_state_only() {
        let mut h = Harness::new(TurnPhase::Boot, false, PendingDecision::default());
        let replay = h.app.world_mut().spawn(UiAction::ArcadeReplay).id();
        let leave = h.app.world_mut().spawn(UiAction::ArcadeLeave).id();
        let choose = h.app.world_mut().spawn(UiAction::ToggleArcadeChoice).id();
        h.app.world_mut().resource_mut::<minigame::Practice>().0 = Some(GameKind::CatchTreasure);

        h.click(choose);
        assert!(h.app.world().resource::<minigame::ArcadeChoiceOpen>().0);
        h.click(replay);
        h.click(replay);
        assert_eq!(
            h.app.world().resource::<minigame::PracticeRounds>().0,
            2,
            "连点两次「再来一局」只算一手"
        );
        assert!(
            h.app.world().resource::<minigame::Practice>().0.is_some(),
            "「再来一局」不该把人踢出练习局"
        );
        h.click(leave);
        assert_eq!(h.app.world().resource::<minigame::Practice>().0, None);
        assert!(
            h.seen().rolls.is_empty() && h.seen().answers.is_empty(),
            "菜单上那几颗不该往规则层发任何东西"
        );
    }

    /// 暂停挡住玩法内的每一颗。遮罩本来压在 `ZIndex(6)` 上点不穿，这一道挡的是
    /// 「ESC 那一帧的点击已经排进队列」那种时序。
    ///
    /// 牙在后半段：解开暂停之后**同一颗按钮**就报得出成绩。只断言「暂停时没报」的话，
    /// 一条根本不会派发的接线也能过（空跑的断言）。
    #[test]
    fn pausing_blocks_the_arcade_and_unpausing_lets_it_through() {
        let mut app = settled_app();
        app.world_mut()
            .resource_mut::<NextState<crate::scene::Scene>>()
            .set(crate::scene::Scene::Board);
        app.update();
        let me = app.world_mut().spawn(PlayerId(0)).id();
        app.world_mut()
            .resource_mut::<PendingDecision>()
            .offer(me, Pending::Minigame(GameKind::PenguinDig));
        app.update();

        app.insert_resource(Paused(true));
        let dig = action_entity(&mut app, UiAction::Dig { cell: 0 });
        click_real(&mut app, dig);
        let quit = action_entity(&mut app, UiAction::ArcadeQuit);
        click_real(&mut app, quit);
        app.update();
        let collect = action_entity(&mut app, UiAction::ArcadeCollect);
        click_real(&mut app, collect);
        assert!(scores_seen(&mut app).is_empty(), "暂停时还能收工并报出成绩");

        app.insert_resource(Paused(false));
        // 解开暂停之后铲一个**已知埋着宝**的格子：种子是 `ArcadeDeal::default()` 那个 0，
        // 布局由 `dig_deal` 算得出来，而格号就是那张表的下标（`build_dig` 的 enumerate）
        let buried = minigame::dig_deal(0)
            .iter()
            .position(|treasure| *treasure)
            .expect("这一手得真埋着宝，否则下面那句断言是空跑") as u8;
        let dig = action_entity(&mut app, UiAction::Dig { cell: buried });
        click_real(&mut app, dig);
        click_real(&mut app, quit);
        app.update();
        click_real(&mut app, collect);
        let scores = scores_seen(&mut app);
        assert_eq!(
            scores.len(),
            1,
            "解开暂停之后同一颗按钮该报得出成绩：{scores:?}"
        );
        assert_eq!(scores[0].player, me);
        assert_eq!(
            scores[0].score, 1,
            "暂停那一下铲子生效了（0 分），或解开之后那一铲没算分"
        );
    }
}
