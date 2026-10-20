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
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BankBalance {
    pub deposit: i64,
    pub loan: i64,
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

/// 被送离场的那一族，三种去处共用一个状态（P4）。
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
    /// 不在棋盘上：被绑架、出国旅游、住旅馆、坐飞机。**没有死胡同格**，
    /// 所以棋子只能停在接入点上（与「棋盘没有分支格」走的是同一条退路）
    Away,
}

impl DetentionPlace {
    /// 这条棋盘上对应哪条死胡同分支。`None` = 那个地方压根不在地图上。
    pub const fn branch(self) -> Option<crate::board::BranchKind> {
        use crate::board::BranchKind as B;
        match self {
            Self::Jail => Some(B::Jail),
            Self::Hospital => Some(B::Hospital),
            Self::Away => None,
        }
    }

    /// 人话：他在哪儿。顶栏、HUD、dump 三处要说同一句话，所以映射只写一次。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Jail => "监狱",
            Self::Hospital => "医院",
            Self::Away => "国外",
        }
    }

    /// 回来那一下的动词。「出狱」「出院」「回国」是三句话，但在模型里是同一条凭据
    /// （`DetentionLifted`）—— 所以变的只有这一个词。
    pub const fn back_verb(self) -> &'static str {
        match self {
            Self::Jail => "出狱",
            Self::Hospital => "出院",
            Self::Away => "回国",
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

/// 绕行方向（转向卡，P3d④b）。
///
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
                    TokenPosition { tile: START_TILE },
                    WalkQueue::default(),
                    // 开局恒正向：转向卡是玩家身上的事，不是棋盘的事（见 [`Heading`]）
                    Heading::default(),
                    BankBalance::default(),
                    crate::stock::StockHoldings::default(),
                    Gameplay,
                ))
                .id()
        })
        .collect()
}
