//! 棋子视觉：一颗把逻辑位置插值出来的球，或一条站着的剪纸人。对应 Godot `player_token.gd`。
//!
//! §4.6 的红线在这一层最要紧：core 在**掷骰那一帧**就把 `TokenPosition` 落定到终点，
//! 这里读 `TileStepped` 把同一次移动展开成逐格动画。于是渲染既不等待逻辑，
//! 也不被逻辑等待 —— Godot 那三处 `await token.arrived` 的形态在这里没有落点。
//!
//! 球与人共用同一条位移：sprite 只换掉网格与材质，并把脚底烘进顶点而不是实体的
//! `Transform`（见 [`Track::build`]），所以下面那套插值不必知道这一颗画的是什么。

use std::f32::consts::PI;

use bevy::{ecs::system::SystemParam, prelude::*, render::mesh::VertexAttributeValues};
use game_core::prelude::{
    BoardDef, Detention, DetentionLifted, GameRules, Mount, PlayerBailed, PlayerColor,
    PlayerDetained, PlayerId, PlayerName, PlayerTeleported, TileIndex, TileStepped, TokenPosition,
    Villain, VillainKind,
};

use super::{
    Rendered, billboard_rot, dim, flat,
    geom::{BoardGeom, BoardRead},
};
use crate::art::{self, Pose};

/// 横条读不到像素尺寸时的兜底 cell 边长 —— 就是那批已烘出来的待机条的形状（1024×128 切八格）。
const FALLBACK_CELL_PX: u32 = 128;

/// 一种姿态：同一张脸上的那几帧。
struct Track {
    meshes: Vec<Handle<Mesh>>,
    /// 四条横条是四张贴图，材质各一个。切姿态要连它一起换，否则汽车那几帧画在待机条上。
    mat: Handle<StandardMaterial>,
}

/// 一颗棋子身上的几条横条。放在组件里而不是做成共享资源：换帧要按人错开相位，
/// 而「哪一帧」这件事除了他自己没有别人关心。
#[derive(Component)]
pub(crate) struct Sheet {
    /// 按 [`Pose::index`] 取。`Stand` 必然是 `Some`（`art::strips_for` 缺它就整个不返），
    /// 其余三条缺了由 [`Sheet::pose_of`] 退回答立。
    tracks: [Option<Track>; 4],
    /// 起播相位。同桌几人同相位时，一眼看去像同一个人复制了几份
    phase: f32,
    /// 走格钟，单位是「格」而不是秒：只在真有位移时前进（见 [`Sheet::frame_of`]）。
    steps: f32,
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
fn rest_tile(
    b: &BoardDef,
    geom: &BoardGeom,
    pos: TileIndex,
    held: Option<&Detention>,
) -> Option<Vec3> {
    if let Some(end) = held
        .and_then(|d| d.place.branch())
        .and_then(|kind| b.branch_end(kind))
    {
        return Some(geom.at(end.col, end.row).at);
    }
    geom.tile(pos.0 as usize).map(|f| f.at)
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
    strips: Option<&'a art::Strips>,
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
    board: BoardRead<'_>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    server: Res<AssetServer>,
    players: Holders<'_, '_>,
    villains: Query<(Entity, &Villain, &TokenPosition, Option<&Detention>)>,
) {
    let b = &board.active.def;
    let geom = &board.geom;
    let mut geo = Geo {
        meshes: &mut meshes,
        mats: &mut mats,
        server: &server,
    };
    for (player, pid, name, color, pos, held) in &players {
        let seat = pid.0 as usize;
        let strips = art::strips_for(&name.0, seat);
        let lift = stand(strips.is_some());
        let slot = slot_offset(pid.0);
        spawn_token(
            &mut commands,
            &mut geo,
            Look {
                entity: player,
                slot,
                lift,
                at: rest_pos(b, geom, pos.tile, held, slot, lift),
                color: Color::srgb(color.0[0], color.0[1], color.0[2]),
                strips: strips.as_ref(),
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
                at: rest_pos(b, geom, pos.tile, held, offset, lift),
                color: villain_color(villain.kind),
                // 四位恶人**不上 sprite**：`assets/sprite/` 烘的是 CHARACTERS.md §四
                // 那十二位，而「强盗／小偷／流氓／间谍」不在里面。按座位去领会抢走
                // 某一位有名有姓的人的脸，宁可让这一桌混着两种形状
                strips: None,
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
    geom: &BoardGeom,
    tile: TileIndex,
    held: Option<&Detention>,
    offset: Vec3,
    lift: f32,
) -> Vec3 {
    let at = rest_tile(b, geom, tile, held).unwrap_or(Vec3::ZERO) + offset;
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
            rotation: if look.strips.is_some() {
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
    match look.strips {
        Some(strips) => {
            let tracks =
                Pose::ALL.map(|pose| strips.get(pose).map(|path| Track::build(pose, path, geo)));
            // 站立是四条里唯一的必需项（`art::strips_for` 缺它整个不返），所以这一句取得到
            let stand = tracks[Pose::Stand.index()].as_ref().unwrap();
            token.insert((
                Mesh3d(stand.meshes[0].clone()),
                MeshMaterial3d(stand.mat.clone()),
                Sheet {
                    tracks,
                    phase: look.phase,
                    steps: 0.0,
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

impl Track {
    /// 一条横条 → 它的几帧网格 + 这一条自己的材质。
    ///
    /// 帧数是设计常量（[`art::Pose::frames`]），cell 尺寸从文件本身读（[`art::png_size`]）：
    /// 待机是 128×128 的方格，载具那条会算出 192×128。于是 quad **只跟着变宽，身高一个
    /// 单位不挪** —— 按 min 缩放把车塞进方格也行，但那会顺手把「人上车就缩小」画成合理样子，
    /// 而缩小正是管线那边共享锚定没做出来时的症状（`.agents/CHARACTERS.md` §十二）。
    fn build(pose: Pose, path: &str, geo: &mut Geo) -> Track {
        let frames = pose.frames();
        let (cell_w, cell_h) = art::png_size(path)
            .and_then(|size| art::cell(size, frames))
            .unwrap_or((FALLBACK_CELL_PX as f32, FALLBACK_CELL_PX as f32));
        let strip_px = cell_w * frames as f32;
        let h = dim::TOKEN_SPRITE_H;
        let meshes =
            (0..frames)
                .map(|k| {
                    let mut mesh = Mesh::from(Rectangle::new(h * cell_w / cell_h, h));
                    // 脚底当原点：`animate_tokens` 每帧写的是「站在哪一格」，把 h/2 烘进顶点
                    // 而不是实体的 `Transform`，那条插值才不需要知道自己画的是球还是人
                    if let Some(VertexAttributeValues::Float32x3(positions)) =
                        mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
                    {
                        for p in positions.iter_mut() {
                            p[1] += h / 2.0;
                        }
                    }
                    geo.meshes.add(mesh.with_inserted_attribute(
                        Mesh::ATTRIBUTE_UV_0,
                        uv_window(k, frames, strip_px),
                    ))
                })
                .collect();
        Track {
            meshes,
            mat: sheet_material(path, geo),
        }
    }
}

impl Sheet {
    /// 这一帧该摆哪种姿态。
    ///
    /// **载具优先，而且静止也算**：他确实坐在车上，画成站着就丢了一条真信息（HUD 那个
    /// 「汽车」标签是文字，不是画面）。缺那一条横条就退回答立 —— 宁可看着像没上车，
    /// 也不要一个人身上同时出现两种形状，也不要为「车在、人站着」写第二套合成。
    fn pose_of(&self, mount: Mount, moving: bool) -> Pose {
        let wanted = match mount {
            Mount::Car => Pose::Car,
            Mount::Bike => Pose::Bike,
            Mount::Foot if moving => Pose::Walk,
            Mount::Foot => Pose::Stand,
        };
        if self.tracks[wanted.index()].is_some() {
            wanted
        } else {
            Pose::Stand
        }
    }

    /// 这一姿态的第几帧。
    ///
    /// 站立那一条按墙钟 + 每人一个相位（`PresentMode::AutoNoVsync` 下按帧计数会跑到看不清，
    /// 而 8fps 是这批图烘出来时的观感，见 `.agents/CHARACTERS.md` §十二）。
    /// **在动的两条不能用墙钟**：一步 0.18s（`GameRules::walk_step_interval`），8fps 下一步
    /// 只推进 1.44 帧，循环周期 1s = 5.5 步 —— 腿与格子各走各的。它们读 `steps` 那根走格钟，
    /// 于是「一个循环 = 几格」是设计数而不是巧合：步行两条腿一个循环，轮子一圈一步。
    /// 载具静止时钉在第 0 帧，那一帧就是轮子停住的样子 —— 不必再烘一条「静止态」。
    fn frame_of(&self, pose: Pose, moving: bool, wall: f32) -> usize {
        let track = self.tracks[pose.index()].as_ref().unwrap();
        let n = track.meshes.len();
        let by_steps =
            |steps_per_cycle: f32| (self.steps / steps_per_cycle * n as f32) as usize % n;
        match pose {
            Pose::Stand => ((wall + self.phase) * dim::SPRITE_FPS) as usize % n,
            Pose::Walk => by_steps(dim::WALK_STEPS),
            Pose::Bike | Pose::Car if moving => by_steps(dim::WHEEL_STEPS),
            Pose::Bike | Pose::Car => 0,
        }
    }
}

/// 第 `k` 帧在横条里的 uv 窗口。顶点顺序照 `Rectangle` 的生成顺序：tr, tl, bl, br。
fn uv_window(k: usize, frames: usize, strip_px: f32) -> Vec<[f32; 2]> {
    // 内缩半个纹素：不缩的话双线性过滤会把左右两帧的颜色渗进这一帧
    let cell = strip_px / frames as f32;
    let u0 = (k as f32 * cell + 0.5) / strip_px;
    let u1 = ((k + 1) as f32 * cell - 0.5) / strip_px;
    [[u1, 0.0], [u0, 0.0], [u0, 1.0], [u1, 1.0]].to_vec()
}

fn sheet_material(strip: &str, geo: &mut Geo) -> Handle<StandardMaterial> {
    geo.mats.add(StandardMaterial {
        base_color_texture: Some(geo.server.load(strip.to_owned())),
        // sprite 更是天生不需要光：明暗已经画死在那几张图里了
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        // 朝向算错一步就整块被背面剔除；两侧都画才看得出是算错了
        cull_mode: None,
        ..default()
    })
}

/// 侧过身的那条横条只画了朝右那一侧，朝左靠 `scale.x` 翻到 -1 —— 素材减半，
/// 而且镜像与「哪一帧」无关，切帧不用重算朝向。
///
/// 翻转不是瞬移而是一段补间：纸片从 +1 扫过 0（那一两帧侧对你，是一条线）再到 -1。
/// 转向卡在 8 代里是一道专门的「纸片反转」动画；这里只是把「镜像必须发生」这件事
/// 补上速度，那个效果就顺手有了 —— 走到拐角掉头时也是同一个动画，不必单做。
///
/// 判据是这一步位移在**屏幕上**的横向分量：billboard 的局部 +X 就是屏幕水平轴（那正是
/// billboard 的全部意义），所以拿它点一下步向。斜街那几段在屏幕上几乎是竖着走的，
/// 横向分量趋零、镜像读不出方向 —— 这时保留当前朝向，连翻转的目标都不改，而不是赌一个。
fn turn_to(tr: &mut Transform, step: Vec3, dt: f32) {
    let flat_len = Vec2::new(step.x, step.z).length();
    if flat_len < 0.01 {
        return;
    }
    let right = billboard_rot() * Vec3::X;
    let screen_x = step.x * right.x + step.z * right.z;
    if screen_x.abs() <= flat_len * 0.15 {
        return;
    }
    // 全程 2.0（+1 → -1），`FLIP_SECS` 走完
    let pace = dt * 2.0 / dim::FLIP_SECS;
    let d = (screen_x.signum() - tr.scale.x).clamp(-pace, pace);
    tr.scale.x += d;
}

fn tile_world(geom: &BoardGeom, tile: TileIndex) -> Option<Vec3> {
    geom.tile(tile.0 as usize).map(|f| f.at)
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

/// 贴纸上那三样（球那一支一样都没有，所以整个是 `Option`）。
/// 别名是为了躲 clippy 的 `type_complexity` —— 这条查询本身是有信息量的，拆成两把查询
/// 就要多遍历一遍实体（同 `ui/items.rs` 那个 `Holders`）。
type Sticker<'a> = Option<(
    &'a mut Mesh3d,
    &'a mut MeshMaterial3d<StandardMaterial>,
    &'a mut Sheet,
)>;

/// 一颗棋子的位移与放帧要动的两把查询，捏成一个参数包。
///
/// `animate_tokens` 与 `retarget` 要共用同一份查询类型，而**签名里各写一遍省略形式是不行的**：
/// 省略出来的每个引用是一个独立的生命周期，把 `&mut tokens` 交给另一条签名时那些生命周期
/// 统一不了 —— 报的是 `lifetime may not live long enough`，指在 `retarget(...)` 那一行，
/// 看着像查询本身写错了。具名的 `SystemParam` 结构体两处都填 `Movers<'_, '_>`，
/// 两个参数自己就统一了。
#[derive(SystemParam)]
pub(crate) struct Movers<'w, 's> {
    tokens: Query<
        'w,
        's,
        (
            &'static TokenVisual,
            &'static mut Transform,
            &'static mut Motion,
            Sticker<'static>,
        ),
    >,
    /// 载具挂在玩家实体上，不在棋子上（棋子是棋手画的）。恶人读不到就按步行 ——
    /// 他们本来也只走路。
    mounts: Query<'w, 's, &'static Mount, With<PlayerId>>,
}

/// 把 core 的移动事实翻译成视觉位移目标，然后按帧积分，并顺手决定这一帧放哪一条横条。
///
/// 放帧与位移**合在同一条 system 里**，是因为放帧要读的那三样（在不在动、这一步多长、
/// 往哪个方向走）全在 `Motion` 上：拆成两条就要在两条查询之间传一份副本，而 `Motion`
/// 是 `&mut` 的 —— 两条 system 同碰它，元组里谁先跑没人保证（§6.1 坑 36）。
/// 解耦的那条红线是「渲染不等逻辑、逻辑不等渲染」（§4.6），不是「渲染内部要拆两条」。
///
/// 行走用 `GameRules.walk_step_interval`（与 core 计步同一个值，两边自然同步；
/// 该值为 0 时 core 一帧走完，这里退化成瞬移），入狱/出狱/瞬移用 0.4s 滑移
/// （`player_token.gd:55,65`）。
pub(crate) fn animate_tokens(
    time: Res<Time>,
    board: BoardRead<'_>,
    rules: Res<GameRules>,
    mut news: MovementNews,
    mut mv: Movers<'_, '_>,
) {
    let geom = &board.geom;
    let hop = rules.walk_step_interval.max(f32::EPSILON);

    for s in news.steps.read() {
        if let Some(at) = tile_world(geom, s.tile) {
            retarget(&mut mv, s.player, at, hop);
        }
    }
    for j in news.detained.read() {
        // `end` 是死胡同格的网格坐标（没有 `TileIndex`），只给表现层停车用；
        // `None` 有两种：这张棋盘没有那条分支，或那个地方压根不在地图上（出国）——
        // 两种都停在接入点上（`card_effects.gd:50-55` 的 (-1,-1) 语义）
        let at = match j.end {
            Some(g) => Some(geom.at(g.col, g.row).at),
            None => geom.tile(j.junction.0 as usize).map(|f| f.at),
        };
        if let Some(at) = at {
            retarget(&mut mv, j.player, at, dim::PARK_SECS);
        }
    }
    for r in news.released.read() {
        if let Some(at) = geom.tile(r.junction.0 as usize).map(|f| f.at) {
            retarget(&mut mv, r.player, at, dim::PARK_SECS);
        }
    }
    for r in news.bailed.read() {
        if let Some(at) = geom.tile(r.junction.0 as usize).map(|f| f.at) {
            retarget(&mut mv, r.player, at, dim::PARK_SECS);
        }
    }
    for t in news.teleported.read() {
        if let Some(at) = tile_world(geom, t.tile) {
            retarget(&mut mv, t.player, at, dim::PARK_SECS);
        }
    }

    let dt = time.delta_secs();
    let wall = time.elapsed_secs();
    for (visual, mut tr, mut mo, sprite) in &mut mv.tokens {
        let moving = mo.t < mo.dur;
        if moving {
            mo.t = (mo.t + dt).min(mo.dur);
            let u = mo.t / mo.dur;
            let at = mo.from.lerp(mo.to, u);
            // 弹跳合成一条正弦弧。Godot 是「走完这一格再弹两拍」（`player_token.gd:105-112`），
            // 那是 `await` 串出来的形态；这里一步之内起跳落地，看着是同一件事（§5.1 记差异）
            tr.translation = Vec3::new(at.x, mo.to.y + dim::BOUNCE * (u * PI).sin(), at.z);
        }
        let Some((mut mesh, mut mat, mut sheet)) = sprite else {
            continue;
        };
        // 球那一支没有 `Sheet`，所以下面这几句只发生在贴纸上
        let mount = mv.mounts.get(visual.player).copied().unwrap_or(Mount::Foot);
        let pose = sheet.pose_of(mount, moving);
        if moving {
            // 走格钟的单位是「格」：一步进一格，与 `walk_step_interval` 具体是多少秒无关，
            // 所以改步时（B7 的设置项）腿的快慢自己跟着变，不用回来改任何常量。
            // 瞬移那一步 `dur` 是 0.4s 而不是 0.18s，`min(1.0)` 让它也只算一格。
            sheet.steps += (dt / mo.dur).min(1.0);
            turn_to(&mut tr, mo.to - mo.from, dt);
        }
        let track = sheet.tracks[pose.index()].as_ref().unwrap();
        let k = sheet.frame_of(pose, moving, wall);
        if mesh.0.id() != track.meshes[k].id() {
            mesh.0 = track.meshes[k].clone();
        }
        if mat.0.id() != track.mat.id() {
            mat.0 = track.mat.clone();
        }
    }
}

fn retarget(mv: &mut Movers<'_, '_>, player: Entity, tile_world: Vec3, dur: f32) {
    for (visual, tr, mut mo, _) in mv.tokens.iter_mut() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 造几条假横条：只关心「盘上有哪几条」与「第几帧」，帧内容用 `Assets` 现造几个
    /// 互不相同的 id 就够（真 handle 要 `AssetServer`，那是有窗口的事）。
    fn stub(n: usize) -> Track {
        let mut meshes: Assets<Mesh> = default();
        let mut mats: Assets<StandardMaterial> = default();
        let quad = Rectangle::new(1.0, 1.0).mesh().build();
        Track {
            meshes: (0..n).map(|_| meshes.add(quad.clone())).collect(),
            mat: mats.add(StandardMaterial::default()),
        }
    }

    fn sheet(on: &[Pose]) -> Sheet {
        let mut tracks: [Option<Track>; 4] = Default::default();
        for pose in on {
            tracks[pose.index()] = Some(stub(pose.frames()));
        }
        Sheet {
            tracks,
            phase: 0.0,
            steps: 0.0,
        }
    }

    /// 只有待机条时，新推导出来的 cell 与 uv 窗口必须逐字等于从前那两个数 ——
    /// 这条是「clip 模型上了身而素材一条没加时画面不许变」的代数证明，
    /// 比拿两张截图找不同可靠（截图那两根钟对不齐，见 §6.1 坑 5/10）。
    #[test]
    fn a_square_strip_is_cut_exactly_where_it_always_was() {
        let (cw, ch) = crate::art::cell((1024, 128), Pose::Stand.frames()).unwrap();
        assert_eq!((cw, ch), (128.0, 128.0), "老素材的 cell 不该被重新算出来");
        let strip_px = cw * Pose::Stand.frames() as f32;
        // 从前是 `(k*128 ± 0.5) / (8*128)`
        for k in 0..Pose::Stand.frames() {
            let w = uv_window(k, Pose::Stand.frames(), strip_px);
            assert_eq!(w[1][0], (k as f32 * 128.0 + 0.5) / 1024.0);
            assert_eq!(w[0][0], ((k + 1) as f32 * 128.0 - 0.5) / 1024.0);
        }
    }

    #[test]
    fn a_missing_strip_degrades_to_standing_not_to_nothing() {
        let only_idle = sheet(&[Pose::Stand]);
        // 载具与步行都没烘：站着不动、站着开车，画出来的都是同一个人
        assert_eq!(only_idle.pose_of(Mount::Foot, false), Pose::Stand);
        assert_eq!(only_idle.pose_of(Mount::Foot, true), Pose::Stand);
        assert_eq!(only_idle.pose_of(Mount::Car, true), Pose::Stand);

        let idle_and_walk = sheet(&[Pose::Stand, Pose::Walk]);
        assert_eq!(idle_and_walk.pose_of(Mount::Foot, true), Pose::Walk);
        assert_eq!(idle_and_walk.pose_of(Mount::Car, true), Pose::Stand);

        // 四条齐全时，**静止也画坐着**：他身上有载具这件事不该只在 HUD 的文字里
        let all = sheet(&[Pose::Stand, Pose::Walk, Pose::Bike, Pose::Car]);
        assert_eq!(all.pose_of(Mount::Car, false), Pose::Car);
        assert_eq!(all.pose_of(Mount::Bike, false), Pose::Bike);
        assert_eq!(all.pose_of(Mount::Foot, false), Pose::Stand);
    }

    #[test]
    fn a_rider_sits_still_on_the_frame_where_the_wheels_stop() {
        let mut s = sheet(&[Pose::Stand, Pose::Walk, Pose::Car]);
        // 静止＝循环第 0 帧：载具不必再单独烘一条「静止态」
        assert_eq!(s.frame_of(Pose::Car, false, 3.0), 0);
        // 一步正好走完一圈（4 帧），所以**只能取步中的分数**：整数步就是回零那一格，
        // 拿它断言「在动就换帧」会永远红 —— 这条第一版就是这么写歪的
        let rolling: [usize; 3] = [0.25, 0.5, 0.75].map(|f| {
            s.steps = f;
            s.frame_of(Pose::Car, true, 3.0)
        });
        assert_eq!(rolling, [1, 2, 3], "轮子没有跟着格子走");
        // 步行一个循环跨两格，所以一步只走到半条 —— 按一格算就是缝纫机
        s.steps = 1.0;
        assert_eq!(s.frame_of(Pose::Walk, true, 0.0), Pose::Walk.frames() / 2);
    }

    /// 镜像只认屏幕横向分量：沿世界轴那两段翻，斜街那两段（屏幕上近乎竖着走）连目标都不改。
    #[test]
    fn the_mirror_flips_across_the_screen_and_holds_when_there_is_nothing_to_read() {
        let whole = dim::FLIP_SECS; // 一个补间刚好走完的时长
        let right = billboard_rot() * Vec3::X;
        let mut tr = Transform::default();
        turn_to(&mut tr, Vec3::Z, whole);
        let after_z = tr.scale.x;
        assert_ne!(after_z, 0.0, "沿世界 Z 走那一步没翻 —— 镜像压根没接上");
        turn_to(&mut tr, -Vec3::Z, whole);
        assert_eq!(tr.scale.x, -after_z, "同一段路倒着走没翻回来");

        // 屏幕上竖着走的那一段：横向分量恰好是 0，这时不许赌一个朝向。
        // **三个起手都要试** —— 只试一个的话，「赌」的结果可能正好等于那一个的期望值：
        // `0.0f32.signum()` 是 1.0，第一版只从 -1 起手，正好撞上，红检③ 当场没红。
        // 0.3 那一手是翻转进行到一半的世界状态：目标不变，半张纸片也不该自己走完。
        let perp = Vec3::new(right.z, 0.0, -right.x);
        for start in [1.0, -1.0, 0.3] {
            tr.scale.x = start;
            turn_to(&mut tr, perp, whole);
            assert_eq!(tr.scale.x, start, "{start} 起手那一步它赌了一个朝向");
        }
    }

    /// 掉头是「纸片翻过去」而不是「换一张贴图」：从 +1 扫过 0（那一两帧侧对你，是一条线）
    /// 再到 -1。转向卡在 8 代里那道专门的反转动画，在这里就是这条补间的速度。
    #[test]
    fn a_facing_change_sweeps_through_zero_instead_of_teleporting() {
        let mut tr = Transform::default();
        tr.scale.x = 1.0;
        turn_to(&mut tr, Vec3::Z, dim::FLIP_SECS / 2.0);
        assert_eq!(tr.scale.x, 0.0, "半程该正好侧对你（一条线），它跳过去了");
        turn_to(&mut tr, Vec3::Z, dim::FLIP_SECS / 2.0);
        assert_eq!(tr.scale.x, -1.0, "全程走完还停在中途");
    }
}
