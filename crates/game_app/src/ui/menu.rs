//! 主菜单。对应 Godot `scenes/ui/main_menu.tscn` + `scripts/ui/main_menu.gd`。

use bevy::prelude::*;

use super::theme::{col, layout, size};
use super::{ButtonSpec, MenuPiece, SECONDARY, UiAction, button, label, separator};
use crate::art;
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

pub fn spawn_menu(mut commands: Commands, font: Res<DevFont>, server: Res<AssetServer>) {
    let bg = art::menu_bg();
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
            // 背景图排在按钮那一叠之前，且 `ZIndex(-1)`：那张牌楼的正中是一片白天空，
            // 而标题是黄的 —— 不垫一块板就看不见「新游戏」四个字（见下面 `scrim`）
            if let Some(bg) = &bg {
                screen.spawn((
                    ImageNode {
                        image: server.load(bg.clone()),
                        // 1920×1088 的画铺满任意比例的窗口：先看感觉，裁切与构图等素材定稿再谈
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Node {
                        position_type: PositionType::Absolute,
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    ZIndex(-1),
                ));
            }
            // 没有背景图时这块板保持全透明：与素材上路前逐像素同一张画面
            let scrim = if bg.is_some() {
                col::MENU_SCRIM
            } else {
                Color::NONE
            };
            screen
                .spawn((
                    Node {
                        // `BoxSizing::BorderBox` 是 Bevy 的默认：`width` 含 padding，
                        // 所以这一块板要比按钮宽出两份内边距
                        width: Val::Px(layout::MENU_WIDTH + 2.0 * layout::MENU_PAD),
                        flex_direction: FlexDirection::Column,
                        align_items: AlignItems::Stretch,
                        row_gap: Val::Px(layout::MENU_GAP),
                        padding: UiRect::axes(
                            Val::Px(layout::MENU_PAD),
                            Val::Px(layout::MENU_PAD * 1.25),
                        ),
                        border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
                        ..default()
                    },
                    BackgroundColor(scrim),
                ))
                .with_children(|stack| {
                    stack.spawn(label(&font.0, "暴富", size::MENU_TITLE, col::MENU_TITLE));
                    stack.spawn(label(&font.0, "New Money", size::BODY, col::MUTED));
                    stack.spawn(separator());
                    menu_button(stack, &font.0, "新游戏", UiAction::NewGame);
                    // 游乐场练习（P7）：三款小游戏各玩一手，**不给点券**。挂在菜单上而不是
                    // 对局里，是为了让「这三个玩法本身好不好玩」能被单独评 —— 而它不给点券
                    // 这件事写在结算屏上，不是「菜单忘了发消息」（见 `ui/minigame.rs` 模块头）
                    menu_button(stack, &font.0, "游乐场练习", UiAction::ToggleArcadeChoice);
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
