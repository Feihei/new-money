//! 主环按**等步长折线**铺开，而不是「整数格 × cell_size」。
//!
//! 网格只留作**方向**与不变式判据（`validate`、生成器还在网格上算），
//! 位置是「前一格 + cell_size × 该段方向」。于是斜街两格的中心距与直街一样是 S：
//! 同样大小的方块绕自身 Y 转 45° 之后**边对边咬合成一条连续街带**，地块等大且等距 ——
//! 原作（底图是画出来的）就是这个形状。
//!
//! 这里**只推导、不画**：画的部分在 [`board`] 与 [`land`]，它们拿 [`BoardGeom::at`] /
//! [`BoardGeom::land`] 而不是 `BoardDef::world_pos`。

use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*};
use game_core::prelude::{ActiveBoard, BoardDef, LandDef, TileDef};

use super::yaw_to_face;

/// 一格在世界里的姿态。`yaw` 是绕世界 Y 的弧度，口径与 [`yaw_to_face`] 相同：
/// 躺平那张方图的局部 +X 转到这一位上。
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frame {
    pub at: Vec3,
    pub yaw: f32,
    /// 这一格的两条臂不同向（转角）。它的方图按角平分线转，与前后两格都**部分重叠**，
    /// 同层就是 z 打架 —— 装配那一层靠这一位把它抬高一档，让缝看得见而不是变雪花。
    pub bend: bool,
}

/// 一块地的姿态：中心、朝向、足迹边长（世界单位）。
#[derive(Clone, Copy, Debug)]
pub(crate) struct LandFrame {
    pub at: Vec3,
    pub yaw: f32,
    pub side: f32,
}

/// 底层那张草地：一张网，每格 UV 各自 0..1，所以整块只花一次 draw。
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Plate {
    pub at: Vec3,
    pub cols: u32,
    pub rows: u32,
}

/// 命中点：角色可能站的那一格与它的世界坐标。
#[derive(Clone, Copy, Debug)]
pub(crate) struct PickPoint {
    pub cell: (u32, u32),
    pub at: Vec3,
}

#[derive(Resource, Debug, Default)]
pub(crate) struct BoardGeom {
    /// 这份几何是为哪张盘推的。换盘（含回菜单再开一局）才重算。
    board: String,
    cell: f32,
    /// 主环格，下标 = `TileIndex`。
    ring: Vec<Frame>,
    /// 主环格与支线格的网格坐标 → 姿态。环外的格子**不在表里**，它们照旧落在网格上。
    by_cell: HashMap<(u32, u32), Frame>,
    lands: HashMap<u32, LandFrame>,
    pub plate: Plate,
    /// 角色可能站的全部坐标点（主环 + 支线）。点击命中走这张表，不给每格铺一块不可见地皮。
    pub picks: Vec<PickPoint>,
    /// 环尾再走一步应当正好回到第一格（两张真盘都是 0.000）。非零就是数据里的环不是
    /// 「等步长闭合」的，那道缝会在棋盘上留一个缺口 —— `tests` 里钉着这一条。
    pub closure_gap: f32,
    /// 与网格坐标（`BoardDef::world_pos`）偏离超过半格的地块数。整条环的形状变了
    /// （斜街那段变短），所以这个数**不是**「只有斜街的地块挪了」，而是「绝对位置挪了多少块」——
    /// 直街上的地块虽然相对自己那条街分毫不差，绝对坐标照样跟着整条环走。
    pub shifted_lands: usize,
    /// 外法线判据读不出侧向的地块数（`off · perp ≈ 0`）。非零就说明数据里有一块地
    /// 既不在街左也不在街右，那一块的位置是猜的。
    pub side_ambiguous: usize,
    /// 同一条街上互相压住的地块对数。咬合成立时必须是 0；一旦 `outward` 的深浅算错，
    /// 相邻两块就会叠起来，而画面上的症状只是「某条街少了一块地」。
    pub overlapping_lands: usize,
}

impl BoardGeom {
    /// 格心的姿态。环上的格被重排过，其余格子照旧落在网格上。
    pub(crate) fn at(&self, col: u32, row: u32) -> Frame {
        *self.by_cell.get(&(col, row)).unwrap_or(&Frame {
            at: grid(self.cell, col, row),
            yaw: 0.0,
            bend: false,
        })
    }

    pub(crate) fn tile(&self, index: usize) -> Option<Frame> {
        self.ring.get(index).copied()
    }

    #[cfg(test)]
    pub(crate) fn ring_len(&self) -> usize {
        self.ring.len()
    }

    pub(crate) fn land(&self, id: u32) -> Option<LandFrame> {
        self.lands.get(&id).copied()
    }

    /// 点中了哪一格：离 `at` 最近、且落在 `0.75 格` 之内的那个命中点。
    ///
    /// 超出半径返回 `None` —— 点在环心或盘外就什么都不选，而不是吸到最近的那条街上。
    pub(crate) fn nearest_cell(&self, at: Vec3) -> Option<(u32, u32)> {
        let limit = self.cell * 0.75;
        self.picks
            .iter()
            .map(|p| (p.cell, Vec2::new(p.at.x - at.x, p.at.z - at.z).length()))
            .filter(|(_, d)| *d <= limit)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(cell, _)| cell)
    }
}

fn grid(cell: f32, col: u32, row: u32) -> Vec3 {
    Vec3::new(col as f32 * cell, 0.0, row as f32 * cell)
}

/// 网格差 → 八方向单位向量（`BoardDef::world_pos` 是 `(col, 0, row)`，所以 N 是 `-row`）。
fn step(from: &TileDef, to: &TileDef) -> Vec3 {
    Vec3::new(
        to.col as f32 - from.col as f32,
        0.0,
        to.row as f32 - from.row as f32,
    )
    .normalize_or_zero()
}

/// 棋盘的两份读数一起取：`ActiveBoard` 给拓扑与尺寸，[`BoardGeom`] 给这一版几何下的位置与朝向。
///
/// 收成参数包不是为了好看：`attach_land_visuals` 已经顶在 clippy 的 7 个参数上限上，
/// 再加一个就把「参数太多了」这个真信号吃掉（§六 坑 8，与 [`super::GroundArt`] 同一条房规）。
#[derive(SystemParam)]
pub(crate) struct BoardRead<'w> {
    pub active: Res<'w, ActiveBoard>,
    pub geom: Res<'w, BoardGeom>,
}

/// 排在 [`super::RenderSet::Boot`] 与 [`super::RenderSet::Live`] **之前**：两者都要读这份几何。
///
/// 两个反直觉的地方，都是 Bevy 0.20 的调度事实：
/// - **不用 `Commands`**。`.chain()` 只保证「不并行」，不在链的节点之间插 flush，所以
///   `insert_resource` 要等到帧末才生效 —— 同帧的 Boot 与 Live 全都扑空，症状是九条
///   「Parameter `Res<BoardGeom>` failed validation」。这里改成 `ResMut` 就地写。
/// - **不挂集合级守门条件**。`resource_added::<ActiveBoard>` 消费 change tick，同一个条件
///   挂到两个集合上时，先求值的那一个就把那一帧的信号吃掉了，后一个集合整局都不跑。
pub(crate) fn build_board_geom(mut geom: ResMut<BoardGeom>, active: Option<Res<ActiveBoard>>) {
    let Some(active) = active else {
        return;
    };
    // 换盘才重算（回菜单再开一局必然带着盘名回来）；同盘重开不必推第二遍
    if geom.board == active.def.board_name {
        return;
    }
    *geom = build(&active.def);
    info!(
        "棋盘几何（{}）：环 {} 格 / 地块 {} 块 / 闭合缝 {:.3} / 挪位地块 {} / 侧向读不出 {} / 压叠对数 {}",
        active.def.board_name,
        geom.ring.len(),
        geom.lands.len(),
        geom.closure_gap,
        geom.shifted_lands,
        geom.side_ambiguous,
        geom.overlapping_lands
    );
}

pub(crate) fn build(b: &BoardDef) -> BoardGeom {
    let n = b.tiles.len();
    let mut g = BoardGeom {
        board: b.board_name.clone(),
        cell: b.cell_size,
        ring: Vec::with_capacity(n),
        by_cell: HashMap::new(),
        lands: HashMap::new(),
        plate: Plate {
            at: Vec3::ZERO,
            cols: b.grid_cols,
            rows: b.grid_rows,
        },
        picks: Vec::new(),
        closure_gap: 0.0,
        shifted_lands: 0,
        side_ambiguous: 0,
        overlapping_lands: 0,
    };
    if n == 0 {
        return g;
    }

    // 每一格的「来向」就是上一格的「去向」；环尾那一段的来向由首尾两格算，不读累加结果
    let mut dir_in: Vec<Vec3> = Vec::with_capacity(n);
    for i in 0..n {
        let prev = if i == 0 { n - 1 } else { i - 1 };
        dir_in.push(step(&b.tiles[prev], &b.tiles[i]));
    }

    // 位置：从第一格起按 `cell_size` 累加
    let mut pos: Vec<Vec3> = Vec::with_capacity(n);
    for (i, t) in b.tiles.iter().enumerate() {
        let cur = if i == 0 {
            grid(b.cell_size, t.col, t.row)
        } else {
            pos[i - 1] + dir_in[i] * b.cell_size
        };
        pos.push(cur);
    }
    // 折线没有闭合约束：环尾再走一步回不到第一格，那道缝就是这张盘的读数
    let last = pos[n - 1] + dir_in[0] * b.cell_size;
    g.closure_gap = last.distance(pos[0]);

    // 转角格两条臂差 45°，取角平分线：两侧各让开一半，比全让给一侧诚实
    for (i, &in_dir) in dir_in.iter().enumerate() {
        let out = dir_in[(i + 1) % n];
        let bis = in_dir + out;
        let d = if bis.length_squared() < 1e-6 {
            out
        } else {
            bis.normalize()
        };
        let frame = Frame {
            at: pos[i],
            yaw: yaw_to_face(d),
            bend: in_dir != out,
        };
        g.by_cell.insert((b.tiles[i].col, b.tiles[i].row), frame);
        g.ring.push(frame);
    }

    g.plate.at = grid(
        b.cell_size,
        b.grid_cols.saturating_sub(1),
        b.grid_rows.saturating_sub(1),
    ) * 0.5;

    // 角色可能站的位置：主环每格 + 支线每格。点击命中走这张表，不给格子铺不可见地皮
    for (i, t) in b.tiles.iter().enumerate() {
        g.picks.push(PickPoint {
            cell: (t.col, t.row),
            at: pos[i],
        });
        let Some(branch) = &t.branch else { continue };
        for bt in &branch.tiles {
            let at = g.at(bt.col, bt.row).at;
            g.picks.push(PickPoint {
                cell: (bt.col, bt.row),
                at,
            });
        }
    }

    // 支线：从接入点沿网格那一侧一路走出去，步长同样统一成 `cell_size`
    for (i, t) in b.tiles.iter().enumerate() {
        let Some(branch) = &t.branch else { continue };
        let mut prev_def = t;
        let mut prev_at = pos[i];
        for bt in &branch.tiles {
            let d = step(prev_def, bt);
            let at = prev_at + d * b.cell_size;
            g.by_cell.insert(
                (bt.col, bt.row),
                Frame {
                    at,
                    yaw: yaw_to_face(d),
                    bend: false,
                },
            );
            prev_def = bt;
            prev_at = at;
        }
    }

    let mut placed: Vec<(u32, LandFrame)> = Vec::with_capacity(b.lands.len());
    for l in &b.lands {
        let (f, ambiguous) = land_frame(b, &g.ring, &dir_in, l);
        g.side_ambiguous += usize::from(ambiguous);
        if let Some(old) = b.land_world_pos(l.id)
            && Vec2::new(f.at.x - old.x, f.at.z - old.z).length() > b.cell_size * 0.5
        {
            g.shifted_lands += 1;
        }
        g.lands.insert(l.id, f);
        placed.push((l.id, f));
    }
    // 同朝向的地块两两查一次压叠：共边相邻的两块在式子里正好取到等号（局部偏移 = 边长），
    // 所以用严格小于而不是小于等于。
    for (i, (id_a, a)) in placed.iter().enumerate() {
        for (_, other) in placed.iter().skip(i + 1) {
            if (a.yaw - other.yaw).abs() > 1e-3 {
                continue;
            }
            let (dx, dz) = (other.at.x - a.at.x, other.at.z - a.at.z);
            let (c, s) = (a.yaw.cos(), a.yaw.sin());
            let (lx, lz) = (dx * c - dz * s, dx * s + dz * c);
            if lx.abs() < a.side - 1e-3 && lz.abs() < a.side - 1e-3 {
                g.overlapping_lands += 1;
                warn!(
                    "地块 {id_a} 与邻块在同一条街上压住了：局部偏移 ({lx:.2}, {lz:.2})，边长 {:.2}",
                    a.side
                );
            }
        }
    }
    g
}

/// 地块的位置由**它绑的那些道路格**推出来：中心 = 绑定格的中心，往外退「半条街 + 半个足迹」。
///
/// 朝外那一侧不靠猜：拿足迹中心与绑定格在网格上的差向量点一下街面的法线，符号就定了。
/// 于是「土地摆在环路外侧」这条生成器的硬约束在这里是**读出来的**，不是又写一遍。
fn land_frame(b: &BoardDef, ring: &[Frame], dir_in: &[Vec3], l: &LandDef) -> (LandFrame, bool) {
    let size = f32::from(l.size.max(1));
    let side = b.cell_size * size;
    let bound: Vec<usize> = b
        .tiles
        .iter()
        .enumerate()
        .filter(|(_, t)| t.land_id == Some(l.id))
        .map(|(i, _)| i)
        .collect();
    // 「没有道路格绑着它」是 `validate` 不该放过的那种数据，那就留在网格上，不替它猜一条街。
    if bound.is_empty() {
        return (
            LandFrame {
                at: b
                    .land_world_pos(l.id)
                    .unwrap_or_else(|| grid(b.cell_size, l.col, l.row)),
                yaw: 0.0,
                side,
            },
            false,
        );
    }

    let anchor = bound
        .iter()
        .map(|i| ring[*i].at)
        .fold(Vec3::ZERO, |a, p| a + p)
        / bound.len() as f32;
    let run = bound
        .iter()
        .map(|i| {
            let out = dir_in[(*i + 1) % ring.len()];
            dir_in[*i] + out
        })
        .fold(Vec3::ZERO, |a, d| a + d)
        .normalize_or_zero();
    // 足迹中心与绑定格心在网格上的差：它落在法线的哪一侧，地就在哪一侧
    let anchor_cell = bound
        .iter()
        .map(|i| {
            let t = &b.tiles[*i];
            Vec2::new(t.col as f32, t.row as f32)
        })
        .fold(Vec2::ZERO, |a, c| a + c)
        / bound.len() as f32;
    let foot_cell = Vec2::new(
        l.col as f32 + (size - 1.0) / 2.0,
        l.row as f32 + (size - 1.0) / 2.0,
    );
    let off = Vec3::new(
        foot_cell.x - anchor_cell.x,
        0.0,
        foot_cell.y - anchor_cell.y,
    );
    let perp = Vec3::new(run.z, 0.0, -run.x);
    // 点积读不出侧向（数据把地摆在街的正上或正下，或 `run` 自己退化成零向量）：
    // 这一块的位置就是猜的，让它数出来而不是静默挑一边
    let side_dot = off.dot(perp);
    let outward = if side_dot >= 0.0 { perp } else { -perp };
    (
        LandFrame {
            at: anchor + outward * (b.cell_size * 0.5 + side * 0.5),
            yaw: yaw_to_face(run),
            side,
        },
        side_dot.abs() < 0.1,
    )
}
