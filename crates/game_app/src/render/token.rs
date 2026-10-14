//! 棋子视觉：一颗把逻辑位置插值出来的球。对应 Godot `player_token.gd`。
//!
//! §4.6 的红线在这一层最要紧：core 在**掷骰那一帧**就把 `TokenPosition` 落定到终点，
//! 这里读 `TileStepped` 把同一次移动展开成逐格动画。于是渲染既不等待逻辑，
//! 也不被逻辑等待 —— Godot 那三处 `await token.arrived` 的形态在这里没有落点。

use std::f32::consts::PI;

use bevy::{ecs::system::SystemParam, prelude::*};
use game_core::prelude::{
    ActiveBoard, BoardDef, GameRules, PlayerColor, PlayerId, PlayerJailed, PlayerReleased,
    PlayerTeleported, TileIndex, TileStepped, TokenPosition,
};

use super::{dim, flat};

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
fn slot_offset(player: u32) -> Vec3 {
    const OFFSETS: [Vec2; 4] = [
        Vec2::new(-0.3, -0.3),
        Vec2::new(0.3, -0.3),
        Vec2::new(-0.3, 0.3),
        Vec2::new(0.3, 0.3),
    ];
    let o = OFFSETS[(player as usize) % OFFSETS.len()];
    Vec3::new(o.x, 0.0, o.y)
}

pub(crate) fn spawn_token_visuals(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    players: Query<(Entity, &PlayerId, &PlayerColor, &TokenPosition)>,
) {
    for (player, pid, color, pos) in &players {
        let slot = slot_offset(pid.0);
        let at = tile_world(&active.def, pos.tile).unwrap_or(Vec3::ZERO) + slot;
        let at = Vec3::new(at.x, dim::TOKEN_REST_Y, at.z);
        commands.spawn((
            TokenVisual { player, slot },
            Mesh3d(meshes.add(Sphere::new(dim::TOKEN_R))),
            MeshMaterial3d(flat(
                &mut mats,
                Color::srgb(color.0[0], color.0[1], color.0[2]),
            )),
            Transform::from_translation(at),
            Motion {
                from: at,
                to: at,
                t: 0.0,
                dur: 0.0,
            },
        ));
    }
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
    jailed: MessageReader<'w, 's, PlayerJailed>,
    released: MessageReader<'w, 's, PlayerReleased>,
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
    for j in news.jailed.read() {
        // `end` 是死胡同格的网格坐标（没有 `TileIndex`），只给表现层停车用；
        // 没有死胡同格的棋盘就停在接入点上（`card_effects.gd:50-55` 的 (-1,-1) 语义）
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
