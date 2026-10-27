//! 卡牌定义：机会 / 命运牌池。效果用内部标签枚举，新增 action 时编译器会强制补分支。
//!
//! 对应 Godot 端 `card_data.gd` + `card_effects.gd`（后者是 `Dictionary` 弱类型 match，
//! 且 `deduct_cash` 失败时只回文案不真的判破产 —— 见 `.agents/ARCHITECTURE.md` §三 第 3 条）。

use bevy::asset::Asset;
use bevy::reflect::TypePath;
use serde::Deserialize;

/// 三副池。前两副是**踩到格子就抽、当场结算**的事件池（PLAN §三「命运 32 条」），
/// 第三副 `Item` 是**拿在手里、由玩家主动打出**的卡 —— 它没有对应的格子，
/// 来源是百货公司（P3d），所以 `domain_of` 里永远不会出现 `Card(Item)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Chance,
    Fate,
    Item,
}

/// 这张卡**不是动词**：它被拿在手里就是答案，问它的那一刻由事件带来。
///
/// P13b 拍的那条「被动卡这一族从此没有『出牌』这个动作」（PLAN §五 第 6 条）在数据上的
/// 落点就是这一位。为什么另开一位而不是让它挂在 [`CardEffect`] 里（① 之前免罪卡写的正是
/// 一条 `insulate` 效果）：一条效果的含义是「打出去会发生什么」，而这三张卡打不出去 ——
/// 把它们留在动词库里，嫁祸卡就得挂一条**永远不会被执行**的效果才能进池
/// （那正是 PLAN §五 第 1 条点名的「表上有、路上没有」那种假答案）。
///
/// 所以这一位是「门该翻谁的手牌」的唯一读数：拿着 = 有资格被问，花掉 = 从手里移除。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PassiveCard {
    /// 免罪卡：挡掉一次「把人关起来」的事（哪些事件吃它见
    /// [`PassiveEvent::offset_card`](crate::passive::PassiveEvent::offset_card)）
    Forgive,
    /// 免费卡：把一笔大额账整笔免掉（门槛见 [`crate::rules`]，与嫁祸卡是同一个数）
    FreePass,
    /// 嫁祸卡：把这一件事／这一笔账转给**点名**的另一个人（10-27 拍板：点名，不是随机）
    Shift,
}

impl PassiveCard {
    /// 人话：这一张在屏幕上叫什么。门问那一句、面板那颗按钮、顶栏那条流水说的是同一张卡，
    /// 所以映射只写一次 —— 三处各念一遍就会出现「按钮写『免罪卡』而顶栏写『护身卡』」。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Forgive => "免罪卡",
            Self::FreePass => "免费卡",
            Self::Shift => "嫁祸卡",
        }
    }
}

/// 这张卡要落在谁/什么头上。写在数据里而不是代码里，是因为出牌浮层的按钮要把目标
/// 编码进去（同 ATM 金额按钮的形状：一颗按钮 = 一个完整意图），而浮层不该去猜
/// 「这张卡是给自己用还是给对手用」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardTarget {
    /// 打在自己身上（免罪、停留、转向）
    #[default]
    SelfOnly,
    /// 选一名对手（陷害卡）。
    ///
    /// ⚠️ 这一格曾经写着「陷害、嫁祸」—— P13b 把嫁祸卡改成被动卡之后它**不在这里了**：
    /// 嫁祸的那一次点名发生在事件落下来的那一刻（门里），不在出牌请求里，
    /// 所以它没有 `aim`、也就不占这一格。10-27 之前那版「获得那一刻点名」的候选
    /// 换掉的就是这一格（PLAN §七 那一行的 ②）。
    Player,
    /// 选一个人，**含自己**（转向、停留，P3d④b/④c）。
    ///
    /// 为什么不并进 [`Self::Player`]：「不能陷害自己」是陷害卡的规则（自己送自己进监狱
    /// 没有意义，而那条判据写在 `aim_is_legal` 里），而掉头与停在原地这两件事
    /// **正是**要能对自己做 —— 4 代的这两张卡点谁都可以（`GLOSSARY-board.md` 那句
    /// 「反转绕行方向 / 给他人转向」）。并成一格的话，下一张「可以打给自己」的卡
    /// 就会去读 `effects` 才知道自己能不能被选。
    ///
    /// P13 魔法屋那 12 条法术按筛选条件点人（走路的人／存款最多／所有女生…），
    /// 挑的也是这一格：那件事的主语永远是「一个人」，而不是「一个对手」。
    Anybody,
    /// 选一块**有主**的地（涨价、查封、怪兽）
    Land,
    /// 选一块**无主**的地（购地卡，P3d④）。
    ///
    /// 为什么不并进 [`Self::Land`]：这两句话的承受者不是同一个人。`Land` 的承受者是那块地的
    /// **业主**（涨价是往别人钱袋上加钱），而无主地根本没有业主可承受 —— 购地卡的承受者就是
    /// 出牌人自己。并成一格的话，[`crate::hand::victim_of`] 与
    /// [`crate::hand::aim_is_legal`] 就得反过来读 `effects` 才知道该问谁，
    /// 「这张卡打给谁」这件事从此散在两张表里。
    UnownedLand,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CardEffect {
    AddCash {
        amount: i64,
    },
    DeductCash {
        amount: i64,
    },
    /// 把承受者送进某一处「不在牌桌上」的地方 N 天（P4）。**承受者是这张卡的目标**
    /// （[`CardTarget`]），不是抽卡的人 —— 所以命运牌的「直接入狱」（target 缺省 = 自己）
    /// 与陷害卡的「送对手入狱」是同一条效果，不必为陷害单开一个变体。
    /// 落在表上哪一行、能不能被手里那张免罪卡挡掉，由 [`crate::passive`] 那张表答
    /// （行按去处数，见 [`crate::afflict::Affliction::detain`]）。
    ///
    /// `place` 是**必填**的：坐牢、住院、出国在三条数据上就是三个数，而「不给 place 默认成
    /// 监狱」会让「掉水沟」这类卡静默变成坐牢 —— 那正是最难查的一类错（文案说住院、
    /// 模型把人送进监狱）。三种去处共用这一条效果的理由见 [`crate::player::DetentionPlace`]。
    Detain {
        place: crate::player::DetentionPlace,
        #[serde(default = "default_detention_turns")]
        turns: u32,
    },
    /// 按格名瞬移。格名是数据与棋盘的隐式契约，`validate` 只能查非空，
    /// 真正「存不存在」要等 B3 拿到 BoardDef 才能校验。
    MoveTo {
        tile_name: String,
    },
    /// 给一片地块盖上限时状态（涨价卡 / 查封卡，P3c）。
    ///
    /// 三个字段缺一不可：`status` 是盖哪一种，`area` 是盖多大一片（4 代这两张都是「整街」，
    /// 但单块与整街是两个不同的数，写成一个变体就得为后者再开一个变体），`days` 是活几天。
    /// 承受者是**这块地的业主**（[`crate::hand::victim_of`]），不是选它的人 —— 所以
    /// 「查封对手整条街」与「涨价自己的街」共用同一条实现，差别只在数据。
    LandStatus {
        status: crate::status::LandStatus,
        days: u32,
        area: crate::plot::PlotArea,
    },
    /// 打一片建筑损害（P11b①：外星人与山洪；4 代那两张拆房卡将来也走这一格）。
    ///
    /// 它带的不是「扣钱」那一族的任何一条：拆的是**街上那几层楼**，而夺不夺地是
    /// [`crate::damage::LandDamage`] 那两支自己说的（`Degrade` 结构上没有 `seize` 那一格）。
    /// 与 [`Self::LandStatus`] 同一条形状：**中心来自 aim，不在卡面/条目上**，
    /// 范围是动词自带的那一格 —— 于是「外星人打哪条街」由作用域回答（新闻是播报者
    /// 脚下那块地），「打多大一片」由数据回答。两处各留一份就会分叉。
    ///
    /// 它**不进结算管线**：这里没有一笔钱过手（P2 那条链管的是钱），所以免罪卡与免费卡
    /// 都挡不住它 —— 与 `LandStatus` 同一条口径，而 PLAN §三 的免疫矩阵将来要分头问这两族。
    Damage {
        depth: crate::damage::LandDamage,
        area: crate::plot::PlotArea,
    },
    /// 按一条基数抽一成税（P11b② 的新闻：地价税、所得税）。
    ///
    /// 它与 [`Self::DeductCash`] 的分别是**金额从哪来**：那一条写在条目上，这一条从世界里现读
    /// （他今天手上有多少钱、名下那些地今天值多少）。把「按 5% 抽」写成「抽 3000」的话，
    /// 地主与穷光蛋交一样的钱 —— 而那正是这两条新闻要的区分。
    ///
    /// 它照样进结算管线（[`crate::charge::ChargeKind::Levy`]）：归 `Penalty` 域，所以免费卡
    /// 免得掉、嫁祸卡转得出去；只有物价指数那一格填「不缩」，理由见 [`LevyBasis`]。
    Levy {
        basis: LevyBasis,
        percent: u32,
    },
    /// 把某一处里的人放出来（P11b② 的新闻「医院传出好消息」）。
    ///
    /// `place` 是**必填**的，而作用域那一档也已经按处筛了人（[`crate::news::NewsScope`] 的
    /// `EveryoneIf(Detained)`）—— 两处各写一半就会分叉成「筛的是医院、放的是监狱」，
    /// 所以那条一致性由 `news::mismatch` 在加载期拦。
    ///
    /// 它走的是与**期满同一条出口**：同一条 [`crate::msg::DetentionLifted`] 凭据、同一个
    /// 「摘组件 + 带接入点」的动作。表现层因此不必认识「新闻放人」这第三种走法，
    /// 而那条死胡同格上的棋子该回哪一格也只需要有一份答案。
    Release {
        place: crate::player::DetentionPlace,
    },
    /// 当场买下一块无主地（购地卡，P3d④）。**卡面上不带地块编号**：选哪块地是玩家按下去
    /// 那一刻定的，来自出牌请求的 `aim`（与 [`Self::LandStatus`] 同一条理由）。
    ///
    /// 价格不在卡面上，也不必在卡面上：走的是 `Books::buy` 那条正路，所以地价过一遍
    /// P2 管线 —— 物价指数照乘、小福神「买地免费」那一类 `Cancel` 照免。这张卡买的不是折扣，
    /// 买的是**不用走到那一格**这件事。
    BuyLand,
    /// 让承受者的绕行方向掉头（转向卡，P3d④b）。**没有字段**：翻的是玩家身上那一位
    /// （[`crate::player::Heading`]），而「翻成什么」不是卡面能说的 —— 翻两次就是原样，
    /// 所以这里既没有 `to` 也没有层数。
    ///
    /// 环还是那一条环，只是步进取 `+1` 还是 `-1`（ADR-001 行为规则 3 + `GLOSSARY-board.md`
    /// 「仍沿同一环，只是反向」）。它**不吃免罪卡**（P13 定稿 10-25：掉了个头是他这一趟的
    /// 处境变了，不是一件灾落在他头上 —— 读数在
    /// [`PassiveEvent::Hold`](crate::passive::PassiveEvent::Hold)，而「不可免者不可转」
    /// 那条蕴含顺手把它从嫁祸那一侧也划掉了）；也因此与 `LandStatus`/`BuyLand` 不同：
    /// 没有 aim 它照样成立（打给自己），所以 `validate` 不给它绑 target。
    Reverse,
    /// 让承受者原地停留 N 回合（停留卡）。**人还在场上**：棋子不动、不改方向、
    /// 名下的地照样收租；P4④ 起也不自动跳回合 —— 轮到他的那颗「掷骰」翻成「停留」，
    /// 点一下才消耗这一回合（见 [`crate::player::StaySentence`]）。
    ///
    /// 与 [`Self::Detain`] 是两条效果而不是同一条带个 bool 的：入狱要把人送进
    /// 死胡同格并（P4② 起）夺走收租权，这两件事停留都不做。它们共享的只是「少走几回合」
    /// 这半条规则，而那半条现在各有各的入口（拘留自动跳过、停留等人按），
    /// 所以更不该由卡面来合并。
    Stay {
        turns: u32,
    },
    /// 让承受者「走不远」N 个自己的掷骰回合：骰子照摇、点数照记，实际只走 1 格
    /// （乌龟卡，P5）。
    ///
    /// 与 [`Self::Stay`] 分成两条：停留是**不动**（吃掉整个回合），少动是**还在动**。
    /// 四代那张 3 回合的停留卡身上混着的正是这条行为，10-20 拍板把它们拆开。
    MoveCap {
        turns: u32,
    },
    /// 请神符（P6d）：把路上**离自己最近**那一位请到自己身上。
    ///
    /// **没有字段，也不进 `CardAim`** —— 选哪位不是玩家点的，是那张表算的
    /// （[`nearest_god`](crate::gods::nearest_god)）。这不是省事，是这张卡的玩法本身：
    /// 可以挑神的话它就退化成一张廉价的「指定大财神」，而 4 代那句「离自己最近」要的就是
    /// 「你请到的可能是恶魔」。它请的也不限于好神、更不限于自己前面那位 ——
    /// 后面那位才是这张卡值钱的地方（前面那位下一回合本来就会踩到）。
    ///
    /// 请过来走的是与「踩到」**同一个** [`attach`](crate::gods::attach) 出口，所以那一次性的
    /// 一笔（大财神那四位数、大穷神抢的那 1/4、福神那两张卡）**照做**：请来的人与踩到的人
    /// 是同一个人，只是来的路不同。
    SummonGod,
    /// 送神符（P6d）：送走自己身上那位，并**连带**送走身上带着的定时炸弹。
    ///
    /// 承受者只能是自己（`target` 缺省 `SelfOnly`）：这张卡的主语是「（我身上）那位神」。
    /// 死神送不走（[`banishable`](crate::gods::banishable)），而那条判据与「身上没有神」
    /// 都在出牌前挡掉 —— 卡不消耗，理由给面板当灰按钮的文案。
    BanishGod,
    /// 把一块地强制上架（拍卖卡 P9 / 新闻「公开拍卖公有土地」P11c①）。
    ///
    /// 卡面上既没有地块编号也没有价钱：哪块地来自出牌请求（与 [`Self::BuyLand`] 同一条），
    /// 而起拍价由 [`crate::settle::Books::market_value`] 现算 —— 那块地此刻值多少，
    /// 含它上面盖着的每一层楼。
    ///
    /// 这条效果与其余每一条的差别是**主语与承受者不是同一个人**：出牌人是开槌的那位
    /// （他不参与竞价，拍出的钱归他 —— 10-22 拍板），承受者是那块地的业主（他会失去这块地，
    /// 而拿不到那笔钱）。[`crate::hand::victim_of`] 给的是后者，所以
    /// [`crate::deck::apply_card`] 为此多带一个「出牌人」参数 —— 那不是冗余，是这张卡的
    /// 语义本来就有两个人。
    ///
    /// `area` 是 P11c① 加的那一格，而**它不是给拍卖卡用的**（数据里没有，走
    /// [`crate::plot::PlotArea::default`] = 只打挑中那一块）。加它的原因与「拆房」那条同一条：
    /// 新闻要的从来不是「一块」，而是「这一带的那几块」，而**范围是地块的拓扑性质**，
    /// 该由数据选而不是由调用方猜。中心从哪来见 [`crate::news::NewsScope`]。
    ///
    /// ⚠️ 同一条 `area` 在两条来路上筛出的集合不同：卡拍的是**有主**的地（无主地本来谁都能买，
    /// 绕一台拍卖机只是多一步），而新闻那条拍的是**无主**的公有地（✅ 10-24 拍板：所得归银行，
    /// 且这笔钱不进任何公司的当月损益账）。那一句差别住在 [`crate::auction::Gavel`] 里，
    /// 不由数据重复一遍 —— 两条来路各答一次「谁挨这一槌」，数据就少一个说谎的位置。
    Auction {
        #[serde(default)]
        area: crate::plot::PlotArea,
    },
    /// 让一家机构停摆 N 天（新闻「股市临时停牌」「银行停止放款」，P11c②）。
    ///
    /// 它是这条动词库里**唯一不打在人也不打在地上的**一条：改的是「这几天这条规矩还作不作数」。
    /// 因此它今天**只能由新闻中心播出来**，不是一张卡 —— [`CardSet::validate`] 那一条把它
    /// 挡在卡池外（要哪一家由条目自带的 `what` 说，而卡没有「某类机构」这一档目标：
    /// 给卡加那一格，面板就要长出一排「点哪家」的按钮，而那不是出牌是批公文）。
    ///
    /// 停的是**哪项能力**由 [`crate::institution::Institution`] 自己说：`Bank` 那一格停的
    /// 是放款，存取款与还款照办（「停止放款」的本意是「不借新的」，不是「不许还钱」）。
    Suspend {
        what: crate::institution::Institution,
        days: u32,
    },
}

/// 一条税按什么收（[`CardEffect::Levy`] 的那一格）。两格的共同点是**读的都是今天的数**，
/// 所以 `ChargeKind::Levy` 在「缩不缩物价」那一格填「不缩」：基数已经是今天的价格，
/// 再乘一次指数等于替税率偷偷涨价（物价 2 倍时「10% 所得税」会变成 20%）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LevyBasis {
    /// 手上现金（所得税）。存款与贷款**不算**：那是另一套账（`BankBalance`），
    /// 4 代那条新闻说的也是「现金的百分之几」。
    Cash,
    /// 名下地块的现值总和，含每一层楼（地价税）。读的是
    /// [`crate::settle::Books::market_value`] —— 与拍卖起拍同一个表达式，
    /// 于是「这块地今天值多少」在世界上只有一份答案。
    LandValue,
}

const fn default_detention_turns() -> u32 {
    3
}
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CardDef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: CardKind,
    pub title: String,
    pub desc: String,
    /// 百货公司的售价（点券）。`None` = 这张卡不出售（两副事件池全是 `None`，
    /// 它们是被抽到的，不是被买的）。点券与现金永不通兑（PLAN §一），所以这条
    /// 只可能由 `Points` 付。
    #[serde(default)]
    pub cost_points: Option<u32>,
    /// 打出去要选谁/选哪块地。缺省 `SelfOnly`。
    #[serde(default)]
    pub target: CardTarget,
    /// 这张卡是**被动卡**吗（P13b）。`Some` = 它没有「出牌」这个动作：躺在手里就是资格，
    /// 事件落到某人头上的那一刻由 [`crate::passive`] 那道门问他要不要用。
    ///
    /// 与 `effects` 的分别是**谁决定什么时候生效**：`effects` 是打出去那一刻执行完的动词，
    /// 而这一位压根不进入 [`crate::deck::apply_effects`]。一条卡若两边都写，
    /// 「按下去会发生什么」与「被问到时能答什么」就会各说一句话，所以
    /// [`CardSet::validate`] 直接拒这种写法。
    #[serde(default)]
    pub passive: Option<PassiveCard>,
    #[serde(default)]
    pub effects: Vec<CardEffect>,
}

#[derive(Debug, Clone, PartialEq, Asset, TypePath, Deserialize)]
#[serde(transparent)]
pub struct CardSet(pub Vec<CardDef>);

impl CardSet {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// `expect_kind` 让「把机会卡写进命运文件」这类数据错误在加载期就暴露。
    pub fn validate(&self, expect_kind: CardKind) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();

        if self.0.is_empty() {
            return Err(vec!["牌池为空".into()]);
        }

        let mut seen = std::collections::HashSet::new();
        for c in &self.0 {
            if c.id.is_empty() {
                errs.push("有卡牌 id 为空".into());
            } else if !seen.insert(c.id.as_str()) {
                errs.push(format!("卡牌 id 重复: {}", c.id));
            }
            if c.kind != expect_kind {
                errs.push(format!(
                    "卡牌 {} 的 type={:?} 与牌池 {:?} 不符",
                    c.id, c.kind, expect_kind
                ));
            }
            if c.title.is_empty() {
                errs.push(format!("卡牌 {} 缺 title", c.id));
            }
            if c.desc.is_empty() {
                errs.push(format!("卡牌 {} 缺 desc", c.id));
            }
            // 一条卡要么有动词（打出去执行完），要么是一张被动卡（被门问到才算数），
            // 两边都写 = 「按下去会发生什么」与「被问到能答什么」各有一份答案，
            // 而两边都空 = 货架上一件按下去什么都不做的事
            let id = c.id.as_str();
            match (c.passive, c.effects.is_empty()) {
                (Some(card), false) => errs.push(format!(
                    "卡牌 {id} 是被动卡（{card:?}）却带效果：被动卡没有『打出去』那一刻"
                )),
                (None, true) => errs.push(format!("卡牌 {id} 既没有任何效果，也不是被动卡")),
                (Some(_), true) | (None, false) => {}
            }
            // 货架上的东西必须有价：没有价的 `Item` 卡要么白送要么永远买不到，
            // 两种都是把数据错误藏进「点数账对不上」里（P3d 的百货正靠这一列定价）
            if expect_kind == CardKind::Item && c.cost_points.unwrap_or(0) == 0 {
                errs.push(format!("卡牌 {} 是货架商品却没有 cost_points", c.id));
            }
            for e in &c.effects {
                match e {
                    CardEffect::AddCash { amount } | CardEffect::DeductCash { amount } => {
                        if *amount <= 0 {
                            errs.push(format!("卡牌 {} 的现金效果数值非法: {amount}", c.id));
                        }
                    }
                    CardEffect::Detain { turns, .. } => {
                        if *turns == 0 {
                            errs.push(format!("卡牌 {} 的关押回合数为 0", c.id));
                        }
                    }
                    CardEffect::MoveTo { tile_name } => {
                        if tile_name.is_empty() {
                            errs.push(format!("卡牌 {} 的 move_to 缺 tile_name", c.id));
                        }
                    }
                    CardEffect::LandStatus { days, .. } => {
                        // 0 天的状态会在面板上挂满一整天再消失（「剩 0 天」），而它的本意是
                        // 「什么也没发生」。数据里写 0 就是写错了，加载期拦下
                        if *days == 0 {
                            errs.push(format!("卡牌 {} 的地块状态天数是 0", c.id));
                        }
                        // 这种效果必须靠目标才知道盖哪儿：锚点来自出牌请求，不在卡面上。
                        // 写成 `target: self_only` 的机会牌会一路走到 `apply_card` 才落空，
                        // 那时候它已经是一张「买了没用的卡」了 —— 在加载期就拦
                        if c.target != CardTarget::Land {
                            errs.push(format!("卡牌 {} 要盖地块状态，但 target 不是 land", c.id));
                        }
                    }
                    CardEffect::Damage { .. } => {
                        // 与 `LandStatus` 同一条绑定：这一拳打在哪一片要有个中心，而中心
                        // 不在卡面上（卡面上有中心的话，一张卡就得为每条街写一遍）
                        if c.target != CardTarget::Land {
                            errs.push(format!("卡牌 {} 要打建筑损害，但 target 不是 land", c.id));
                        }
                    }
                    CardEffect::Levy { percent, .. } => {
                        // 0% 是一条不收钱的新闻（而它照样会播出一条标题、留下一条 0 元流水），
                        // >100% 是「抽的比基数还多」—— 那已经不是税而是另一条 `DeductCash`，
                        // 该写成定额而不是百分比。两格都在加载期拦
                        if *percent == 0 || *percent > 100 {
                            errs.push(format!("卡牌 {} 的税率是 {percent}%，要写 1..=100", c.id));
                        }
                    }
                    CardEffect::Release { .. } => {
                        // 这一格今天**没有卡会用**（新闻专用）。写出来是为了那条穷尽表不许有 `_`：
                        // 哪天有人做一张「交钱赎人」的卡，它要在这里回答「绑哪个 target」——
                        // 而那一问有真答案（放人的受益者是被关的那位，不是掏钱的另一位，
                        // 与保释同一条分叉），静默走 `self_only` 会做出「自己放自己」那张卡
                    }
                    CardEffect::BuyLand => {
                        // 与 `LandStatus` 同一条绑定：这块地来自出牌请求，不在卡面上。
                        // 目标写成 `land` 的话，购地卡会去问一块**有主**的地（那条判据要业主），
                        // 而它要买的恰恰是无主的 —— 两个判据互相矛盾，抽到/打出都只会静默失败
                        if c.target != CardTarget::UnownedLand {
                            errs.push(format!("卡牌 {} 要买地，但 target 不是 unowned_land", c.id));
                        }
                    }
                    CardEffect::Reverse => {
                        // 这一格**没有**要校验的数，而它必须写在这里：`Reverse` 翻的是玩家身上
                        // 那一位，没有字段、也没有「没有 aim 就静默失效」那条毛病（打给自己
                        // 照样成立），所以它不该被绑到任何 target 上。哪天它长出参数来
                        // （比如「转向 N 次」），编译会停在这一格
                    }
                    CardEffect::Stay { turns } => {
                        // 与 `GoToJail` 同一条：0 回合的停留是「什么都没发生」，而卡已经被打掉
                        if *turns == 0 {
                            errs.push(format!("卡牌 {} 的停留回合数为 0", c.id));
                        }
                    }
                    CardEffect::MoveCap { turns } => {
                        // 同一条：0 回合的「走不远」什么都不做，而卡已经被打掉
                        if *turns == 0 {
                            errs.push(format!("卡牌 {} 的乌龟回合数为 0", c.id));
                        }
                    }
                    CardEffect::Auction { .. } => {
                        // 与 `LandStatus`／`BuyLand` 同一条绑定：要拍哪块地来自出牌请求，不在卡面上。
                        // 绑成 `land`（有主地）而不是 `unowned_land`，是因为这张卡卖的就是
                        // 「别人手里那块地被迫上拍」—— 无主地本来谁都能买，拍它只是多绕一台机器
                        if c.target != CardTarget::Land {
                            errs.push(format!(
                                "卡牌 {} 要拍卖地，但 target 不是 land（得有原主）",
                                c.id
                            ));
                        }
                    }
                    CardEffect::SummonGod | CardEffect::BanishGod => {
                        // 两张符都**没有** aim：请哪位由 `gods::nearest_god` 算，送的是自己身上
                        // 那位。这里绑成「必须是 `self_only`」而不是干脆不校验 —— 数据若写成
                        // `player`，面板会长出一排点人的按钮，而点谁都不改变办成的那件事：
                        // 那是一排骗人的按钮。与 `LandStatus`／`BuyLand` 那两格同一条纪律，
                        // 只是方向相反（那两张**必须**有 target，这两张**不能有**）
                        if c.target != CardTarget::SelfOnly {
                            errs.push(format!(
                                "卡牌 {} 是神明符，它自己找对象，不该带 target",
                                c.id
                            ));
                        }
                    }
                    CardEffect::Suspend { .. } => {
                        // **卡根本不该带这条效果**：停摆是公文不是交易，卡池里没有「哪一家机构」
                        // 这一档目标。放进卡池的下一步必然是给 `CardTarget` 加一格、给面板加一排
                        // 「点哪家」的按钮 —— 那条路今天不铺。天数那一问住在新闻侧
                        // （`NewsSet::validate`），因为只有它收这条效果
                        errs.push(format!(
                            "卡牌 {} 要停摆机构：那只能由新闻中心播出来，卡没有「某类机构」这一档目标",
                            c.id
                        ));
                    }
                }
            }
        }

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANCE: &str = include_str!("../../../assets/data/cards/chance_cards.json");
    const FATE: &str = include_str!("../../../assets/data/cards/fate_cards.json");
    const ITEMS: &str = include_str!("../../../assets/data/cards/item_cards.json");

    #[test]
    fn both_pools_parse_and_validate() {
        let chance = CardSet::from_json(CHANCE).expect("机会牌应能解析");
        let fate = CardSet::from_json(FATE).expect("命运牌应能解析");
        assert!(chance.validate(CardKind::Chance).is_ok());
        assert!(fate.validate(CardKind::Fate).is_ok());
        assert!(chance.0.len() >= 5, "机会牌数偏少: {}", chance.0.len());
        assert!(fate.0.len() >= 5, "命运牌数偏少: {}", fate.0.len());
    }

    /// 第三副池：拿在手里的卡。它有自己的不变式（必须有价），所以不能只靠上面那条
    /// 顺带过一遍 —— 忘了给 `Item` 定价的卡会安静地变成「货架上买不到的商品」。
    #[test]
    fn the_item_pool_validates_and_carries_prices() {
        let items = CardSet::from_json(ITEMS).expect("道具牌应能解析");
        assert!(items.validate(CardKind::Item).is_ok());
        assert!(!items.0.is_empty(), "第一批可持有卡该有货");
        for c in &items.0 {
            assert!(c.cost_points.is_some(), "{} 没有点券售价", c.id);
        }
        // 混进事件池的 kind 由同一条 `validate` 拦住
        assert!(items.validate(CardKind::Fate).is_err());
        let mut free = items.clone();
        free.0[0].cost_points = None;
        assert!(
            free.validate(CardKind::Item).is_err(),
            "货架商品没有价应该报错"
        );
    }

    /// P13b 那一族的形状：**被动卡是一张钥匙，不是一个动词**。
    ///
    /// 三件事一起钉：① 嫁祸卡在池里、45 点券、**不带效果**（PLAN §七 那一行的 ④ ——
    /// 它与免费卡同档，那张免掉一笔很贵的账，这张把一次很贵的东西推给别人）；
    /// ② `validate` 新加的那两格禁令真的会咬人 —— 给被动卡塞一条效果、
    /// 或把一张普通卡的效果清空，都必须报错而不是安静地变成「按下去什么都不做」；
    /// ③ 出牌那条路认不认（`crate::hand::is_passive`）读的就是这一位，
    /// 所以「池里说它是被动的」与「规则层拒得掉它」不可能分叉。
    #[test]
    fn a_passive_card_is_a_key_not_a_verb() {
        let items = CardSet::from_json(ITEMS).expect("道具牌应能解析");
        let shift = items
            .0
            .iter()
            .find(|c| c.id == "item_shift")
            .expect("嫁祸卡该在第三副池里（10-27 拍板：进 item_cards）");
        assert_eq!(shift.passive, Some(PassiveCard::Shift));
        assert_eq!(shift.cost_points, Some(45), "卡面价是那张拍板里的数");
        assert!(
            shift.effects.is_empty(),
            "被动卡不许带效果：它没有『打出去』那一刻"
        );
        // 池里凡是被动卡，出牌判据都认它 —— 这一句是「两处各判一次」的防线
        for c in items.0.iter().filter(|c| c.passive.is_some()) {
            assert!(
                crate::hand::is_passive(c),
                "{} 在数据里是被动卡而出牌判据认它打得出去",
                c.id
            );
        }

        // 禁令的两条各咬一次：把嫁祸卡装上一条效果（它「打出去会发生什么」就有了第二个答案）
        let mut rigged = items.clone();
        let victim = rigged
            .0
            .iter_mut()
            .find(|c| c.id == "item_shift")
            .expect("同上");
        victim.effects = vec![CardEffect::DeductCash { amount: 100 }];
        assert!(
            rigged.validate(CardKind::Item).is_err(),
            "被动卡带效果必须报错，否则门与出牌各处各有一份答案"
        );
        // 再把一张普通卡掏空（货架上一件按下去什么都不做的东西）
        let mut hollow = items.clone();
        let first = &mut hollow.0[0];
        first.effects.clear();
        assert!(
            hollow.validate(CardKind::Item).is_err(),
            "既无效果又非被动的那条卡必须报错"
        );
    }

    /// 陷害卡与命运入狱共用同一条效果，区别只在 `target`。这条钉住那个形状：
    /// 将来谁给「陷害」单开一个 `CardEffect` 变体，两边就会开始各修一半。
    ///
    /// （① 之前这一条还顺手检查了免罪卡那条 `insulate` 效果；P13b② 把它换成
    /// `passive: "forgive"` 之后那一半归 `a_passive_card_is_a_key_not_a_verb`，
    /// 这里不再重复。）
    #[test]
    fn framing_is_the_jail_effect_aimed_at_somebody_else() {
        let items = CardSet::from_json(ITEMS).unwrap();
        let frame = items
            .0
            .iter()
            .find(|c| c.target == CardTarget::Player)
            .expect("第一批卡里该有指定对手的卡");
        assert!(
            frame
                .effects
                .iter()
                .any(|e| matches!(e, CardEffect::Detain { turns, .. } if *turns > 0)),
            "{} 的效果不是关人",
            frame.id
        );
    }

    #[test]
    fn every_effect_kind_is_exercised_by_the_data() {
        let all = CardSet::from_json(CHANCE)
            .unwrap()
            .0
            .into_iter()
            .chain(CardSet::from_json(FATE).unwrap().0)
            .chain(CardSet::from_json(ITEMS).unwrap().0)
            .flat_map(|c| c.effects)
            .collect::<Vec<_>>();

        let has = |pred: fn(&CardEffect) -> bool| all.iter().any(pred);
        assert!(has(|e| matches!(e, CardEffect::AddCash { .. })));
        assert!(has(|e| matches!(e, CardEffect::DeductCash { .. })));
        assert!(has(|e| matches!(
            e,
            CardEffect::Detain {
                place: crate::player::DetentionPlace::Jail,
                turns: 3
            }
        )));
        // 三个去处各要有真生产者。只落「监狱」而把另两个留给「以后再说」，那个枚举就是
        // 两格死代码 —— 而「坐牢/住院/出国是同一件事的三种名字」这条设计意图也就无从验证
        for place in [
            crate::player::DetentionPlace::Jail,
            crate::player::DetentionPlace::Hospital,
            crate::player::DetentionPlace::Away,
        ] {
            assert!(
                all.iter()
                    .any(|e| matches!(e, CardEffect::Detain { place: p, .. } if *p == place)),
                "没有任何一张卡会把人送去{place:?} —— 那一格是死的"
            );
        }
        // `MoveTo` 是这张表上**唯一**没有数据使用者的动作：10-26 起点整格删掉之后，
        // 原来那条「回到起点领 $2000」跟着退场。留它不是因为舍不得，是因为已经有一位
        // 拍过板的真生产者在路上 —— 研究所 3 级的**传送机**（10-26 实报，P10 的车行与
        // 研究所那一站）要把自己、别的玩家或路上的放置物瞬移到视野里指定的那一格。
        // 在它落地之前，这一格的证据只有 `tests/settle.rs` 那两条行为用例；**哪天牌池里
        // 出现了 move_to，就把下面这条反向断言删掉**，它存在的唯一目的是让豁免不会过期
        assert!(
            !has(|e| matches!(e, CardEffect::MoveTo { .. })),
            "牌池里出现了 `move_to` —— 上面那条豁免该销掉了"
        );
        assert!(has(|e| matches!(e, CardEffect::LandStatus { .. }),));
        // 被动卡那三张**不在**上面这张表里（它们的 `effects` 是空的，理由见
        // `a_passive_card_is_a_key_not_a_verb`），所以这一句要换个键问：门上那三问
        // 各自要有一张买得到的卡。少一张的形状是「门问得起这一问，而货架／宝箱／命运
        // 三条来源一条都发不到它」
        let held: Vec<_> = CardSet::from_json(ITEMS)
            .unwrap()
            .0
            .iter()
            .filter_map(|c| c.passive)
            .collect();
        for want in [
            PassiveCard::Forgive,
            PassiveCard::FreePass,
            PassiveCard::Shift,
        ] {
            assert!(
                held.contains(&want),
                "道具池里没有 `passive` 是 {want:?} 的那一张 —— 门上那一问没有来源"
            );
        }
        assert!(has(|e| matches!(e, CardEffect::BuyLand)));
        assert!(has(|e| matches!(e, CardEffect::Reverse)));
        assert!(has(|e| matches!(e, CardEffect::Stay { .. })));
        assert!(
            has(|e| matches!(e, CardEffect::MoveCap { turns } if *turns == 3)),
            "乌龟卡该是「3 回合只走 1 格」"
        );
    }

    /// 「卡上的效果」与「卡要落在什么头上」必须成对，否则数据能过加载、卡却永远打不出去。
    /// 两条绑定各测一个反例：写歪的那一半会在 `validate` 里响，而不是等到玩家按下去。
    #[test]
    fn an_effect_that_needs_a_target_is_bound_to_that_target() {
        let items = CardSet::from_json(ITEMS).unwrap();

        let mut hike = items.clone();
        hike.0
            .iter_mut()
            .find(|c| {
                c.effects
                    .iter()
                    .any(|e| matches!(e, CardEffect::LandStatus { .. }))
            })
            .expect("货架上该有盖状态的卡")
            .target = CardTarget::SelfOnly;
        assert!(
            hike.validate(CardKind::Item).is_err(),
            "盖地块状态的卡没了 land 目标就该被拦下"
        );

        let mut grab = items;
        grab.0
            .iter_mut()
            .find(|c| c.effects.contains(&CardEffect::BuyLand))
            .expect("货架上该有购地卡")
            .target = CardTarget::Land;
        assert!(
            grab.validate(CardKind::Item).is_err(),
            "买无主地的卡指着有主的地，两条判据当场矛盾"
        );

        // 反例的反例：转向卡**不该**被绑住。它没有 aim 也成立（打给自己掉头），
        // 所以把它写成 `self_only` 照样是一张合法的卡 —— 哪天有人给 `Reverse` 加一条
        // target 检查，这条会先红，而那正是「为了整齐把一条真规则写窄了」的形状
        let mut turn = CardSet::from_json(ITEMS).unwrap();
        turn.0
            .iter_mut()
            .find(|c| c.effects.contains(&CardEffect::Reverse))
            .expect("货架上该有转向卡")
            .target = CardTarget::SelfOnly;
        assert!(
            turn.validate(CardKind::Item).is_ok(),
            "转向卡被 target 绑住了：{:?}",
            turn.validate(CardKind::Item).unwrap_err()
        );
    }

    #[test]
    fn card_ids_are_unique_within_pool() {
        let mut set = CardSet::from_json(CHANCE).unwrap();
        let first = set.0[0].clone();
        set.0.push(first);
        assert!(set.validate(CardKind::Chance).is_err());
    }

    #[test]
    fn wrong_kind_is_rejected() {
        let mut set = CardSet::from_json(CHANCE).unwrap();
        set.0[0].kind = CardKind::Fate;
        let errs = set.validate(CardKind::Chance).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("不符")), "{errs:?}");
    }

    #[test]
    fn empty_effects_and_bad_amounts_are_rejected() {
        let mut set = CardSet::from_json(FATE).unwrap();
        set.0[0].effects.clear();
        assert!(set.validate(CardKind::Fate).is_err());

        let mut set = CardSet::from_json(FATE).unwrap();
        set.0[0].effects = vec![CardEffect::DeductCash { amount: 0 }];
        assert!(set.validate(CardKind::Fate).is_err());
    }
}
