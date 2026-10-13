//! 棋盘定义：ADR-001「网格路网 + 单向主环 + 事件死胡同分支」的 serde 落地。
//!
//! 只描述静态定义（`BoardDef`），运行时状态（归属/等级/抵押）是 Land 实体上的组件，
//! 因此 JSON 里的 `owner`/`level` 字段刻意不出现在本模块——静态与运行时不可混。

use bevy::math::Vec3;
use serde::Deserialize;

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
}

impl TileKind {
    /// 会把玩家关起来的格子类型（对应 Godot 端 `TileType.is_detention_tile`）。
    pub fn is_detention(self) -> bool {
        matches!(self, Self::Jail | Self::Hospital)
    }

    pub fn is_card(self) -> bool {
        matches!(self, Self::Chance | Self::Fate)
    }
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

/// 格子附加数据。当前只有拘留格用得到，其余格为 `{}`。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
pub struct TileData {
    #[serde(default)]
    pub turns: Option<u32>,
    #[serde(default)]
    pub fine: Option<i64>,
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

#[derive(Debug, Clone, PartialEq, Deserialize)]
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
        assert_eq!(b.loop_len(), 12);
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
        assert_eq!(b.destination(11, 1), 0);
        assert_eq!(b.destination(0, -1), 11);
        assert_eq!(b.destination(5, 12), 5);
        assert_eq!(b.tile(-1).unwrap().id, 11);
        assert_eq!(b.tile(12).unwrap().id, 0);
    }

    #[test]
    fn diagonal_segments_count_is_two() {
        // 设计意图（ADR-001）：斜向街道要能渲染成对角路面
        let b = board();
        let diag: Vec<u32> = (0..b.loop_len())
            .filter(|i| b.is_diagonal_step(*i as i64))
            .map(|i| b.tiles[i as usize].id)
            .collect();
        assert_eq!(diag, vec![2, 9], "非对角段被误判: {diag:?}");
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
        assert_eq!(b.branch_end(BranchKind::Jail).unwrap().data.turns, Some(3));
        assert_eq!(b.branch_end(BranchKind::Hospital).unwrap().name, "医院");
        assert_eq!(
            b.branch_end(BranchKind::Hospital).unwrap().data.fine,
            Some(300)
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
        ];

        for (break_it, label) in cases {
            let mut b = board();
            break_it(&mut b);
            assert!(b.validate().is_err(), "坏棋盘被放过了（{label}）");
        }
    }
}
