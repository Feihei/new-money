//! 主菜单。对应 Godot `scenes/ui/main_menu.tscn` + `scripts/ui/main_menu.gd`。

use bevy::prelude::*;

use super::theme::{col, layout, size};
use super::{ButtonSpec, MenuPiece, SECONDARY, UiAction, button, label, separator};
use crate::art;
use crate::paths::{SaveNotice, save_exists};
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

pub fn spawn_menu(
    mut commands: Commands,
    font: Res<DevFont>,
    server: Res<AssetServer>,
    notice: Res<SaveNotice>,
) {
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
                    // 读档（P15b）：不再是灰盒子。有档才亮；没档时灰着并**自带一句为什么**
                    // —— 「没有存档」不是玩家猜得到的事（他可能刚存过，只是 `NM_SAVE_DIR`
                    // 指去了另一个目录）
                    let has_save = save_exists();
                    button(
                        stack,
                        &font.0,
                        ButtonSpec::new(
                            "读档",
                            [layout::MENU_WIDTH, layout::MENU_BUTTON_H],
                            SECONDARY,
                            UiAction::LoadGame,
                        )
                        .disabled(!has_save),
                    );
                    if !has_save {
                        stack.spawn(label(
                            &font.0,
                            "没有存档 —— 对局里暂停后可以保存",
                            size::SMALL,
                            col::KEY,
                        ));
                    }
                    // 存档/读档最后那句话（失败时说为什么）。空的时候不占位：
                    // `refresh_notice` 把它写成空串，行高按空文本算
                    stack.spawn((
                        MenuNotice,
                        label(
                            &font.0,
                            notice_text(&notice),
                            size::SMALL,
                            notice_color(&notice),
                        ),
                    ));
                    // 设置在 Godot 端只 print 一句未实现（main_menu.gd:23-30）。这里照摆但灰掉，
                    // 并把「为什么按不动」写在按钮上（P15c 才做）—— 空灰盒子读起来像坏了
                    stub_button(stack, &font.0, "设置（暂未开放）");
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

/// 菜单上那一行「存档/读档最后怎么了」。没话说时是空串，不摆占位符。
#[derive(Component)]
pub(crate) struct MenuNotice;

fn notice_text(notice: &SaveNotice) -> String {
    notice.text.clone().unwrap_or_default()
}

fn notice_color(notice: &SaveNotice) -> Color {
    if notice.failed {
        col::BANKRUPT
    } else {
        col::TITLE_GOLD
    }
}

/// 派发在点击那一帧写 `SaveNotice`（读档失败），这条排在它后面把那一行改掉。
///
/// 不做签名记忆：这一行只有一句字，逐帧写一个 `String` 比维护「上次写成什么」
/// 便宜，而那份记忆正是坑 22 那一族（跨局残留）的常客。
pub(crate) fn refresh_notice(
    notice: Res<SaveNotice>,
    mut lines: Query<(&mut Text, &mut TextColor), With<MenuNotice>>,
) {
    for (mut text, mut color) in &mut lines {
        let want = notice_text(&notice);
        if text.0 != want {
            text.0 = want;
        }
        let want_color = notice_color(&notice);
        if color.0 != want_color {
            color.0 = want_color;
        }
    }
}
