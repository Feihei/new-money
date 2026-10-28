//! P14 AI 玩家 —— **一个会自己发意图的座位**，而不是第二套规则。
//!
//! 它发的与真人点按钮时发的**是同一条消息**（[`RequestRoll`]／[`DecisionChosen`]／[`AuctionMove`]
//! …），判据一条都不复制：真人能做的事 AI 都能做，AI 能做的事真人全都能在画面上看到。
//! P12a 的恶人已经是这个形状的第一个样板（`villain::villain_turn_system` 发的就是同一条
//! `RequestRoll`/`RequestStay`），这一模块把同一件事扩展到「玩家座位」这一族。
//!
//! ## 为什么住在规则层而不是表现层
//!
//! 打分要读账本：这块地买得起吗、下一笔要付多少、名下有几块押着的地。表现层那份一次性的
//! 脚手架（`crates/game_app/src/autopilot.rs` 的 `answer_for`）之所以只会「一律接受」，正因为它
//! 是个只有规则表的纯函数，**拿不到世界**。装进 core 还换来第二条更硬的：验收那句
//! 「3 AI + 1 人类可完局」从此在 `cargo test -p game_core` 里成立，不必开窗口。
//!
//! ## 两条 system 注册在 `PreUpdate`
//!
//! 不是「排在 `decision_system` 之前」那种隐式顺序 —— §6.1 坑 36 说得很直白：并行元组内部
//! 没有顺序可言，于是「这一帧发的回答当帧读不读得到」变成帧相关的。测试夹具那份
//! `auto_answer_system` 早就写在 `PreUpdate` 并把理由写在注释里，AI 抄同一条路：
//! **发完就在同一帧被任何 `Update` 里的读者读到**，谁先谁后不再有意义。
//!
//! ## 延迟不改答案
//!
//! [`AiCadence`] 只决定**什么时候开始问**，[`decide`] 只看世界此刻的样子。这两件事分开之后，
//! 「同一颗种子跑出同一条轨迹」不再依赖帧率 —— 高刷屏与低刷屏上 AI 想的久一点或短一点，
//! 想的结果一样。取那个间隔走第 20 条子流 [`pick_ai`](crate::rng::Rng::pick_ai)：
//! 「AI 想了多久」不该改变下一张机会牌是谁，就像「这一局进过几次店」不该改变它一样。
//!
//! ## 穷尽留在 [`greedy`]，打分长在 [`AiHand::decide`]
//!
//! [`greedy`] 对 17 个 [`Pending`] 变体**每一格都有答案**，而那份答案就是 P14a 交付的「今天
//! 脚手架那一份」。为什么必须先穷尽：[`Pending::Auction`] 与 [`Pending::Passive`] 对三种
//! [`Decision`] 一条都不认（[`decision_system`](crate::landing::decision_system) 里那一格），
//! 答成 `Acknowledge` 等于没答，而**症状是整局静默停住** —— 不 panic、不 warn，画面还在动，
//! 只是再没人行动。新增一种窗口而忘了给答案，编译器当场替我们响（与 `landing::domain_of`
//! 同一条纪律）。
//!
//! P14b 起 [`AiHand::decide`] 在**它已经会算账的那几格**上盖过它（钱那一刀先接 `Buy` 与
//! `Upgrade`），其余一律落回 `greedy`。这条分工有两个后果，都是要的：
//! ① 穷尽匹配只有一处，所以「AI 会不会把回合机挂死」那条编译期网不会因为新增一臂而漏；
//! ② `decide` 里少写一臂只是「还没学会」，不是「会停住」——于是这一刀可以真的只切两格，
//!    不必为了对称而把十七格各改一遍（PLAN §四・十五 切五刀的理由）。
//!
//! ## 回合前理财（P14b③）：掷骰之前先看一眼活钱
//!
//! 变现这件事在规则层有两个入口，而它们**互不重叠**：押地与借贷只能在银行柜台的窗口里办，
//! 卖股与取款却挂在「自己回合、掷骰之前」（`market::trade_request_system` 只在 `WaitRoll`
//! 跑）。于是 [`ai_roll_system`] 从「一发掷骰」变成两步：先问一句「活钱够不够水位」，
//! 不够就卖股，**每回合最多理一次、最多动一只股**，第二帧才掷骰。
//!
//! 三个约束是刻意的：
//! ① **每回合只一次、只一只**（[`AiErrands::rebalanced`] 带回合号）：「AI 炒股要尽量少」
//!    不是一句口味，它得在结构上成立。一次动作只动一只股，而一次动作之后这条 system 才掷骰。
//! ② **只卖不买除非闲钱多得没处去**：有主的地一块都不剩（[`AiHand::land_left`]）才轮到股票，
//!    而买只在「跌到近一均值以下」时发生（[`AiProfile::STOCK_DIP`]）—— 见着有闲钱就买，
//!    那不是理财是做市。
//! ③ **取款那一档不写**：AI 的存款恒为 0（存钱只有柜台存现金与女巫的「存现金」两条路，
//!    AI 两条都不走），写进来就是一条永远不响的候选 —— 与 P14b② 那条赎回空档同一类坑。
//!    真要有存款了，这里加一档的判据是 `market_halt_now` 之外什么都不用问。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::bank::bank_op_allowed;
use crate::calendar::HolidaySet;
use crate::charge::ChargeKind;
use crate::economy;
use crate::institution::Institution;
use crate::land::LandId;
use crate::landing::{Pending, PendingDecision};
use crate::lottery::Jackpot;
use crate::market::{MarketClock, market_halt_now};
use crate::minigame::GameKind;
use crate::mortgage::{counter_plots, mortgage_op_allowed, read_facts};
use crate::msg::{
    AuctionMove, BankOp, BankOpRequested, BidMove, BuyTicketRequested, Decision, DecisionChosen,
    MinigameResult, MortgageOp, MortgageRequested, PassiveAnswer, PassiveMove, RequestRoll,
    RequestStay, StockTradeRequested, TradeSide,
};
use crate::player::StaySentence;
use crate::rng::Rng;
use crate::rules::GameRules;
use crate::settle::Books;
use crate::stock::{PriceHistory, StockId, StockPrice};
use crate::turn::TurnOrder;

/// 这个座位由谁开车。**必挂**：查不到组件的人按 [`Controller::Human`] 处理。
///
/// 「默认等人点」而不是「默认替他决定」是刻意的：漏挂一处装配的后果是「这个座位不动」，
/// 那看得见、报得出、排查得动；反过来（漏挂＝AI 代打）的后果是「玩家的操作被静默吃掉」，
/// 而那种症状没人报——他只会以为游戏坏了。与 §6.1 坑 34 那一族同形。
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub enum Controller {
    /// 真人：等 UI 发意图
    Human,
    /// 电脑：读这份先验自己发
    Ai(AiProfile),
}

impl Controller {
    pub fn is_ai(&self) -> bool {
        matches!(self, Controller::Ai(_))
    }

    /// 不是 AI 座位就算人类（包括**根本没挂组件**的那种）。
    pub fn is_human(&self) -> bool {
        !self.is_ai()
    }

    pub fn profile(&self) -> Option<&AiProfile> {
        match self {
            Controller::Ai(profile) => Some(profile),
            Controller::Human => None,
        }
    }
}

/// AI 的先验：**五个偏好 + 一个失误率**。
///
/// 前五格是 [CHARACTERS.md 表 B](../../../.agents/CHARACTERS.md) 那四维加上 P14 要的第五格
/// `harm`（害意）——那一格管「用不用卡与道具往别人头上落事」，四维里没有任何一格表达它，
/// 而 P14 一半以上的判断都落在上面。`fumble` 不是第六格偏好而是「这次算不算错」的概率：
/// 它不参与打分，只决定要不要改用次优的那一个，混进向量会让「大佬奸很少犯错」这句话
/// 变成「大佬奸买地阈值高」。
///
/// **三档性格就是这三条预设向量，不是一条枚举通道**：`decide` 里没有任何一处
/// `match` 过「哪一档」，「大佬奸」在代码里永远是一行读数而不是十处 `if`。
/// `who` 只上屏（dump 与 HUD 念它），**没有任何判据读它** —— 那是「性格是数不是枚举」
/// 这条约束唯一允许的字符串，将来 `characters.json` 一人一行时填的是角色名。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AiProfile {
    /// 买地时敢把现金压到多低（0 ＝ 只肯花零头）
    pub buy: f32,
    /// 敢不敢动身家换现金：贷款、押地、卖股是同一句话的两个方向
    pub debt: f32,
    /// 买不买股
    pub stock: f32,
    /// 升不升级、建不建
    pub build: f32,
    /// 害意：用卡、放道具、进魔法屋选哪一道
    pub harm: f32,
    /// 失误率：落在这一格之内就改用次优的那个答案（P14b 第一处打分起才有读者）
    pub fumble: f32,
    pub who: &'static str,
}

impl AiProfile {
    /// 大佬奸：爱买股、常用卡道具害人、进魔法屋也选人害。
    pub const SHARK: AiProfile = AiProfile {
        buy: 0.90,
        debt: 0.80,
        stock: 0.90,
        build: 0.80,
        harm: 0.90,
        fumble: 0.05,
        who: "大佬奸",
    };
    /// 普通人：中间那一档，而场上多数本该是这一档。
    pub const PLAIN: AiProfile = AiProfile {
        buy: 0.55,
        debt: 0.40,
        stock: 0.40,
        build: 0.50,
        harm: 0.30,
        fumble: 0.15,
        who: "普通人",
    };
    /// 乖宝宝：基本不买股、很少害人，而且**缺钱也不动自己的资产**（4 代实测那一档的归宿）。
    pub const GOODY: AiProfile = AiProfile {
        buy: 0.30,
        debt: 0.10,
        stock: 0.10,
        build: 0.25,
        harm: 0.05,
        fumble: 0.35,
        who: "乖宝宝",
    };

    /// 三档预设。座位不够分时按这个次序领。
    pub const THREE: [AiProfile; 3] = [AiProfile::SHARK, AiProfile::PLAIN, AiProfile::GOODY];

    /// **肯不肯动身家的那道门槛**：`debt`（或 `stock`）低于它的座位，缺钱时宁可破产也不
    /// 押地、不借、不卖股。
    ///
    /// 这是一个设计值而不是算出来的数，理由要说清：PLAN §四・十五 的 ② 拍的是「按性格分叉」
    /// 而不是「按缺口分叉」—— 4 代实测那条「AI 从不抵押，直到破产」是一个**人**的行为，
    /// 所以它必须能被某一档性格复现，而不是被某一档的算术复现。放在门槛而不是门槛之外，
    /// 是为了让它可被一句断言钉住（乖宝宝在同一个世界里看着自己破产，见 `tests/ai.rs`）。
    ///
    /// 三档与它的距离是刻意的：大佬奸 0.80（+0.55）、普通人 0.40（+0.15）、乖宝宝 0.10
    /// （−0.15）—— 中间那一档要留得出余量，否则挪一次预设值就翻掉「谁会自救」这句话
    /// （P14b① 在买地那格上刚被同样的 3% 余量咬过一口）。
    pub const LIQUIDIZE: f32 = 0.25;

    /// **肯不肯拿闲钱去买股的那道门槛**：`stock` 低于它的座位，闲钱再多也只是躺着。
    ///
    /// 「AI 炒股要尽量少」在这一格里落成一条门槛而不是一个折扣：三档离它是 +0.30 / −0.20 /
    /// −0.50，于是**一局里只有一个座位会碰股票**（大佬奸），而另外两档的 `stock` 仍然照旧
    /// 上屏、照旧在文档里读得出档次（它们是同一张表里的三行，只是这一行只有一个读者）。
    /// 与 [`Self::LIQUIDIZE`] 同一手：门槛是设计值，不是算出来的，改一次要连着看用例。
    pub const STOCK_APPETITE: f32 = 0.60;

    /// **跌到近一均价的几成才算「便宜」**（现价 ÷ 历史均价 ≤ 它才买）。
    ///
    /// 有闲钱是一件事，见着有闲钱就下单是另一件 —— 这条是「尽量少」的第二半：
    /// 现价还在均价之上时不买，只有真跌下去才接。`0.90` 是一个设计值（跌幅 10%），
    /// 与演示盘五只股 5–15% 的日波动同量级：跌得出来，但不会天天发生。
    pub const STOCK_DIP: f32 = 0.90;
}

/// 两次按按钮之间的人味：`NM_AUTOROLL` 那种长跑要 0，看人下棋要半秒到一秒半。
///
/// **core 的默认是两个 0** —— 无头测试里没有真实时间流逝（`Time::elapsed_secs()` 不前进），
/// 任何正数都会把 AI 钉死在第一帧不动，而那些用例要的是「AI 会走位」而不是「AI 会等」。
/// 真实客户端在 `insert_boot_resources` 里换成 `0.5..=1.5`（PLAN §四 P14 那句伪延迟），
/// `NM_AUTOGAP=<秒>` 把它压成一个常数（与表现层那套脚手架共用同一个旋钮：两个都在决定
/// 「这台机器多久动一次手」，分开拧只会拍出「AI 秒答而人类座位还在想」那种怪画面）。
#[derive(Resource, Debug, Clone, Copy, PartialEq)]
pub struct AiCadence {
    pub min: f32,
    pub max: f32,
}

impl Default for AiCadence {
    fn default() -> Self {
        Self { min: 0.0, max: 0.0 }
    }
}

impl AiCadence {
    /// 这一次动作之后隔多久再问。取数走 `pick_ai` 那条子流。
    pub fn gap(&self, rng: &mut Rng) -> f32 {
        if self.min <= 0.0 || self.max <= self.min {
            // 两个 0（测试）或区间退化（`NM_AUTOGAP=<秒>`）：不抽流，一次随机数都不该消耗
            return self.min.max(0.0);
        }
        self.min + (self.max - self.min) * rng.pick_ai()
    }
}

/// AI 那一条钟：下一次可以动手的时刻（墙钟秒）。
///
/// 是**资源**而不是 system 的 `Local`：`Local` 跟着 schedule 活，回主菜单再开一局时它带着
/// 上一局的 `due`，而那一局可能刚把 `NM_AUTOGAP` 拧过 —— 症状是「新局头几个 AI 座位迟迟不动」，
/// 与 ARCHITECTURE §6.1 坑 22/25 那一族（跨局残留）完全同形，只是藏在表现层看不见的地方。
/// 由 `begin_game_system` 每局归零。
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct AiClock {
    pub ready_at: f32,
}

/// AI 在银行柜台那一次访问里办的那一件。**一次访问只办一件**，理由见 [`AiErrands`]。
///
/// 金额是这一格里唯一由性格决定的东西：`debt` 不决定「借不借」（那是门槛
/// [`AiProfile::LIQUIDIZE`]），它决定「借这一层的几成」—— 于是同一句「敢不敢动身家」
/// 在大佬奸嘴里是 2400、在普通人嘴里是 1200，而不是一句「是 / 否」。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TellerMove {
    /// 什么都不办，直接按「办完」
    Idle,
    Borrow(i64),
    Repay(i64),
    Mortgage(LandId),
    Redeem(LandId),
}

/// 这一回合已经办过的那件「不改变世界状态就看不出来」的差事。
///
/// 为什么必须是**资源**而不是 system 的 `Local`：`Local` 跟着 schedule 活，回主菜单再开一局
/// 时它带着上一局的记录（§6.1 坑 22/25 那一族），症状是「新一局第一个路过银行的人什么业务
/// 都办不成」。由 `begin_game_system` 每局清空。
///
/// 为什么这一格非要有：柜台那两条业务（`bank_op_system`／`mortgage_op_system`）与
/// `decision_system` 分属两条 `add_systems(Update, ..)`，之间**没排过序** —— 于是「同一帧里
/// 既发业务又发『办完』」是一件靠调度运气的事（业务会被自己的『办完』挡在门外，因为
/// `at_counter` 读的就是窗口还开不开）。P8 用 `chain` 解过同一道题（`lottery_op_system`
/// 排在前），这里选另一条路：**分成两帧**，第一帧只发业务、第二帧只发『办完』，于是谁先谁后
/// 根本不出现在判据里。代价就是这一格记忆 —— 而它顺带把「一次访问只办一件」变成规则，
/// 挡住了另一条更坏的形状：押完一块地立刻有资格赎回它，那是一台不需要外力就能永远转下去的循环。
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct AiErrands {
    /// `(谁, 第几回合)`：这位在这一回合已经在柜台办过一笔了。
    pub banked: Option<(Entity, u32)>,
    /// `(谁, 第几回合)`：这位这一回合的**回合前理财**已经问过了。
    ///
    /// 它挡的不是「柜台」而是「同一回合掷骰之前反复下单」：AI 卖出之后要等下一帧才掷骰，
    /// 中间那几帧如果没有这一格，`treasury` 会再算一遍 —— 而缺口还在（卖一笔地租可能
    /// 补不满），于是同一回合里会一只一只卖下去。用户那句「每回合前最多买卖一只股票」
    /// 就是靠这一格**在结构上**成立的，不是靠候选表里只有一条。
    ///
    /// 与 `banked` 同一条生命周期：资源、每局清空（`Local` 会带着上一局的回合号），
    /// 且带回合号 —— 「谁在第几回合」这句话少一半就会跨人、跨回合误挡。
    pub rebalanced: Option<(Entity, u32)>,
}

/// 回合前理财那一笔（P14b③）：**一只股**，买或卖，不给第二只。
///
/// 只带「哪只、几股」而不带方向之外的东西：股数按缺口或闲钱算好，成交价与部分成交由
/// `Market::trade` 自己定 —— 与真人拖那条浮层的滑杆走的是同一条 `StockTradeRequested`，
/// 于是 AI 这边的「多少钱」与规则层那边不可能分叉。
///
/// 没有 `Idle`：没得办就是不办（[`AiHand::treasury`] 返 `None`），而「不办」不需要一条消息。
#[derive(Debug, Clone, PartialEq)]
pub enum TradeMove {
    Buy { stock: StockId, shares: u32 },
    Sell { stock: StockId, shares: u32 },
}

/// AI 对一问的回答。**六种答案形状，与真人点按钮时点的那六类东西一模一样**。
///
/// 单开一个类型而不是直接返回 [`Decision`]：游乐场那一格要回的是一份**成绩**、彩票柜台那
/// 一格要的是一笔**买券请求**、拍卖台那一个要的是**喊价**、被动门那一个要的是
/// **用／不用／给谁**、银行柜台那一个要的是**一笔业务** —— 这五种在类型上都不是 `Decision`，
/// 而把它们并成一个 enum 之后，新增一种窗口时必须当场决定 AI 怎么答它（穷尽匹配替我们响）。
///
/// 后面五臂存在的最低理由是**别把回合机挂死**：`decision_system` 对拍卖与门连三种
/// `Decision` 都不认，答成 `Acknowledge` 只是让长跑停在第一个破产或第一个被关的人那儿，
/// 而且不报错。
#[derive(Debug)]
pub enum AiMove {
    /// 报价与收据那两类窗口：接受／放弃／已阅
    Decide(Decision),
    /// 游乐场：报一份成绩（真对局，不是练习，所以 `awarded` 由调用侧给）
    Score { game: GameKind, score: u32 },
    /// 彩票柜台：买一注，然后还得有人关窗口
    Ticket,
    /// 拍卖台：加一档或放手
    Bid,
    /// 被动卡那道门：用／不用／嫁祸给谁
    Gate(PassiveMove),
    /// 银行柜台：办一件业务（**不带**「办完」，那一句在下一帧 —— 见 [`AiErrands`]）
    Teller(TellerMove),
    /// 回合前理财：卖一只股补水位／买一只跌下去的股（在 `WaitRoll` 里、掷骰之前）
    Trade(TradeMove),
}

/// 这一问的答案里**不打分**的那一份：P14a 的全部，也是 AI 还没学会算账时那些格子的默认值。
///
/// 分工不是讲究：柜台类窗口只认「已阅」（`decision_system` 那一格），一律发 `Accept` 的写法
/// 会让 AI 第一个踩到银行格的回合就永久停在 `WaitDecision`。
///
/// 游乐场报**满分的一半**：报 0 的话长跑里「成绩换点券」这条路径一次都没被走过。
/// 彩票答「买一注」而不是「离开」：不买的话奖池永远是 0，而 P8 那条验收
/// 「每笔进池的钱都能从流水追溯」就只在单测里成立过。
/// 门答「不用」而不是「用」：手里那张卡花掉一次会把一整片「他进了监狱」的既有断言
/// 翻成「他没进」——P14c 换成按 `harm` 与那道门槛打分之前，这一格维持原样。
///
/// ⚠️ **穷尽匹配留在这里而不是留在 [`AiHand::decide`]**：那张守「AI 不会把回合机挂死」的
/// 编译期网必须罩住所有窗口，而 `decide` 只打算显式列出**已经会打分**的那几格、其余一律
/// 落到这一份表上。于是新增一种窗口时编译器照样当场响（在这一格里），而 `decide` 里少写一臂
/// 只是「还没学会」，不是「会停住」。表现层那份替人类座位按按钮的脚手架读的也是这一份
/// （`game_app/src/autopilot.rs`）—— 它替的是一个不在键盘前的人，不是一份先验。
pub fn greedy(about: Pending, rules: &GameRules) -> AiMove {
    match about {
        Pending::Buy(_) | Pending::Upgrade(_) => AiMove::Decide(Decision::Accept),
        Pending::Minigame(game) => AiMove::Score {
            game,
            score: rules.minigames.of(game).perfect / 2,
        },
        Pending::Lottery => AiMove::Ticket,
        Pending::Auction => AiMove::Bid,
        Pending::RentNotice(_)
        | Pending::Card
        | Pending::BankTeller
        | Pending::Shop
        | Pending::CompanyNotice
        | Pending::Bail(_) => AiMove::Decide(Decision::Acknowledge),
        // 建设窗口（P10c）：答「先不建」——五类设施同价而效果差得很远，「建哪一类」问的是
        // 「谁会被收钱」＝`harm`，那是 P14c 那一批的判据（PLAN §四・十五 的 P14b ④）。
        // 这一格的代价说清楚：AI 局里商业地保持空地。
        Pending::Build(_) => AiMove::Decide(Decision::Acknowledge),
        // 远程建设窗口（P10d②）：与本地建设同一条回答，理由相同
        Pending::RemoteBuild => AiMove::Decide(Decision::Acknowledge),
        // 服务费收据（P10c）：钱已经扣完，与租金收据同一个回答
        Pending::ServiceNotice(_) => AiMove::Decide(Decision::Acknowledge),
        // 魔法屋的两步（P13）：**不施法**。法术的后果是坐牢、拍卖、变卖，会把长跑里
        // 几十条断言一起搅浑；P14d 按 `harm` 换掉这一格。
        Pending::Magic | Pending::MagicSpell(_) => AiMove::Decide(Decision::Acknowledge),
        // 被动卡那道门（P13b②）：答「不用」，与上面「不施法」、拍卖那一条「不喊价」同一条理由
        Pending::Passive => AiMove::Gate(PassiveMove::Decline),
    }
}

/// 「我肯为一块地（或一级楼）付出它一次租金的 N 倍」里的那个 N。
///
/// 一条直线而不是一个表：`0` 那一端要留一个**大于 1 的下限**（乖宝宝也不是不买房，他只是
/// 只肯买稳赚的），`1` 那一端要能盖住「回本就买」那种人（大佬奸）。`4..=28` 是按演示盘的
/// 租金表与地价比例挑的：一块 Lv1 的南京路一次租金 180、地价 1500，`1500/180 ≈ 8.3` 倍
/// 回本，所以中间那一档（普通人 `buy = 0.55 → 17.2` 倍）买得下它，而乖宝宝
/// （`0.30 → 11.2`）也买得下、只是补不成同色组就不买最贵的那几块。
pub fn leverage(x: f32) -> f32 {
    4.0 + 24.0 * x
}

/// 一块（或一级）产出的期望租金折成今天的钱，值不值那个价。
///
/// **不除价**：这一格比的是「绝对赚多少」而不是「回本比」，因为候选表上另一头永远是
/// 「什么都不做 ＝ 0 分」，两者同量纲才可比（除过价之后「一块便宜而没用的地」会盖过
/// 「一块贵而垄断的地」，那不是我要的判断）。
fn plot_value(rent_next: i64, price: i64, leverage: f32) -> f32 {
    rent_next as f32 * leverage - price as f32
}

/// 候选表：**「次优」这个词只有在一张表上才有定义**（PLAN §四・十五 的 P14b ③）。
///
/// 所以打分不产出「一个 bool」而产出这张表，最后一步才从表里取一条。取法：按分从高到低，
/// `fumble` 命中且表里还有第二名就交第二名。两条纪律住在类型上：
/// ① **被拒的候选不许 `add` 进来**（付不起、不够格、神挡着）—— 否则失误会变成「发一条
///    注定被拒的请求」，那不是手滑而是白扣一次动作，而且它在账上什么都没动，伪装得极好；
/// ② 并列时按**加入次序**定胜负（`sort_by` 是稳定排序），所以调用侧必须以确定的顺序 `add`
///    （格号、枚举序），不许依赖 `HashMap` 的迭代序 —— 同种子同轨迹这条就靠它。
#[derive(Debug, Default)]
pub struct Candidates {
    items: Vec<(f32, AiMove)>,
}

impl Candidates {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, score: f32, mv: AiMove) {
        self.items.push((score, mv));
    }

    /// 取走这一问的答案。表是空的＝这一格没给任何合法动作，调用侧要当「没答」处理。
    pub fn take(mut self, fumble: bool) -> Option<AiMove> {
        if self.items.is_empty() {
            return None;
        }
        self.items.sort_by(|a, b| b.0.total_cmp(&a.0));
        let pick = if fumble && self.items.len() > 1 { 1 } else { 0 };
        Some(self.items.swap_remove(pick).1)
    }
}

/// AI 的「一只手」：它读得着的**世界**与写得出去的那几种消息。
///
/// 打包成参数包是房规（§6.1 坑 8）：这条要写五种消息、读账本与棋盘，散装就是十几个参数，
/// 而 `#[allow(clippy::too_many_arguments)]` 会把「一个 system 要动这么多」这个信号吃掉。
///
/// 它也是**表现层那套脚手架的手**（`game_app/src/autopilot.rs`）—— 两边共用同一个
/// [`AiHand::emit`]，于是「答一句话到底要写几条消息」这份知识世界上只有一份。
/// 那份脚手架替的是**不在键盘前的人类座位**（读 [`greedy`] 那份表，不读先验），与这里替的
/// AI 座位由 [`Controller`] 分开，永远不会同一个人收到两份回答。
///
/// ## 为什么这里必须有 `Books`
///
/// 打分要读账本（这块地买得起吗、下一次收多少、名下押着几块），而**报价与成交必须问同一句**：
/// `decide` 里那句「买得起」读的就是 [`Books::buy_price`] —— 与 `decision_system` 成交时调的
/// 是同一条式子。P14a 之前这一格根本不存在（那份表只看规则表），也正是因此它永远只能
/// 「一律接受」。
///
/// ⚠️ **带了 `Books` 之后就不许再带这几样**（同一条 system 参数里两条对同一资源的可变借用
/// 会在参数建立期直接撞，坑 37 那一族；`Books` 内部字段清单见 `settle.rs:39-126`）：
/// `Res<Auction>`（走 `books.auction`）、`Query<&Wallet>`（走 `books.balance`）、
/// `Res<GameRules>`（走 `books.rules`）、`Res<GameSession>`、`Res<Corrections>`、
/// `Query<&Detention>`（走 `books.held`）、`Query<&StockHoldings>`（走 `books.holdings_of`）、
/// `Market` 与任何 `ResMut<Rng>`（后者归 [`AiPace`] 那一格）。
#[derive(SystemParam)]
pub struct AiHand<'w, 's> {
    pub books: Books<'w, 's>,
    pub pending: Res<'w, PendingDecision>,
    /// 上期开出的号：跟着上期买是真玩家会做的事，而这个选择不需要世界状态之外的东西
    pub jackpot: Res<'w, Jackpot>,
    pub controllers: Query<'w, 's, &'static Controller>,
    /// 这一回合办过的差事（柜台「一次访问只办一件」记在这儿）
    pub errands: ResMut<'w, AiErrands>,
    pub answers: MessageWriter<'w, DecisionChosen>,
    pub scores: MessageWriter<'w, MinigameResult>,
    pub tickets: MessageWriter<'w, BuyTicketRequested>,
    pub bids: MessageWriter<'w, AuctionMove>,
    pub passives: MessageWriter<'w, PassiveAnswer>,
    pub bank_ops: MessageWriter<'w, BankOpRequested>,
    pub mortgages: MessageWriter<'w, MortgageRequested>,
    pub trades: MessageWriter<'w, StockTradeRequested>,
    /// 行情表（只读）。**不借 `Market`**：那一条参数包里握着 `ResMut<Rng>`，与本模块
    /// 的 [`AiPace`] 是同一个资源的第二个主人（同种子同轨迹靠的正是「只有一处抽」）。
    pub stocks: Query<'w, 's, (&'static StockId, &'static StockPrice, &'static PriceHistory)>,
    /// 休市的两条来源（P2b 日历 / P11c② 停牌）：理财那一步问的与下单那一步是**同一句**
    /// [`market_halt_now`]，两边各拼一遍就会有一边读着旧世界。
    pub holidays: Res<'w, HolidaySet>,
    pub clock: Res<'w, MarketClock>,
}

impl AiHand<'_, '_> {
    /// 这个座位是不是电脑开的。**查不到组件算人类**（见 [`Controller`] 那条理由）。
    pub fn is_ai(&self, player: Entity) -> bool {
        self.controllers.get(player).is_ok_and(Controller::is_ai)
    }

    /// 这个座位领的是哪一份先验。`None` ＝ 不是电脑开的（人类，或漏挂组件）。
    ///
    /// 有了这一句，`is_ai` 与「读他的性格」就不是两次查询之间会漂移的事实：判据只有
    /// 「拿得到先验吗」这一条。
    pub fn profile_of(&self, player: Entity) -> Option<AiProfile> {
        self.controllers
            .get(player)
            .ok()
            .and_then(Controller::profile)
            .copied()
    }

    /// **下一笔要付的账** —— 「付完之后手里剩下的钱」的底线（PLAN §四・十五 的 P14b ②）。
    ///
    /// 三条凑出来的，全部是现成读数，所以它随物价与处境一起动，而不是一个要人盯着的常数：
    /// ① 场上别人名下**最贵的那一笔租金**（折过物价与我的修正）—— 下一掷踩中它就要付这个数，
    ///    这是「我可能付的账」里最大的那一格；② 我的贷款到下个月末的利息；③ 名下押着的地
    ///    的赎回款（付不起赎回就是丢地，而「丢地」比「少买一块地」贵得多）。
    ///
    /// 场上还没有任何人有地时它是 0 —— 那不是漏洞，是开局那一手的真账：谁都没地可踩，
    /// 留底本来就该是零，于是三个 AI 开局都买地（与真人一样）。
    fn reserve(&self, subject: Entity) -> i64 {
        let Some(me) = self.books.id_of(subject) else {
            return 0;
        };
        let views = self.books.views();
        let worst_rent = views
            .iter()
            .filter(|view| view.owner.is_some() && view.owner != Some(me))
            .map(|view| {
                let nominal = self.books.rent_of(view.id);
                self.books
                    .quoted(ChargeKind::Rent { land: view.id }, subject, nominal)
            })
            .max()
            .unwrap_or(0);
        let interest = economy::loan_interest(self.books.bank_of(subject).loan, &self.books.rules);
        let redeem = views
            .iter()
            .filter(|view| view.owner == Some(me) && view.mortgaged)
            .map(|view| self.books.redeem_cost_of(view.id).unwrap_or(0))
            .sum::<i64>();
        worst_rent + interest + redeem
    }

    /// **买下这块地之后**它一次收多少钱（假想我名下的那份视图喂给同一条租金式子）。
    ///
    /// 不另写一条租金函数：`economy::calculate_rent` 就是成交之后真会读的那一条，这里只是
    /// 把「这块地的 `owner` 换成我」这件事在**快照**上做一遍（`LandView` 本来就是为「判定
    /// 不去查世界」设计的，所以假想不需要同步任何状态，也不可能与成交分叉）。
    ///
    /// 补全同色组那一跳就在这一个数里：`owns_full_group` 读的是同一份假想视图，所以「差一块
    /// 就垄断」的地天然比别的地贵出 `monopoly_rent_multiplier` 倍 —— 不是一条特例，是式子本身。
    fn rent_if_mine(&self, subject: Entity, land: LandId) -> i64 {
        let Some(me) = self.books.id_of(subject) else {
            return 0;
        };
        let mut views = self.books.views();
        let Some(index) = views.iter().position(|view| view.id == land) else {
            return 0;
        };
        views[index].owner = Some(me);
        // 无主地立着的楼是破产留下的那种「无主而立着楼」，买下它连楼一起得到；
        // 而它若正押着（押着的地不会无主）就当作不押 —— 成交那一刻抵押是要赎回的
        views[index].mortgaged = false;
        let nominal = economy::calculate_rent(
            &self.books.active.def,
            &views[index],
            &views,
            &self.books.rules,
        );
        self.books
            .quoted(ChargeKind::Rent { land }, subject, nominal)
    }

    /// 这一问值不值：`Accept` 与 `Decline` 两条候选，闸门在打分之前。
    ///
    /// 两条闸门都不许写成「扣分」：`buy_price` 是 `None`（无主地没了、付不起、身上有衰神）
    /// 与「过了留底」都是**这一格不许做**，而不是「做了但分低」。把它们混进分数里，
    /// `fumble` 就会把一条注定被规则层拒掉的请求挑成第一名 —— 那正是 ③ 要挡的形状。
    fn buy_table(&self, subject: Entity, profile: &AiProfile, land: LandId) -> Candidates {
        let mut table = Candidates::new();
        if let Some(price) = self.books.buy_price(subject, land)
            && self.books.balance(subject) - price >= self.reserve(subject)
        {
            let rent = self.rent_if_mine(subject, land);
            table.add(
                plot_value(rent, price, leverage(profile.buy)),
                AiMove::Decide(Decision::Accept),
            );
        }
        table.add(0.0, AiMove::Decide(Decision::Decline));
        table
    }

    /// 加盖那一问：读的是**多出来的那一段租金**（不是加盖之后的总租金），
    /// 因为「不加盖」这一头收的是此刻那个数，两边同量纲才可比。
    fn upgrade_table(&self, subject: Entity, profile: &AiProfile, land: LandId) -> Candidates {
        let mut table = Candidates::new();
        if let Some((cost, uplift)) = self.upgrade_uplift(subject, land)
            && self.books.balance(subject) - cost >= self.reserve(subject)
        {
            table.add(
                plot_value(uplift, cost, leverage(profile.build)),
                AiMove::Decide(Decision::Accept),
            );
        }
        table.add(0.0, AiMove::Decide(Decision::Decline));
        table
    }

    /// 加盖要付多少、多收多少。`None` = 这一格不许加盖（不是我的地、抵押着、满级、
    /// 不是组内最低层、身上有衰神、付不起）—— 判据一条都不重写，读 `economy::can_upgrade`。
    fn upgrade_uplift(&self, subject: Entity, land: LandId) -> Option<(i64, i64)> {
        let view = self.books.view(land)?;
        let me = self.books.id_of(subject)?;
        if view.owner != Some(me) || self.books.acquire_denied(subject) {
            return None;
        }
        let base = economy::upgrade_cost(&self.books.active.def, &view)?;
        let cost = self
            .books
            .quoted(ChargeKind::Upgrade { land }, subject, base);
        let views = self.books.views();
        let cash = self.books.balance(subject);
        if !economy::can_upgrade(&view, me, &views, cost, cash, &self.books.rules) {
            return None;
        }
        let now = self
            .books
            .quoted(ChargeKind::Rent { land }, subject, self.books.rent_of(land));
        let mut after = views.clone();
        let index = after.iter().position(|v| v.id == land)?;
        after[index].level = view.level.saturating_add(1);
        let later = economy::calculate_rent(
            &self.books.active.def,
            &after[index],
            &after,
            &self.books.rules,
        );
        let later = self.books.quoted(ChargeKind::Rent { land }, subject, later);
        Some((cost, later - now))
    }

    /// 目标水位：他愿意让活钱低到哪儿。
    ///
    /// `留底 × (2 − debt)` —— **敢动身家的人可以揣着少一点活钱**（他随时押得动、借得动、
    /// 卖得动，不必自己当自己的银行），不敢动的人必须把那道账留在口袋里。方向与 `debt`
    /// 相反是这一格的全部要点：写成 `× (1 + debt)` 就造出一个「越敢借钱越怕缺钱」的人。
    fn target(&self, subject: Entity, profile: &AiProfile) -> i64 {
        (self.reserve(subject) as f32 * (2.0 - profile.debt)).floor() as i64
    }

    /// 银行此刻放不放款（新闻「银行停止放款」那一条，P11c②）。
    ///
    /// 只有 `Borrow` 读它：还款、存取款不受影响 —— 那条新闻说的是「不借新的」，
    /// 把还款一起挡掉等于凭空给欠债的人多记一笔逾期，而那句惩罚没有任何规则配得上它。
    fn lending_open(&self) -> bool {
        !self.books.session.suspensions.halted(Institution::Bank)
    }

    /// 押哪一块：缺口小就别押大地。先取「到手额 ≤ 缺口」里最大的那一块（正好补齐、不多押），
    /// 一块都嫌大时取最小的那块（少押一点是一点）。
    ///
    /// 两个 `if` 都用**严格**比较，于是并列时留下的是先遇到的，而 `counter_plots` 按 `LandId`
    /// 排过序 —— 同种子同轨迹这条就靠这一句，不靠 `>` 与 `>=` 的手感。
    fn pick_pawn(&self, subject: Entity, gap: i64) -> Option<(LandId, i64)> {
        let mut fits: Option<(LandId, i64)> = None;
        let mut least: Option<(LandId, i64)> = None;
        for (land, view) in counter_plots(&self.books, subject) {
            let facts = read_facts(&self.books, subject, land, true);
            if mortgage_op_allowed(MortgageOp::Take, &facts).is_err() {
                continue;
            }
            let raised =
                economy::mortgage_cashout(&self.books.active.def, &view, &self.books.rules);
            if raised <= 0 {
                continue;
            }
            if raised <= gap && fits.is_none_or(|(_, got)| raised > got) {
                fits = Some((land, raised));
            }
            if least.is_none_or(|(_, got)| raised < got) {
                least = Some((land, raised));
            }
        }
        fits.or(least)
    }

    /// 押着的那块地**赎回来之后**一次收多少 —— 与 [`Self::rent_if_mine`] 同一手：把「押着」
    /// 这一位在快照上抹掉，再喂给同一条租金式子。
    ///
    /// 直接读 `rent_of` 会得到 0（`calculate_rent` 里抵押那一格是短路成 0 的），于是赎回这一档
    /// 永远 0 分、永远上不了桌 —— 而它恰恰是「有余钱时该做的事」里最该被想到的那一件。
    /// 这一格是写完赎回之后回读才发现的：一条永远不会被选中的候选，与一条没写的候选
    /// 在画面上长得一模一样。
    fn rent_if_free(&self, subject: Entity, land: LandId) -> i64 {
        let mut views = self.books.views();
        let Some(index) = views.iter().position(|view| view.id == land) else {
            return 0;
        };
        views[index].mortgaged = false;
        let nominal = economy::calculate_rent(
            &self.books.active.def,
            &views[index],
            &views,
            &self.books.rules,
        );
        self.books
            .quoted(ChargeKind::Rent { land }, subject, nominal)
    }

    /// 柜台那一次访问的候选表。
    ///
    /// **分全部用同一把尺：这一档替我换回多少钱**（借到手的、还掉之后省下的利息、押到手
    /// 的、赎回之后收得回来的那一次租）。同一把尺才允许四档坐在同一张表上比，而
    /// 「什么都不做 ＝ 0 分」那条基准**先** `add` 进来 —— 于是任何平局都归「不办事」，
    /// 而 `add` 的次序就是 tie-break（P14b① 立的纪律）。
    ///
    /// 资格与额度是两件事，各由一个数管：肯不肯动身家是门槛 [`AiProfile::LIQUIDIZE`]
    /// （乖宝宝那一档就是照它定的），动哪一层、动几成是 `debt` 那一格。所以同一句话在
    /// 大佬奸嘴里是 2400、在普通人嘴里是 1200、在乖宝宝嘴里是「不办」。
    fn teller_table(&self, subject: Entity, profile: &AiProfile) -> Candidates {
        let mut table = Candidates::new();
        table.add(0.0, AiMove::Teller(TellerMove::Idle));
        // 一次访问只办一件：办过了，这一格剩下的就只有「办完」那一句
        if self.errands.banked == Some((subject, self.books.session.turn_count)) {
            return table;
        }
        let cash = self.books.balance(subject);
        let target = self.target(subject, profile);
        let gap = (target - cash).max(0);
        let surplus = (cash - target).max(0);
        let bank = self.books.bank_of(subject);

        if gap > 0 && profile.debt >= AiProfile::LIQUIDIZE {
            let room = self.books.max_loan(subject);
            let headroom = (room - bank.loan).max(0);
            let want = gap.min((headroom as f32 * profile.debt).floor() as i64);
            if want > 0
                && bank_op_allowed(BankOp::Borrow, false, true, self.lending_open())
                && economy::loan_plan(want, bank.loan, room).is_some()
            {
                table.add(want as f32, AiMove::Teller(TellerMove::Borrow(want)));
            }
            if let Some((land, raised)) = self.pick_pawn(subject, gap) {
                table.add(raised as f32, AiMove::Teller(TellerMove::Mortgage(land)));
            }
        }
        // 花钱的两档只在有余钱时上桌：拿缺口的钱去还债或修产，是拿今天的救命钱换明天的账
        if surplus > 0 {
            let want = surplus.min(bank.loan);
            if want > 0
                && bank_op_allowed(BankOp::Repay, false, true, self.lending_open())
                && economy::repay_plan(want, cash, bank.loan).is_some()
            {
                let saved = economy::loan_interest(want, &self.books.rules);
                if saved > 0 {
                    table.add(saved as f32, AiMove::Teller(TellerMove::Repay(want)));
                }
            }
            for (land, _) in counter_plots(&self.books, subject)
                .into_iter()
                .filter(|(_, view)| view.mortgaged)
            {
                let facts = read_facts(&self.books, subject, land, true);
                if mortgage_op_allowed(MortgageOp::Redeem, &facts).is_err() {
                    continue;
                }
                let back = self.rent_if_free(subject, land);
                if back > 0 {
                    table.add(back as f32, AiMove::Teller(TellerMove::Redeem(land)));
                }
            }
        }
        table
    }

    /// 场上还有没有**无主的地**。理财那一步读它：「有地可买的时候，钱是留着买地的」。
    ///
    /// 这一句比任何「现金超过多少才谈得上投资」的常数都稳：地价随棋盘走、物价指数随
    /// 全场资产走，而「还剩几块无主地」是规则层自己的读数。演示盘六块地卖完之后 AI 才开始
    /// 炒股，真棋盘上这自然把这件事推到中局 —— 「AI 炒股要尽量少」的一半靠它。
    fn land_left(&self) -> bool {
        self.books.views().iter().any(|view| view.owner.is_none())
    }

    /// 此刻开不开盘。与 `trade_request_system` 读的是**同一句**（[`market_halt_now`]）。
    fn market_open(&self) -> bool {
        market_halt_now(&self.holidays, &self.books.session, self.clock.tick).is_open()
    }

    /// 持仓按 `StockId` 排过序。**这一步不是洁癖**：`Books::holdings_of` 走的是 `HashMap`
    /// 的迭代序，而 std 的 `RandomState` 每个进程一份随机种子 —— 不排序的话「先挑到哪一只」
    /// 每次启动都不同，「同一颗种子同一条轨迹」当场破功（`Candidates` 那条 tie-break 是
    /// 同一条纪律，只是这里连并列都谈不上：顺序本身就是随机的）。
    fn holdings_sorted(&self, subject: Entity) -> Vec<(StockId, u32)> {
        let mut held = self.books.holdings_of(subject);
        held.sort_by(|a, b| a.0.0.cmp(&b.0.0));
        held
    }

    fn price_of(&self, id: &StockId) -> Option<f32> {
        self.stocks
            .iter()
            .find(|(stock, ..)| *stock == id)
            .map(|(_, price, _)| price.0)
    }

    /// 卖股补缺口：**一只股，卖够为止**。
    ///
    /// 挑法照 [`Self::pick_pawn`] 那一手（押哪块地）：先找「整笔卖得掉、且值 ≥ 缺口里最小的
    /// 那一笔」（别为一个小缺口把最大的持仓一次清掉），都比缺口小时卖最大那一笔（少卖一点
    /// 是一点）。两个比较都是严格的、而 `holdings_sorted` 已按 id 排过序，于是并列时留下的是
    /// id 靠前的那一只 —— 同种子同轨迹靠这一句，不靠 `HashMap` 的脾气。
    fn sell_plan(&self, subject: Entity, gap: i64) -> Option<TradeMove> {
        let mut fits: Option<(StockId, u32, i64, f32)> = None;
        let mut biggest: Option<(StockId, u32, i64, f32)> = None;
        for (id, shares) in self.holdings_sorted(subject) {
            let Some(price) = self.price_of(&id) else {
                continue;
            };
            if shares == 0 || price <= 0.0 {
                continue;
            }
            let value = (price * shares as f32) as i64;
            if value <= 0 {
                continue;
            }
            if value >= gap && fits.as_ref().is_none_or(|(_, _, best, _)| value < *best) {
                fits = Some((id.clone(), shares, value, price));
            }
            if biggest.as_ref().is_none_or(|(_, _, best, _)| value > *best) {
                biggest = Some((id, shares, value, price));
            }
        }
        let (stock, held, _, price) = fits.or(biggest)?;
        let need = ((gap as f32) / price).ceil().max(1.0) as u32;
        Some(TradeMove::Sell {
            stock,
            shares: need.min(held),
        })
    }

    /// 买股只用**闲钱**，而且只接跌下去的那一只。
    ///
    /// 现价 ÷ 历史均价最小者胜、并列按 id：这一格是「抄底」而不是「挑便宜的」—— 股息按股价
    /// 派生（`dividend_per_share`），贵贱与分红无关，涨跌才是买股赚得到的唯一那一笔。
    /// 没有历史时当 1.0（「没跌过」），而装配时历史里本来就塞了基准价，所以那只是防御。
    ///
    /// 跌得不够（[`AiProfile::STOCK_DIP`]）就什么都不买：有闲钱是一件事，见着有闲钱就下单
    /// 是另一件 —— 「尽量少」的第二半在这里。
    fn buy_plan(&self, spare: i64) -> Option<TradeMove> {
        let mut best: Option<(StockId, f32, f32)> = None;
        for (id, price, history) in self.stocks.iter() {
            let price = price.0;
            if price <= 0.0 {
                continue;
            }
            let mean = if history.0.is_empty() {
                0.0
            } else {
                history.0.iter().sum::<f32>() / history.0.len() as f32
            };
            let ratio = if mean > 0.0 { price / mean } else { 1.0 };
            let better = best.as_ref().is_none_or(|(best_id, _, best_ratio)| {
                ratio
                    .total_cmp(best_ratio)
                    .then_with(|| id.0.cmp(&best_id.0))
                    .is_lt()
            });
            if better {
                best = Some((id.clone(), price, ratio));
            }
        }
        let (stock, price, ratio) = best?;
        if ratio > AiProfile::STOCK_DIP {
            return None;
        }
        let shares = (spare as f32 / price).floor() as u32;
        if shares == 0 {
            return None;
        }
        Some(TradeMove::Buy { stock, shares })
    }

    /// 回合前理财那一笔：`None` ＝ 这一回合没得办（调用侧把手滑也归进这一格）。
    ///
    /// 三档判据自上而下：
    /// ① 活钱低于水位 → 卖股。卖股是「动身家」，所以资格读 [`AiProfile::LIQUIDIZE`]
    ///    （与押地借钱同一个门槛），**不读 `stock`** —— `stock` 说的是买不买；
    /// ② 闲钱多出「水位 + 一整笔留底」→ 且 `stock` 过门槛、且场上已没有无主的地 → 买一只
    ///    跌下去的股。多留一整笔留底是买与卖之间那道缝：贴着水位买，下一笔账就会把它逼回来
    ///    卖，同一只股一进一出只是白跑一趟；
    /// ③ 其余什么都不办。
    ///
    /// 取款不在这里（AI 的存款恒为 0，理由见模块头那条 ③）。
    pub fn treasury(&self, subject: Entity, profile: &AiProfile) -> Option<TradeMove> {
        if !self.market_open() {
            return None;
        }
        let cash = self.books.balance(subject);
        let target = self.target(subject, profile);
        if cash < target {
            if profile.debt < AiProfile::LIQUIDIZE {
                return None;
            }
            return self.sell_plan(subject, target - cash);
        }
        let spare = cash - target - self.reserve(subject);
        if spare > 0 && profile.stock >= AiProfile::STOCK_APPETITE && !self.land_left() {
            return self.buy_plan(spare);
        }
        None
    }

    /// 这一问的答案（会算账的那一份）。
    ///
    /// 只显式列出**已经会打分**的格子，其余一律落到 [`greedy`] 那份表上 —— 穷尽匹配留在
    /// `greedy` 里，所以新增一种窗口仍然会编译失败，而这里少写一臂只是「还没学会」。
    /// `fumble` 由调用侧掷好传进来（那条子流握在 [`AiPace`] 手里，与 `Books` 里的
    /// `ResMut<GameSession>` 是两条不同的可变借用）。
    pub fn decide(&self, subject: Entity, about: Pending, fumble: bool) -> AiMove {
        let Some(profile) = self.profile_of(subject) else {
            return greedy(about, &self.books.rules);
        };
        let table = match about {
            Pending::Buy(land) => self.buy_table(subject, &profile, land),
            Pending::Upgrade(land) => self.upgrade_table(subject, &profile, land),
            Pending::BankTeller => self.teller_table(subject, &profile),
            _ => return greedy(about, &self.books.rules),
        };
        table
            .take(fumble)
            .unwrap_or_else(|| greedy(about, &self.books.rules))
    }

    /// 把一条回答落成消息。**返回「这一问真的被答出去了没有」**——拍卖那一格可能因为
    /// 轮转指针与窗口不一致而什么都不发（不一致时不发比发一条注定被拒的请求好，
    /// 下一帧自然一致；理由写在下面那一格）。
    pub fn emit(&mut self, subject: Entity, mv: AiMove) -> bool {
        match mv {
            AiMove::Decide(action) => {
                self.answers.write(DecisionChosen {
                    player: subject,
                    action,
                });
                true
            }
            AiMove::Score { game, score } => {
                // 真对局不是练习：`awarded: false` 那条路只有主菜单那个游乐场走（P7 拍板）
                self.scores.write(MinigameResult {
                    player: subject,
                    game,
                    score,
                    awarded: true,
                });
                true
            }
            AiMove::Ticket => {
                let numbers = self.books.rules.lottery_numbers;
                // 夹进号码池：上期那个号在「改过号码池的设置」之后可能已经不存在了
                let number = self
                    .jackpot
                    .last_draw
                    .unwrap_or(1)
                    .clamp(1, numbers.max(1) as u8);
                self.tickets.write(BuyTicketRequested {
                    player: subject,
                    number,
                });
                // 买完还得有人关窗口：柜台不会因为一笔成交自己收摊（那是它与报价的区别）
                self.answers.write(DecisionChosen {
                    player: subject,
                    action: Decision::Acknowledge,
                });
                true
            }
            AiMove::Gate(moved) => {
                // 主语取窗口而不是门里的队列：`gate_system` 认的就是「这一问挂在他身上吗」，
                // 而这两处在同一帧里可能不一致（窗口刚被下一个承受者覆盖）。发错人那句会被
                // 读掉（「答了一句没有人在问的话」），不发就整片挂死——取窗口是更稳的那一侧
                self.passives.write(PassiveAnswer {
                    player: subject,
                    what: moved,
                });
                true
            }
            AiMove::Teller(what) => {
                // 业务与「办完」**不同帧**：办一件、下一帧再收摊。这两条 system 与
                // `decision_system` 之间没排过序（坑 36），而柜台那两条认的就是
                // 「窗口还开着吗」—— 同一帧发两样，业务会被自己的「办完」挡在门外。
                // P8 用 `chain` 解过同一道题（`lottery_op_system` 排在前），这里分成两帧，
                // 于是先后根本不出现在判据里。理由与那条循环防线一起写在 [`AiErrands`] 上。
                match what {
                    TellerMove::Idle => {
                        self.answers.write(DecisionChosen {
                            player: subject,
                            action: Decision::Acknowledge,
                        });
                    }
                    TellerMove::Borrow(amount) => {
                        self.bank_ops.write(BankOpRequested {
                            player: subject,
                            op: BankOp::Borrow,
                            amount,
                        });
                        self.errands.banked = Some((subject, self.books.session.turn_count));
                    }
                    TellerMove::Repay(amount) => {
                        self.bank_ops.write(BankOpRequested {
                            player: subject,
                            op: BankOp::Repay,
                            amount,
                        });
                        self.errands.banked = Some((subject, self.books.session.turn_count));
                    }
                    TellerMove::Mortgage(land) => {
                        self.mortgages.write(MortgageRequested {
                            player: subject,
                            op: MortgageOp::Take,
                            land,
                        });
                        self.errands.banked = Some((subject, self.books.session.turn_count));
                    }
                    TellerMove::Redeem(land) => {
                        self.mortgages.write(MortgageRequested {
                            player: subject,
                            op: MortgageOp::Redeem,
                            land,
                        });
                        self.errands.banked = Some((subject, self.books.session.turn_count));
                    }
                }
                true
            }
            AiMove::Trade(what) => {
                let (stock, side, shares) = match what {
                    TradeMove::Buy { stock, shares } => (stock, TradeSide::Buy, shares),
                    TradeMove::Sell { stock, shares } => (stock, TradeSide::Sell, shares),
                };
                self.trades.write(StockTradeRequested {
                    player: subject,
                    stock,
                    side,
                    shares,
                });
                // 与柜台那一格同一条：这一回合的理财到此用完（见 [`AiErrands::rebalanced`]）
                self.errands.rebalanced = Some((subject, self.books.session.turn_count));
                true
            }
            AiMove::Bid => {
                // **主语取机器而不是取窗口**：轮转指针与窗口由两条 system 写
                // （`auction_move_system` 改机器、`auction_hold_system` 挂窗口），而这两条之间
                // 没排过序 —— 读到「新机器 + 旧窗口」那一帧是可能的。不一致时这一帧什么都不发，
                // 下一帧自然一致：发一条注定被「还没轮到你」拒掉的请求，除了刷日志什么也不换。
                // 拍卖机读 `books.auction` 而不是另带一条 `Res<Auction>`（`Books` 里那条是
                // `ResMut`，同参数里并存会在建立期撞 —— 坑 37 那一族，`auction::AuctionDesk` 同例）
                let Some(lot) = self.books.auction.lot.as_ref() else {
                    return false;
                };
                if lot.bidder() != Some(subject) {
                    return false;
                }
                let needed = lot.next_amount();
                // 加得起就一直加：这样长跑里**一定**会有拍卖成交。反过来一律放弃也能让机器转完，
                // 但那一局里 P9 的过户与所得两条路径一次都没被走过
                let what = if self.books.balance(subject) >= needed {
                    BidMove::Raise
                } else {
                    BidMove::Withdraw
                };
                self.bids.write(AuctionMove {
                    player: subject,
                    what,
                });
                true
            }
        }
    }
}

/// AI 的那条钟：读得着的间隔、写得动的时刻、抽间隔用的那条子流。
///
/// 抽成参数包有两个理由，第二个是硬的：
/// ① §6.1 坑 8 那条房规 —— 「一道 system 要动这么多」是信号而不是麻烦，
/// 用 `#[allow(clippy::too_many_arguments)]` 把它吃掉等于把信号拆了；
/// ② 「什么时候许他动手」这条判据在世界上只该有一处 —— 掷骰那一条与答窗口那一条
/// 若各写一次门禁，改延迟时会漏掉一半（症状是「AI 一半的动作快一半的动作慢」）。
#[derive(SystemParam)]
pub struct AiPace<'w> {
    cadence: Res<'w, AiCadence>,
    clock: ResMut<'w, AiClock>,
    rng: ResMut<'w, Rng>,
    time: Res<'w, Time>,
}

impl AiPace<'_> {
    /// 现在许不许动手。
    fn held(&self) -> bool {
        self.time.elapsed_secs() < self.clock.ready_at
    }

    /// 这一动作允许了，把下一次的时刻推到 `gap` 之后。
    fn charge(&mut self) {
        let gap = self.cadence.gap(&mut self.rng);
        self.clock.ready_at = self.time.elapsed_secs() + gap;
    }

    /// 这一次决策算不算错（PLAN §四・十五 的 P14b ③）。
    ///
    /// 掷在**这一格**而不是 `decide` 里，因为那条子流握在 [`AiPace`] 手上而 `decide` 只拿得到
    /// 账本 —— 同一条 `ResMut<Rng>` 不许有第二个主人（那才是「同种子同轨迹」真正的守门人）。
    /// 每问掷一次、不问不掷：人类座位走的是 `greedy` 那条路，一次都不该消耗这条流。
    fn fumble(&mut self, profile: AiProfile) -> bool {
        self.rng.pick_ai() < profile.fumble
    }
}

/// AI 在 `WaitRoll` 里那一下要动的所有东西（与 [`AiPace`] 拼成一条 system）。
#[derive(SystemParam)]
pub struct AiTurn<'w, 's> {
    order: Res<'w, TurnOrder>,
    pending: Res<'w, PendingDecision>,
    controllers: Query<'w, 's, &'static Controller>,
    staying: Query<'w, 's, &'static StaySentence>,
    rolls: MessageWriter<'w, RequestRoll>,
    stays: MessageWriter<'w, RequestStay>,
}

/// AI 座位在 `WaitRoll` 里的那一下：**先理财、再掷骰**，两步不同帧。
///
/// 与真人点那颗按钮同一条意图：真人也是先能买卖证券、再按掷骰（`trade_request_system`
/// 只在 `WaitRoll` 受理）。第一步只在真有得办时发生（[`AiHand::treasury`] 返 `Some`），
/// 并且写一格 [`AiErrands::rebalanced`] —— 于是「每回合最多买卖一只股」是结构而不是约定。
///
/// 身上挂着 [`StaySentence`] 的人**发「停留」而不是掷骰** —— 少这一支是**死锁**而不是「少走一步」：
/// `roll_dice_system` 对被按住的人直接 `return`（那条判据在规则层，不认 UI），于是 `RequestRoll`
/// 每帧被写、每帧被吞，回合机停在 `WaitRoll` 不动也不报错。P4④ 当时把这一条写成「P14 的前置」，
/// 就是等这一刻。
///
/// 理财排在停留之前：被按住的人还在牌桌上（P4④ 的新口径），这一回合照样可以买卖证券，
/// 而「停留」只是他这一回合迈不开步。
pub fn ai_roll_system(mut pace: AiPace<'_>, mut turn: AiTurn<'_, '_>, mut hand: AiHand<'_, '_>) {
    let Some(current) = turn.order.current() else {
        return;
    };
    // 窗口开着的时候不许他插嘴：拍卖那一种会**换人问**，此刻被问的未必是他
    if turn.pending.active().is_some() {
        return;
    }
    if !turn.controllers.get(current).is_ok_and(Controller::is_ai) {
        return;
    }
    if pace.held() {
        return;
    }
    // P14b③ 回合前理财：掷骰之前先看一眼活钱。**这一帧只办这一件**，掷骰在下一帧 ——
    // 卖股的消息要与下单那条 system 同帧生效，而它和掷骰谁先被读没有保证（坑 36 那一族）；
    // 分成两帧之后先后根本不出现在判据里，与柜台那两帧同一手。
    let turn_no = hand.books.session.turn_count;
    if hand.errands.rebalanced != Some((current, turn_no))
        && let Some(profile) = hand.profile_of(current)
    {
        match hand.treasury(current, &profile) {
            // 手滑：候选表第二名就是「什么都不办」（这条表只有一件真事），于是这一回合的
            // 理财就这么忘了 —— 标记照写，免得下一帧再掷一次骰把它想回来
            Some(mv) if !pace.fumble(profile) => {
                pace.charge();
                hand.emit(current, AiMove::Trade(mv));
                return;
            }
            _ => hand.errands.rebalanced = Some((current, turn_no)),
        }
    }
    pace.charge();
    if let Ok(sentence) = turn.staying.get(current) {
        debug!(
            "AI 被停留按住（剩 {} 回合），发「停留」意图",
            sentence.turns_left
        );
        turn.stays.write(RequestStay { player: current });
        return;
    }
    // `dice: 0` = 按身上的载具摇（与真人那颗按钮同一个值）
    turn.rolls.write(RequestRoll {
        player: current,
        dice: 0,
    });
}

/// AI 座位在 `WaitDecision` 里的那一下：谁被挂着、被挂的是不是电脑开的座位。
///
/// 「是不是电脑开的」这一句问的是 [`AiHand::profile_of`] 拿不拿得到先验 —— 与 [`Controller`]
/// 是同一条事实的两种说法，所以这里不再有第二次查询，也不会有「两次读之间被人换了座位」那种
/// 帧相关的形状。
///
/// 人类座位那一侧由表现层的脚手架（`NM_AUTOROLL`）替不在键盘前的人按按钮，两侧靠
/// [`Controller`] 分开 —— **同一个人收到两份回答**的症状不是崩溃而是「买地买了两次」，
/// 那比崩难查得多，所以这道分界有专项用例（`tests/ai.rs`）。
pub fn ai_answer_system(mut pace: AiPace<'_>, mut hand: AiHand<'_, '_>) {
    let Some((subject, about)) = hand.pending.active() else {
        return;
    };
    let Some(profile) = hand.profile_of(subject) else {
        return;
    };
    if pace.held() {
        return;
    }
    pace.charge();
    let fumble = pace.fumble(profile);
    let mv = hand.decide(subject, about, fumble);
    if !hand.emit(subject, mv) {
        trace!("AI 这一问没答出去（轮转与窗口不一致），下一帧再问");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每一种窗口都要有答案，而三种「不认 `Decision`」的窗口必须答在它自己那一格里：
    /// 答成 `Acknowledge` 的话长跑会在第一个游乐场格、第一次破产、第一个被关的人那儿
    /// **永久停住且不报错**（画面还在动，只是没人行动）。这一条从表现层那份脚手架的
    /// 用例搬上来 —— 守「AI 不会把回合机挂死」的牙齿应该在规则层。
    #[test]
    fn every_window_has_an_answer_and_the_sticky_ones_are_not_decisions() {
        let rules = GameRules::default();
        for game in GameKind::ALL {
            assert!(
                matches!(
                    greedy(Pending::Minigame(game), &rules),
                    AiMove::Score { .. }
                ),
                "游乐场被答成了别的 —— 长跑会停在那一格"
            );
        }
        assert!(matches!(greedy(Pending::Auction, &rules), AiMove::Bid));
        assert!(matches!(
            greedy(Pending::Passive, &rules),
            AiMove::Gate(PassiveMove::Decline)
        ));
        assert!(matches!(greedy(Pending::Lottery, &rules), AiMove::Ticket));
        assert!(matches!(
            greedy(Pending::Buy(crate::land::LandId(0)), &rules),
            AiMove::Decide(Decision::Accept)
        ));
    }

    /// 游乐场那一格报的必须是**带分的**成绩：报 0 分等于长跑里「成绩换点券」那条路
    /// 一次都没被走过，而那是 P7 的验收项之一。
    #[test]
    fn the_arcade_answer_scores_half_of_perfect() {
        let rules = GameRules::default();
        for game in GameKind::ALL {
            let AiMove::Score { game: g, score } = greedy(Pending::Minigame(game), &rules) else {
                panic!("游乐场不该被答成别的形状");
            };
            assert_eq!(g, game);
            assert!(score > 0, "{} 报 0 分", game.label());
            assert_eq!(score, rules.minigames.of(game).perfect / 2);
        }
    }

    /// 候选表是「次优」这个词唯一的落脚点：命中失误交第二名。两条边界各钉一头 ——
    /// **只有一条时不许乱交**（拿越界当特色，那条 `swap_remove(1)` 会 panic），
    /// **并列时按加入次序分得出胜负**（同种子同轨迹这条靠它，而不是靠哈希的脾气）。
    #[test]
    fn a_fumble_takes_the_second_place_and_ties_break_by_insertion_order() {
        let mut table = Candidates::new();
        table.add(5.0, AiMove::Decide(Decision::Accept));
        table.add(1.0, AiMove::Decide(Decision::Decline));
        assert!(matches!(
            table.take(false),
            Some(AiMove::Decide(Decision::Accept))
        ));
        let mut table = Candidates::new();
        table.add(5.0, AiMove::Decide(Decision::Accept));
        table.add(1.0, AiMove::Decide(Decision::Decline));
        assert!(matches!(
            table.take(true),
            Some(AiMove::Decide(Decision::Decline))
        ));

        // 只有一条：失误没有第二名可取，交出去的仍是那一条
        let mut lone = Candidates::new();
        lone.add(0.0, AiMove::Decide(Decision::Decline));
        assert!(matches!(
            lone.take(true),
            Some(AiMove::Decide(Decision::Decline))
        ));

        // 同分：先加入的赢（稳定排序），所以调用侧的 `add` 次序就是判据的一部分
        let mut tie = Candidates::new();
        tie.add(0.0, AiMove::Decide(Decision::Accept));
        tie.add(0.0, AiMove::Decide(Decision::Decline));
        assert!(matches!(
            tie.take(false),
            Some(AiMove::Decide(Decision::Accept))
        ));
        let mut tie = Candidates::new();
        tie.add(0.0, AiMove::Decide(Decision::Accept));
        tie.add(0.0, AiMove::Decide(Decision::Decline));
        assert!(matches!(
            tie.take(true),
            Some(AiMove::Decide(Decision::Decline))
        ));
    }

    /// `leverage` 的两端都要有名字：`0` 那一端不是「不买」（下限仍是四次租金），
    /// `1` 那一端是「回本就买」。中段的三档预设落在什么位置，这条用例念一遍给人听。
    #[test]
    fn the_leverage_span_is_the_whole_personality_range() {
        assert_eq!(leverage(0.0), 4.0);
        assert_eq!(leverage(1.0), 28.0);
        let [shark, plain, goody] = AiProfile::THREE;
        assert!(
            leverage(shark.buy) > leverage(plain.buy) && leverage(plain.buy) > leverage(goody.buy)
        );
    }

    /// 打分的量纲必须与「什么都不做 ＝ 0 分」可比：收益 × 倍数 刚好抵掉代价时是 0，
    /// 而那一格正是候选表上 Decline 的分。除过价就没了这件事（一块便宜而没用的地会盖过
    /// 一块贵而垄断的地），所以这条钉的是**式子的形状**而不只是数。
    #[test]
    fn a_break_even_plot_scores_exactly_like_doing_nothing() {
        let rent = 180;
        let price = (rent as f32 * leverage(0.5)) as i64;
        assert!(plot_value(rent, price, leverage(0.5)).abs() < 1.0);
        // 同一次租金、贵一倍的价：分是负的，于是 Decline 赢
        assert!(plot_value(rent, price * 2, leverage(0.5)) < 0.0);
        // 补全同色组那一跳（垄断翻倍）能把同一块地从「不买」翻成「买」
        let doubled = rent * GameRules::default().monopoly_rent_multiplier;
        assert!(plot_value(doubled, price, leverage(0.5)) > 0.0);
    }

    /// 三档先验必须是**能读出差别**的三行读数：`harm` 单调、`fumble` 单调，
    /// 而「大佬奸很少犯错」与「大佬奸爱买股」是两件事（所以 `fumble` 不能与偏好同序）。
    #[test]
    fn the_three_presets_read_as_three_different_people() {
        let [shark, plain, goody] = AiProfile::THREE;
        assert_eq!(
            (shark.who, plain.who, goody.who),
            ("大佬奸", "普通人", "乖宝宝")
        );
        for (name, s, p, g) in [
            ("buy", shark.buy, plain.buy, goody.buy),
            ("debt", shark.debt, plain.debt, goody.debt),
            ("stock", shark.stock, plain.stock, goody.stock),
            ("build", shark.build, plain.build, goody.build),
            ("harm", shark.harm, plain.harm, goody.harm),
        ] {
            assert!(
                s > p && p > g,
                "{name} 那一格在三档预设之间读不出档次（{s} / {p} / {g}）"
            );
        }
        // 失误率与偏好反着走：越精的算盘越少算错
        assert!(shark.fumble < plain.fumble && plain.fumble < goody.fumble);
        // 乖宝宝那一档就是 §七 那条实测的归宿：缺钱也不动自己的资产
        assert!(
            goody.debt < 0.2,
            "变现那一格读 debt，乖宝宝必须低到不会去押地"
        );
    }

    /// 先验取值必须在 0..=1 内 —— 它是阈值与概率，不是钱。越界的数会被读成
    /// 「永远买」或「负失误率」（＝永不失误，但看起来像是配过了）。
    #[test]
    fn priors_are_unit_sized() {
        for profile in AiProfile::THREE {
            for v in [
                profile.buy,
                profile.debt,
                profile.stock,
                profile.build,
                profile.harm,
                profile.fumble,
            ] {
                assert!((0.0..=1.0).contains(&v), "{profile:?} 有一格不在 0..=1");
            }
        }
    }

    /// core 的默认延迟必须是 0。这条不是风格：无头测试里 `Time::elapsed_secs()` 不前进，
    /// 任何正数都会把 AI 钉死在第一帧不动 —— 症状与「AI 装配漏了」一模一样。
    #[test]
    fn the_headless_default_cadence_never_holds() {
        let cadence = AiCadence::default();
        let mut rng = Rng::new(7);
        assert_eq!(cadence.gap(&mut rng), 0.0);
        // 退化区间（`NM_AUTOGAP=<秒>`）不消耗那条子流：抽过一次就是把「AI 想了多久」
        // 接到了别人的随机数上
        let one = AiCadence { min: 0.0, max: 0.0 };
        assert_eq!(one.gap(&mut rng), 0.0);
    }

    /// 只有真正的区间才去抽那条子流，而同种子必须抽出同一个间隔（可复现）。
    #[test]
    fn a_real_gap_draws_the_ai_stream_and_replays() {
        let cadence = AiCadence { min: 0.5, max: 1.5 };
        let got: Vec<f32> = (0..4)
            .map(|_| {
                let mut rng = Rng::new(0x5EED);
                cadence.gap(&mut rng)
            })
            .collect();
        for gap in &got {
            assert!((0.5..1.5).contains(gap), "抽出的间隔 {gap} 不在区间里");
            // 同种子每次都同一个数（每次都新建一份 Rng，所以第一个数必然相同）
        }
        let mut shared = Rng::new(0x5EED);
        let a = cadence.gap(&mut shared);
        let b = cadence.gap(&mut shared);
        assert_ne!(a, b, "同一条流连着抽两次抽出同一个数");
    }
}
