//! 规则层对外广播「发生了什么」，对应 Godot 的 `event_bus.gd`（24 个信号）。
//!
//! 与信号的实质差别：`EventBus.emit()` 是同步递归调用（结算里再触发别的事件会嵌套执行，
//! 这正是 `DEV_PLAN.md:458` 担心的隐患）；消息是**帧末批量投递** —— 本帧写的消息
//! 下一帧才读得到（缓冲在 `First` 交换），天然无重入，因果顺序由 schedule 顺序显式表达。
//!
//! 玩家一律用 `Entity` 指代而不是 `PlayerId`：接收方要拿它查组件，`Entity` 是可直接使用的句柄；
//! `PlayerId` 只在需要跨存档稳定标识时才用（见 §九）。

use bevy::prelude::*;

use crate::afflict::AfflictDomain;
use crate::board::{GridPos, TileIndex};
use crate::card::CardKind;
use crate::deck::CardId;
use crate::hand::CardAim;
use crate::land::LandId;
use crate::stock::StockId;

/// 某位玩家的回合开始了。阶段推进的唯一入口是 `OnEnter(TurnPhase::TurnStart)`，
/// 所以每回合恰好一条 —— Godot 端 `_advance_and_start` 与 `_start_turn` 各发一次（缺陷 #2）
/// 在这个结构下没有落点。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnStarted {
    pub player: Entity,
}

/// 请求掷骰。只有「当前回合玩家」的请求会被响应，UI 出错也不会替别人掷骰。
///
/// 发送者只会是表现层或 AI；规则层从不因为收到它就「推进」到什么 —— 见 §4.6 的红线。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRoll {
    pub player: Entity,
}

/// 骰子结果。`from`/`to` 一起带上，省掉监听方再去查一次移动前的位置。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DiceRolled {
    pub player: Entity,
    pub steps: u32,
    pub from: TileIndex,
    pub to: TileIndex,
}

/// 行走开始：`roll_dice_system` 定好路径后发，携带本次移动的完整意图。
/// 表现层由此开始逐格动画；规则层的逻辑位置此刻已指向 `to`
/// （§4.6 红线：core 拥有逻辑位置，渲染只做插值）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkStarted {
    pub player: Entity,
    pub from: TileIndex,
    pub to: TileIndex,
    pub steps: u32,
}

/// 棋子踏入一格（每步一条，`tile` 是踏入的格）。
///
/// 路径恒为主环下标：分支死胡同没有 `TileIndex`，结构上进不来
/// （ADR-001 行为规则 4，见 `BoardDef::walk_path`）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TileStepped {
    pub player: Entity,
    pub tile: TileIndex,
}

/// 走完骰子点数、落定在 `to`。落格结算（B3 起 `resolve_landing_system` 的分派）自此开始。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerArrived {
    pub player: Entity,
    pub from: TileIndex,
    pub to: TileIndex,
}

/// 某位玩家的回合结束。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TurnEnded {
    pub player: Entity,
}

/// 服刑期满自动出狱。逻辑位置一直是监狱对应的**接入点**（`player_data.gd:62-68` 的语义），
/// 所以这里带的是「从哪格继续沿环前进」，不是死胡同格。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerReleased {
    pub player: Entity,
    pub junction: TileIndex,
}

/// 仍在服刑，本回合跳过。`turns_left` 是已经扣掉本回合之后的剩余值。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct JailSkipped {
    pub player: Entity,
    pub turns_left: u32,
}

/// 对局结束。`winner` 为 `None` 表示无人存活。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOver {
    pub winner: Option<Entity>,
}

// ===== 移动（B3 补齐的两条：入狱与瞬移都是卡牌效果的后果）=====

/// 被卡牌瞬移到指定主环格。逻辑位置由 core 改写，`render` 只做位移动画。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerTeleported {
    pub player: Entity,
    pub tile: TileIndex,
}

/// 被关进拘留格（监狱/医院）。`junction` 是逻辑位置继续沿环前进的接入点，
/// `end` 是死胡同格的网格坐标 —— 只给表现层把棋子停过去用，`None` 表示这张棋盘
/// 没有死胡同格，棋子就停在接入点（`card_effects.gd:50-55` 的 `emit(-1,-1)` 语义）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerJailed {
    pub player: Entity,
    pub junction: TileIndex,
    pub end: Option<GridPos>,
}

// ===== 地产 =====

/// 无主地产的购买报价。是否买由 `DecisionChosen` 回答（Godot 的
/// `confirm_buy`/`cancel_buy` 两个方法在这里合成一条消息 + 一个 `Pending`）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyOffered {
    pub player: Entity,
    pub land: LandId,
}

/// 已成交：`land` 归 `player`，价格 `price` 已从现金扣除。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandBought {
    pub player: Entity,
    pub land: LandId,
    pub price: i64,
}

/// 自有垄断地的升级报价（`cost` 是这一级要付的钱）。
///
/// §4.4 的消息表里没有这条：Godot 用 `upgrade_decision_requested` 表达同一事实，
/// 移植时照搬语义比凑齐表格重要 —— UI 得知道「现在有一个可拒绝的升级」。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct UpgradeOffered {
    pub player: Entity,
    pub land: LandId,
    pub cost: i64,
}

/// 租金已结算：`payer` 付给 `owner`。付不起时不发这条，改发 `PlayerBankrupt`。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RentDue {
    pub payer: Entity,
    pub owner: Entity,
    pub land: LandId,
    pub amount: i64,
}

#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandUpgraded {
    pub player: Entity,
    pub land: LandId,
    pub new_level: u8,
}

// ===== 经济与银行 =====

/// 现金变动。所有 `Wallet` 的增减都必须留下一条这条流水 ——
/// B3 的现金审计断言（`balance == start_cash + Σ delta`）就是靠它兜住
/// 「改了钱包忘了发消息」这类错误。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct CashChanged {
    pub player: Entity,
    pub delta: i64,
    pub balance: i64,
}

/// 点券变动。**与 [`CashChanged`] 是两套账**：这条流水不计入现金审计，反之也不该出现
/// 在这里（PLAN §一 拍板「点券与现金永不通兑」）。
///
/// `delta` 带符号：捡点券箱是 `+30`，而 P5 的小偷偷一半、P12 的保释 300 都写负数，
/// 不必为「支出」再开一条消息。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsChanged {
    pub player: Entity,
    pub delta: i64,
    pub balance: u32,
}

/// 破产宣告。`unpaid` 是付不出来的那笔钱（租金/罚款/地价）。
///
/// 缺陷 #3 的修复点：Godot `card_effects.gd:34-41` 只回一句「现金不足，破产！」
/// 的文案却不改状态；这里 `CouldNotPay` 与 `PlayerBankrupt` 成对出现，状态与说法必然一致。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerBankrupt {
    pub player: Entity,
    pub unpaid: i64,
}

/// 踩到银行格：全员利息已结算，带上该玩家结算后的余额供 HUD 显示。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankVisited {
    pub player: Entity,
    pub deposit: i64,
    pub loan: i64,
}

/// 银行柜台的四种操作。**存/取与借/还的解锁条件不同**（见 `bank::bank_op_allowed`）：
/// 路过银行格就能存取（ATM），但借钱必须走进柜台（踩到那一格并停住）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BankOp {
    Deposit,
    Withdraw,
    Borrow,
    Repay,
}

/// 请求办一笔银行操作。主语必须是当前回合玩家，且在 [`crate::bank::AtmAccess`]
/// 或柜台的许可范围内 —— 判据在 `bank::bank_op_system`，不在按钮的 `disabled` 上。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankOpRequested {
    pub player: Entity,
    pub op: BankOp,
    pub amount: i64,
}

/// 已办成的银行操作。`amount` 是**实际**处理额（`economy` 的 plan 函数会拒单或打折）。
/// 被拒的操作不发这条 —— UI 若发现按下去没流水，就该把按钮亮成灰的。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BankOpDone {
    pub player: Entity,
    pub op: BankOp,
    pub amount: i64,
}

/// 柜台上的两件**地产变现**业务（P3d③）。与 [`BankOp`] 分两个枚举而不是并成六个变体：
/// 那四件的主语是「一个人 + 一个金额」，这两件的主语是「一个人 + **一块地**」，
/// 硬塞进同一张表就要给 `Deposit` 配一个永远用不上的 `land` 字段。
/// 两者共用同一个窗口（`Pending::BankTeller`）与同一条休市判据。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum MortgageOp {
    /// 押出去：到手 `账面 × mortgage_percent / 100`
    Take,
    /// 赎回来：付掉本金那一笔 + 手续费
    Redeem,
}

/// 请求办一件抵押/赎回。主语必须是**站在柜台前的那个人**，且这块地得是他的 ——
/// 判据在 `mortgage::mortgage_op_allowed`，不在按钮的 `disabled` 上。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MortgageRequested {
    pub player: Entity,
    pub op: MortgageOp,
    pub land: LandId,
}

/// 已办成的那一笔。`amount` 是**实际**过手的钱（到手额或赎回费），被拒的不发这条。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MortgageDone {
    pub player: Entity,
    pub op: MortgageOp,
    pub land: LandId,
    pub amount: i64,
}

// ===== 股票 =====

/// 股价真的动了：`old → new`。
///
/// 缺陷 #4 的修复点：Godot `stock_market.gd:118,148` 在买卖时发
/// `stock_price_changed(price, current_price)`，两个值相同（价格根本没变），
/// 真正的波动事件反而缺失。这里拆开：波动发 `StockPriceChanged`，成交发 `StockTraded`。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct StockPriceChanged {
    pub stock: StockId,
    pub old: f32,
    pub new: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TradeSide {
    Buy,
    Sell,
}

/// 成交凭据：`shares` 股按 `price` 成交。实际成交数可能少于请求数（现金不足时部分成交）。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct StockTraded {
    pub player: Entity,
    pub stock: StockId,
    pub shares: u32,
    pub price: f32,
    pub side: TradeSide,
}

/// 请求买卖 `shares` 股。只有当前回合玩家、且状态是 `WaitRoll`（掷骰之前）才会被受理 ——
/// 交易是「本回合打算怎么花钱」的一部分，走完路再决定就晚了（见 `market::trade_request_system`）。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct StockTradeRequested {
    pub player: Entity,
    pub stock: StockId,
    pub side: TradeSide,
    pub shares: u32,
}

/// 踩到证券交易格。行情本身已经改到「每个回合结束 tick 一次」，所以踩格只报到、
/// 不推行情 —— 保留这条消息是因为 UI 的提示行要说「某人来到交易所」。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StockVisited {
    pub player: Entity,
}

// ===== 卡牌 =====

/// 抽到一张牌并已结算完效果。文案由 UI 从 `outcomes` 生成，结算层不拼字符串。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CardDrawn {
    pub player: Entity,
    pub card: CardNotice,
}

/// 请求打出手里的一张卡。`aim` 只在 `CardDef.target` 要求时才有值 ——
/// 目标编码在按钮里（同 ATM 那颗「存 1,000」），所以规则层不必再挂一个「选目标」的待决事项。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayCardRequested {
    pub player: Entity,
    pub card: CardId,
    pub aim: Option<CardAim>,
}

/// 一张卡已经打出去。`player` 是出牌的人，`victim` 是效果落在谁头上 ——
/// 两者分开是这张流水的全部价值：「A 用陷害卡打了 B」与「A 把自己关进去了」
/// 在 UI 与回放里必须是两件事。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CardPlayed {
    pub player: Entity,
    pub victim: Entity,
    pub card: CardNotice,
}

/// 一张卡进了手牌。`pushed_out` 是被上限挤掉的那张（PLAN §三「满了替换最前面那张」）：
/// 玩家买了卡却没拿到，唯一能自证的形状就是让他看见**哪张被挤掉了**。
///
/// 第一个生产者就是百货公司（P3d②）：`shop::shop_op_system` 买入那一笔走的是
/// [`crate::hand::deal_to`]，上限与挤档规则本来就在它里面。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CardGained {
    pub player: Entity,
    pub card: CardNotice,
    pub pushed_out: Option<CardNotice>,
}

/// 请求在百货公司买或卖一张道具卡。`side` 借用股票那对 [`TradeSide`]：买卖方向是同一个概念，
/// 再开一个 enum 就是第二份真相。
///
/// 与 [`BankOpRequested`] 同一条形状 —— 一笔交易一个请求，主语由 `PendingDecision` 定，
/// 按钮不携带玩家句柄。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShopOpRequested {
    pub player: Entity,
    pub side: TradeSide,
    pub card: CardId,
}

/// 已经办成的百货交易。`points` 是**实际**动过的点券：买入＝卡面价，卖出＝回收价（9 折）。
/// 被拒的交易不发这条（同 [`BankOpDone`]）—— 面板该把办不动的那颗按钮亮成灰的。
///
/// 买入时这里还**没有**钱动过的现金流水：点券与现金两套账（PLAN §一），这条只走
/// [`PointsChanged`]。卡进手牌那半由 [`CardGained`] 说，两件一起才是一次完整的成交。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct ShopOpDone {
    pub player: Entity,
    pub side: TradeSide,
    pub card: CardNotice,
    pub points: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CardNotice {
    pub id: String,
    pub kind: CardKind,
    pub title: String,
    pub desc: String,
    pub outcomes: Vec<EffectOutcome>,
}

/// 单条卡牌效果的结算凭据（与 `CardEffect` 的分支一一对应，另加抵消/护身两条）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EffectOutcome {
    Gained(i64),
    Lost(i64),
    /// 该扣而扣不起 → 同帧必然还有一条 `PlayerBankrupt`
    CouldNotPay {
        needed: i64,
    },
    Jailed {
        turns: u32,
        junction: TileIndex,
    },
    Teleported {
        tile: TileIndex,
        name: String,
    },
    /// 数据写了个棋盘上不存在的格名。不猜、不回退到起点（`card_effects.gd:76-81`
    /// 已经修过这个坑），UI 该把它当成本局数据错误显示出来。
    UnknownTile {
        name: String,
    },
    /// 一件厄运被护身吃掉了（免罪卡生效）。与 `Jailed` 互斥 —— 抵消了就没有入狱这回事，
    /// 而流水里必须留下「被抵消」这一条，否则玩家只看见「什么都没发生」。
    Blocked {
        domain: AfflictDomain,
    },
    /// 挂上了护身。`charges` 是**叠完之后剩余**的层数（两张免罪卡 = 2）。
    Shielded {
        charges: u32,
    },
    /// 给一片地块盖上了限时状态（涨价 / 查封，P3c）。`lands` 是被盖到的块数 ——
    /// 要说清块数，因为「整街」在缺路段键的盘上可能只有一块，玩家有权知道自己
    /// 到底封了什么。
    Stamped {
        status: crate::status::LandStatus,
        lands: u32,
        days: u32,
    },
    /// 挂上了一条待用的免付（免费卡）。`floor` 是门槛的名义额 —— 这条凭据说的是
    /// 「从今天起，超过这个数的账可以免一次」，而**不是**「已经免了」。真免掉那笔是
    /// 另一条消息 [`ChargeWaived`]，两件事之间可以隔几十个回合。
    FreePassArmed {
        floor: i64,
    },
    /// 买下了这块地（购地卡，P3d④）。`price` 是**实付**额（过了物价与修正），与
    /// [`LandBought`] 上那个数同源 —— 那条说的是「归属变了」，这条说的是「这张卡办成的」。
    Purchased {
        land: crate::land::LandId,
        price: i64,
    },
}

/// 一条一次性修正**真的生效了**：`saved` 这笔账从这位身上免掉了，那张卡就此消失。
///
/// 与 [`CashChanged`] 分开，是因为这里**没有钱动** —— 「少付」不是一笔收入，把它记成
/// `+2400` 会让现金审计对不上账（P1 那两套账同一条理由）。而没有这条消息，玩家看见的
/// 就是「租金那一格显示 0」，读不出自己刚花掉一张卡。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChargeWaived {
    pub player: Entity,
    pub saved: i64,
    pub via: crate::charge::Source,
}

// ===== 决策 =====

/// 玩家对当前报价的回答。
///
/// Godot 端是六个方法（`confirm_buy`/`cancel_buy`/`confirm_upgrade`/`cancel_upgrade`/
/// `dismiss_rent_notice`/`dismiss_card`），每个方法开头都要重复一句
/// `if _phase != WAIT_DECISION or _pending_decision != "...": return`。
/// 这里语义在 `Pending`（系统状态）里，消息只带意图，六个方法塌成三个动作。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecisionChosen {
    pub player: Entity,
    pub action: Decision,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// 接受当前报价（买地 / 升级）
    Accept,
    /// 放弃当前报价（不买 / 不升级）
    Decline,
    /// 关闭只需阅读的通知（租金单 / 卡面）
    Acknowledge,
}
