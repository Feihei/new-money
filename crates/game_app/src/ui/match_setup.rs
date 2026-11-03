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
//! 地图锚点（城市点与头像卡中心）住在 `assets/data/world_map_pins.json`，走的是**盘上
//! 那份为真**：改完存盘，这一屏半秒内自己重建（[`watch_map_pins`]），不用重编译。
//! 量坐标有两个开发入口：`NM_MAP_GRID=1` 打开 128 像素格线与光标处的原图坐标读数；
//! `NM_MAP_DRAG=1` 让城市点和头像卡可以直接拖（这时点卡片不再换人），松手写回 JSON。
//! 盘上那份读不出来（缺文件或语法错）时退回编译期内嵌的同一份内容，发行包照常能跑。
//!
//! 面板按签名重建（同设置面板）：开关、阵容、当前资金档、角色表是否加载完、锚点表修订号，
//! 五位都是可见位。

use std::collections::BTreeSet;

use bevy::{ecs::system::SystemParam, math::Rot2, prelude::*};
use game_core::prelude::{
    BoardDef, CharacterDef, CharacterSet, Controller, DataHandles, GameSetup, START_CASH_TIERS,
    StartCashChoice,
};
use serde::{Deserialize, Serialize};

use crate::art;
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

/// 世界地图在屏上的固定尺寸（2:1）。**固定像素而不是百分比**：引线的长度与角度
/// 要从锚点现算，而 `UiTransform` 的旋转只认像素；百分比在这里算不出角度。
const MAP_SIZE: [f32; 2] = [1000.0, 500.0];
/// 锚点表按原图 1536×768 量，屏上按 `MAP_SIZE` 等比缩 —— 换一张地图只要重量一次。
const MAP_DESIGN: [f32; 2] = [1536.0, 768.0];
/// 头像卡片与头像本身。卡片要容得下「名字 + 电脑 N + 设为你」三样，
/// 再窄就得把「设为你」挪出卡片，而那会让「换真人」与「加入/移出」两件事混在一起。
const CHIP_SIZE: [f32; 2] = [128.0, 44.0];
const AVATAR_SIZE: f32 = 32.0;

/// 一位角色的注册地在地图上的两处坐标（原图像素）：
/// `city` 是城市点，`chip` 是头像卡片中心。中国这一簇六位挤在东亚，
/// 卡片散在上方与右下太平洋，由引线指回城市点 —— 这就是「太密集时用引线」那条。
/// `note` 只是给人看的注册地（改坐标时好认行），不参与任何判据。
#[derive(Debug, Clone, Serialize, Deserialize)]
struct HomePin {
    id: String,
    city: [f32; 2],
    chip: [f32; 2],
    #[serde(default)]
    note: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PinFile {
    pins: Vec<HomePin>,
}

/// 锚点文件的编译期内嵌副本。盘上那一份优先（见 [`MapPins::load`]），
/// 它缺了或坏了就退回这里 —— 测试与发行包读的都是同一份内容。
const DEFAULT_PINS_JSON: &str = include_str!("../../../../assets/data/world_map_pins.json");
/// 盘上那一份的绝对路径。用 `CARGO_MANIFEST_DIR` 而不是相对路径：
/// 从 IDE / 双击 exe 启动时工作目录不一定是仓库根。
const PINS_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../assets/data/world_map_pins.json"
);

fn parse_pins(json: &str) -> Result<Vec<HomePin>, String> {
    serde_json::from_str::<PinFile>(json)
        .map(|file| file.pins)
        .map_err(|err| err.to_string())
}

fn embedded_pins() -> Vec<HomePin> {
    parse_pins(DEFAULT_PINS_JSON).expect("world_map_pins.json 内嵌副本解析失败")
}

fn file_mtime() -> Option<std::time::SystemTime> {
    std::fs::metadata(PINS_PATH)
        .and_then(|meta| meta.modified())
        .ok()
}

/// 锚点表 + 两个调试开关。`rev` 进面板签名：盘上那份一改，整屏重建一次；
/// 拖动期间只改坐标、不动 `rev`，否则每拖一帧都会把正在拖的那张卡重建掉。
#[derive(Resource, Debug)]
pub(crate) struct MapPins {
    pins: Vec<HomePin>,
    rev: u64,
    /// `NM_MAP_GRID=1`：128 像素格线 + 光标处的原图坐标读数
    grid: bool,
    /// `NM_MAP_DRAG=1`：城市点与头像卡可拖，松手写回 JSON（此时点卡不再换人）
    drag: bool,
    mtime: Option<std::time::SystemTime>,
}

impl Default for MapPins {
    /// 测试与 headless 用：不碰磁盘，只有内嵌那份。
    fn default() -> Self {
        Self {
            pins: embedded_pins(),
            rev: 0,
            grid: false,
            drag: false,
            mtime: None,
        }
    }
}

impl MapPins {
    /// 真实启动：盘上那份为真。解析失败只喊一句并退回内嵌，不挡住开局。
    pub(crate) fn load() -> Self {
        let mtime = file_mtime();
        let pins = std::fs::read_to_string(PINS_PATH)
            .ok()
            .and_then(|text| match parse_pins(&text) {
                Ok(pins) => Some(pins),
                Err(err) => {
                    warn!("{PINS_PATH} 解析失败，改用内嵌锚点：{err}");
                    None
                }
            })
            .unwrap_or_else(embedded_pins);
        Self {
            pins,
            rev: 0,
            grid: std::env::var_os("NM_MAP_GRID").is_some(),
            drag: std::env::var_os("NM_MAP_DRAG").is_some(),
            mtime,
        }
    }

    fn pin(&self, id: &str) -> Option<(usize, [f32; 2], [f32; 2])> {
        self.pins
            .iter()
            .enumerate()
            .find(|(_, pin)| pin.id == id)
            .map(|(index, pin)| (index, pin.city, pin.chip))
    }
}

/// 光标在地图上的原图坐标（`None` = 还没移上去过）。观测量在
/// `PointerMove` 的 `hit.position` 里是**相对节点的归一化坐标**
/// （左上 -0.5、右下 +0.5），乘回原图尺寸就是 JSON 里要填的数。
#[derive(Resource, Default)]
pub(crate) struct MapCursor(pub Option<Vec2>);

#[derive(Component)]
pub(crate) struct MapReadout;

/// 拖动与实时重摆要认得出「这条线/这个点/这张卡属于哪一位」。
#[derive(Component, Clone, Copy)]
pub(crate) struct MapPinMark {
    index: usize,
    part: PinPart,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PinPart {
    Dot,
    /// 拖动模式下那圈 24×24 的透明命中区（城市点本身只有 8 像素，抓不住）
    DotHit,
    Chip,
    Line,
}

/// 表外角色（将来加人）排在底边一行、卡片与城市点重合：宁可没有引线，
/// 也不能让人从地图上消失。
fn fallback_pin(index: usize) -> [f32; 2] {
    let x = 120.0 + (index % 10) as f32 * 120.0;
    [x, 700.0]
}

/// 原图像素 → 屏上像素。
fn map_px(p: [f32; 2]) -> Vec2 {
    Vec2::new(
        p[0] / MAP_DESIGN[0] * MAP_SIZE[0],
        p[1] / MAP_DESIGN[1] * MAP_SIZE[1],
    )
}

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
    /// 地图锚点表。`rev` 进签名：盘上那份一改就重建，不用重编译
    pub pins: Res<'w, MapPins>,
}

pub(crate) fn sync_match_setup_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    server: Res<AssetServer>,
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
        "{}|{}|{}|{}|{}|{}|{}|{}|{}",
        open.0,
        facts.roster.human,
        facts.roster.computers.join(","),
        facts.pick.0,
        current.unwrap_or(-1),
        default.unwrap_or(-1),
        map_stamp,
        set.map_or(0, |s| s.0.len()),
        // 锚点文件一改就整屏重建：这是「拖完/改完立刻看得到」的那一半
        facts.pins.rev
    );
    if open.0 && shown.iter().any(|(_, seen)| seen.0 == stamp) {
        return;
    }
    for (entity, _) in &shown {
        commands.entity(entity).despawn();
    }
    if open.0 {
        let board_line = maps
            .get(facts.pick.0)
            .map(|map| format!("{} · {}", map.label, map.line))
            .unwrap_or_default();
        spawn_panel(
            &mut commands,
            &font,
            &server,
            PanelView {
                roster: &facts.roster,
                picked: facts.choice.0,
                board_cash: default,
                maps,
                board_line,
                set,
                pins: &facts.pins,
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

/// 地图选择那一颗：只写 `BoardPick`，真正换盘在「开始对局」那一刻。
/// 统计数字不再塞进这颗按钮（地图一屏之后空间给了世界地图），改为页脚右侧一行。
fn map_chip(parent: &mut ChildSpawnerCommands, font: &Handle<Font>, width: f32, map: &MapView) {
    let bg = if map.selected {
        col::HUD_CARD_ACTIVE
    } else {
        col::BTN_SECONDARY
    };
    let edge = if map.selected {
        col::BORDER_BUY
    } else {
        col::BTN_BORDER
    };
    parent
        .spawn((
            Node {
                width: Val::Px(width),
                height: Val::Px(30.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(if map.selected {
                    layout::PANEL_BORDER
                } else {
                    layout::BTN_BORDER
                })),
                border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                ..default()
            },
            BackgroundColor(bg),
            border_color(edge),
            UiAction::SetBoard(map.index),
            Palette {
                normal: bg,
                hover: col::BTN_SECONDARY_HOVER,
            },
        ))
        .with_children(|chip| {
            let title = if map.selected {
                format!("● {}", map.label)
            } else {
                map.label.clone()
            };
            chip.spawn(label(
                font,
                title,
                size::SMALL,
                if map.selected {
                    col::TITLE_GOLD
                } else {
                    col::HINT
                },
            ));
        });
}

fn pending_map_chip(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    width: f32,
    title: &str,
) {
    parent
        .spawn((
            Node {
                width: Val::Px(width),
                height: Val::Px(30.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                ..default()
            },
            BackgroundColor(col::BTN_DISABLED),
            border_color(col::BTN_BORDER_DISABLED),
            Disabled,
        ))
        .with_children(|chip| {
            chip.spawn(label(font, title, size::SMALL, col::BTN_TEXT_DISABLED));
        });
}

/// 注册地只取城市那半（「乌拉圭·蒙得维的亚」→「蒙得维的亚」）：卡片宽 128，
/// 半行 10 号字塞得下六个汉字，而带国名会把这一行顶成两截。
fn home_city(home: &str) -> &str {
    home.rsplit('·').next().unwrap_or(home).trim()
}

/// 引线几何：从城市点连到卡片中心的 2 像素线。返回 `Node` + 旋转而不是直接生成，
/// 是因为拖动时要在原地重算同一份几何（[`apply_map_pins`]）—— 两处必须同一个表达式。
fn line_parts(city: [f32; 2], chip: [f32; 2]) -> Option<(Node, UiTransform)> {
    let a = map_px(city);
    let b = map_px(chip);
    let delta = b - a;
    let len = delta.length();
    if len < 1.0 {
        return None;
    }
    let mid = (a + b) * 0.5;
    Some((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(mid.x - len * 0.5),
            top: Val::Px(mid.y - 1.0),
            width: Val::Px(len),
            height: Val::Px(2.0),
            ..default()
        },
        UiTransform::from_rotation(Rot2::radians(delta.y.atan2(delta.x))),
    ))
}

/// 引线：卡片后画，所以线尾被卡片盖住，视觉上正好停在卡片边缘外。
fn leader_line(
    parent: &mut ChildSpawnerCommands,
    city: [f32; 2],
    chip: [f32; 2],
    index: Option<usize>,
) {
    let Some((node, transform)) = line_parts(city, chip) else {
        return;
    };
    let mut line = parent.spawn((node, BackgroundColor(col::MAP_LINE), transform));
    if let Some(index) = index {
        line.insert(MapPinMark {
            index,
            part: PinPart::Line,
        });
    }
}

fn city_dot(parent: &mut ChildSpawnerCommands, city: [f32; 2], index: Option<usize>, drag: bool) {
    let p = map_px(city);
    let mut dot = parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(p.x - 4.0),
            top: Val::Px(p.y - 4.0),
            width: Val::Px(8.0),
            height: Val::Px(8.0),
            border_radius: BorderRadius::all(Val::Px(4.0)),
            ..default()
        },
        BackgroundColor(col::MAP_DOT),
        // 纯装饰：不吃命中测试，光标读数才通得过它
        Pickable::IGNORE,
    ));
    let Some(index) = index else { return };
    dot.insert(MapPinMark {
        index,
        part: PinPart::Dot,
    });
    if drag {
        // 8 像素的点抓不住，拖动模式给一圈 24×24 的透明命中区
        let mut hit = parent.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(p.x - 12.0),
                top: Val::Px(p.y - 12.0),
                width: Val::Px(24.0),
                height: Val::Px(24.0),
                ..default()
            },
            BackgroundColor(Color::NONE),
            MapPinMark {
                index,
                part: PinPart::DotHit,
            },
        ));
        make_draggable(&mut hit, index, PinPart::Dot);
    }
}

/// 拖动一位的坐标：屏幕像素 → 原图像素后累加。只写资源、不碰 `rev` ——
/// 面板靠 `rev` 重建，拖一帧重建一次就没法拖了。
fn drag_pin(mut pins: ResMut<MapPins>, index: usize, part: PinPart, delta: Vec2) {
    let step = Vec2::new(
        delta.x * MAP_DESIGN[0] / MAP_SIZE[0],
        delta.y * MAP_DESIGN[1] / MAP_SIZE[1],
    );
    let Some(pin) = pins.pins.get_mut(index) else {
        return;
    };
    let target = match part {
        PinPart::Chip => &mut pin.chip,
        _ => &mut pin.city,
    };
    target[0] += step.x;
    target[1] += step.y;
}

fn save_pins(pins: &MapPins) {
    let file = PinFile {
        pins: pins.pins.clone(),
    };
    let text = match serde_json::to_string_pretty(&file) {
        Ok(text) => text + "\n",
        Err(err) => {
            warn!("世界地图锚点序列化失败：{err}");
            return;
        }
    };
    match std::fs::write(PINS_PATH, text) {
        Ok(()) => info!("世界地图锚点已写回 {PINS_PATH}"),
        Err(err) => warn!("写回 {PINS_PATH} 失败：{err}"),
    }
}

/// 拖动与松手写回共用一对观察者（城市点与头像卡各挂一次）。
fn make_draggable(entity: &mut EntityCommands, index: usize, part: PinPart) {
    entity.observe(move |drag: On<PointerDrag>, pins: ResMut<MapPins>| {
        drag_pin(pins, index, part, drag.delta);
    });
    entity.observe(|_: On<PointerDragEnd>, pins: Res<MapPins>| save_pins(&pins));
}

/// `NM_MAP_GRID=1` 的开发覆盖层：每 128 原图像素一条线（12×6 格），
/// 每 256 像素标一个原图坐标。它只是量尺，整层 `Pickable::IGNORE` ——
/// 否则它会顶掉底图上的光标读数。
fn grid_overlay(parent: &mut ChildSpawnerCommands, font: &Handle<Font>) {
    let step = 128.0;
    let mut x = step;
    while x < MAP_DESIGN[0] {
        let sx = x / MAP_DESIGN[0] * MAP_SIZE[0];
        parent.spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(sx),
                width: Val::Px(1.0),
                height: Val::Percent(100.0),
                ..default()
            },
            BackgroundColor(col::MAP_GRID),
            Pickable::IGNORE,
        ));
        x += step;
    }
    let mut y = step;
    while y < MAP_DESIGN[1] {
        let sy = y / MAP_DESIGN[1] * MAP_SIZE[1];
        parent.spawn((
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(sy),
                width: Val::Percent(100.0),
                height: Val::Px(1.0),
                ..default()
            },
            BackgroundColor(col::MAP_GRID),
            Pickable::IGNORE,
        ));
        y += step;
    }
    let mut x = step * 2.0;
    while x < MAP_DESIGN[0] {
        let sx = x / MAP_DESIGN[0] * MAP_SIZE[0];
        grid_label(parent, font, &format!("{x:.0}"), sx + 2.0, 1.0);
        x += step * 2.0;
    }
    let mut y = step * 2.0;
    while y < MAP_DESIGN[1] {
        let sy = y / MAP_DESIGN[1] * MAP_SIZE[1];
        grid_label(parent, font, &format!("{y:.0}"), 2.0, sy + 1.0);
        y += step * 2.0;
    }
}

fn grid_label(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    text: &str,
    left: f32,
    top: f32,
) {
    parent
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(left),
                top: Val::Px(top),
                padding: UiRect::horizontal(Val::Px(2.0)),
                ..default()
            },
            BackgroundColor(col::MAP_GRID_BACKDROP),
            Pickable::IGNORE,
        ))
        .with_children(|chip| {
            chip.spawn((label(font, text, 9.0, col::MAP_GRID_TEXT), Pickable::IGNORE));
        });
}

/// 世界地图本身：底图 + 引线 + 城市点 + 头像卡片。素材缺席时底图退回素纸，
/// 但头像仍按同一套锚点摆 —— 缺一张图不该让这一屏失去功能（`art` 模块的纪律）。
fn spawn_world_map(
    parent: &mut ChildSpawnerCommands,
    font: &Handle<Font>,
    server: &AssetServer,
    set: Option<&CharacterSet>,
    roster: &RosterChoice,
    pins: &MapPins,
) {
    let mut frame = parent.spawn((
        Node {
            width: Val::Px(MAP_SIZE[0]),
            height: Val::Px(MAP_SIZE[1]),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        BackgroundColor(col::MAP_PAPER),
        border_color(col::TITLE_GOLD),
    ));
    if let Some(path) = art::world_map() {
        frame.with_children(|map| {
            // 圆角裁切只包住底图这一层。**不能把整张地图框设成 `Overflow::clip()`**：
            // 裁剪会把所有子节点一起挡在命中测试之外，而头像卡片正是它的子节点 ——
            // 症状是「图看着对、头像也画出来了，但怎么点都没反应」（本阶段真鼠标冒烟踩到）
            map.spawn((Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
                overflow: Overflow::clip(),
                ..default()
            },))
                .with_children(|clip| {
                    clip.spawn((
                        ImageNode {
                            image: server.load(path),
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Node {
                            width: Val::Percent(100.0),
                            height: Val::Percent(100.0),
                            ..default()
                        },
                    ))
                    // 光标读数挂在**底图**上：`hit.position` 是相对命中节点的归一化坐标
                    // （左上 -0.5、右下 +0.5），底图正好铺满整张地图，换算回原图像素
                    // 就是 JSON 里要填的数
                    .observe(
                        |mov: On<PointerMove>, mut cursor: ResMut<MapCursor>| {
                            if let Some(pos) = mov.hit.position {
                                cursor.0 = Some(Vec2::new(
                                    (pos.x + 0.5) * MAP_DESIGN[0],
                                    (pos.y + 0.5) * MAP_DESIGN[1],
                                ));
                            }
                        },
                    );
                });
        });
    }
    frame.with_children(|map| {
        if pins.grid {
            grid_overlay(map, font);
            map.spawn((
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(8.0),
                    top: Val::Px(8.0),
                    padding: UiRect::axes(Val::Px(6.0), Val::Px(3.0)),
                    border: UiRect::all(Val::Px(1.0)),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    ..default()
                },
                BackgroundColor(col::MAP_PAPER),
                border_color(col::MAP_DOT),
                Pickable::IGNORE,
            ))
            .with_children(|badge| {
                badge.spawn((
                    label(font, "地图坐标  x —   y —", 10.0, col::MAP_GRID_TEXT),
                    MapReadout,
                    Pickable::IGNORE,
                ));
            });
        }
        let Some(set) = set else {
            return;
        };
        // 先把每位角色的锚点算齐：三段循环（线 / 点 / 卡）读的必须是同一份，
        // 而 `pins.pin` 的查找只做一次
        let anchors = set
            .0
            .iter()
            .enumerate()
            .map(|(seat, def)| match pins.pin(&def.id) {
                Some((index, city, chip)) => (Some(index), city, chip),
                None => {
                    let p = fallback_pin(seat);
                    (None, p, p)
                }
            })
            .collect::<Vec<_>>();
        for &(index, city, chip) in &anchors {
            leader_line(map, city, chip, index);
        }
        for &(index, city, _) in &anchors {
            city_dot(map, city, index, pins.drag);
        }
        for (seat, (def, &(index, _, chip))) in set.0.iter().zip(&anchors).enumerate() {
            let role = roster.role_of(&def.id);
            let clickable = match role {
                None => roster.can_add(&def.id),
                Some(Role::Human) => false,
                Some(Role::Computer(_)) => roster.can_remove(&def.id),
            };
            character_chip(
                map,
                ChipSpec {
                    font,
                    server,
                    def,
                    role,
                    clickable,
                    seat,
                    chip,
                    pin: index,
                    drag: pins.drag,
                },
            );
        }
    });
}

/// 每 0.25 秒看一眼锚点文件的修改时间：改完存盘就重载，把 `rev` 加一让整屏重建。
/// 轮询而不是 watcher 依赖：这一屏一局最多开几十秒，一次 `metadata` 比多一个依赖便宜。
pub(crate) fn watch_map_pins(time: Res<Time>, mut pins: ResMut<MapPins>, mut poll: Local<f32>) {
    *poll += time.delta_secs();
    if *poll < 0.25 {
        return;
    }
    *poll = 0.0;
    let Some(mtime) = file_mtime() else {
        return;
    };
    if pins.mtime == Some(mtime) {
        return;
    }
    // 先把时间戳记住再读：文件正在写一半时解析失败，等下一次修改再试，
    // 而不是每 0.25 秒骂一句
    pins.bypass_change_detection().mtime = Some(mtime);
    match std::fs::read_to_string(PINS_PATH)
        .ok()
        .and_then(|text| parse_pins(&text).ok())
    {
        Some(next) => {
            pins.pins = next;
            pins.rev += 1;
            info!("世界地图锚点已重载：{PINS_PATH}");
        }
        None => warn!("{PINS_PATH} 读不出来，继续用上一次的锚点"),
    }
}

/// 拖动时把新坐标落到已有节点上（不重建整屏 —— 重建会把正在拖的实体换掉）。
/// `line_parts` 与生成时是同一个表达式，所以引线不会越拖越歪。
pub(crate) fn apply_map_pins(
    pins: Res<MapPins>,
    mut marks: Query<(&MapPinMark, &mut Node, Option<&mut UiTransform>)>,
) {
    if !pins.is_changed() {
        return;
    }
    for (mark, mut node, transform) in &mut marks {
        let Some(pin) = pins.pins.get(mark.index) else {
            continue;
        };
        match mark.part {
            PinPart::Dot => {
                let p = map_px(pin.city);
                node.left = Val::Px(p.x - 4.0);
                node.top = Val::Px(p.y - 4.0);
            }
            PinPart::DotHit => {
                let p = map_px(pin.city);
                node.left = Val::Px(p.x - 12.0);
                node.top = Val::Px(p.y - 12.0);
            }
            PinPart::Chip => {
                let p = map_px(pin.chip);
                node.left = Val::Px(p.x - CHIP_SIZE[0] * 0.5);
                node.top = Val::Px(p.y - CHIP_SIZE[1] * 0.5);
            }
            PinPart::Line => {
                let Some(mut transform) = transform else {
                    continue;
                };
                match line_parts(pin.city, pin.chip) {
                    Some((src, rotation)) => {
                        node.left = src.left;
                        node.top = src.top;
                        node.width = src.width;
                        *transform = rotation;
                    }
                    None => node.width = Val::Px(0.0),
                }
            }
        }
    }
}

/// 网格模式下左上角那行读数：`x` / `y` 就是 JSON 里要填的原图像素。
pub(crate) fn refresh_map_readout(
    cursor: Res<MapCursor>,
    mut texts: Query<&mut Text, With<MapReadout>>,
) {
    if !cursor.is_changed() {
        return;
    }
    let line = match cursor.0 {
        Some(p) => format!("地图坐标  x {:.0}   y {:.0}", p.x, p.y),
        None => "地图坐标  x —   y —".to_string(),
    };
    for mut text in &mut texts {
        *text = Text::new(line.clone());
    }
}

/// 一张头像卡片的全部读数。打包是房规（§六 坑 8）：八个参数撞
/// `too_many_arguments`，而它们说的本来就是同一件事 ——「这一位怎么画」。
struct ChipSpec<'a> {
    font: &'a Handle<Font>,
    server: &'a AssetServer,
    def: &'a CharacterDef,
    role: Option<Role>,
    clickable: bool,
    /// 座位号：名字查不到 sprite 时按它领人（见 `art::strip_for`）
    seat: usize,
    chip: [f32; 2],
    /// 锚点表里的下标（表外角色没有，于是不挂拖动标记）
    pin: Option<usize>,
    /// `NM_MAP_DRAG=1`：可拖，且**不挂 `UiAction`**
    drag: bool,
}

fn character_chip(parent: &mut ChildSpawnerCommands, spec: ChipSpec) {
    let ChipSpec {
        font,
        server,
        def,
        role,
        clickable,
        seat,
        chip,
        pin,
        drag,
    } = spec;
    let selected = role.is_some();
    let bg = if !clickable && !selected {
        col::BTN_DISABLED
    } else if selected {
        col::HUD_CARD_ACTIVE
    } else {
        col::HUD_CARD
    };
    let edge = if selected {
        col::TITLE_GOLD
    } else if clickable {
        col::BTN_BORDER
    } else {
        col::BTN_BORDER_DISABLED
    };
    let center = map_px(chip);
    let [width, height] = CHIP_SIZE;
    let mut node = parent.spawn((
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(center.x - width * 0.5),
            top: Val::Px(center.y - height * 0.5),
            width: Val::Px(width),
            height: Val::Px(height),
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(6.0),
            padding: UiRect::axes(Val::Px(5.0), Val::Px(4.0)),
            border: UiRect::all(Val::Px(if selected {
                layout::PANEL_BORDER
            } else {
                layout::BTN_BORDER
            })),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            overflow: Overflow::clip(),
            ..default()
        },
        BackgroundColor(bg),
        border_color(edge),
        Palette {
            normal: bg,
            hover: col::BTN_SECONDARY_HOVER,
        },
    ));
    // 拖动模式里**故意不挂 `UiAction`**：Bevy 0.20 的 `PointerClick` 只看
    // 「按下与抬起是不是同一实体」，拖完松手照样会发一条 —— 留着动作就等于
    // 每拖一次顺手改一次阵容。要换人就关掉 `NM_MAP_DRAG` 再开
    if !drag {
        node.insert(UiAction::ToggleCharacter(def.id.clone()));
    }
    if !clickable {
        node.insert(Disabled);
    }
    if let Some(index) = pin {
        node.insert(MapPinMark {
            index,
            part: PinPart::Chip,
        });
        if drag {
            make_draggable(&mut node, index, PinPart::Chip);
        }
    }
    let [r, g, b] = def.color;
    let tint = Color::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let sprite = art::strip_for(&def.name, seat);
    node.with_children(|card| {
        let mut avatar = card.spawn((
            Node {
                width: Val::Px(AVATAR_SIZE),
                height: Val::Px(AVATAR_SIZE),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                border: UiRect::all(Val::Px(1.0)),
                border_radius: BorderRadius::all(Val::Px(AVATAR_SIZE * 0.5)),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(tint),
            border_color(Color::WHITE),
        ));
        if let Some(path) = sprite {
            avatar.with_children(|face| {
                face.spawn((
                    ImageNode {
                        image: server.load(path),
                        // 八帧横条的第一帧，只取头 + 上半身：整帧缩到 32px 时
                        // 脸只剩几像素，认不出是谁
                        rect: Some(Rect::new(16.0, 0.0, 112.0, 96.0)),
                        image_mode: NodeImageMode::Stretch,
                        ..default()
                    },
                    Node {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                ));
            });
        } else {
            avatar.with_children(|face| {
                face.spawn(label(
                    font,
                    def.name.chars().next().unwrap_or('?').to_string(),
                    14.0,
                    Color::WHITE,
                ));
            });
        }
        card.spawn(Node {
            flex_grow: 1.0,
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(1.0),
            ..default()
        })
        .with_children(|text| {
            text.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                justify_content: JustifyContent::SpaceBetween,
                ..default()
            })
            .with_children(|line| {
                line.spawn(label(
                    font,
                    &def.name,
                    13.0,
                    if clickable || selected {
                        col::HINT
                    } else {
                        col::BTN_TEXT_DISABLED
                    },
                ));
                let mark = match role {
                    Some(_) => "",
                    None if clickable => "+",
                    None => "满",
                };
                if !mark.is_empty() {
                    line.spawn(label(
                        font,
                        mark,
                        size::SMALL,
                        if clickable { col::TITLE_GOLD } else { col::KEY },
                    ));
                }
            });
            match role {
                Some(Role::Human) => {
                    text.spawn(label(font, "你 · 真人", 10.0, col::TITLE_GOLD));
                }
                Some(Role::Computer(index)) => {
                    text.spawn(Node {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        justify_content: JustifyContent::SpaceBetween,
                        ..default()
                    })
                    .with_children(|line| {
                        line.spawn(label(font, format!("电脑 {}", index + 1), 10.0, col::GAIN));
                        let mut btn = line.spawn((
                            Node {
                                width: Val::Px(40.0),
                                height: Val::Px(16.0),
                                justify_content: JustifyContent::Center,
                                align_items: AlignItems::Center,
                                border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                                border_radius: BorderRadius::all(Val::Px(3.0)),
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
                            b.spawn(label(font, "设为你", 9.0, Color::WHITE));
                        });
                    });
                }
                None => {
                    text.spawn(label(
                        font,
                        home_city(&def.home),
                        10.0,
                        if clickable {
                            col::MUTED
                        } else {
                            col::BTN_TEXT_DISABLED
                        },
                    ));
                }
            }
        });
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
    /// 选中那张盘的统计行（页脚右侧）
    board_line: String,
    set: Option<&'a CharacterSet>,
    pins: &'a MapPins,
    stamp: String,
}

fn spawn_panel(commands: &mut Commands, font: &DevFont, server: &AssetServer, view: PanelView) {
    let PanelView {
        roster,
        picked,
        board_cash,
        maps,
        board_line,
        set,
        pins,
        stamp,
    } = view;
    // 全屏场景，不再是模态框：内容宽 1184，四周留 32/12 的内边距
    let width = 1184.0;
    let inner = width;
    let map_chip_width = 126.0;
    let tier_width = 84.0;
    let current = picked.or(board_cash);
    let ready = set.and_then(|s| roster.to_setup(s)).is_some();
    let start_reason = if set.is_none() {
        Some("角色表还没加载完")
    } else if !ready {
        Some("阵容里有角色不在角色表里")
    } else {
        None
    };
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
                padding: UiRect::axes(Val::Px(32.0), Val::Px(12.0)),
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
                    // ---- 标题与阵容读数 ----
                    panel
                        .spawn(Node {
                            width: Val::Percent(100.0),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(label(&font.0, "新对局", size::SCENE_TITLE, col::TITLE_GOLD));
                            row.spawn(label(
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
                    panel.spawn(separator());

                    // ---- 地图 / 起始资金（同一行工具条）----
                    panel
                        .spawn(Node {
                            width: Val::Px(inner),
                            flex_direction: FlexDirection::Row,
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            row.spawn(Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(8.0),
                                ..default()
                            })
                            .with_children(|group| {
                                group.spawn(label(&font.0, "地图", size::BODY, col::HINT));
                                if maps.is_empty() {
                                    pending_map_chip(
                                        group,
                                        &font.0,
                                        map_chip_width,
                                        "棋盘加载中…",
                                    );
                                }
                                for map in &maps {
                                    map_chip(group, &font.0, map_chip_width, map);
                                }
                                pending_map_chip(group, &font.0, map_chip_width, "世界 待做");
                            });
                            row.spawn(Node {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(8.0),
                                ..default()
                            })
                            .with_children(|group| {
                                group.spawn(label(&font.0, "起始资金", size::BODY, col::HINT));
                                for (index, cash) in START_CASH_TIERS.iter().enumerate() {
                                    let on = current == Some(*cash);
                                    let text = if on {
                                        format!("● {} 万", cash / 10_000)
                                    } else {
                                        format!("{} 万", cash / 10_000)
                                    };
                                    button(
                                        group,
                                        &font.0,
                                        ButtonSpec {
                                            font: size::SMALL,
                                            ..ButtonSpec::new(
                                                &text,
                                                [tier_width, 30.0],
                                                SECONDARY,
                                                UiAction::SetStartCash(index),
                                            )
                                        },
                                    );
                                }
                            });
                        });

                    // ---- 世界地图：头像钉在注册地，密集处用引线 ----
                    panel
                        .spawn(Node {
                            width: Val::Px(inner),
                            flex_grow: 1.0,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|center| {
                            spawn_world_map(center, &font.0, server, set, roster, pins);
                        });

                    // ---- 页脚：操作提示（或开局被拒的理由）与选中盘的统计 ----
                    panel
                        .spawn(Node {
                            width: Val::Px(inner),
                            justify_content: JustifyContent::SpaceBetween,
                            align_items: AlignItems::Center,
                            ..default()
                        })
                        .with_children(|row| {
                            let (hint, hint_color) = match start_reason {
                                Some(reason) => (reason, col::KEY),
                                None => (
                                    "点头像加入 / 移出 · 选中电脑可点「设为你」换真人 · 至少两位电脑",
                                    col::MUTED,
                                ),
                            };
                            row.spawn(label(&font.0, hint, size::SMALL, hint_color));
                            row.spawn(label(&font.0, board_line, size::SMALL, col::KEY));
                        });

                    // ---- 开始 / 返回 ----
                    panel
                        .spawn(Node {
                            width: Val::Px(inner),
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(8.0),
                            ..default()
                        })
                        .with_children(|row| {
                            button(
                                row,
                                &font.0,
                                ButtonSpec::new(
                                    "开始对局",
                                    [inner - 248.0, 42.0],
                                    PRIMARY,
                                    UiAction::StartMatch,
                                )
                                .disabled(start_reason.is_some()),
                            );
                            button(
                                row,
                                &font.0,
                                ButtonSpec::new(
                                    "返回",
                                    [240.0, 42.0],
                                    SECONDARY,
                                    UiAction::ToggleMatchSetup,
                                ),
                            );
                        });
                });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shipped() -> CharacterSet {
        CharacterSet::from_json(include_str!("../../../../assets/data/characters.json"))
            .expect("assets/data/characters.json 解析失败")
    }

    /// 每一位首发角色都得有**显式**锚点：`fallback_pin` 只保证「不消失」，
    /// 不保证「钉在注册地」。少了锚点的人会落在底边一排，而地图上没人看得出那是错的。
    /// 读的是盘上那份 JSON（与运行时同一份），所以重量坐标之后这条仍然管用。
    #[test]
    fn every_shipped_character_has_a_world_map_pin() {
        let set = shipped();
        let pins = embedded_pins();
        for def in &set.0 {
            assert!(
                pins.iter().any(|pin| pin.id == def.id),
                "{}（{}）没有注册地锚点",
                def.name,
                def.id
            );
        }
        // 反过来也要钉：表里多出一行没人用的锚点，多半是改了 id 忘了删
        for pin in &pins {
            assert!(set.get(&pin.id).is_some(), "{} 的锚点没有对应角色", pin.id);
        }
    }

    /// 卡片必须整块留在图内（引线可以把城市点拉到边缘，卡片不行）；任意两张不重叠。
    /// 这是「太密集就用引线」那条的机器判据：挤不开就散开，而不是让名字叠成一团。
    #[test]
    fn avatar_chips_stay_inside_the_map_and_never_overlap() {
        let half = Vec2::new(CHIP_SIZE[0] * 0.5, CHIP_SIZE[1] * 0.5);
        let pins = embedded_pins();
        for pin in &pins {
            let p = map_px(pin.chip);
            assert!(
                p.x >= half.x
                    && p.x <= MAP_SIZE[0] - half.x
                    && p.y >= half.y
                    && p.y <= MAP_SIZE[1] - half.y,
                "{} 的卡片出图了：{p:?}",
                pin.id
            );
        }
        for (i, a) in pins.iter().enumerate() {
            for b in &pins[i + 1..] {
                let pa = map_px(a.chip);
                let pb = map_px(b.chip);
                let overlap =
                    (pa.x - pb.x).abs() < CHIP_SIZE[0] && (pa.y - pb.y).abs() < CHIP_SIZE[1];
                assert!(!overlap, "{} 与 {} 的卡片叠在一起", a.id, b.id);
            }
        }
    }

    /// 城市点必须落在纸面内：卡片可以引到图边，点不行 —— 线的起点跑到框外
    /// 就是一条从画外伸进来的线，而 JSON 里写错一个零很容易看不出来。
    #[test]
    fn city_dots_stay_inside_the_map() {
        for pin in &embedded_pins() {
            let p = map_px(pin.city);
            assert!(
                (0.0..=MAP_SIZE[0]).contains(&p.x) && (0.0..=MAP_SIZE[1]).contains(&p.y),
                "{} 的城市点出图了：{p:?}",
                pin.id
            );
        }
    }

    /// 拖动写回的那条路必须读得回来：`save_pins` 与 `parse_pins` 是同一份
    /// `PinFile`，但两条路各写一遍字段的话，调完坐标一启动就退回内嵌那份。
    #[test]
    fn pin_file_round_trips_through_json() {
        let pins = embedded_pins();
        let text =
            serde_json::to_string_pretty(&PinFile { pins: pins.clone() }).expect("锚点序列化失败");
        let back = parse_pins(&text).expect("锚点回读失败");
        assert_eq!(back.len(), pins.len());
        for (a, b) in pins.iter().zip(&back) {
            assert_eq!(a.id, b.id);
            assert_eq!(a.city, b.city);
            assert_eq!(a.chip, b.chip);
        }
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
