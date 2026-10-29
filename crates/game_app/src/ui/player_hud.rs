//! 底部玩家卡：颜色点、名字、现金、点券、地产数与状态，高亮当前回合那位。
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
    Bankrupt, Controller, Detention, Heading, Inventory, LandId, Mount, MoveCap, Owner,
    PlayerColor, PlayerId, PlayerName, Points, Possession, StaySentence, TurnOrder, Wallet,
};

use super::theme::{col, layout, size};
use super::{BoardUi, label};
use crate::render::DevFont;

/// 一张玩家卡：五段文本 + 颜色点，外加「上次写进去的整行内容」。
///
/// 缓存整张卡而不是逐段比对，是因为这几段本来就要一起变：一次比较，要么全写要么全不写。
/// 高亮另记一个 `bool` —— 它是样式，不在文本里。
#[derive(Component)]
pub(crate) struct PlayerCard {
    player: Entity,
    name: Entity,
    cash: Entity,
    points: Entity,
    props: Entity,
    status: Entity,
    cached: String,
    highlighted: bool,
}

pub(crate) fn spawn_player_hud(
    mut commands: Commands,
    font: Res<DevFont>,
    players: Query<(
        Entity,
        &PlayerId,
        &PlayerName,
        &Wallet,
        &Points,
        &PlayerColor,
    )>,
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
                // 高度**跟着人数长**。原来钉的是 `HUD_BOTTOM[0]`（112 = 两张卡的高度），
                // P14a 之后默认局有四个座位，钉死的高度装不下四行 —— 而 bevy_ui 不裁子节点
                // （§6.1 坑 34），第四行不是被裁掉而是**画到窗口下沿之外**，症状是
                // 「有一个玩家整个 HUD 看不见」，看着像 HUD 坏了。`bottom` 锚住 + `Auto` 高度
                // 就是往上长，一位与八位都刚好贴着底边
                height: Val::Auto,
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::HUD_CARD_GAP),
                ..default()
            },
        ))
        .id();

    for (player, _, name, wallet, points, color) in sorted {
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
        // 第二套账单独一段：与现金并排、另取一色，「永不通兑」在界面上的表达就是
        // 它是另一个数字，不是 `$` 后面多出来的几位
        let (t, f, c) = label(
            &font.0,
            format!("点券:{}", points.0),
            size::BODY,
            col::HUD_POINTS,
        );
        let points_e = commands
            .spawn((
                ChildOf(card),
                Node {
                    min_width: Val::Px(84.0),
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
            points: points_e,
            props: props_e,
            status: status_e,
            cached: String::new(),
            highlighted: false,
        });
    }
}

/// 一名玩家在 HUD 上需要的全部状态。起别名是因为这个八元组写进 `Query<...>` 会撞
/// clippy 的 `type_complexity`，而拆成两个查询又要多遍历一遍实体。
///
/// `Heading` 是**必挂**的那一格（`spawn_players` 给的），不用 `Option`：「没有这个组件」
/// 与「正向」会读出同一个方向，用 `Option` 只会把漏挂组件变成一条永远不响的静默降级。
type HudPlayer<'w> = (
    &'w PlayerId,
    &'w PlayerName,
    &'w Wallet,
    &'w Points,
    Option<&'w Detention>,
    // 停留卡挂上的计时（P3d④c）。与拘留并排而不是合并：这两个标记在屏幕上说的是两件
    // 不同的事 —— 被拘留的人在死胡同格里，停留的人就站在自己踩到的那一格上
    Option<&'w StaySentence>,
    Option<&'w Bankrupt>,
    &'w Heading,
    // P5：载具、乌龟卡的夹步与道具总数。前两样是「这一回合怎么走」，道具数是
    // 「还能放几件」—— 三样都不改名字与钱包，但截图要能在 HUD 上一眼读到
    &'w Mount,
    Option<&'w MoveCap>,
    &'w Inventory,
    // P6e②：身上那位神。它与 `MoveCap` 同一族（都是「挂在人身上一段时间」的状态），但
    // 天数是**天**不是回合 —— 拘留期间照算，所以这一格读的是 core 那个 `days_left`，
    // 面板不许自己数回合
    Option<&'w Possession>,
    // P14e：这一位是电脑还是等人点，以及它是哪一档性格 —— `AiProfile.who` 的第二个
    // 上屏处（第一处是 dump）。HUD 上那行 `电脑·大佬奸` 是「现在轮到谁在想」的一半，
    // 另一半在顶栏那句旁白里（`top_bar::phase_hint`）
    &'w Controller,
);

/// 每帧从世界状态重算五段文案，只在真的变了才写文本（反复赋同一个字符串会让排版重来）。
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
        let Ok((
            pid,
            name,
            wallet,
            points,
            detained,
            stayed,
            bankrupt,
            heading,
            mount,
            cap,
            inventory,
            held,
            controller,
        )) = players.get(card.player)
        else {
            continue;
        };
        let props = owned
            .iter()
            .find(|(id, _)| *id == *pid)
            .map_or(0, |(_, count)| *count);
        // 文案与配色照 `player_hud.gd:191-199`；破产优先于拘留（同时成立时人已经出局）。
        // 三个去处共用一种颜色：它们在规则上是同一族（`col::JAILED` 如今是「不在场」的颜色），
        // 而「他在哪儿」由 `place.label()` 那一个词说清 —— 用三种颜色就是把三件事当三套状态
        let (status, status_color) = match (bankrupt, detained, stayed) {
            (Some(_), ..) => ("[破产]".to_string(), col::BANKRUPT),
            (_, Some(d), _) => (format!("[{}]", d.place.label()), col::JAILED),
            (_, _, Some(_)) => ("[停留]".to_string(), col::MUTED),
            (None, None, None) => (String::new(), col::BODY),
        };
        // 方向与处境共用右侧那一段，但它**不是**处境：正向是常态、什么都不写，
        // 只有逆向才占一个字 —— 一个永远亮着的标记等于没有标记。
        // P5 起这一段还读两个**不动位置**的状态：乌龟卡的夹步与载具 —— 前者解释
        // 「骰了 5 点为什么只走 1 格」，后者解释「为什么能选 2/3 颗骰」
        let mut flags = vec![status];
        // 神名放在这一族最前面：它是这一段里唯一「会自己变没」的东西（期满／被顶走／被送走），
        // 玩家回头翻截图时第一个要对齐的就是「那一回合我身上是谁」
        if let Some(held) = held {
            flags.push(format!("{}剩{}", held.god.label(), held.days_left));
        }
        if *heading == Heading::Backward {
            flags.push("逆向".to_string());
        }
        if let Some(cap) = cap {
            flags.push(format!("少动剩{}", cap.turns_left));
        }
        if *mount != Mount::Foot {
            flags.push(mount.label().to_string());
        }
        // 电脑座位照着自己那份先验动手：`who` 是角色卡上那一档性格的名字。
        // 人类座位什么都不写 —— 一个永远亮着的标记等于没有标记（同上面「逆向」那条）
        if let Controller::Ai(profile) = controller {
            flags.push(format!("电脑·{}", profile.who));
        }
        let status = flags
            .into_iter()
            .filter(|f| !f.is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        let cash = format!("${}", wallet.cash);
        let points = format!("点券:{}", points.0);
        let items: u32 = inventory.entries().iter().map(|(_, n)| *n).sum();
        let props_line = format!("地产:{props} 道具:{items}");

        let line = format!("{}|{cash}|{points}|{props_line}|{status}", name.0);
        if line != card.cached {
            card.cached = line;
            write(&mut texts, card.name, &name.0);
            write(&mut texts, card.cash, &cash);
            write(&mut texts, card.points, &points);
            write(&mut texts, card.props, &props_line);
            write(&mut texts, card.status, &status);
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
