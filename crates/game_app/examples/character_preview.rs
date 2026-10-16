//! 角色试装场景（一次性验收，不进游戏）：把 `assets/model/*.glb` 或 sprite 剪纸条
//! 摆到真实棋盘尺寸旁边看观感。
//!
//! 为什么是 example 而不是往对局里塞 `NM_MODELS=1`：现在要判断的是「模型本身行不行」
//! （朝向、缩放、贴图、PBR 光照下还看不看得出是个人），跟回合机与 UI 无关；把开发开关种进
//! `render/token.rs` 会让一条尚未拍板的路径污染生产代码。真定稿要接进对局，改的是 `token.rs`。
//!
//! 用法（与主程序同一套截图钩子）：
//! ```text
//! # 朝向矩阵：4 个 yaw × 2 个模型，透视相机 —— 结论：模型原生朝 +Z，yaw 45° 正对等轴测相机
//! NM_MODEL_VIEW=grid NM_SHOT=.tmp/grid.png cargo run --example character_preview
//! # 尺寸对照：与对局同参数的正交等轴测相机（视野高 8），球形棋子作参照
//! NM_MODEL_VIEW=iso NM_MODEL_YAW=45 NM_MODEL_SCALE=1.6 NM_SHOT=.tmp/iso.png cargo run --example character_preview
//! # 贴脸看贴图与网格：NM_MODEL_INDEX 选模型（0 阿土伯 / 1 孙小美）
//! NM_MODEL_VIEW=close NM_MODEL_INDEX=0 NM_SHOT=.tmp/close.png cargo run --example character_preview
//! # 对局现状（一个光源都没有）：模型会黑成剪影，棋盘却照常亮着
//! NM_NO_LIGHT=1 NM_AMBIENT=80 NM_MODEL_VIEW=iso NM_MODEL_SCALE=1.6 NM_SHOT=.tmp/dark.png cargo run --example character_preview
//! # 剪纸 sprite：I2V 抽帧打成的横条，左边贴屏对齐、右边世界竖直，中间那颗球是现状棋子
//! NM_MODEL_VIEW=sprite NM_SHOT=.tmp/sprite.png cargo run --example character_preview
//! # 整队人上棋盘：首发 12 + 隐藏位一排站开，看彼此区分度与白边在绿底上的表现
//! NM_MODEL_VIEW=lineup NM_SHOT_AT=5.2 NM_SHOT=.tmp/lineup.png cargo run --example character_preview
//! ```
//! `NM_MODEL_SCALE` 覆盖缩放（默认 1.0），`NM_CAM_SIZE` 覆盖正交视野高。
//! sprite 那条走的是另一套尺寸：`NM_SPRITE_H` 覆盖四边形世界高（默认 1.4），
//! `NM_SPRITE_FPS` 覆盖放帧速率（默认 8），`NM_SPRITE_STRIP` 换横条以 A/B 两种画风。
//! 地面、棋子、相机的尺寸抄自 `render/mod.rs` 的 `dim` 与 `camera.rs`（example 在 bin 之外，
//! 拿不到那些模块）。改了那边要记得回来改这里，否则观感判断建立在假尺寸上。

use std::path::PathBuf;

use bevy::{
    asset::LoadState,
    camera::ScalingMode,
    gltf::GltfAssetLabel,
    prelude::*,
    render::view::screenshot::{Screenshot, save_to_disk},
    window::{PresentMode, Window, WindowResolution},
    world_serialization::WorldAsset,
};

// ---- 与 render/mod.rs `dim` / camera.rs 同步的常量 ----
const CELL: f32 = 2.0; // BoardDef::cell_size（prototype.json）
const FLOOR_Y: f32 = -0.1;
const FLOOR_H: f32 = 0.25;
const PLOT_H: f32 = 0.3;
const SLAB_H: f32 = 0.4;
const BUILD_SCALE: f32 = 0.84;
const TOKEN_R: f32 = 0.5;
const TOKEN_REST_Y: f32 = 0.8;
const CAM_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);
const CAM_DIST: f32 = 40.0;
/// 对局的跟随取景高度（`dim::CAM_SIZE`）：约看得见 4×4 格。用它才能读出棋子占多少像素。
const CAM_SIZE: f32 = 8.0;
const SKY: Color = Color::srgb(0.6, 0.6, 0.7);
/// 地板顶面：模型脚底贴这里，才和对局里「站在格子上」一致
const GROUND_TOP: f32 = FLOOR_Y + FLOOR_H / 2.0;

const MODELS: [&str; 2] = [
    "model/character-ATubo.glb",
    "model/character-SunXiaomei.glb",
];

// ---- sprite 剪纸路线（`.tmp/mksprite.cjs` 的产物）----
const STRIP_DEFAULT: &str = "sprite/wenjiuzhi.png";
const STRIP_FRAMES: usize = 8;
const STRIP_CELL_PX: f32 = 128.0;
/// 首发 12 + 隐藏位沈万三。`lineup` 视图按这个顺序在棋盘上排一排。
const ROSTER: [&str; 13] = [
    "baotai",
    "leidang",
    "jibainian",
    "laixi",
    "mazu",
    "geshoutian",
    "wenjiuzhi",
    "fanyiwen",
    "duoji",
    "lvrenjia",
    "taluo",
    "suoyuan",
    "shenwansan",
];

/// 等轴测相机的仰角：视线 (1,2,1) → `atan2(2, √2)`。贴屏对齐的 sprite 要按这个角往后倒。
const CAM_ELEV: f32 = 54.7356;

/// 一个会放帧的剪纸。**帧网格挂在实体身上而不是全局 Resource**：
/// 一行十三个角色各用一条横条，共用一份帧表就换不了贴图。
///
/// 为什么是 8 个网格而不是一起改一个网格的 uv：`Mesh::insert_attribute` 对已经抽取到
/// render world 的数据会 panic，逐帧改 uv 等于每帧赌一次；换 `Mesh3d` 句柄只是改组件，永远安全。
#[derive(Component)]
struct BillboardSprite {
    frames: Vec<Handle<Mesh>>,
    fps: f32,
    /// 起播相位。整队人同相位会像同一张图复制的，错开才看得出各自的小动作。
    phase: f32,
}

/// 已请求加载的 gltf 场景，用于把「资产 404」变成一句 panic。
#[derive(Resource, Default)]
struct Pending(Vec<Handle<WorldAsset>>);

fn main() {
    let root = asset_root();
    info!("资产根：{}", root.display());
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "角色模型试装".into(),
                        present_mode: PresentMode::AutoNoVsync,
                        resolution: WindowResolution::new(1280, 800),
                        ..default()
                    }),
                    ..default()
                })
                .set(AssetPlugin {
                    file_path: root.display().to_string(),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(SKY))
        // 模型是带 baseColor / metallic-roughness / normal 三张图的 PBR 材质，必须有光。
        // 对局的材质全 `unlit`（`render/mod.rs` 的 `flat`），直接把它搬过来会渲成一块纯色。
        .insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: env_num("NM_AMBIENT", 220.0),
            ..default()
        })
        .init_resource::<Pending>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (report_failed_loads, animate_billboard, maybe_capture),
        )
        .run();
}

fn setup(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut pending: ResMut<Pending>,
) {
    let view = std::env::var("NM_MODEL_VIEW").unwrap_or_else(|_| "iso".into());
    let scale = env_num("NM_MODEL_SCALE", 1.0);
    let yaw = env_num("NM_MODEL_YAW", 0.0).to_radians();

    let is_sprite = view == "sprite" || view == "lineup";
    // lineup 要站一排，地板得跟着加长
    spawn_ground(
        &mut commands,
        &mut meshes,
        &mut mats,
        if view == "lineup" { 8 } else { 3 },
    );
    // `NM_NO_LIGHT=1` 模拟对局现状：`render/` 里一个光源都没有，材质全 unlit
    if std::env::var("NM_NO_LIGHT").is_err() {
        spawn_light(&mut commands);
    }

    if is_sprite {
        // sprite 这条路跟 glb 无关，别去 load 那 29MB —— 缺文件时 report_failed_loads 会误报
        if view == "lineup" {
            spawn_lineup(&mut commands, &server, &mut meshes, &mut mats);
        } else {
            spawn_sprites(&mut commands, &server, &mut meshes, &mut mats);
            spawn_reference_token(&mut commands, &mut meshes, &mut mats);
        }
    } else {
        for (i, path) in MODELS.iter().enumerate() {
            let handle = server.load(GltfAssetLabel::Scene(0).from_asset(path.to_string()));
            pending.0.push(handle.clone());
            if view == "grid" {
                // 一行四个朝向（0/90/180/270），两个模型各一行：哪一列是正面一眼看得出来
                for (k, deg) in [0.0f32, 90.0, 180.0, 270.0].into_iter().enumerate() {
                    spawn_model(
                        &mut commands,
                        handle.clone(),
                        Vec3::new(
                            if i == 0 { -2.4 } else { 2.4 },
                            GROUND_TOP,
                            (k as f32 - 1.5) * 2.6,
                        ),
                        Quat::from_rotation_y(deg.to_radians()),
                        scale,
                    );
                }
            } else if view == "close" {
                // 单个模型摆到原点，看贴图与网格细节（`NM_MODEL_INDEX=0|1` 选哪个）
                if env_num("NM_MODEL_INDEX", 0.0) as usize != i {
                    continue;
                }
                spawn_model(
                    &mut commands,
                    handle.clone(),
                    Vec3::new(0.0, GROUND_TOP, 0.0),
                    Quat::from_rotation_y(yaw),
                    scale,
                );
            } else {
                // iso：左右各一个模型，中间放现在对局里用的那颗球当尺寸参照
                spawn_model(
                    &mut commands,
                    handle.clone(),
                    Vec3::new((i as f32 - 0.5) * 2.0 * CELL, GROUND_TOP, 0.0),
                    Quat::from_rotation_y(yaw),
                    scale,
                );
            }
        }
        if view == "iso" {
            spawn_reference_token(&mut commands, &mut meshes, &mut mats);
        }
    }

    match view.as_str() {
        "grid" => commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(SKY),
                ..default()
            },
            Projection::from(PerspectiveProjection {
                fov: 30f32.to_radians(),
                ..default()
            }),
            Transform::from_xyz(0.0, 1.6, 15.0).looking_at(Vec3::new(0.0, 0.9, 0.0), Vec3::Y),
        )),
        "close" => commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(SKY),
                ..default()
            },
            Projection::from(PerspectiveProjection {
                fov: 35f32.to_radians(),
                ..default()
            }),
            Transform::from_xyz(0.0, 0.95, 3.0).looking_at(Vec3::new(0.0, 0.6, 0.0), Vec3::Y),
        )),
        // 与 camera.rs 的 rig 同参数：视线 (1,2,1) 对角、正交视野高 8
        // lineup 一排十三个人，默认视野放宽到 11 才装得下（`NM_CAM_SIZE` 仍可覆盖）
        _ => commands.spawn((
            Camera3d::default(),
            Camera {
                clear_color: ClearColorConfig::Custom(SKY),
                ..default()
            },
            Projection::from(OrthographicProjection {
                near: 0.1,
                far: 200.0,
                scaling_mode: ScalingMode::FixedVertical {
                    viewport_height: env_num(
                        "NM_CAM_SIZE",
                        if view == "lineup" { 11.0 } else { CAM_SIZE },
                    ),
                },
                ..OrthographicProjection::default_3d()
            }),
            Transform::from_translation(CAM_OFFSET.normalize() * CAM_DIST)
                .looking_at(Vec3::ZERO, Vec3::Y),
        )),
    };
}

/// 一格一块的地板（与 `board.rs` 的 `tile_floor` 同尺寸同配色）+ 一块地作背景参照。
/// `cols` 是半宽：lineup 要站一排，得把地板拉长。
fn spawn_ground(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
    cols: i32,
) {
    for col in -cols..=cols {
        for row in -1i32..=1 {
            // 棋盘是双色密铺，这里只取「一眼看出格子边界」这个作用
            let c = if (col + row).rem_euclid(2) == 0 {
                Color::srgb(0.34, 0.42, 0.3)
            } else {
                Color::srgb(0.3, 0.38, 0.27)
            };
            commands.spawn((
                Mesh3d(meshes.add(Cuboid::new(CELL, FLOOR_H, CELL))),
                MeshMaterial3d(unlit(mats, c)),
                Transform::from_xyz(col as f32 * CELL, FLOOR_Y, row as f32 * CELL),
            ));
        }
    }
    // 后排一格：地块 + 一层建筑（顶 0.7），模型和建筑谁高谁矮要有个已知答案
    let back = Vec3::new(0.0, 0.0, -3.0 * CELL);
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(CELL * 0.96, PLOT_H, CELL * 0.96))),
        MeshMaterial3d(unlit(mats, Color::srgb(0.4, 0.2, 0.2))),
        Transform::from_xyz(back.x, PLOT_H / 2.0, back.z),
    ));
    commands.spawn((
        Mesh3d(meshes.add(Cuboid::new(CELL * BUILD_SCALE, SLAB_H, CELL * BUILD_SCALE))),
        MeshMaterial3d(unlit(mats, Color::srgb(0.9, 0.25, 0.25))),
        Transform::from_xyz(back.x, PLOT_H + SLAB_H / 2.0, back.z),
    ));
}

/// 对局现在的棋子：半径 0.5 的球，停在 y=0.8。缩放定标就靠它当尺子。
fn spawn_reference_token(
    commands: &mut Commands,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) {
    commands.spawn((
        Mesh3d(meshes.add(Sphere::new(TOKEN_R))),
        MeshMaterial3d(unlit(mats, Color::srgb(1.0, 0.85, 0.2))),
        Transform::from_xyz(0.0, TOKEN_REST_Y, 0.0),
    ));
}

/// 两个朝向并排：左＝贴屏对齐（画多大就上屏多大），右＝世界竖直（真站在格子上，但被压扁）。
/// 中间那颗球是现状棋子，用来读「进游戏到底占多少像素」。
fn spawn_sprites(
    commands: &mut Commands,
    server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) {
    let h = env_num("NM_SPRITE_H", 1.4);
    let strip = strip_path();
    // 视线 (1,2,1)：先绕 Y 转 45° 对上方位角，再绕局部 X 往后倒 54.7°，法线即与视线重合
    let screen_aligned =
        Quat::from_rotation_y(45.0f32.to_radians()) * Quat::from_rotation_x(-CAM_ELEV.to_radians());
    for (i, rot) in [screen_aligned, Quat::from_rotation_y(45.0f32.to_radians())]
        .into_iter()
        .enumerate()
    {
        let (mesh, mat, frames) = make_sprite(&strip, h, server, meshes, mats);
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            BillboardSprite {
                frames,
                fps: env_num("NM_SPRITE_FPS", 8.0),
                phase: 0.0,
            },
            // 网格中心在原点，所以把实体抬到脚底：沿这个朝向自己的「上」走半个高
            Transform::from_translation(
                Vec3::new((i as f32 - 0.5) * 2.0 * CELL, GROUND_TOP, 0.0)
                    + rot * (Vec3::Y * (h / 2.0)),
            )
            .with_rotation(rot),
        ));
    }
}

/// 首发十二人 + 隐藏位站一排，全部贴屏对齐。看的是整队人在棋盘上的比例与彼此区分度。
fn spawn_lineup(
    commands: &mut Commands,
    server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) {
    let h = env_num("NM_SPRITE_H", 1.4);
    let rot =
        Quat::from_rotation_y(45.0f32.to_radians()) * Quat::from_rotation_x(-CAM_ELEV.to_radians());
    for (i, who) in ROSTER.iter().enumerate() {
        let (mesh, mat, frames) =
            make_sprite(&format!("sprite/{who}.png"), h, server, meshes, mats);
        commands.spawn((
            Mesh3d(mesh),
            MeshMaterial3d(mat),
            BillboardSprite {
                frames,
                // 每人错开 0.37s 起播：全部同相位的话，一眼看去像同一张图复制了十三份
                fps: env_num("NM_SPRITE_FPS", 8.0),
                phase: i as f32 * 0.37,
            },
            Transform::from_translation(
                Vec3::new(
                    (i as f32 - (ROSTER.len() as f32 - 1.0) / 2.0) * 1.15,
                    GROUND_TOP,
                    0.0,
                ) + rot * (Vec3::Y * (h / 2.0)),
            )
            .with_rotation(rot),
        ));
    }
}

/// 一条横条 → 一个初始网格 + 材质 + 八个帧网格。几何全同，只有 uv 窗口不同。
fn make_sprite(
    strip: &str,
    h: f32,
    server: &AssetServer,
    meshes: &mut Assets<Mesh>,
    mats: &mut Assets<StandardMaterial>,
) -> (Handle<Mesh>, Handle<StandardMaterial>, Vec<Handle<Mesh>>) {
    require_strip(strip);
    let frames: Vec<Handle<Mesh>> = (0..STRIP_FRAMES)
        .map(|k| {
            meshes.add(
                Mesh::from(Rectangle::new(h, h))
                    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uv_window(k)),
            )
        })
        .collect();
    let mat = mats.add(StandardMaterial {
        base_color_texture: Some(server.load(strip.to_owned())),
        // 对局的材质全 unlit，sprite 更是天生不需要光：明暗已经画死在那几张图里了
        unlit: true,
        alpha_mode: AlphaMode::Blend,
        // 贴屏那块往后倒，朝向算错一步就整块被背面剔除；两侧都画才判断得出来
        cull_mode: None,
        ..default()
    });
    (frames[0].clone(), mat, frames)
}

/// 换一张横条就能 A/B 两种画风（`NM_SPRITE_STRIP=sprite/wenjiuzhi_bb.png`）。
fn strip_path() -> String {
    std::env::var("NM_SPRITE_STRIP").unwrap_or_else(|_| STRIP_DEFAULT.into())
}

/// `assets/sprite/` 与 glb 一样不入库（AGENTS.md 硬约束 8），所以缺文件是这个 example 的**正常**状态。
/// Bevy 对加载失败的资产只打日志然后永远等下去，屏幕上是一张静默的白卡（§六 坑 6）——
/// 换成一句指出重烘方法的 panic。
fn require_strip(rel: &str) {
    let abs = asset_root().join(rel);
    if !abs.is_file() {
        panic!(
            "缺 sprite 横条 {rel}（找的是 {}）。重烘方法见 .agents/CHARACTERS.md §十二，脚本 `.tmp/batch_i2v.sh`",
            abs.display()
        );
    }
}

/// 第 `k` 帧在横条里的 uv 窗口。顶点顺序照 `Rectangle` 的生成顺序：tr, tl, bl, br。
fn uv_window(k: usize) -> Vec<[f32; 2]> {
    // 内缩半个纹素：不缩的话双线性过滤会把左右两帧的颜色渗进这一帧
    let full = STRIP_FRAMES as f32 * STRIP_CELL_PX;
    let u0 = (k as f32 * STRIP_CELL_PX + 0.5) / full;
    let u1 = ((k + 1) as f32 * STRIP_CELL_PX - 0.5) / full;
    [[u1, 0.0], [u0, 0.0], [u0, 1.0], [u1, 1.0]].to_vec()
}

/// 放帧按墙钟而不是帧计数：`PresentMode::AutoNoVsync` 下两者差得远，8fps 是想要的观感。
fn animate_billboard(time: Res<Time>, mut sprites: Query<(&mut Mesh3d, &BillboardSprite)>) {
    for (mut mesh, sheet) in &mut sprites {
        let fps = if sheet.fps > 0.0 { sheet.fps } else { 8.0 };
        let frame = ((time.elapsed_secs() + sheet.phase) * fps) as usize % sheet.frames.len();
        if mesh.0.id() != sheet.frames[frame].id() {
            mesh.0 = sheet.frames[frame].clone();
        }
    }
}

fn spawn_model(
    commands: &mut Commands,
    scene: Handle<WorldAsset>,
    at: Vec3,
    rot: Quat,
    scale: f32,
) {
    commands.spawn((
        WorldAssetRoot(scene),
        Transform::from_translation(at)
            .with_rotation(rot)
            .with_scale(Vec3::splat(scale)),
    ));
}

/// 一个斜上方主光 + 冷色补光。模型是 48k 三角面、2K 贴图，光影方向定了才看得出体积。
fn spawn_light(commands: &mut Commands) {
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.97, 0.9),
            illuminance: 12000.0,
            shadow_maps_enabled: true,
            ..default()
        },
        // 两个欧拉角要合成一个四元数：`Transform::with_rotation` 是覆盖而不是叠加
        Transform::from_rotation(
            Quat::from_rotation_y(-35.0f32.to_radians())
                * Quat::from_rotation_x(-50.0f32.to_radians()),
        ),
    ));
}

/// Bevy 对失败资产默认只打日志然后永远等下去，静默白屏比一句 panic 难查（§六 坑 6）。
fn report_failed_loads(server: Res<AssetServer>, pending: Res<Pending>) {
    for h in &pending.0 {
        if let LoadState::Failed(err) = server.load_state(h) {
            panic!(
                "模型加载失败：{err}（路径 {:?}，检查资产根与 NM_MODEL_VIEW）",
                h.path()
            );
        }
    }
}

/// 与主程序同款的墙钟截图钩子。默认 4s：29MB 模型 + 两张 2K 贴图，1.5s 还没解码完。
fn maybe_capture(time: Res<Time>, mut done: Local<bool>, mut commands: Commands) {
    let Ok(path) = std::env::var("NM_SHOT") else {
        return;
    };
    let at = std::env::var("NM_SHOT_AT")
        .ok()
        .and_then(|s| s.parse::<f32>().ok())
        .unwrap_or(4.0);
    if *done {
        if time.elapsed_secs() >= at + 1.5 {
            std::process::exit(0);
        }
        return;
    }
    if time.elapsed_secs() < at {
        return;
    }
    *done = true;
    info!("截图输出：{path}（t={:.2}s）", time.elapsed_secs());
    commands
        .spawn(Screenshot::primary_window())
        .observe(save_to_disk(path));
}

fn env_num(key: &str, default: f32) -> f32 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.trim().parse::<f32>().ok())
        .unwrap_or(default)
}

/// 与 `render/mod.rs` 的 `flat` 同义：地面这些参照物保持对局的无光照观感，
/// 只有模型走 PBR —— 两者的差别本身就是要看的东西。
fn unlit(mats: &mut Assets<StandardMaterial>, c: Color) -> Handle<StandardMaterial> {
    mats.add(StandardMaterial {
        base_color: c,
        unlit: true,
        ..default()
    })
}

/// 与 `main.rs` 同一条理由：0.20 的 `file_path` 相对可执行文件目录，`cargo run` 的 CWD 在仓库根。
fn asset_root() -> PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let from_cwd = cwd.join("assets");
    if from_cwd.is_dir() {
        return from_cwd;
    }
    let exe = std::env::current_exe().unwrap_or_default();
    exe.parent().map(|p| p.join("assets")).unwrap_or(from_cwd)
}
