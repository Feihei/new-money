//! 棋子视觉：一颗把逻辑位置插值出来的球。对应 Godot `player_token.gd`。
//!
//! §4.6 的红线在这一层最要紧：core 在**掷骰那一帧**就把 `TokenPosition` 落定到终点，
//! 这里读 `TileStepped` 把同一次移动展开成逐格动画。于是渲染既不等待逻辑，
//! 也不被逻辑等待 —— Godot 那三处 `await token.arrived` 的形态在这里没有落点。

use std::f32::consts::PI;

use bevy::{ecs::system::SystemParam, prelude::*};
use game_core::prelude::{
    ActiveBoard, BoardDef, Detention, DetentionLifted, GameRules, PlayerBailed, PlayerColor,
    PlayerDetained, PlayerId, PlayerTeleported, TileIndex, TileStepped, TokenPosition, Villain,
    VillainKind,
};

use super::{Rendered, dim, flat};

/// 一颗视觉棋子。`pub` 是给相机模块的：rig 要跟的就是它的位置。
#[derive(Component)]
pub struct TokenVisual {
    /// 对应的玩家实体 —— `TurnOrder` 与所有消息都用它指代玩家
    pub player: Entity,
    /// 同格多人时的分角偏移（只含 XZ），见 [`slot_offset`]
    slot: Vec3,
}

/// 本段视觉位移：`from → to`，`t` 走到 `dur` 即到位。
#[derive(Component)]
pub(crate) struct Motion {
    from: Vec3,
    to: Vec3,
    t: f32,
    dur: f32,
}

/// 一格站得下四个人：按 `PlayerId` 分角，各偏 0.3。
/// Godot 端所有棋子都摆在格心（`board.gd:426` 的 `set_tile_index(0)`），开局两颗球完全重合。
///
/// 后四个角是**恶人**的（P12）：内圈留给四名玩家，外圈给四位恶人 —— 让他们与玩家抢同
/// 四个角，「强盗站在阿土伯那一格」在画面上就成了一颗球。八个位置都是同一条规则：
/// 同一格里谁都不遮住谁。
fn slot_offset(slot: u32) -> Vec3 {
    const OFFSETS: [Vec2; 8] = [
        Vec2::new(-0.3, -0.3),
        Vec2::new(0.3, -0.3),
        Vec2::new(-0.3, 0.3),
        Vec2::new(0.3, 0.3),
        Vec2::new(-0.62, -0.62),
        Vec2::new(0.62, -0.62),
        Vec2::new(-0.62, 0.62),
        Vec2::new(0.62, 0.62),
    ];
    let o = OFFSETS[(slot as usize) % OFFSETS.len()];
    Vec3::new(o.x, 0.0, o.y)
}

/// 恶人的归属色。**颜色是表现层的事**，所以这份映射在这里而不是 `VillainKind::label` 旁边
/// （规则层的 `default_app` 特性子集连 `bevy_color` 都不链接）。
/// 四位各占一个一眼分得出的色相，而「流氓」那一位刻意是脏橄榄色：他勒索的是地主自己。
fn villain_color(kind: VillainKind) -> Color {
    match kind {
        VillainKind::Robber => Color::srgb(0.72, 0.16, 0.16),
        VillainKind::Thief => Color::srgb(0.34, 0.22, 0.52),
        VillainKind::Hoodlum => Color::srgb(0.45, 0.44, 0.16),
        VillainKind::Spy => Color::srgb(0.18, 0.36, 0.46),
    }
}

/// 这颗棋子该停在世界的哪一格。**被关着的人停在死胡同尽头**，不在接入点：
/// `TokenPosition` 说的那一格是「放出来之后从哪格继续走」的逻辑读数（core 拥有位置），
/// 而眼睛要看的是他在牢里。开局那四位笼中的恶人就靠这一句站到牢房里，
/// 而不是站在汉口路的门口 —— 后者看起来像「四个恶人在路上等着抢我」。
fn rest_tile(b: &BoardDef, pos: TileIndex, held: Option<&Detention>) -> Option<Vec3> {
    if let Some(end) = held
        .and_then(|d| d.place.branch())
        .and_then(|kind| b.branch_end(kind))
    {
        return Some(b.world_pos(end.col, end.row));
    }
    b.tile_world_pos(pos.0 as i64)
}

pub(crate) fn spawn_token_visuals(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    players: Query<(
        Entity,
        &PlayerId,
        &PlayerColor,
        &TokenPosition,
        Option<&Detention>,
    )>,
    villains: Query<(Entity, &Villain, &TokenPosition, Option<&Detention>)>,
) {
    let b = &active.def;
    for (player, pid, color, pos, held) in &players {
        let at = rest_pos(b, pos.tile, held, slot_offset(pid.0));
        spawn_token(
            &mut commands,
            &mut meshes,
            &mut mats,
            player,
            slot_offset(pid.0),
            Color::srgb(color.0[0], color.0[1], color.0[2]),
            at,
        );
    }
    // 恶人的棋子与玩家的分开两条查询、共用同一个构造：他们没有 `PlayerId`，
    // 而槽位、颜色、停在哪一格的规则都该一样
    for (entity, villain, pos, held) in &villains {
        let slot = 4 + VillainKind::ALL
            .iter()
            .position(|k| *k == villain.kind)
            .unwrap_or(0) as u32;
        let offset = slot_offset(slot);
        let at = rest_pos(b, pos.tile, held, offset);
        spawn_token(
            &mut commands,
            &mut meshes,
            &mut mats,
            entity,
            offset,
            villain_color(villain.kind),
            at,
        );
    }
}

/// 这一颗棋子 resting 的世界坐标（缺格位时退到原点而不是猜一格）。
fn rest_pos(b: &BoardDef, tile: TileIndex, held: Option<&Detention>, offset: Vec3) -> Vec3 {
    let at = rest_tile(b, tile, held).unwrap_or(Vec3::ZERO) + offset;
    Vec3::new(at.x, dim::TOKEN_REST_Y, at.z)
}

fn spawn_token(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    entity: Entity,
    slot: Vec3,
    color: Color,
    at: Vec3,
) {
    commands.spawn((
        TokenVisual {
            player: entity,
            slot,
        },
        Mesh3d(meshes.add(Sphere::new(dim::TOKEN_R))),
        MeshMaterial3d(flat(mats, color)),
        Transform::from_translation(at),
        Motion {
            from: at,
            to: at,
            t: 0.0,
            dur: 0.0,
        },
        Rendered,
    ));
}

fn tile_world(b: &BoardDef, tile: TileIndex) -> Option<Vec3> {
    b.tile_world_pos(tile.0 as i64)
}

/// 一次移动在 core 里是四条不同的消息，收拢成一个参数包：
/// `animate_tokens` 的签名一旦长到要 `#[allow(clippy::too_many_arguments)]`，
/// 「参数太多了」这个真信号就被吃掉了（§六 坑 8）。
#[derive(SystemParam)]
pub(crate) struct MovementNews<'w, 's> {
    steps: MessageReader<'w, 's, TileStepped>,
    detained: MessageReader<'w, 's, PlayerDetained>,
    released: MessageReader<'w, 's, DetentionLifted>,
    /// 保释出来（P4③）：与期满同一条视觉——棋子从死胡同格挪回接入点
    bailed: MessageReader<'w, 's, PlayerBailed>,
    teleported: MessageReader<'w, 's, PlayerTeleported>,
}

/// 把 core 的移动事实翻译成视觉位移目标，然后按帧积分。
///
/// 行走用 `GameRules.walk_step_interval`（与 core 计步同一个值，两边自然同步；
/// 该值为 0 时 core 一帧走完，这里退化成瞬移），入狱/出狱/瞬移用 0.4s 滑移
/// （`player_token.gd:55,65`）。
pub(crate) fn animate_tokens(
    time: Res<Time>,
    active: Res<ActiveBoard>,
    rules: Res<GameRules>,
    mut news: MovementNews,
    mut tokens: Query<(&TokenVisual, &mut Transform, &mut Motion)>,
) {
    let b = &active.def;
    let hop = rules.walk_step_interval.max(f32::EPSILON);

    for s in news.steps.read() {
        if let Some(at) = tile_world(b, s.tile) {
            retarget(&mut tokens, s.player, at, hop);
        }
    }
    for j in news.detained.read() {
        // `end` 是死胡同格的网格坐标（没有 `TileIndex`），只给表现层停车用；
        // `None` 有两种：这张棋盘没有那条分支，或那个地方压根不在地图上（出国）——
        // 两种都停在接入点上（`card_effects.gd:50-55` 的 (-1,-1) 语义）
        let at = match j.end {
            Some(g) => Some(b.world_pos(g.col, g.row)),
            None => b.tile_world_pos(j.junction.0 as i64),
        };
        if let Some(at) = at {
            retarget(&mut tokens, j.player, at, dim::PARK_SECS);
        }
    }
    for r in news.released.read() {
        if let Some(at) = b.tile_world_pos(r.junction.0 as i64) {
            retarget(&mut tokens, r.player, at, dim::PARK_SECS);
        }
    }
    for r in news.bailed.read() {
        if let Some(at) = b.tile_world_pos(r.junction.0 as i64) {
            retarget(&mut tokens, r.player, at, dim::PARK_SECS);
        }
    }
    for t in news.teleported.read() {
        if let Some(at) = tile_world(b, t.tile) {
            retarget(&mut tokens, t.player, at, dim::PARK_SECS);
        }
    }

    let dt = time.delta_secs();
    for (_, mut tr, mut mo) in &mut tokens {
        if mo.t >= mo.dur {
            continue;
        }
        mo.t = (mo.t + dt).min(mo.dur);
        let u = mo.t / mo.dur;
        let at = mo.from.lerp(mo.to, u);
        // 弹跳合成一条正弦弧。Godot 是「走完这一格再弹两拍」（`player_token.gd:105-112`），
        // 那是 `await` 串出来的形态；这里一步之内起跳落地，看着是同一件事（§5.1 记差异）
        tr.translation = Vec3::new(at.x, mo.to.y + dim::BOUNCE * (u * PI).sin(), at.z);
    }
}

fn retarget(
    tokens: &mut Query<(&TokenVisual, &mut Transform, &mut Motion)>,
    player: Entity,
    tile_world: Vec3,
    dur: f32,
) {
    for (visual, tr, mut mo) in tokens.iter_mut() {
        if visual.player != player {
            continue;
        }
        mo.from = tr.translation;
        mo.to = Vec3::new(
            tile_world.x + visual.slot.x,
            dim::TOKEN_REST_Y,
            tile_world.z + visual.slot.z,
        );
        mo.t = 0.0;
        mo.dur = dur;
        return;
    }
}
