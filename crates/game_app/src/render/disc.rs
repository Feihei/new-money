//! 特殊格的薄饼标记：一块扁圆柱坐在草地上，顶面贴一张圆形剪纸图标。
//!
//! 撤掉同色组色条（11-04）之后，「这一格不是地、踩上去有事」这句话只剩地面本身在说，
//! 而无缝地面恰恰把它说得最弱 —— 路格与路格长得一样。这一层把非地块格重新抬出来：
//! **底下那格贴的是什么图无所谓，饼会把它挡住**，所以地面词汇表不必为特殊格再长一套。
//!
//! 分两半画是因为 Bevy 的 `Cylinder` 只有一个材质槽（§六 坑 30 的同一件事）：
//! 侧壁要的是纯色纸边，顶面要的是那张带透明的图标，一张图盖不满六面也盖不满侧面。
//! 顶面那块圆片压在圆柱上表面之上 `CHALK_LIFT`，与粉笔框用同一个「够躲开 z 竞争、
//! 肉眼看不出浮着」的高度。

use bevy::prelude::*;
use game_core::board::TileKind;
use game_core::prelude::ActiveBoard;

use super::{DiscIcons, Rendered, darkened, dim, flat, geom::BoardGeom, palette, photo};

/// 十六个键，与 [`disc_key`] 一一对应。`DiscIcons::load` 按它逐键问盘上有没有。
///
/// 写两遍不是冗余：这张表是「该去盘上找哪些文件」，`disc_key` 是「这一格该用哪张」，
/// 后者的穷尽匹配由编译器保证覆盖全部格类型，前者由单测保证与后者不发散。
pub(crate) const KEYS: [&str; 16] = [
    "bank",
    "stock",
    "fate",
    "card",
    "jail",
    "hospital",
    "lottery",
    "goddeity",
    "neutral",
    "transport",
    "coupon",
    "shop",
    "arcade",
    "company",
    "news",
    "magic",
];

/// 这一格该摆哪张圆片；`None` = 不摆。
///
/// **穷尽表**：新增格类型必须在这里当场表态（给键 = 要一张新图，给 `None` = 说清它靠
/// 什么表达），不给分支就编译失败。那两格 `None` 的理由写在各自那一行上。
pub(crate) fn disc_key(kind: TileKind) -> Option<&'static str> {
    match kind {
        // 地块自己有一套视觉（底座 + 按等级堆的楼 + 粉笔框），不需要饼
        TileKind::Property => None,
        // 普通路格：踩上去什么都不发生，摆一块饼等于在说「这里有事」
        TileKind::Road => None,
        TileKind::Bank => Some("bank"),
        TileKind::Stock => Some("stock"),
        TileKind::Fate => Some("fate"),
        TileKind::Card => Some("card"),
        TileKind::Jail => Some("jail"),
        TileKind::Hospital => Some("hospital"),
        TileKind::Lottery => Some("lottery"),
        TileKind::GodDeity => Some("goddeity"),
        TileKind::Neutral => Some("neutral"),
        TileKind::Transport => Some("transport"),
        TileKind::Coupon => Some("coupon"),
        TileKind::Shop => Some("shop"),
        TileKind::Arcade => Some("arcade"),
        TileKind::Company => Some("company"),
        TileKind::News => Some("news"),
        TileKind::Magic => Some("magic"),
    }
}

/// 圆片躺平后的朝向：让图标的「上」对着屏幕上方。
///
/// 等轴测相机在 `(8, 16, 8)`，把世界 +Y 投影到视平面上，地面平面上「往上走」那个方向
/// 是 `(-1, 0, -1)`。圆片建在 XY 平面（图的上方 = 局部 +Y），`rotate_x(-90°)` 之后
/// 局部 +Y 落在世界 -Z，再绕 Y 转 +45° 才正好落到 `(-X, -Z)`。
/// 转错不会报错，只会让全部图标横躺 —— 汽车侧视图是最容易看出来的一张。
fn icon_up() -> Quat {
    Quat::from_rotation_y(std::f32::consts::FRAC_PI_4)
        * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2)
}

/// 给主环上每一格「有事」的格子摆一块饼。**只摆主环**：支线尽头已经有楼
/// （[`super::board::spawn_branch_buildings`]），饼压在那座楼底下只会糊出一圈边。
pub(crate) fn spawn_disc_markers(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    discs: Res<DiscIcons>,
    geom: Res<BoardGeom>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    let b = &active.def;
    let r = b.cell_size * dim::DISC_R;
    // 网格各一份，材质按格类型出（同一类型的几格共用一份）：饼的数量是几十块，
    // 逐格建网格不会更省，只会让「回菜单清场」那一趟多遍历几百个实体
    let side = meshes.add(Cylinder::new(r, dim::DISC_H));
    let top = meshes.add(Circle::new(r));
    let mut total = 0;
    let mut with_icon = 0;
    for t in &b.tiles {
        let Some(key) = disc_key(t.kind) else {
            continue;
        };
        let pos = geom.at(t.col, t.row).at;
        commands.spawn((
            Mesh3d(side.clone()),
            MeshMaterial3d(flat(&mut mats, darkened(palette::tile(t.kind), 0.35))),
            Transform::from_xyz(pos.x, dim::DISC_Y, pos.z),
            Rendered,
        ));
        if let Some(icon) = discs.get(key) {
            commands.spawn((
                Mesh3d(top.clone()),
                MeshMaterial3d(photo(&mut mats, icon, true)),
                Transform::from_xyz(
                    pos.x,
                    dim::DISC_Y + dim::DISC_H / 2.0 + dim::CHALK_LIFT,
                    pos.z,
                )
                .with_rotation(icon_up()),
                Rendered,
            ));
            with_icon += 1;
        }
        total += 1;
    }
    info!("特殊格薄饼：{total} 块（顶面图标 {with_icon} 张）");
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 全部格类型。新增一位时这里要加一行 —— 而 `disc_key` 的穷尽匹配会先一步编译失败，
    /// 所以这张表漏改不会静默，只会在那一臂之后由下面两条断言指出「表和匹配对不上了」。
    const ALL: [TileKind; 18] = [
        TileKind::Property,
        TileKind::Bank,
        TileKind::Stock,
        TileKind::Fate,
        TileKind::Card,
        TileKind::Jail,
        TileKind::Hospital,
        TileKind::Lottery,
        TileKind::GodDeity,
        TileKind::Neutral,
        TileKind::Road,
        TileKind::Transport,
        TileKind::Coupon,
        TileKind::Shop,
        TileKind::Arcade,
        TileKind::Company,
        TileKind::News,
        TileKind::Magic,
    ];

    #[test]
    fn every_key_is_used_by_exactly_one_kind() {
        let used: Vec<&str> = ALL.iter().filter_map(|k| disc_key(*k)).collect();
        assert_eq!(used.len(), KEYS.len(), "表与匹配发散：{used:?}");
        for key in KEYS {
            assert_eq!(
                used.iter().filter(|k| **k == key).count(),
                1,
                "「{key}」被 {used:?} 用了不止一次或根本没人用"
            );
        }
    }

    #[test]
    fn only_the_two_kinds_with_their_own_visual_are_bare() {
        let bare: Vec<TileKind> = ALL
            .iter()
            .copied()
            .filter(|k| disc_key(*k).is_none())
            .collect();
        assert_eq!(bare, vec![TileKind::Property, TileKind::Road]);
    }

    #[test]
    fn the_icon_lies_flat_and_facing_the_camera() {
        // 躺平 = 法线朝上；朝上 = 图标的「上」落到地面平面的 (-X,-Z)
        let q = icon_up();
        let normal = q * Vec3::Z;
        assert!(normal.distance(Vec3::Y) < 1e-6, "圆片没躺平：{normal}");
        let up = q * Vec3::Y;
        let want = Vec3::new(-1.0, 0.0, -1.0).normalize();
        assert!(up.distance(want) < 1e-6, "图标朝向偏了：{up}");
    }
}
