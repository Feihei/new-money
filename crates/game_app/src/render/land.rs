//! 土地视觉：地块底座 + 按等级摆出的建筑。对应 Godot `land_view.gd`。
//!
//! 建筑等级是规则层的事实（`BuildingLevel` 组件），这里只是把它画出来。两条路径：
//!
//! - **住宅模型**（`model/dwell_L1..L5.glb` 五级齐全）：等级 1..=5 各摆一栋，只露当前那一栋。
//!   模型是 Z-up、原点在脚底，进对局先绕 X 立起来再缩（`dim::dwell_upright`/`DWELL_SCALE`）。
//!   归属不染模型（模型保持原色），而是再摆一份**反向外壳**做角色色描边：同一份场景、
//!   网格沿法线外扩一圈（`OutlineMeshes`，不是按原点放大），材质换成纯色且只画背面
//!   （`cull_mode: Front`），轮廓就从模型边缘露出来。
//! - **商业设施模型**（`model/com_<槽>_L1..L5.glb`，五类各一套）：2×2 地块上五类各五级
//!   一次摆全、只切可见性 —— 设施类型要到**建楼那一刻**才知道（空商业地身上没有
//!   `Facility`，`Pending::Build` 才带 kind）。预建设施的 `BuildingLevel` 是 0（它数的是
//!   加盖次数），可视等级按 `max(1)` 补一层，否则购物中心会画成平地却照收服务费。
//!   描边与住宅同一套。
//! - **程序化方体**（素材不在盘上，或民用 1×1）：一次铺满 `max_building_level` 个方体，
//!   等级只决定露出几层 —— 于是「升级」这条路径上没有 spawn/despawn 的簿记。
//!
//! 两条路径都**一次摆满、只切可见性**，这样升级不会产生「这栋旧的没清掉」的空间。

use std::collections::HashMap;

use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::world_serialization::WorldAssetRoot;
use game_core::board::BuildingKind;
use game_core::prelude::{
    BuildingLevel, Facility, GameRules, LandId, LandType, Owner, PerformanceHold, PlayerColor,
    PlayerId, PlayerName, TurnPhase,
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

/// 标志物落下动画的起点高度（世界单位，P16b）。约半格多一点 —— 看得出「从上面落下来」，
/// 又不至于飞出取景。落下时长就是规则层的 `place_beat`。
const PROP_DROP_H: f32 = 1.1;

/// 标志物正在落下（P16b）。挂在**两张四边形各自身上**而不是 `LandVisual` 上：
/// 它们各有各的落点（蒙版那张退后一点），而动画只碰 `Transform`。
#[derive(Component)]
pub(crate) struct PropDrop {
    /// 停稳时的位置。
    base: Vec3,
    age: f32,
    /// 是否已经见过 `Placing`。挂上它的那一帧相位可能还没转移过来（`NextState` 下一帧
    /// 才生效），那不算「过场已经结束」—— 少了这一位，动画会在第一帧就被就地收掉。
    seen_placing: bool,
}

/// 挂在 Land 实体上（Land 实体本身没有 `Transform`，建筑都是各自摆好世界坐标的独立实体）。
#[derive(Component)]
pub(crate) struct LandVisual {
    /// 等级 1..=5 各一份「身体」，下标 = 等级 − 1。
    /// 模型路径下同时只露一个（换等级＝换一栋楼）；方体路径下是堆叠（露出前 `level` 个）。
    bodies: Vec<Entity>,
    /// 模型路径的描边外壳（与 `bodies` 同下标一一对应）；方体路径为空。
    outlines: Vec<Entity>,
    /// 与 `bodies` 同下标：(设施类别, **类别内等级**)。类别 `None` 是住宅模型或方体
    ///  —— 这两种没有「设施类别」这个概念。
    ///
    /// 等级必须与下标分开存：商业地把五类各五级摊在同一个 `bodies` 里（25 栋），
    /// 加油站 Lv1 的下标是 10，按 `下标 + 1` 当等级会把它读成 11 级，整类永不显形。
    body_kinds: Vec<(Option<BuildingKind>, u8)>,
    /// 方体路径的归属色材质（模型路径没有 —— 模型保持原色，归属靠描边）。
    owner_mat: Option<Handle<StandardMaterial>>,
    /// 模型路径的描边材质：一块地一份，换主人只改这一个。
    outline_mat: Option<Handle<StandardMaterial>>,
    /// `bodies` 是场景根（模型）而不是方体。同步时两者对等级的解释不同。
    scenic: bool,
    /// 这块地此刻立着什么设施 —— 规则层 `Facility` 的一份缓存，由 `sync_facility_kind` 维护。
    /// 住宅/民用地恒 `None`；空商业地也是 `None`（建成那一刻才由组件填上）。
    facility: Option<BuildingKind>,
    /// 上一次漏掉的同步：设施刚插上时 `BuildingLevel` 可能还没变（见 `sync_facility_kind`）。
    dirty: bool,
    /// 空地上那件归属标志物的两张贴屏四边形：本体与归属色蒙版。
    /// 与建筑一样**一次摆好、只切可见性**，摆不摆由 `sync_buildings` 定。
    prop_body: Entity,
    prop_edge: Entity,
    /// 标志物的两张材质（一块地一份：换主人只换贴图与一个颜色）。
    prop_body_mat: Handle<StandardMaterial>,
    prop_edge_mat: Handle<StandardMaterial>,
    /// 现在贴的是谁的图。`None` = 这块地没有标志物（没主人／已盖楼／这位没素材）。
    prop_key: Option<&'static str>,
    /// 标志物两张四边形停稳时的位置（P16b 落下动画的落点；蒙版那张退后一点，所以两份）。
    prop_body_base: Vec3,
    prop_edge_base: Vec3,
    /// 这张地块的视觉是否已经同步过一次。**首帧同步不播落下动画**：读档进来时
    /// 75 块地一起从天上掉下来不是过场，是一场雨。
    synced_once: bool,
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
    lands: Query<(Entity, &LandId, Option<&Facility>)>,
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
    for (entity, id, facility) in &lands {
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

        // 两条素材路径。住宅是 1×1，五级一套；商业地是 2×2，设施类型要到**建楼那一刻**
        // 才知道（空商业地身上没有 `Facility`），所以五类各五级一次摆全、只切可见性 ——
        // 隐藏的 scene 实例不进渲染，这里省掉的是运行时 spawn 的簿记，不是画 25 栋楼。
        // 某类缺一级时 `BuildingModels::facility` 返回 None，那一类不摆；一类都不齐才退回方体。
        let residential = def.land_type == LandType::Residential && !art.models.levels.is_empty();
        let facility_sets: Vec<(BuildingKind, Vec<Handle<WorldAsset>>)> =
            if def.land_type == LandType::Commercial {
                BuildingKind::ALL
                    .iter()
                    .filter_map(|kind| art.models.facility(*kind).map(|set| (*kind, set.to_vec())))
                    .collect()
            } else {
                Vec::new()
            };
        let scenic = residential || !facility_sets.is_empty();

        let scale = if residential {
            dim::DWELL_SCALE
        } else {
            dim::COM_SCALE
        };
        let (bodies, outlines, body_kinds, owner_mat, outline_mat) = if scenic {
            // 描边材质每块地一份：同一套模型共享，换主人只改这一个颜色。
            // `unlit` 是「这一圈就是代表色本身」；只画背面（`cull_mode: Front`）才是描边 ——
            // 正面照常画的话这就是一层壳，把模型整个盖住。
            let outline = mats.add(StandardMaterial {
                base_color: Color::WHITE,
                unlit: true,
                cull_mode: Some(bevy::render::render_resource::Face::Front),
                ..default()
            });
            // (类别, 类别内等级, 句柄)：住宅整列类别都是 `None`，商业按 `ALL` 序一类接一类。
            let handles: Vec<(Option<BuildingKind>, u8, Handle<WorldAsset>)> = if residential {
                art.models
                    .levels
                    .iter()
                    .enumerate()
                    .map(|(i, handle)| (None, i as u8 + 1, handle.clone()))
                    .collect()
            } else {
                facility_sets
                    .iter()
                    .flat_map(|(kind, set)| {
                        let kind = Some(*kind);
                        set.iter()
                            .enumerate()
                            .map(move |(i, handle)| (kind, i as u8 + 1, handle.clone()))
                    })
                    .collect()
            };
            let mut bodies = Vec::with_capacity(handles.len());
            let mut outlines = Vec::with_capacity(handles.len());
            let mut body_kinds = Vec::with_capacity(handles.len());
            for (kind, level, handle) in handles {
                let at = Transform::from_xyz(center.x, dim::PLOT_H, center.z)
                    .with_rotation(plot_rot * dim::dwell_upright())
                    .with_scale(Vec3::splat(scale));
                bodies.push(
                    commands
                        .spawn((
                            WorldAssetRoot(handle.clone()),
                            at,
                            Visibility::Hidden,
                            Rendered,
                        ))
                        .id(),
                );
                outlines.push(
                    commands
                        .spawn((
                            WorldAssetRoot(handle),
                            SceneOutline {
                                mat: outline.clone(),
                                patched: false,
                            },
                            at,
                            Visibility::Hidden,
                            Rendered,
                        ))
                        .id(),
                );
                body_kinds.push((kind, level));
            }
            (bodies, outlines, body_kinds, None, Some(outline))
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
            (slabs, Vec::new(), Vec::new(), Some(owner_mat), None)
        };
        // 空地的归属标志物：本体 + 归属色蒙版两张。一开始都藏着 —— 摆不摆由
        // `sync_buildings` 按「有主人 + 还是空地 + 这位角色有素材」决定。
        // 蒙版那张沿远离相机方向退一点：两张同位置会 z 打架。
        let prop_rot = billboard_rot();
        // 锚在格子**正中央**：底边落在中心、身子立起来越过格子上沿，读数就是「立在这块地上」。
        // 空地（Lv0）上没有楼，中心是空着的，不必再为避开建筑做任何位移。
        let prop_base = Vec3::new(center.x, dim::PROP_BASE, center.z);
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
        let prop_edge_base = prop_base - prop_rot * Vec3::Z * dim::PROP_EDGE_BACK;
        let prop_edge = commands
            .spawn((
                Mesh3d(prop_mesh.clone()),
                MeshMaterial3d(prop_edge_mat.clone()),
                Transform::from_translation(prop_edge_base).with_rotation(prop_rot),
                Visibility::Hidden,
                Rendered,
            ))
            .id();
        commands.entity(entity).insert(LandVisual {
            bodies,
            outlines,
            body_kinds,
            owner_mat,
            outline_mat,
            scenic,
            facility: facility.map(|f| f.0),
            // `LandVisual` 是 `Commands` 延迟插的，`sync_buildings` 第一次看见它时
            // `Owner`/`BuildingLevel` 的 change tick 可能已经与上次运行对齐，`is_changed()`
            // 读成 false —— 那就一次都不摆（预建设施与空地全停在 `Hidden`，画面上只有粉笔框）。
            // 首帧强制同步一次，代价是每块地一次。
            dirty: true,
            prop_body,
            prop_edge,
            prop_body_mat,
            prop_edge_mat,
            prop_key: None,
            prop_body_base: prop_base,
            prop_edge_base,
            synced_once: false,
        });
    }
}

/// 模型是 PBR 材质，盘上其余东西全 `unlit` —— 没有光模型就是一团黑剪影（§7.5）。
/// 只在模型真装了的时候加一盏斜上方的主光；环境光在 `RenderPlugin` 里常驻。
pub(crate) fn spawn_building_light(mut commands: Commands, models: Res<BuildingModels>) {
    if !models.any() {
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

/// 把规则层的 [`Facility`] 抄进 [`LandVisual`]。
///
/// 为什么另开一条而不是让 `sync_buildings` 直接读：`Facility` 是建楼时用 `Commands` 插的
/// （`property::build_op_system`），同一帧的 `sync_buildings` 还看不到它 —— 而到了下一帧
/// `BuildingLevel` 已经不再变化，那条会整块跳过，楼就永远不出现。这条每帧扫一遍
/// （几十块地，成本可忽略），只在类别真的变了时打一个「漏同步」标记。
pub(crate) fn sync_facility_kind(lands: Query<(Option<&Facility>, &mut LandVisual)>) {
    for (facility, mut visual) in lands {
        let kind = facility.map(|f| f.0);
        if visual.facility != kind {
            visual.facility = kind;
            visual.dirty = true;
        }
    }
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
        if !owner.is_changed() && !level.is_changed() && !visual.dirty {
            continue;
        }
        // 有设施就至少露 Lv1：预建设施在棋盘数据里只写「这块地立着什么」，而
        // `BuildingLevel` 数的是**加盖次数**，开局是 0。不补这一格，购物中心会画成一块
        // 平地却照收服务费（服务费只看 `Facility`，不看等级）。
        let shown = if visual.facility.is_some() {
            level.0.max(1)
        } else {
            level.0
        };
        visual.dirty = false;
        // 首帧这次同步只摆状态、不播动画（读档时的「一场雨」见 `synced_once` 的注）
        let first = !visual.synced_once;
        visual.synced_once = true;
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
                // 商业地还要挑对类别：五类各五级只露「当前设施 + 当前等级」这一栋。
                let (kind, at) = visual.body_kinds.get(i).copied().unwrap_or((None, 0));
                kind == visual.facility && shown == at
            } else {
                (i as u8) < shown
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

        // 空地的归属标志物：只在 `BuildingLevel == 0`（**空地**）时露出来。买下地就是 Lv0，
        // 加盖第一层才起楼 —— 楼一立起来就只靠模型 + 描边说归属，两样东西不可能同框。
        // 「这位有没有素材」也在这里判 —— 查不到就不摆，不按座位借别人的道具（那是假信息）。
        let want = who
            .and_then(|(_, _, name)| crate::art::prop_key(&name.0))
            .filter(|key| shown == 0 && props.get(key).is_some());
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
            // 元素真的换了（从没有到有、或换了主人）→ 播一次落下动画。首帧那次例外。
            // 落下的起点在这里就写好，免得动画系统晚一帧才把它抬起来、中间闪一格平地。
            if !first && want.is_some() {
                let rot = billboard_rot();
                for (entity, base) in [
                    (visual.prop_body, visual.prop_body_base),
                    (visual.prop_edge, visual.prop_edge_base),
                ] {
                    commands.entity(entity).try_insert((
                        PropDrop {
                            base,
                            age: 0.0,
                            seen_placing: false,
                        },
                        Transform::from_translation(base + Vec3::Y * PROP_DROP_H)
                            .with_rotation(rot)
                            .with_scale(Vec3::splat(1.12)),
                    ));
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

/// 标志物落下（P16b）：从 [`PROP_DROP_H`] 高处落下、末段收住，`place_beat` 秒内停稳。
///
/// 钟在本地推进、不读 `BeatClock`：挂上它是在「归属真的变了」那一帧，与 `Placing` 的
/// 起点可能差一帧，本地年龄对这点错位不敏感。两件事仍然对齐：时长读的就是
/// `GameRules::place_beat`（与过场同一份），台词闸按住时 `dt` 记 0（与走步、骰子
/// 同一条口径：表现层全局停）。相位离开 `Placing`（自然走完或单击跳过）→ 立刻停在落点上。
pub(crate) fn animate_prop_drops(
    mut commands: Commands,
    time: Res<Time>,
    phase: Res<State<TurnPhase>>,
    hold: Res<PerformanceHold>,
    rules: Res<GameRules>,
    mut props: Query<(Entity, &mut PropDrop, &mut Transform)>,
) {
    let dt = if hold.0 { 0.0 } else { time.delta_secs() };
    let dur = rules.place_beat.max(0.05);
    let live = phase.get() == &TurnPhase::Placing;
    for (entity, mut drop, mut tr) in &mut props {
        if live {
            drop.seen_placing = true;
        }
        drop.age += dt;
        let t = (drop.age / dur).clamp(0.0, 1.0);
        // 三次缓出：落得先快后慢，停住时不再有速度
        let eased = 1.0 - (1.0 - t).powi(3);
        let lift = (1.0 - eased) * PROP_DROP_H;
        tr.translation = drop.base + Vec3::Y * lift;
        tr.scale = Vec3::splat(1.0 + 0.12 * (1.0 - eased));
        if (drop.seen_placing && !live) || t >= 1.0 {
            tr.translation = drop.base;
            tr.scale = Vec3::ONE;
            commands.entity(entity).remove::<PropDrop>();
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;

    /// P16b 的标志物落下：时长内抬起、到点回到落点并自己摘掉组件。
    ///
    /// 两半都断言：只断言「摘掉了」的话，把 `translation` 写成乱数也是绿的；
    /// 只断言「抬起来了」的话，动画永远不会停 —— 而那块地会一直悬在半空。
    #[test]
    fn a_prop_drop_rises_then_settles_and_frees_itself() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StatesPlugin)
            .init_state::<TurnPhase>()
            .init_resource::<PerformanceHold>()
            .init_resource::<GameRules>()
            .add_systems(Update, animate_prop_drops);
        // 相位留在 `Boot`（`live == false`）：本地年龄照样推进，这正是「读档后没进过场
        // 也要把动画收尾」那条退路
        let base = Vec3::new(1.0, 0.3, 2.0);

        let falling = app
            .world_mut()
            .spawn((
                PropDrop {
                    base,
                    age: 0.0,
                    seen_placing: false,
                },
                Transform::from_xyz(0.0, 0.0, 0.0),
            ))
            .id();
        app.update();
        let at = app
            .world()
            .entity(falling)
            .get::<Transform>()
            .expect("还在落")
            .translation;
        assert!(
            (at.y - (base.y + PROP_DROP_H)).abs() < 1e-2,
            "刚挂上时该在起点高处：{at:?}"
        );
        assert!(app.world().entity(falling).get::<PropDrop>().is_some());

        // 时间到头：回到落点、摘掉组件
        app.world_mut()
            .entity_mut(falling)
            .get_mut::<PropDrop>()
            .expect("还在落")
            .age = 99.0;
        app.update();
        let at = app
            .world()
            .entity(falling)
            .get::<Transform>()
            .expect("实体还在，只是不再动")
            .translation;
        assert_eq!(at, base, "落定时该正好在停稳的位置");
        assert!(
            app.world().entity(falling).get::<PropDrop>().is_none(),
            "动画结束要自己摘掉组件"
        );
    }
}
