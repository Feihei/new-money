//! 道具栏（P5）：身上的放置类道具、遥控骰子与载具颗数。
//!
//! 与手牌浮层同一类：**不阻断回合**、按签名重建、只在掷骰前可动手。差别在「目标」
//! 不烤在按钮里而是**在前方几格上**——道具是放到路上某个位置的，所以这块面板
//! 列的是「从你脚下往前 1..=6 格」，而不是名单。
//!
//! 候选格的灰按钮理由直接来自 `occupancy::place_denied`：面板与规则层读同一句话，
//! 所以「按钮亮着却被拒」不会发生。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, Heading, Inventory, ItemKind, Mount, MoveCap, Occupancy, OccupancyAt, Paused,
    PendingDecision, PlayerId, StaySentence, TileIndex, TokenPosition, TurnOrder, TurnPhase,
    place_denied,
};

use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, PRIMARY, SECONDARY, UiAction, border_color, button, can_play, label,
};
use crate::render::DevFont;

/// 道具浮层开着吗。顶栏那颗按钮的唯一记忆（与手牌同一条：留着不清的理由见
/// [`super::finance::AtmPanelOpen`]）。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ItemsPanelOpen(pub bool);

impl ItemsPanelOpen {
    /// `NM_ITEMS=1`：开面板，并由 `dev::force_items` 给当前玩家各发一个道具。
    pub(crate) fn from_env() -> Self {
        Self(std::env::var_os("NM_ITEMS").is_some())
    }
}

#[derive(Component)]
pub(crate) struct ItemsPanel;

/// 面板根上的「上次写成什么样」。少一位就是一次静默过期（§6.1 坑 30）：
/// 候选格、道具数、载具、停留计时，每一样都会改变可见内容。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct ItemsStamp(pub String);

/// 一名玩家在道具面板上要读的全部状态。别名是为了躲 clippy 的 `type_complexity`，
/// 与 `player_hud::HudPlayer` 同一条处理。
type ItemPlayer = (
    Entity,
    &'static PlayerId,
    &'static TokenPosition,
    &'static Heading,
    &'static Mount,
    &'static Inventory,
    Option<&'static StaySentence>,
    Option<&'static MoveCap>,
);

/// 面板要读的世界（打包理由同 `HandView`）。
#[derive(SystemParam)]
pub(crate) struct ItemsView<'w, 's> {
    open: Res<'w, ItemsPanelOpen>,
    paused: Res<'w, Paused>,
    pending: Res<'w, PendingDecision>,
    phase: Res<'w, State<TurnPhase>>,
    order: Option<Res<'w, TurnOrder>>,
    board: Option<Res<'w, ActiveBoard>>,
    players: Query<'w, 's, ItemPlayer>,
    occupancies: Query<'w, 's, &'static Occupancy>,
}

/// 一颗候选格按钮。「5 南京路」与它此刻能不能放。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TilePick {
    pub tile: TileIndex,
    pub text: String,
    pub blocked: Option<&'static str>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PlaceRow {
    item: ItemKind,
    count: u32,
    picks: Vec<TilePick>,
}

fn tile_text(board: &ActiveBoard, tile: TileIndex) -> String {
    let name = board
        .def
        .tile(tile.0 as i64)
        .map_or("？", |t| t.name.as_str());
    format!("{} {}", tile.0, name)
}

/// 道具浮层。内容分四段：放置类道具各一行候选格、遥控骰子、载具颗数、停留/乌龟的
/// 当前计时。没有道具时写一句「身上没有道具」，而不是留一块空面板。
pub(crate) fn sync_items_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    view: ItemsView<'_, '_>,
    shown: Query<(Entity, &ItemsPanel, &ItemsStamp)>,
) {
    if !view.open.0 {
        for (entity, _, _) in &shown {
            commands.entity(entity).despawn();
        }
        return;
    }
    let player = view.order.as_ref().and_then(|o| o.current());
    let playable = player.is_some() && can_play(*view.phase.get(), &view.paused, &view.pending);
    let row = player.and_then(|p| view.players.get(p).ok());
    let held = row.map(|r| r.5.clone()).unwrap_or_default();
    let mount = row.map(|r| *r.4).unwrap_or(Mount::Foot);
    let cap = row.and_then(|r| r.7.copied());
    let held_by_stay = row.is_some_and(|r| r.6.is_some());

    // 候选格：从脚下往前 1..=6。`BoardDef::walk_path` 与掷骰走同一条拓扑
    let mut rows: Vec<PlaceRow> = Vec::new();
    if let (Some(board), Some((_, _, pos, heading, ..))) = (view.board.as_deref(), row) {
        for item in ItemKind::ALL {
            if item.occupancy().is_none() {
                continue;
            }
            let count = held.count(item);
            if count == 0 {
                continue;
            }
            let mut picks = Vec::new();
            for tile in board.def.walk_path(pos.tile.0 as i64, heading.sign() * 6) {
                let occupied = view
                    .occupancies
                    .iter()
                    .any(|occ| occ.at == OccupancyAt::OnTile(tile));
                let somebody_there = view
                    .players
                    .iter()
                    .any(|(_, _, other, ..)| other.tile == tile);
                let blocked = place_denied(item, count, true, occupied, somebody_there)
                    .err()
                    .map(|why| why.label());
                picks.push(TilePick {
                    tile,
                    text: tile_text(board, tile),
                    blocked,
                });
            }
            rows.push(PlaceRow { item, count, picks });
        }
    }
    let remote = held.count(ItemKind::RemoteDice);
    // 每件放置类道具的候选格是**同一排**（都从脚下往前 6 格），所以灰的理由也只写一次。
    // 写进每一行会让三行各重复一遍，把面板顶到顶栏后面 —— 那正是冒烟第一张的形状
    let mut header = if playable {
        "掷骰前可放".to_string()
    } else {
        "只在掷骰前可放".to_string()
    };
    if let Some(first) = rows.first() {
        let why = reasons(&first.picks);
        if !why.is_empty() {
            header.push_str(" · ");
            header.push_str(&why);
        }
    }

    // 签名覆盖每一处可见位：道具数、候选格与它们的灰理由、遥控骰子、载具、两种计时
    let stamp = format!(
        "{}|{playable}|{held_by_stay}|{}|{}|{remote}|{}|{}",
        player.map_or_else(|| "-".into(), |p| format!("{p:?}")),
        ItemKind::ALL
            .iter()
            .map(|kind| held.count(*kind).to_string())
            .collect::<Vec<_>>()
            .join(","),
        rows.iter()
            .map(|r| format!(
                "{}:{}:{}",
                r.item.label(),
                r.count,
                r.picks
                    .iter()
                    .map(|p| format!("{}{}", p.text, p.blocked.unwrap_or("")))
                    .collect::<Vec<_>>()
                    .join("|")
            ))
            .collect::<Vec<_>>()
            .join(";"),
        mount.label(),
        cap.map_or_else(|| "-".into(), |c| c.turns_left.to_string()),
    );
    if shown.iter().next().is_some_and(|(_, _, s)| s.0 == stamp) {
        return;
    }
    for (entity, _, _) in shown.iter() {
        commands.entity(entity).despawn();
    }

    let mut root = commands.spawn((
        BoardUi,
        ItemsPanel,
        ItemsStamp(stamp),
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(layout::ITEMS_PANEL[1]),
            bottom: Val::Px(layout::ITEMS_PANEL[2]),
            width: Val::Px(layout::ITEMS_PANEL[0]),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(layout::ITEM_ROW_GAP),
            padding: UiRect::axes(Val::Px(16.0), Val::Px(14.0)),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        BackgroundColor(col::PANEL),
        border_color(col::BORDER_ITEM),
        ZIndex(4),
    ));
    root.with_children(|p| {
        p.spawn(label(&font.0, "道具", size::NAME, col::TITLE_GOLD));
        p.spawn(label(
            &font.0,
            header,
            size::ROW,
            if playable { col::MUTED } else { col::BANKRUPT },
        ));
        if rows.is_empty() && remote == 0 {
            p.spawn(label(&font.0, "身上没有道具", size::BODY, col::KEY));
        }
        for row in &rows {
            p.spawn(label(
                &font.0,
                format!("{} ×{}", row.item.label(), row.count),
                size::NAME,
                col::HINT,
            ));
            let mut grid = p.spawn(Node {
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(6.0),
                row_gap: Val::Px(4.0),
                ..default()
            });
            grid.with_children(|line| {
                for pick in &row.picks {
                    button(
                        line,
                        &font.0,
                        ButtonSpec {
                            font: size::SMALL,
                            ..ButtonSpec::new(
                                &pick.text,
                                layout::ITEM_TILE_BUTTON,
                                PRIMARY,
                                UiAction::PlaceItem {
                                    item: row.item,
                                    tile: pick.tile,
                                },
                            )
                        }
                        .disabled(!playable || pick.blocked.is_some()),
                    );
                }
            });
        }
        if remote > 0 {
            p.spawn(label(
                &font.0,
                format!("遥控骰子 ×{remote}"),
                size::NAME,
                col::HINT,
            ));
            // 被停留按住时掷骰都被拒，遥控骰子也一样 —— 规则层那一条不是只给按钮看的
            let blocked = if !playable {
                Some("掷骰前才能用")
            } else if held_by_stay {
                Some("被停留按住时不能走")
            } else {
                None
            };
            let mut grid = p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            });
            grid.with_children(|line| {
                for steps in 1..=6u32 {
                    button(
                        line,
                        &font.0,
                        ButtonSpec {
                            font: size::ROW,
                            ..ButtonSpec::new(
                                &steps.to_string(),
                                layout::ITEM_DICE_BUTTON,
                                PRIMARY,
                                UiAction::UseRemoteDice { steps },
                            )
                        }
                        .disabled(blocked.is_some()),
                    );
                }
            });
            if let Some(reason) = blocked {
                p.spawn(label(&font.0, reason, size::ROW, col::JAILED));
            }
        }
        // 载具那一行一直写：步行时它解释「为什么只有一颗骰」，有车时才长出颗数按钮
        p.spawn(label(
            &font.0,
            format!("载具：{}（最多 {} 颗骰）", mount.label(), mount.dice()),
            size::ROW,
            col::MUTED,
        ));
        if mount.dice() > 1 {
            let blocked = if !playable {
                Some("掷骰前才能选")
            } else if held_by_stay {
                Some("被停留按住时不能走")
            } else {
                None
            };
            let mut grid = p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            });
            grid.with_children(|line| {
                line.spawn(label(&font.0, "掷", size::ROW, col::KEY));
                for dice in 1..=mount.dice() {
                    button(
                        line,
                        &font.0,
                        ButtonSpec {
                            font: size::ROW,
                            ..ButtonSpec::new(
                                &format!("{dice} 颗"),
                                layout::ITEM_DICE_BUTTON,
                                PRIMARY,
                                UiAction::RollWith(dice),
                            )
                        }
                        .disabled(blocked.is_some()),
                    );
                }
            });
            if let Some(reason) = blocked {
                p.spawn(label(&font.0, reason, size::ROW, col::JAILED));
            }
        }
        if let Some(cap) = cap {
            p.spawn(label(
                &font.0,
                format!("乌龟卡：还剩 {} 回合，只走 1 格", cap.turns_left),
                size::ROW,
                col::JAILED,
            ));
        }
        p.spawn(super::separator());
        button(
            p,
            &font.0,
            ButtonSpec::new(
                "关闭",
                layout::PANEL_CLOSE_BUTTON,
                SECONDARY,
                UiAction::ToggleItems,
            ),
        );
    });
}

/// 候选格里所有灰按钮的理由，去重后拼成一句。没有灰的就不写。
///
/// 每颗按钮单独解释在 360 宽的面板里放不下；这里说的是「这一排里哪些理由在生效」，
/// 与手牌面板一行卡一颗理由是同一条取舍。
fn reasons(picks: &[TilePick]) -> String {
    // 按理由归组、只留格号：候选格的名字已经在按钮上，重复七遍会把这一行撑成三行，
    // 面板跟着长到顶栏后面（真鼠标冒烟第一张就是这个形状）
    let mut by_reason: std::collections::BTreeMap<&'static str, Vec<u32>> =
        std::collections::BTreeMap::new();
    for pick in picks {
        if let Some(why) = pick.blocked {
            by_reason.entry(why).or_default().push(pick.tile.0);
        }
    }
    if by_reason.is_empty() {
        return String::new();
    }
    let parts: Vec<String> = by_reason
        .into_iter()
        .map(|(why, mut tiles)| {
            tiles.sort_unstable();
            let nums = tiles
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join("、");
            format!("{nums}（{why}）")
        })
        .collect();
    format!("灰：{}", parts.join("；"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reason_line_names_each_blocking_cause_once() {
        let picks = vec![
            TilePick {
                tile: TileIndex(1),
                text: "1 起点".into(),
                blocked: Some("这一格上有人"),
            },
            TilePick {
                tile: TileIndex(2),
                text: "2 南京路".into(),
                blocked: Some("这一格已经有东西了"),
            },
            TilePick {
                tile: TileIndex(3),
                text: "3 外滩".into(),
                blocked: Some("这一格上有人"),
            },
            TilePick {
                tile: TileIndex(4),
                text: "4 银行".into(),
                blocked: None,
            },
        ];
        let line = reasons(&picks);
        // 每颗灰按钮说清「哪一格、为什么」：只说理由的话，三颗里哪一颗按不动还得猜
        assert_eq!(
            line, "灰：1、3（这一格上有人）；2（这一格已经有东西了）",
            "同一理由要归成一组，否则每个道具行都重复三遍"
        );
    }

    #[test]
    fn no_blocked_pick_means_no_line() {
        let picks = vec![TilePick {
            tile: TileIndex(1),
            text: "1 起点".into(),
            blocked: None,
        }];
        assert!(reasons(&picks).is_empty());
    }
}
