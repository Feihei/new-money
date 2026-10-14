//! 主菜单。对应 Godot `scenes/ui/main_menu.tscn` + `scripts/ui/main_menu.gd`。

use bevy::prelude::*;

use super::theme::{col, layout, size};
use super::{ButtonSpec, MenuPiece, SECONDARY, UiAction, button, label, separator};
use crate::render::DevFont;

/// 菜单自带一台相机。
///
/// Bevy 的 UI 必须有相机才画得出来，而对局的 `Camera3d` 要等 `ActiveBoard` 才 spawn
/// （棋盘没装配好就摆相机，拍到的是一片空白）。少了这台相机，主菜单就是一张纯黑窗口
/// —— 没有报错、没有日志，只有黑屏。
pub fn spawn_menu_camera(mut commands: Commands) {
    commands.spawn((
        MenuPiece,
        Camera2d,
        Camera {
            clear_color: ClearColorConfig::Custom(col::MENU_BG),
            ..default()
        },
    ));
}

pub fn spawn_menu(mut commands: Commands, font: Res<DevFont>) {
    // 整屏底色（`main_menu.tscn:14-21` 的 ColorRect），同时充当居中容器
    commands
        .spawn((
            MenuPiece,
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            BackgroundColor(col::MENU_BG),
            GlobalZIndex(10),
        ))
        .with_children(|screen| {
            screen
                .spawn(Node {
                    width: Val::Px(layout::MENU_WIDTH),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    row_gap: Val::Px(layout::MENU_GAP),
                    ..default()
                })
                .with_children(|stack| {
                    stack.spawn(label(&font.0, "暴富", size::MENU_TITLE, col::MENU_TITLE));
                    stack.spawn(label(&font.0, "New Money", size::BODY, col::MUTED));
                    stack.spawn(separator());
                    menu_button(stack, &font.0, "新游戏", UiAction::NewGame);
                    // 读档与设置在 Godot 端只 print 一句未实现（main_menu.gd:23-30）。
                    // 这里照摆但灰掉：能点却什么都不做的按钮比没有按钮更糟。
                    stub_button(stack, &font.0, "读档");
                    stub_button(stack, &font.0, "设置");
                    menu_button(stack, &font.0, "退出", UiAction::Quit);
                    stack.spawn(separator());
                    stack.spawn(label(
                        &font.0,
                        "v0.1.0 · Bevy 0.20 重写 · B5",
                        size::SMALL,
                        col::MENU_FOOTER,
                    ));
                });
        });
}

fn menu_button(
    stack: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    text: &str,
    action: UiAction,
) {
    button(
        stack,
        font,
        ButtonSpec::new(
            text,
            [layout::MENU_WIDTH, layout::MENU_BUTTON_H],
            SECONDARY,
            action,
        ),
    );
}

/// 未实现的按钮：一个灰盒子加文字，**不挂 `UiAction`** —— 没有动作可派发，
/// 比挂一个「被 `Disabled` 挡住的假动作」少一处将来会咬人的谎。
fn stub_button(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, text: &str) {
    parent
        .spawn((
            Node {
                width: Val::Percent(100.0),
                height: Val::Px(layout::MENU_BUTTON_H),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                ..default()
            },
            BackgroundColor(col::BTN_DISABLED),
            super::border_color(col::BTN_DISABLED),
        ))
        .with_children(|b| {
            b.spawn(label(font, text, size::BODY, col::BTN_TEXT_DISABLED));
        });
}
