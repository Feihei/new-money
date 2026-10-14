//! 占位 HUD：B5 的顶栏 + `player_hud` 会整块换掉它。
//!
//! 现在只留三行。后两行**必须是活的** —— 否则 B4 的截图上挂着一行「$20000」的开局快照，
//! 看着像结算根本没跑，而这是验收时最容易骗过眼睛的一种假象。

use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, GameSession, PlayerId, PlayerName, TurnOrder, TurnPhase, Wallet,
};

use crate::render::DevFont;

#[derive(Component)]
pub(crate) struct TurnLine;

#[derive(Component)]
pub(crate) struct RosterLine;

pub(crate) fn spawn_hud(mut commands: Commands, active: Res<ActiveBoard>, font: Res<DevFont>) {
    let style = |px: f32| TextFont {
        font: font.0.clone().into(),
        font_size: FontSize::Px(px),
        ..default()
    };
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                top: px(12),
                left: px(12),
                padding: UiRect::axes(px(12), px(8)),
                row_gap: px(6),
                display: Display::Flex,
                flex_direction: FlexDirection::Column,
                ..default()
            },
            BackgroundColor(Color::srgba(1.0, 1.0, 1.0, 0.78)),
        ))
        .with_children(|panel| {
            panel.spawn((
                Text::new(format!(
                    "暴富 · B4 等轴测渲染 · 棋盘「{}」",
                    active.def.board_name
                )),
                style(24.0),
                TextColor(Color::srgb(0.08, 0.08, 0.1)),
            ));
            panel.spawn((
                TurnLine,
                Text::new("准备中"),
                style(20.0),
                TextColor(Color::srgb(0.1, 0.25, 0.55)),
            ));
            panel.spawn((
                RosterLine,
                Text::new(String::new()),
                style(20.0),
                TextColor(Color::srgb(0.45, 0.2, 0.05)),
            ));
        });
}

/// 每帧拼一次、只在文本真的变了才写：反复赋同一个字符串会让文本排版每帧重来。
pub(crate) fn refresh_hud(
    order: Res<TurnOrder>,
    session: Res<GameSession>,
    phase: Res<State<TurnPhase>>,
    players: Query<(&PlayerId, &PlayerName, &Wallet)>,
    mut lines: Query<(&mut Text, Option<&TurnLine>, Option<&RosterLine>)>,
    mut last: Local<(String, String)>,
) {
    let current = order
        .current()
        .and_then(|e| players.get(e).ok())
        .map(|(_, name, _)| name.0.clone());
    let turn_text = match &current {
        Some(name) => format!("回合 {} · 当前：{name}", session.turn_count),
        None => format!("回合 {} · {:?}", session.turn_count, phase.get()),
    };
    // `Query` 的迭代顺序不保证，按 `PlayerId` 排一下，免得名单每帧换顺序
    let mut rows: Vec<(PlayerId, String, i64)> = players
        .iter()
        .map(|(id, name, wallet)| (*id, name.0.clone(), wallet.cash))
        .collect();
    rows.sort_by_key(|(id, _, _)| *id);
    let roster = rows
        .iter()
        .map(|(_, name, cash)| format!("{name} ${cash}"))
        .collect::<Vec<_>>()
        .join("　");

    if last.0 == turn_text && last.1 == roster {
        return;
    }
    let mut wrote = false;
    for (mut text, is_turn, is_roster) in &mut lines {
        if is_turn.is_some() {
            *text = Text::new(turn_text.clone());
            wrote = true;
        } else if is_roster.is_some() {
            *text = Text::new(roster.clone());
            wrote = true;
        }
    }
    // 面板是上一帧的 `Commands` 才 spawn 出来的：这一帧一个都没匹配到时，
    // 不能把文本记成「已是最新」—— 那样第一帧就把后面每一帧都短路掉了
    if wrote {
        last.0 = turn_text;
        last.1 = roster;
    }
}
