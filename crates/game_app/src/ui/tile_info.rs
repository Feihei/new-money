//! 点格详情卡。**Godot 端没有这条交互**，是 B5 应需求新增的：点一格 → 看它的名字、
//! 类型、归属、等级、租金表与地价。
//!
//! 三条与既有约定咬合的设计：
//! - 命中测试走**坐标点表**：`BoardGeom` 里存着主环与支线每一格的世界坐标，点击射线打到
//!   地面那一层再找最近的一格。不给每格铺一块不可见地皮，也不必让地面几何带着拾取标记 ——
//!   折线版的地面是「一张草地 + 一圈会转弯的街面」，本来就没有「一格一块」这回事。
//! - 卡片不铺全屏遮罩：它挂在右上角，可以边看边点别格，换格时整块重建。
//! - 内容全部从 `ActiveBoard` + 组件现算。实体上只留 `(col, row)` 这一对坐标，
//!   格名/类型/绑定的土地都回 `BoardDef` 查（§4.2「拓扑查询走 BoardDef」，不复制第二份真相）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, BuildingLevel, LandId, Owner, PlayerId, PlayerName, TileKind,
};

use super::skin::Skins;
use super::theme::{col, layout, size};
use super::{BoardUi, ButtonSpec, UiAction, border_color, building_label, button, info_row, label};
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
/// 带上归属与等级，是为了让「内容变了」这个判据覆盖到买地、升级这类**不换格**的变化 ——
/// 只比坐标就会出现「模态里已经买下、右上角还写着无主」。
///
/// 为什么记在实体上而不是 `Local`：回主菜单时卡片随 `BoardUi` 被收走，`Local` 里的
/// 「上次显示成什么样」却留了下来；下一局再点同一格，键一比相等就直接 return，
/// **卡片再也不出现**。实体没了，键自然没了。
#[derive(Component, PartialEq, Clone)]
pub(crate) struct TileView {
    index: Option<u32>,
    name: String,
    kind: TileKind,
    land: Option<LandId>,
    owner: Option<PlayerId>,
    level: u8,
}

/// 挂在详情卡根节点上，值即「这张卡显示的内容」。
#[derive(Component)]
pub(crate) struct TileCard(pub(crate) TileView);

/// 点中地面就把它记成当前格：屏幕坐标 → 地面那一层 → 最近的一个命中点。
///
/// 与按钮派发是两条独立的 `MessageReader<PointerClick>`：各自读完自己的那份缓冲，顺序无关
/// —— 点在 UI 上就直接跳过（按钮自己会处理），点空处则什么也不做。
pub(crate) fn pick_tile(
    mut clicks: MessageReader<PointerClick>,
    cameras: Query<(&Camera, &GlobalTransform)>,
    geom: Res<BoardGeom>,
    nodes: Query<&Node>,
    mut selected: ResMut<SelectedTile>,
) {
    for click in clicks.read() {
        if click.button != PointerButton::Primary {
            continue;
        }
        if nodes.get(click.entity).is_ok() {
            continue;
        }
        let Some((camera, camera_at)) = cameras.iter().next() else {
            continue;
        };
        let Ok(ray) = camera.viewport_to_world(camera_at, click.pointer.position) else {
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
    lands: Query<(&LandId, &Owner, &BuildingLevel)>,
    players: Query<(&PlayerId, &PlayerName)>,
    selected: Res<SelectedTile>,
    shown: Query<(Entity, &TileCard)>,
) {
    let want = selected.0.and_then(|(col, row)| {
        let (index, tile) = ctx.active.def.tile_at_grid(col, row)?;
        let land = tile.land_id.map(LandId);
        let state = land.and_then(|id| {
            lands
                .iter()
                .find(|(land_id, _, _)| **land_id == id)
                .map(|(_, owner, level)| (owner.0, level.0))
        });
        Some(TileView {
            index: index.map(|i| i.0),
            name: tile.name.clone(),
            kind: tile.kind,
            land,
            owner: state.and_then(|(owner, _)| owner),
            level: state.map_or(0, |(_, level)| level),
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

    let mut rows: Vec<(&'static str, String, Color)> = vec![
        ("类型", kind_name(view.kind).to_string(), value_color),
        (
            "位置",
            match view.index {
                Some(index) => format!("主环第 {index} 格"),
                None => "死胡同分支".to_string(),
            },
            value_color,
        ),
    ];
    if let Some(def) = def {
        rows.push(("地价", format!("${}", def.price), col::COST));
        rows.push((
            "归属",
            owner_name.unwrap_or_else(|| "无主".to_string()),
            value_color,
        ));
        rows.push(("建筑等级", building_label(view.level), col::GAIN));
        for (i, rent) in def.rent.iter().enumerate() {
            rows.push((
                "租金",
                format!("{} — ${rent}", building_label(i as u8)),
                value_color,
            ));
        }
    }
    // 点券箱的面额写在格子上，玩家点一下就该看见「这一格值多少点券」——
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
        button(
            box_,
            &font.0,
            ButtonSpec::new("关闭", [100.0, 32.0], super::SECONDARY, UiAction::CloseInfo),
        );
    });
}

/// 格子类型的中文名。`TileKind` 在规则层是枚举，屏幕上要给人话，映射属于表现层。
fn kind_name(kind: TileKind) -> &'static str {
    match kind {
        TileKind::Property => "地产",
        TileKind::Bank => "银行",
        TileKind::Stock => "证券交易",
        TileKind::Fate => "命运",
        TileKind::Card => "卡片格",
        TileKind::Jail => "拘留所",
        TileKind::Hospital => "医院",
        TileKind::Lottery => "彩票",
        TileKind::GodDeity => "神明",
        TileKind::Neutral => "中立",
        TileKind::Road => "道路",
        TileKind::Transport => "交通",
        TileKind::Coupon => "点券箱",
        TileKind::Shop => "百货公司",
        TileKind::Arcade => "游乐场",
        TileKind::Company => "公司",
        TileKind::News => "新闻中心",
        TileKind::Magic => "魔法屋",
    }
}
