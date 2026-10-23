//! 玩家实体：运行时状态是组件，开局配置是 `GameSetup`。

use bevy::prelude::*;
use serde::Deserialize;

use crate::board::TileIndex;
use crate::rules::GameRules;
use crate::world::Gameplay;

use std::collections::VecDeque;

/// 玩家标识。与 `LandId`、`TileIndex` 互不通用 —— 这是缺陷 #1
/// （拿 `"L"+land_id` 字符串 key 去调 `get_tile(index:int)`，导致贷款上限恒为 0）
/// 在 Rust 里编译不过的结构性原因。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PlayerId(pub u32);

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct PlayerName(pub String);

/// 归属色，sRGB 三分量。规则层刻意不用 bevy 的 `Color`：`default_app` 特性子集里
/// 根本不包含 `bevy_color`，而且颜色本就只该由表现层解释。
#[derive(Component, Debug, Clone, Copy, PartialEq)]
pub struct PlayerColor(pub [f32; 3]);

impl PlayerColor {
    pub const fn srgb(r: f32, g: f32, b: f32) -> Self {
        Self([r, g, b])
    }
}

/// 棋子配色，照搬 Godot `board.gd:33-38`（四名上限即 `DEV_PLAN.md` 的对局人数上限）。
pub const PLAYER_COLORS: [PlayerColor; 4] = [
    PlayerColor::srgb(1.0, 0.3, 0.3),
    PlayerColor::srgb(0.3, 0.5, 1.0),
    PlayerColor::srgb(0.3, 0.85, 0.4),
    PlayerColor::srgb(1.0, 0.75, 0.1),
];

/// 现金。加减钱的入口在 B3 的结算 system，那里要同时发 `CashChanged` 与处理不足额（缺陷 #3）。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wallet {
    pub cash: i64,
}

/// 点券：与现金**完全分账的第二货币**（PLAN §一 拍板「永不通兑」）。
///
/// 单独一个组件而不是 `Wallet` 的第二个字段，是为了让「两套账」在类型上成立：
/// 点券的变动只走 [`crate::msg::PointsChanged`]，永不进 `CashChanged`，
/// 于是 B3 那条现金审计（余额 == 起始 + Σ 流水）既抓不到点券、也不该抓到。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Points(pub u32);

/// 银行账户：存款生息、贷款计息（`bank_system.gd` 的 deposit/loan 字段）。
/// 利息结算不经过现金 —— 存款利息进余额、贷款利息进负债，与 Godot 一致。
///
/// `financing` 是融资（P10d②，银行保险董事长特权）：从**其他玩家的存款**里挪来的
/// 那部分钱。它与 `loan` 分家有三条理由 —— 不计息（挪用不是贷款）、丢掉董事长身份
/// 的那一刻被立刻收回（`bank.rs` 的回收 system）、赔付「银行付不出的取款」时先于
/// 贷款挪用（见 `bank_pool` 的口径）。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BankBalance {
    pub deposit: i64,
    pub loan: i64,
    pub financing: i64,
}

/// 逻辑位置：永远只在主环上。
///
/// 入狱时停在**接入点**而不是死胡同格本身（`player_data.gd:62-68` 的语义），
/// 死胡同格只是表现层把棋子挪过去的视觉停车位 —— 这条区分是「core 拥有逻辑位置、
/// render 只做插值」的前提，否则出狱后不知道该从哪格继续走。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenPosition {
    pub tile: TileIndex,
}

/// 被送离场的那一族，各去处共用一个状态（P4；P10c 起多了第四个名字）。
///
/// **为什么是一个枚举而不是三个组件**：4 代把它们叫作坐牢、住院、被绑架/出国旅游，
/// 后面还有旅馆住宿与航空出国（P10）、外星人绑架（P11）—— 六七个名字，一件事：
/// 「人不在牌桌上 N 个回合，棋子停在某个死胡同格，回不来就跳过回合」。各建一个状态
/// 就要在每一处结算里写一遍 `if 坐牢 || 住院 || 出国`，而那些分支的差别只有文案与
/// 死胡同格是哪一个。差别收成数据，规则就只有一处。
///
/// 与 [`StaySentence`] 的区别是这条线的边界：**停留的人还在场上**（棋子不动、地照收租），
/// 被拘留的人不在。所以两张计时各自独立，谁也不许顺手把对方的规则扩过来。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DetentionPlace {
    /// 监狱（`BranchKind::Jail` 那条死胡同）
    Jail,
    /// 医院（`BranchKind::Hospital`）
    Hospital,
    /// 不在棋盘上：被绑架、出国旅游、坐飞机。**没有死胡同格**，
    /// 所以棋子只能停在接入点上（与「棋盘没有分支格」走的是同一条退路）
    Away,
    /// 旅馆住宿（P10c，B 型原型装配出来的那一个）。与 [`Self::Away`] 一样**不在地图上**
    /// —— 棋子停在旅馆所在的那一格，等退房；它与「出国」分开成两枚不是模型需要，
    /// 是文案需要：住旅馆的人「退房」回来，说「回国」是撒谎。
    Hotel,
}

impl DetentionPlace {
    /// 这条棋盘上对应哪条死胡同分支。`None` = 那个地方压根不在地图上。
    pub const fn branch(self) -> Option<crate::board::BranchKind> {
        use crate::board::BranchKind as B;
        match self {
            Self::Jail => Some(B::Jail),
            Self::Hospital => Some(B::Hospital),
            Self::Away | Self::Hotel => None,
        }
    }

    /// 人话：他在哪儿。顶栏、HUD、dump 三处要说同一句话，所以映射只写一次。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Jail => "监狱",
            Self::Hospital => "医院",
            Self::Away => "国外",
            Self::Hotel => "旅馆",
        }
    }

    /// 回来那一下的动词。「出狱」「出院」「回国」是几句话，但在模型里是同一条凭据
    /// （`DetentionLifted`）—— 所以变的只有这一个词。
    pub const fn back_verb(self) -> &'static str {
        match self {
            Self::Jail => "出狱",
            Self::Hospital => "出院",
            Self::Away => "回国",
            Self::Hotel => "退房",
        }
    }

    /// 「他此刻在那一处」的短语，供按处筛人的那一句人话拼（P11b② 的 `FieldFilter::Detained`）。
    ///
    /// 不复用 [`Self::label`] 再在外面加一个「关在」：监狱与医院确实是关着，而**出国与住旅馆
    /// 不是** —— 那句写成「关在国外的人」是一件不存在的事，而它读起来像一条规则。
    /// 四格都写成中性的「在 X」，谁要加「关」字由各自的句子去说。
    pub const fn held_phrase(self) -> &'static str {
        match self {
            Self::Jail => "在监狱",
            Self::Hospital => "在医院",
            Self::Away => "在国外",
            Self::Hotel => "在旅馆",
        }
    }
}

/// 存在即被拘留中（P4，取代 B2b 起的 `JailSentence`）。递减口径见 `begin_turn_system`：
/// 每次回合开始扣一格，扣到只剩 1 的那一回合**当场可行动**，所以「判 3 天」= 跳过 2 个回合。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Detention {
    pub place: DetentionPlace,
    pub turns_left: u32,
}

/// 存在即「原地停留」中（停留卡）。**字面口径（P4④）**：写 N 就停 N 个回合。
///
/// 它不是自动跳回合：身上有它的人照常进入 `WaitRoll`，只是掷骰按钮翻成「停留」，
/// 点一下（AI 发 [`crate::msg::RequestStay`]）才结束这一回合，同时把计数减一。
/// 所以「20 点券 / 停 1 回合」在盘面上读作「少走一步」，与拘留的「判 3 天 = 跳过 2 回合」
/// 是两把不同的尺 —— 拘留数的是天（含回来那天），停留数的是回合。
///
/// **为什么不并进 [`Detention`]**：被拘留的人被送进死胡同
/// 格、且（P4② 起）丧失收租权；停留的人**还在场上**，棋子停在他踩到的那一格，名下的地
/// 照样收租。这两件事在规则上只差「他在不在牌桌上」这一点，而差别一旦写进同一个组件，
/// 就要靠 `place` 字段的分支去还原 —— [`Detention`] 收的是
/// 「被送离场」那一族，停留不属于它（PLAN §四 P4 那行的边界）。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct StaySentence {
    pub turns_left: u32,
}

/// 「这一回合走不远」（乌龟卡，P5）：连续 `turns_left` 个自己的掷骰回合，
/// 骰子照摇、点数照记，实际只走 1 格。
///
/// 与 [`StaySentence`] 是两条计时，分工就是 PLAN 拍板那一句：停留＝**不动**（吃掉整个
/// 回合），少动＝**还在动、只是走不远**（回合照过、地照收租）。所以它不挂 `Detention`、
/// 也不阻止掷骰，只在 `roll_dice_system` 入队前把步数夹到 1。
///
/// 计数口径与停留同一条**字面**：写 3 就影响 3 个自己的掷骰回合；被拘留/被停留按掉的
/// 回合没有掷骰，也就不消耗它。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct MoveCap {
    pub turns_left: u32,
}

/// 一份在效期内的保单（P10d②，人寿保险）：`days_left` 个自己的回合内，坐牢/住院
/// 由 `insurer` 那家公司按 `indemnity_per_day × 拘留天数` 赔现金。
///
/// 它是身上的**第四条计时**（与 [`Detention`] / [`StaySentence`] / [`MoveCap`] 同族）：
/// 扣在**他自己的回合开始**上 —— 这局里「一天」的既定口径就是「他的一个回合」，
/// 拘留的天数是这么数的，保单的效期没有理由换一把尺。
///
/// `insurer` 存**股票 id 字符串**而不存实体：赔付要把负数记进那家公司的损益账
/// （`CompanyLedgers` 按股票 id 分页），账页的键就是它。保费与赔付单价在**投保那一刻
/// 随保单冻结** —— 落格时棋盘数据给了什么就是什么，之后棋盘改价不追溯旧保单。
///
/// 再投保 = 买一份新保单、旧的作废（不是 `max` 也不是累加 —— 你付了新保费，
/// 就该拿到新保单写的那几天）。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Insured {
    pub days_left: u32,
    pub insurer: String,
    pub indemnity_per_day: i64,
}

/// 载具（P5）：步行 1 颗骰、机车 2 颗、汽车 3 颗。
///
/// 必挂组件而不是 `Option`：缺组件与「步行」读出同一个能力，而 `Option` 只会把漏挂
/// 变成一条永远不响的静默降级 —— 与 [`Heading`] 同一条理由。
///
/// 它决定的是**摇几颗骰子**（`Rng::roll_die` 摇 `dice()` 次再相加），不是「走几格」：
/// 骰子颗数由玩家在掷骰前选（`RequestRoll.dice`），遥控骰子与乌龟卡的夹步是另一层。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mount {
    #[default]
    Foot,
    Bike,
    Car,
}

impl Mount {
    /// 这一载具最多能摇几颗骰子。
    pub const fn dice(self) -> u32 {
        match self {
            Self::Foot => 1,
            Self::Bike => 2,
            Self::Car => 3,
        }
    }

    /// 是不是「有车」——恶犬与「交通堵塞」这类按载具筛人的规则读这一句。
    pub const fn is_vehicle(self) -> bool {
        !matches!(self, Self::Foot)
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Foot => "步行",
            Self::Bike => "机车",
            Self::Car => "汽车",
        }
    }
}

/// 绕行方向（转向卡，P3d④b）。
/// **它是玩家身上的一个位，不是棋盘的第二份拓扑**：ADR-001 行为规则 3 说主环是单向环，
/// 而转向卡的定义（`GLOSSARY-board.md`）是「仍沿同一环，只是反向」—— 所以环还是那一条
/// `tiles[]`，变的只是步进取 `+1` 还是 `-1`。如果这里存的是「一条反向的格子表」，
/// 就等于承认有两份真相（硬约束第 4 条）。
///
/// 缺省是 `Forward`，而开局每个玩家都挂着它（[`spawn_players`]）—— 查询一律按 `&Heading`
/// 要它，别用 `Option`：「没有这个组件」与「正向」读出同一个方向，用 `Option` 只会把
/// 漏挂组件这件事变成一条永远不响的静默降级。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Heading {
    /// 沿 `tiles[]` 的下标递增方向走（正常绕行）。
    #[default]
    Forward,
    /// 沿同一条环倒着走（`i → (i-1+N)%N`）。
    Backward,
}

impl Heading {
    /// 步进符号。`BoardDef::walk_path` 吃的是**带方向的步数**，所以骰子的点数在这里乘上它。
    pub const fn sign(self) -> i64 {
        match self {
            Self::Forward => 1,
            Self::Backward => -1,
        }
    }

    /// 掉头。转向卡翻的就是这一位，而翻两次回到原样（不是「叠加两层」）。
    pub const fn flipped(self) -> Self {
        match self {
            Self::Forward => Self::Backward,
            Self::Backward => Self::Forward,
        }
    }

    /// 人话。HUD 与 dump 里「他接下来往哪走」要说得出口。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Forward => "正向",
            Self::Backward => "逆向",
        }
    }
}

/// 一次行走的进度，挂在玩家实体上；`path` 为空即「没在走」。
///
/// 与 §4.6 原案（`WalkQueue{path, elapsed}`）的差异：耗时累计放在全局
/// [`WalkClock`](crate::walk::WalkClock)（行走是互斥的，同一时刻至多一人在走），
/// 队列里只留「绝对进度」—— `done` 记已发过 `TileStepped` 的步数，
/// 时钟换算出的应到步数减去它就是本帧要补的步数。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct WalkQueue {
    /// 还没走的格（不含出发点，按绕行方向）。空 = 已到站或没在走。
    pub path: VecDeque<TileIndex>,
    /// 本段行走已完成的步数。
    pub done: u32,
    /// 本段行走的出发点（`PlayerArrived` 的 `from`）。
    pub from: TileIndex,
}

impl Default for WalkQueue {
    fn default() -> Self {
        Self {
            path: VecDeque::new(),
            done: 0,
            from: START_TILE,
        }
    }
}

impl WalkQueue {
    /// 队列里的总步数（含已完成的）。
    pub fn total(&self) -> usize {
        self.done as usize + self.path.len()
    }
}

/// 存在即破产。破产玩家退出回合序，钱包与地产由结算 system 清空。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bankrupt;

/// 起始格：主环 0 号（prototype 里是「起点」）。Godot 端同样假定 `tile_index = 0`
/// （`game_state.gd:42`），棋盘校验保证主环非空。
pub const START_TILE: TileIndex = TileIndex(0);

/// 开局时一个玩家的配置项。 spawned 之后就不再读。
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerSetup {
    pub name: String,
    pub color: PlayerColor,
}

/// 对局配置：玩家名单。表现层/测试在开局前插入即可覆盖默认阵容。
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameSetup {
    pub players: Vec<PlayerSetup>,
}

impl Default for GameSetup {
    /// 默认阵容与 Godot `board.gd:341-344` 一致：阿土伯 vs 孙小美。
    fn default() -> Self {
        Self {
            players: vec![
                PlayerSetup {
                    name: "阿土伯".into(),
                    color: PLAYER_COLORS[0],
                },
                PlayerSetup {
                    name: "孙小美".into(),
                    color: PLAYER_COLORS[1],
                },
            ],
        }
    }
}

/// 铺出玩家实体，返回实体句柄（回合序 `TurnOrder` 用）。
pub fn spawn_players(commands: &mut Commands, setup: &GameSetup, rules: &GameRules) -> Vec<Entity> {
    setup
        .players
        .iter()
        .enumerate()
        .map(|(i, p)| {
            commands
                .spawn((
                    PlayerId(i as u32),
                    PlayerName(p.name.clone()),
                    p.color,
                    Wallet {
                        cash: rules.start_cash,
                    },
                    // 点券开局为 0（PLAN §三：来源只有点券格与小游戏格）
                    Points::default(),
                    // 手牌开局为空：道具卡只能从百货公司买到（P3d），没有任何格子会发卡
                    crate::hand::Hand::default(),
                    // 道具栏开局送一个路障（PLAN §三：路障/机器娃娃/机器工人开局即送）
                    crate::items::Inventory::starting(),
                    TokenPosition { tile: START_TILE },
                    WalkQueue::default(),
                    // 开局恒正向：转向卡是玩家身上的事，不是棋盘的事（见 [`Heading`]）
                    Heading::default(),
                    // 开局步行：载具的来路（车行）还没装配，P5 先让规则层把骰子颗数
                    // 与「毁载具」两条跑通；表现层的换车入口等车行落地再补
                    Mount::Foot,
                    BankBalance::default(),
                    crate::stock::StockHoldings::default(),
                    // 乐透券开局为空。它是**一期**的凭证：开奖有人猜中就全场清空，
                    // 没人猜中则留到下月（见 `crate::lottery` 模块头）
                    crate::lottery::LotteryTickets::default(),
                    Gameplay,
                ))
                .id()
        })
        .collect()
}
