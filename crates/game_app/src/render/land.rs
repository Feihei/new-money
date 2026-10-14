//! 土地视觉：地块底座 + 按等级堆叠的建筑方体。对应 Godot `land_view.gd`。
//!
//! 建筑等级是规则层的事实（`BuildingLevel` 组件），这里只是把它画出来：
//! 一次铺满 `GameRules.max_building_level` 个方体，等级只决定露出几层 ——
//! 于是「升级」这条路径上没有 spawn/despawn 的簿记，也就没有漏清的空间。

use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, BuildingLevel, GameRules, LandId, Owner, PlayerColor, PlayerId,
};

use super::{Rendered, darkened, dim, flat, palette};

/// 挂在 Land 实体上（Land 实体本身没有 `Transform`，方体是各自摆好世界坐标的独立实体）。
#[derive(Component)]
pub(crate) struct LandVisual {
    slabs: Vec<Entity>,
    /// 归属色写在一份共享给本地块所有方体的材质上：换主人只改这一个材质
    owner_mat: Handle<StandardMaterial>,
}

/// 给每块地铺底座与一池隐藏的建筑方体。
pub(crate) fn attach_land_visuals(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    rules: Res<GameRules>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    lands: Query<(Entity, &LandId)>,
) {
    let b = &active.def;
    for (entity, id) in &lands {
        // `BoardDef::validate` 保证土地有定义且坐标在网格内；拿不到就是数据错，不画即可
        let (Some(def), Some(center)) = (b.land(id.0), b.land_world_pos(id.0)) else {
            continue;
        };
        let footprint = b.cell_size * def.size as f32;
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(
                footprint * dim::PLOT_SCALE,
                dim::PLOT_H,
                footprint * dim::PLOT_SCALE,
            ))),
            MeshMaterial3d(flat(
                &mut mats,
                darkened(
                    def.group
                        .as_deref()
                        .map(palette::group)
                        .unwrap_or(Color::srgb(0.6, 0.6, 0.6)),
                    0.55,
                ),
            )),
            Transform::from_xyz(center.x, dim::PLOT_H / 2.0, center.z),
            Rendered,
        ));

        let w = footprint * dim::BUILD_SCALE;
        let owner_mat = flat(&mut mats, Color::WHITE);
        let mut slabs = Vec::new();
        for i in 0..rules.max_building_level {
            let y = dim::PLOT_H + i as f32 * (dim::SLAB_H + dim::SLAB_GAP) + dim::SLAB_H / 2.0;
            let slab = commands
                .spawn((
                    Mesh3d(meshes.add(Cuboid::new(w, dim::SLAB_H, w))),
                    MeshMaterial3d(owner_mat.clone()),
                    Transform::from_xyz(center.x, y, center.z),
                    Visibility::Hidden,
                    Rendered,
                ))
                .id();
            slabs.push(slab);
        }
        commands
            .entity(entity)
            .insert(LandVisual { slabs, owner_mat });
    }
}

/// 归属与等级是唯一的真相来源，这里只做变化驱动的同步（`Ref<T>` 变化检测，不轮询）。
pub(crate) fn sync_buildings(
    mut commands: Commands,
    lands: Query<(Ref<Owner>, Ref<BuildingLevel>, &LandVisual)>,
    players: Query<(&PlayerId, &PlayerColor)>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    for (owner, level, visual) in &lands {
        if !owner.is_changed() && !level.is_changed() {
            continue;
        }
        // 归属色来自玩家实体，而不是按 PlayerId 取模 —— 阵容颜色是 `GameSetup` 定的事实
        let owned = owner
            .0
            .and_then(|pid| players.iter().find(|(p, _)| **p == pid).map(|(_, c)| *c));
        if let Some(PlayerColor([r, g, b])) = owned
            && let Some(mut mat) = mats.get_mut(&visual.owner_mat)
        {
            mat.base_color = Color::srgb(r, g, b);
        }
        for (i, slab) in visual.slabs.iter().enumerate() {
            commands.entity(*slab).insert(if (i as u8) < level.0 {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
        }
    }
}
