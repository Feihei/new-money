//! 底部玩家卡：颜色点、名字、现金、地产数与状态，高亮当前回合那位。
//!
//! 对应 Godot `player_hud.gd`（220 行、七个信号回调各自找自己的 label 改文字）。
//!
//! 移植成「一次装配 + 每帧按世界状态重算」：信号驱动的问题是少接一个就有一处数字不更新，
//! 而「地产数没跟着破产变」这类错平时看不出来，只在关键回合暴露。改成从组件直接算之后，
//! 卡片内容与 `Owner`/`Wallet` 不可能不一致（§5.1 差异表）。
//!
//! 整块挂 [`Pickable::IGNORE`]：它横在棋盘下方，一旦参与命中测试就会吃掉那一片的点格点击
//! （Godot 端同一个 `mouse_filter = MOUSE_FILTER_IGNORE`，`player_hud.gd:20`）。

use bevy::prelude::*;
use game_core::prelude::{
    Bankrupt, JailSentence, LandId, Owner, PlayerColor, PlayerId, PlayerName, TurnOrder, Wallet,
};

use super::theme::{col, layout, size};
use super::{BoardUi, label};
use crate::render::DevFont;

/// 一张玩家卡：四段文本 + 颜色点，外加「上次写进去的整行内容」。
///
/// 缓存整张卡而不是逐段比对，是因为这四段本来就要一起变：一次比较，要么全写要么全不写。
/// 高亮另记一个 `bool` —— 它是样式，不在文本里。
#[derive(Component)]
pub(crate) struct PlayerCard {
    player: Entity,
    name: Entity,
    cash: Entity,
    props: Entity,
    status: Entity,
    cached: String,
    highlighted: bool,
}

pub(crate) fn spawn_player_hud(
    mut commands: Commands,
    font: Res<DevFont>,
    players: Query<(Entity, &PlayerId, &PlayerName, &Wallet, &PlayerColor)>,
) {
    let mut sorted: Vec<_> = players.iter().collect();
    sorted.sort_by_key(|(_, id, ..)| id.0);

    let list = commands
        .spawn((
            BoardUi,
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                // Godot 的 `offset_top = -120` 是「从底边往上 120」，Bevy 的 `top` 是「从
                // 父节点顶边往下 120」。照抄数字会把整条 HUD 甩到屏幕上沿，得翻成 bottom+height
                bottom: Val::Px(layout::HUD_BOTTOM[1]),
                left: Val::Px(layout::HUD_BOTTOM[2]),
                right: Val::Px(layout::HUD_BOTTOM[3]),
                height: Val::Px(layout::HUD_BOTTOM[0]),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::HUD_CARD_GAP),
                ..default()
            },
        ))
        .id();

    for (player, _, name, wallet, color) in sorted {
        let card = commands
            .spawn((
                Pickable::IGNORE,
                ChildOf(list),
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(layout::HUD_ROW_GAP),
                    padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                    border: UiRect::all(Val::Px(0.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(col::HUD_CARD),
                super::border_color(Color::NONE),
            ))
            .id();

        // 颜色圆点：Godot 用 `ColorRect` + `SIZE_SHRINK_CENTER`，这里靠父级的
        // `align_items: Center` 居中，圆角取半径当成圆
        commands.spawn((
            Pickable::IGNORE,
            ChildOf(card),
            Node {
                width: Val::Px(layout::HUD_DOT),
                height: Val::Px(layout::HUD_DOT),
                border_radius: BorderRadius::all(Val::Px(layout::HUD_DOT / 2.0)),
                ..default()
            },
            BackgroundColor(Color::srgb(color.0[0], color.0[1], color.0[2])),
        ));

        let (t, f, c) = label(&font.0, name.0.clone(), size::NAME, col::HUD_NAME);
        let name_e = commands
            .spawn((
                ChildOf(card),
                Node {
                    min_width: Val::Px(80.0),
                    ..default()
                },
                t,
                f,
                c,
            ))
            .id();
        let (t, f, c) = label(
            &font.0,
            format!("${}", wallet.cash),
            size::BODY,
            col::HUD_CASH,
        );
        let cash_e = commands
            .spawn((
                ChildOf(card),
                Node {
                    min_width: Val::Px(100.0),
                    ..default()
                },
                t,
                f,
                c,
            ))
            .id();
        let (t, f, c) = label(&font.0, "地产:0", size::SMALL, col::HUD_PROPS);
        let props_e = commands.spawn((ChildOf(card), t, f, c)).id();
        let (t, f, c) = label(&font.0, String::new(), size::SMALL, col::BODY);
        // 状态标签贴右（`SIZE_EXPAND_FILL` + `HORIZONTAL_ALIGNMENT_RIGHT`）
        let status_e = commands
            .spawn((
                ChildOf(card),
                Node {
                    flex_grow: 1.0,
                    justify_content: JustifyContent::FlexEnd,
                    ..default()
                },
                TextLayout::justify(Justify::Right),
                t,
                f,
                c,
            ))
            .id();

        commands.entity(card).insert(PlayerCard {
            player,
            name: name_e,
            cash: cash_e,
            props: props_e,
            status: status_e,
            cached: String::new(),
            highlighted: false,
        });
    }
}

/// 一名玩家在 HUD 上需要的全部状态。起别名是因为这个五元组写进 `Query<...>` 会撞
/// clippy 的 `type_complexity`，而拆成两个查询又要多遍历一遍实体。
type HudPlayer<'w> = (
    &'w PlayerId,
    &'w PlayerName,
    &'w Wallet,
    Option<&'w JailSentence>,
    Option<&'w Bankrupt>,
);

/// 每帧从世界状态重算四段文案，只在真的变了才写文本（反复赋同一个字符串会让排版重来）。
///
/// 归属用的是 `PlayerId` 而不是实体句柄（`land.rs:20` 的 `Owner(pub Option<PlayerId>)`），
/// 所以地产数按 `PlayerId` 数 —— 混用两种指代就会算出「同一块地有两个主人」。
pub(crate) fn refresh_player_hud(
    order: Res<TurnOrder>,
    players: Query<HudPlayer<'_>>,
    lands: Query<&Owner, With<LandId>>,
    mut cards: Query<(
        &mut PlayerCard,
        &mut BackgroundColor,
        &mut BorderColor,
        &mut Node,
    )>,
    mut texts: Query<&mut Text>,
    mut colors: Query<&mut TextColor>,
) {
    let current = order.current();
    let mut owned: Vec<(PlayerId, usize)> = players.iter().map(|(id, ..)| (*id, 0usize)).collect();
    for owner in &lands {
        if let Some(id) = owner.0
            && let Some(row) = owned.iter_mut().find(|(pid, _)| *pid == id)
        {
            row.1 += 1;
        }
    }

    for (mut card, mut bg, mut border, mut node) in &mut cards {
        let Ok((pid, name, wallet, jailed, bankrupt)) = players.get(card.player) else {
            continue;
        };
        let props = owned
            .iter()
            .find(|(id, _)| *id == *pid)
            .map_or(0, |(_, count)| *count);
        // 文案与配色照 `player_hud.gd:191-199`；破产优先于服刑（同时成立时人已经出局）
        let (status, status_color) = match (bankrupt, jailed) {
            (Some(_), _) => ("[破产]", col::BANKRUPT),
            (None, Some(_)) => ("[监狱]", col::JAILED),
            (None, None) => ("", col::BODY),
        };
        let cash = format!("${}", wallet.cash);

        let line = format!("{}|{cash}|{props}|{status}", name.0);
        if line != card.cached {
            card.cached = line;
            write(&mut texts, card.name, &name.0);
            write(&mut texts, card.cash, &cash);
            write(&mut texts, card.props, &format!("地产:{props}"));
            write(&mut texts, card.status, status);
            if let Ok(mut color) = colors.get_mut(card.status) {
                color.0 = status_color;
            }
        }

        let active = current == Some(card.player);
        if active != card.highlighted {
            card.highlighted = active;
            bg.0 = if active {
                col::HUD_CARD_ACTIVE
            } else {
                col::HUD_CARD
            };
            let edge = if active {
                col::HUD_BORDER_ACTIVE
            } else {
                Color::NONE
            };
            border.top = edge;
            border.right = edge;
            border.bottom = edge;
            border.left = edge;
            node.border = UiRect::all(Val::Px(if active { 2.0 } else { 0.0 }));
        }
    }
}

fn write(texts: &mut Query<&mut Text>, target: Entity, value: &str) {
    if let Ok(mut text) = texts.get_mut(target) {
        *text = Text::new(value);
    }
}
