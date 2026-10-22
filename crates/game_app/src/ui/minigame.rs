//! 游乐场（P7）：三个能真玩的小游戏，与主菜单那一块的练习入口。
//!
//! 分工照 PLAN §五：**规则层只有接口** —— `Pending::Minigame(kind)` 把回合挂住，玩完报一份
//! `MinigameResult`，点券由 `GameRules::minigames` 那张表算。玩法本体全在这里：气球怎么升、
//! 篮子怎么接、冰洞里埋着什么，core 一概不知道。
//!
//! 四件刻意的事：
//!
//! - **随机从 core 领，不在这里摇**。挖宝埋在哪几格决定得分、得分换点券、点券是账，所以那份
//!   随机由 [`ArcadeDeal::seed`] 发下来（同一个 `NM_SEED` 走到同一格摆出同一局）。这里拿它
//!   自己开一条 `ChaCha8Rng`：表现层不碰规则层的随机流（与 P2b 老虎机同一条分工）。
//! - **画面不靠重建**。三种玩法每帧都在动，所以节点在开局那一刻建好，之后逐帧**就地改**
//!   `Node`/`Text`/`BackgroundColor`。「签名变了才重建」那套（§6.1 坑 30）只管「这一屏是
//!   哪一局」，不管分数与秒数 —— 那两个数每帧都算，但只在字符串真变了的时候才赋值
//!   （反复赋同一个串会让排版每帧重来一遍）。
//! - **练习局不给点券是写出来的**。这一屏的状态里带着 `practice` 那一位，结算屏上写着
//!   「练习局不计点券（这一局值 N 点券）」，而 N 用的是 core 那张表 —— 映射只有一份，
//!   开关是一个值。⚠️ 它**不发** `MinigameResult`：菜单里没有 `ActiveBoard`，而
//!   [`game_core::prelude::Books`]（点券的唯一入口）要它；硬发的话那条消息会躺在队列里，
//!   等到下一局窗口第一次打开时被读走 —— 那是跨局残留（§6.1 坑 22/25）的一个新变种。
//!   `awarded: false` 那半条路由 `game_core/tests/minigame.rs` 在**有窗口**的场合钉住。
//! - **面板跟着 `PendingDecision` 生灭**，不跟着消息：消息晚一帧，而玩家在那一帧就已经能
//!   点别的东西了（`dialog::sync_modal` 的同一条理由）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ArcadeDeal, GameKind, GameRules, MinigameResult, Paused, Payout, Pending, PendingDecision,
};
use rand::Rng as _;
use rand::SeedableRng as _;
use rand_chacha::ChaCha8Rng;

use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, Disabled, MenuPiece, PRIMARY, Palette, SECONDARY, UiAction, border_color,
    button, label, overlay, panel, separator,
};
use crate::render::DevFont;
use crate::scene::Scene;

// ===== 玩法参数 =====
// ⚠️ 这几个数与 `GameRules::minigames` 的**满分是一对**：满分写 25 只而这里 25 秒只升得起
// 12 只，那一档就成了永远拿不到的装饰（改一边就去另一边对一眼）。

/// 两种限时玩法各给多久。25 秒是「一个回合里插一段小游戏」的上限 —— 再长就成了打断对局的
/// 另一款游戏，而 4 代那一档本来也就半分钟。
const ROUND_SECS: f32 = 25.0;
/// 场上同时有几只气球。点掉一只立刻从底部换一只新的，所以「密度」是这个数而不是刷怪间隔。
const BALLOONS: usize = 8;
/// 气球升起的速度区间（px/s）与直径区间（px）。
const BALLOON_RISE: [f32; 2] = [55.0, 110.0];
const BALLOON_SIZE: [f32; 2] = [34.0, 52.0];
/// 接财宝：多久掉一件、掉落速度区间、财宝占比（其余是石头，接到倒扣一分）。
const FALL_EVERY: f32 = 0.7;
const FALL_SPEED: [f32; 2] = [130.0, 210.0];
const TREASURE_ODDS: f32 = 0.8;
/// 掉下来的东西多大、篮子多宽多高。接住判定用的是两个半宽之和，不单开一个常数 ——
/// 判定比看起来宽一点是这类手感游戏的常规，而「宽多少」由这两个尺寸说了算，不该再有一个旋钮。
const FALL_SIZE: f32 = 26.0;
const BASKET_W: f32 = 96.0;
const BASKET_H: f32 = 26.0;
/// 键盘移动篮子的速度（px/s）。鼠标是「指哪到哪」，键盘得给一个跟得上的数。
const BASKET_SPEED: f32 = 540.0;
/// 挖宝：4×4 = 16 格，埋 6 件宝，只有 8 铲。8 铲配 6 件宝让「六铲全中」要靠判断加运气
/// （铲空的区域要舍得放弃），而期望值 3 件正好落在那张表的中间档。
const DIG_SIDE: usize = 4;
const DIG_CELLS: usize = DIG_SIDE * DIG_SIDE;
const DIG_TREASURES: usize = 6;
const DIG_BUDGET: u32 = 8;
/// 一帧最多推进多少秒：切窗口回来那一帧的 `delta` 可以是几十秒，不夹的话气球会瞬移穿顶、
/// 财宝会直接落到地上（而玩家看到的是「一开始就结束」）。
const MAX_DT: f32 = 0.05;

fn field_w() -> f32 {
    layout::ARCADE_FIELD[0]
}

fn field_h() -> f32 {
    layout::ARCADE_FIELD[1]
}

// ===== 状态 =====

/// 主菜单那块「选一款练习」开着吗。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ArcadeChoiceOpen(pub bool);

/// 练习局要玩哪一款（`None` = 没有）。菜单那颗按钮写下它、「离开」清掉它。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Practice(pub Option<GameKind>);

/// 练习局开过几手。它存在的唯一理由是**当签名用**：「再来一局」把它加一，[`sync_arcade`]
/// 就看出这一屏该换一手布局重建。并进 `Practice` 里不行 —— 连点两次「再来一局」
/// 第二次会被判成「同一局，不用重建」，而那正是玩家刚刚按下去的那颗按钮。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PracticeRounds(pub u64);

/// 现在开着的那一局（`None` = 没有）。UI 自己的状态：规则层只知道「窗口挂着」。
#[derive(Resource, Default)]
pub(crate) struct Arcade(pub Option<Round>);

/// 这一屏该是哪一局。只有两个来源：对局里的窗口，与菜单里的练习请求。
#[derive(Debug, Clone, Copy, PartialEq)]
enum Wanted {
    Match {
        player: Entity,
        game: GameKind,
        seed: u64,
    },
    Practice {
        game: GameKind,
        rounds: u64,
    },
}

impl Wanted {
    fn game(self) -> GameKind {
        match self {
            Self::Match { game, .. } | Self::Practice { game, .. } => game,
        }
    }

    fn practice(self) -> bool {
        matches!(self, Self::Practice { .. })
    }

    /// 布局种子。对局里那一手由 core 发（[`ArcadeDeal`]）；练习局没有 core 可问，
    /// 而它不发点券，所以「同一手重开」不是账上的事 —— 用第几手当种子就够。
    fn seed(self) -> u64 {
        match self {
            Self::Match { seed, .. } => seed,
            Self::Practice { rounds, .. } => rounds,
        }
    }

    fn player(self) -> Option<Entity> {
        match self {
            Self::Match { player, .. } => Some(player),
            Self::Practice { .. } => None,
        }
    }
}

pub(crate) struct Round {
    wanted: Wanted,
    /// 开局那一刻从 `GameRules` 抄下来的那一档。抄一份而不是每帧去查：
    /// 这一屏上写着「满分 25 · 每分 4 点券」，而那句话在整局里都不该变
    payout: Payout,
    /// 这一局的随机。从 core 发下来的那个种子展开，**不与规则层的流共用**
    rng: ChaCha8Rng,
    score: u32,
    /// 限时玩法的剩余秒数。挖宝不看它（看 `Play::Dig::digs`），所以那一支把铲数写进来
    /// 只为了让「还剩几铲」那一行有个来源
    left: f32,
    over: bool,
    /// 结算屏已经摆出来了。分成两位是因为「收工」由派发那一侧写下，
    /// 而摆屏要动节点，只有这一侧手里有查询
    over_shown: bool,
    play: Play,
    // 建好的节点：逐帧就地改，不重建
    field: Entity,
    result_box: Entity,
    footer: Entity,
    score_text: Entity,
    clock_text: Entity,
    result_text: Entity,
}

enum Play {
    /// 打气球：`BALLOONS` 只一直在场上，点掉或飞出顶的就从底部换一只新的
    Balloons { slots: Vec<Balloon> },
    /// 接财宝：篮子 + 一串往下掉的东西
    Catch {
        basket: f32,
        /// 上一帧鼠标在哪。只在它**真动了**的那一帧接管篮子，否则键盘永远抢不回来
        cursor: Option<Vec2>,
        falls: Vec<Fall>,
        spawn_in: f32,
        basket_node: Entity,
    },
    /// 企鹅挖宝：4×4，6 格埋着宝，8 铲
    Dig { cells: Vec<Cell>, digs: u32 },
}

struct Balloon {
    x: f32,
    y: f32,
    vy: f32,
    size: f32,
    node: Entity,
}

struct Fall {
    x: f32,
    y: f32,
    vy: f32,
    treasure: bool,
    node: Entity,
}

struct Cell {
    treasure: bool,
    dug: bool,
    /// 已经按「铲开了」上过色。没有这一位的话每帧都会重写一遍背景与文字，
    /// 而重写会触发变更检测（那 16 格的排版每帧重来一次）
    painted: bool,
    node: Entity,
    mark: Entity,
}

/// 玩法区的根。只用来让 [`sync_arcade`] 查得到「现在挂着没有」——
/// 与 `dialog::ModalRoot`、`pause::PausePanel` 同一条理由：存成 `Local` 的话，回菜单时面板
/// 随 `BoardUi` 被收走、句柄却留着，下一局就会对着死实体调 `despawn`。
#[derive(Component)]
pub(crate) struct ArcadePanel;

/// 菜单上那块「选一款练习」。
#[derive(Component)]
pub(crate) struct ArcadeChoice;

/// 逐帧就地刷画面要动的那几样。打包是房规（§六 坑 8）：这一屏每帧要改位置、文字、
/// 字色、底色与悬停色，五个查询散装就是十一个参数。
#[derive(SystemParam)]
pub(crate) struct Paint<'w, 's> {
    nodes: Query<'w, 's, &'static mut Node>,
    texts: Query<'w, 's, &'static mut Text>,
    inks: Query<'w, 's, &'static mut TextColor>,
    bgs: Query<'w, 's, &'static mut BackgroundColor>,
    palettes: Query<'w, 's, &'static mut Palette>,
}

// ===== 纯函数：这一手怎么发 =====

/// 挖宝那一手：`DIG_CELLS` 格里埋 `DIG_TREASURES` 件宝。
///
/// 部分 Fisher-Yates（只把前 6 个位置换到位），与 `shop::restock` 同一条手法。
/// 纯函数 + 自己那条流，所以「哪几格有宝」既能被单测钉住，又碰不到规则层的随机性。
pub(crate) fn dig_deal(seed: u64) -> [bool; DIG_CELLS] {
    let mut rng = ChaCha8Rng::seed_from_u64(seed);
    let mut idx: Vec<usize> = (0..DIG_CELLS).collect();
    for i in 0..DIG_TREASURES {
        let j = i + rng.random_range(0..=(DIG_CELLS - 1 - i));
        idx.swap(i, j);
    }
    let mut out = [false; DIG_CELLS];
    for at in idx.iter().take(DIG_TREASURES) {
        out[*at] = true;
    }
    out
}

/// 一只气球的样子。`scattered` = 开局那一批（散布在整个玩法区里），否则是从底部升起来的新气球。
fn balloon_look(rng: &mut ChaCha8Rng, scattered: bool) -> (f32, f32, f32, f32, Color) {
    let size = rng.random_range(BALLOON_SIZE[0]..=BALLOON_SIZE[1]);
    let x = rng.random_range(0.0..=(field_w() - size));
    let y = if scattered {
        rng.random_range(0.0..=field_h())
    } else {
        -size
    };
    let vy = rng.random_range(BALLOON_RISE[0]..=BALLOON_RISE[1]);
    let color = col::BALLOONS[rng.random_range(0..col::BALLOONS.len())];
    (x, y, vy, size, color)
}

/// 一件掉下来的东西：八成是财宝，两成是石头。
fn fall_look(rng: &mut ChaCha8Rng) -> (f32, f32, bool) {
    let x = rng.random_range(0.0..=(field_w() - FALL_SIZE));
    let vy = rng.random_range(FALL_SPEED[0]..=FALL_SPEED[1]);
    (x, vy, rng.random_range(0.0..1.0) < TREASURE_ODDS)
}

/// 这一款怎么玩、满分多少 —— 玩法区上面那一行。
fn how_to_play(game: GameKind, payout: Payout) -> String {
    match game {
        GameKind::PopBalloons => format!(
            "点掉升起来的气球，一只一分 · {} 秒 · 满分 {} 只（每分 {} 点券）",
            ROUND_SECS as u32, payout.perfect, payout.per_score
        ),
        GameKind::CatchTreasure => format!(
            "← → 或 A D 移动篮子（鼠标也行）· 财宝 +1、石头 −1 · {} 秒 · 满分 {} 件（每分 {} 点券）",
            ROUND_SECS as u32, payout.perfect, payout.per_score
        ),
        GameKind::PenguinDig => format!(
            "点一格下铲 · {DIG_CELLS} 格里埋着 {DIG_TREASURES} 件宝，只有 {DIG_BUDGET} 铲 · 满分 {} 件（每分 {} 点券）",
            payout.perfect, payout.per_score
        ),
    }
}

/// 成绩那一行的动词。三种玩法的单位不同（只／件），写出来才读得懂「12」是十二只还是十二件。
fn score_word(game: GameKind) -> &'static str {
    match game {
        GameKind::PopBalloons => "点掉",
        GameKind::CatchTreasure => "接到",
        GameKind::PenguinDig => "挖到",
    }
}

/// 结算屏那一行。练习局与对局只差后半句，而**那半句就是 `awarded` 那位的人话**：
/// 「值 N 点券」与「换 N 点券」读的是同一个数，区别只在这笔钱发不发。
fn result_line(round: &Round) -> String {
    let game = round.wanted.game();
    let points = round.payout.points(round.score);
    format!(
        "{} {} / 满分 {} · {}",
        score_word(game),
        round.score,
        round.payout.perfect,
        if round.wanted.practice() {
            format!("练习局不计点券（这一局值 {points} 点券）")
        } else {
            format!("换 {points} 点券")
        }
    )
}

// ===== 建屏 =====

/// 「这一屏该是哪一局」要问的五样。打包是房规（§六 坑 8）：五位散装参数撞的是
/// `too_many_arguments`，而它们回答的其实是同一个问题。
#[derive(SystemParam)]
pub(crate) struct ArcadeWish<'w> {
    scene: Res<'w, State<Scene>>,
    pending: Res<'w, PendingDecision>,
    /// 对局里那一手的布局种子（core 发下来的）
    deal: Res<'w, ArcadeDeal>,
    practice: Res<'w, Practice>,
    rounds: Res<'w, PracticeRounds>,
}

impl ArcadeWish<'_> {
    /// 对局里的窗口优先（它挂着回合）；没有窗口才轮到菜单里的练习请求。
    ///
    /// 场景那一道门是防呆：回菜单时 `clear_match` 会清 `PendingDecision`，
    /// 但「清场」与「切场景」不同帧，多问一句比赌顺序便宜。
    fn wanted(&self) -> Option<Wanted> {
        match self.pending.active() {
            Some((player, Pending::Minigame(game))) if *self.scene.get() == Scene::Board => {
                Some(Wanted::Match {
                    player,
                    game,
                    seed: self.deal.seed,
                })
            }
            _ => self.practice.0.map(|game| Wanted::Practice {
                game,
                rounds: self.rounds.0,
            }),
        }
    }
}

/// 「现在该不该有一块游乐场屏」。跟着 [`PendingDecision`]（对局）与 [`Practice`]（菜单）走。
///
/// **不受 `ActiveBoard` 门禁**（不在 `UiSet::Live` 里）：练习局在主菜单上，那里没有棋盘
/// —— 与派发、悬停两条同一条理由（§6.1 坑 23：挂进 Live 集，菜单上的东西就点不动了）。
pub(crate) fn sync_arcade(
    mut commands: Commands,
    font: Res<DevFont>,
    rules: Res<GameRules>,
    wish: ArcadeWish<'_>,
    mut arcade: ResMut<Arcade>,
    panels: Query<Entity, With<ArcadePanel>>,
) {
    let wanted = wish.wanted();
    if arcade.0.as_ref().map(|r| r.wanted) == wanted {
        return;
    }
    for entity in &panels {
        commands.entity(entity).despawn();
    }
    arcade.0 = wanted.map(|w| start(&mut commands, &font, &rules, w));
}

/// 建一屏并把这一局的状态摆好。三种玩法共用外壳（标题、说明、分数、秒数、玩法区、底栏），
/// 差别只在玩法区里长什么。
fn start(commands: &mut Commands, font: &DevFont, rules: &GameRules, wanted: Wanted) -> Round {
    let game = wanted.game();
    let practice = wanted.practice();
    let payout = rules.minigames.of(game);
    let mut rng = ChaCha8Rng::seed_from_u64(wanted.seed());

    let (overlay_node, overlay_bg, _) = overlay(5);
    let (panel_node, panel_transform, panel_bg, panel_border) = panel(
        layout::ARCADE_PANEL[0],
        layout::ARCADE_PANEL,
        col::BORDER_ARCADE,
    );
    let mut root = commands.spawn((ArcadePanel, overlay_node, overlay_bg, ZIndex(5)));
    // 练习局挂在菜单上（随 `MenuPiece` 收走），对局里挂在棋盘 UI 上（随 `BoardUi` 收走）。
    // 两个标记必须给对：给错了这一屏会活过场景切换，而它是**唯一**一块两个场景都要用的面板。
    // 分成两句写是因为两个标记是不同的类型，`if … else` 给不出一个共同的 bundle
    if practice {
        root.insert(MenuPiece);
    } else {
        root.insert(BoardUi);
    }
    // 菜单那块背景压在 `GlobalZIndex(10)` 上（`menu.rs`），所以练习这一屏要跳到全局层才看得见；
    // 对局里**不跳**：暂停菜单在 `ZIndex(6)`，ESC 得压得住游乐场，而游戏钟确实会跟着停
    // （`step_arcade` 读 `Paused`）。同一块面板两种层级，是「两个场景都要用」的代价
    if practice {
        root.insert(GlobalZIndex(11));
    }
    let root = root.id();
    let panel = commands
        .spawn((
            ChildOf(root),
            panel_node,
            panel_transform,
            panel_bg,
            panel_border,
        ))
        .id();

    let title = if practice {
        format!("游乐场 · {}（练习局）", game.label())
    } else {
        format!("游乐场 · {}", game.label())
    };
    commands.entity(panel).with_children(|p| {
        p.spawn(centered(font, title, size::DIALOG_TITLE, col::TITLE_GOLD));
        p.spawn(centered(
            font,
            if practice {
                "练习局 · 不计点券（结算时会写出这一局值多少）"
            } else {
                "成绩换点券，玩完才继续"
            },
            size::ROW,
            if practice { col::JAILED } else { col::MUTED },
        ));
        p.spawn(centered(
            font,
            how_to_play(game, payout),
            size::ROW,
            col::KEY,
        ));
    });
    // 分数与秒数那两行开局是空的，逐帧就地写
    let score_text = blank_line(commands, font, panel, size::NAME, col::HINT);
    let clock_text = blank_line(commands, font, panel, size::BODY, col::PHASE);

    // 玩法区。`row_gap` 只有挖宝那一套 flex 行用得上（气球与财宝是绝对定位，不看它）。
    // **必须裁切**：气球从底部进场时 `bottom` 是负的、财宝落到地面以下时也是，而 bevy_ui
    // 默认不裁子节点 —— 不裁的话那几只会画在玩法区外面、压在 HUD 上（第一张冒烟图就是）
    let field = commands
        .spawn((
            ChildOf(panel),
            Node {
                width: Val::Px(field_w()),
                height: Val::Px(field_h()),
                flex_shrink: 0.0,
                // 挖宝那四行要竖着叠（气球与财宝是绝对定位，不受这一位影响）
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                row_gap: Val::Px(layout::ARCADE_CELL_GAP),
                overflow: Overflow::clip(),
                ..default()
            },
            BackgroundColor(col::FIELD),
            border_color(col::BTN_BORDER_DISABLED),
        ))
        .id();
    let play = match game {
        GameKind::PopBalloons => build_balloons(commands, &mut rng, field),
        GameKind::CatchTreasure => build_catch(commands, &mut rng, field),
        GameKind::PenguinDig => build_dig(commands, font, wanted.seed(), field),
    };

    // 结算屏：开局就建好、藏起来（`Display::None`），收工时翻出来。不等到那一刻再建 ——
    // 那一帧要同时改分数、改按钮、改文字，而按钮是哪两颗由「练习还是对局」决定，
    // 那是建屏时就知道的事
    let result_box = commands
        .spawn((
            ChildOf(panel),
            Node {
                display: Display::None,
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                align_items: AlignItems::Center,
                row_gap: Val::Px(12.0),
                padding: UiRect::vertical(Val::Px(24.0)),
                ..default()
            },
        ))
        .id();
    let result_text = blank_line(
        commands,
        font,
        result_box,
        size::CARD_TITLE,
        col::TITLE_GOLD,
    );
    let result_buttons = commands
        .spawn((
            ChildOf(result_box),
            Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                column_gap: Val::Px(layout::BTN_ROW_GAP),
                ..default()
            },
        ))
        .id();
    commands.entity(result_buttons).with_children(|line| {
        if practice {
            button(
                line,
                &font.0,
                ButtonSpec::new(
                    "再来一局",
                    layout::ARCADE_BUTTON,
                    PRIMARY,
                    UiAction::ArcadeReplay,
                ),
            );
            button(
                line,
                &font.0,
                ButtonSpec::new(
                    "离开",
                    layout::ARCADE_BUTTON,
                    SECONDARY,
                    UiAction::ArcadeLeave,
                ),
            );
        } else {
            button(
                line,
                &font.0,
                ButtonSpec::new(
                    "收下",
                    layout::ARCADE_BUTTON,
                    PRIMARY,
                    UiAction::ArcadeCollect,
                ),
            );
        }
    });

    // 底栏：玩的过程中那颗「提前收工」。收工时整条藏起来（结算屏上有别的按钮）
    let footer = commands
        .spawn((
            ChildOf(panel),
            Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
        ))
        .id();
    commands.entity(footer).with_children(|line| {
        button(
            line,
            &font.0,
            ButtonSpec::new(
                "提前收工",
                layout::ARCADE_BUTTON,
                SECONDARY,
                UiAction::ArcadeQuit,
            ),
        );
    });

    Round {
        wanted,
        payout,
        rng,
        score: 0,
        left: ROUND_SECS,
        over: false,
        over_shown: false,
        play,
        field,
        result_box,
        footer,
        score_text,
        clock_text,
        result_text,
    }
}

/// 一行居中的文字。`dialog::center_label` 是私有的，而这一屏要的行数比模态多，
/// 所以这里自己拼一份（同一形状：节点拉满宽、靠 `Justify::Center` 让文字在行内居中）。
fn centered(
    font: &DevFont,
    text: impl Into<String>,
    px: f32,
    color: Color,
) -> (Node, TextLayout, Text, TextFont, TextColor) {
    let (t, f, c) = label(&font.0, text, px, color);
    (
        Node {
            width: Val::Percent(100.0),
            ..default()
        },
        TextLayout::justify(Justify::Center),
        t,
        f,
        c,
    )
}

/// 一行等着被逐帧填的居中文字。
///
/// **字体不能漏**：`TextFont::default()` 退回 Fira Mono 子集，中文会变成豆腐块（§6.1 坑 2）——
/// 而挖宝那 16 个洞里写的正是「宝」，漏了字体的症状是「铲开全是方块」。
fn blank_line(
    commands: &mut Commands,
    font: &DevFont,
    parent: Entity,
    px: f32,
    color: Color,
) -> Entity {
    commands
        .spawn((ChildOf(parent), centered(font, String::new(), px, color)))
        .id()
}

fn build_balloons(commands: &mut Commands, rng: &mut ChaCha8Rng, field: Entity) -> Play {
    let slots = (0..BALLOONS)
        .map(|slot| {
            let (x, y, vy, size, color) = balloon_look(rng, true);
            let node = commands
                .spawn((
                    ChildOf(field),
                    balloon_node(x, y, size, color),
                    UiAction::Pop { slot: slot as u8 },
                    // 悬停亮一档：在近黑的玩法区里，「点得到」得看得出来
                    Palette {
                        normal: color,
                        hover: brighten(color),
                    },
                ))
                .id();
            Balloon {
                x,
                y,
                vy,
                size,
                node,
            }
        })
        .collect();
    Play::Balloons { slots }
}

fn balloon_node(x: f32, y: f32, size: f32, color: Color) -> (Node, BackgroundColor) {
    (
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(x),
            bottom: Val::Px(y),
            width: Val::Px(size),
            height: Val::Px(size),
            border_radius: BorderRadius::all(Val::Px(size / 2.0)),
            ..default()
        },
        BackgroundColor(color),
    )
}

fn build_catch(commands: &mut Commands, rng: &mut ChaCha8Rng, field: Entity) -> Play {
    let basket_node = commands
        .spawn((
            ChildOf(field),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px((field_w() - BASKET_W) / 2.0),
                bottom: Val::Px(8.0),
                width: Val::Px(BASKET_W),
                height: Val::Px(BASKET_H),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            },
            BackgroundColor(col::BASKET),
            // 篮子不点：它是被移动的那个，点它什么也不该发生
            Pickable::IGNORE,
        ))
        .id();
    // 开局先落两件，免得头 0.7 秒是一块空地（那一秒玩家正在找篮子在哪）
    let falls = (0..2)
        .map(|_| spawn_fall(commands, rng, field, field_h() * 0.6))
        .collect();
    Play::Catch {
        basket: field_w() / 2.0,
        cursor: None,
        falls,
        spawn_in: FALL_EVERY,
        basket_node,
    }
}

fn spawn_fall(commands: &mut Commands, rng: &mut ChaCha8Rng, field: Entity, at: f32) -> Fall {
    let (x, vy, treasure) = fall_look(rng);
    let node = commands
        .spawn((
            ChildOf(field),
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(x),
                bottom: Val::Px(at),
                width: Val::Px(FALL_SIZE),
                height: Val::Px(FALL_SIZE),
                // 财宝是方的、石头是圆的：颜色之外再给一个形状，色弱也分得开哪件该躲
                border_radius: BorderRadius::all(Val::Px(if treasure { 4.0 } else { 13.0 })),
                ..default()
            },
            BackgroundColor(if treasure { col::TREASURE } else { col::ROCK }),
            Pickable::IGNORE,
        ))
        .id();
    Fall {
        x,
        y: at,
        vy,
        treasure,
        node,
    }
}

fn build_dig(commands: &mut Commands, font: &DevFont, seed: u64, field: Entity) -> Play {
    let deal = dig_deal(seed);
    // 4 行 × 4 格用 flex 排而不是绝对定位：格子不动，动的只有颜色与那一个字
    let rows: Vec<Entity> = (0..DIG_SIDE)
        .map(|_| {
            commands
                .spawn((
                    ChildOf(field),
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(layout::ARCADE_CELL_GAP),
                        ..default()
                    },
                ))
                .id()
        })
        .collect();
    let mut cells = Vec::with_capacity(DIG_CELLS);
    for (at, &treasure) in deal.iter().enumerate() {
        let node = commands
            .spawn((
                ChildOf(rows[at / DIG_SIDE]),
                Node {
                    width: Val::Px(layout::ARCADE_CELL),
                    height: Val::Px(layout::ARCADE_CELL),
                    justify_content: JustifyContent::Center,
                    align_items: AlignItems::Center,
                    border: UiRect::all(Val::Px(layout::BTN_BORDER)),
                    border_radius: BorderRadius::all(Val::Px(layout::BTN_RADIUS)),
                    ..default()
                },
                BackgroundColor(col::ICE),
                border_color(col::BTN_BORDER),
                UiAction::Dig { cell: at as u8 },
                Palette {
                    normal: col::ICE,
                    hover: brighten(col::ICE),
                },
            ))
            .id();
        // 没铲时是一块空冰面，铲开才写「宝」或「·」
        let mark = commands
            .spawn((
                ChildOf(node),
                label(&font.0, "", size::DIALOG_TITLE, col::TREASURE),
            ))
            .id();
        cells.push(Cell {
            treasure,
            dug: false,
            painted: false,
            node,
            mark,
        });
    }
    Play::Dig {
        cells,
        digs: DIG_BUDGET,
    }
}

// ===== 玩法内的点击 =====

/// 点掉第 `slot` 只气球。派发那一侧调它（`UiAction::Pop`）—— 与浮层的开关同一条分工：
/// 按钮只声明自己代表什么，「这一下算不算」由状态说了算。
///
/// 不在这里换新的：换一只新气球要摇随机（大小、位置、速度、颜色），而随机挂在 `Round` 上，
/// 这一侧手里只有它自己。所以这里只把它**顶出玩法区**，下一帧 `step_arcade` 看见就换新的。
pub(crate) fn pop(arcade: &mut Arcade, slot: u8) {
    let Some(round) = arcade.0.as_mut() else {
        return;
    };
    if round.over {
        return;
    }
    let Play::Balloons { slots } = &mut round.play else {
        return;
    };
    let Some(balloon) = slots.get_mut(slot as usize) else {
        return;
    };
    round.score += 1;
    balloon.y = field_h() + balloon.size;
}

/// 往第 `cell` 格下铲。铲过的格子再点不算（那把铲已经用掉了）。
pub(crate) fn dig(arcade: &mut Arcade, cell: u8) {
    let Some(round) = arcade.0.as_mut() else {
        return;
    };
    if round.over {
        return;
    }
    let Play::Dig { cells, digs } = &mut round.play else {
        return;
    };
    let Some(cell) = cells.get_mut(cell as usize) else {
        return;
    };
    if cell.dug || *digs == 0 {
        return;
    }
    cell.dug = true;
    *digs -= 1;
    if cell.treasure {
        round.score += 1;
    }
    if *digs == 0 {
        round.over = true;
    }
}

/// 提前收工：把当前分数送去做结算，**不直接提交**（结算屏上那颗「收下」才提交）。
pub(crate) fn quit_early(arcade: &mut Arcade) {
    if let Some(round) = arcade.0.as_mut() {
        round.over = true;
    }
}

/// 结算屏上那颗「收下」要报的那一份成绩。练习局返回 `None` —— 它不发（理由见模块头）。
pub(crate) fn result_of(arcade: &Arcade) -> Option<MinigameResult> {
    let round = arcade.0.as_ref()?;
    if !round.over || round.wanted.practice() {
        return None;
    }
    Some(MinigameResult {
        player: round.wanted.player()?,
        game: round.wanted.game(),
        score: round.score,
        awarded: true,
    })
}

// ===== 逐帧推进 =====

/// 推进这一局并就地刷画面。**它不重建任何东西**（除了接财宝里掉出去的那些）：
/// 三种玩法每帧都在动，而「签名变了才重建」那一套是给「一屏里几颗按钮」用的，
/// 不是给 60 帧的动画用的。
pub(crate) fn step_arcade(
    mut commands: Commands,
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    // 0.20 的 `Window` 是**组件**不是资源（§六 的 API 事实之一）。菜单里也有这一屏
    // （练习局），而无头跑的测试里没有窗口实体 —— 所以是 `Option`：缺了它只走键盘那半条
    windows: Query<&Window>,
    paused: Res<Paused>,
    mut arcade: ResMut<Arcade>,
    mut paint: Paint<'_, '_>,
) {
    let Some(round) = arcade.0.as_mut() else {
        return;
    };
    // 收工那一刻把结算屏摆出来（藏玩法区、露那一块、写成绩）。分两帧是因为「收工」
    // 由派发那一侧写下，而摆屏要动节点，只有这一侧手里有查询
    if round.over && !round.over_shown {
        show_result(round, &mut paint);
        return;
    }
    if round.over || paused.0 {
        return;
    }
    let dt = time.delta_secs().min(MAX_DT);
    let timed = !matches!(round.play, Play::Dig { .. });

    match &mut round.play {
        Play::Balloons { slots } => {
            let mut refill = Vec::new();
            for (at, balloon) in slots.iter_mut().enumerate() {
                balloon.y += balloon.vy * dt;
                if balloon.y > field_h() {
                    refill.push(at);
                }
                paint_balloon(&mut paint, balloon);
            }
            // 顶出去的那几只换新的：大小、位置、速度、颜色都重新摇一手（用它自己那条流）
            for at in refill {
                let (x, y, vy, size, color) = balloon_look(&mut round.rng, false);
                let balloon = &mut slots[at];
                *balloon = Balloon {
                    x,
                    y,
                    vy,
                    size,
                    node: balloon.node,
                };
                // 三个查询各取一次：同一只气球要改尺寸、底色与悬停色，而一个 `get_mut`
                // 只给得出一格组件
                if let Ok(mut node) = paint.nodes.get_mut(balloon.node) {
                    node.width = Val::Px(size);
                    node.height = Val::Px(size);
                    node.border_radius = BorderRadius::all(Val::Px(size / 2.0));
                }
                if let Ok(mut bg) = paint.bgs.get_mut(balloon.node) {
                    bg.0 = color;
                }
                if let Ok(mut palette) = paint.palettes.get_mut(balloon.node) {
                    *palette = Palette {
                        normal: color,
                        hover: brighten(color),
                    };
                }
                paint_balloon(&mut paint, balloon);
            }
        }
        Play::Catch {
            basket,
            cursor,
            falls,
            spawn_in,
            basket_node,
        } => {
            move_basket(basket, cursor, &keys, windows.iter().next(), dt);
            if let Ok(mut node) = paint.nodes.get_mut(*basket_node) {
                node.left = Val::Px(*basket - BASKET_W / 2.0);
            }
            *spawn_in -= dt;
            let mut score = round.score;
            let mut refill = 0usize;
            falls.retain_mut(|fall| {
                fall.y -= fall.vy * dt;
                // 接住判定：落到篮子那一条高度上、且横向压着篮子。用两个半宽之和，
                // 于是「看着擦到边」也算接住 —— 这一类玩法手感差一点就像坏了
                let caught = fall.y <= BASKET_H + 8.0
                    && (fall.x + FALL_SIZE / 2.0 - *basket).abs() <= (BASKET_W + FALL_SIZE) / 2.0;
                if caught {
                    score = if fall.treasure {
                        score + 1
                    } else {
                        score.saturating_sub(1)
                    };
                    refill += 1;
                    return false;
                }
                if fall.y < -FALL_SIZE {
                    refill += 1;
                    return false;
                }
                if let Ok(mut node) = paint.nodes.get_mut(fall.node) {
                    node.bottom = Val::Px(fall.y);
                }
                true
            });
            round.score = score;
            for _ in 0..refill {
                let fall = spawn_fall(&mut commands, &mut round.rng, round.field, field_h());
                falls.push(fall);
            }
            if *spawn_in <= 0.0 {
                *spawn_in += FALL_EVERY;
                let fall = spawn_fall(&mut commands, &mut round.rng, round.field, field_h());
                falls.push(fall);
            }
        }
        Play::Dig { cells, digs } => {
            for cell in cells.iter_mut() {
                if !cell.dug || cell.painted {
                    continue;
                }
                cell.painted = true;
                let (bg, ink) = if cell.treasure {
                    (col::DUG_TREASURE, col::FIELD)
                } else {
                    (col::ICE_DUG, col::KEY)
                };
                if let Ok(mut color) = paint.bgs.get_mut(cell.node) {
                    color.0 = bg;
                }
                if let Ok(mut palette) = paint.palettes.get_mut(cell.node) {
                    // 铲过的格子按不动了，就该长成按不动的样子（灰按钮那一条纪律的挖宝版）
                    *palette = Palette {
                        normal: bg,
                        hover: bg,
                    };
                }
                if let Ok(mut text) = paint.texts.get_mut(cell.mark) {
                    text.0 = if cell.treasure { "宝" } else { "·" }.to_string();
                }
                if let Ok(mut color) = paint.inks.get_mut(cell.mark) {
                    color.0 = ink;
                }
                commands.entity(cell.node).insert(Disabled);
            }
            // 挖宝不看秒数：把「还剩几铲」写进这一格，于是下面那两行状态有个统一来源
            round.left = *digs as f32;
        }
    }

    if timed {
        round.left -= dt;
        if round.left <= 0.0 {
            round.left = 0.0;
            round.over = true;
        }
    }
    write_status(round, &mut paint.texts);
}

fn paint_balloon(paint: &mut Paint<'_, '_>, balloon: &Balloon) {
    if let Ok(mut node) = paint.nodes.get_mut(balloon.node) {
        node.left = Val::Px(balloon.x);
        node.bottom = Val::Px(balloon.y);
    }
}

/// 篮子怎么动：鼠标指哪到哪（**只在它真动了的那一帧**接管，否则键盘永远抢不回来），
/// 键盘按秒推。
fn move_basket(
    basket: &mut f32,
    cursor: &mut Option<Vec2>,
    keys: &ButtonInput<KeyCode>,
    window: Option<&Window>,
    dt: f32,
) {
    if let Some(window) = window
        && let Some(at) = window.cursor_position()
        && *cursor != Some(at)
    {
        *cursor = Some(at);
        // 玩法区恰好横向居中（面板宽 = 玩法区宽 + 两侧内边距，见 `layout::ARCADE_PANEL`），
        // 所以窗口横坐标减去两边的留白就是它在这一块里的位置 —— 不必去读布局之后的
        // `UiTransform`，那玩意儿的原点与这一屏要的读数差着一层相机
        let margin = (window.width() - field_w()) / 2.0;
        *basket = (at.x - margin).clamp(0.0, field_w());
    }
    let mut dx = 0.0;
    if keys.pressed(KeyCode::ArrowLeft) || keys.pressed(KeyCode::KeyA) {
        dx -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) || keys.pressed(KeyCode::KeyD) {
        dx += 1.0;
    }
    if dx != 0.0 {
        *basket = (*basket + dx * BASKET_SPEED * dt).clamp(0.0, field_w());
    }
}

/// 分数与秒数那两行。只在字符串真变了的时候赋值 —— 每帧重写同一个串会让排版每帧重来。
fn write_status(round: &Round, texts: &mut Query<&mut Text>) {
    write_text(
        texts,
        round.score_text,
        format!(
            "{} {} / 满分 {} · 此刻值 {} 点券",
            score_word(round.wanted.game()),
            round.score,
            round.payout.perfect,
            round.payout.points(round.score)
        ),
    );
    let clock = match &round.play {
        Play::Dig { digs, .. } => format!("还剩 {digs} 铲"),
        _ => format!("剩 {:.0} 秒", round.left.max(0.0)),
    };
    write_text(texts, round.clock_text, clock);
}

fn write_text(texts: &mut Query<&mut Text>, entity: Entity, want: String) {
    if let Ok(mut text) = texts.get_mut(entity)
        && text.0 != want
    {
        text.0 = want;
    }
}

/// 收工：藏玩法区与底栏、露结算屏、把成绩写成人话。
fn show_result(round: &mut Round, paint: &mut Paint<'_, '_>) {
    round.over_shown = true;
    if let Ok(mut node) = paint.nodes.get_mut(round.field) {
        node.display = Display::None;
    }
    if let Ok(mut node) = paint.nodes.get_mut(round.footer) {
        node.display = Display::None;
    }
    if let Ok(mut node) = paint.nodes.get_mut(round.result_box) {
        node.display = Display::Flex;
    }
    write_text(&mut paint.texts, round.result_text, result_line(round));
}

/// 悬停亮一档。`Palette` 只有两格颜色，而气球与冰洞的底色是随机的，
/// 所以「亮一档」得从底色算出来 —— 与 `render::darkened` 同一条思路，方向相反。
fn brighten(c: Color) -> Color {
    let s = c.to_srgba();
    let up = |v: f32| (v + (1.0 - v) * 0.35).min(1.0);
    Color::srgba(up(s.red), up(s.green), up(s.blue), s.alpha)
}

// ===== 主菜单那块「选一款练习」 =====

/// 菜单上那块选择屏。签名法与其余浮层同一条：可见内容只由「哪几款、各自的表」决定，
/// 两样都不在对局里变，所以签名就是那三段文字本身。
pub(crate) fn sync_choice(
    mut commands: Commands,
    font: Res<DevFont>,
    rules: Res<GameRules>,
    open: Res<ArcadeChoiceOpen>,
    practice: Res<Practice>,
    shown: Query<(Entity, &ChoiceStamp), With<ArcadeChoice>>,
) {
    // 练习已经开起来了就把选择屏收走：两块都压在全局层 11 上，留着就是两层一样的板子
    if !open.0 || practice.0.is_some() {
        for (entity, _) in &shown {
            commands.entity(entity).despawn();
        }
        return;
    }
    let stamp = GameKind::ALL
        .iter()
        .map(|g| {
            let p = rules.minigames.of(*g);
            format!("{}:{}:{}", g.label(), p.perfect, p.per_score)
        })
        .collect::<Vec<_>>()
        .join("|");
    if shown.iter().any(|(_, s)| s.0 == stamp) {
        return;
    }
    for (entity, _) in &shown {
        commands.entity(entity).despawn();
    }

    let (overlay_node, overlay_bg, _) = overlay(5);
    let (panel_node, panel_transform, panel_bg, panel_border) = panel(
        layout::ARCADE_CHOICE[0],
        layout::ARCADE_CHOICE,
        col::BORDER_ARCADE,
    );
    let root = commands
        .spawn((
            MenuPiece,
            ArcadeChoice,
            ChoiceStamp(stamp),
            overlay_node,
            overlay_bg,
            ZIndex(5),
            // 菜单背景在 `GlobalZIndex(10)` 上，不跳到全局层就永远压在它后面（黑屏一块）
            GlobalZIndex(11),
        ))
        .id();
    let body = commands
        .spawn((
            ChildOf(root),
            panel_node,
            panel_transform,
            panel_bg,
            panel_border,
        ))
        .id();
    commands.entity(body).with_children(|p| {
        p.spawn(centered(
            &font,
            "游乐场 · 练习",
            size::DIALOG_TITLE,
            col::TITLE_GOLD,
        ));
        p.spawn(centered(
            &font,
            "练习局不给点券（4 代原规则）。踩到游乐场那一格才算数。",
            size::ROW,
            col::MUTED,
        ));
        p.spawn(separator());
        for game in GameKind::ALL {
            let payout = rules.minigames.of(game);
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(10.0),
                ..default()
            })
            .with_children(|line| {
                line.spawn(Node {
                    flex_grow: 1.0,
                    flex_direction: FlexDirection::Column,
                    row_gap: Val::Px(2.0),
                    ..default()
                })
                .with_children(|stack| {
                    stack.spawn(label(&font.0, game.label(), size::NAME, col::HINT));
                    stack.spawn(label(
                        &font.0,
                        how_to_play(game, payout),
                        size::SMALL,
                        col::KEY,
                    ));
                    stack.spawn(label(
                        &font.0,
                        format!("打满 {} 点券", payout.best()),
                        size::SMALL,
                        col::GAIN,
                    ));
                });
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "开始",
                        layout::ARCADE_CHOICE_BUTTON,
                        PRIMARY,
                        UiAction::Practice(game),
                    ),
                );
            });
        }
        p.spawn(separator());
        p.spawn(Node {
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|line| {
            button(
                line,
                &font.0,
                ButtonSpec::new(
                    "返回",
                    layout::ARCADE_CHOICE_BUTTON,
                    SECONDARY,
                    UiAction::ToggleArcadeChoice,
                ),
            );
        });
    });
}

#[derive(Component)]
pub(crate) struct ChoiceStamp(String);

#[cfg(test)]
mod tests {
    use super::*;

    /// 那一手宝：数量对、可复现、且换个种子就不是同一手。
    /// 第三条是这条用例的牙 —— 少了它，「16 格全埋宝」也能过前两条。
    #[test]
    fn the_dig_deal_hides_exactly_the_right_number() {
        for seed in [0u64, 1, 7, 99] {
            let deal = dig_deal(seed);
            assert_eq!(
                deal.iter().filter(|t| **t).count(),
                DIG_TREASURES,
                "种子 {seed} 埋的宝不是 {DIG_TREASURES} 件"
            );
            assert_eq!(deal.len(), DIG_CELLS);
            assert_eq!(dig_deal(seed), deal, "同一个种子必须是同一手");
        }
        assert_ne!(dig_deal(1), dig_deal(2), "换种子还是同一手说明没读种子");
    }

    /// 满分那一档要**可能**拿到：宝比铲多就永远挖不满，那张表的满分就成了装饰。
    #[test]
    fn a_lucky_run_can_find_them_all() {
        assert!(
            (DIG_TREASURES as u32) <= DIG_BUDGET,
            "铲数比宝还少，满分那一档永远拿不到"
        );
        let deal = dig_deal(3);
        assert_eq!(deal.iter().filter(|t| **t).count(), DIG_TREASURES);
    }

    /// 气球与财宝的落点必须留在玩法区里：越界的东西点不到（命中测试只看节点矩形），
    /// 而症状是「明明点中了却没爆」。
    #[test]
    fn spawns_stay_inside_the_field() {
        let mut rng = ChaCha8Rng::seed_from_u64(11);
        for _ in 0..400 {
            let (x, _, _, size, _) = balloon_look(&mut rng, true);
            assert!(x >= 0.0 && x + size <= field_w(), "气球出界：{x} + {size}");
            let (x, _, _) = fall_look(&mut rng);
            assert!(
                x >= 0.0 && x + FALL_SIZE <= field_w(),
                "掉下来的那件出界：{x}"
            );
        }
    }

    /// 财宝与石头两样都要真出现：`TREASURE_ODDS` 写成 1.0 或 0.0 的话，
    /// 「接到石头倒扣一分」那条规则就永远玩不到、也永远测不到。
    #[test]
    fn both_kinds_of_falling_thing_show_up() {
        let mut rng = ChaCha8Rng::seed_from_u64(5);
        let (mut treasures, mut rocks) = (0, 0);
        for _ in 0..300 {
            if fall_look(&mut rng).2 {
                treasures += 1;
            } else {
                rocks += 1;
            }
        }
        assert!(treasures > 0 && rocks > 0, "财宝 {treasures} 石头 {rocks}");
        assert!(
            treasures > rocks,
            "财宝该是多数（{treasures} 对 {rocks}）：接错一件是罚，不该是常态"
        );
    }

    /// 三种玩法的说明各说各的，而且都带上了那一档表里的两个数 —— 玩家在开局前就该知道
    /// 「满分是多少、每一分值几点券」。**不断言说明里写着游戏名**：那一屏的标题上已经有了
    /// （`start` 拼的「游乐场 · 打气球」），说明行再写一遍是重复。
    #[test]
    fn every_game_explains_itself_and_names_its_payout() {
        let rules = GameRules::default();
        let mut lines = Vec::new();
        for game in GameKind::ALL {
            let payout = rules.minigames.of(game);
            let line = how_to_play(game, payout);
            assert!(
                line.contains(&payout.perfect.to_string()),
                "{line} 没写出满分 {}",
                payout.perfect
            );
            assert!(
                line.contains(&payout.per_score.to_string()),
                "{line} 没写出每一分值 {} 点券",
                payout.per_score
            );
            lines.push(line);
        }
        lines.sort();
        lines.dedup();
        assert_eq!(
            lines.len(),
            GameKind::ALL.len(),
            "有两种玩法说了同一句话：{lines:?}"
        );
    }

    /// 结算那一行是 `awarded` 那位的人话：同一个分数，练习局说「值」、对局说「换」，
    /// 而两个数必须相等 —— 否则「练习局告诉你的数」与「真玩一把拿到的数」就对不上了。
    #[test]
    fn the_result_line_only_differs_by_whether_it_pays() {
        let rules = GameRules::default();
        for game in GameKind::ALL {
            let payout = rules.minigames.of(game);
            let line = |practice: bool| {
                result_line(&Round {
                    wanted: if practice {
                        Wanted::Practice { game, rounds: 1 }
                    } else {
                        Wanted::Match {
                            player: Entity::PLACEHOLDER,
                            game,
                            seed: 1,
                        }
                    },
                    payout,
                    rng: ChaCha8Rng::seed_from_u64(0),
                    score: payout.perfect,
                    left: 0.0,
                    over: true,
                    over_shown: false,
                    play: Play::Dig {
                        cells: Vec::new(),
                        digs: 0,
                    },
                    field: Entity::PLACEHOLDER,
                    result_box: Entity::PLACEHOLDER,
                    footer: Entity::PLACEHOLDER,
                    score_text: Entity::PLACEHOLDER,
                    clock_text: Entity::PLACEHOLDER,
                    result_text: Entity::PLACEHOLDER,
                })
            };
            let best = payout.best().to_string();
            assert!(line(true).contains("练习局不计点券"), "{}", line(true));
            assert!(
                line(false).contains(&format!("换 {} 点券", best)),
                "{}",
                line(false)
            );
            assert!(
                line(true).contains(&format!("值 {} 点券", best)),
                "练习局报出来的数该与真玩一把拿到的一样：{}",
                line(true)
            );
        }
    }
}
