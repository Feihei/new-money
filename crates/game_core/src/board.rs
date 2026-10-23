//! 棋盘定义：ADR-001「网格路网 + 单向主环 + 事件死胡同分支」的 serde 落地。
//!
//! 只描述静态定义（`BoardDef`），运行时状态（归属/等级/抵押）是 Land 实体上的组件，
//! 因此 JSON 里的 `owner`/`level` 字段刻意不出现在本模块——静态与运行时不可混。

use std::collections::BTreeMap;

use bevy::asset::Asset;
use bevy::math::Vec3;
use bevy::reflect::TypePath;
use serde::Deserialize;

use crate::player::Heading;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TileKind {
    Start,
    Property,
    Bank,
    Stock,
    Chance,
    Fate,
    Jail,
    Hospital,
    Lottery,
    GodDeity,
    Neutral,
    Road,
    Transport,
    /// 点券箱：一次性拾取、按回合冷却刷新（`.agents/PLAN.md` §五，实现见 `coupon.rs`）。
    Coupon,
    /// 百货公司：踩上去开柜台窗口，点券买卡 / 卖回打 9 折（PLAN §三「百货公司」行，
    /// 实现见 `shop.rs`）。路过不给任何资格 —— 这一格的玩法就是「停下来逛」。
    Shop,
    /// 游乐场：踩上去开一局小游戏，成绩换点券（PLAN §五「小游戏格」，实现见 `minigame.rs`）。
    ///
    /// **三种游戏共用这一个格类型**，靠 `data.game` 分流 —— 不各开一个 `TileKind`，
    /// 否则格类型的数量会被美术资产的数量牵着走。`data.game` 缺省 = 这一趟随机一款。
    Arcade,
    /// 公司企业（P10d①）：**不可拥有的固定格**，绑定一只股票（`data.stock`）——
    /// 不走 `land_id`/`building_kind` 那条路：公司没有「地」可买，持股与董事长
    /// 都挂在股票上（见 `company.rs`），格子只是那家公司的**门面**。
    Company,
}

impl TileKind {
    // 这里刻意**没有** `is_detention()`：格子类型不决定「人会不会被关进去」——
    // 天数只来自卡面，踩到哪一格都不抓人（10-20 拍板，P4④）。那类谓词留着只会
    // 让后来人以为规则在格子上。
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BranchKind {
    Jail,
    Hospital,
}

/// 新增 land_type 会得到一条明确的加载错误，而不是静默降级——模组作者需要的是前者。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LandType {
    Residential,
    Commercial,
    Insurance,
}

/// 商业设施五类（PLAN §四 P10）。住宅地没有这一位；商业地缺省是
/// `None` ＝ 空商业地（业主买下后再踩一次选建设项，P10c 的 `Pending::Build`），
/// 写了就是预建设施 —— 演示盘的外滩是预建的购物中心，P10c 先有东西可冒烟。
///
/// 五类各自绑定哪个收费原型（A 随机金额 / B 随机停留 / C 按步数 / D 功能）
/// 是 [`FeeDef`] 的形状决定的，不是这里的分支：换参数就换行为是 P10 的真验收
/// （见 `game_core::service` 模块头），原型库对「住在哪类设施」一无所知。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BuildingKind {
    /// 购物中心：踩到摇 1–6，按档位收（A 型）。
    Mall,
    /// 旅馆：踩到摇 1–4，强制住宿该天数 + 按天付费（B 型，接 P4 拘留）。
    Hotel,
    /// 加油站：油费按步数、只对载具收（C 型，接 P5 载具）。
    GasStation,
    /// 研究所：功能型，产道具（D 型，接 P5 库存）。
    Lab,
    /// 公园：功能型，无费用（D 型空实现）。
    Park,
}

impl BuildingKind {
    /// 五类设施的**全部候选**，顺序就是建设窗口里那五颗按钮的次序。写成常量而不是让
    /// UI 自己去枚举：加第六类时这里与 `label` 两处必须一起改，而漏掉一个分支会编译失败。
    pub const ALL: [Self; 5] = [
        Self::Mall,
        Self::Hotel,
        Self::GasStation,
        Self::Lab,
        Self::Park,
    ];

    /// 屏幕上那个名字。建设按钮、服务费收据、dump 的地块栏、顶栏播报四处共用一个 ——
    /// 各写一份就会出现「按钮写购物中心、dump 写商场」这种同一样东西两个名字的问题
    /// （与 `ItemKind::label`／`Mount::label` 同一条分工）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::Mall => "购物中心",
            Self::Hotel => "旅馆",
            Self::GasStation => "加油站",
            Self::Lab => "研究所",
            Self::Park => "公园",
        }
    }
}

/// 收费原型的参数（serde 形，P10b）。**收费就是数据**：购物中心与旅馆的档位、
/// 单价都由棋盘 JSON 给，改平衡不回 Rust 改代码。加载期的形状校验见
/// `validate`（档位表必须盖满骰面、C 型必须给单价）—— 运行期的退路
/// （缺档不收钱、不 panic）在 [`crate::service::assess`]。
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FeeDef {
    /// A 型：`tiers[i]` 是摇出 `i+1` 点时收的数。表长必须等于骰面数。
    RandomAmount { sides: u32, tiers: Vec<i64> },
    /// B 型：住几天付几天的钱。旅馆骰 1–4，航空转盘是另一面数 —— 同一形状。
    RandomStay { sides: u32, per_day: i64 },
    /// C 型：单价 × 本回合骰面。`requires_mount` 是载体参数（油费 true、
    /// 电脑维修 false），写错方向就是「步行付油费」。
    PerStep { rate: i64, requires_mount: bool },
}

impl FeeDef {
    /// 换成原型库认的那个形状。数据 → 原型是纯映射，没有任何判定 ——
    /// 判定全在 [`crate::service::assess`]，这里错了 `validate` 会拦。
    pub fn to_prototype(&self) -> crate::service::FeePrototype {
        match self {
            Self::RandomAmount { sides, tiers } => crate::service::FeePrototype::RandomAmount {
                sides: *sides,
                tiers: tiers.clone(),
            },
            Self::RandomStay { sides, per_day } => crate::service::FeePrototype::RandomStay {
                sides: *sides,
                per_day: *per_day,
            },
            Self::PerStep {
                rate,
                requires_mount,
            } => crate::service::FeePrototype::PerStep {
                rate: *rate,
                requires_mount: *requires_mount,
            },
        }
    }
}

/// 公司企业的收费参数（P10d①）。与 [`FeeDef`] 分开不是形状不同，是**可选集**不同：
/// 档位式的 A 型是购物中心那条街的生意，公司不收；D 型公司（日产建设，P10d②）
/// 根本没有 fee 字段。B/C 两型与商业设施完全同形 —— 「同一原型换两个参数」
/// 是 P10 的真验收，这里多长一层皮就多一处会分叉的地方。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CompanyFee {
    /// B 型：住几天付几天的钱。联合航空的转盘（出国 N 天）。
    RandomStay { sides: u32, per_day: i64 },
    /// C 型：单价 × 本回合骰面。暴富科技的电脑维修费（与载具无关）。
    PerStep { rate: i64, requires_mount: bool },
    /// B 型变体（P10d②）：转盘摇出的是**投保天数**而不是离场天数 —— 人不离场，
    /// 付 `premium_per_day × 天数` 的保费换来一份保单；投保期间坐牢/住院由公司按
    /// `indemnity_per_day × 拘留天数` 赔现金。它不进 [`FeePrototype`]：保险的产物
    /// 是身上的一个状态而不是一笔离场，`assess` 的「stay_days = 人走几天」在这里
    /// 没有对应物，硬塞进去就是第二处会分叉的地方。
    Insure {
        sides: u32,
        premium_per_day: i64,
        indemnity_per_day: i64,
    },
}

impl CompanyFee {
    /// 数据 → 原型库是纯映射，判定全在 [`crate::service::assess`]。
    /// 保险（[`CompanyFee::Insure`]）没有对应物 —— 它的产物是保单不是账单，
    /// 结算走 `company.rs` 自己那条路，这里返回 `None`。
    pub fn to_prototype(&self) -> Option<crate::service::FeePrototype> {
        match self {
            Self::RandomStay { sides, per_day } => Some(crate::service::FeePrototype::RandomStay {
                sides: *sides,
                per_day: *per_day,
            }),
            Self::PerStep {
                rate,
                requires_mount,
            } => Some(crate::service::FeePrototype::PerStep {
                rate: *rate,
                requires_mount: *requires_mount,
            }),
            Self::Insure { .. } => None,
        }
    }
}

/// 主环下标。环形拓扑的一切索引都用它，而不是裸 `i64`/`u32`：
/// 与 `LandId`、`PlayerId` 是不同 newtype，混用直接编译失败（缺陷 #1 的防线）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TileIndex(pub u32);

/// 网格坐标（不含世界坐标换算，那是 `BoardDef::world_pos` 的事）。
///
/// 死胡同分支格**没有** `TileIndex`（它们不在环上），所以「棋子被视觉上停到监狱格」
/// 这类事实只能由 `GridPos` 表达 —— 见 `msg::PlayerJailed`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GridPos {
    pub col: u32,
    pub row: u32,
}

impl GridPos {
    pub fn of(tile: &TileDef) -> Self {
        Self {
            col: tile.col,
            row: tile.row,
        }
    }
}

/// 格子附加数据。点券格用 `coupon`/`refresh`，游乐场用 `game`，公司格用 `stock`/`fee`，
/// 其余格为 `{}`。
///
/// 拘留格**不带天数与罚金**：天数只来自卡面（P4④ 拍板），`turns`/`fine` 那份
/// Godot 残留数据已删 —— 留着就是第二份真相。
///
/// **没有 `Copy`**（P10d① 起）：`stock` 是字符串，而 `TileData` 随 `TileDef` 一起
/// 被 `Assets` 持有，读它的地方全是引用 —— 为省两个 `&` 把整份数据变成可隐式移动，
/// 值不上丢掉移动检查。
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TileData {
    /// 点券格的面额。三档 10/30/50 **由数据决定、不随机抽**（`validate` 要求正面额）：
    /// 随机抽会让三档的期望值全部相同，「这一段路值得走」的梯度当场失效（PLAN §五）。
    #[serde(default)]
    pub coupon: Option<u32>,
    /// 取走后隔多少回合重新出现。缺省走 `GameRules::coupon_refresh_turns`。
    /// 写 `0` 是数据错误（等于每次踩都发，「一次性」这条规则白写），由 `validate` 拦下。
    #[serde(default)]
    pub refresh: Option<u32>,
    /// 游乐场这一格固定玩哪一款（P7）。
    ///
    /// **缺省是「这一趟随机一款」而不是数据错误** —— 与点券面额那条相反，这里没有梯度要保：
    /// 三档成绩的点券上限被 `minigame::Payouts` 压在同一个量级，所以「踩到哪一种」不是抽奖。
    /// 真棋盘（P10）一格写死一款，演示盘那一格留空让三种都轮得到。
    #[serde(default)]
    pub game: Option<crate::minigame::GameKind>,
    /// 公司格绑定的股票 id（P10d①）。公司格的**存在理由**就是它：持股、董事长、
    /// 损益账、分红全挂在股票上（`company.rs`），格子只是门面。`validate` 拦两件事：
    /// 公司格不给股票、非公司格带了这一位。
    #[serde(default)]
    pub stock: Option<String>,
    /// 公司格的收费参数（P10d①）。缺省 `None` = 不收费的公司（D 型，P10d② 的日产建设）。
    #[serde(default)]
    pub fee: Option<CompanyFee>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct BranchDef {
    #[serde(rename = "type")]
    pub kind: BranchKind,
    /// 从靠近接入点排到末端；最后一项是死胡同。
    pub tiles: Vec<TileDef>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct TileDef {
    /// 主环下标。内联分支格（死胡同）按 schema 不写 `id`，故有默认值；
    /// `validate()` 会检查主环格是否 `id == 数组下标`。
    #[serde(default)]
    pub id: u32,
    pub col: u32,
    pub row: u32,
    #[serde(rename = "type")]
    pub kind: TileKind,
    pub name: String,
    #[serde(default)]
    pub land_id: Option<u32>,
    #[serde(default)]
    pub branch: Option<BranchDef>,
    #[serde(default)]
    pub data: TileData,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct LandDef {
    pub id: u32,
    pub col: u32,
    pub row: u32,
    /// 1 = 住宅 1x1，2 = 商业 2x2（由 2 个相邻道路格共享）。
    pub size: u8,
    pub name: String,
    /// 同色组键，垄断判定用。
    pub group: Option<String>,
    pub land_type: LandType,
    pub price: i64,
    pub rent: Vec<i64>,
    pub upgrade_cost: i64,
    /// 商业设施类别（P10b）。缺省 `None`；住宅/保险地写了它、商业地写了
    /// 与设施不匹配的 `fee`，都由 `validate` 在加载期拦下。
    #[serde(default)]
    pub building_kind: Option<BuildingKind>,
    /// 这块商业地的收费参数。缺省 `None`（研究所/公园不需要）。
    #[serde(default)]
    pub fee: Option<FeeDef>,
}

#[derive(Debug, Clone, PartialEq, Asset, TypePath, Deserialize)]
pub struct BoardDef {
    pub board_name: String,
    #[serde(default = "default_cell_size")]
    pub cell_size: f32,
    pub grid_cols: u32,
    pub grid_rows: u32,
    /// 主环有序道路格，顺序即绕行方向。`loop` 是 Rust 关键字，字段名用 `tiles`。
    #[serde(rename = "loop")]
    pub tiles: Vec<TileDef>,
    #[serde(default)]
    pub lands: Vec<LandDef>,
    /// 空商业地上**现建**各类设施的默认收费参数（P10c）。
    ///
    /// 预建设施的参数写各自 `LandDef.fee`（外滩的购物中心档位是那条街独有的）；
    /// 这里是「空地上业主自己立一座」那套价 —— 同一类设施在一条棋盘上只有一套
    /// 默认参数，改平衡改 JSON 不回 Rust。缺哪一类，那条棋盘就建不了那一类
    /// （研究所/公园不收费，不需要条目）。形状由 `validate` 拦：给购物中心配
    /// B 型参数、或给研究所配了费，加载期就红。
    #[serde(default)]
    pub facility_fees: BTreeMap<BuildingKind, FeeDef>,
}

const fn default_cell_size() -> f32 {
    2.0
}

pub type BoardError = String;

impl BoardDef {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn loop_len(&self) -> u32 {
        self.tiles.len() as u32
    }

    /// 环形取格：越界回绕，且支持倒着走（转向卡）。
    pub fn tile(&self, index: i64) -> Option<&TileDef> {
        let n = self.tiles.len() as i64;
        if n == 0 {
            return None;
        }
        self.tiles.get(index.rem_euclid(n) as usize)
    }

    /// 走 `steps` 步后的主环索引（`steps` 可为负 = 后退）。
    pub fn destination(&self, from: i64, steps: i64) -> i64 {
        let n = self.tiles.len() as i64;
        if n == 0 {
            return from;
        }
        (from + steps).rem_euclid(n)
    }

    /// 把任意索引（可为负、可越界）折成 `TileIndex`，取模规则与 `tile()` 一致。
    pub fn wrapped(&self, index: i64) -> TileIndex {
        TileIndex(self.destination(index, 0) as u32)
    }

    /// 从 `from` 起走 `steps` 步途经的主环格（不含起点、含终点），按绕行方向。
    ///
    /// ADR-001 行为规则 3/4：正向步进恒为 `i → (i+1)%N`，分支**绝不**作为常规前进选项 ——
    /// 途经挂分支的接入点也只记录主环下标，死胡同格（没有 `TileIndex`）结构上进不了结果。
    ///
    /// `steps` 的**符号就是方向**（P3d④b 用掉了这里预留的那一格）：`Heading::sign()` 乘上
    /// 骰子点数传进来，负数即倒着走（`i → (i-1+N)%N`，回绕规则与 `tile()` 同一套
    /// `rem_euclid`）。路径长度恒等于 `|steps|`，方向不改变"走了几步"这件事。
    /// `steps == 0` 时为空。
    pub fn walk_path(&self, from: i64, steps: i64) -> Vec<TileIndex> {
        if steps == 0 {
            return Vec::new();
        }
        let dir = steps.signum();
        (1..=steps.abs())
            .map(|k| self.wrapped(from + k * dir))
            .collect()
    }

    /// 环上两格之间的**最短**步数（双向都数，同一格＝0）。
    ///
    /// 与 [`Self::walk_path`] 相反，这里不带方向：它回答的是「离我多远」，而「我走过去要几步」
    /// 是另一个问题（[`Self::steps_toward`]）。请神符的「离自己最近」读的是这一条 ——
    /// 那张卡买的就是**不必走到那一格**（与购地卡同一条理由，见 `gods::nearest_god`）。
    pub fn ring_distance(&self, from: TileIndex, to: TileIndex) -> u32 {
        let n = self.loop_len();
        if n == 0 {
            return 0;
        }
        let (a, b) = (from.0 % n, to.0 % n);
        let forward = if b >= a { b - a } else { n - (a - b) };
        forward.min(n - forward)
    }

    /// 沿 `heading` 走到那一格要几步（同一格＝0；倒着走就是倒着数）。
    ///
    /// 它存在的唯一理由是**并列要有个不 arbitrary 的决胜键**：环上两格可以与起点同距
    /// （一前一后），而那时候「他接下来会先遇到谁」是唯一还有玩法意义的分法 ——
    /// 一张转向卡会换掉这一位，而按格号大小决胜什么都换不掉。
    pub fn steps_toward(&self, from: TileIndex, to: TileIndex, heading: Heading) -> u32 {
        let n = self.loop_len();
        if n == 0 {
            return 0;
        }
        let (a, b) = (from.0 % n, to.0 % n);
        match heading {
            Heading::Forward => {
                if b >= a {
                    b - a
                } else {
                    n - (a - b)
                }
            }
            Heading::Backward => {
                if a >= b {
                    a - b
                } else {
                    n - (b - a)
                }
            }
        }
    }

    /// 网格坐标 → 世界坐标（y=0）。ADR-001：坐标一律推导，不手填 x/z。
    pub fn world_pos(&self, col: u32, row: u32) -> Vec3 {
        Vec3::new(
            col as f32 * self.cell_size,
            0.0,
            row as f32 * self.cell_size,
        )
    }

    pub fn tile_world_pos(&self, index: i64) -> Option<Vec3> {
        let t = self.tile(index)?;
        Some(self.world_pos(t.col, t.row))
    }

    /// 土地足迹中心：size=2 时向右下各偏半格，使建筑落在土地格正中。
    pub fn land_world_pos(&self, land_id: u32) -> Option<Vec3> {
        let l = self.land(land_id)?;
        let offset = (l.size as f32 - 1.0) * 0.5;
        Some(self.world_pos_f32(l.col as f32 + offset, l.row as f32 + offset))
    }

    fn world_pos_f32(&self, col: f32, row: f32) -> Vec3 {
        Vec3::new(col * self.cell_size, 0.0, row * self.cell_size)
    }

    /// 相邻两段之间是否为对角（斜向街道）路段。
    pub fn is_diagonal_step(&self, from: i64) -> bool {
        let (Some(a), Some(b)) = (self.tile(from), self.tile(from + 1)) else {
            return false;
        };
        (a.col as i64 - b.col as i64).abs() == 1 && (a.row as i64 - b.row as i64).abs() == 1
    }

    pub fn land(&self, land_id: u32) -> Option<&LandDef> {
        self.lands.iter().find(|l| l.id == land_id)
    }

    /// 这块地上 `kind` 那类设施的收费参数（P10c）。
    ///
    /// 预建设施用**地块自己的**参数（外滩的档位是那条街独有的）；空地上现建的
    /// 用棋盘级默认表 —— 两个来源只在这一处汇合，服务结算与建造判据都走这条，
    /// 「同一座旅馆两套价」在结构上写不出来。
    pub fn facility_fee(&self, land_id: u32, kind: BuildingKind) -> Option<&FeeDef> {
        match self.land(land_id) {
            Some(l) if l.building_kind == Some(kind) => l.fee.as_ref(),
            _ => self.facility_fees.get(&kind),
        }
    }

    pub fn land_for_tile(&self, index: i64) -> Option<&LandDef> {
        self.tile(index)?.land_id.and_then(|id| self.land(id))
    }

    /// 某块土地绑定了哪些道路格（size=2 商业应有 2 个）。
    pub fn tiles_bound_to_land(&self, land_id: u32) -> Vec<u32> {
        self.tiles
            .iter()
            .filter(|t| t.land_id == Some(land_id))
            .map(|t| t.id)
            .collect()
    }

    /// 按网格坐标找格，**含死胡同分支格**。返回 `(主环下标, 格)`，分支格的下标是 `None`。
    ///
    /// 表现层点格、以及任何「手上只有 (col,row)」的场合都该走这里，而不是自己遍历
    /// `tiles`/`branch.tiles` —— 后者的顺序与归属规则（分支格挂在谁的 `branch` 下）
    /// 只写在 `BoardDef` 里一次。
    pub fn tile_at_grid(&self, col: u32, row: u32) -> Option<(Option<TileIndex>, &TileDef)> {
        if let Some(t) = self.tiles.iter().find(|t| t.col == col && t.row == row) {
            return Some((Some(TileIndex(t.id)), t));
        }
        for junction in &self.tiles {
            if let Some(branch) = &junction.branch
                && let Some(t) = branch.tiles.iter().find(|t| t.col == col && t.row == row)
            {
                return Some((None, t));
            }
        }
        None
    }

    pub fn branch_junction(&self, kind: BranchKind) -> Option<&TileDef> {
        self.tiles
            .iter()
            .find(|t| matches!(&t.branch, Some(b) if b.kind == kind))
    }

    /// 死胡同末端格（视觉棋子停这里），不是接入点。
    pub fn branch_end(&self, kind: BranchKind) -> Option<&TileDef> {
        self.branch_junction(kind)?.branch.as_ref()?.tiles.last()
    }

    /// 移植 Godot `BoardData::validate` + `tools/validate_board.py` 的全部不变式。
    pub fn validate(&self) -> Result<(), Vec<BoardError>> {
        let mut errs = Vec::new();

        if self.tiles.is_empty() {
            return Err(vec!["主环为空".into()]);
        }

        for (i, t) in self.tiles.iter().enumerate() {
            if t.id as usize != i {
                errs.push(format!("loop[{i}] 的 id={} 与下标不符", t.id));
            }
            if t.col >= self.grid_cols || t.row >= self.grid_rows {
                errs.push(format!(
                    "格 {} ({},{}) 越界 {}x{}",
                    t.id, t.col, t.row, self.grid_cols, self.grid_rows
                ));
            }
        }

        // 主环闭合：相邻（含末→首）必须正交或对角相邻
        for i in 0..self.tiles.len() {
            let a = &self.tiles[i];
            let b = &self.tiles[(i + 1) % self.tiles.len()];
            let dc = (a.col as i64 - b.col as i64).abs();
            let dr = (a.row as i64 - b.row as i64).abs();
            if dc > 1 || dr > 1 || (dc == 0 && dr == 0) {
                errs.push(format!(
                    "主环断裂：格 {}({},{}) → {}({},{}) 非法步长",
                    a.id, a.col, a.row, b.id, b.col, b.row
                ));
            }
        }

        // 道路格坐标不得重复
        let mut seen = std::collections::HashMap::new();
        for t in &self.tiles {
            if let Some(prev) = seen.insert((t.col, t.row), t.id) {
                errs.push(format!(
                    "道路格 {} 与 {} 坐标重复 ({},{})",
                    prev, t.id, t.col, t.row
                ));
            }
        }

        // property 格必带 land_id，且能解析；非 property 格不该带
        for t in &self.tiles {
            match (t.kind, t.land_id) {
                (TileKind::Property, None) => errs.push(format!("property 格 {} 缺 land_id", t.id)),
                (TileKind::Property, Some(id)) if self.land(id).is_none() => {
                    errs.push(format!("property 格 {} 的 land_id={} 不存在", t.id, id))
                }
                (kind, Some(id)) if kind != TileKind::Property => {
                    errs.push(format!("{kind:?} 格 {} 不应绑定土地 {id}", t.id))
                }
                _ => {}
            }
        }

        // 点券格：面额是它存在的理由，缺失/为 0 就是一颗踩上去什么都不发的格子；
        // 反过来，点券字段只能住在点券格上（与上面 `land_id` 那组同一条理由：
        // 数据写歪了要在加载期响，而不是让玩家用一局的时间去发现）。
        // 只看主环：死胡同格结构上进不了落格分派（`walk_path` 恒为主环下标）。
        for t in &self.tiles {
            if t.kind == TileKind::Coupon {
                if t.data.coupon.unwrap_or(0) == 0 {
                    errs.push(format!(
                        "coupon 格 {}({}) 缺 data.coupon 面额",
                        t.id, t.name
                    ));
                }
                if t.data.refresh == Some(0) {
                    errs.push(format!(
                        "coupon 格 {}({}) 的 refresh=0 等于每次踩都发，与「一次性」矛盾",
                        t.id, t.name
                    ));
                }
            } else if t.data.coupon.is_some() || t.data.refresh.is_some() {
                errs.push(format!(
                    "{:?} 格 {}({}) 不该带点券字段",
                    t.kind, t.id, t.name
                ));
            }
            // `game` 只能住在游乐场格上（与上面那组同一条理由：数据写歪了要在加载期响，
            // 而不是让玩家用一局的时间去发现「这一格踩上去什么都不发生」）。
            // 缺省是合法的 —— 那一格的意思是「这一趟随机一款」，见 `TileData::game`
            if t.kind != TileKind::Arcade && t.data.game.is_some() {
                errs.push(format!(
                    "{:?} 格 {}({}) 不该带 game 字段",
                    t.kind, t.id, t.name
                ));
            }
            // 公司格（P10d①）：股票绑定是它存在的理由，缺了就是一颗踩上去什么都没有的格；
            // 反过来 stock/fee 只能住在公司格上。B 型骰面为 0 与点券 refresh=0 同罪 ——
            // 转盘一个数都摇不出来。
            if t.kind == TileKind::Company {
                match t.data.stock.as_deref() {
                    None | Some("") => {
                        errs.push(format!("company 格 {}({}) 缺 data.stock", t.id, t.name))
                    }
                    _ => {}
                }
                if let Some(CompanyFee::RandomStay { sides, .. } | CompanyFee::Insure { sides, .. }) =
                    t.data.fee
                    && sides == 0
                {
                    errs.push(format!("company 格 {}({}) 的 B 型骰面数为 0", t.id, t.name));
                }
                // 保险两项都得是正数：保费 0 等于白送保单、赔付 0 等于一张废纸 ——
                // 数据写歪了要在加载期响，而不是让玩家用一局去发现保单是空的
                if let Some(CompanyFee::Insure {
                    premium_per_day,
                    indemnity_per_day,
                    ..
                }) = t.data.fee
                    && (premium_per_day <= 0 || indemnity_per_day <= 0)
                {
                    errs.push(format!(
                        "company 格 {}({}) 的保费/赔付单价必须是正数",
                        t.id, t.name
                    ));
                }
            } else if t.kind == TileKind::Shop {
                // P10d②：百货格也带 stock —— 董事长白拿一件的特权挂在股上，
                // 分红那一半（P10d 行 ④）本阶段不做，但身份要先有地方长出来
                match t.data.stock.as_deref() {
                    None | Some("") => {
                        errs.push(format!("shop 格 {}({}) 缺 data.stock", t.id, t.name))
                    }
                    _ => {}
                }
                if t.data.fee.is_some() {
                    errs.push(format!("shop 格 {}({}) 不该带 fee 字段", t.id, t.name));
                }
            } else if t.data.stock.is_some() || t.data.fee.is_some() {
                errs.push(format!(
                    "{:?} 格 {}({}) 不该带公司字段",
                    t.kind, t.id, t.name
                ));
            }
        }

        let mut land_cells = std::collections::HashSet::new();
        for l in &self.lands {
            if !(1..=2).contains(&l.size) {
                errs.push(format!(
                    "土地 {} 的 size={} 非法（应为 1 或 2）",
                    l.id, l.size
                ));
            }
            if l.rent.is_empty() {
                errs.push(format!("土地 {} 的 rent 表为空", l.id));
            }
            if l.col >= self.grid_cols || l.row >= self.grid_rows {
                errs.push(format!("土地 {} ({},{}) 越界", l.id, l.col, l.row));
            }

            // 1 对 1 住宅，2 对 2 商业共享
            let bound = self.tiles_bound_to_land(l.id);
            if bound.is_empty() {
                errs.push(format!("土地 {} 没有任何道路格绑定", l.id));
            } else if l.size == 2 && bound.len() != 2 {
                errs.push(format!(
                    "2x2 土地 {} 应被 2 个道路格共享，实际 {}",
                    l.id,
                    bound.len()
                ));
            } else if l.size == 1 && bound.len() != 1 {
                errs.push(format!(
                    "1x1 土地 {} 应只绑 1 个道路格，实际 {}",
                    l.id,
                    bound.len()
                ));
            }

            for dc in 0..l.size as u32 {
                for dr in 0..l.size as u32 {
                    let cell = (l.col + dc, l.row + dr);
                    if land_cells.insert(cell) {
                        if self.tiles.iter().any(|t| (t.col, t.row) == cell) {
                            errs.push(format!(
                                "土地 {} 与道路格重叠于 ({},{})",
                                l.id, cell.0, cell.1
                            ));
                        }
                    } else {
                        errs.push(format!("土地足迹重叠于 ({},{})", cell.0, cell.1));
                    }
                }
            }

            // 商业设施（P10b）：住宅/保险地不挂设施与收费；商业地必须 2x2；
            // fee 的形状必须与设施类别对得上（购物中心拿 PerStep 就是数据写歪），
            // A 型的档位表必须盖满骰面 —— 摇出 6 却没有第 6 档，玩家掷出的是
            // 「白踩」而不是「没建好」，那颗骰子就撒谎了
            match (l.land_type, l.building_kind, l.fee.as_ref()) {
                (LandType::Commercial, kind, fee) => {
                    if l.size != 2 {
                        errs.push(format!(
                            "商业地 {} 的 size={}，商业设施只能建在 2x2 上",
                            l.id, l.size
                        ));
                    }
                    match (kind, fee) {
                        (Some(BuildingKind::Mall), Some(FeeDef::RandomAmount { sides, tiers })) => {
                            if *sides == 0 || tiers.len() != *sides as usize {
                                errs.push(format!(
                                    "购物中心 {} 的档位表 {} 条盖不住骰面 {}（且面数 ≥1）",
                                    l.id,
                                    tiers.len(),
                                    sides
                                ));
                            }
                        }
                        (Some(BuildingKind::Hotel), Some(FeeDef::RandomStay { sides, .. })) => {
                            if *sides == 0 {
                                errs.push(format!("旅馆 {} 的骰面数为 0", l.id));
                            }
                        }
                        (Some(BuildingKind::GasStation), Some(FeeDef::PerStep { .. })) => {}
                        (Some(BuildingKind::Lab | BuildingKind::Park), None) => {}
                        (kind, fee) => errs.push(format!(
                            "土地 {} 的设施 {:?} 与收费 {:?} 不匹配（购物中心=A 档位、旅馆=B 按天、加油站=C 按步数、研究所/公园=无）",
                            l.id, kind, fee
                        )),
                    }
                }
                (_, Some(kind), _) => errs.push(format!(
                    "{:?} 地 {} 不该挂商业设施 {:?}",
                    l.land_type, l.id, kind
                )),
                (_, None, Some(fee)) => errs.push(format!(
                    "{:?} 地 {} 不该带收费参数 {:?}",
                    l.land_type, l.id, fee
                )),
                (_, None, None) => {}
            }
        }

        // 空商业地现建的默认参数表（P10c）：与上面那张矩阵同一套判据，只是对象
        // 换成棋盘级 —— 给购物中心配 B 型、给研究所配了费，与地块上写歪同罪
        for (kind, fee) in &self.facility_fees {
            let ok = match (kind, fee) {
                (BuildingKind::Mall, FeeDef::RandomAmount { sides, tiers }) => {
                    *sides > 0 && tiers.len() == *sides as usize
                }
                (BuildingKind::Hotel, FeeDef::RandomStay { sides, .. }) => *sides > 0,
                (BuildingKind::GasStation, FeeDef::PerStep { .. }) => true,
                _ => false,
            };
            if !ok {
                errs.push(format!(
                    "默认设施参数 {:?} 与收费 {:?} 不匹配（购物中心=A 档位、旅馆=B 按天、加油站=C 按步数；研究所/公园不需要条目）",
                    kind, fee
                ));
            }
        }

        // 分支：接入点在主环上、末端坐标合法、死胡同格不属于主环
        for j in self.tiles.iter().filter(|t| t.branch.is_some()) {
            let b = j.branch.as_ref().unwrap();
            if b.tiles.is_empty() {
                errs.push(format!("格 {} 的 {:?} 分支没有格子", j.id, b.kind));
            }
            let mut prev = (j.col, j.row);
            for bt in &b.tiles {
                if bt.col >= self.grid_cols || bt.row >= self.grid_rows {
                    errs.push(format!("{:?} 分支格 ({},{}) 越界", b.kind, bt.col, bt.row));
                }
                if self
                    .tiles
                    .iter()
                    .any(|t| (t.col, t.row) == (bt.col, bt.row))
                {
                    errs.push(format!("死胡同格 ({},{}) 出现在主环上", bt.col, bt.row));
                }
                let dc = (bt.col as i64 - prev.0 as i64).abs();
                let dr = (bt.row as i64 - prev.1 as i64).abs();
                if dc > 1 || dr > 1 {
                    errs.push(format!(
                        "分支不连通：({},{}) → ({},{})",
                        prev.0, prev.1, bt.col, bt.row
                    ));
                }
                prev = (bt.col, bt.row);
            }
        }

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROTOTYPE: &str = include_str!("../../../assets/data/boards/prototype.json");

    fn board() -> BoardDef {
        BoardDef::from_json(PROTOTYPE).expect("prototype.json 应能解析")
    }

    #[test]
    fn prototype_parses_and_validates() {
        let b = board();
        assert_eq!(b.board_name, "prototype");
        assert_eq!(b.loop_len(), 19);
        assert_eq!(b.lands.len(), 6);
        assert!(
            b.validate().is_ok(),
            "校验失败: {:?}",
            b.validate().unwrap_err()
        );
        // P10d①：两家公司的门面格 —— 门面绑的股票必须真的存在于股票表里
        let tech = b.tile(12).unwrap();
        assert_eq!(tech.kind, TileKind::Company);
        assert_eq!(tech.data.stock.as_deref(), Some("stock_001"));
        let airline = b.tile(14).unwrap();
        assert_eq!(airline.kind, TileKind::Company);
        assert_eq!(airline.data.stock.as_deref(), Some("stock_005"));
    }

    #[test]
    fn ring_wraps_both_directions() {
        let b = board();
        // P3d②/P7/P8 把百货、游乐场、彩票依次推到环尾（原 12/13/14 号）。
        // P10d① 的两家公司插在既有格的斜段中间（科技 (1,4)、航空 (0,3)）。
        // P10d② 的保险公司 (4,1) 与日产建设 (5,1) 插在汉口路与命运之间的直段上：
        // 2→3→4→5，既有格整体顺移两位（3–16 号变 5–18 号），0–2 号一个没动 ——
        // 与 P10d①「只顺移编号、不动坐标」同一个先例，代价是老格号集体 +2。
        assert_eq!(b.destination(2, 1), 3);
        assert_eq!(b.destination(3, 1), 4);
        assert_eq!(b.destination(4, 1), 5);
        assert_eq!(b.destination(11, 1), 12);
        assert_eq!(b.destination(12, 1), 13);
        assert_eq!(b.destination(13, 1), 14);
        assert_eq!(b.destination(14, 1), 15);
        assert_eq!(b.destination(15, 1), 16);
        assert_eq!(b.destination(16, 1), 17);
        assert_eq!(b.destination(17, 1), 18);
        assert_eq!(b.destination(18, 1), 0, "环尾下一步该回到起点");
        assert_eq!(b.destination(0, -1), 18);
        assert_eq!(b.destination(5, 19), 5, "整整一圈回到原地");
        assert_eq!(b.tile(-1).unwrap().id, 18);
        assert_eq!(b.tile(19).unwrap().id, 0);
        // wrapped() 与 tile() 必须同一套取模规则，否则棋子位置与看到的格子会错位
        assert_eq!(b.wrapped(-1), TileIndex(18));
        assert_eq!(b.wrapped(19), TileIndex(0));
    }

    /// 请神符（P6d）的「离自己最近」读的是这两句：先比双向最短，同距时比行进方向。
    #[test]
    fn ring_distance_counts_both_ways_while_the_heading_breaks_a_tie() {
        let b = board(); // 19 格
        assert_eq!(b.ring_distance(TileIndex(0), TileIndex(0)), 0);
        assert_eq!(b.ring_distance(TileIndex(0), TileIndex(6)), 6);
        assert_eq!(
            b.ring_distance(TileIndex(0), TileIndex(9)),
            9,
            "19 格环上远侧由 9 号与 10 号夹出：正向 9 步"
        );
        assert_eq!(
            b.ring_distance(TileIndex(0), TileIndex(16)),
            3,
            "跨过环尾也要连着数：0→18→17→16 倒着 3 步，不能在此断开各数一半"
        );
        // 同距的一对：19 格是**奇数**环，没有正对面那一格，远侧由两格夹出 ——
        // 9 号与 10 号离 0 号都是 9 步。正向取 9 号，掉头之后取 10 号。
        // 这一对是「决胜键真的有牙」的全部依据：只用其中一位的话，`Heading` 那一格
        // 永远测不到（另一个方向数出来是 10，本来就输）
        assert_eq!(
            b.steps_toward(TileIndex(0), TileIndex(9), Heading::Forward),
            9
        );
        assert_eq!(
            b.steps_toward(TileIndex(0), TileIndex(10), Heading::Forward),
            10
        );
        assert_eq!(
            b.steps_toward(TileIndex(0), TileIndex(10), Heading::Backward),
            9,
            "倒着走时 10 号才是前面那位"
        );
    }

    #[test]
    fn walk_path_wraps_past_the_start_tile() {
        let b = board();
        // 从环尾走 3 步：17（保险公司）→ 18（日产建设）→ 踏过 0 号起点 —— 「绕回起点经过」
        assert_eq!(
            b.walk_path(16, 3),
            vec![TileIndex(17), TileIndex(18), TileIndex(0)]
        );
        // 从倒数第一格走 2 步：先踏过起点再落到 1 号，回绕在第一步上
        assert_eq!(b.walk_path(18, 2), vec![TileIndex(0), TileIndex(1)]);
        assert_eq!(b.walk_path(0, 1), vec![TileIndex(1)]);
        assert!(b.walk_path(3, 0).is_empty(), "0 步不该有路径");
    }

    /// 转向卡（P3d④b）用掉了 `walk_path` 预留的那一格负数：倒着走还是同一条环，
    /// 回绕规则与正向共用同一个 `rem_euclid`。
    #[test]
    fn a_negative_step_count_walks_the_same_ring_backwards() {
        let b = board();
        assert_eq!(b.walk_path(3, -1), vec![TileIndex(2)]);
        assert_eq!(
            b.walk_path(5, -3),
            vec![TileIndex(4), TileIndex(3), TileIndex(2)],
            "倒着走是下标递减，不是把路径整个反过来念"
        );
        // 从 1 号倒着越过起点：0 → 18（日产建设）→ 17（保险公司）。回绕在负方向上同样成立，
        // 而这条路径正是「被转向卡送去环尾那几格」的那条路
        assert_eq!(
            b.walk_path(1, -3),
            vec![TileIndex(0), TileIndex(18), TileIndex(17)]
        );
        // 方向不改变走了几步：正负三步都是三格，而倒着走满一圈回到出发那一格
        assert_eq!(b.walk_path(2, -19).len(), 19);
        assert_eq!(b.walk_path(2, -19).last(), Some(&TileIndex(2)));
    }

    #[test]
    fn walk_path_never_dives_into_a_branch() {
        let b = board();
        // 接入点 2 挂监狱死胡同（见 branches_expose_junction_and_dead_end）。
        // 穿过接入点必须继续沿主环走，分支不顶掉任何一步 —— 路径长度恒等于步数，
        // 且全为主环下标（死胡同格没有 TileIndex，结构上进不来）。
        assert_eq!(b.branch_junction(BranchKind::Jail).unwrap().id, 2);
        assert_eq!(
            b.walk_path(1, 3),
            vec![TileIndex(2), TileIndex(3), TileIndex(4)]
        );
        assert_eq!(b.walk_path(2, 1), vec![TileIndex(3)]);
        // 整整一圈也恰好 17 步：分支不是环的一部分
        assert_eq!(b.walk_path(2, 17).len(), 17);
    }

    #[test]
    fn tile_at_grid_covers_branch_tiles_too() {
        let b = board();
        // 主环格：带下标
        let (index, tile) = b.tile_at_grid(b.tiles[5].col, b.tiles[5].row).unwrap();
        assert_eq!(index, Some(TileIndex(5)));
        assert_eq!(tile.id, 5);
        // 死胡同格（监狱末端）：找得到，但没有主环下标
        let end = b.branch_end(BranchKind::Jail).unwrap();
        let (index, found) = b.tile_at_grid(end.col, end.row).unwrap();
        assert_eq!(index, None, "分支格不在环上");
        assert_eq!(found.name, end.name);
        // 棋盘外的坐标：None，而不是回退到某一格
        assert!(b.tile_at_grid(999, 999).is_none());
    }

    #[test]
    fn diagonal_segments_count_is_three() {
        // 设计意图（ADR-001）：斜向街道要能渲染成对角路面
        let b = board();
        let diag: Vec<u32> = (0..b.loop_len())
            .filter(|i| b.is_diagonal_step(*i as i64))
            .map(|i| b.tiles[i as usize].id)
            .collect();
        // 2→3（汉口路→保险公司）那段在 P10d① 被补直（9→10 同理）；
        // 5→6（命运→外滩北）那一段是原盘就有的斜街；新长的两段是
        // 14→15（航空公司 (0,3) → 西单 (1,2)）与 15→16（西单 → 百货）。
        // P10d② 的日产建设 (5,1) 落在 4→5 那一跳上，把「百货→游乐场」还原成
        // 直段 —— 所以「追加一格」的代价不总是多一条斜段：这一串数读的是
        // 「现在这张盘上有几段斜街可画」，不是「加了几格」。
        assert_eq!(diag, vec![4, 14, 15], "非对角段被误判: {diag:?}");
    }

    /// 百货公司在演示盘上真有一格，而且踩得到（P3d② 验收第一条）。
    /// 与点券格同一条理由：机制阶段做出来的格子如果盘上没有载体，测试全绿也等于没演示过。
    #[test]
    fn prototype_carries_a_shop_the_player_can_step_on() {
        let b = board();
        let shops: Vec<u32> = b
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::Shop)
            .map(|t| t.id)
            .collect();
        assert_eq!(shops, vec![16], "演示盘该恰好一格百货");
        // 它关于一块地吗？不该 —— 百货是事件格，`validate` 也会替这条把关
        assert_eq!(b.tiles[16].land_id, None);
        assert_eq!(b.tiles[16].name, "百货公司");
    }

    /// 演示盘上那一格游乐场（P7）。`tests/minigame.rs` 钉着同一个编号，两处一起改才不会
    /// 悄悄错位 —— 而「机制在演示盘上真能踩到」是这一格存在的理由（16 百货掷 1 点即到，
    /// 15 西单掷 2 点即到）。
    ///
    /// **`data` 是空的，那是「这一趟随机一款」**：三种游戏在演示盘上只有一格的余量，
    /// 写死一款就等于另外两种只能靠 `NM_MINIGAME` 才看得见。真棋盘（P10）一格写死一款。
    #[test]
    fn prototype_carries_an_arcade_the_player_can_step_on() {
        let b = board();
        let arcades: Vec<u32> = b
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::Arcade)
            .map(|t| t.id)
            .collect();
        assert_eq!(arcades, vec![17], "演示盘该恰好一格游乐场");
        assert_eq!(b.tiles[17].land_id, None, "游乐场是事件格，不关于一块地");
        assert_eq!(b.tiles[17].name, "游乐场");
        assert_eq!(b.tiles[17].data.game, None, "缺省＝随机一款");
    }

    /// 公司格（P10d①）在演示盘上真有四格，且都绑着股票表里存在的 id。
    /// 「机制在演示盘上真能踩到」的同一条理由：科技公司插在王府井与点券箱之间的
    /// 斜段上，从 11 王府井掷 1 点即踩到；航空公司插在点券箱与西单之间的斜段上，
    /// 从 13 号掷 1 点即到。P10d② 再添两格：保险公司 (4,1)、日产建设 (5,1)
    /// 插在汉口路与命运之间的直段上 —— 老格号 3–16 集体顺移成 5–18。
    #[test]
    fn prototype_carries_two_companies_the_player_can_step_on() {
        let b = board();
        let companies: Vec<u32> = b
            .tiles
            .iter()
            .filter(|t| t.kind == TileKind::Company)
            .map(|t| t.id)
            .collect();
        assert_eq!(companies, vec![3, 4, 12, 14], "演示盘该恰好四格公司");
        // 科技公司：C 型维修费（与载具无关 —— 走路踩坏了也要修）
        let tech = &b.tiles[12];
        assert_eq!(tech.name, "科技公司");
        match tech.data.fee {
            Some(CompanyFee::PerStep {
                rate,
                requires_mount,
            }) => {
                assert!(rate > 0);
                assert!(!requires_mount, "电脑维修费不挑载具");
            }
            other => panic!("科技公司读成了 {other:?}"),
        }
        // 航空公司：B 型转盘，摇出几出国几天
        let airline = &b.tiles[14];
        assert_eq!(airline.name, "航空公司");
        match airline.data.fee {
            Some(CompanyFee::RandomStay { sides, per_day }) => {
                assert_eq!(sides, 6);
                assert!(per_day > 0);
            }
            other => panic!("航空公司读成了 {other:?}"),
        }
        // 保险公司（P10d②）：B 型变体 —— 转盘摇出的是投保天数，保费与赔付单价都必须是正数
        let insurer = &b.tiles[3];
        assert_eq!(insurer.name, "保险公司");
        match insurer.data.fee {
            Some(CompanyFee::Insure {
                sides,
                premium_per_day,
                indemnity_per_day,
            }) => {
                assert!(sides > 0);
                assert!(premium_per_day > 0);
                assert!(indemnity_per_day > 0);
            }
            other => panic!("保险公司读成了 {other:?}"),
        }
        // 日产建设（P10d②）：D 型功能格 —— 没有 fee，踩到不收费只开远程升级窗口
        let builder = &b.tiles[4];
        assert_eq!(builder.name, "日产建设");
        assert!(builder.data.fee.is_none(), "日产建设不该带 fee 字段");
        // 四格都不关于一块地：公司没有「地」可买
        assert_eq!(tech.land_id, None);
        assert_eq!(airline.land_id, None);
        assert_eq!(insurer.land_id, None);
        assert_eq!(builder.land_id, None);
        // 百货格（P10d②）绑着 stock_004：董事长白拿一件的身份挂在股上
        let shop = b.tile(16).unwrap();
        assert_eq!(shop.kind, TileKind::Shop);
        assert_eq!(shop.data.stock.as_deref(), Some("stock_004"));
        assert!(shop.data.fee.is_none());
    }

    /// `game` 只能住在游乐场格上，与点券字段那一组同一条不变式：数据写歪了要在加载期响，
    /// 而不是让玩家用一局的时间去发现「这一格踩上去什么都不发生」。
    #[test]
    fn a_game_field_on_any_other_tile_is_a_load_error() {
        let mut b = board();
        b.tiles[1].data.game = Some(crate::minigame::GameKind::PopBalloons);
        let errs = b.validate().unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("不该带 game 字段")),
            "地产格上写了 game 却没被拦下：{errs:?}"
        );

        let mut b = board();
        b.tiles[17].data.game = Some(crate::minigame::GameKind::PenguinDig);
        assert!(
            b.validate().is_ok(),
            "游乐场格上写死一款是合法的：{:?}",
            b.validate().err()
        );
    }

    #[test]
    fn world_pos_is_derived_from_grid() {
        let b = board();
        assert_eq!(b.world_pos(3, 5), Vec3::new(6.0, 0.0, 10.0));
        assert_eq!(b.tile_world_pos(6), Some(Vec3::new(10.0, 0.0, 4.0)));
    }

    #[test]
    fn two_by_two_land_centers_between_its_cells() {
        let b = board();
        // land 2 在 (6,2) size=2 → 足迹中心 (6.5, 2.5) * 2.0
        assert_eq!(b.land_world_pos(2), Some(Vec3::new(13.0, 0.0, 5.0)));
        assert_eq!(b.tiles_bound_to_land(2), vec![6, 7]);
    }

    #[test]
    fn branches_expose_junction_and_dead_end() {
        let b = board();
        assert_eq!(b.branch_junction(BranchKind::Jail).unwrap().id, 2);
        assert_eq!(b.branch_end(BranchKind::Jail).unwrap().name, "监狱");
        assert_eq!(b.branch_junction(BranchKind::Hospital).unwrap().id, 9);
        assert_eq!(b.branch_end(BranchKind::Hospital).unwrap().name, "医院");
        // 拘留天数与罚金**不在格子上**（P4④）：天数只来自卡面，保释不动现金。
        // 这条断言反过来钉住「谁把字段加回来谁就红」
        assert_eq!(
            b.branch_end(BranchKind::Jail).unwrap().data,
            TileData::default()
        );
        assert_eq!(
            b.branch_end(BranchKind::Hospital).unwrap().data,
            TileData::default()
        );
    }

    /// 点券的面额写在格子上，不在代码里：三档 10/30/50 是数据，改排布不必动 system。
    #[test]
    fn prototype_carries_a_coupon_box_face_value() {
        let b = board();
        let tile = &b.tiles[13];
        assert_eq!(tile.id, 13, "P10d② 之后点券箱顺移到 13 号");
        assert_eq!(tile.kind, TileKind::Coupon);
        assert_eq!(tile.data.coupon, Some(30));
        assert_eq!(
            tile.data.refresh, None,
            "冷却缺省走 `GameRules::coupon_refresh_turns`，不在格子上重复一份"
        );
    }

    #[test]
    fn land_lookup_through_tile() {
        let b = board();
        assert_eq!(b.land_for_tile(1).unwrap().id, 0);
        assert_eq!(b.land_for_tile(11).unwrap().id, 4);
        assert!(b.land_for_tile(0).is_none(), "起点格不应解析出土地");
        assert_eq!(b.land_for_tile(6).unwrap().id, 2, "外滩北");
        assert_eq!(b.land_for_tile(7).unwrap().id, 2, "外滩南与北共享 land 2");
    }

    #[test]
    fn validate_rejects_broken_boards() {
        type Breaker = fn(&mut BoardDef);
        let cases: Vec<(Breaker, &str)> = vec![
            (|b| b.tiles.truncate(1), "主环为空或断裂"),
            (
                |b| {
                    b.tiles.remove(0);
                },
                "删格后 id 与下标错位",
            ),
            (|b| b.tiles[3].col = 40, "越界"),
            (|b| b.tiles[1].land_id = None, "property 缺 land_id"),
            (|b| b.tiles[0].land_id = Some(0), "非 property 绑地"),
            (|b| b.tiles[9].land_id = Some(0), "1x1 土地被两个道路格共享"),
            (|b| b.lands[0].rent.clear(), "空租金表"),
            (|b| b.lands[0].row = b.tiles[1].row, "土地与道路格重叠"),
            (
                |b| b.tiles[2].branch.as_mut().unwrap().tiles.clear(),
                "空分支",
            ),
            (
                |b| {
                    b.tiles[2].branch.as_mut().unwrap().tiles[0].col = 7;
                    b.tiles[2].branch.as_mut().unwrap().tiles[0].row = 7;
                },
                "分支不连通",
            ),
            (|b| b.tiles[13].data.coupon = None, "点券格没有面额"),
            (
                |b| b.tiles[13].data.refresh = Some(0),
                "点券格的冷却写成 0（等于每次踩都发）",
            ),
            (
                |b| b.tiles[3].data.coupon = Some(50),
                "点券字段挂在别的格上",
            ),
            (|b| b.tiles[12].data.stock = None, "公司格没有绑股票"),
            (
                |b| b.tiles[1].data.stock = Some("stock_001".into()),
                "股票字段挂在别的格上",
            ),
            (
                |b| {
                    b.tiles[12].data.fee = Some(CompanyFee::RandomStay {
                        sides: 0,
                        per_day: 100,
                    })
                },
                "公司格 B 型骰面数为 0",
            ),
            (
                |b| {
                    b.lands[2].fee = Some(FeeDef::PerStep {
                        rate: 100,
                        requires_mount: true,
                    })
                },
                "购物中心配了 C 型单价（A 型要档位表）",
            ),
            (
                |b| {
                    b.lands[2].fee = Some(FeeDef::RandomAmount {
                        sides: 6,
                        tiers: vec![100, 200],
                    })
                },
                "档位表盖不住骰面（摇出 3 就是白踩）",
            ),
            (
                |b| b.lands[0].building_kind = Some(BuildingKind::Hotel),
                "住宅地挂商业设施",
            ),
            (
                |b| {
                    b.lands[2].fee = Some(FeeDef::RandomStay {
                        sides: 4,
                        per_day: 500,
                    });
                    b.lands[2].building_kind = Some(BuildingKind::Park);
                },
                "公园不需要收费参数却带了",
            ),
        ];

        for (break_it, label) in cases {
            let mut b = board();
            break_it(&mut b);
            assert!(b.validate().is_err(), "坏棋盘被放过了（{label}）");
        }
    }

    /// 演示盘外滩是预建购物中心：serde 那层 `{"random_amount": {...}}` 的外挂标签
    /// 形状、`rename_all` 的蛇形命名、档位表盖满骰面，三件事一起在这里钉住。
    #[test]
    fn prototype_mall_fee_parses_and_maps_to_the_right_prototype() {
        let b = board();
        let land = &b.lands[2];
        assert_eq!(land.building_kind, Some(BuildingKind::Mall));
        let proto = land
            .fee
            .as_ref()
            .expect("购物中心必须带收费参数")
            .to_prototype();
        use crate::service::FeePrototype;
        match proto {
            FeePrototype::RandomAmount { sides, tiers } => {
                assert_eq!(sides, 6);
                assert_eq!(tiers, vec![200, 400, 600, 800, 1200, 1600]);
                assert_eq!(tiers.len() as u32, sides, "摇出任何一点都有档可走");
            }
            other => panic!("购物中心读成了 {other:?}"),
        }
    }

    /// 默认设施参数表（P10c）：空商业地上现建的那套价。
    /// 解析 + 校验 + [`BoardDef::facility_fee`] 的两来源汇合，三件事各一条。
    #[test]
    fn prototype_default_facility_fees_parse_and_validate() {
        let b = board();
        assert_eq!(
            b.facility_fees.len(),
            3,
            "演示盘配了购物中心/旅馆/加油站三套默认价"
        );
        assert!(
            b.validate().is_ok(),
            "校验失败: {:?}",
            b.validate().unwrap_err()
        );
    }

    #[test]
    fn a_mismatched_default_fee_is_rejected() {
        let mut b = board();
        b.facility_fees.insert(
            BuildingKind::Mall,
            FeeDef::RandomStay {
                sides: 4,
                per_day: 500,
            },
        );
        assert!(
            b.validate().is_err(),
            "购物中心配 B 型按天计费被放过了（A 型要档位表）"
        );
        let mut b = board();
        b.facility_fees.insert(
            BuildingKind::Park,
            FeeDef::PerStep {
                rate: 1,
                requires_mount: false,
            },
        );
        assert!(
            b.validate().is_err(),
            "公园不需要参数却配了（它根本不产生账）"
        );
    }

    /// 预建设施用**地块自己的**参数，空地现建用棋盘默认表 ——
    /// 两个来源只在 `facility_fee` 这一处汇合，「同一座旅馆两套价」写不出来。
    #[test]
    fn facility_fee_takes_the_lot_own_price_before_the_board_default() {
        let mut b = board();
        // 外滩（L2）预建购物中心：读它自己的档位表
        let own = b
            .facility_fee(2, BuildingKind::Mall)
            .expect("预建购物中心有参数");
        assert!(matches!(own, FeeDef::RandomAmount { tiers, .. } if tiers[0] == 200));
        // 西单（L5）是住宅： facility_fee(5, Hotel) 落到默认表
        let fallback = b
            .facility_fee(5, BuildingKind::Hotel)
            .expect("默认表里有旅馆");
        assert!(matches!(fallback, FeeDef::RandomStay { per_day, .. } if *per_day == 500));
        // 没配的那一类（研究所）查不到 —— 建不出来，而不是建出一座不收钱的
        assert!(b.facility_fee(5, BuildingKind::Lab).is_none());
        // 把外滩的预建改成旅馆后，旅馆读的是**它自己那格**的参数：
        // 哪怕默认表里旅馆是另一个价
        b.lands[2].building_kind = Some(BuildingKind::Hotel);
        b.lands[2].fee = Some(FeeDef::RandomStay {
            sides: 4,
            per_day: 999,
        });
        let hotel = b
            .facility_fee(2, BuildingKind::Hotel)
            .expect("预建旅馆有参数");
        assert!(matches!(hotel, FeeDef::RandomStay { per_day, .. } if *per_day == 999));
    }
}
