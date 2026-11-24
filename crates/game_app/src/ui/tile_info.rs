//! 点格详情卡。**Godot 端没有这条交互**，是 B5 应需求新增的：按住一格 → 看它的名字，
//! 地块另有当前等级、租金与归属。**按住才显示，松开就消失**（12-08 实报：地面常驻格名
//! 把棋盘糊成字海，地名这种「想看才看」的信息不该永远铺在画面上）。
//!
//! 三条与既有约定咬合的设计：
//! - 命中测试走**坐标点表**：`BoardGeom` 里存着主环与支线每一格的世界坐标，按下射线打到
//!   地面那一层再找最近的一格。不给每格铺一块不可见地皮，也不必让地面几何带着拾取标记 ——
//!   折线版的地面是「一张草地 + 一圈会转弯的街面」，本来就没有「一格一块」这回事。
//! - 读 [`PointerPress`] 而不是 [`PointerClick`]：「按下就显示」要的是按下那一刻，而
//!   click 要等松手才发 —— 那半个握手的时间足够让卡片闪一下又消失。松开走
//!   [`PointerRelease`]，**不挑目标**：按在地面、拖到按钮上、拖到棋盘外松手，卡片都该收走。
//! - 内容全部从 `ActiveBoard` + 账本现算。实体上只留 `(col, row)` 这一对坐标，
//!   格名/类型/绑定的土地都回 `BoardDef` 查（§4.2「拓扑查询走 BoardDef」，不复制第二份真相）；
//!   租金走 `Books::rent_of` —— 与落格收租是同一个数，「卡片写的」与「真的收的」不许两套口径。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{ActiveBoard, Books, LandId, PlayerId, PlayerName, TileKind};

use super::skin::Skins;
use super::theme::{col, layout, size};
use super::{BoardUi, border_color, building_label, facility_label, info_row, label};
use crate::render::{DevFont, dim, geom::BoardGeom, palette};

/// 现在看的是哪一格（网格坐标，含死胡同格）。`None` = 没有详情卡。
#[derive(Resource, Default)]
pub(crate) struct SelectedTile(pub Option<(u32, u32)>);

impl SelectedTile {
    /// `NM_PICK_TILE=列:行` 开局即选中那格。
    ///
    /// 详情卡只能被点击唤出，而 agent 没有鼠标可点 —— 与 [`super::pause::PauseMenuOpen::from_env`]
    /// 同理，进不了截图的界面等于没做完。解析失败就当没设，不 panic。
    pub(crate) fn from_env() -> Self {
        let spec = match std::env::var("NM_PICK_TILE") {
            Ok(spec) => spec,
            Err(_) => return Self(None),
        };
        let mut it = spec.split(':');
        let (Some(col), Some(row), None) = (it.next(), it.next(), it.next()) else {
            return Self(None);
        };
        match (col.parse::<u32>(), row.parse::<u32>()) {
            (Ok(col), Ok(row)) => Self(Some((col, row))),
            _ => Self(None),
        }
    }
}

/// 卡片要显示的那一格的全部事实，同时挂在卡片根实体上充当「已经写成这样」的记录。
///
/// 带上归属、等级与租金，是为了让「内容变了」这个判据覆盖到买地、升级这类**不换格**的变化 ——
/// 只比坐标就会出现「模态里已经买下、右上角还写着无主」。
///
/// 为什么记在实体上而不是 `Local`：回主菜单时卡片随 `BoardUi` 被收走，`Local` 里的
/// 「上次显示成什么样」却留了下来；下一局再按同一格，键一比相等就直接 return，
/// **卡片再也不出现**。实体没了，键自然没了。
#[derive(Component, PartialEq, Clone)]
pub(crate) struct TileView {
    index: Option<u32>,
    name: String,
    kind: TileKind,
    land: Option<LandId>,
    owner: Option<PlayerId>,
    level: u8,
    rent: i64,
}

/// 挂在详情卡根节点上，值即「这张卡显示的内容」。
#[derive(Component)]
pub(crate) struct TileCard(pub(crate) TileView);

/// 按住地面就把它记成当前格：屏幕坐标 → 地面那一层 → 最近的一个命中点。
///
/// 与按钮派发是两条独立的 reader：各自读完自己的那份缓冲，顺序无关 —— 按在 UI 上
/// 就直接跳过（按钮自己处理 `PointerPress`），按空处则什么也不做。松开清选中格
/// **不挑目标**：`PointerRelease` 发给谁不重要，主键松手卡片就该走。
pub(crate) fn pick_tile(
    mut presses: MessageReader<PointerPress>,
    mut releases: MessageReader<PointerRelease>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    geom: Res<BoardGeom>,
    nodes: Query<&Node>,
    mut selected: ResMut<SelectedTile>,
) {
    for press in presses.read() {
        if press.button != PointerButton::Primary {
            continue;
        }
        if nodes.get(press.entity).is_ok() {
            continue;
        }
        let Some((camera, camera_at)) = cameras.iter().next() else {
            continue;
        };
        let Ok(ray) = camera.viewport_to_world(camera_at, press.pointer.position) else {
            continue;
        };
        // 正交等轴测的射线永远斜着落向地面，`direction.y` 不会趋零
        let t = (dim::TILE_Y - ray.origin.y) / ray.direction.y;
        if !t.is_finite() {
            continue;
        }
        if let Some(cell) = geom.nearest_cell(ray.origin + *ray.direction * t) {
            selected.0 = Some(cell);
        }
    }
    for release in releases.read() {
        if release.button != PointerButton::Primary {
            continue;
        }
        selected.0 = None;
    }
}

/// 当前格变了就重建卡片。判据是卡片自己那份 [`TileView`]，见它的注释。
/// 面板要读的三样世界状态收成一包：system 参数八格撞上限，收拢是房规（§六 坑 8）。
#[derive(SystemParam)]
pub(crate) struct TileCtx<'w> {
    font: Res<'w, DevFont>,
    skins: Res<'w, Skins>,
    active: Res<'w, ActiveBoard>,
}

pub(crate) fn sync_tile_card(
    mut commands: Commands,
    ctx: TileCtx<'_>,
    books: Books<'_, '_>,
    players: Query<(&PlayerId, &PlayerName)>,
    selected: Res<SelectedTile>,
    shown: Query<(Entity, &TileCard)>,
) {
    let want = selected.0.and_then(|(col, row)| {
        let (index, tile) = ctx.active.def.tile_at_grid(col, row)?;
        let land = tile.land_id.map(LandId);
        // 归属/等级/租金三样从账本读，与落格结算同一份口径 —— 卡片上那个数与
        // 真收到的钱不许是两条式子算出来的（`rent_of` 里无主/抵押/查封都短路成 0）
        let (owner, level, rent) = land
            .and_then(|id| {
                books
                    .view(id)
                    .map(|v| (v.owner, v.level, books.rent_of(id)))
            })
            .unwrap_or((None, 0, 0));
        Some(TileView {
            index: index.map(|i| i.0),
            name: tile.name.clone(),
            kind: tile.kind,
            land,
            owner,
            level,
            rent,
        })
    });

    if shown
        .iter()
        .next()
        .is_some_and(|(_, card)| Some(&card.0) == want.as_ref())
    {
        return;
    }
    for (entity, _) in shown.iter() {
        commands.entity(entity).despawn();
    }
    if let Some(view) = want {
        spawn_card(
            &mut commands,
            &ctx.font,
            &ctx.skins,
            &ctx.active,
            &players,
            view,
        );
    }
}

fn spawn_card(
    commands: &mut Commands,
    font: &DevFont,
    skins: &Skins,
    active: &ActiveBoard,
    players: &Query<(&PlayerId, &PlayerName)>,
    view: TileView,
) {
    let accent = palette::tile(view.kind);
    let paper = skins.float.image.is_some();
    let title_color = if paper { col::INK } else { accent };
    let value_color = if paper { col::INK_DIM } else { col::VALUE };
    let (title, title_font, title_color) =
        label(&font.0, view.name.clone(), size::NAME, title_color);
    let def = view.land.and_then(|id| active.def.land(id.0));
    let owner_name = view
        .owner
        .and_then(|id| players.iter().find(|(pid, _)| **pid == id))
        .map(|(_, name)| name.0.clone());

    // 街道格只带地名；地块（有地可查的）才长出归属/等级/租金三行。
    // 按住才显示的一张小卡，铺一整张租金表就是把它当常驻面板用了
    let mut rows: Vec<(&'static str, String, Color)> = vec![];
    if let Some(def) = def {
        rows.push((
            "归属",
            owner_name.unwrap_or_else(|| "无主".to_string()),
            value_color,
        ));
        rows.push((
            "当前等级",
            if def.land_type == game_core::board::LandType::Commercial {
                facility_label(view.level)
            } else {
                building_label(view.level)
            },
            col::GAIN,
        ));
        rows.push(("当前租金", format!("${}", view.rent), col::COST));
    }
    // 点券箱的面额写在格子上，按一下就该看见「这一格值多少点券」——
    // 三档梯度的全部意义是「这段路值得走」，看不见数字就谈不上绕路
    if let Some(index) = view.index
        && let Some(coupon) = active.def.tile(index as i64).and_then(|t| t.data.coupon)
    {
        rows.push(("面额", format!("{coupon} 点券"), col::GAIN));
    }

    let mut card = commands.spawn((
        BoardUi,
        TileCard(view),
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(layout::TOP_BAR_INSET + layout::ROLL_BUTTON[1] + 8.0),
            right: Val::Px(layout::TOP_BAR_INSET),
            width: Val::Px(260.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: super::skin::padding(&skins.float, 16.0, 26.0, 260.0),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        BackgroundColor(col::PANEL),
        border_color(accent),
        ZIndex(4),
    ));
    // 底板皮肤：盘上有 panel_float 就叠九宫格底图，没有保持纯色面板（浮层同一句）
    if let Some((skin_node, skin_image)) = super::skin::panel_image(&skins.float) {
        card.with_children(|p| {
            p.spawn((skin_node, skin_image));
        });
    }
    card.with_children(|box_| {
        box_.spawn((
            Node {
                width: Val::Percent(100.0),
                ..default()
            },
            TextLayout::justify(Justify::Center),
            title,
            title_font,
            title_color,
        ));
        for (key, val, color) in &rows {
            info_row(box_, &font.0, key, val, *color, 72.0);
        }
    });
}
