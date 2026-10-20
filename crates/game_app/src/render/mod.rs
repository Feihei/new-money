//! 表现层（B4 渲染切片）：把规则层的实体与消息画成等轴测棋盘。
//!
//! 分层红线（`.agents/ARCHITECTURE.md` §4.6）：这一层只**读** core 的状态与消息，
//! 从不向 core 发「推进回合」的消息 —— 逻辑位置与步时归 core，棋子在这里只是插值出来的球。
//! 唯一的例外是「玩家意图」那两条（`RequestRoll`/`DecisionChosen`），它们的发送者是输入侧：
//! B4 期间是 `crate::autopilot`，B5 换成按钮、B6 换成 AI，都不是本模块。

pub mod board;
pub mod camera;
pub mod labels;
pub mod land;
pub mod occupancy;
pub mod token;

use bevy::prelude::*;
use game_core::prelude::ActiveBoard;

/// 表现层自己 spawn 的实体标记（棋盘几何、土地视觉、棋子、格名、相机、弹层底板）。
/// 回主菜单时按它一次性清场 —— 规则层的实体由 `game_core::Gameplay` 负责，两边互不知晓。
#[derive(Component)]
pub struct Rendered;

/// 一块可点的道路格地板：它的网格坐标。
///
/// 只带坐标、不带格名与类型 —— 那些由 `BoardDef::tile_at_grid` 现查（§4.2）。
/// 只有地板挂它，加上 `MeshPickingSettings::require_markers = true`，
/// 于是点建筑、点棋子都不会误开详情卡。
#[derive(Component)]
pub struct TilePick {
    pub col: u32,
    pub row: u32,
}

/// 开发期 CJK 字体句柄。`assets/font/dev_*` 是本地开发副本（不入库，见 AGENTS.md 硬约束 7），
/// Bevy 没有 Godot 那套内建中文回退，不显式指定字体就是豆腐块（§六 坑 2）。
#[derive(Resource)]
pub struct DevFont(pub Handle<Font>);

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
        app.add_systems(
            Update,
            (
                (
                    board::spawn_board_geometry,
                    land::attach_land_visuals,
                    token::spawn_token_visuals,
                    camera::spawn_camera_rig,
                    labels::spawn_tile_labels,
                )
                    .in_set(RenderSet::Boot),
                (
                    token::animate_tokens,
                    land::sync_buildings,
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
    use bevy::math::Vec3;

    /// 道路格地板：整格密铺（`tile_view.gd:94,97`）
    pub const FLOOR_H: f32 = 0.25;
    pub const FLOOR_Y: f32 = -0.1;
    /// 路面（`board.gd:244` 高 0.12、`board.gd:253` 宽 = 格边长 × 0.5）
    pub const ROAD_H: f32 = 0.12;
    pub const ROAD_W: f32 = 0.5;
    /// 同色组标记条（`tile_view.gd:111-114`）
    pub const STRIP_H: f32 = 0.08;
    pub const STRIP_DEPTH: f32 = 0.5;
    pub const STRIP_Y: f32 = 0.16;
    pub const STRIP_OFF: f32 = 0.4;
    /// 土地底座与建筑方体（`land_view.gd:19-23`）
    pub const PLOT_H: f32 = 0.3;
    pub const SLAB_H: f32 = 0.4;
    pub const SLAB_GAP: f32 = 0.12;
    pub const PLOT_SCALE: f32 = 0.96;
    pub const BUILD_SCALE: f32 = 0.84;
    /// 棋子（`player_token.gd:18-19`）与落格弹跳峰值（`:109` 的 +0.3）
    pub const TOKEN_R: f32 = 0.5;
    pub const TOKEN_REST_Y: f32 = 0.8;
    pub const BOUNCE: f32 = 0.3;
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
    /// `board.gd:298-300` 的双色地面（读起来像草地/地皮）
    pub const FILLER_A: Color = Color::srgb(0.34, 0.42, 0.3);
    pub const FILLER_B: Color = Color::srgb(0.3, 0.38, 0.27);

    /// `tile_view.gd:11-25` 的 `TYPE_COLORS`
    pub fn tile(kind: TileKind) -> Color {
        match kind {
            TileKind::Start => Color::srgb(1.0, 0.85, 0.2),
            TileKind::Property => Color::srgb(0.3, 0.6, 0.9),
            TileKind::Bank => Color::srgb(0.5, 0.85, 0.5),
            TileKind::Stock => Color::srgb(0.85, 0.5, 0.85),
            TileKind::Chance => Color::srgb(0.95, 0.7, 0.3),
            TileKind::Fate => Color::srgb(0.85, 0.4, 0.4),
            TileKind::Jail => Color::srgb(0.6, 0.6, 0.65),
            TileKind::Hospital => Color::srgb(0.9, 0.9, 0.95),
            TileKind::Lottery => Color::srgb(0.95, 0.85, 0.4),
            TileKind::GodDeity => Color::srgb(0.7, 0.5, 0.95),
            TileKind::Neutral => Color::srgb(0.4, 0.4, 0.45),
            TileKind::Road => Color::srgb(0.5, 0.5, 0.5),
            TileKind::Transport => Color::srgb(0.5, 0.7, 0.7),
            // 点券箱没有 Godot 对照（`TYPE_COLORS` 里没这一格）， mint 是与青绿路面
            // （transport）、紫（stock）、黄（start/lottery）都不撞的那一档
            TileKind::Coupon => Color::srgb(0.35, 0.9, 0.75),
            // 百货公司同样没有 Godot 对照。玫瑰一档：与 stock 的紫红差在蓝分量上，
            // 与 chance 的橙差在绿分量上，两个都是「暖色但不同族」，扫一眼分得开
            TileKind::Shop => Color::srgb(0.9, 0.55, 0.65),
        }
    }

    /// `land_view.gd:152-163` 的同色组配色（比 Godot 多出的 brown/cyan/purple/orange 也在表里）
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
