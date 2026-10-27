//! 规则层对外广播「发生了什么」，对应 Godot 的 `event_bus.gd`（24 个信号）。
//!
//! 与信号的实质差别：`EventBus.emit()` 是同步递归调用（结算里再触发别的事件会嵌套执行，
//! 这正是 `DEV_PLAN.md:458` 担心的隐患）；消息是**帧末批量投递** —— 本帧写的消息
//! 下一帧才读得到（缓冲在 `First` 交换），天然无重入，因果顺序由 schedule 顺序显式表达。
//!
//! 玩家一律用 `Entity` 指代而不是 `PlayerId`：接收方要拿它查组件，`Entity` 是可直接使用的句柄；
//! `PlayerId` 只在需要跨存档稳定标识时才用（见 §九）。

use bevy::prelude::*;

use crate::board::{GridPos, TileIndex};
use crate::card::CardKind;
use crate::deck::CardId;
use crate::hand::CardAim;
use crate::land::LandId;
use crate::passive::PassiveEvent;
use crate::player::DetentionPlace;
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
/// `dice` 是**这一掷用几颗骰子**（P5 的载具：步行 1／机车 2／汽车 3）。`0` = 用载具的
/// 上限（foot 就是 1），于是既有调用方与 AI 不必关心载具；真人可以按 1..=上限 先定颗数。
/// 与「遥控骰子」是两条消息：那条决定的是**精确步数**，不是摇几颗。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRoll {
    pub player: Entity,
    pub dice: u32,
}

/// 请求「停留」：把这一回合用在原地不动上（停留卡，P4④）。
///
/// 与 [`RequestRoll`] 是同一层的两个意图，不是 UI 独有的一颗按钮 —— 被停留按住的
/// AI 与 P12 的恶人也要发这一条，否则那个 `StaySentence` 没人消耗，回合会死锁。
/// 只有当前回合玩家、且身上确实挂着停留计时时才被受理；受理之后这一回合结束。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestStay {
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

/// 拘留期满，回到牌桌上。`place` 说的是他从哪儿回来（出狱/出院/回国是三句话、一条凭据），
/// `junction` 是逻辑位置继续沿环前进的接入点 —— 逻辑位置一直是接入点而不是死胡同格
/// （`player_data.gd:62-68` 的语义）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetentionLifted {
    pub player: Entity,
    pub place: DetentionPlace,
    pub junction: TileIndex,
}

/// 仍在被拘留中，本回合跳过。`left` 是**已经扣掉本回合之后**的剩余回合，
/// `None` = 他身上那条不是钟而是笼子（[`crate::player::Term::UntilBailed`]，P12 的恶人：
/// 只有保释办得出来，所以「还剩几天」这一问在这里没有答案）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DetentionSkipped {
    pub player: Entity,
    pub place: DetentionPlace,
    pub left: Option<u32>,
}

/// 仍在原地停留（停留卡，P3d④c / P4④），本回合被玩家主动拿来「停」。`turns_left` 是
/// **已经扣掉本回合之后**的剩余值。
///
/// P4④ 起停留不再自动跳回合：身上有计时的人进入 `WaitRoll` 时，掷骰按钮翻成「停留」，
/// 点一下（或 AI 发 [`RequestStay`]）才结束这一回合。所以这条消息说的是「他刚按了」，
/// 不是「回合机替他跳了」—— 与 [`DetentionSkipped`] 的区别正在这里。
///
/// 为什么单开一条而不是让 `DetentionSkipped` 带个 `reason`：被拘留的人被送进死胡同格、
/// （P4② 起）丧失收租权，停留的人**还在场上**、地还收得到租。屏幕上那两句话不是一句话，
/// 而合并之后要靠调用方再查一次「他是哪种」才说得出口 —— 那正是这条消息该带的答案。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaySkipped {
    pub player: Entity,
    pub turns_left: u32,
}

/// 停留期满，下一回合起可正常行动。**不带格号**：他从没离开过那一格
/// （与 [`DetentionLifted`] 的差别正在这里 —— 回来说的是「从哪格继续沿环前进」）。
///
/// P4④ 起它由「最后那一格被按掉」触发：`turns_left == 1` 的人按下「停留」时，
/// 组件摘掉、这条发出来，所以它说的是「用完了」，不是「自动期满」。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StayLifted {
    pub player: Entity,
}

/// 对局结束。`winner` 为 `None` 表示无人存活。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameOver {
    pub winner: Option<Entity>,
}

// ===== 移动（B3 补齐的两条：入狱与瞬移都是卡牌效果的后果）=====

/// 请求保释：花点券把关在眼前这处的人放出来（P4③）。
///
/// **不是**自救：被困的人棋子停在死胡同尽头格，不会沿环经过接入点，结构上就发不出这条。
/// 主语必须是此刻「站在那条分支接入点上、被挂了保释窗口」的人（判据在
/// [`crate::bail::bail_op_allowed`]，不在按钮的 `disabled` 上）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BailRequested {
    pub player: Entity,
    pub target: Entity,
}

/// 已经放出来了：`by` 花 `points` 点券把 `player` 从 `place` 保释出来，
/// `junction` 是他接下来继续沿环前进的接入点。
///
/// 与 [`DetentionLifted`] 分开：那条说的是「期满」，这条说的是「有人替他付了账」。
/// 渲染层两条都要听（棋子从死胡同格挪回接入点），顶栏写的却是两句话。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerBailed {
    pub player: Entity,
    pub by: Entity,
    pub place: DetentionPlace,
    pub points: u32,
    pub junction: TileIndex,
}

/// 被卡牌瞬移到指定主环格。逻辑位置由 core 改写，`render` 只做位移动画。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerTeleported {
    pub player: Entity,
    pub tile: TileIndex,
}

/// 被送进拘留类状态（监狱/医院/国外）。`junction` 是逻辑位置继续沿环前进的接入点，
/// `end` 是死胡同格的网格坐标 —— 只给表现层把棋子停过去用，`None` 表示这张棋盘
/// 没有那条分支（或那个地方压根不在地图上，如 [`DetentionPlace::Away`]），
/// 棋子就停在接入点（`card_effects.gd:50-55` 的 `emit(-1,-1)` 语义）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlayerDetained {
    pub player: Entity,
    pub place: DetentionPlace,
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

/// 土地公直接占有（P6c）：`land` 归 `player`，`from` 是他从谁手里拿走的
/// （`None` = 那块地本来无主）。
///
/// 与 [`LandBought`] 分两条是刻意的：那条说「有人付了这笔钱」，而这一条**没有付款**——
/// 过路费是另一笔账（由 `settle_property_landing` 照原样收，见 PLAN §三「过路费照付原主」），
/// 合成一条就会让人以为「神帮他免了那一笔」，而事实正好相反：他付了钱，只是地也归了他。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LandTaken {
    pub player: Entity,
    pub land: LandId,
    pub from: Option<Entity>,
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

/// 建设报价（P10c）：`player` 站在自己的空商业地 `land` 上，造价一律 `cost`
/// （五类设施同价，取的是 `upgrade_cost`）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildOffered {
    pub player: Entity,
    pub land: LandId,
    pub cost: i64,
}

/// 建造请求（P10c）：在 `land` 上立起 `kind` 那类设施。窗口与柜台同一形
/// （`Pending::Build` 开着才被受理），请求逐笔来、办完窗口照开。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuildRequested {
    pub player: Entity,
    pub land: LandId,
    pub kind: crate::board::BuildingKind,
}

/// 已建成（P10c）：`land` 上立起 `kind`，造价 `cost` 已从现金扣除
/// （`ChargeKind::Build`，过修正管线 —— 大福神那类 Acquire 豁免吃得到它）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FacilityBuilt {
    pub player: Entity,
    pub land: LandId,
    pub kind: crate::board::BuildingKind,
    pub cost: i64,
}

/// 服务费已结算（P10c）：`payer` 踩到 `owner` 的设施 `land` 上被收了 `amount`。
/// `roll` 是 A/B 型摇出的数（摇出几走第几档/住几天；C 型为 0）；
/// `stay_days` 非 0 时 `payer` 已被送去 [`crate::player::DetentionPlace::Hotel`] 住宿。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceCharged {
    pub payer: Entity,
    pub owner: Entity,
    pub land: LandId,
    pub kind: crate::board::BuildingKind,
    pub amount: i64,
    pub roll: u32,
    pub stay_days: u32,
}

/// 公司服务费已结算（P10d①）：`payer` 踩到那家公司的门面格被收了 `amount`。
/// 与 [`ServiceCharged`] 分开不是形状不同，是**收款方**不同：这笔钱不进任何玩家的钱包，
/// 而是记进那家公司的**当月损益账**（`company::MonthLedger`），分红日按持股比例分它。
/// `chairman` 是此刻持股最多的人 —— 他踩格免费（`amount == 0` 就是免单的那一笔）；
/// `stock` 供面板与顶栏读「是哪家公司收的」。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct CompanyCharged {
    pub payer: Entity,
    pub stock: String,
    pub chairman: Option<Entity>,
    pub amount: i64,
    pub roll: u32,
    pub stay_days: u32,
}

/// 公司分红已发放（P10d①）：分红日（15 号）按持股比例把 `MonthLedger` 的当月净额
/// 分给持股人。`per_share` 可为**负数** —— 亏损月份按比例扣现金（10-20 实报的口径），
/// 负得付不出的人当场破产（与租金同一条判据）。`profit` 是当月净额（含符号），
/// 顶栏与 dump 读它说清「这笔钱从哪来」。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct CompanyPayout {
    pub stock: String,
    pub player: Entity,
    pub shares: u32,
    pub per_share: i64,
    pub profit: i64,
}

/// 投保完成（P10d②）：`player` 在保险格付了 `premium`、拿到 `days` 天的保单。
/// 保费进公司账（与 [`CompanyCharged`] 同一条纪律：钱不进谁的口袋），这里只报
/// 「保单生效了」—— 效期剩几天是 [`game_core::player::Insured`] 组件的事，
/// HUD 与 dump 读组件不读流水（与「[停留]」徽章同一理由：流水只活两三帧）。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct PolicyBought {
    pub player: Entity,
    pub stock: String,
    pub days: u32,
    pub premium: i64,
}

/// 保险赔付（P10d②）：`player` 投保期间被拘留了 `days` 天，保险公司赔 `amount`。
/// 赔付是**公司记账负数** + 玩家记 `CashChanged` 正数两件事 —— 现金审计盯后者，
/// 分红的「亏损月是负数」盯前者，一条消息带不全就变成两处各记各的。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct IndemnityPaid {
    pub player: Entity,
    pub stock: String,
    pub days: u32,
    pub amount: i64,
}

/// 董事长在百货白拿了一件（P10d② 特权）：`player` 是那家百货绑定股票的董事长，
/// 拿走 `item` 没花点券。与投保收据同理 —— 「免费」要有一条说得清为什么的流水，
/// 静默少扣 30 券会让「点券审计对不上」变成一桩悬案。
#[derive(Message, Debug, Clone, PartialEq, Eq)]
pub struct ChairmanTook {
    pub player: Entity,
    pub stock: String,
    pub item: String,
}

/// 远程建设窗口已开（P10d②，日产建设）：`player` 踩到建设格，可以选一块
/// **自己的**地升级。真正的升级走 [`game_core::msg::LandUpgraded`]（`Books::build`
/// 正路），这条只是「窗口开了」的报到 —— 面板与 autopilot 读它弹 UI。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteBuildOffered {
    pub player: Entity,
}

/// 一笔远程升级请求（P10d②，日产建设）：窗口开着时逐笔进来办（与
/// [`BuildRequested`] 同形），升级正路在 `Books::upgrade` —— 判据、造价、垄断
/// 与「组内最低层」全在那里，这条消息只是把「哪块地」递进柜台。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteUpgradeRequested {
    pub player: Entity,
    pub land: crate::land::LandId,
}

/// 融资动作完成（P10d②，银行保险董事长特权）：`chairman` 挪用了 `amount` 的
/// 他人存款进自己现金（记 [`CashChanged`] 正数），`financing` 债务同步 +`amount`
/// 不计息。兜底那一半也发这条 —— 「钱从现金口袋进了银行储备、换成融资债」
/// 是同一类账目事实。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinancingDrawn {
    pub chairman: Entity,
    pub amount: i64,
}

/// 融资被收回（P10d② 融资三件套 ③）：`player` 的银行保险董事长身份没了
/// （持股被反超），背着的 `amount` 融资债当场用他的现金清算。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinancingRecalled {
    pub player: Entity,
    pub amount: i64,
}

/// 一笔融资请求（P10d②）：`player`（银行保险的董事长）要从 `of_player` 的存款
/// 里挪 `amount`。资格判据在 `bank::financing_system`（每帧重算董事长，反超的
/// 那一帧起请求就被拒）；余额不足时按实挪数成交（与证券的部分成交同一形）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct FinancingDrawRequested {
    pub player: Entity,
    pub of_player: Entity,
    pub amount: i64,
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
    Detained {
        place: DetentionPlace,
        turns: u32,
        junction: TileIndex,
    },
    /// 从某一处被放出来（P11b② 的新闻「住院者提前出院」）。与 [`Self::Detained`] 正好是一对：
    /// 那条说「他进去了、接入点在这格」，这条说「他出来了、从这一格继续走」。
    ///
    /// 它不复用 `Detained` 的某个 bool 位，也不与期满共用一条凭据 —— 期满是「刑满」，
    /// 这条是「有人替他开了门」，两句话在顶栏上是两句不同的话（而世界效果那半确实是同一条：
    /// 摘 `Detention` + 同一条 [`DetentionLifted`]，见 `deck::apply_effects` 那一支）。
    Released {
        place: DetentionPlace,
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
    /// 一件厄运被挡下来了。与 `Detained` 互斥 —— 抵消了就没有入狱这回事，而流水里必须留下
    /// 「被抵消」这一条，否则玩家只看见「什么都没发生」。
    ///
    /// 带的是**表上的那一行**而不是旧那个「族」：`Hold`（停留／乌龟／掉头）与 `JailTrip`
    /// 在屏幕上是两句话，而 P13b① 之后「能不能免」这件事只在行上有答案
    /// （见 [`crate::passive`]）。
    ///
    /// `by` 是 P13b② 补的那半句：① 之前这条说不出**是谁**挡的，于是面板那句
    /// 「一次出国旅游被抵消」在神明挡下时是句假话（免罪卡一张没花）。门的回答与神的回答
    /// 在玩家眼里是两件事 —— 前者刚少了手里一张卡。
    Blocked {
        event: PassiveEvent,
        by: Blocker,
    },
    /// 给一片地块盖上了限时状态（涨价 / 查封，P3c）。`lands` 是被盖到的块数 ——
    /// 要说清块数，因为「整街」在缺路段键的盘上可能只有一块，玩家有权知道自己
    /// 到底封了什么。
    Stamped {
        status: crate::status::LandStatus,
        lands: u32,
        days: u32,
    },
    /// 一片地上的建筑挨了一拳（P11b①）。`lands` 是被扫到的块数（含本来就是空地的那些 ——
    /// 范围是范围，不为「没打到东西」缩小，与 [`crate::damage::DamageTally::hit`] 同一个口径），
    /// `seized` 是其中所有权被收走的块数。
    ///
    /// 两个数都要有：「拆了房没夺地」与「连地一起收」是阶梯上两个不同的格子，只看块数
    /// 分不出来。而**具体是哪几块**在这条上放不下也不该放 —— 地块类新闻的凭据
    /// [`NewsAudience::Ground`] 带那份清单，因为「外星人拆了哪几块」是玩家要念出来的那句话。
    Razed {
        lands: u32,
        seized: u32,
    },
    /// 买下了这块地（购地卡，P3d④）。`price` 是**实付**额（过了物价与修正），与
    /// [`LandBought`] 上那个数同源 —— 那条说的是「归属变了」，这条说的是「这张卡办成的」。
    Purchased {
        land: crate::land::LandId,
        price: i64,
    },
    /// 这位的绕行方向被翻到了**这个值**（转向卡，P3d④b）。带的是翻完之后那一位，
    /// 不是「翻了一次」这个动作 —— 面板要说的是「他现在朝哪边走」，那才是玩家下一手要用的数。
    Reversed {
        heading: crate::player::Heading,
    },
    /// 挂上「原地停留 N 回合」（停留卡）。与 [`Self::Detained`] 同一个形状：
    /// 说的是**接下来**他要自己停几个回合，而不是「已经跳过了」—— 第一次停发生在他
    /// 下一次进入 `WaitRoll` 并按下「停留」的时候，由 [`StaySkipped`] 播报，
    /// 两条之间可以隔着别人的回合。
    /// `turns` 是**叠完之后**的剩余回合数：身上已有 2 的人再吃一张 1，这里报的是 2，不是 1
    /// （`max` 那条口径与 [`Self::Detained`] 的 `turns` 同一条理由）。
    Stayed {
        turns: u32,
    },
    /// 挂上「接下来 N 个掷骰回合只走 1 格」（乌龟卡，P5）。`turns` 是**叠完之后**的
    /// 剩余回合数（同 [`Self::Stayed`] 的 `turns`）：短的不覆盖长的。
    Slowed {
        turns: u32,
    },
    /// 请神符请来了哪一位（P6d）。`from` 是他原来站着的格、`steps` 是离出牌人几步 ——
    /// 这两个数就是「最近」那两个字的内容，玩家有权核对它是不是真的最近（而不是随机的一位）。
    /// 神上身本身另有 [`GodPossessed`]，这条说的是「这张卡办成的那一件事」。
    Summoned {
        god: crate::gods::God,
        from: TileIndex,
        steps: u32,
    },
    /// 挂上了拍卖台（拍卖卡 P9 / 新闻「公开拍卖公有土地」P11c①）。`lands` 是**这一槌真的挂上去的
    /// 那几块** —— 卡那一槌永远是一块，而新闻那一条是范围内所有无主地（有主的不在其中：
    /// 这条新闻不替业主卖地，所以「范围内有几块」与「挂了几块」是两个数，念出来的是后者）。
    ///
    /// 它说的只是「槌敲下去了」—— 起拍多少、谁拍走、多少钱，都在 [`AuctionStarted`] 与
    /// [`AuctionWon`] 那两条上，因为那是**世界**的读数而不是这一槌办成的那一件事。
    ///
    /// 为什么这条凭据非有不可：敲下去而拍卖没开场（场上没有别人可出价）与开场了又流标，
    /// 在画面上是三件不同的事，玩家有权知道自己是哪一件。
    Auctioned {
        lands: Vec<crate::land::LandId>,
    },
    /// 送神符送走了哪一位（P6d）。`bomb` 说那一趟**有没有连带带走**身上的定时炸弹 ——
    /// 那是这张卡的第二件事，玩家有权知道自己是不是顺手把要命的东西也送了。
    /// 离场本身另有 [`GodLeft`]（reason = `Banished`），这条与它是一对，不重复它。
    Banished {
        god: crate::gods::God,
        bomb: bool,
    },
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

// ===== 被动卡这道门的回答（P13b②）=====

/// 那一件事被挡下来了。**谁**挡的两臂不合成一个 bool：
/// 卡挡的那一次玩家手里少了一张卡，而神挡的那一次什么都没少 ——
/// 这两句话在顶栏上是两句，而「少了一张卡」这件事必须有处可查（P3d④a 那条纪律：
/// 「卡没了事没成」是缺陷，「事没成而卡还在」才是正确的那一侧）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Blocker {
    /// 手里那张被动卡花掉了（免罪卡；账那侧是免费卡，归 ③）
    Card,
    /// 身上那位神豁免了这一族厄运，不消耗任何东西
    God(crate::gods::God),
}

/// 被动卡那道门的一句话回答（P13b②）。
///
/// 它**不是** [`Decision`] 的第四臂：那三臂说的是「一个报价的三种态度」，而这里要带
/// 「转给谁」那个 `Entity`，`Decision` 是 `Copy` 而没有 `Entity` 的位置。另开一条消息
/// 还与 P9 的拍卖同形（`AuctionMove` 也不走 `DecisionChosen`，理由写在 `Pending::Auction`
/// 那一格）：门在等的那一句不是「要不要买」，而是「用哪张、给谁」。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PassiveAnswer {
    pub player: Entity,
    pub what: PassiveMove,
}

/// 回答的三种动作。**没有「用哪一张」这一格**：同类多张卡在门上是同一问
/// （PLAN §五 第 7 条），由哪一位答、门就花他手里的那一张。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PassiveMove {
    /// 用掉那张抵消卡（免罪／免费），这件事不落下来
    Use,
    /// 不用：什么都不消耗，事件照原样往下走
    Decline,
    /// 嫁祸：把这件事转给点名的那一位（`to` 由门校验，不在面板上猜）
    Shift { to: Entity },
}

/// 一条**账**被被动卡免掉时没有钱动，所以这里没有对应的 [`CashChanged`] ——
/// 「少付」不是一笔收入，把它记成 `+2400` 会让现金审计对不上账（P1 那两套账同一条理由）。
/// 那句「玩家刚花掉一张卡」由 [`PassiveUsed`] 一条话说完，它带的 `saved` 就是
/// P3c② 那条 `ChargeWaived` 的归宿（③ 把免付从管线里搬进那道门，两条凭据就此合成一条）。
///
/// 与 [`EffectOutcome::Blocked`] 的区别：那一条是「这一次调用里的那条效果」的凭据，
/// 进的是卡面/法事/新闻那份清单；这一条说的是**卡**没了，顶栏与 dump 要单独念它。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct PassiveUsed {
    pub player: Entity,
    pub card: CardNotice,
    pub event: PassiveEvent,
    /// 嫁祸那一句带来的下一位（抵消那一句是 `None`）。顶栏那句「嫁祸给谁」读的就是它
    pub moved_to: Option<Entity>,
    /// 这一次花卡**省（或转出去）多少钱**。事那侧永远是 0 —— 坐牢没有价签。
    ///
    /// 它与 [`CashChanged`] 无关：这里没有钱动（免付）或动的不是他的钱（嫁祸）。
    /// 带上它是为了「玩家刚花掉一张卡」与「这张卡值多少」是同一句话，而不是两条消息
    /// 各自的一半（`ChargeWaived` 当年那样）。
    pub saved: i64,
}

/// 一件事被从甲转到乙（P13b② 嫁祸卡）。
///
/// 它与 [`PassiveUsed`] 同时发（一条说卡没了、一条说事换人了），因为两句话的主语不同：
/// 一句是甲的钱包少了张卡，一句是乙接下来要挨这一下。合成一条的话，「用了嫁祸卡」
/// 与「嫁祸给了谁」在流水里就成了同一个事件，而后者才是别人那一条记录里的事。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlameShifted {
    pub from: Entity,
    pub to: Entity,
    pub event: PassiveEvent,
}

// ===== 日历事件（P2b）=====

/// 圣诞礼物：12 月 25 日给每个在场玩家一张道具卡。
///
/// 一条消息带全桌，而不是每人一条：表现层的动画是**一段**（一屏列出谁收到什么），
/// 逐人一条会让 UI 去猜「这一批到底有几个人」。卡本身另走既有的 [`CardGained`]
/// （手牌满时 `pushed_out` 那半条因此不用为圣诞再写一遍）。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct GiftsGiven {
    pub year: i32,
    pub gifts: Vec<GiftGiven>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GiftGiven {
    pub player: Entity,
    pub card: CardNotice,
}

/// 春节红包：正月初一那天每个在场玩家抽一个四位数（0000–9999），
/// 效果等于大财神那笔现金、不附免费特权。
///
/// **0 元也发这一条**：老虎机照播、账上分文不动 —— 空红包是规则的一部分，
/// 把它吞掉会让「为什么我没收到红包」变成一句查不动的话。现金那半走 [`CashChanged`]，
/// 且只在 `amount > 0` 时写（0 元不该在现金流水里留一条假账）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedEnvelope {
    pub player: Entity,
    pub amount: i64,
}

// ===== 新闻（P11）=====

/// 一条新闻已经播完并结算完。
///
/// `broadcaster` 是踩到新闻中心的那个人，而**他通常不是挨打的那个** —— 这两个主语分开写
/// 是这条玩法的全部新意（PLAN §三「踩格者只负责开广播，挨打的是别人」）。合成一个的话，
/// 「他开了广播」与「他挨了打」在流水里就是同一句话，而验收①（触发者金额不变、
/// 被点名者金额变）压根问不出来。
///
/// 它**不带 `Pending`**：新闻没有人要回答「要不要听」，钱与地在这一帧已经动完。
/// 钱动过的每一笔另有 [`CashChanged`]（新闻不另开一套账），这条只说「世界变了什么」。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct NewsFired {
    pub broadcaster: Entity,
    pub news: NewsNotice,
}

/// 一条新闻的读数。字段与 [`CardNotice`] 同构（`id`/`title`/`desc` + 逐条效果凭据），
/// 多出来的是 `scope` 与 `audience`：**同一条效果打在几个人头上**是新闻才有的形状，
/// 而「谁挨了哪一下」必须逐人留在流水里（验收②「全场类新闻影响到的每个人都留流水」）。
#[derive(Debug, Clone, PartialEq)]
pub struct NewsNotice {
    pub id: String,
    pub title: String,
    pub desc: String,
    pub scope: crate::news::NewsScope,
    pub audience: NewsAudience,
}

/// 一条新闻点到的是**人**还是**地**。
///
/// 两档不是同一个问题的两种写法：人档的主语是承受者（他的钱、他的停走），地档的主语是
/// 那片地（谁被拆了、哪几块盖上了状态）。把它们塞进同一个 `Vec<NewsHit>` 会得到
/// 「一块地的 `player` 填谁」这种没有答案的字段 —— 而**无主的那几块也得列出来**，
/// 按业主分组恰好会把它们丢掉（外星人拆了三块地、其中一块没人要，那句话仍然要说得出）。
#[derive(Debug, Clone, PartialEq)]
pub enum NewsAudience {
    /// 人档：每个人一笔（`NewsHit::player` 是承受者，与播报者可以是同一个人）。
    /// 空表说的是「点名点不到人」（只剩他一个人时的 `random_other`），不是「没播」。
    People(Vec<NewsHit>),
    /// 地档：一次打在一片地上。`center` 是锚（P11b① 用的是播报者脚下那块地），
    /// `lands` 是**真的被扫到的那几块**（含无主的、含本来就是空地的），`outcomes` 是这一片上
    /// 办成的事。`center == None` 只可能是「这张盘上一块地都没有」，那时 `lands` 必空。
    Ground {
        center: Option<crate::land::LandId>,
        lands: Vec<crate::land::LandId>,
        outcomes: Vec<EffectOutcome>,
    },
    /// 机构档：一次改一条规矩（P11c②）。既没有人挨它（谁的钱包都没动）也没有地挨它
    /// （街上哪一块都一样），所以前两族的字段在这里都没有答案 ——
    /// 硬塞进 `People` 会得到「承受者填谁」那一格，硬塞进 `Ground` 会得到一个假锚点。
    ///
    /// `days` 是**叠完之后**世界上的剩余天数（不是条目上那个数），因为顶栏那句要念的
    /// 就是「从现在起停几天」；`outcomes` 不带 —— 这一条只有一条动词、一个数，
    /// 再列一份「办成了什么」的清单就是把同一个数说两遍。
    Institution {
        what: crate::institution::Institution,
        days: u32,
    },
}

/// 一条新闻落在一个人头上的那些事。文案由 UI 从 `outcomes` 生成，结算层不拼字符串
/// （与 [`CardNotice`] 同一条分工）。
#[derive(Debug, Clone, PartialEq)]
pub struct NewsHit {
    pub player: Entity,
    pub outcomes: Vec<EffectOutcome>,
}

// ===== 道路放置物与道具（P5）=====

/// 把一件放置类道具放到主环的某一格（路障／地雷／定时炸弹）。
///
/// 主语与窗口由 `occupancy::place_item_system` 判（当前回合玩家、掷骰前、没有待决事项），
/// 目标格的三条合法性由 `occupancy::place_denied` 判 —— 两处合起来与 UI 的灰按钮同一句话。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlaceItemRequested {
    pub player: Entity,
    pub item: crate::items::ItemKind,
    pub tile: TileIndex,
}

/// 放置成功。`item` 已经从道具栏扣掉；被拒的那一笔不发这条。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemPlaced {
    pub player: Entity,
    pub item: crate::items::ItemKind,
    pub tile: TileIndex,
}

/// 走步被道路放置物中途打断（地雷／定时炸弹／恶犬）。
///
/// **与 [`PlayerArrived`] 互斥**：中断不是到站，脚下那一格不结算；逻辑位置已经改回
/// `tile`，剩余的步数一步都不走（ADR-002 §7）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct WalkHalted {
    pub player: Entity,
    pub tile: TileIndex,
    pub cause: crate::occupancy::HaltCause,
}

/// 定时炸弹在路径经过另一名玩家所在格时自动换主人（ADR-002 §8）。
///
/// 不问、不挂 `Pending`：这是「相遇」的直接后果。`tile` 是他们相遇的那一格，
/// `steps_left` 是换手时炸弹还剩几步（流水里要能看出它没有因换人而重置）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BombTransferred {
    pub from: Entity,
    pub to: Entity,
    pub tile: TileIndex,
    pub steps_left: u32,
}

/// 落脚拾取宝箱：得到一个随机道具（`occupancy::settle_landing_occupancy_system`）。
///
/// 路过不给 —— 宝箱是落脚触发种类（ADR-002 §5）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChestOpened {
    pub player: Entity,
    pub tile: TileIndex,
    pub item: crate::items::ItemKind,
}

/// 用一颗遥控骰子决定**精确步数**（1..=6）。
///
/// 与 [`RequestRoll`] 分成两条：那条是「摇几颗骰子」，这条是「这一回合走几格」。
/// 道具在受理时扣掉；被拒的那一笔不发 [`RemoteDiceUsed`]。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RequestRemoteDice {
    pub player: Entity,
    pub steps: u32,
}

/// 遥控骰子生效：这一回合按 `steps` 走（乌龟卡的夹步仍会把它压到 1）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RemoteDiceUsed {
    pub player: Entity,
    pub steps: u32,
}

/// 乌龟卡／夹步生效：骰子掷出 `rolled`，实际只走 `moved` 格。
///
/// **`DiceRolled.steps` 仍是 `rolled`**（P3d④b 与 ADR-002 §7.1 的口径）：
/// 骰面还是骰面，「这一回合走几格」是另一件事。P10 的 C 型收费读前者。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveCapped {
    pub player: Entity,
    pub rolled: u32,
    pub moved: u32,
    /// 夹完之后还剩几回合；`0` = 这一夹用完了。
    pub turns_left: u32,
}

/// 载具变了（P5：地雷/炸弹把它炸毁时回到步行）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MountChanged {
    pub player: Entity,
    pub mount: crate::player::Mount,
}

// ===== 神明（P6）=====

/// 一位神怎么走的。三种走法的差别是玩法，不是日志等级：
/// 玩家要能分辨「我自己踩到另一位把它顶了」（可以怪自己）、「日子到了」（谁也不怪）
/// 和「花点券请它走的」（花了钱）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GodExit {
    /// 天数用完，自然走
    Expired,
    /// 踩到另一位神，被顶走（一人一神）
    Replaced,
    /// 被送神符送走（P6d）—— 死神不吃这一张
    Banished,
}

impl GodExit {
    /// 顶栏、HUD、dump 与测试流水要说同一句话，所以映射只写一次（与
    /// [`DetentionPlace::label`] 同一条理由）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Expired => "期满",
            Self::Replaced => "顶走",
            Self::Banished => "送走",
        }
    }
}

/// 路上刷出一位神（开局装配与每回合补刷都算）。
///
/// 漫游**不发这一条**：每回合几位神各挪一格会把顶栏刷成滚动字幕，而那件事没有玩家要
/// 回答的动作。要看神此刻站在哪儿，读 dump 的放置物那一栏。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GodAppeared {
    pub god: crate::gods::God,
    pub tile: TileIndex,
}

/// 踩到神 → 上身。`replaced` 是被它顶走的那位（`None` = 他身上原本没有神）。
///
/// `days` 是这一位管几天（死神 13、其余 7）：写成数而不是让面板去查表，
/// 因为「HUD 上那句还剩几天」要能与「上身那一刻写下的那个数」对得上账。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GodPossessed {
    pub player: Entity,
    pub god: crate::gods::God,
    pub days: u32,
    /// 踩到它的那一格。
    pub tile: TileIndex,
    pub replaced: Option<crate::gods::God>,
}

/// 一张卡**离开手牌但不是被打出**（P6b② 衰神每回合弄走一张）。
///
/// 为什么不给 `CardPlayed` 加一个「谁弄走的」字段：那条说的是「某人按了某颗按钮」，
/// 带着出牌人与目标，而这里没有人按任何东西 —— 玩家问的是「我明明有卡，它去哪了」，
/// 这一条说得出是**哪位神**干的。`Hand::spend` 那边也不动：它是出牌那条路的出口。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct CardLost {
    pub player: Entity,
    pub card: CardNotice,
    /// 谁弄走的。今天只有衰神，但这一格是这条消息存在的全部理由
    pub by: crate::gods::God,
}

/// 一位神离开某人。与 [`GodPossessed`] 分成两条而不是带一个方向字段的同一条：
/// 一来一回的读者不同（附身要播动画、离场只要把挂着的修正与 HUD 那一行清掉），
/// 而 `GodExit` 已经说了「怎么走的」。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GodLeft {
    pub player: Entity,
    pub god: crate::gods::God,
    pub reason: GodExit,
}

/// 一位神上身那一刻**当场做完的那件事**（P6b）：送钱、挨个收钱、抢钱、送卡。
///
/// 与 [`GodPossessed`] 分成两条是刻意的：那一条说的是「谁身上换了一位神、管几天」，
/// HUD 与将来的存档读它就够；这一条说的是钱与卡的去处，顶栏那句「大财神给了你 3,521 元」
/// 与老虎机动画读它。合成一条带可选字段的话，「附身但这次没给东西」与「没附身」就分不开了。
///
/// `cash` 是这个人现金的**净值**（进为正、出为负），只给面板一句话用；每一笔的明细照旧走
/// [`CashChanged`]，所以它对不上账时审计仍然抓得住（B3 那条「余额 == 起始 + Σdelta」）。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct GodDeed {
    pub player: Entity,
    pub god: crate::gods::God,
    pub cash: i64,
    /// 送到手上的卡（大福神 2 张、小福神 1 张，其余为空）。
    pub cards: Vec<CardNotice>,
}

/// 一局小游戏的成绩（P7，表现层 → 规则层的意图）。
///
/// 规则层不实现玩法本体（PLAN §五），所以这一条是那个接口**唯一**的入口：玩成什么样由
/// `game_app/src/ui/minigame.rs` 说了算，它把结果报上来，换多少点券由
/// [`crate::minigame::Payouts`] 那张表算 —— 表现层不参与定价。
///
/// `awarded` 是那一格**显式的奖励开关**：主菜单的练习局报 `false`，规则层照样记一条
/// [`MinigameScore`]、但一分不发。不靠「练习局不发消息」来实现，是因为隐式的奖励开关
/// 最难查 —— 症状是「点券莫名其妙多了」，而那条路径上没有任何一行代码提到练习。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinigameResult {
    pub player: Entity,
    pub game: crate::minigame::GameKind,
    /// 这一局打出的成绩。单位由那一款自己定（气球几只、财宝几件、挖到几格宝），
    /// 规则层只把它喂进那张表。
    pub score: u32,
    pub awarded: bool,
}

/// 一份成绩已经结算（P7 的凭据）。
///
/// 与 [`MinigameResult`] 分成两条而不是复用一条：那条是「玩家报上来的数」，这条是
/// 「规则层认了、并且换成了点券」。多出来的 `points` 正是这一条存在的理由 ——
/// 顶栏那句「得分 21 → 84 点券」要能与 HUD 上点券的变化对得上账。
///
/// 它**不产生**一条 `CashChanged`：点券是另一套账（PLAN §一「永不通兑」），
/// 所以这一笔在现金审计里看不见，只出现在点券审计里。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinigameScore {
    pub player: Entity,
    pub game: crate::minigame::GameKind,
    pub score: u32,
    /// 真正发下去的点券。练习局（`awarded == false`）恒为 0。
    pub points: u32,
    pub awarded: bool,
}

/// 有人要买一注（P8，表现层 → 规则层的意图）。**号码烤在请求里**（那颗「17」本身就是一整个
/// 意图，与 `ShopOpRequested` 带的卡、`BailRequested` 带的人同一条形状），而价钱与「买不买得成」
/// 由 [`crate::lottery::buy_allowed`] 判 —— 面板不许自己定价。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BuyTicketRequested {
    pub player: Entity,
    pub number: u8,
}

/// 奖池动了（P8）。**奖池唯一的流水** —— PLAN §四 P8 那条验收「每笔进池的钱都能从流水
/// 追溯」就是靠这一条，而不是靠读代码数加法。
///
/// 它**不是**一条 `CashChanged`：池子不是任何人的钱包（10-22 拍板：买券钱 + 月末利息滚存，
/// 不从任何玩家的既有账上抽）。所以这一笔在现金审计里看不见，与点券那两套账同一条纪律。
/// `balance` 是动完之后的余额，面板与 dump 直接念它，不必自己累加。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct JackpotChanged {
    /// 进为正、出为负。
    pub delta: i64,
    pub balance: i64,
    pub why: JackpotWhy,
}

/// 这一笔为什么动。写成穷尽表（与 `landing::domain_of`、`charge::scales_with_price` 同一条
/// 纪律）：将来给池子加第三条来源（比如「流局的那一期把某笔钱留在池里」）时，
/// 必须在这里加一格，于是面板那句「这个数是怎么来的」不会静默地说不出口。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum JackpotWhy {
    /// 有人买了一注（全额进池，规则层不抽头）
    Ticket,
    /// 月末利息（`GameRules::jackpot_rate`，与储金结息同一条日子）
    Interest,
    /// 发奖出池
    PaidOut,
}

impl JackpotWhy {
    /// 面板与顶栏那句「这笔钱哪来的」。住在这里而不是散在 UI 里：规则层与面板要说同一句话。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Ticket => "买券",
            Self::Interest => "月息",
            Self::PaidOut => "发奖",
        }
    }
}

/// 有人买了一注乐透券（P8）。`price` 是**实付**（过不了物价：买券是账面搬运，
/// 不在结算管线上，理由见 `charge.rs` 模块头那句「管线的边界」）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LotteryBought {
    pub player: Entity,
    pub number: u8,
    pub price: i64,
}

/// 开奖那一刻（P8）。`winner == None` 就是**滚存**：这个月没人猜中，池子与所有人的券都留到
/// 下月（PLAN §三「无人中奖则累积、号码留到下月」）。
///
/// 它带的不是「谁中了」而是「开了几号、池子里有多少、这一期归了谁」—— 那三样在无人中奖时
/// 仍然要说得出，所以不能只发 [`LotteryWon`]（那条一位中奖者都没有）。
///
/// **一个人，不是一个人数**：号码独占（10-26 实报）让中奖号至多只有一位还活着的持有者，
/// 所以这里给的是「谁」而不是「几个、每人多少」。`pot` 同时是答案的两半 —— 独得整池这条
/// 规则成立时，「开奖前池里有多少」与「他拿走多少」是同一个数。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LotteryDrawn {
    pub number: u8,
    /// 独得整池的那个人。`None` = 本期流局。
    pub winner: Option<Entity>,
    /// 开奖**之前**那一刻的池子。发奖那条 [`JackpotChanged`] 带的是出多少，这一句带的是
    /// 「这一期总共攒了多少」—— 玩家问的是后者。
    pub pot: i64,
}

/// 一位中奖者（P8）。带名字的那一条，顶栏念的是它；`amount` 与他那条 `CashChanged`
/// 的 delta 必须相等（`tests/lottery.rs` 钉这一格）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct LotteryWon {
    pub player: Entity,
    pub number: u8,
    pub amount: i64,
}

// ===== 拍卖（P9）=====

/// 一次拍卖里的两个动作。**没有「出价多少」这一格**：金额由 [`crate::auction::Lot::next_amount`]
/// 从「当前最高价 + 一档」算出来，请求侧只能表达「我要继续」或「我不玩了」。
///
/// 为什么不做成任意喊价：任意价会让面板必须给一个输入框（而这一屏是鼠标点的），
/// 会让「同种子复现同一串拍卖」多一个来自客户端的自由变量，而英式拍卖里真正在玩的
/// 从来不是「喊多少」而是「敢不敢再加一档」。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub enum BidMove {
    /// 加一档（没人认价时那一档就是起拍价本身）
    Raise,
    /// 放弃并退出这一场（之后不能再回来）
    Withdraw,
}

/// 有人对正在进行的那一场做了一个动作（P9）。这是**意图**，不是结果 ——
/// 与 `RequestStay`／`RollRequested` 同一条纪律：AI 与人类发同一条，判据只在规则层。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionMove {
    pub player: Entity,
    pub what: BidMove,
}

/// 开场那一句（P9）。`bidders` 是**有资格出价的人数**，而 `by == None` 说的是
/// 「这是破产清算，没有开槌的人，所得归银行」—— 顶栏那句要区分这两件事，
/// 否则玩家会以为某个玩家在卖自己的地。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionStarted {
    pub land: LandId,
    pub by: Option<Entity>,
    pub seller: Option<Entity>,
    pub opening: i64,
    pub step: i64,
    pub bidders: u32,
}

/// 有人喊了一个价（P9）。`amount` 是**他这一刻认下的数** —— 成交那条 [`AuctionWon`] 带的
/// 是最终那个数，而顶栏要播的是这一条（每一次加价都得看得见，否则拍卖看起来像一次点击）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BidPlaced {
    pub player: Entity,
    pub land: LandId,
    pub amount: i64,
}

/// 有人退出了这一场（P9）。他**不再被问**，所以面板上该把他念成「已放弃」而不是消失 ——
/// 消失的话玩家会以为那个人离场了（`PlayerBankrupt` 是另一条凭据，会明说）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BidWithdrawn {
    pub player: Entity,
    pub land: LandId,
}

/// 落槌（P9）。`by == None` 时钱进了银行（没有人收到它），所以**不要**在这条之外
/// 再假想一条「原主拿到补偿」—— 10-22 拍板：变卖所得不分配、拍卖卡所得归开槌的人。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionWon {
    pub player: Entity,
    pub land: LandId,
    pub price: i64,
    /// 谁开的槌（钱给他）。`None` = 破产清算。
    pub by: Option<Entity>,
    /// 从谁手里拍走的
    pub seller: Option<Entity>,
}

/// 流标（P9）。它带着 `price` 而不是只有「流标」两个字：玩家问的下一句就是「起拍多少都没人要」。
///
/// 两种流标在这一条上**不分叉**：没人出价，与有人喊到落槌却付不出（拍卖期间他仍能动 ATM）。
/// 后者由 `warn!` 单独说清 —— 它是世界状态真的允许的路径，但不是另一种结果：地都没转手。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuctionFailed {
    pub land: LandId,
    pub by: Option<Entity>,
    pub seller: Option<Entity>,
    pub price: i64,
}

// ===== 四大恶人（P12b–d）=====

/// 一位恶人被保释出来、开始替人办事（P12b）。
///
/// 这是**出场行为的入口**，不是身份变化：`Hired` 组件在 `bail_op_system` 里挂、轮转在那一句
/// `insert_after` 里插，两者都是「他出来了」的结构事实；这条消息说的是「此刻起该跑他那份行为」
/// —— 强盗抢存款、小偷偷点券都发生在**出场那一帧**，与他在环上走到哪无关。分派在
/// [`crate::villain::on_hired_system`]：`bail.rs` 发完就走，不认识任何一种恶人的名字。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct VillainHired {
    pub villain: Entity,
    pub employer: Entity,
    pub kind: crate::villain::VillainKind,
}

/// 强盗出场：全场存款被抢走一笔（P12b）。
///
/// `total` 进的是**强盗自己的口袋**（世界外）—— 他没有钱包，这笔钱不存在「转给谁」的出口，
/// 而「含雇主自己」正是 PLAN 拍板的那句特性（不是 bug）。每个玩家少了多少读
/// [`BankOpDone`]？读不到 —— 这不是银行操作；存款变动的可见凭据只有这一条汇总，
/// 份额要逐人核对去 dump 的现金栏（顶栏的播报给总数，那是玩家真正要读的数）。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct DepositsRobbed {
    pub villain: Entity,
    pub employer: Entity,
    pub total: i64,
    /// 被抢的人数（0 存款的人不算 —— 没有什么可抢的）。
    pub victims: u32,
}

/// 小偷出场：其他玩家的一半点券进了雇主口袋（P12c）。
///
/// 每一位玩家的点券变动各自发过 [`PointsChanged`]（走 [`crate::settle::Books::spend_points`]
/// 与 `award_points` 那条正路），这一条只说**汇总**：谁偷的、给谁、一共几格。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointsStolen {
    pub villain: Entity,
    pub employer: Entity,
    pub total: u32,
    /// 被偷的人数（0 点券的人不算）。
    pub victims: u32,
}

/// 小偷把路上的一件放置物搬走了（P12c）。
///
/// 地雷与路障在**走步经过**那一帧由 `walk_step_system` 发（偷窃必须与地雷的「踩到即炸」
/// 同帧裁定，晚了人就进笼了）；宝箱在**落脚**那一帧由
/// `settle_landing_occupancy_system` 发 —— 宝箱开出什么由掉落子流当场抽好，
/// 消息里带的是**抽到的结果**而不是「一只宝箱」：道具栏里没有「宝箱」这一格，
/// 「偷来给雇主」落进雇主道具栏的只能是具体的东西。
///
/// 雇主由 [`crate::villain::thief_deliver_system`] 查 `Hired` 后收货 —— 发送侧不查：
/// `walk.rs` 该说的是「这一格少了一件东西」，「该给谁」是恶人模块的家务。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ItemThieved {
    pub thief: Entity,
    pub item: crate::items::ItemKind,
    pub tile: TileIndex,
}

/// 强盗与一位玩家擦肩，从他手里抢走一张卡（P12b）。
///
/// 「擦肩」的判据与炸弹转让同一条：路径经过**对方当前所在格**的那一步。目标不问雇主 ——
/// 刀口不对着自己人；但那一格上站着的**每一位**符合条件的玩家都会被抢一张（擦过一圈）。
/// `card` 是被抢走的那张：受害者与雇主的手牌面板都按签名重建，各自会跟上；
/// 这一条是给顶栏播报的凭据，也回答「我那张陷害卡去哪了」。
#[derive(Message, Debug, Clone)]
pub struct CardSnatched {
    pub robber: Entity,
    pub victim: Entity,
    pub employer: Entity,
    pub card: CardNotice,
}

/// 流氓路过别人的地，向地主勒索了地价等额的现金（P12b）。
///
/// 钱归**流氓自己**（PLAN §三 拍板的反制窗口：雇主一分拿不到，而流氓没有钱包，
/// 所以这笔钱在世界外消失 —— `Books::charge` 那条路只扣不加）。地主付不起时走的是
/// 与租金同一条破产出口：被流氓勒到破产是这条规则字面就允许的事。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TollExtorted {
    pub hoodlum: Entity,
    pub land: LandId,
    pub owner: Entity,
    pub amount: i64,
}

/// 间谍把本该付给地主的租金转给了雇主（P12d）。
///
/// 这一笔在 [`crate::settle::Books::settle`] 里就已经改了收款人（钱从结算那一刻就走对口袋，
/// 不做「先给地主再转手」的两段账），[`RentDue`] 的 `owner` 带的同样是实际收款的雇主 ——
/// 这一条单独存在，是因为「阿土伯的租为什么进了孙小美的口袋」需要一个明说的答案。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct RentRerouted {
    pub spy: Entity,
    pub employer: Entity,
    pub land: LandId,
    pub amount: i64,
}

// ===== 魔法屋（P13）=====

/// 一位玩家踩到魔法屋，女巫的两步法事开台（P13）。
///
/// 它只是**报到**：条件的候选名单、施法的出口都在窗口里，规则层不预设任何一步。
/// 与 [`BankVisited`] 同一条形状 —— 面板与 autopilot 读它弹 UI。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagicOffered {
    pub player: Entity,
}

/// 玩家选定了一条**筛选条件**（第一步的回答，P13）。
///
/// 这是意图不是结果：条件合不合法、筛出几个人，由 [`crate::magic::magic_filter_system`]
/// 对着窗口判。与 [`RequestRoll`] 同一条纪律 —— 人、AI、旋钮发同一条。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagicFilterPicked {
    pub player: Entity,
    pub filter: crate::magic::MagicFilter,
}

/// 女巫按条件筛出了**这些人**（第二步窗口的名单，P13）。
///
/// 名单带在消息上而不是让 UI 自己再筛一遍：筛选的口径（并列怎么取、破产者算不算）
/// 只该有一份答案，而「第二步窗口显示的人」与「法术真正落到的头」必须是同一批。
/// **空名单也发这一条** —— 那正是「落空」的凭据：窗口当场收掉，第二步不会开。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct MagicTargets {
    pub player: Entity,
    pub filter: crate::magic::MagicFilter,
    pub targets: Vec<Entity>,
}

/// 玩家选定了**一道法术**（第二步的回答，P13）。与 [`MagicFilterPicked`] 同一条纪律。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagicSpellPicked {
    pub player: Entity,
    pub spell: crate::magic::MagicSpell,
}

/// 一道法术施放完毕（P13）。`targets` 是**实际生效**的承受者 ——
/// 被免罪挡掉的那一位不在这里（他有自己的凭据 [`MagicBlocked`]），
/// 「落空」（脚下没有可加盖的地、手牌本来是空的）也不进名单。
#[derive(Message, Debug, Clone, PartialEq)]
pub struct MagicCast {
    pub player: Entity,
    pub spell: crate::magic::MagicSpell,
    pub targets: Vec<Entity>,
}

/// 免罪挡下了一道法术（P13）。十二种法术里只有坐牢与住院走得进这一条：
/// 那两条构造 [`crate::afflict::Affliction`] 过护身，其余十条根本不进那条路 ——
/// 「不受任何卡与神影响」是构造方式的差别，不是一次查询能答的。
#[derive(Message, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MagicBlocked {
    pub player: Entity,
    pub spell: crate::magic::MagicSpell,
    pub victim: Entity,
}
