//! 表现层（B4 渲染切片）：把规则层的实体与消息画成等轴测棋盘。
//!
//! 分层红线（`.agents/ARCHITECTURE.md` §4.6）：这一层只**读** core 的状态与消息，
//! 从不向 core 发「推进回合」的消息 —— 逻辑位置与步时归 core，棋子在这里只是插值出来的球。
//! 唯一的例外是「玩家意图」那两条（`RequestRoll`/`DecisionChosen`），它们的发送者是输入侧：
//! B4 期间是 `crate::autopilot`，B5 换成按钮、B6 换成 AI，都不是本模块。

pub mod board;
pub mod camera;
pub mod dice;
pub mod disc;
pub mod geom;
pub mod labels;
pub mod land;
pub mod occupancy;
pub mod token;

use bevy::{
    ecs::system::SystemParam, gltf::GltfAssetLabel, prelude::*, world_serialization::WorldAsset,
};
use game_core::prelude::ActiveBoard;

/// 表现层自己 spawn 的实体标记（棋盘几何、土地视觉、棋子、格名、相机、弹层底板）。
/// 回主菜单时按它一次性清场 —— 规则层的实体由 `game_core::Gameplay` 负责，两边互不知晓。
#[derive(Component)]
pub struct Rendered;

/// 开发期 CJK 字体句柄。`assets/font/dev_*` 是本地开发副本（不入库，见 AGENTS.md 硬约束 7），
/// Bevy 没有 Godot 那套内建中文回退，不显式指定字体就是豆腐块（§六 坑 2）。
#[derive(Resource)]
pub struct DevFont(pub Handle<Font>);

/// 棋盘格子的素面纸卡。`RenderPlugin::build` 里只拿句柄不解码 —— 与 `ui::skin::Skins` 同一条：
/// 盘上没有就是 `None`，那一块几何退回现在的纯色方块。
///
/// 配色**不在这里**：`base_color` 仍由 `palette` 出，纸只出形（圆角、切边线、纸纹）。
/// 于是 18 种格子类型共用一张图，而 `NM_ART=0/1` 两张对照图差的只是纸，不是颜色。
#[derive(Resource, Clone)]
pub struct BoardPaper {
    /// 格子素面纸卡（圆角，四角真透明）
    pub plate: Option<Handle<Image>>,
}

/// 地面的两张图：草坪（整盘一层）与直路（每格一张，按街道朝向转）。
///
/// 各自独立退回：草坪缺了整盘用纯色垫底，街面缺了用纯色方块 —— 形状由几何保证，
/// 素材只决定花纹（素材不入库，别的机器上这是常态，AGENTS.md 硬约束 8）。
#[derive(Resource, Clone)]
pub struct RoadTiles {
    pub grass: Option<Handle<Image>>,
    pub straight: Option<Handle<Image>>,
}

impl RoadTiles {
    pub fn load_all(server: &AssetServer) -> Self {
        Self {
            grass: crate::art::road_grass().map(|path| server.load(path)),
            straight: crate::art::road_unit("straight_orth").map(|path| server.load(path)),
        }
    }
}

/// 地块边界的粉笔框（`textures/board/chalk_*.png`，透明底，只画线不画面）。
/// 两块都要有：方框盖 1×1，角片按格拼出 2×2 那一圈外框。
#[derive(Resource, Clone)]
pub struct Chalk {
    pub frame: Option<Handle<Image>>,
    pub corner: Option<Handle<Image>>,
}

impl Chalk {
    pub fn load(server: &AssetServer) -> Self {
        Self {
            frame: crate::art::chalk_frame().map(|p| server.load(p)),
            corner: crate::art::chalk_corner().map(|p| server.load(p)),
        }
    }

    pub fn pair(&self) -> Option<(&Handle<Image>, &Handle<Image>)> {
        Some((self.frame.as_ref()?, self.corner.as_ref()?))
    }
}

/// 特殊格薄饼上的圆形图标，按 [`disc::disc_key`] 的键存。
///
/// **逐键退回**，不成套：道路单元少一张会在那一格断沥青，而这一块饼少一张只是没花纹，
/// 饼本身（扁圆柱）照画 —— 「这一格不是地」那句话仍然说得出。
#[derive(Resource, Clone, Default)]
pub struct DiscIcons {
    icons: std::collections::HashMap<&'static str, Handle<Image>>,
}

impl DiscIcons {
    pub fn load(server: &AssetServer) -> Self {
        Self {
            icons: disc::KEYS
                .iter()
                .filter_map(|k| crate::art::disc_icon(k).map(|path| (*k, server.load(path))))
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&Handle<Image>> {
        self.icons.get(key)
    }
}

/// 空地上的归属标志物（`prop/<角色键>.png` + 同键 `_edge.png`），按**角色键**存。
///
/// 两张图一起进：本体是那件道具，蒙版是它的轮廓环，运行时把环染成归属色 ——
/// 与住宅楼的描边同一条通道（换主人只改一个材质）。缺任意一张的那位不进来，
/// 那块地就退回原来的「粉笔框 + 色块」，不响也不 panic（素材不入库，别的机器上这是常态）。
#[derive(Resource, Clone, Default)]
pub(crate) struct PropArt {
    sheets: std::collections::HashMap<&'static str, (Handle<Image>, Handle<Image>)>,
}

impl PropArt {
    pub fn load(server: &AssetServer) -> Self {
        Self {
            sheets: crate::art::roster_keys()
                .filter_map(|k| {
                    crate::art::prop(k)
                        .map(|(body, edge)| (k, (server.load(body), server.load(edge))))
                })
                .collect(),
        }
    }

    pub fn get(&self, key: &str) -> Option<&(Handle<Image>, Handle<Image>)> {
        self.sheets.get(key)
    }

    /// 一张都没装上 —— 别的机器上这是常态（`assets/prop/` 不入库），不是错误。
    pub fn len(&self) -> usize {
        self.sheets.len()
    }
}

/// 住宅五级建筑模型（`model/dwell_L1..L5.glb`，下标 = 等级 − 1）。
///
/// **成套才上**：缺任意一级就整块退回方体 —— 半套素材摆出来会是「Lv2 是一栋房子、
/// Lv3 是一摞盒子」，而那种画面查起来比全是盒子难得多（与 `Chalk` 同一条纪律）。
/// 素材不入库（AGENTS.md 硬约束 8），所以空表在别的机器上是正常状态，不是错误。
#[derive(Resource, Clone, Default)]
pub struct BuildingModels {
    pub levels: Vec<Handle<WorldAsset>>,
}

/// 地面与住宅模型用到的素材。它们永远一起被读（哪一版地面、要不要粉笔、有没有模型），
/// 所以收成一个参数包：`attach_land_visuals` 逐个传会撞到 clippy 的 7 个参数上限，
/// 而那一撞会把「参数太多了」这个真信号吃掉（§六 坑 8，与 `UiKit`、`MovementNews` 同一条房规）。
#[derive(SystemParam)]
pub(crate) struct GroundArt<'w> {
    pub paper: Res<'w, BoardPaper>,
    pub units: Res<'w, RoadTiles>,
    pub chalk: Res<'w, Chalk>,
    pub models: Res<'w, BuildingModels>,
    /// 空地上的归属标志物。并进这个包而不是单开一个参数：`attach_land_visuals` 的参数
    /// 已经顶到 clippy 的 7 个上限（见上面那段注释）。
    pub props: Res<'w, PropArt>,
}

/// 表现层的两组 system。守门条件挂在**集合**上而不是逐个 system 上：
/// 一个元组整体 `.run_if(...)` 在参数里含双生命周期 `SystemParam`（如 `MovementNews`）时
/// 会退化成 `ObserverSystemExt::run_if` 的重载解析失败，写成分组才站得住。
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum RenderSet {
    /// 棋盘就位那一刻的一次性装配
    Boot,
    /// 逐帧动画与投影
    Live,
}

/// 一次性装配等 `ActiveBoard` 出现，逐帧动画等它存在即可。
pub struct RenderPlugin;

impl Plugin for RenderPlugin {
    fn build(&self, app: &mut App) {
        // 地面纸素材只拿句柄不解码。AssetServer 属于 DefaultPlugins，本插件注册时必然已在
        // —— 与 `UiPlugin` 取 `Skins` 同一手。
        let server = app.world_mut().resource::<AssetServer>().clone();
        app.insert_resource(BoardPaper {
            plate: crate::art::board_plate().map(|p| server.load(p)),
        });
        app.insert_resource(RoadTiles::load_all(&server));
        // 几何先占一个空的：`build_board_geom` 就地填它，所以资源必须一直在（否则读它的
        // system 整批报「Resource does not exist」），而空表只意味着「还没推」
        app.init_resource::<geom::BoardGeom>();
        app.insert_resource(Chalk::load(&server));
        app.insert_resource(DiscIcons::load(&server));
        app.insert_resource(PropArt::load(&server));
        // 五级齐了才装；缺一级就整块退回方体（见 `BuildingModels` 的注释）
        let levels = (1..=5)
            .map(crate::art::dwell_model)
            .collect::<Option<Vec<String>>>();
        app.init_resource::<land::OutlineMeshes>();
        app.insert_resource(BuildingModels {
            levels: levels
                .map(|paths| {
                    paths
                        .into_iter()
                        .map(|p| server.load(GltfAssetLabel::Scene(0).from_asset(p)))
                        .collect()
                })
                .unwrap_or_default(),
        });
        // 建筑模型是带 PBR 材质的 glb，对局里一个光源都没有会渲成黑剪影（§7.5）。
        // 环境光只有 PBR 材质读，盘上那些 `unlit` 的平涂色块不受影响。
        app.insert_resource(GlobalAmbientLight {
            color: Color::WHITE,
            brightness: 280.0,
            ..default()
        });
        app.add_systems(
            Update,
            (
                geom::build_board_geom,
                (
                    board::spawn_board_geometry,
                    disc::spawn_disc_markers,
                    land::attach_land_visuals,
                    land::spawn_building_light,
                    token::spawn_token_visuals,
                    camera::spawn_camera_rig,
                    labels::spawn_tile_labels,
                )
                    .in_set(RenderSet::Boot),
                (
                    token::animate_tokens,
                    dice::spawn_dice_visuals,
                    dice::animate_dice_visuals,
                    land::sync_buildings,
                    land::paint_outline_meshes,
                    occupancy::sync_occupancy_visuals,
                    camera::follow_current_token,
                    labels::project_tile_labels,
                )
                    .in_set(RenderSet::Live),
            )
                .chain(),
        );
        // `begin_game_system` 在同一批 `Commands` 里插 `ActiveBoard` 并 spawn 土地/玩家实体，
        // 所以「棋盘就位」与「`LandId`/`PlayerId` 是新加的」必然同帧成立 ——
        // 一个守门条件就够，不必每个 system 各自 `Local<bool>`。
        // 上面那条 `build_board_geom` 故意**不在任何守门条件里**：同一个 `resource_added`
        // 挂到两个集合上会互相吃掉 change tick，详见 [`geom::build_board_geom`]。
        app.configure_sets(
            Update,
            (
                RenderSet::Boot.run_if(resource_added::<ActiveBoard>),
                RenderSet::Live.run_if(resource_exists::<ActiveBoard>),
            ),
        );
    }
}

/// 等轴测几何尺寸。逐个对应 Godot 表现层的常量，改这里等于改全图比例。
pub mod dim {
    use bevy::math::{Quat, Vec3};

    /// 道路格地板：整格密铺（`tile_view.gd:94,97`）
    pub const FLOOR_H: f32 = 0.25;
    pub const FLOOR_Y: f32 = -0.1;
    /// 街面与草地所在的高度 —— 就是地板顶面，换地面模型不该挪动任何别的东西
    pub const TILE_Y: f32 = FLOOR_Y + FLOOR_H / 2.0;
    /// 特殊格薄饼（[`super::disc`]）：半径与厚度按 `cell_size` 的倍数给。
    /// 撤了同色组色条之后，「这一格不是地」全靠这块饼说出来，所以它要明显小于格、
    /// 又明显高于地面 —— 0.4 是格边长的八成直径，留出一圈草边读得出「摆在草地上」。
    pub const DISC_R: f32 = 0.4;
    pub const DISC_H: f32 = 0.14;
    /// 薄饼的轴心高度：坐在无缝地面上（盒子版的地面也是这个顶面）
    pub const DISC_Y: f32 = TILE_Y + DISC_H / 2.0;
    /// 土地底座与建筑方体（`land_view.gd:19-23`）
    pub const PLOT_H: f32 = 0.3;
    /// 粉笔框浮在地块顶面之上多少 —— 只要够躲开 z 竞争，肉眼看不出它是浮着的
    pub const CHALK_LIFT: f32 = 0.01;
    pub const SLAB_H: f32 = 0.4;
    pub const SLAB_GAP: f32 = 0.12;
    /// 空地上的归属标志物（`assets/prop/<键>.png`）：一张贴屏四边形的边长。
    /// 素材是 128²、物件底边对齐格底，所以四边形取正方形即可，不必按每件的宽高比另算。
    /// 0.85 世界单位在 `CAM_SIZE=8` 下约 85px —— 比棋子（140px）矮一档，读作「放在地上的东西」。
    pub const PROP_H: f32 = 0.85;
    /// 标志物落在地块顶面上。**Lv1 的模型其实是一栋小房子**（不是空地），所以它不能摆在
    /// 地块正中 —— 那会站到屋顶上去；沿「朝向相机」的水平方向挪到地块近角，站在房子前面。
    /// 倍数乘的是地块边长：1×1 挪 0.96、2×2 挪 1.92，都刚好落在粉笔框里。
    pub const PROP_AHEAD: f32 = 0.48;
    pub const PROP_BASE: f32 = PLOT_H + 0.02;
    /// 蒙版那张四边形**沿远离相机的方向**退这么多：两张同位置会 z 打架（花屏）。
    /// 0.012 世界单位在 `CAM_SIZE=8` 下约 1 像素，肉眼看不出偏移。
    pub const PROP_EDGE_BACK: f32 = 0.012;
    pub const PLOT_SCALE: f32 = 0.96;
    pub const BUILD_SCALE: f32 = 0.84;
    /// 住宅模型（`dwell_L1..L5.glb`）的缩放。生成时墙体外皮 0.362、原点在脚底、
    /// **+Z 朝上**（Blender 里按 Z-up 导出），所以进对局先立起来再缩。
    /// 取 1.7：最宽的一级（Lv3/Lv4 的 0.73）约 1.24 世界单位 —— 地块 2.0、粉笔框画在格边，
    /// 楼基两侧各留约 0.38 才看得见框；Lv1 的墙宽约 0.80（四成格宽），是空地中间的一间小屋。
    /// Lv5 的楼高约 3.35，仍要抬头看，但不至于占掉邻格的屋顶。
    pub const DWELL_SCALE: f32 = 1.7;
    /// Z-up → Y-up：绕 X 转 −90°。转完模型正面（Blender 的 −Y）朝 +Z，
    /// 正好对着 (8,16,8) 那台等轴测相机。
    pub fn dwell_upright() -> Quat {
        Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
    }
    /// 棋子（`player_token.gd:18-19`）与落格弹跳峰值（`:109` 的 +0.3）
    pub const TOKEN_R: f32 = 0.5;
    pub const TOKEN_REST_Y: f32 = 0.8;
    pub const BOUNCE: f32 = 0.3;
    /// sprite 棋子的身高（世界单位）。`cell_size` 是 2.0，所以一个人站在自己那一格里
    /// 占大半个格边 —— §7.5 末那句「1.0 缩放在 `CAM_SIZE=8` 下读不出脸」定的是同一件事
    pub const TOKEN_SPRITE_H: f32 = 1.8;
    /// 脚底。球的 `TOKEN_REST_Y` 是**球心**（半径 0.5 → 触点 0.3），而人要从脚底长出来，
    /// 两者差的那 0.6 就是「站在地上」与「浮在地上」的差别
    pub const TOKEN_FEET_Y: f32 = 0.2;
    /// 待机横条放帧的帧率。`AutoNoVsync` 下按帧计数会跑到看不清，所以按墙钟（见 `token`）。
    /// **只管待机**：在动的那两条按「格」走钟，见下面两个数
    pub const SPRITE_FPS: f32 = 8.0;
    /// 步行横条一个循环跨几格。两条腿一个循环（左右各迈一次），所以是 2 而不是 1：
    /// 按 1 算就是把整条循环塞进 0.18s，腿快成缝纫机
    pub const WALK_STEPS: f32 = 2.0;
    /// 载具横条一个循环跨几格。轮子那几条只有 4 帧（辐条相位），一步走完一圈正好
    pub const WHEEL_STEPS: f32 = 1.0;
    /// 纸片朝左↔朝右翻一整趟（+1 → -1）的时长。转向卡那个「纸片反转」就是这个速度；
    /// 快了看不出翻面，慢了拐角处的朝向追不上脚步
    pub const FLIP_SECS: f32 = 0.15;
    /// 入狱/出狱/瞬移的位移时长（`player_token.gd:55,65`）
    pub const PARK_SECS: f32 = 0.4;
    /// 格名浮在格子上方的高度（`tile_view.gd:151`）
    pub const LABEL_Y: f32 = 0.5;
    /// 相机（`board.gd:30-31`）：视线 (8,16,8) 即 (1,2,1) 对角，正交视野高 8
    pub const CAM_OFFSET: Vec3 = Vec3::new(8.0, 16.0, 8.0);
    pub const CAM_SIZE: f32 = 8.0;
    /// 正交投影下距离只影响裁剪面，取够远以免棋盘被近/远平面切掉
    pub const CAM_DIST: f32 = 40.0;
    /// 相机跟随的收敛时间常数（`board.gd:472` 是 0.35s 补间）
    pub const CAM_FOLLOW_TAU: f32 = 0.12;
}

/// 配色。数值取自 Godot 端同一段代码，免得「两边看起来不一样」其实是配色差。
pub mod palette {
    use bevy::prelude::*;
    use game_core::board::TileKind;

    /// `board.gd:181` 的 `Environment.background_color`
    pub const SKY: Color = Color::srgb(0.6, 0.6, 0.7);
    /// `board.gd:258` 的路面
    pub const ROAD: Color = Color::srgb(0.28, 0.28, 0.32);
    /// 沥青均值：从 `assets/textures/road/units/straight_orth.png` 的沥青带量出来的
    /// （中线 v=0.5 那条是白的，所以量的是 v∈0.2..0.4 那一带）。转角盘用它。
    ///
    /// 为什么不贴图：把那张图的沥青块放大到 1.5 格，噪声会粗到像一块脏抹布（试过一版）；
    /// 而 `ROAD` 偏蓝、与照片差一档。这个值是两者之间的折中，改它要连截图一起看。
    pub const ASPHALT: Color = Color::srgb(0.31, 0.305, 0.30);
    /// 草地垫底的纯色（素材缺失时用它，读起来像草地/地皮）
    pub const FILLER_A: Color = Color::srgb(0.34, 0.42, 0.3);

    /// `tile_view.gd:11-25` 的 `TYPE_COLORS`
    pub fn tile(kind: TileKind) -> Color {
        match kind {
            TileKind::Property => Color::srgb(0.3, 0.6, 0.9),
            TileKind::Bank => Color::srgb(0.5, 0.85, 0.5),
            TileKind::Stock => Color::srgb(0.85, 0.5, 0.85),
            TileKind::Fate => Color::srgb(0.85, 0.4, 0.4),
            // 卡片格（11-04）：琥珀一档，与 fate 的砖红、shop 的玫瑰、lottery 的黄都分得开
            TileKind::Card => Color::srgb(0.92, 0.66, 0.32),
            TileKind::Jail => Color::srgb(0.6, 0.6, 0.65),
            TileKind::Hospital => Color::srgb(0.9, 0.9, 0.95),
            TileKind::Lottery => Color::srgb(0.95, 0.85, 0.4),
            TileKind::GodDeity => Color::srgb(0.7, 0.5, 0.95),
            // 中立格（P2c 起是「公园」）：真正的什么都不发生，颜色按绿地取
            TileKind::Neutral => Color::srgb(0.38, 0.6, 0.42),
            TileKind::Road => Color::srgb(0.5, 0.5, 0.5),
            TileKind::Transport => Color::srgb(0.5, 0.7, 0.7),
            // 点券箱没有 Godot 对照（`TYPE_COLORS` 里没这一格）， mint 是与青绿路面
            // （transport）、紫（stock）、黄（start/lottery）都不撞的那一档
            TileKind::Coupon => Color::srgb(0.35, 0.9, 0.75),
            // 百货公司同样没有 Godot 对照。玫瑰一档：与 stock 的紫红差在蓝分量上，
            // 与 fate 的砖红差在绿分量上，两个都是「暖色但不同族」，扫一眼分得开
            TileKind::Shop => Color::srgb(0.9, 0.55, 0.65),
            // 游乐场（P7）也没有对照。青柠一档：它是这一盘上唯一「黄绿」的格子 ——
            // bank 的绿偏蓝（0.5,0.85,0.5）、coupon 的 mint 偏青（0.35,0.9,0.75），
            // 而这一格要读起来像「玩」，所以往黄那边推满
            TileKind::Arcade => Color::srgb(0.8, 0.95, 0.3),
            // 公司门面（P10d①）也没有 Godot 对照。钢青一档：与 property 的蓝差在灰度上
            // （它是暗蓝、不是「可购买」的亮蓝），与 transport 的青灰差在饱和度上 ——
            // 公司不是任何人的产业，颜色上就要读出「这格不属于可拥有那一族」
            TileKind::Company => Color::srgb(0.3, 0.42, 0.62),
            // 新闻中心（P11）也没有 Godot 对照。天青一档：它与 property 的亮蓝差在绿与明度上
            // （公告板是浅的、地契是深的），与 transport 的青灰差在蓝分量上，与 coupon 的
            // mint 差在「偏蓝还是偏绿」。选浅而不选艳，是因为这一格上屏的是**标题**（顶栏），
            // 路面只需要说「这是公共设施」，不需要抢那条播报的眼球
            TileKind::News => Color::srgb(0.62, 0.78, 0.9),
            // 魔法屋（P13）也没有 Godot 对照。深紫一档：这一族已经有三格紫红了 ——
            // `stock` 的亮紫红（0.85,0.5,0.85）、`GodDeity` 的亮紫（0.7,0.5,0.95），
            // 而女巫那一格要读起来比那两位**沉**：神明是刷在路上、要你踩的一闪而过的东西，
            // 魔法屋是盘上一栋固定的建筑。所以往暗与往红两侧各推一格，亮度压到 0.5 以下
            TileKind::Magic => Color::srgb(0.52, 0.28, 0.66),
        }
    }

    /// `land_view.gd:152-163` 的同色组配色。前八个是演示盘留下的旧键；
    /// 后十六个是 P2c 台湾盘的十六条街 —— 相邻街的色相至少隔一档，否则整盘看过去
    /// 只剩「一大片彩色」，街与街的边界要靠数格子才分得出来。
    pub fn group(group: &str) -> Color {
        match group {
            "red" => Color::srgb(0.9, 0.25, 0.25),
            "yellow" => Color::srgb(0.95, 0.85, 0.2),
            "green" => Color::srgb(0.3, 0.8, 0.4),
            "blue" => Color::srgb(0.3, 0.5, 0.95),
            "brown" => Color::srgb(0.55, 0.35, 0.2),
            "cyan" => Color::srgb(0.3, 0.85, 0.9),
            "purple" => Color::srgb(0.65, 0.3, 0.8),
            "orange" => Color::srgb(0.95, 0.6, 0.15),
            "keelung" => Color::srgb(0.35, 0.45, 0.62),
            "taipei" => Color::srgb(0.88, 0.28, 0.3),
            "taoyuan" => Color::srgb(0.9, 0.55, 0.2),
            "hsinchu" => Color::srgb(0.92, 0.75, 0.2),
            "miaoli" => Color::srgb(0.62, 0.75, 0.25),
            "taichung" => Color::srgb(0.35, 0.72, 0.35),
            "changhua" => Color::srgb(0.25, 0.68, 0.55),
            "yunlin" => Color::srgb(0.25, 0.62, 0.72),
            "chiayi" => Color::srgb(0.3, 0.48, 0.88),
            "tainan" => Color::srgb(0.55, 0.35, 0.85),
            "kaohsiung" => Color::srgb(0.78, 0.3, 0.72),
            "pingtung" => Color::srgb(0.9, 0.45, 0.6),
            "taitung" => Color::srgb(0.72, 0.5, 0.25),
            "hualien" => Color::srgb(0.45, 0.55, 0.35),
            "yilan" => Color::srgb(0.4, 0.7, 0.6),
            "tamsui" => Color::srgb(0.6, 0.6, 0.75),
            _ => Color::srgb(0.6, 0.6, 0.6),
        }
    }
}

/// 与 Godot 一致：材质全 unshaded，不依赖光照（`board.gd` 里每个 `StandardMaterial3D` 都是
/// `SHADING_MODE_UNSHADED`），否则单一光源会把色组颜色压成看不出区别的暗块。
pub fn flat(mats: &mut Assets<StandardMaterial>, c: Color) -> Handle<StandardMaterial> {
    mats.add(StandardMaterial {
        base_color: c,
        unlit: true,
        ..default()
    })
}

/// 带纸面的材质：`base_color` 出配色，纸出形与纹。素材缺席就原样退回 [`flat`]。
///
/// `AlphaMode::Mask` 而不是 `Blend`：纸卡四角是「要么透明要么不透明」，Mask 让这些方块
/// 留在不透明通道里写深度 —— 几百格走混合通道就要按视深排序，而排错的症状是地面忽隐忽现。
///
/// 一张图会糊在 `Cuboid` 的**六个面**上（Bevy 的 `CuboidMeshBuilder` 每面 UV 都是 0..1、
/// 只有一个材质槽）。对素面纸卡这是可接受的：侧面只有 `FLOOR_H` 高，压扁后露出的是画面
/// 中段那条纯纸面，读起来就是「卡片的纸边」。真要「顶面贴图 + 侧面纯色」得拆几何，
/// 等这张在板上看过了再定值不值那一刀。
pub fn plated(
    mats: &mut Assets<StandardMaterial>,
    c: Color,
    paper: &Option<Handle<Image>>,
) -> Handle<StandardMaterial> {
    let Some(image) = paper else {
        return flat(mats, c);
    };
    mats.add(StandardMaterial {
        base_color: c,
        base_color_texture: Some(image.clone()),
        unlit: true,
        alpha_mode: AlphaMode::Mask(0.5),
        ..default()
    })
}

/// 一张**不乘色**的贴图材质：`base_color` 留白，图原样上屏。道路单元与粉笔框用它，
/// 因为它们的颜色已经画在图里（草坪色调是「无缝」的前提，被乘一下就断了）。
///
/// `cutout` 是「透明底要挖掉」：粉笔是画在透明底上的线，不挖就会有一块绿框压在草地上。
pub fn photo(
    mats: &mut Assets<StandardMaterial>,
    image: &Handle<Image>,
    cutout: bool,
) -> Handle<StandardMaterial> {
    mats.add(StandardMaterial {
        base_color: Color::WHITE,
        base_color_texture: Some(image.clone()),
        unlit: true,
        alpha_mode: if cutout {
            AlphaMode::Mask(0.5)
        } else {
            AlphaMode::Opaque
        },
        ..default()
    })
}

/// Godot 的 `Color.darkened(amount)`：在 sRGB 分量上按比例压暗，不是线性空间混合。
pub fn darkened(c: Color, amount: f32) -> Color {
    let s = c.to_srgba();
    let k = 1.0 - amount;
    Color::srgba(s.red * k, s.green * k, s.blue * k, s.alpha)
}

/// 绕 +Y 旋转，把实体的局部 +X 轴对齐到 `dir`。
///
/// Bevy 与 Godot 都是 Y-up 右手系，`from_rotation_y(a)` 把 +X 送到 `(cos a, 0, -sin a)`，
/// 于是 `a = atan2(-dz, dx)` —— 与 `board.gd:248` 同式。
pub fn yaw_to_face(dir: Vec3) -> f32 {
    (-dir.z).atan2(dir.x)
}

/// sprite 棋子该朝向哪儿：**正对相机**，不是正对 +Z。
///
/// 相机是正交的，所以这条朝向对全盘每一格都成立（不必逐格算视线）。从 `CAM_OFFSET`
/// 现算而不是写死 45°/54.7°：那是同一颗常数，改了相机而 sprite 还按旧角度立着，
/// 症状是人被压扁成一条而不报错。
///
/// 顺序有讲究：先把矩形从「躺平」绕 X 仰起到正对相机，再绕 Y 转到相机的方位角。
/// 两个乘数换位置就是绕**已转过的** X 仰起，人会歪着站。
pub fn billboard_rot() -> Quat {
    let o = dim::CAM_OFFSET;
    let yaw = o.x.atan2(o.z);
    let elev = o.y.atan2(Vec2::new(o.x, o.z).length());
    Quat::from_rotation_y(yaw) * Quat::from_rotation_x(-elev)
}
