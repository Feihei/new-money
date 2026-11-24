//! 玩家一览：顶栏「玩家」那颗按钮后面的全座位账目。
//!
//! HUD 只留当前行动者（见 [`super::player_hud`]），这一屏是它的另一半：四个座位
//! 谁有多少钱、几块地、持了什么股、身上挂着什么状态，一张表里全摆出来。
//! **它不阻断回合**（与 ATM / 证券 / 手牌同一类）：翻开只是看账，掷骰与走格照旧，
//! 所以它也没有「办完」只有「关闭」。
//!
//! 与 dump 面板的分工：dump 是开发者的调试视图（含实体句柄、牌堆序号、地块层数），
//! 这一屏是给玩家看的同一份世界 —— 同样的数，少掉一切调试字段，多出「谁是当前行动者」。
//! 两处读的是同一批账本方法（[`game_core::prelude::Books`]），不各算各的。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    Books, Controller, Detention, DetentionPlace, Hand, Heading, Inventory, Market, Mount, MoveCap,
    PlayerColor, PlayerId, PlayerName, StaySentence, TokenPosition, TurnOrder,
};

use super::finance::comma;
use super::skin::Skins;
use super::theme::{col, layout, size};
use super::{BoardUi, ButtonSpec, SECONDARY, UiAction, border_color, button, label, separator};
use crate::render::DevFont;

/// 玩家一览开着吗。顶栏那颗按钮的唯一记忆（与手牌/道具同一条）。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PlayersPanelOpen(pub bool);

impl PlayersPanelOpen {
    /// `NM_PLAYERS=1`：开局即开。每个界面都要有一条不依赖鼠标的入口，
    /// 否则截图脚本抓不到它（同 `NM_STOCK` 那批）。
    pub(crate) fn from_env() -> Self {
        Self(std::env::var_os("NM_PLAYERS").is_some())
    }
}

#[derive(Component)]
pub(crate) struct PlayersPanel;

/// 面板根上的「上次写成什么样」。面板整块重建而不是逐段改写：四行玩家卡各五段文字，
/// 逐段缓存要维护二十来个节点，漏一段就是过期数字（与其余浮层同一条）。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayersStamp(pub String);

/// 一名玩家在这一屏上要读的全部组件。`Hand` / 两条计时用 `Option`：
/// 它们不是每个实体都有（手牌是 P3a 才挂的，停留与乌龟只在卡面生效期间存在）。
type RosterPlayer = (
    Entity,
    &'static PlayerId,
    &'static PlayerName,
    &'static PlayerColor,
    &'static TokenPosition,
    &'static Heading,
    &'static Mount,
    &'static Inventory,
    &'static Controller,
    Option<&'static Hand>,
    Option<&'static StaySentence>,
    Option<&'static MoveCap>,
);

/// 一行玩家卡的全部读数。拼成结构体是为了让「面板画什么」与「签名比什么」
/// 读同一份数据 —— 两处各拼一遍字符串就会出现「数字换了但面板没重建」。
struct RosterRow {
    id: u32,
    name: String,
    color: [f32; 3],
    current: bool,
    identity: String,
    place: String,
    money: String,
    holdings: String,
    flags: String,
}

impl RosterRow {
    /// 签名要覆盖每一处可见位：换人、身份、位置、两行账目、状态。
    fn stamp(&self) -> String {
        format!(
            "{}|{}|{}|{}|{}|{}|{}|{}",
            self.id,
            self.name,
            self.identity,
            self.place,
            self.money,
            self.holdings,
            self.flags,
            self.current
        )
    }
}

/// 状态那一行的**纯函数**：从几个布尔/可选读数拼出一句人话。
///
/// 破产是终局，压过其余一切（人已经出局，再写「在同一格上」没有意义）；
/// 其余几条互不排斥 —— 一个人可以同时被停留按住、身上带着神、又刚吃了乌龟卡。
/// 什么都没挂时写「正常」而不是空串：一排空白读起来像这一栏坏了。
struct FlagFacts<'a> {
    bankrupt: bool,
    detention: Option<&'a str>,
    stay: Option<u32>,
    cap: Option<u32>,
    mount: Mount,
    backward: bool,
    god: Option<(&'a str, u32)>,
}

fn flags_text(facts: &FlagFacts<'_>) -> String {
    if facts.bankrupt {
        return "破产".to_string();
    }
    let mut parts: Vec<String> = Vec::new();
    if let Some(phrase) = facts.detention {
        parts.push(phrase.to_string());
    }
    if let Some(turns) = facts.stay {
        parts.push(format!("停留剩{turns}回合"));
    }
    if let Some(turns) = facts.cap {
        parts.push(format!("少动剩{turns}"));
    }
    if facts.mount != Mount::Foot {
        parts.push(facts.mount.label().to_string());
    }
    if facts.backward {
        parts.push("逆向".to_string());
    }
    if let Some((god, days)) = facts.god {
        parts.push(format!("{god}剩{days}"));
    }
    if parts.is_empty() {
        "正常".to_string()
    } else {
        parts.join(" · ")
    }
}

/// 面板要读的 UI 侧那几样。规则层账本单独作为 [`Books`] 参数传进来 ——
/// 它是本仓库里唯一同时握着钱包/存款/持股/地块写侧的类型，混进参数包会让
/// 「这一屏为什么需要可变借账本」这件事在签名上消失。
#[derive(SystemParam)]
pub(crate) struct PlayersView<'w, 's> {
    open: Res<'w, PlayersPanelOpen>,
    order: Option<Res<'w, TurnOrder>>,
    /// 现价读侧归行情模块（`Market::price`）：与证券浮层读同一份，
    /// 不在这里自己扫 `StockPrice` 实体 —— 那会是同一个问题的第二种问法
    market: Market<'w, 's>,
    players: Query<'w, 's, RosterPlayer>,
}

/// 全部座位的一览。打开时按世界状态重建；关掉时把根实体收走。
pub(crate) fn sync_players_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    skins: Res<Skins>,
    books: Books<'_, '_>,
    view: PlayersView<'_, '_>,
    shown: Query<(Entity, &PlayersPanel, &PlayersStamp)>,
) {
    if !view.open.0 {
        for (entity, _, _) in &shown {
            commands.entity(entity).despawn();
        }
        return;
    }

    let current = view.order.as_ref().and_then(|order| order.current());
    // 拘留名单一次算好：`Books::held` 按去处筛，四个去处各扫一遍人（≤4 位，成本看不见）。
    // 不并进 `players` 查询：`Detention` 的写侧在账本里，同一 system 再开一条读查询会在
    // 参数建立期撞成借用冲突（§6.1 坑 33）。
    let mut detained: Vec<(Entity, Detention)> = Vec::new();
    for place in [
        DetentionPlace::Jail,
        DetentionPlace::Hospital,
        DetentionPlace::Away,
        DetentionPlace::Hotel,
    ] {
        detained.extend(books.held(place));
    }

    let mut rows: Vec<RosterRow> = view
        .players
        .iter()
        .map(
            |(entity, id, name, color, position, heading, mount, inventory, controller, hand, stay, cap)| {
                let balance = books.bank_of(entity);
                let held = detained
                    .iter()
                    .find(|(who, _)| *who == entity)
                    .map(|(_, d)| *d);
                let god = books.god_of(entity);
                let land_count = books.land_count_of(entity);
                let land_value = books.land_value_of(entity);
                let holdings = books.stock_holdings_of(entity);
                let stock_value: i64 = holdings
                    .iter()
                    .map(|(id, shares)| {
                        (view.market.price(id).unwrap_or(0.0) * *shares as f32) as i64
                    })
                    .sum();
                let items: u32 = inventory.entries().iter().map(|(_, n)| *n).sum();
                let cards = hand.map_or(0, |h| h.len());
                let cash = books.balance(entity);
                let points = books.points_of(entity);
                let net =
                    cash + balance.deposit - balance.loan - balance.financing + land_value + stock_value;
                let tile = books.active.def.tile(position.tile.0 as i64);
                let place = format!(
                    "{} {}",
                    position.tile.0,
                    tile.map_or("？", |t| t.name.as_str())
                );
                let identity = match controller {
                    Controller::Ai(profile) => format!("电脑·{}", profile.who.label()),
                    Controller::Human => "真人".to_string(),
                };
                let detention = held.map(|d| super::dialog::detention_phrase(&d));
                let flags = flags_text(&FlagFacts {
                    bankrupt: books.is_bankrupt(entity),
                    detention: detention.as_deref(),
                    stay: stay.map(|s| s.turns_left),
                    cap: cap.map(|c| c.turns_left),
                    mount: *mount,
                    backward: *heading == Heading::Backward,
                    god: god.map(|p| (p.god.label(), p.days_left)),
                });
                let mut money = format!(
                    "现金 ${} · 点券 {} · 存款 ${} · 贷款 ${}",
                    comma(cash),
                    points,
                    comma(balance.deposit),
                    comma(balance.loan)
                );
                if balance.financing > 0 {
                    money.push_str(&format!(" · 融资债 ${}", comma(balance.financing)));
                }
                money.push_str(&format!(" · 净值 ${}", comma(net)));
                RosterRow {
                    id: id.0,
                    name: name.0.clone(),
                    color: color.0,
                    current: current == Some(entity),
                    identity,
                    place,
                    money,
                    holdings: format!(
                        "地产 {land_count} 块（${}）· 持股 {} 种（${}）· 手牌 {cards} 张 · 道具 {items} 件",
                        comma(land_value),
                        holdings.len(),
                        comma(stock_value)
                    ),
                    flags,
                }
            },
        )
        .collect();
    rows.sort_by_key(|row| row.id);

    let stamp = rows
        .iter()
        .map(RosterRow::stamp)
        .collect::<Vec<_>>()
        .join(";");
    if shown.iter().next().is_some_and(|(_, _, s)| s.0 == stamp) {
        return;
    }
    for (entity, _, _) in &shown {
        commands.entity(entity).despawn();
    }

    let paper = skins.float.image.is_some();
    let title_color = if paper { col::INK } else { col::TITLE_GOLD };
    let pad = super::skin::padding(&skins.float, 18.0, 14.0, layout::PLAYERS_PANEL[0]);
    let mut root = commands.spawn((
        BoardUi,
        PlayersPanel,
        PlayersStamp(stamp),
        // 根节点要避开命中测试：它横在棋盘中央，不挂 IGNORE 的话整块面板会
        // 把背后的点格点击吃掉（§6.1 坑 27 的 UI 同族，顶栏那一坑）
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Percent(50.0),
            top: Val::Px(layout::PLAYERS_PANEL[1]),
            width: Val::Px(layout::PLAYERS_PANEL[0]),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: pad,
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        UiTransform::from_translation(Val2::new(
            Val::Px(-layout::PLAYERS_PANEL[0] / 2.0),
            Val::Px(0.0),
        )),
        BackgroundColor(col::PANEL),
        border_color(col::BORDER_HAND),
        ZIndex(4),
    ));
    // 底板皮肤：盘上有 panel_float 就叠九宫格底图，没有保持纯色面板（同其余浮层）
    if let Some((skin_node, skin_image)) = super::skin::panel_image(&skins.float) {
        root.with_children(|p| {
            p.spawn((skin_node, skin_image));
        });
    }
    root.with_children(|p| {
        p.spawn((
            Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            TextLayout::justify(Justify::Center),
            label(&font.0, "玩家一览", size::NAME, title_color),
        ));
        p.spawn(separator());
        for row in &rows {
            let edge = if row.current {
                col::HUD_BORDER_ACTIVE
            } else {
                Color::NONE
            };
            p.spawn((
                Node {
                    width: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    padding: UiRect::axes(Val::Px(10.0), Val::Px(6.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(if row.current {
                    col::HUD_CARD_ACTIVE
                } else {
                    col::HUD_CARD
                }),
                border_color(edge),
            ))
            .with_children(|card| {
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|head| {
                    head.spawn((
                        Node {
                            width: Val::Px(layout::HUD_DOT),
                            height: Val::Px(layout::HUD_DOT),
                            border_radius: BorderRadius::all(Val::Px(layout::HUD_DOT / 2.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgb(row.color[0], row.color[1], row.color[2])),
                    ));
                    head.spawn(label(&font.0, row.name.clone(), size::NAME, col::HUD_NAME));
                    head.spawn(label(
                        &font.0,
                        row.identity.clone(),
                        size::SMALL,
                        col::MUTED,
                    ));
                    head.spawn((
                        Node {
                            flex_grow: 1.0,
                            justify_content: JustifyContent::FlexEnd,
                            ..default()
                        },
                        label(
                            &font.0,
                            if row.current { "◀ 当前行动" } else { "" },
                            size::SMALL,
                            col::TITLE_GOLD,
                        ),
                    ));
                });
                card.spawn(label(&font.0, row.money.clone(), size::ROW, col::BODY));
                card.spawn(label(&font.0, row.holdings.clone(), size::ROW, col::BODY));
                card.spawn(label(
                    &font.0,
                    format!("位置 {}", row.place),
                    size::SMALL,
                    col::MUTED,
                ));
                let (flag_color, flag_text) = match row.flags.as_str() {
                    "正常" => (col::HUD_PROPS, row.flags.clone()),
                    "破产" => (col::BANKRUPT, row.flags.clone()),
                    _ => (col::JAILED, row.flags.clone()),
                };
                card.spawn(label(
                    &font.0,
                    format!("状态 {flag_text}"),
                    size::SMALL,
                    flag_color,
                ));
            });
        }
        button(
            p,
            &font.0,
            ButtonSpec::new(
                "关闭",
                layout::PANEL_CLOSE_BUTTON,
                SECONDARY,
                UiAction::TogglePlayers,
            ),
        );
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 破产压过其余一切：人已经出局，再写「在同一格上」没有意义。
    /// 反过来，没破产时几条互不排斥的状态要**全写出来** —— 漏一条就是
    /// 「他明明被停留按着，面板却说正常」。
    #[test]
    fn the_status_line_keeps_everything_that_is_true_at_once() {
        let normal = flags_text(&FlagFacts {
            bankrupt: false,
            detention: None,
            stay: None,
            cap: None,
            mount: Mount::Foot,
            backward: false,
            god: None,
        });
        assert_eq!(normal, "正常");

        let crowded = flags_text(&FlagFacts {
            bankrupt: false,
            detention: Some("住院剩2回合"),
            stay: Some(1),
            cap: Some(3),
            mount: Mount::Car,
            backward: true,
            god: Some(("财神", 4)),
        });
        for expected in [
            "住院剩2回合",
            "停留剩1回合",
            "少动剩3",
            "汽车",
            "逆向",
            "财神剩4",
        ] {
            assert!(
                crowded.contains(expected),
                "状态那一行漏了 {expected}：{crowded}"
            );
        }

        let broke = flags_text(&FlagFacts {
            bankrupt: true,
            detention: Some("监狱·等保释"),
            stay: Some(2),
            cap: None,
            mount: Mount::Bike,
            backward: true,
            god: Some(("穷神", 1)),
        });
        assert_eq!(broke, "破产", "破产之后还念别的状态，读起来像人还在场上");
    }

    /// 签名必须覆盖每一处可见位。少一位就是「数字变了，面板还停在旧数上」——
    /// 这一屏只有整块重建一条路，签名就是它的全部正确性。
    #[test]
    fn the_stamp_moves_whenever_a_visible_field_moves() {
        let base = RosterRow {
            id: 0,
            name: "阿土伯".into(),
            color: [1.0, 0.0, 0.0],
            current: false,
            identity: "真人".into(),
            place: "3 南京路".into(),
            money: "现金 $20000".into(),
            holdings: "地产 0 块".into(),
            flags: "正常".into(),
        };
        let stamp = base.stamp();
        let with_cash = RosterRow {
            money: "现金 $19000".into(),
            ..base
        };
        let with_cash_stamp = with_cash.stamp();
        assert_ne!(stamp, with_cash_stamp);

        let now_current = RosterRow {
            current: true,
            ..with_cash
        };
        assert_ne!(now_current.stamp(), with_cash_stamp);
    }
}
