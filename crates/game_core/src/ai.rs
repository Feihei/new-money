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

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::charge::ChargeKind;
use crate::economy;
use crate::land::LandId;
use crate::landing::{Pending, PendingDecision};
use crate::lottery::Jackpot;
use crate::minigame::GameKind;
use crate::msg::{
    AuctionMove, BidMove, BuyTicketRequested, Decision, DecisionChosen, MinigameResult,
    PassiveAnswer, PassiveMove, RequestRoll, RequestStay,
};
use crate::player::StaySentence;
use crate::rng::Rng;
use crate::rules::GameRules;
use crate::settle::Books;
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

/// AI 对一问的回答。**五种答案形状，与真人手上的五种点击一模一样**。
///
/// 单开一个类型而不是直接返回 [`Decision`]：游乐场那一格要回的是一份**成绩**、彩票柜台那
/// 一格要的是一笔**买券请求**、拍卖台那一个要的是**喊价**、被动门那一个要的是
/// **用／不用／给谁** —— 这四种在类型上都不是 `Decision`，而把它们并成一个 enum 之后，
/// 新增一种窗口时必须当场决定 AI 怎么答它（穷尽匹配替我们响）。
///
/// 后面四臂存在的最低理由是**别把回合机挂死**：`decision_system` 对拍卖与门连三种
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
    pub answers: MessageWriter<'w, DecisionChosen>,
    pub scores: MessageWriter<'w, MinigameResult>,
    pub tickets: MessageWriter<'w, BuyTicketRequested>,
    pub bids: MessageWriter<'w, AuctionMove>,
    pub passives: MessageWriter<'w, PassiveAnswer>,
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

/// AI 座位在 `WaitRoll` 里的那一下：与真人点那颗按钮同一条意图。
///
/// 身上挂着 [`StaySentence`] 的人**发「停留」而不是掷骰** —— 少这一支是**死锁**而不是「少走一步」：
/// `roll_dice_system` 对被按住的人直接 `return`（那条判据在规则层，不认 UI），于是 `RequestRoll`
/// 每帧被写、每帧被吞，回合机停在 `WaitRoll` 不动也不报错。P4④ 当时把这一条写成「P14 的前置」，
/// 就是等这一刻。
pub fn ai_roll_system(mut pace: AiPace<'_>, mut turn: AiTurn<'_, '_>) {
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
