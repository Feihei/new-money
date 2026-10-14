//! UI 控件层：按钮、面板、遮罩与点击派发。
//!
//! 0.20 的 `bevy_ui::Button` 与 `Interaction` 已弃用（prelude 里带着
//! `#[expect(deprecated, reason = "Should be removed after 0.20 is released…")]`），
//! 用它们会直接吃 `-D warnings`。点击一律走 picking 的指针事件：
//! 交互节点挂一个 [`UiAction`]，[`dispatch_ui_actions`] 读 `PointerClick` 消息派发。
//!
//! 命中测试给出的是**最深的**节点 —— 按到的是按钮里的文字而不是按钮本身，
//! 所以两个入口都要沿 `ChildOf` 往上找第一个带 `UiAction` 的祖先
//! （[`action_of`]）。顺带这也让遮罩真的能挡住下面：遮罩是最深命中节点、没有动作，
//! 事件就停在那里，不会穿到顶栏按钮上。

pub mod dialog;
pub mod menu;
pub mod pause;
pub mod player_hud;
pub mod theme;
pub mod tile_info;
pub mod top_bar;

use bevy::{app::AppExit, ecs::system::SystemParam, prelude::*};
use game_core::prelude::{
    ActiveBoard, Decision, DecisionChosen, Paused, PendingDecision, RequestRoll, TurnOrder,
    TurnPhase,
};

use theme::{col, layout, size};

pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<dialog::RecentCard>()
            // `NM_PAUSE=1` 开局即暂停：没有键盘可按，暂停面板否则无法被截图核对
            .insert_resource(pause::PauseMenuOpen::from_env())
            // `NM_PICK_TILE=列:行` 开局即选中那格：详情卡否则也进不了截图
            .insert_resource(tile_info::SelectedTile::from_env())
            // `NM_DECISION=buy|upgrade|rent|card` 直接挂出某个模态（见 [`dialog::ForceModal`]）
            .insert_resource(dialog::ForceModal::from_env())
            .add_systems(
                Update,
                (
                    (top_bar::spawn_top_bar, player_hud::spawn_player_hud).in_set(UiSet::Boot),
                    (
                        pause::toggle_pause_on_escape,
                        top_bar::refresh_top_bar,
                        top_bar::hint_from_events,
                        player_hud::refresh_player_hud,
                        dialog::remember_drawn_card,
                        dialog::force_decision,
                        dialog::sync_modal,
                        pause::sync_pause_menu,
                        tile_info::pick_tile,
                        tile_info::sync_tile_card,
                        dispatch_ui_actions,
                        hover_feedback,
                    )
                        .in_set(UiSet::Live),
                )
                    .chain(),
            );
        // 守门挂在集合上而不是逐个 system 上：元组整体 `.run_if(...)` 在参数含双生命周期
        // `SystemParam`（`Books`、`Notices`）时会退化成重载解析失败（§六 坑 9）
        app.configure_sets(
            Update,
            (
                UiSet::Boot.run_if(resource_added::<ActiveBoard>),
                UiSet::Live.run_if(resource_exists::<ActiveBoard>),
            ),
        );
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum UiSet {
    /// 棋盘就位那一刻装配一次
    Boot,
    /// 逐帧刷新与输入派发。顺序有讲究：阶段旁白 → 事件覆盖 → 模态 → 派发
    Live,
}

/// 挂在可点击节点上。语义是「按下去要做的事」，不携带状态 —— 能不能做由派发方判断。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiAction {
    NewGame,
    Quit,
    BackToMenu,
    /// 掷骰。只有当前回合玩家、且没有待决事项时才生效（见 dispatch）
    Roll,
    /// 回答当前待决事项。「回答关于谁」由 `PendingDecision` 决定，按钮不背这个信息
    Decide(Decision),
    /// 关闭点格详情卡（新增交互，见 [`tile_info`]）
    CloseInfo,
    /// 开合暂停菜单（ESC 键与「继续」「暂停」按钮同一个动作）
    TogglePause,
}

/// 存在即禁用：着色变灰，且点击不派发。对应 Godot 的 `button.disabled = true`。
#[derive(Component, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Disabled;

/// 按钮的常态与悬停色。
#[derive(Component, Debug, Clone, Copy)]
pub struct Palette {
    pub normal: Color,
    pub hover: Color,
}

/// 主菜单的根节点标记，`OnEnter(Scene::Board)` 按它整块收走。
#[derive(Component)]
pub struct MenuPiece;

/// 对局内的 UI（顶栏、HUD、弹层）根节点标记，回菜单时整块收走。
#[derive(Component)]
pub struct BoardUi;

/// 掷骰此刻可用吗。派发方与顶栏着色共用一份判据，免得出现「看着能点、按下去没反应」。
///
/// 规则层还会再认一次「是不是当前玩家」，所以这里放行错人也不会真的掷出骰子
/// （`roll_dice_system` 的守门）。
pub(crate) fn can_roll(phase: TurnPhase, paused: &Paused, pending: &PendingDecision) -> bool {
    phase == TurnPhase::WaitRoll && !paused.0 && pending.active().is_none()
}

/// 回主菜单时清掉 UI 侧**跨局存活**的记忆。挂在 `OnEnter(Scene::Menu)` 的链上。
///
/// 面板实体本身不用管：它们都带 [`BoardUi`]，由 `despawn_board_view` 整块收走，
/// 而「现在挂着什么」这类键已经改挂在实体上（见 [`dialog::ModalRoot`]、[`tile_info::TileView`]），
/// 随实体一起消失。剩下这四个是纯资源，不清的后果各不一样，且都**不会报错**：
/// - `Paused` 留着 true → 下一局开局即冻结，画面看着完全正常；
/// - `PauseMenuOpen` 留着 true → 下一局第一帧就把面板盖回来；
/// - `SelectedTile` 留着 → 下一局凭空显示上一局那格的详情卡；
/// - `RecentCard` 留着 → 下一局第一次抽到「无内容」的卡时，显示的是上一局那张的文案。
pub(crate) fn reset_ui_state(
    mut commands: Commands,
    mut memory: ResMut<dialog::RecentCard>,
    mut selected: ResMut<tile_info::SelectedTile>,
) {
    memory.0 = None;
    selected.0 = None;
    commands.insert_resource(pause::PauseMenuOpen::default());
    commands.insert_resource(Paused::default());
}

/// 一个文本节点需要的三件套。包一层的理由是漏掉 `TextFont` 不会报错，
/// 只会退回 Fira Mono 子集把中文变成豆腐块（§六 坑 2）。
pub fn label(
    font: &Handle<Font>,
    text: impl Into<String>,
    px: f32,
    color: Color,
) -> (Text, TextFont, TextColor) {
    (
        Text::new(text),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(px),
            ..default()
        },
        TextColor(color),
    )
}

/// 按钮配色：主（绿）与次（灰）。数值来自 dialog 的 `_style_button`。
pub const PRIMARY: (Color, Color) = (col::BTN_PRIMARY, col::BTN_PRIMARY_HOVER);
pub const SECONDARY: (Color, Color) = (col::BTN_SECONDARY, col::BTN_SECONDARY_HOVER);

/// 造一个按钮要说的全部信息。收成一个结构是因为参数一多就撞 `too_many_arguments`，
/// 而房规是「打包参数而不是挂 `#[allow]`」（§六 坑 8）。
pub struct ButtonSpec<'a> {
    pub text: &'a str,
    pub size: [f32; 2],
    pub font: f32,
    pub palette: (Color, Color),
    pub action: UiAction,
    /// 灰掉且不可点（Godot 的 `button.disabled`）
    pub disabled: bool,
}

impl<'a> ButtonSpec<'a> {
    pub fn new(text: &'a str, size: [f32; 2], palette: (Color, Color), action: UiAction) -> Self {
        Self {
            text,
            size,
            font: theme::size::BODY,
            palette,
            action,
            disabled: false,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// 一个定尺寸的圆角按钮：盒子 + 居中文本 + 动作。挂到 `parent` 下。
pub fn button(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, spec: ButtonSpec) -> Entity {
    let (normal, hover) = if spec.disabled {
        (col::BTN_DISABLED, col::BTN_DISABLED)
    } else {
        spec.palette
    };
    let edge = if spec.disabled {
        col::BTN_DISABLED
    } else {
        col::BTN_BORDER
    };
    let mut spawned = parent.spawn((
        Node {
            width: Val::Px(spec.size[0]),
            height: Val::Px(spec.size[1]),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            border: UiRect::all(Val::Px(layout::BTN_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
            ..default()
        },
        BackgroundColor(normal),
        border_color(edge),
        spec.action,
        Palette { normal, hover },
    ));
    if spec.disabled {
        spawned.insert(Disabled);
    }
    spawned
        .with_children(|b| {
            b.spawn(label(
                font,
                spec.text,
                spec.font,
                if spec.disabled {
                    col::BTN_TEXT_DISABLED
                } else {
                    Color::WHITE
                },
            ));
        })
        .id()
}

/// Godot 的 `HSeparator`：一条 2px 高的填充节点（没有 `BackgroundColor` 的 `Node`
/// 什么都画不出来，别以为它会自动有线）。
pub fn separator() -> (Node, BackgroundColor) {
    (
        Node {
            width: Val::Percent(100.0),
            height: Val::Px(2.0),
            ..default()
        },
        BackgroundColor(col::BTN_BORDER),
    )
}

/// 一行「键 值」信息。`key_w` 是键列的固定宽（Godot 的 `custom_minimum_size.x`），
/// 值列 `flex_grow` 吃掉余下宽度，长文案就在列内换行。
pub fn info_row(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    key: &str,
    val: &str,
    val_color: Color,
    key_w: f32,
) {
    let (key_text, key_font, key_color) = label(font, key, size::ROW, col::KEY);
    let (val_text, val_font, val_ink) = label(font, val, size::ROW, val_color);
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            column_gap: Val::Px(8.0),
            ..default()
        })
        .with_children(|row| {
            row.spawn((
                Node {
                    min_width: Val::Px(key_w),
                    ..default()
                },
                key_text,
                key_font,
                key_color,
            ));
            row.spawn((
                Node {
                    flex_grow: 1.0,
                    ..default()
                },
                val_text,
                val_font,
                val_ink,
            ));
        });
}

/// 四边同色的边框。0.20 的 `BorderColor` 是四个字段而不是 `all()`，收在这里免得散开。
pub fn border_color(c: Color) -> BorderColor {
    BorderColor {
        top: c,
        right: c,
        bottom: c,
        left: c,
    }
}

/// 弹层面板：绝对定位到「屏幕中心 + 左上角偏移」，复刻 Godot 的 `PRESET_CENTER` + `position`。
///
/// 用 `Percent(50)` 定左上角、再用 `UiTransform::translation` 挪像素，是因为面板高度由内容撑开，
/// 拿不到 Godot 那个「已知高度的一半」—— translation 是布局之后的位移，不参与重排。
pub fn panel(
    width: f32,
    offset: [f32; 3],
    accent: Color,
) -> (Node, UiTransform, BackgroundColor, BorderColor) {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Percent(50.0),
            width: Val::Px(width),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(10.0),
            padding: UiRect::axes(Val::Px(layout::PANEL_PAD_X), Val::Px(layout::PANEL_PAD_Y)),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        UiTransform::from_translation(Val2::new(Val::Px(offset[1]), Val::Px(offset[2]))),
        BackgroundColor(col::PANEL),
        border_color(accent),
    )
}

/// 全屏遮罩。`z` 决定它压在哪些 UI 之上（同层靠 spawn 顺序，跨层靠 `ZIndex`）。
pub fn overlay(z: i32) -> (Node, BackgroundColor, ZIndex) {
    (
        Node {
            width: Val::Percent(100.0),
            height: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
            ..default()
        },
        BackgroundColor(col::OVERLAY),
        ZIndex(z),
    )
}

/// 派发要读的世界状态。三个参数包是房规换来的：这个 system 需要知道的确实不少
/// （点到了什么、能不能做、要往哪发），但参数超过七个不挂 `#[allow]` 而是收拢（§六 坑 8）。
#[derive(SystemParam)]
struct Gate<'w> {
    pending: Res<'w, PendingDecision>,
    order: Option<Res<'w, TurnOrder>>,
    phase: Res<'w, State<TurnPhase>>,
    paused: Res<'w, Paused>,
}

/// 命中测试给出的实体那一侧：动作、禁用标记、父链。
#[derive(SystemParam)]
struct Widgets<'w, 's> {
    actions: Query<'w, 's, &'static UiAction>,
    disabled: Query<'w, 's, &'static Disabled>,
    parents: Query<'w, 's, &'static ChildOf>,
}

/// 派发的三个出口。`AppExit` 在 0.20 也是消息，所以能和另两个收在一起；
/// `MessageWriter` 只带一个生命周期（`MessageReader` 才带两个，§六 坑 13）。
#[derive(SystemParam)]
struct Sends<'w> {
    rolls: MessageWriter<'w, RequestRoll>,
    answers: MessageWriter<'w, DecisionChosen>,
    exit: MessageWriter<'w, AppExit>,
}

/// 只动 UI 自己状态的那两个开关。
#[derive(SystemParam)]
struct UiState<'w> {
    toggle: ResMut<'w, pause::PauseMenuOpen>,
    selected: ResMut<'w, tile_info::SelectedTile>,
}

/// 读 `PointerClick`，沿父链找到动作后派发。
///
/// 「能不能做」全部在这里判一次，而不是散到各按钮的构造代码里：按钮只声明自己代表什么。
/// 这也意味着弹层开着时掷骰必然无效 —— 判据是 `PendingDecision`，不是「有没有遮罩」。
fn dispatch_ui_actions(
    mut clicks: MessageReader<PointerClick>,
    gate: Gate,
    widgets: Widgets,
    mut next_scene: ResMut<NextState<crate::scene::Scene>>,
    mut sends: Sends,
    mut ui: UiState,
) {
    for click in clicks.read() {
        if click.button != PointerButton::Primary {
            continue;
        }
        let Some((entity, action)) = action_of(&widgets.actions, &widgets.parents, click.entity)
        else {
            continue;
        };
        if widgets.disabled.get(entity).is_ok() {
            continue;
        }
        match action {
            UiAction::NewGame => next_scene.set(crate::scene::Scene::Board),
            UiAction::BackToMenu => next_scene.set(crate::scene::Scene::Menu),
            UiAction::Quit => {
                sends.exit.write(AppExit::Success);
            }
            // 掷骰：等轮到人、没暂停、且没有待决事项（判据与顶栏着色同源）
            UiAction::Roll => {
                if !can_roll(*gate.phase.get(), &gate.paused, &gate.pending) {
                    continue;
                }
                if let Some(player) = gate.order.as_ref().and_then(|o| o.current()) {
                    sends.rolls.write(RequestRoll { player });
                }
            }
            // 回答待决事项：主语取自 `PendingDecision`，不是「当前玩家」——
            // 两者在「A 踩了 B 的地、收据挂在 A 名下」这类场合就是不同的人
            UiAction::Decide(answer) => {
                if let Some((player, _)) = gate.pending.active() {
                    sends.answers.write(DecisionChosen {
                        player,
                        action: answer,
                    });
                }
            }
            UiAction::CloseInfo => ui.selected.0 = None,
            UiAction::TogglePause => ui.toggle.flip(),
        }
    }
}

/// 沿父链找第一个带 `UiAction` 的祖先，返回它和它的动作。
fn action_of(
    actions: &Query<&UiAction>,
    parents: &Query<&ChildOf>,
    hit: Entity,
) -> Option<(Entity, UiAction)> {
    let mut current = Some(hit);
    while let Some(entity) = current {
        if let Ok(action) = actions.get(entity) {
            return Some((entity, *action));
        }
        current = parents.get(entity).ok().map(|p| p.parent());
    }
    None
}

/// 悬停换色。同样走父链，否则鼠标移到按钮文字上就会「掉色」。
fn hover_feedback(
    mut overs: MessageReader<PointerOver>,
    mut outs: MessageReader<PointerOut>,
    actions: Query<&UiAction>,
    parents: Query<&ChildOf>,
    mut buttons: Query<(&mut BackgroundColor, &Palette, Option<&Disabled>)>,
) {
    let mut repaint = |event_entity: Entity, hover: bool| {
        let Some((entity, _)) = action_of(&actions, &parents, event_entity) else {
            return;
        };
        if let Ok((mut bg, palette, disabled)) = buttons.get_mut(entity)
            && disabled.is_none()
        {
            bg.0 = if hover { palette.hover } else { palette.normal };
        }
    };
    for e in overs.read() {
        repaint(e.entity, true);
    }
    for e in outs.read() {
        repaint(e.entity, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::message::Messages;
    use bevy::picking::{
        backend::HitData,
        pointer::{Location, PointerId},
    };
    use bevy::state::app::StatesPlugin;
    use bevy::window::WindowRef;
    use game_core::prelude::{Decision, LandId, Pending, PlayerId};
    use std::time::Duration;

    /// 造一条落在 `entity` 上的左键点击。这些字段平时由 picking 后端填，
    /// 测试里没有窗口也没有指针，只能手拼一份最小可信的。
    fn make_click(entity: Entity) -> PointerClick {
        // `NormalizedWindowRef` 的字段是私有的，只能借 `WindowRef::normalize` 造
        let window = WindowRef::Entity(Entity::PLACEHOLDER)
            .normalize(None)
            .expect("占位窗口应当能归一化");
        PointerClick {
            entity,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target: NormalizedRenderTarget::Window(window),
                    position: Vec2::ZERO,
                },
            ),
            button: PointerButton::Primary,
            hit: HitData {
                camera: Entity::PLACEHOLDER,
                depth: 0.0,
                position: None,
                normal: None,
                extra: None,
            },
            duration: Duration::ZERO,
            count: 1,
        }
    }

    /// 派发写出去的消息留在原地，由这条探针收下来供断言。
    #[derive(Resource, Default)]
    struct Seen {
        rolls: Vec<Entity>,
        answers: Vec<(Entity, Decision)>,
    }

    fn collect(
        mut seen: ResMut<Seen>,
        mut rolls: MessageReader<RequestRoll>,
        mut answers: MessageReader<DecisionChosen>,
    ) {
        seen.rolls.extend(rolls.read().map(|r| r.player));
        seen.answers
            .extend(answers.read().map(|a| (a.player, a.action)));
    }

    /// 一套只装配到「能跑 UI 派发」的对局骨架：两名玩家、一颗掷骰按钮（文字是它的子节点）、
    /// 一颗决策按钮、一颗灰掉的按钮、一层没有动作的遮罩，外加详情卡与暂停的两颗 UI 态按钮。
    struct Harness {
        app: App,
        /// `TurnOrder` 的当前玩家
        player: Entity,
        /// 另一名玩家：用来验证「决策的主语是待决玩家，不是当前玩家」
        other: Entity,
        roll_label: Entity,
        decide: Entity,
        disabled_roll: Entity,
        bare: Entity,
        close: Entity,
        pause: Entity,
    }

    impl Harness {
        fn new(phase: TurnPhase, paused: bool, pending: PendingDecision) -> Self {
            let mut app = App::new();
            app.add_plugins(MinimalPlugins)
                .add_plugins(StatesPlugin)
                .init_state::<TurnPhase>()
                .init_state::<crate::scene::Scene>()
                .add_message::<PointerClick>()
                .add_message::<RequestRoll>()
                .add_message::<DecisionChosen>()
                .add_message::<AppExit>()
                .init_resource::<Seen>()
                .init_resource::<tile_info::SelectedTile>()
                .init_resource::<pause::PauseMenuOpen>()
                .insert_resource(Paused(paused))
                .insert_resource(pending)
                .add_systems(Update, (dispatch_ui_actions, collect).chain());

            let player = app.world_mut().spawn(PlayerId(0)).id();
            let other = app.world_mut().spawn(PlayerId(1)).id();
            app.world_mut()
                .insert_resource(TurnOrder::new(vec![player, other]));
            app.world_mut()
                .resource_mut::<NextState<TurnPhase>>()
                .set(phase);

            let roll = app.world_mut().spawn(UiAction::Roll).id();
            let roll_label = app.world_mut().spawn(ChildOf(roll)).id();
            let decide = app
                .world_mut()
                .spawn(UiAction::Decide(Decision::Accept))
                .id();
            let disabled_roll = app.world_mut().spawn((UiAction::Roll, Disabled)).id();
            let bare = app.world_mut().spawn(Node::default()).id();
            let close = app.world_mut().spawn(UiAction::CloseInfo).id();
            let pause = app.world_mut().spawn(UiAction::TogglePause).id();

            // 先把阶段落实，再点击才不会落在 `Boot` 上
            app.update();
            Self {
                app,
                player,
                other,
                roll_label,
                decide,
                disabled_roll,
                bare,
                close,
                pause,
            }
        }

        /// 点一格实体，跑两帧收消息。
        fn click(&mut self, target: Entity) {
            self.app
                .world_mut()
                .resource_mut::<Messages<PointerClick>>()
                .write(make_click(target));
            self.app.update();
            self.app.update();
        }

        fn seen(&self) -> &Seen {
            self.app.world().resource::<Seen>()
        }
    }

    #[test]
    fn clicking_the_text_inside_the_roll_button_rolls() {
        // 命中测试报的是**最深的**节点，也就是按钮里的文字，所以父链必须走通
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.roll_label);
        assert_eq!(h.seen().rolls, vec![h.player]);
    }

    #[test]
    fn a_pending_decision_blocks_the_roll() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = h.other;
        h.app
            .world_mut()
            .resource_mut::<PendingDecision>()
            .offer(other, Pending::Buy(LandId(3)));
        h.click(h.roll_label);
        assert!(h.seen().rolls.is_empty(), "还有待决事项就掷骰了");
    }

    #[test]
    fn pausing_blocks_the_roll() {
        let mut h = Harness::new(TurnPhase::WaitRoll, true, PendingDecision::default());
        h.click(h.roll_label);
        assert!(h.seen().rolls.is_empty(), "暂停时不该放行掷骰");
    }

    #[test]
    fn a_disabled_button_does_not_dispatch() {
        // 阶段是对的、判据也是通的，唯一拦路的是 `Disabled` —— 顶栏着色与派发共用它
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.disabled_roll);
        assert!(h.seen().rolls.is_empty());
    }

    #[test]
    fn an_entity_without_an_action_is_dropped() {
        // 遮罩就是这一类：命中停在它身上，事件不会穿到顶栏按钮上
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.click(h.bare);
        assert!(h.seen().rolls.is_empty());
    }

    #[test]
    fn deciding_answers_for_the_pending_player_not_the_current_one() {
        let mut h = Harness::new(TurnPhase::WaitDecision, false, PendingDecision::default());
        let other = h.other;
        h.app
            .world_mut()
            .resource_mut::<PendingDecision>()
            .offer(other, Pending::RentNotice(LandId(3)));
        h.click(h.decide);
        assert_eq!(h.seen().answers, vec![(other, Decision::Accept)]);
    }

    #[test]
    fn the_ui_only_actions_write_ui_state() {
        let mut h = Harness::new(TurnPhase::WaitRoll, false, PendingDecision::default());
        h.app
            .world_mut()
            .resource_mut::<tile_info::SelectedTile>()
            .0 = Some((1, 1));

        h.click(h.close);
        assert_eq!(h.app.world().resource::<tile_info::SelectedTile>().0, None);

        h.click(h.pause);
        assert!(h.app.world().resource::<pause::PauseMenuOpen>().0);
    }
}
