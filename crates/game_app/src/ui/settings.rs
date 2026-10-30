//! 设置面板（P15c）：行走速度与调试面板。
//!
//! 只有主菜单进得来（面板挂 `MenuPiece`，进对局随菜单一起收走）。两件真控件 ——
//! 音量等 P16（今天没有音频管线，摆上去按了不会有任何东西响），起始资金四档仍归 P2c。
//!
//! 「改完立刻生效」与「下次启动才生效」在同一个屏上并存，所以每一行都得说清楚：
//! 行走速度是**立刻**的（`settings::apply_walk_speed` 每帧把它抄进 `GameRules`），
//! 调试面板要重开进程（egui 插件一旦装上就开始吃输入，不能在运行中装卸）。
//!
//! 面板按签名重建（同手牌/暂停）：速度、开关、上一次写盘的成败，三位都是可见位。

use bevy::prelude::*;

use super::theme::{col, layout, size};
use super::{
    ButtonSpec, MenuPiece, PRIMARY, SECONDARY, UiAction, border_color, button, label, separator,
};
use crate::render::DevFont;
use crate::settings::{SPEED_STEPS, Settings, SettingsStatus};

/// 面板开着吗。菜单那颗「设置」翻它，离开菜单时 [`super::reset_ui_state`] 关掉它。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct SettingsOpen(pub bool);

#[derive(Component)]
pub(crate) struct SettingsPanel;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct SettingsStamp(pub String);

pub(crate) fn sync_settings_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    open: Res<SettingsOpen>,
    settings: Res<Settings>,
    status: Res<SettingsStatus>,
    shown: Query<(Entity, &SettingsStamp)>,
) {
    let stamp = format!("{settings:?}|{status:?}");
    if open.0 && shown.iter().any(|(_, seen)| seen.0 == stamp) {
        return;
    }
    for (entity, _) in &shown {
        commands.entity(entity).despawn();
    }
    if open.0 {
        spawn_panel(&mut commands, &font, &settings, &status, stamp);
    }
}

fn spawn_panel(
    commands: &mut Commands,
    font: &DevFont,
    settings: &Settings,
    status: &SettingsStatus,
    stamp: String,
) {
    let width = 460.0;
    let inner = width - layout::PANEL_PAD_X * 2.0;
    commands
        .spawn((
            MenuPiece,
            SettingsPanel,
            SettingsStamp(stamp),
            // 全屏遮罩：菜单那几颗按钮不能被点穿（同暂停面板的理由）
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(col::PAUSE_DIM),
            GlobalZIndex(20),
        ))
        .with_children(|mask| {
            mask.spawn((
                Node {
                    width: Val::Px(width),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
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
                panel.spawn(label(&font.0, "设置", size::DIALOG_TITLE, col::TITLE_GOLD));
                panel.spawn(separator());

                // ---- 行走速度 ----
                panel.spawn((
                    Node {
                        width: Val::Px(inner),
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    label(&font.0, "行走速度", size::BODY, col::HINT),
                ));
                panel
                    .spawn((Node {
                        width: Val::Px(inner),
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        ..default()
                    },))
                    .with_children(|line| {
                        // 五档一排。当前那一档带「●」而不是灰掉：灰掉读起来是「不能选」，
                        // 而它恰恰是**已经选着**的那一档
                        for (index, (secs, name)) in SPEED_STEPS.iter().enumerate() {
                            let current = *secs == settings.walk_step_interval;
                            let text = if current {
                                format!("● {name}")
                            } else {
                                (*name).to_string()
                            };
                            button(
                                line,
                                &font.0,
                                ButtonSpec {
                                    font: size::ROW,
                                    ..ButtonSpec::new(
                                        &text,
                                        [76.0, 36.0],
                                        SECONDARY,
                                        UiAction::SetWalkSpeed(index),
                                    )
                                },
                            );
                        }
                    });
                panel.spawn(label(
                    &font.0,
                    format!("每格 {:.2} 秒 · 改完立刻生效", settings.walk_step_interval),
                    size::SMALL,
                    col::MUTED,
                ));

                // ---- 调试面板 ----
                panel
                    .spawn((Node {
                        width: Val::Px(inner),
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(12.0),
                        ..default()
                    },))
                    .with_children(|line| {
                        line.spawn((
                            Node {
                                flex_grow: 1.0,
                                ..default()
                            },
                            label(&font.0, "调试面板（只读 dump）", size::BODY, col::HINT),
                        ));
                        button(
                            line,
                            &font.0,
                            ButtonSpec::new(
                                if settings.debug_panel { "开" } else { "关" },
                                [72.0, 36.0],
                                if settings.debug_panel {
                                    PRIMARY
                                } else {
                                    SECONDARY
                                },
                                UiAction::ToggleDebugPanel,
                            ),
                        );
                    });
                panel.spawn(label(
                    &font.0,
                    "调试面板下次启动生效（egui 装上就开始吃输入，不能中途装卸）",
                    size::SMALL,
                    col::MUTED,
                ));

                // ---- 上一次写盘的结果 ----
                if let Some(text) = &status.text {
                    panel.spawn(label(
                        &font.0,
                        text.clone(),
                        size::SMALL,
                        if status.failed {
                            col::BANKRUPT
                        } else {
                            col::TITLE_GOLD
                        },
                    ));
                }

                panel.spawn(separator());
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new("关闭", [inner, 42.0], SECONDARY, UiAction::ToggleSettings),
                );
            });
        });
}
