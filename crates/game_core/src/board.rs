//! 棋盘定义：ADR-001「网格路网 + 单向主环 + 事件死胡同分支」的 serde 落地。
//!
//! 只描述静态定义（`BoardDef`），运行时状态（归属/等级/抵押）是 Land 实体上的组件，
//! 因此 JSON 里的 `owner`/`level` 字段刻意不出现在本模块——静态与运行时不可混。

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

/// 格子附加数据。点券格用 `coupon`/`refresh`，游乐场用 `game`，其余格为 `{}`。
///
/// 拘留格**不带天数与罚金**：天数只来自卡面（P4④ 拍板），`turns`/`fine` 那份
/// Godot 残留数据已删 —— 留着就是第二份真相。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
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
        assert_eq!(b.loop_len(), 14);
        assert_eq!(b.lands.len(), 6);
        assert!(
            b.validate().is_ok(),
            "校验失败: {:?}",
            b.validate().unwrap_err()
        );
    }

    #[test]
    fn ring_wraps_both_directions() {
        let b = board();
        // 12 号（百货公司）与 13 号（游乐场）是 P3d②/P7 先后追加在环尾的两格：
        // 都不改任何既有格的编号，只把「回绕」那一跳一路往后挪（11→0 变 12→0 再变 13→0）。
        assert_eq!(b.destination(11, 1), 12);
        assert_eq!(b.destination(12, 1), 13);
        assert_eq!(b.destination(13, 1), 0, "环尾下一步该回到起点");
        assert_eq!(b.destination(0, -1), 13);
        assert_eq!(b.destination(5, 14), 5, "整整一圈回到原地");
        assert_eq!(b.tile(-1).unwrap().id, 13);
        assert_eq!(b.tile(14).unwrap().id, 0);
        // wrapped() 与 tile() 必须同一套取模规则，否则棋子位置与看到的格子会错位
        assert_eq!(b.wrapped(-1), TileIndex(13));
        assert_eq!(b.wrapped(14), TileIndex(0));
    }

    /// 请神符（P6d）的「离自己最近」读的是这两句：先比双向最短，同距时比行进方向。
    #[test]
    fn ring_distance_counts_both_ways_while_the_heading_breaks_a_tie() {
        let b = board(); // 14 格
        assert_eq!(b.ring_distance(TileIndex(0), TileIndex(0)), 0);
        assert_eq!(b.ring_distance(TileIndex(0), TileIndex(6)), 6);
        assert_eq!(
            b.ring_distance(TileIndex(0), TileIndex(8)),
            6,
            "顺着数 8 步的那一边，倒着数只有 6 步 —— 最近是 6"
        );
        assert_eq!(
            b.ring_distance(TileIndex(0), TileIndex(7)),
            7,
            "偶数格环上正对面那一格两个方向一样远"
        );
        assert_eq!(
            b.ring_distance(TileIndex(11), TileIndex(1)),
            4,
            "跨过环尾与起点也要连着数，不能在此断开各数一半"
        );
        // 同距的一对（3 号与 11 号离 0 号都是 3 步）：正向取前面那位，掉头之后取另一位。
        // 这一对是「决胜键真的有牙」的全部依据 —— 14 格环上 7 号是唯一与自己同距的格，
        // 两个方向都数出 7，用它当用例的话 `Heading` 那一格永远测不到
        assert_eq!(
            b.steps_toward(TileIndex(0), TileIndex(3), Heading::Forward),
            3
        );
        assert_eq!(
            b.steps_toward(TileIndex(0), TileIndex(11), Heading::Forward),
            11
        );
        assert_eq!(
            b.steps_toward(TileIndex(0), TileIndex(11), Heading::Backward),
            3,
            "倒着走时 11 号才是前面那位"
        );
    }

    #[test]
    fn walk_path_wraps_past_the_start_tile() {
        let b = board();
        // 从环尾走 2 步：先踏过 0 号起点再落到 1 号 —— 「绕回起点经过」
        assert_eq!(b.walk_path(13, 2), vec![TileIndex(0), TileIndex(1)]);
        // 从倒数第二格走 2 步：先经过游乐场再踏过起点，回绕在第二步上
        assert_eq!(b.walk_path(12, 2), vec![TileIndex(13), TileIndex(0)]);
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
        // 从 1 号倒着越过起点：0 → 13（游乐场）→ 12（百货）。回绕在负方向上同样成立，
        // 而这条路径正是「被转向卡送去环尾那两格」的那条路
        assert_eq!(
            b.walk_path(1, -3),
            vec![TileIndex(0), TileIndex(13), TileIndex(12)]
        );
        // 方向不改变走了几步：正负三步都是三格，而倒着走满一圈回到出发那一格
        assert_eq!(b.walk_path(2, -14).len(), 14);
        assert_eq!(b.walk_path(2, -14).last(), Some(&TileIndex(2)));
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
        // 整整一圈也恰好 14 步：分支不是环的一部分
        assert_eq!(b.walk_path(2, 14).len(), 14);
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
    fn diagonal_segments_count_is_four() {
        // 设计意图（ADR-001）：斜向街道要能渲染成对角路面
        let b = board();
        let diag: Vec<u32> = (0..b.loop_len())
            .filter(|i| b.is_diagonal_step(*i as i64))
            .map(|i| b.tiles[i as usize].id)
            .collect();
        // 2→3（汉口路→命运）与 9→10（王府井→点券箱）是原盘就有的两段；
        // 11→12 是 P3d② 追加百货格时新长的第三段 —— 那一格贴在左上角 (0,1)，
        // 从西单 (1,2) 只能斜着过去；13→0 是 P7 追加游乐场格时新长的第四段 ——
        // 它贴在角上 (0,0)，回起点 (1,1) 同样只能斜着走。
        // 对角段只被「报告」不被禁止（validate 允许 Δ≤1），所以这一串数的意义是：
        // 路面渲染与斜段特性仍然有真东西可画，而「追加一格」的代价一直是「多一条斜段」。
        assert_eq!(diag, vec![2, 9, 11, 13], "非对角段被误判: {diag:?}");
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
        assert_eq!(shops, vec![12], "演示盘该恰好一格百货");
        // 它关于一块地吗？不该 —— 百货是事件格，`validate` 也会替这条把关
        assert_eq!(b.tiles[12].land_id, None);
        assert_eq!(b.tiles[12].name, "百货公司");
    }

    /// 演示盘上那一格游乐场（P7）。`tests/minigame.rs` 钉着同一个编号，两处一起改才不会
    /// 悄悄错位 —— 而「机制在演示盘上真能踩到」是这一格存在的理由（12 百货掷 1 点即到，
    /// 11 西单掷 2 点即到）。
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
        assert_eq!(arcades, vec![13], "演示盘该恰好一格游乐场");
        assert_eq!(b.tiles[13].land_id, None, "游乐场是事件格，不关于一块地");
        assert_eq!(b.tiles[13].name, "游乐场");
        assert_eq!(b.tiles[13].data.game, None, "缺省＝随机一款");
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
        b.tiles[13].data.game = Some(crate::minigame::GameKind::PenguinDig);
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
        assert_eq!(b.tile_world_pos(4), Some(Vec3::new(10.0, 0.0, 4.0)));
    }

    #[test]
    fn two_by_two_land_centers_between_its_cells() {
        let b = board();
        // land 2 在 (6,2) size=2 → 足迹中心 (6.5, 2.5) * 2.0
        assert_eq!(b.land_world_pos(2), Some(Vec3::new(13.0, 0.0, 5.0)));
        assert_eq!(b.tiles_bound_to_land(2), vec![4, 5]);
    }

    #[test]
    fn branches_expose_junction_and_dead_end() {
        let b = board();
        assert_eq!(b.branch_junction(BranchKind::Jail).unwrap().id, 2);
        assert_eq!(b.branch_end(BranchKind::Jail).unwrap().name, "监狱");
        assert_eq!(b.branch_junction(BranchKind::Hospital).unwrap().id, 7);
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
        let tile = &b.tiles[10];
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
        assert_eq!(b.land_for_tile(9).unwrap().id, 4);
        assert!(b.land_for_tile(0).is_none(), "起点格不应解析出土地");
        assert_eq!(b.land_for_tile(4).unwrap().id, 2, "外滩北");
        assert_eq!(b.land_for_tile(5).unwrap().id, 2, "外滩南与北共享 land 2");
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
            (|b| b.tiles[10].data.coupon = None, "点券格没有面额"),
            (
                |b| b.tiles[10].data.refresh = Some(0),
                "点券格的冷却写成 0（等于每次踩都发）",
            ),
            (
                |b| b.tiles[3].data.coupon = Some(50),
                "点券字段挂在别的格上",
            ),
        ];

        for (break_it, label) in cases {
            let mut b = board();
            break_it(&mut b);
            assert!(b.validate().is_err(), "坏棋盘被放过了（{label}）");
        }
    }
}
