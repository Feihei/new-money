//! 土地视觉：地块底座 + 按等级摆出的建筑。对应 Godot `land_view.gd`。
//!
//! 建筑等级是规则层的事实（`BuildingLevel` 组件），这里只是把它画出来。两条路径：
//!
//! - **住宅模型**（`model/dwell_L1..L5.glb` 五级齐全）：等级 1..=5 各摆一栋，只露当前那一栋。
//!   模型是 Z-up、原点在脚底，进对局先绕 X 立起来再缩（`dim::dwell_upright`/`DWELL_SCALE`）。
//!   归属不染模型（模型保持原色），而是再摆一份**反向外壳**做角色色描边：同一份场景、
//!   网格沿法线外扩一圈（`OutlineMeshes`，不是按原点放大），材质换成纯色且只画背面
//!   （`cull_mode: Front`），轮廓就从模型边缘露出来。
//! - **程序化方体**（素材不在盘上，或商业地）：一次铺满 `max_building_level` 个方体，
//!   等级只决定露出几层 —— 于是「升级」这条路径上没有 spawn/despawn 的簿记。
//!
//! 两条路径都**一次摆满、只切可见性**，这样升级不会产生「这栋旧的没清掉」的空间。

use std::collections::HashMap;

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use game_core::prelude::{
    BuildingLevel, GameRules, LandId, LandType, Owner, PlayerColor, PlayerId, PlayerName,
};

use super::geom::BoardRead;
use super::{
    BuildingModels, GroundArt, PropArt, Rendered, billboard_rot, darkened, dim, flat, palette,
    photo, plated,
};

/// 描边外壳沿法线外扩的距离（**模型单位**）。进对局再乘 `DWELL_SCALE`，所以五级楼的
/// 描边一样粗（1.7 倍后约 0.04 世界单位，`CAM_SIZE=8` 下约 3–4 像素）。
///
/// 不能用「把场景整体放大一点」来代替：放大是按原点算的，楼顶离原点远、描边就比侧面粗
/// 好几倍，高楼的顶边会变成一圈厚红边。
const OUTLINE_INFLATE: f32 = 0.024;

/// 挂在 Land 实体上（Land 实体本身没有 `Transform`，建筑都是各自摆好世界坐标的独立实体）。
#[derive(Component)]
pub(crate) struct LandVisual {
    /// 等级 1..=5 各一份「身体」，下标 = 等级 − 1。
    /// 模型路径下同时只露一个（换等级＝换一栋楼）；方体路径下是堆叠（露出前 `level` 个）。
    bodies: Vec<Entity>,
    /// 模型路径的描边外壳（与 `bodies` 同下标一一对应）；方体路径为空。
    outlines: Vec<Entity>,
    /// 方体路径的归属色材质（模型路径没有 —— 模型保持原色，归属靠描边）。
    owner_mat: Option<Handle<StandardMaterial>>,
    /// 模型路径的描边材质：一块地一份，换主人只改这一个。
    outline_mat: Option<Handle<StandardMaterial>>,
    /// `bodies` 是场景根（模型）而不是方体。同步时两者对等级的解释不同。
    scenic: bool,
    /// 空地上那件归属标志物的两张贴屏四边形：本体与归属色蒙版。
    /// 与建筑一样**一次摆好、只切可见性**，摆不摆由 `sync_buildings` 定。
    prop_body: Entity,
    prop_edge: Entity,
    /// 标志物的两张材质（一块地一份：换主人只换贴图与一个颜色）。
    prop_body_mat: Handle<StandardMaterial>,
    prop_edge_mat: Handle<StandardMaterial>,
    /// 现在贴的是谁的图。`None` = 这块地没有标志物（没主人／已盖楼／这位没素材）。
    prop_key: Option<&'static str>,
}

/// 挂在描边场景根上：等实例化完成，把它所有图元都换成纯色描边材质。
///
/// 为什么不在 spawn 时就换：`WorldAssetRoot` 的子孙要到 glTF 解码完才出现，
/// 而地块一开场就摆好了 —— 只能等它长出来再走一遍。
#[derive(Component)]
pub(crate) struct SceneOutline {
    mat: Handle<StandardMaterial>,
    patched: bool,
}

/// 描边用的外扩网格（源网格 → 外壳网格）。glb 里的网格被 75 块地共享，
/// 外扩只需要每个图元做一次，缓存起来全场复用。
#[derive(Resource, Default)]
pub(crate) struct OutlineMeshes {
    cache: HashMap<AssetId<Mesh>, Handle<Mesh>>,
}

impl OutlineMeshes {
    /// 取（或第一次造）这份网格的外壳。没有法线就返回 `None` —— 那说明素材不是我们的
    /// 生成器出的，硬造会把模型拉变形。
    fn inflated(&mut self, src: &Handle<Mesh>, meshes: &mut Assets<Mesh>) -> Option<Handle<Mesh>> {
        if let Some(hull) = self.cache.get(&src.id()) {
            return Some(hull.clone());
        }
        let mut hull = meshes.get(src)?.clone();
        let normals = match hull.attribute(Mesh::ATTRIBUTE_NORMAL) {
            Some(VertexAttributeValues::Float32x3(normals)) => normals.clone(),
            _ => {
                warn!("建筑模型 {:?} 没有 float32 法线，描边跳过", src.path());
                return None;
            }
        };
        // 硬边会把顶点按面拆开（同一个角上有多份、法线各不相同）。逐顶点沿自己的法线外扩
        // 会在棱上撕出缝 —— 缝正好落在描边该出现的地方。所以先把同一位置的顶点法线求平均，
        // 再让这几份一起沿平均法线走，棱才是连续的。
        let mut keys: HashMap<[i32; 3], Vec3> = HashMap::new();
        let quant = |p: &[f32; 3]| {
            [
                (p[0] * 100_000.0).round() as i32,
                (p[1] * 100_000.0).round() as i32,
                (p[2] * 100_000.0).round() as i32,
            ]
        };
        let Some(VertexAttributeValues::Float32x3(positions)) =
            hull.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            warn!("建筑模型 {:?} 没有 float32 顶点，描边跳过", src.path());
            return None;
        };
        for (p, n) in positions.iter().zip(&normals) {
            *keys.entry(quant(p)).or_default() += Vec3::from_array(*n);
        }
        if let Some(VertexAttributeValues::Float32x3(positions)) =
            hull.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        {
            for p in positions.iter_mut() {
                let dir = keys
                    .get(&quant(p))
                    .map(|sum| sum.normalize_or_zero())
                    .unwrap_or(Vec3::ZERO);
                let moved = Vec3::from_array(*p) + dir * OUTLINE_INFLATE;
                *p = moved.to_array();
            }
        }
        let hull = meshes.add(hull);
        self.cache.insert(src.id(), hull.clone());
        Some(hull)
    }
}

/// 给每块地铺底座与建筑。
pub(crate) fn attach_land_visuals(
    mut commands: Commands,
    board: BoardRead<'_>,
    rules: Res<GameRules>,
    art: GroundArt<'_>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    lands: Query<(Entity, &LandId)>,
) {
    let b = &board.active.def;
    let props = art.props.len();
    if props == 0 {
        info!("归属标志物：盘上没有 prop/*.png，空地的归属退回粉笔框 + 色块");
    } else {
        info!("归属标志物：{props} 件（缺图的角色在空地上不摆）");
    }
    // 地块只是「道路旁多出来的一块草地」，边界靠粉笔框说；草坪素材缺了才退回街色纸卡。
    let chalk_pair = art.chalk.pair();
    let chalk_quad = chalk_pair
        .map(|_| meshes.add(Rectangle::new(b.cell_size, b.cell_size)))
        .clone();
    // 归属标志物的四边形：与棋子同一套做法，把底边烘进顶点 —— 实体位置就是「它站在哪儿」。
    // 素材是 128²、物件底边对齐格底，所以四边形取正方形，贴图一比一落上去。
    let prop_mesh = {
        let mut mesh = Mesh::from(Rectangle::new(dim::PROP_H, dim::PROP_H));
        if let Some(VertexAttributeValues::Float32x3(positions)) =
            mesh.attribute_mut(Mesh::ATTRIBUTE_POSITION)
        {
            for p in positions.iter_mut() {
                p[1] += dim::PROP_H / 2.0;
            }
        }
        meshes.add(mesh)
    };
    for (entity, id) in &lands {
        // `BoardDef::validate` 保证土地有定义且坐标在网格内；拿不到就是数据错，不画即可
        let (Some(def), Some(frame)) = (b.land(id.0), board.geom.land(id.0)) else {
            continue;
        };
        // 位置与朝向都从几何那一份读：折线版里地块不再踩在格子上，而是紧贴它绑的那条街
        let center = frame.at;
        let footprint = frame.side;
        let plot_rot = Quat::from_rotation_y(frame.yaw);
        let base_mat = match &art.units.grass {
            Some(grass) => photo(&mut mats, grass, false),
            None => plated(
                &mut mats,
                darkened(
                    def.group
                        .as_deref()
                        .map(palette::group)
                        .unwrap_or(Color::srgb(0.6, 0.6, 0.6)),
                    0.55,
                ),
                &art.paper.plate,
            ),
        };
        commands.spawn((
            Mesh3d(meshes.add(Cuboid::new(
                footprint * dim::PLOT_SCALE,
                dim::PLOT_H,
                footprint * dim::PLOT_SCALE,
            ))),
            MeshMaterial3d(base_mat),
            Transform::from_xyz(center.x, dim::PLOT_H / 2.0, center.z).with_rotation(plot_rot),
            Rendered,
        ));

        // 粉笔框：1×1 一整张方框；2×2 用四张角片各转一档，拼出的是一整圈**外框**
        // （中间那道缝不该有线）。`size >= 3` 只有四角有线、边上是空的 —— 三张盘都只有 1 与 2，
        // 真要用大地块得先补一张「直边」角片。
        if let (Some((frame_img, corner)), Some(quad)) = (chalk_pair, &chalk_quad) {
            let y = dim::PLOT_H + dim::CHALK_LIFT;
            let last = def.size as u32 - 1;
            // (格偏移, 档位)：档位是这张角片要绕世界 Y 顺时针转几个 90°
            let corners = [
                ((0u32, 0u32), 0.0),
                ((last, 0), 1.0),
                ((0, last), 3.0),
                ((last, last), 2.0),
            ];
            let piece = if def.size == 1 { frame_img } else { corner };
            // 角片的位置在**地块自己的坐标系**里算：折线版里整块地随街转，四角跟着转
            let half = (def.size.max(1) as f32 - 1.0) / 2.0;
            for ((dc, dr), rot) in corners {
                let local = Vec3::new(dc as f32 - half, 0.0, dr as f32 - half) * b.cell_size;
                let pos = center + plot_rot * local;
                let yaw = frame.yaw - rot * std::f32::consts::FRAC_PI_2;
                commands.spawn((
                    Mesh3d(quad.clone()),
                    MeshMaterial3d(photo(&mut mats, piece, true)),
                    Transform::from_xyz(pos.x, y, pos.z).with_rotation(
                        Quat::from_rotation_y(yaw)
                            * Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2),
                    ),
                    Rendered,
                ));
            }
        }

        // 住宅 1×1 才走模型：商业地是 2×2 的设施，模型是「一户人」的尺度，铺上去会像
        // 把房子摆在停车场里。素材缺一级时 `BuildingModels` 已经是空表，这里自然退回方体。
        let scenic = def.land_type == LandType::Residential && !art.models.levels.is_empty();
        let (bodies, outlines, owner_mat, outline_mat) = if scenic {
            // 描边材质每块地一份：五个等级共享，换主人只改这一个颜色。
            // `unlit` 是「这一圈就是代表色本身」；只画背面（`cull_mode: Front`）才是描边 ——
            // 正面照常画的话这就是一层壳，把模型整个盖住。
            let outline = mats.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: Some(bevy::render::render_resource::Face::Front),
                ..default()
            });
            let mut bodies = Vec::with_capacity(art.models.levels.len());
            let mut outlines = Vec::with_capacity(art.models.levels.len());
            for handle in &art.models.levels {
                let at = Transform::from_xyz(center.x, dim::PLOT_H, center.z)
                    .with_rotation(plot_rot * dim::dwell_upright());
                bodies.push(
                    commands
                        .spawn((
                            WorldAssetRoot(handle.clone()),
                            at.with_scale(Vec3::splat(dim::DWELL_SCALE)),
                            Visibility::Hidden,
                            Rendered,
                        ))
                        .id(),
                );
                outlines.push(
                    commands
                        .spawn((
                            WorldAssetRoot(handle.clone()),
                            SceneOutline {
                                mat: outline.clone(),
                                patched: false,
                            },
                            at.with_scale(Vec3::splat(dim::DWELL_SCALE)),
                            Visibility::Hidden,
                            Rendered,
                        ))
                        .id(),
                );
            }
            (bodies, outlines, None, Some(outline))
        } else {
            let w = footprint * dim::BUILD_SCALE;
            let owner_mat = flat(&mut mats, Color::WHITE);
            let mut slabs = Vec::new();
            for i in 0..rules.max_building_level {
                let y = dim::PLOT_H + i as f32 * (dim::SLAB_H + dim::SLAB_GAP) + dim::SLAB_H / 2.0;
                let slab = commands
                    .spawn((
                        Mesh3d(meshes.add(Cuboid::new(w, dim::SLAB_H, w))),
                        MeshMaterial3d(owner_mat.clone()),
                        Transform::from_xyz(center.x, y, center.z).with_rotation(plot_rot),
                        Visibility::Hidden,
                        Rendered,
                    ))
                    .id();
                slabs.push(slab);
            }
            (slabs, Vec::new(), Some(owner_mat), None)
        };
        // 空地的归属标志物：本体 + 归属色蒙版两张。一开始都藏着 —— 摆不摆由
        // `sync_buildings` 按「有主人 + 还是空地 + 这位角色有素材」决定。
        // 蒙版那张沿远离相机方向退一点：两张同位置会 z 打架。
        let prop_rot = billboard_rot();
        // 摆在地块**近角**而不是正中：Lv1 的模型是一栋小房子，摆正中会站到屋顶上。
        // 位移沿「相机方向在地面上的投影」，也就是屏幕往下那条线，于是它总在房子前面。
        let toward_cam = Vec3::new(dim::CAM_OFFSET.x, 0.0, dim::CAM_OFFSET.z).normalize();
        let prop_base = Vec3::new(center.x, dim::PROP_BASE, center.z)
            + toward_cam * (footprint * dim::PROP_AHEAD);
        let prop_body_mat = mats.add(StandardMaterial {
            unlit: true,
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            ..default()
        });
        let prop_edge_mat = mats.add(StandardMaterial {
            unlit: true,
            alpha_mode: AlphaMode::Mask(0.5),
            cull_mode: None,
            ..default()
        });
        let prop_body = commands
            .spawn((
                Mesh3d(prop_mesh.clone()),
                MeshMaterial3d(prop_body_mat.clone()),
                Transform::from_translation(prop_base).with_rotation(prop_rot),
                Visibility::Hidden,
                Rendered,
            ))
            .id();
        let prop_edge = commands
            .spawn((
                Mesh3d(prop_mesh.clone()),
                MeshMaterial3d(prop_edge_mat.clone()),
                Transform::from_translation(prop_base - prop_rot * Vec3::Z * dim::PROP_EDGE_BACK)
                    .with_rotation(prop_rot),
                Visibility::Hidden,
                Rendered,
            ))
            .id();
        commands.entity(entity).insert(LandVisual {
            bodies,
            outlines,
            owner_mat,
            outline_mat,
            scenic,
            prop_body,
            prop_edge,
            prop_body_mat,
            prop_edge_mat,
            prop_key: None,
        });
    }
}

/// 模型是 PBR 材质，盘上其余东西全 `unlit` —— 没有光模型就是一团黑剪影（§7.5）。
/// 只在模型真装了的时候加一盏斜上方的主光；环境光在 `RenderPlugin` 里常驻。
pub(crate) fn spawn_building_light(mut commands: Commands, models: Res<BuildingModels>) {
    if models.levels.is_empty() {
        return;
    }
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.97, 0.90),
            illuminance: 12_000.0,
            ..default()
        },
        Transform::from_rotation(
            Quat::from_rotation_y(-35.0f32.to_radians())
                * Quat::from_rotation_x(-50.0f32.to_radians()),
        ),
        Rendered,
    ));
}

/// 归属与等级是唯一的真相来源，这里只做变化驱动的同步（`Ref<T>` 变化检测，不轮询）。
pub(crate) fn sync_buildings(
    mut commands: Commands,
    mut lands: Query<(Ref<Owner>, Ref<BuildingLevel>, &mut LandVisual)>,
    players: Query<(&PlayerId, &PlayerColor, &PlayerName)>,
    props: Res<PropArt>,
    mut mats: ResMut<Assets<StandardMaterial>>,
) {
    for (owner, level, mut visual) in &mut lands {
        if !owner.is_changed() && !level.is_changed() {
            continue;
        }
        // 归属色来自玩家实体，而不是按 PlayerId 取模 —— 阵容颜色是 `GameSetup` 定的事实
        let who = owner
            .0
            .and_then(|pid| players.iter().find(|(p, _, _)| **p == pid));
        let owned = who.map(|(_, c, _)| *c);
        // 方体路径：整摞方体共用一个材质，换主人＝改这一个颜色
        if let Some(handle) = &visual.owner_mat
            && let Some(mut mat) = mats.get_mut(handle)
        {
            mat.base_color =
                owned.map_or(Color::WHITE, |PlayerColor([r, g, b])| Color::srgb(r, g, b));
        }
        // 模型路径：描边就是归属标记，没主人时不画
        if let (Some(handle), Some(PlayerColor([r, g, b]))) = (&visual.outline_mat, owned)
            && let Some(mut mat) = mats.get_mut(handle)
        {
            mat.base_color = Color::srgb(r, g, b);
        }
        for (i, body) in visual.bodies.iter().enumerate() {
            // 模型：同时只露等级对应的那一栋；方体：露出前 `level` 个，堆成塔
            let visible = if visual.scenic {
                level.0 == i as u8 + 1
            } else {
                (i as u8) < level.0
            };
            // `try_insert` 而不是 `insert`（P10c 踩到）：这一帧若正好在拆场
            // （`OnEnter(Menu)` 把整盘连建筑一起 despawn），`Commands` 的延迟写会
            // 落到一个已经失效的实体上 —— 那是 `Entity despawned` 的 panic，
            // 而语义上「那栋楼已经不在了」本来就等于「没什么要同步的」
            commands.entity(*body).try_insert(if visible {
                Visibility::Visible
            } else {
                Visibility::Hidden
            });
            // 描边跟着同一栋楼走，但**只在有主人时**画 —— 它是归属标记，不是模型的一部分
            if let Some(outline) = visual.outlines.get(i) {
                commands
                    .entity(*outline)
                    .try_insert(if visible && owned.is_some() {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    });
            }
        }

        // 空地的归属标志物：只在**还是空地**（Lv1）时露出来。楼一盖起来，描边已经在说
        // 归属，两处同时摆等于说两遍，而且楼会把那件道具吃掉一半。
        // 「这位有没有素材」也在这里判 —— 查不到就不摆，不按座位借别人的道具（那是假信息）。
        let want = who
            .and_then(|(_, _, name)| crate::art::prop_key(&name.0))
            .filter(|key| level.0 <= 1 && props.get(key).is_some());
        if want != visual.prop_key {
            if let Some(key) = want
                && let Some((body, edge)) = props.get(key)
            {
                for (handle, image) in [
                    (visual.prop_body_mat.clone(), body),
                    (visual.prop_edge_mat.clone(), edge),
                ] {
                    if let Some(mut mat) = mats.get_mut(&handle) {
                        mat.base_color_texture = Some(image.clone());
                    }
                }
            }
            visual.prop_key = want;
        }
        // 蒙版就是归属色本身（图是纯白 alpha 环，`base_color` 一染就成）
        if let Some(mut mat) = mats.get_mut(&visual.prop_edge_mat) {
            mat.base_color =
                owned.map_or(Color::WHITE, |PlayerColor([r, g, b])| Color::srgb(r, g, b));
        }
        let prop_vis = if visual.prop_key.is_some() {
            Visibility::Visible
        } else {
            Visibility::Hidden
        };
        commands.entity(visual.prop_body).try_insert(prop_vis);
        commands.entity(visual.prop_edge).try_insert(prop_vis);
    }
}

/// 描边外壳长出来之后，把它每一片图元都换成纯色材质（只画背面的那一份）。
///
/// 引擎不会给现成场景套「替身材质」，所以只能等实例化完再逐片换。只走一遍：
/// `WorldAssetRoot` 的子孙一次性生成，只要场景根长出 `Children`，整栋楼就在了 ——
/// 扫完置 `patched`，此后每帧只做一次布尔判断。
pub(crate) fn paint_outline_meshes(
    mut commands: Commands,
    mut roots: Query<(Entity, &mut SceneOutline)>,
    children: Query<&Children>,
    prims: Query<(&Mesh3d, &MeshMaterial3d<StandardMaterial>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut hulls: ResMut<OutlineMeshes>,
) {
    for (root, mut patch) in &mut roots {
        if patch.patched {
            continue;
        }
        // 还没实例化：下一帧再看
        let Ok(kids) = children.get(root) else {
            continue;
        };
        let mut stack: Vec<Entity> = kids.iter().collect();
        while let Some(e) = stack.pop() {
            if let Ok((mesh, _)) = prims.get(e)
                && let Some(hull) = hulls.inflated(&mesh.0, &mut meshes)
            {
                commands
                    .entity(e)
                    .try_insert((Mesh3d(hull), MeshMaterial3d(patch.mat.clone())));
            }
            if let Ok(kids) = children.get(e) {
                stack.extend(kids.iter());
            }
        }
        patch.patched = true;
    }
}
