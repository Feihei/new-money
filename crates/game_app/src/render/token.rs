//! 棋子视觉：一颗把逻辑位置插值出来的球，或一条站着的剪纸人。对应 Godot `player_token.gd`。
//!
//! §4.6 的红线在这一层最要紧：core 在**掷骰那一帧**就把 `TokenPosition` 落定到终点，
//! 这里读 `TileStepped` 把同一次移动展开成逐格动画。于是渲染既不等待逻辑，
//! 也不被逻辑等待 —— Godot 那三处 `await token.arrived` 的形态在这里没有落点。
//!
//! 球与人共用同一条位移：sprite 只换掉网格与材质，并把脚底烘进顶点而不是实体的
//! `Transform`（见 [`sheet_frames`]），所以下面那套插值不必知道这一颗画的是什么。

use std::f32::consts::PI;

use bevy::{ecs::system::SystemParam, prelude::*, render::mesh::VertexAttributeValues};
use game_core::prelude::{
    ActiveBoard, BoardDef, Detention, DetentionLifted, GameRules, PlayerBailed, PlayerColor,
    PlayerDetained, PlayerId, PlayerName, PlayerTeleported, TileIndex, TileStepped, TokenPosition,
    Villain, VillainKind,
};

use super::{Rendered, billboard_rot, dim, flat};
use crate::art;

/// 一条横条的切法：八帧 128×128（`.agents/CHARACTERS.md` §十二 那条「抠底→描白边→打横条」
/// 的产物形状）。横条改了帧数要回来改这两个数，否则 uv 窗口切到隔壁帧去。
const STRIP_FRAMES: usize = 8;
const STRIP_CELL_PX: f32 = 128.0;

/// 一颗棋子身上的八帧。放在组件里而不是做成共享资源：换帧要按人错开相位，
/// 而「哪一帧」这件事除了他自己没有别人关心。
#[derive(Component)]
pub(crate) struct Sheet {
    frames: Vec<Handle<Mesh>>,
    /// 起播相位。同桌几人同相位时，一眼看去像同一个人复制了几份
    phase: f32,
}

/// 一颗视觉棋子。`pub` 是给相机模块的：rig 要跟的就是它的位置。
#[derive(Component)]
pub struct TokenVisual {
    /// 对应的玩家实体 —— `TurnOrder` 与所有消息都用它指代玩家
    pub player: Entity,
    /// 同格多人时的分角偏移（只含 XZ），见 [`slot_offset`]
    slot: Vec3,
    /// 站姿高度：球按球心（`dim::TOKEN_REST_Y`），人按脚底（`dim::TOKEN_FEET_Y`）。
    /// 存在这一颗棋子上而不是取一个全局值，是因为素材缺某一条横条时同一桌上会
    /// 同时站着人和球
    lift: f32,
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

/// 装配一颗棋子要动用的三个资产写入器。捏成一个结构体是为了让 [`spawn_token`]
/// 的签名剩下「往谁身上装」与「装成什么样」两件事。
struct Geo<'a> {
    meshes: &'a mut Assets<Mesh>,
    mats: &'a mut Assets<StandardMaterial>,
    server: &'a AssetServer,
}

/// 一颗棋子的外观与落点。`at` 已经含了分角偏移与 `lift`。
struct Look<'a> {
    /// 他挂在谁身上（`TokenVisual::player`）
    entity: Entity,
    slot: Vec3,
    lift: f32,
    at: Vec3,
    color: Color,
    /// 有横条就画剪纸人，没有就画球（缺文件与 `NM_ART=0` 都走这一支）
    strip: Option<&'a str>,
    phase: f32,
}

/// 装配棋子时要读的「环上行动者」那一份状态。命名一下：这个六元组已经过了 clippy 的
/// 「类型太复杂」那条线，而拆成两条查询就会让「他站在哪一格」在两处各算一遍。
type Holders<'w, 's> = Query<
    'w,
    's,
    (
        Entity,
        &'static PlayerId,
        &'static PlayerName,
        &'static PlayerColor,
        &'static TokenPosition,
        Option<&'static Detention>,
    ),
>;

pub(crate) fn spawn_token_visuals(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    players: Holders<'_, '_>,
    villains: Query<(Entity, &Villain, &TokenPosition, Option<&Detention>)>,
) {
    let b = &active.def;
    let mut geo = Geo {
        meshes: &mut meshes,
        mats: &mut mats,
        server: &server,
    };
    for (player, pid, name, color, pos, held) in &players {
        let seat = pid.0 as usize;
        let strip = art::strip_for(&name.0, seat);
        let lift = stand(strip.is_some());
        let slot = slot_offset(pid.0);
        spawn_token(
            &mut commands,
            &mut geo,
            Look {
                entity: player,
                slot,
                lift,
                at: rest_pos(b, pos.tile, held, slot, lift),
                color: Color::srgb(color.0[0], color.0[1], color.0[2]),
                strip: strip.as_deref(),
                phase: seat as f32 * 0.37,
            },
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
        let lift = stand(false);
        spawn_token(
            &mut commands,
            &mut geo,
            Look {
                entity,
                slot: offset,
                lift,
                at: rest_pos(b, pos.tile, held, offset, lift),
                color: villain_color(villain.kind),
                // 四位恶人**不上 sprite**：`assets/sprite/` 烘的是 CHARACTERS.md §四
                // 那十二位，而「强盗／小偷／流氓／间谍」不在里面。按座位去领会抢走
                // 某一位有名有姓的人的脸，宁可让这一桌混着两种形状
                strip: None,
                phase: 0.0,
            },
        );
    }
}

/// 站姿高度：球取球心，人取脚底。
fn stand(sprite: bool) -> f32 {
    if sprite {
        dim::TOKEN_FEET_Y
    } else {
        dim::TOKEN_REST_Y
    }
}

/// 这一颗棋子 resting 的世界坐标（缺格位时退到原点而不是猜一格）。
fn rest_pos(
    b: &BoardDef,
    tile: TileIndex,
    held: Option<&Detention>,
    offset: Vec3,
    lift: f32,
) -> Vec3 {
    let at = rest_tile(b, tile, held).unwrap_or(Vec3::ZERO) + offset;
    Vec3::new(at.x, lift, at.z)
}

fn spawn_token(commands: &mut Commands, geo: &mut Geo, look: Look) {
    let mut token = commands.spawn((
        TokenVisual {
            player: look.entity,
            slot: look.slot,
            lift: look.lift,
        },
        Transform {
            translation: look.at,
            // 球是各向同性的，转不转都一样；sprite 必须正对相机，否则同一张图
            // 在屏幕上是斜的一条
            rotation: if look.strip.is_some() {
                billboard_rot()
            } else {
                Quat::IDENTITY
            },
            ..default()
        },
        Motion {
            from: look.at,
            to: look.at,
            t: 0.0,
            dur: 0.0,
        },
        Rendered,
    ));
    match look.strip {
        Some(strip) => {
            let frames = sheet_frames(geo.meshes);
            token.insert((
                Mesh3d(frames[0].clone()),
                MeshMaterial3d(sheet_material(strip, geo)),
                Sheet {
                    frames,
                    phase: look.phase,
                },
            ));
        }
        None => {
            token.insert((
                Mesh3d(geo.meshes.add(Sphere::new(dim::TOKEN_R))),
                MeshMaterial3d(flat(geo.mats, look.color)),
            ));
        }
    }
}

/// 一条横条 → 八帧网格。几何全同，只有 uv 窗口差一格。
fn sheet_frames(meshes: &mut Assets<Mesh>) -> Vec<Handle<Mesh>> {
    let h = dim::TOKEN_SPRITE_H;
    (0..STRIP_FRAMES)
        .map(|k| {
            let mut mesh = Mesh::from(Rectangle::new(h, h));
            // 脚底当原点：`animate_tokens` 每帧写的是「站在哪一格」，把 h/2 烘进顶点
            // 而不是实体的 `Transform`，那条插值才不需要知道自己画的是球还是人
            if let Some(VertexAttributeValues::Float32x3(positions)) =
                mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
            {
                for p in positions.iter_mut() {
                    p[1] += h / 2.0;
                }
            }
            meshes.add(mesh.with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv_window(k)))
        })
        .collect()
}

/// 第 `k` 帧在横条里的 uv 窗口。顶点顺序照 `Rectangle` 的生成顺序：tr, tl, bl, br。
fn uv_window(k: usize) -> Vec<[f32; 2]> {
    // 内缩半个纹素：不缩的话双线性过滤会把左右两帧的颜色渗进这一帧
    let full = STRIP_FRAMES as f32 * STRIP_CELL_PX;
    let u0 = (k as f32 * STRIP_CELL_PX + 0.5) / full;
    let u1 = ((k + 1) as f32 * STRIP_CELL_PX - 0.5) / full;
    [[u1, 0.0], [u0, 0.0], [u0, 1.0], [u1, 1.0]].to_vec()
}

fn sheet_material(strip: &str, geo: &mut Geo) -> Handle<StandardMaterial> {
    geo.mats.add(StandardMaterial {
        base_color_texture: Some(geo.server.load(strip.to_owned())),
        // sprite 更是天生不需要光：明暗已经画死在那八张图里了
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        // 朝向算错一步就整块被背面剔除；两侧都画才看得出是算错了
        cull_mode: None,
        ..default()
    })
}

/// 横条放帧。按墙钟而不是帧计数：`PresentMode::AutoNoVsync` 下两者差得远，
/// 而 8fps 是这批图烘出来时的观感（`.agents/CHARACTERS.md` §十二）。
pub(crate) fn animate_sprites(time: Res<Time>, mut sheets: Query<(&mut Mesh3d, &Sheet)>) {
    for (mut mesh, sheet) in &mut sheets {
        let frame =
            ((time.elapsed_secs() + sheet.phase) * dim::SPRITE_FPS) as usize % sheet.frames.len();
        if mesh.0.id() != sheet.frames[frame].id() {
            mesh.0 = sheet.frames[frame].clone();
        }
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
            visual.lift,
            tile_world.z + visual.slot.z,
        );
        mo.t = 0.0;
        mo.dur = dur;
        return;
    }
}
