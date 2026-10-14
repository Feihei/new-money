//! 暂停菜单。对应 Godot `scenes/ui/pause_menu.tscn` + `scripts/ui/pause_menu.gd`。
//!
//! 谁是真相：[`PauseMenuOpen`]（UI 状态）。它同时决定面板在不在、以及规则层的
//! [`Paused`] 闸开不开 —— 面板与闸门由同一个资源驱动，所以不会出现
//! 「菜单还开着但回合已经继续跑」这种 Godot 端要靠 `process_mode` 记住的配对。

use bevy::prelude::*;
use game_core::prelude::{Paused, TurnPhase};

use super::theme::{col, layout, size};
use super::{BoardUi, ButtonSpec, SECONDARY, UiAction, border_color, button, label, separator};
use crate::render::DevFont;

/// 暂停菜单开合。ESC 与「继续」按钮翻它，`sync_pause_menu` 把它抄给规则层。
#[derive(Resource, Default)]
pub(crate) struct PauseMenuOpen(pub bool);

impl PauseMenuOpen {
    pub(crate) fn flip(&mut self) {
        self.0 = !self.0;
    }

    /// `NM_PAUSE=1` 让程序开局即暂停。
    ///
    /// 加这个开关不是为了玩：agent 没有键盘可敲 ESC，暂停面板否则永远进不了截图，
    /// 而「没被看到过的界面」等于没做完。
    pub(crate) fn from_env() -> Self {
        Self(std::env::var("NM_PAUSE").as_deref() == Ok("1"))
    }
}

/// ESC 开合（Godot 的 `ui_cancel`）。
pub(crate) fn toggle_pause_on_escape(
    keys: Res<ButtonInput<KeyCode>>,
    mut open: ResMut<PauseMenuOpen>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        open.flip();
    }
}

/// 暂停面板的根节点。只用来让 [`sync_pause_menu`] 查得到「现在挂着没有」——
/// 存成 `Local<Option<Entity>>` 的话，回主菜单时面板被 `BoardUi` 收走、句柄却留在 Local 里，
/// 下一局开合暂停会对着一个已死的实体调 `despawn`。
#[derive(Component)]
pub(crate) struct PausePanel;

/// 面板跟着 `PauseMenuOpen` 生灭，闸门跟着面板走。
///
/// 停在 `WaitRoll`/`WaitDecision` 时暂停，回合机本来就在等人 —— 暂停的实质作用是
/// **停掉行走时钟**（`advance_walk_clock_system` 被 `not_paused` 挡住），
/// 于是「走到一半按暂停」能真的停在半路。
pub(crate) fn sync_pause_menu(
    mut commands: Commands,
    font: Res<DevFont>,
    phase: Res<State<TurnPhase>>,
    mut open: ResMut<PauseMenuOpen>,
    paused: Res<Paused>,
    panels: Query<Entity, With<PausePanel>>,
) {
    if paused.0 != open.0 {
        commands.insert_resource(Paused(open.0));
    }
    // 对局结束后没有可暂停的东西：把菜单关掉，别让「继续」按钮骗人
    if *phase.get() == TurnPhase::Over && open.0 {
        open.0 = false;
    }
    match (open.0, panels.iter().next()) {
        (true, None) => {
            spawn_pause_panel(&mut commands, &font);
        }
        (false, Some(entity)) => {
            commands.entity(entity).despawn();
        }
        _ => {}
    }
}

fn spawn_pause_panel(commands: &mut Commands, font: &DevFont) {
    let (title, title_font, title_color) =
        label(&font.0, "已暂停", size::PAUSE_TITLE, col::TITLE_GOLD);
    commands
        .spawn((
            BoardUi,
            PausePanel,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(col::PAUSE_DIM),
            ZIndex(6),
        ))
        .with_children(|mask| {
            mask.spawn((
                Node {
                    width: Val::Px(layout::PAUSE_PANEL[0]),
                    height: Val::Px(layout::PAUSE_PANEL[1]),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(layout::MENU_GAP),
                    padding: UiRect::axes(
                        Val::Px(layout::PANEL_PAD_X),
                        Val::Px(layout::PANEL_PAD_Y),
                    ),
                    border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
                    border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
                    ..default()
                },
                BackgroundColor(col::PANEL),
                border_color(col::BORDER_BUY),
            ))
            .with_children(|panel| {
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    title,
                    title_font,
                    title_color,
                ));
                panel.spawn(separator());
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new(
                        "继续",
                        [layout::PAUSE_PANEL[0] - layout::PANEL_PAD_X * 2.0, 44.0],
                        super::PRIMARY,
                        UiAction::TogglePause,
                    ),
                );
                // 设置在 Godot 端也是 print 一句未实现（pause_menu.gd:42-44），照摆但灰掉
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Px(44.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                        border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                        ..default()
                    },
                    BackgroundColor(col::BTN_DISABLED),
                    border_color(col::BTN_DISABLED),
                ));
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new(
                        "返回主菜单",
                        [layout::PAUSE_PANEL[0] - layout::PANEL_PAD_X * 2.0, 44.0],
                        SECONDARY,
                        UiAction::BackToMenu,
                    ),
                );
            });
        });
}
