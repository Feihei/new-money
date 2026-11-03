//! 对局设置页（P2d）：「新游戏」先到这里选地图、角色与起始资金。
//!
//! 三块内容一屏：**地图**（今天只有台湾一张真盘，另外两张灰着写「待做」）、
//! **角色**（首发十二人，点卡加入/移出；第一席是你，其余是电脑）、**起始资金**（四档，
//! 与 `NM_CASH` 同一条写入口）。地图现在没有可切换的第二张，所以它只是一块只读的
//! 「这一局在哪张盘上」；等第二张真盘落地，卡片上接一个动作即可，`DataHandles`
//! 与存档里的 `board` 字段已经按「一场一张盘」准备好了。
//!
//! 角色的真相在 `assets/data/characters.json`（core 的 `CharacterSet`）：名字、颜色、性别、
//! AI 先验都从表里来，**不从索引来** —— 否则同一角色换个座位就换一种颜色。
//! 选好的阵容在「开始对局」那一刻才写成 `GameSetup`；`BeginCfg` 只认这一份。
//!
//! 面板按签名重建（同设置面板）：开关、阵容、当前资金档、角色表是否加载完，四位都是可见位。

use std::collections::BTreeSet;

use bevy::{ecs::system::SystemParam, prelude::*};
use game_core::prelude::{
    BoardDef, CharacterDef, CharacterSet, Controller, DataHandles, GameSetup, START_CASH_TIERS,
    StartCashChoice,
};

use crate::boards::{BoardLibrary, BoardPick};

use super::theme::{col, layout, size};
use super::{
    ButtonSpec, Disabled, MenuPiece, PRIMARY, Palette, SECONDARY, UiAction, border_color, button,
    label, separator,
};
use crate::render::DevFont;

/// 一局最多四个人（4 代的上限，也是 `PLAYER_COLORS` 那四格的原意）。
const MAX_PLAYERS: usize = 4;
/// 至少两位电脑：一个人开局就终局（`turn.rs` 那句「可行动的人只剩一个」直接判胜）。
const MIN_COMPUTERS: usize = 2;

/// 面板开着吗。菜单那颗「新游戏」开它，开始一局或回菜单时关掉。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct MatchSetupOpen(pub bool);

impl MatchSetupOpen {
    /// `NM_SETUP=1` 开局即开：面板不等鼠标也能截到（判据同 `NM_PAUSE`/`NM_STOCK`）。
    pub(crate) fn from_env() -> Self {
        Self(std::env::var_os("NM_SETUP").is_some())
    }
}

/// 这一局的阵容选择。存的是**角色 id** 而不是表里的下标 —— 改一次 JSON 行序
/// 不该把玩家上次选的人换掉。默认阵容＝P14 的 `GameSetup::ai_field()` 那四位。
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub(crate) struct RosterChoice {
    pub human: String,
    pub computers: Vec<String>,
}

impl Default for RosterChoice {
    fn default() -> Self {
        Self {
            human: "ji-bainian".into(),
            computers: vec!["lai-xi".into(), "fan-yiwen".into(), "duoji".into()],
        }
    }
}

/// 一位角色在这张阵容里的身份。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Human,
    Computer(usize),
}

impl RosterChoice {
    fn total(&self) -> usize {
        1 + self.computers.len()
    }

    fn contains(&self, id: &str) -> bool {
        self.human == id || self.computers.iter().any(|c| c == id)
    }

    fn role_of(&self, id: &str) -> Option<Role> {
        if self.human == id {
            return Some(Role::Human);
        }
        self.computers
            .iter()
            .position(|c| c == id)
            .map(Role::Computer)
    }

    /// 点一下一张角色卡：已选的移出、未选的加入。
    ///
    /// 三条拒绝都在这里，界面只负责把不能点的卡画灰：
    /// 第一席不能移出（必须有一位真人）、电脑不能少于 [`MIN_COMPUTERS`] 位、
    /// 总人数不能超过 [`MAX_PLAYERS`]。
    pub(crate) fn toggle(&mut self, id: &str) {
        if id == self.human {
            return;
        }
        if let Some(pos) = self.computers.iter().position(|c| c == id) {
            if self.computers.len() > MIN_COMPUTERS {
                self.computers.remove(pos);
            }
            return;
        }
        if self.total() < MAX_PLAYERS {
            self.computers.push(id.to_string());
        }
    }

    /// 把「你」挪到这张卡上。原来那位退成电脑，座位顺序不变。
    pub(crate) fn promote(&mut self, id: &str) {
        if id == self.human {
            return;
        }
        if let Some(pos) = self.computers.iter().position(|c| c == id) {
            self.computers[pos] = self.human.clone();
            self.human = id.to_string();
            return;
        }
        if self.total() < MAX_PLAYERS {
            self.computers
                .push(std::mem::replace(&mut self.human, id.to_string()));
        }
    }

    fn can_remove(&self, id: &str) -> bool {
        id != self.human
            && self.computers.iter().any(|c| c == id)
            && self.computers.len() > MIN_COMPUTERS
    }

    fn can_add(&self, id: &str) -> bool {
        !self.contains(id) && self.total() < MAX_PLAYERS
    }

    /// 阵容 → 开局配置。表里查不到任何一位就返回 `None`（把「角色表坏了」挡在开局之外，
    /// 而不是静默少一个人）。
    pub(crate) fn to_setup(&self, set: &CharacterSet) -> Option<GameSetup> {
        let human = set.get(&self.human)?;
        let mut players = vec![human.setup(Controller::Human)];
        for id in &self.computers {
            let def = set.get(id)?;
            players.push(def.setup(Controller::Ai(def.ai)));
        }
        Some(GameSetup { players })
    }
}

#[derive(Component)]
pub(crate) struct MatchSetupPanel;

#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchSetupStamp(pub String);

/// 面板要读的五样只读事实。打包是房规（§六 坑 8）：`sync_*` 的参数一多就撞
/// `too_many_arguments`，而它们说的其实是同一件事 ——「画这一屏要看什么」。
#[derive(SystemParam)]
pub(crate) struct SetupFacts<'w> {
    pub roster: Res<'w, RosterChoice>,
    pub choice: Res<'w, StartCashChoice>,
    pub library: Res<'w, BoardLibrary>,
    pub pick: Res<'w, BoardPick>,
    pub data: Res<'w, DataHandles>,
    pub boards: Res<'w, Assets<BoardDef>>,
    pub characters: Res<'w, Assets<CharacterSet>>,
}

pub(crate) fn sync_match_setup_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    open: Res<MatchSetupOpen>,
    facts: SetupFacts,
    shown: Query<(Entity, &MatchSetupStamp)>,
) {
    let selected = facts.library.entries.get(facts.pick.0);
    let board = selected.and_then(|entry| facts.boards.get(&entry.handle));
    let set = facts.characters.get(&facts.data.characters);
    let default = board.map(|def| def.start_cash);
    let current = facts.choice.0.or(default);
    let maps = facts
        .library
        .entries
        .iter()
        .enumerate()
        .map(|(index, entry)| MapView {
            index,
            label: entry.label.to_string(),
            line: facts
                .boards
                .get(&entry.handle)
                .map(map_line)
                .unwrap_or_else(|| "加载中…".into()),
            selected: index == facts.pick.0,
        })
        .collect::<Vec<_>>();
    let map_stamp = maps
        .iter()
        .map(|m| format!("{}={}", m.label, m.line))
        .collect::<Vec<_>>()
        .join(",");
    let stamp = format!(
        "{}|{}|{}|{}|{}|{}|{}|{}",
        open.0,
        facts.roster.human,
        facts.roster.computers.join(","),
        facts.pick.0,
        current.unwrap_or(-1),
        default.unwrap_or(-1),
        map_stamp,
        set.map_or(0, |s| s.0.len())
    );
    if open.0 && shown.iter().any(|(_, seen)| seen.0 == stamp) {
        return;
    }
    for (entity, _) in &shown {
        commands.entity(entity).despawn();
    }
    if open.0 {
        spawn_panel(
            &mut commands,
            &font,
            PanelView {
                roster: &facts.roster,
                picked: facts.choice.0,
                board_cash: default,
                maps,
                set,
                stamp,
            },
        );
    }
}

/// 地图卡上那一行：全部从 `BoardDef` 现读，不抄一份数进 UI。
fn map_line(def: &BoardDef) -> String {
    let streets: BTreeSet<&str> = def
        .lands
        .iter()
        .filter_map(|land| land.group.as_deref())
        .collect();
    format!(
        "{} 格 · {} 块地 · {} 条街 · 默认 {} 万",
        def.tiles.len(),
        def.lands.len(),
        streets.len(),
        def.start_cash / 10_000
    )
}

fn map_card(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, width: f32, map: &MapView) {
    let bg = if map.selected {
        col::HUD_CARD_ACTIVE
    } else {
        col::HUD_CARD
    };
    parent
        .spawn((
            Node {
                width: Val::Px(width),
                height: Val::Px(72.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(4.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(if map.selected {
                    layout::PANEL_BORDER
                } else {
                    layout::BTN_BORDER
                })),
                border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                ..default()
            },
            BackgroundColor(bg),
            border_color(if map.selected {
                col::BORDER_BUY
            } else {
                col::BTN_BORDER
            }),
            UiAction::SetBoard(map.index),
            Palette {
                normal: bg,
                hover: col::BTN_SECONDARY_HOVER,
            },
        ))
        .with_children(|card| {
            let title = if map.selected {
                format!("● {}", map.label)
            } else {
                map.label.clone()
            };
            card.spawn(label(
                font,
                title,
                size::BODY,
                if map.selected {
                    col::TITLE_GOLD
                } else {
                    col::HINT
                },
            ));
            card.spawn(label(font, &map.line, size::SMALL, col::MUTED));
        });
}

fn pending_map_card(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    width: f32,
    title: &str,
    line: &str,
) {
    parent
        .spawn((
            Node {
                width: Val::Px(width),
                height: Val::Px(72.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                row_gap: Val::Px(4.0),
                padding: UiRect::axes(Val::Px(12.0), Val::Px(8.0)),
                border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                ..default()
            },
            BackgroundColor(col::BTN_DISABLED),
            border_color(col::BTN_BORDER_DISABLED),
            Disabled,
        ))
        .with_children(|card| {
            card.spawn(label(font, title, size::BODY, col::BTN_TEXT_DISABLED));
            card.spawn(label(font, line, size::SMALL, col::BTN_TEXT_DISABLED));
        });
}

fn character_card(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    def: &CharacterDef,
    role: Option<Role>,
    clickable: bool,
    width: f32,
) {
    let selected = role.is_some();
    let bg = if !clickable && !selected {
        col::BTN_DISABLED
    } else if selected {
        col::HUD_CARD_ACTIVE
    } else {
        col::HUD_CARD
    };
    let mut card = parent.spawn((
        Node {
            width: Val::Px(width),
            height: Val::Px(88.0),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(2.0),
            padding: UiRect::axes(Val::Px(8.0), Val::Px(6.0)),
            border: UiRect::all(Val::Px(if selected {
                layout::PANEL_BORDER
            } else {
                layout::BTN_BORDER
            })),
            border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
            ..default()
        },
        BackgroundColor(bg),
        border_color(if selected {
            col::BORDER_BUY
        } else if clickable {
            col::BTN_BORDER
        } else {
            col::BTN_BORDER_DISABLED
        }),
        UiAction::ToggleCharacter(def.id.clone()),
        Palette {
            normal: bg,
            hover: col::BTN_SECONDARY_HOVER,
        },
    ));
    if !clickable {
        card.insert(Disabled);
    }
    card.with_children(|card| {
        card.spawn((Node {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            ..default()
        },))
            .with_children(|line| {
                let [r, g, b] = def.color;
                line.spawn((
                    Node {
                        width: Val::Px(14.0),
                        height: Val::Px(14.0),
                        border_radius: BorderRadius::all(Val::Px(3.0)),
                        ..default()
                    },
                    BackgroundColor(Color::srgb(
                        r as f32 / 255.0,
                        g as f32 / 255.0,
                        b as f32 / 255.0,
                    )),
                ));
                line.spawn(label(
                    font,
                    &def.name,
                    size::BODY,
                    if clickable || selected {
                        col::HINT
                    } else {
                        col::BTN_TEXT_DISABLED
                    },
                ));
            });
        card.spawn(label(
            font,
            format!("{} · {}", def.tier.label(), def.color_name),
            size::SMALL,
            if clickable || selected {
                col::MUTED
            } else {
                col::BTN_TEXT_DISABLED
            },
        ));
        match role {
            Some(Role::Human) => {
                card.spawn(label(font, "你", size::SMALL, col::TITLE_GOLD));
            }
            Some(Role::Computer(index)) => {
                card.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::SpaceBetween,
                    ..default()
                })
                .with_children(|line| {
                    line.spawn(label(
                        font,
                        format!("电脑 {}", index + 1),
                        size::SMALL,
                        col::GAIN,
                    ));
                    let mut btn = line.spawn((
                        Node {
                            width: Val::Px(52.0),
                            height: Val::Px(20.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                            border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                            ..default()
                        },
                        BackgroundColor(col::BTN_SECONDARY),
                        border_color(col::BTN_BORDER),
                        UiAction::SetHuman(def.id.clone()),
                        Palette {
                            normal: col::BTN_SECONDARY,
                            hover: col::BTN_SECONDARY_HOVER,
                        },
                    ));
                    btn.with_children(|b| {
                        b.spawn(label(font, "设为你", size::SMALL, Color::WHITE));
                    });
                });
            }
            None => {
                card.spawn(label(font, "", size::SMALL, col::MUTED));
            }
        }
    });
}

/// 一张地图卡的全部可见位。统计数字在 `sync` 里从 `BoardDef` 现读。
struct MapView {
    index: usize,
    label: String,
    line: String,
    selected: bool,
}

/// 一次重建要用的全部读数（`spawn_panel` 的参数超过七格就撞 `too_many_arguments`）。
struct PanelView<'a> {
    roster: &'a RosterChoice,
    picked: Option<i64>,
    board_cash: Option<i64>,
    maps: Vec<MapView>,
    set: Option<&'a CharacterSet>,
    stamp: String,
}

fn spawn_panel(commands: &mut Commands, font: &DevFont, view: PanelView) {
    let PanelView {
        roster,
        picked,
        board_cash,
        maps,
        set,
        stamp,
    } = view;
    // 全屏场景，不再是模态框：内容宽 1184，四周留 32/20 的内边距
    let width = 1184.0;
    let inner = width;
    // 可选盘 + 一张「世界」灰位；将来加第三张真盘这里自动跟着变窄
    let map_count = (maps.len().max(1) + 1) as f32;
    let map_width = (inner - (map_count - 1.0) * 8.0) / map_count;
    let card_width = (inner - 5.0 * 8.0) / 6.0;
    let tier_width = (inner - 3.0 * 8.0) / 4.0;
    let current = picked.or(board_cash);
    let ready = set.and_then(|s| roster.to_setup(s)).is_some();
    commands
        .spawn((
            MenuPiece,
            MatchSetupPanel,
            MatchSetupStamp(stamp),
            // 全屏场景，不是模态框：不铺遮罩、不画面板边框，内容直接铺在深色底上
            Node {
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                position_type: PositionType::Absolute,
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(32.0), Val::Px(20.0)),
                ..default()
            },
            BackgroundColor(col::MENU_BG),
            GlobalZIndex(20),
        ))
        .with_children(|screen| {
            screen
                .spawn(Node {
                    width: Val::Px(width),
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Stretch,
                    justify_content: JustifyContent::SpaceBetween,
                    row_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|panel| {
                // 标题居中
                panel
                    .spawn(Node {
                        width: Val::Percent(100.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    })
                    .with_children(|row| {
                        row.spawn(label(&font.0, "新对局", size::DIALOG_TITLE, col::TITLE_GOLD));
                    });
                panel.spawn(separator());

                // ---- 地图 ----
                panel.spawn(label(&font.0, "地图", size::BODY, col::HINT));
                panel
                    .spawn(Node {
                        width: Val::Px(inner),
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        if maps.is_empty() {
                            pending_map_card(row, &font.0, map_width, "棋盘加载中…", "");
                        }
                        for map in &maps {
                            map_card(row, &font.0, map_width, map);
                        }
                        pending_map_card(row, &font.0, map_width, "世界", "待做");
                    });

                // ---- 角色 ----
                panel
                    .spawn(Node {
                        width: Val::Px(inner),
                        justify_content: JustifyContent::SpaceBetween,
                        align_items: AlignItems::Center,
                        ..default()
                    })
                    .with_children(|line| {
                        line.spawn(label(&font.0, "角色", size::BODY, col::HINT));
                        line.spawn(label(
                            &font.0,
                            format!(
                                "已选 {}/{} · {} 位电脑",
                                roster.total(),
                                MAX_PLAYERS,
                                roster.computers.len()
                            ),
                            size::SMALL,
                            col::MUTED,
                        ));
                    });
                match set {
                    Some(set) => {
                        panel
                            .spawn(Node {
                                width: Val::Px(inner),
                                flex_direction: FlexDirection::Row,
                                flex_wrap: FlexWrap::Wrap,
                                column_gap: Val::Px(8.0),
                                row_gap: Val::Px(8.0),
                                ..default()
                            })
                            .with_children(|grid| {
                                for def in &set.0 {
                                    let role = roster.role_of(&def.id);
                                    let clickable = match role {
                                        None => roster.can_add(&def.id),
                                        Some(Role::Human) => false,
                                        Some(Role::Computer(_)) => roster.can_remove(&def.id),
                                    };
                                    character_card(grid, &font.0, def, role, clickable, card_width);
                                }
                            });
                        panel.spawn(label(
                            &font.0,
                            "点卡加入或移出 · 已选中的电脑卡上那颗「设为你」换真人 · 至少要两位电脑",
                            size::SMALL,
                            col::MUTED,
                        ));
                    }
                    None => {
                        panel.spawn(label(
                            &font.0,
                            "角色表加载中…",
                            size::BODY,
                            col::MUTED,
                        ));
                    }
                }

                // ---- 起始资金 ----
                panel.spawn(label(
                    &font.0,
                    "起始资金（每人）",
                    size::BODY,
                    col::HINT,
                ));
                panel
                    .spawn(Node {
                        width: Val::Px(inner),
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(8.0),
                        ..default()
                    })
                    .with_children(|row| {
                        for (index, cash) in START_CASH_TIERS.iter().enumerate() {
                            let on = current == Some(*cash);
                            let text = if on {
                                format!("● {} 万", cash / 10_000)
                            } else {
                                format!("{} 万", cash / 10_000)
                            };
                            button(
                                row,
                                &font.0,
                                ButtonSpec {
                                    font: size::ROW,
                                    ..ButtonSpec::new(
                                        &text,
                                        [tier_width, 38.0],
                                        SECONDARY,
                                        UiAction::SetStartCash(index),
                                    )
                                },
                            );
                        }
                    });
                let note = match board_cash {
                    Some(cash) if picked.is_none() => {
                        format!("棋盘默认档 {} —— 不动它就用这一档（四档覆盖角色表里的起始现金）", cash_label(cash))
                    }
                    Some(cash) => format!(
                        "棋盘默认档 {} · 地价与租金不随档位变 · 四档覆盖角色表里的起始现金",
                        cash_label(cash)
                    ),
                    None => "棋盘默认档由棋盘数据给".to_string(),
                };
                panel.spawn(label(&font.0, note, size::SMALL, col::MUTED));

                panel.spawn(separator());
                let start_reason = if set.is_none() {
                    Some("角色表还没加载完")
                } else if !ready {
                    Some("阵容里有角色不在角色表里")
                } else {
                    None
                };
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new("开始对局", [inner, 42.0], PRIMARY, UiAction::StartMatch)
                        .disabled(start_reason.is_some()),
                );
                if let Some(reason) = start_reason {
                    panel.spawn(label(&font.0, reason, size::SMALL, col::KEY));
                }
                button(
                    panel,
                    &font.0,
                    ButtonSpec::new("返回", [inner, 36.0], SECONDARY, UiAction::ToggleMatchSetup),
                );
            });
        });
}

fn cash_label(cash: i64) -> String {
    format!("{} 万", cash / 10_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> CharacterSet {
        CharacterSet::from_json(include_str!("../../../../assets/data/characters.json"))
            .expect("assets/data/characters.json 解析失败")
    }

    /// 默认阵容就是 P14 那四位；一条补充/一条移出/一条换人都从它出发。
    #[test]
    fn the_default_roster_is_one_human_and_three_computers() {
        let roster = RosterChoice::default();
        assert_eq!(roster.total(), 4);
        assert_eq!(roster.role_of("ji-bainian"), Some(Role::Human));
        assert_eq!(roster.role_of("lai-xi"), Some(Role::Computer(0)));
        assert_eq!(roster.role_of("duoji"), Some(Role::Computer(2)));
        assert_eq!(roster.role_of("lu-renjia"), None);
    }

    /// 三条拒绝各一格：第一席移不掉、电脑不能少于两位、满员不能再加。
    #[test]
    fn the_roster_refuses_to_lose_the_human_or_go_below_two_computers() {
        let mut roster = RosterChoice::default();
        roster.toggle("ji-bainian");
        assert_eq!(
            roster.role_of("ji-bainian"),
            Some(Role::Human),
            "第一席被移出了"
        );

        roster.toggle("lai-xi");
        assert_eq!(roster.total(), 3, "先移掉一位电脑");
        roster.toggle("fan-yiwen");
        assert_eq!(
            roster.total(),
            3,
            "电脑只剩两位时还能再移 —— 一个人开局会立刻终局"
        );

        roster.toggle("lu-renjia");
        assert_eq!(roster.total(), 4, "还有空位时加人应当成功");
        roster.toggle("ta-luo");
        assert_eq!(roster.total(), 4, "满四位");
        roster.toggle("he-lanshan");
        assert_eq!(roster.total(), 4, "满员还能再加人");
        assert_eq!(roster.role_of("he-lanshan"), None);
    }

    /// 「设为你」：已选的电脑原地换位，未选的（还有空位时）把人挤到电脑席。
    #[test]
    fn promoting_swaps_the_human_seat_without_changing_the_seat_count() {
        let mut roster = RosterChoice::default();
        roster.promote("lai-xi");
        assert_eq!(roster.human, "lai-xi");
        assert_eq!(
            roster.role_of("ji-bainian"),
            Some(Role::Computer(0)),
            "原来那位应当原地退成电脑，座位顺序不变"
        );
        assert_eq!(roster.total(), 4);

        roster.toggle("duoji");
        assert_eq!(roster.total(), 3);
        roster.promote("ta-luo");
        assert_eq!(roster.human, "ta-luo");
        assert_eq!(roster.total(), 4, "空位上的角色当人时要顶满四位");
        assert!(roster.computers.contains(&"lai-xi".to_string()));
    }

    /// 阵容翻成 `GameSetup`：名字/颜色/性别/AI 都从角色表来，第一位永远是真人。
    #[test]
    fn the_roster_turns_into_a_setup_with_the_tables_names_and_colors() {
        let set = shipped();
        let roster = RosterChoice::default();
        let setup = roster.to_setup(&set).expect("默认阵容都在表里");
        assert_eq!(
            setup
                .players
                .iter()
                .map(|p| p.name.as_str())
                .collect::<Vec<_>>(),
            ["姬百年", "莱息", "范一文", "多吉"]
        );
        assert_eq!(setup.players[0].controller, Controller::Human);
        for computer in &setup.players[1..] {
            assert!(matches!(computer.controller, Controller::Ai(_)));
        }
        let lai = set.get("lai-xi").unwrap();
        assert_eq!(setup.players[1].color, lai.player_color());
        assert_eq!(setup.players[1].gender, lai.gender);
    }

    /// 表里查不到任何一位就拒绝开局 —— 拿默认阵容顶上等于静默换人。
    #[test]
    fn a_roster_with_an_unknown_id_does_not_become_a_setup() {
        let set = shipped();
        let mut roster = RosterChoice::default();
        roster.computers[0] = "no-such-character".into();
        assert!(roster.to_setup(&set).is_none());
    }
}
