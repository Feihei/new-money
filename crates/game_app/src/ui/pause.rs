//! 暂停菜单。对应 Godot `scenes/ui/pause_menu.tscn` + `scripts/ui/pause_menu.gd`。
//!
//! 谁是真相：[`PauseMenuOpen`]（UI 状态）。它同时决定面板在不在、以及规则层的
//! [`Paused`] 闸开不开 —— 面板与闸门由同一个资源驱动，所以不会出现
//! 「菜单还开着但回合已经继续跑」这种 Godot 端要靠 `process_mode` 记住的配对。
//!
//! P15b 起这里多了一件事：**保存进度**。能不能存由 core 的可存点判据给
//! （`SaveDenied` 每帧重算，与「按下去被拒」共用同一份事实），面板只负责把那句
//! 为什么写在按钮旁边 —— 灰按钮不带理由与坏掉在玩家眼里是同一件事。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{Paused, SaveDenied, SaveDeny, TurnPhase};

use super::skin::Skins;
use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, PRIMARY, SECONDARY, UiAction, border_color, button, label, separator,
};
use crate::paths::SaveNotice;
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
///
/// 同时带一份「上次画的是哪一版」的签名（`PauseStamp`）：保存那一行跟着「此刻能不能存」
/// 与「上一次存/读说了什么」变，而面板是一次建成的 —— 变了就整块重建，
/// 与手牌面板同一种做法（少一份「哪里要就地改」的清单）。
#[derive(Component)]
pub(crate) struct PausePanel;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct PauseStamp(pub String);

/// 面板那一侧要读的世界：回合在哪、此刻存不存得下、上次存/读说了什么。
///
/// 打包是房规（§六 坑 8）：这一条要读的东西随面板长大只会变多，而「参数确实太多了」
/// 这个真信号不该被一条 `#[allow]` 吃掉。
#[derive(SystemParam)]
pub(crate) struct PauseView<'w, 's> {
    phase: Res<'w, State<TurnPhase>>,
    denied: Res<'w, SaveDenied>,
    notice: Res<'w, SaveNotice>,
    panels: Query<'w, 's, (Entity, &'static PauseStamp), With<PausePanel>>,
}

/// 面板跟着 `PauseMenuOpen` 生灭，闸门跟着面板走。
///
/// 停在 `WaitRoll`/`WaitDecision` 时暂停，回合机本来就在等人 —— 暂停的实质作用是
/// **停掉行走时钟**（`advance_walk_clock_system` 被 `not_paused` 挡住），
/// 于是「走到一半按暂停」能真的停在半路。
pub(crate) fn sync_pause_menu(
    mut commands: Commands,
    font: Res<DevFont>,
    skins: Res<Skins>,
    mut open: ResMut<PauseMenuOpen>,
    paused: Res<Paused>,
    view: PauseView<'_, '_>,
) {
    if paused.0 != open.0 {
        commands.insert_resource(Paused(open.0));
    }
    // 对局结束后没有可暂停的东西：把菜单关掉，别让「继续」按钮骗人
    if *view.phase.get() == TurnPhase::Over && open.0 {
        open.0 = false;
    }
    // 签名要覆盖面板上每一个由判据决定的可见位（§6.1 坑 30）：存不存得下、那句话是什么、
    // 是成功还是失败。少一位就会出现「按钮还灰着，而旁边那句理由已经换了」
    let stamp = format!(
        "{:?}|{:?}|{}",
        view.denied.0, view.notice.text, view.notice.failed
    );
    if open.0 && view.panels.iter().any(|(_, shown)| shown.0 == stamp) {
        return;
    }
    for (entity, _) in &view.panels {
        commands.entity(entity).despawn();
    }
    if open.0 {
        spawn_pause_panel(
            &mut commands,
            &font,
            &skins,
            &view.denied,
            &view.notice,
            stamp,
        );
    }
}

/// 保存那一行的三个可见位：按不按得动、旁边写什么、那句话的颜色。
///
/// 抽成纯函数是为了能逐条打「每一种拒因都说得出为什么」——
/// 灰按钮不带理由与坏掉在玩家眼里是同一件事（坑 27 的 UI 版）。
fn save_row(denied: Option<SaveDeny>, notice: &SaveNotice, paper: bool) -> (bool, String, Color) {
    if let Some(why) = denied {
        // 存不了时那句话优先：上一次存成功过，不代表此刻还能存
        return (false, why.label().to_string(), col::BANKRUPT);
    }
    match &notice.text {
        Some(text) if notice.failed => (true, text.clone(), col::BANKRUPT),
        Some(text) => (
            true,
            text.clone(),
            if paper { col::INK } else { col::TITLE_GOLD },
        ),
        None => (
            true,
            "可存：轮到你的回合、掷骰之前".to_string(),
            if paper { col::INK_DIM } else { col::MUTED },
        ),
    }
}

fn spawn_pause_panel(
    commands: &mut Commands,
    font: &DevFont,
    skins: &Skins,
    denied: &SaveDenied,
    notice: &SaveNotice,
    stamp: String,
) {
    let paper = skins.float.image.is_some();
    let (save_enabled, save_line, save_color) = save_row(denied.0, notice, paper);
    let (pad_x, pad_y) = super::skin::inset(
        &skins.float,
        layout::PANEL_PAD_X,
        layout::PANEL_PAD_Y,
        layout::PAUSE_PANEL[0],
    );
    let inner_w = layout::PAUSE_PANEL[0] - pad_x * 2.0;
    let (title, title_font, title_color) = label(
        &font.0,
        "已暂停",
        size::PAUSE_TITLE,
        if paper { col::INK } else { col::TITLE_GOLD },
    );
    commands
        .spawn((
            BoardUi,
            PausePanel,
            PauseStamp(stamp),
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
            let mut panel_cmd = mask.spawn((
                Node {
                    width: Val::Px(layout::PAUSE_PANEL[0]),
                    height: Val::Px(layout::PAUSE_PANEL[1]),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(layout::MENU_GAP),
                    padding: UiRect::axes(Val::Px(pad_x), Val::Px(pad_y)),
                    border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
                    border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
                    ..default()
                },
                BackgroundColor(col::PANEL),
                border_color(col::BORDER_BUY),
            ));
            // 底板皮肤：盘上有 panel_modal 就叠九宫格底图，没有保持纯色面板。
            // 0.20 无 `Option<C>` bundle，spawn 后按句柄有无再 insert（与 `dialog::frame` 同一条）
            if let Some((skin_node, skin_image)) = super::skin::panel_image(&skins.float) {
                panel_cmd.with_children(|p| {
                    p.spawn((skin_node, skin_image));
                });
            }
            panel_cmd.with_children(|panel| {
                panel.spawn((
                    Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    TextLayout::justify(Justify::Center),
                    title,
                    title_font,
                    title_color,
                ));
                panel.spawn(separator());
                // 保存（P15b）。灰的时候旁边那句就是规则层拒绝时说的话 ——
                // 两句话出自同一次判断（`game_core::prelude::judge`）
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new("保存进度", [inner_w, 44.0], PRIMARY, UiAction::SaveGame)
                        .disabled(!save_enabled),
                );
                panel.spawn((
                    // 宽度钉在面板内沿：失败那句里带着路径，不钉住它会画到边框外面
                    Node {
                        width: Val::Px(inner_w),
                        ..default()
                    },
                    label(&font.0, save_line, size::SMALL, save_color),
                ));
                panel.spawn(separator());
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new("继续", [inner_w, 44.0], PRIMARY, UiAction::TogglePause),
                );
                // 设置在 Godot 端也是 print 一句未实现（pause_menu.gd:42-44），照摆但灰掉。
                // P15c 把它做在了主菜单上，这里就写明「去哪改」—— 空灰盒子读起来像坏了
                panel
                    .spawn((
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
                    ))
                    .with_child(label(
                        &font.0,
                        "设置（回主菜单改）",
                        size::BODY,
                        col::BTN_TEXT_DISABLED,
                    ));
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new(
                        "返回主菜单",
                        [inner_w, 44.0],
                        SECONDARY,
                        UiAction::BackToMenu,
                    ),
                );
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 每一种拒因都要在按钮旁边写得出自己那一句（同 `ShopDeny` 的纪律），而可存点上
    /// 那颗按钮必须是亮的 —— 一条永远灰着的按钮与坏掉同形，所以两半都验。
    #[test]
    fn every_save_denial_says_its_own_why_and_the_save_point_is_live() {
        let notice = SaveNotice::default();
        let (enabled, text, _) = save_row(None, &notice, false);
        assert!(enabled, "可存点上按钮却灰着");
        assert!(text.contains("可存"), "亮的按钮也该说一句此刻是什么状态");

        for why in SaveDeny::ALL {
            let (enabled, text, color) = save_row(Some(why), &notice, false);
            assert!(!enabled, "{why:?} 下按钮还亮着");
            assert_eq!(text, why.label(), "{why:?} 的灰按钮没念出它自己那句话");
            assert_eq!(color, col::BANKRUPT, "{why:?} 的理由不是警告色");
        }
    }

    /// 回执与拒因是两件事：**存失败之后仍然可以再存一次**（磁盘满过不代表这一刻不能存），
    /// 而成功那句话说的是「已保存」，不是「可以存」。
    #[test]
    fn a_notice_does_not_turn_the_save_point_off() {
        let ok = SaveNotice {
            text: Some("已保存到 save.json".into()),
            failed: false,
        };
        let (enabled, text, color) = save_row(None, &ok, false);
        assert!(enabled);
        assert_eq!(text, "已保存到 save.json");
        assert_eq!(color, col::TITLE_GOLD);

        let bad = SaveNotice {
            text: Some("保存失败：磁盘满了".into()),
            failed: true,
        };
        let (enabled, text, color) = save_row(None, &bad, false);
        assert!(enabled, "存失败过不代表此刻不能存");
        assert!(text.starts_with("保存失败"));
        assert_eq!(color, col::BANKRUPT);

        // 而「存不了」优先于任何回执：上一次成功了，此刻不在可存点上就别让它亮
        let (enabled, text, _) = save_row(Some(SaveDeny::Walking), &ok, false);
        assert!(!enabled);
        assert_eq!(text, SaveDeny::Walking.label());
    }

    /// 贴了剪纸底板的面板中央是米色纸面，`TITLE_GOLD`/`MUTED` 那一族读不出来：
    /// 同一条回执在纸面上必须换成深色，素材缺席时仍走原色。
    #[test]
    fn a_paper_panel_swaps_the_save_line_to_ink() {
        let notice = SaveNotice::default();
        let (_, _, color) = save_row(None, &notice, true);
        assert_eq!(color, col::INK_DIM);

        let ok = SaveNotice {
            text: Some("已保存到 save.json".into()),
            failed: false,
        };
        let (_, _, color) = save_row(None, &ok, true);
        assert_eq!(color, col::INK);

        // 失败那一句仍然是警告色，不因纸面换色
        let bad = SaveNotice {
            text: Some("保存失败：磁盘满了".into()),
            failed: true,
        };
        let (_, _, color) = save_row(None, &bad, true);
        assert_eq!(color, col::BANKRUPT);
    }
}
