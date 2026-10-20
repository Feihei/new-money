//! 道路放置物的图标（P5）：路障、地雷、定时炸弹、恶犬、宝箱。
//!
//! 规则层的 `Occupancy` 是挂在**主环格坐标**上的运行时实体（ADR-002），道路格本身
//! 照旧不建实体。这一层只回答「那个实体画在哪儿」：在地上的画在格子中心，被人捡起的
//! 定时炸弹画在携带者棋子上方 —— 它不再占格，但仍是一件要看得见的东西。
//!
//! 图标是程序化几何（同棋子与建筑）：美术资源还没有，形状与颜色先让五种东西彼此
//! 分得开。表现层只读 `Occupancy`，不向 core 发任何推进消息。

use bevy::prelude::*;
use game_core::prelude::{ActiveBoard, BoardDef, God, Occupancy, OccupancyAt, OccupancyKind};

use super::{Rendered, flat, token::TokenVisual};

/// 一枚图标对应的规则实体。改 `Occupancy` 的位置（炸弹被捡起/转让）时，
/// 这里跟着换锚点；实体被 despawn（宝箱打开、地雷踩掉）时，这里跟着消失。
#[derive(Component)]
pub(crate) struct OccupancyVisual {
    occupancy: Entity,
}

/// 一枚图标在格子上的落脚高度：路障横杆抬到膝盖、地雷贴着地、宝箱半身。
///
/// 神是这些人形里唯一「站在路上」的，所以它的锚点比道具高一档 —— 画在格心会读成
/// 「地上一件东西」，而它是要被踩的那一位。
fn icon_y(kind: OccupancyKind) -> f32 {
    match kind {
        OccupancyKind::Roadblock => 0.55,
        OccupancyKind::Mine => 0.28,
        OccupancyKind::TimeBomb => 0.45,
        OccupancyKind::Dog => 0.42,
        OccupancyKind::Chest => 0.32,
        OccupancyKind::God(_) => 0.78,
    }
}

/// 十三位现在是十二位（死神不自己上街，见 `gods::roams`），今天先按**族**分色：
/// 财神金、福神粉、穷神灰蓝、衰神暗紫、天使白、恶魔黑红、土地公土黄。
/// 正式造型是剪纸 sprite（CHARACTERS §十二），这一层只是让截图里分得开谁是谁。
fn icon_color(kind: OccupancyKind) -> Color {
    match kind {
        OccupancyKind::Roadblock => Color::srgb(0.95, 0.55, 0.15),
        OccupancyKind::Mine => Color::srgb(0.28, 0.28, 0.32),
        OccupancyKind::TimeBomb => Color::srgb(0.85, 0.20, 0.20),
        OccupancyKind::Dog => Color::srgb(0.55, 0.35, 0.20),
        OccupancyKind::Chest => Color::srgb(0.95, 0.80, 0.30),
        OccupancyKind::God(god) => match god {
            God::BigRich | God::SmallRich => Color::srgb(0.92, 0.75, 0.22),
            God::BigFortune | God::SmallFortune => Color::srgb(0.98, 0.62, 0.75),
            God::BigPoor | God::SmallPoor => Color::srgb(0.42, 0.48, 0.58),
            God::BigDoom | God::SmallDoom => Color::srgb(0.45, 0.22, 0.42),
            God::Angel => Color::srgb(0.95, 0.96, 1.0),
            God::Devil => Color::srgb(0.30, 0.10, 0.12),
            God::LandDeity => Color::srgb(0.62, 0.45, 0.26),
            God::Death => Color::srgb(0.86, 0.86, 0.80),
        },
    }
}

/// 形状先让六种东西分得开：横杆 / 扁球 / 球 / 长条 / 方块 / 立着的柱。
fn icon_mesh(meshes: &mut Assets<Mesh>, kind: OccupancyKind) -> Handle<Mesh> {
    match kind {
        OccupancyKind::Roadblock => meshes.add(Cuboid::new(0.82, 0.16, 0.24)),
        OccupancyKind::Mine => meshes.add(Sphere::new(0.22)),
        OccupancyKind::TimeBomb => meshes.add(Sphere::new(0.26)),
        OccupancyKind::Dog => meshes.add(Cuboid::new(0.32, 0.34, 0.62)),
        OccupancyKind::Chest => meshes.add(Cuboid::new(0.58, 0.38, 0.44)),
        // 一位神 = 一根竖着的窄柱。它与宝箱一样是方块，但**立着**：
        // 平放读作货物，立着才读作「站在那里的一位」
        OccupancyKind::God(_) => meshes.add(Cuboid::new(0.3, 0.66, 0.3)),
    }
}

/// 这枚图标此刻的世界坐标。`None` = 暂时画不出来（棋盘上没有那一格、携带者不在）。
fn anchor(
    def: &BoardDef,
    occ: &Occupancy,
    tokens: &Query<(&TokenVisual, &Transform)>,
) -> Option<Vec3> {
    match occ.at {
        OccupancyAt::OnTile(tile) => def
            .tile_world_pos(tile.0 as i64)
            .map(|at| Vec3::new(at.x, icon_y(occ.kind), at.z)),
        // 携带中的炸弹浮在棋子上方：它已经不是路上的一格了，再画在格心会骗人
        OccupancyAt::Carried { by, .. } => tokens
            .iter()
            .find(|(visual, _)| visual.player == by)
            .map(|(_, at)| at.translation + Vec3::new(0.0, 0.95, 0.0)),
    }
}

/// 同步图标：新增的 spawn、消失的 despawn、还在的每帧更新锚点。
///
/// 不按「变化检测」省这一步：棋子每帧在插值，携带中的炸弹必须跟着走；而全场
/// 放置物只有个位数，每帧线性扫一遍比维护一份索引便宜（ADR-002 §3 也明确不要第二份索引）。
pub(crate) fn sync_occupancy_visuals(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    occupancies: Query<(Entity, &Occupancy)>,
    // `Without<TokenVisual>`：图标与棋子都要读 `Transform`，两条查询同场必须显式
    // 说明它们碰的不是同一批实体（Bevy B0001），否则装配期直接 panic
    mut shown: Query<
        (Entity, &OccupancyVisual, &mut Transform, &mut Visibility),
        Without<TokenVisual>,
    >,
    tokens: Query<(&TokenVisual, &Transform)>,
) {
    let def = &active.def;

    // 先清：规则实体没了（踩掉/捡走/爆掉），图标跟着走
    for (visual_entity, visual, _, _) in &shown {
        if occupancies.get(visual.occupancy).is_err() {
            commands.entity(visual_entity).despawn();
        }
    }

    // 再补：新出现的放置物各 spawn 一枚
    for (occ_entity, occ) in &occupancies {
        if shown.iter().any(|(_, v, _, _)| v.occupancy == occ_entity) {
            continue;
        }
        let Some(at) = anchor(def, occ, &tokens) else {
            // 棋盘上还没有那一格（数据错）或携带者还不在：这一帧不画，下一帧再试。
            // 不在这里 panic —— 表现层不该因为一个找不到的格号把整局干掉
            continue;
        };
        commands.spawn((
            OccupancyVisual {
                occupancy: occ_entity,
            },
            Mesh3d(icon_mesh(&mut meshes, occ.kind)),
            MeshMaterial3d(flat(&mut mats, icon_color(occ.kind))),
            Transform::from_translation(at),
            Rendered,
        ));
    }

    // 最后跟着走：在地上的不动，被捡起的炸弹跟着棋子
    for (_, visual, mut transform, mut visibility) in &mut shown {
        let Ok((_, occ)) = occupancies.get(visual.occupancy) else {
            continue;
        };
        match anchor(def, occ, &tokens) {
            Some(at) => {
                transform.translation = at;
                *visibility = Visibility::Inherited;
            }
            // 锚点暂时算不出来：藏起来而不是留在上一帧的位置上
            None => *visibility = Visibility::Hidden,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_has_its_own_colour() {
        let mut colors: Vec<[u32; 3]> = Vec::new();
        for kind in [
            OccupancyKind::Roadblock,
            OccupancyKind::Mine,
            OccupancyKind::TimeBomb,
            OccupancyKind::Dog,
            OccupancyKind::Chest,
        ] {
            let c = icon_color(kind).to_srgba();
            colors.push([
                (c.red * 1000.0) as u32,
                (c.green * 1000.0) as u32,
                (c.blue * 1000.0) as u32,
            ]);
        }
        let mut unique = colors.clone();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), colors.len(), "两种放置物用了同一个颜色");
    }
}
