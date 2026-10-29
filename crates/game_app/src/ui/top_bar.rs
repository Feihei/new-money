//! 顶栏：提示行、阶段标签、掷骰与返回主菜单。
//!
//! 对应 Godot `board.tscn:33-67` 的 `TopBar` 与 `board.gd:548-583`（`_on_phase_changed`）、
//! `:537-538,668-679`（事件提示）。
//!
//! 提示行有两条来源：**阶段旁白**（谁的回合、移动中）与**事件流水**（服刑跳过、来到银行……）。
//! 两条分开写、事件覆盖阶段，是因为它们抢同一行文字：阶段跳变与消息送达同帧发生，
//! 让更具体的那条赢，才不会把「在监狱中（剩 2 回合）」这种一次性通知盖掉。
//! 买地/升级/租金/卡面**不进事件流水** —— 模态框就在屏幕中央说同一件事，
//! Godot 端那行提示被紧接着的「X 的回合」冲掉，本来也读不到（§5.1 差异表）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, AtmAccess, AuctionFailed, AuctionStarted, AuctionWon, BankOpDone, BankVisited,
    BidPlaced, BidWithdrawn, BombTransferred, CardLost, CardPlayed, CardSnatched, ChairmanTook,
    ChestOpened, CompanyCharged, Controller, DepositsRobbed, DetentionLifted, DetentionSkipped,
    FacilityBuilt, FinancingDrawn, FinancingRecalled, GameOver, GameSession, GodAppeared, GodDeed,
    GodExit, GodLeft, GodPossessed, IndemnityPaid, Institution, ItemPlaced, ItemThieved, LandId,
    LandTaken, LotteryBought, LotteryDrawn, LotteryWon, MagicBlocked, MagicCast, MagicSpell,
    MarketClock, MinigameScore, MortgageDone, MortgageOp, MountChanged, MoveCapped, NewsAudience,
    NewsFired, NewsScope, PassiveUsed, Paused, PendingDecision, PlayerBailed, PlayerBankrupt,
    PlayerName, PointsChanged, PointsStolen, PolicyBought, RemoteDiceUsed, RentRerouted,
    ServiceCharged, ShopOpDone, StayLifted, StaySentence, StaySkipped, StockTraded, StockVisited,
    TileIndex, TollExtorted, TradeSide, TurnOrder, TurnPhase, VillainHired, WalkHalted,
};

use super::dialog::describe;
use super::finance::comma;
use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, Disabled, PRIMARY, Palette, SECONDARY, UiAction, button, can_roll, label,
};
use crate::render::DevFont;

#[derive(Component)]
pub(crate) struct HintText;

#[derive(Component)]
pub(crate) struct PhaseText;

/// 顶栏根节点。`refresh_top_bar` 只刷它名下的按钮 —— 见那个系统的注释。
#[derive(Component)]
pub(crate) struct TopBar;

/// 顶栏。装进 `UiSet::Boot`：与棋盘同批出生，回菜单时随 [`BoardUi`] 一起收走。
pub(crate) fn spawn_top_bar(mut commands: Commands, font: Res<DevFont>) {
    let (hint, hint_font, hint_color) = label(&font.0, "准备中", size::BODY, col::HINT);
    let (phase, phase_font, phase_color) = label(&font.0, "", size::BODY, col::PHASE);
    let bar = commands
        .spawn((
            BoardUi,
            TopBar,
            // 顶栏横跨整屏宽，而命中测试只看「被报到的最深那个节点」：
            // 不挂 `Pickable::IGNORE`，它背后那一整条带上的地板就永远点不到
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                top: Val::Px(layout::TOP_BAR_INSET),
                left: Val::Px(layout::TOP_BAR_INSET),
                right: Val::Px(layout::TOP_BAR_INSET),
                height: Val::Px(layout::ROLL_BUTTON[1]),
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(layout::TOP_BAR_GAP),
                ..default()
            },
        ))
        .id();
    commands
        .entity(bar)
        .with_child((
            HintText,
            // `flex_grow` 让提示行吃掉剩余宽度，阶段名与按钮才被推到右边
            // （Godot 的 `size_flags_horizontal = 3`）
            Node {
                flex_grow: 1.0,
                ..default()
            },
            hint,
            hint_font,
            hint_color,
        ))
        .with_child((
            PhaseText,
            // 不许收缩：顶栏六颗按钮 + 一句长提示在一行 1024 里是挤的，而挤出来的
            // 代价不该由阶段名付 —— 「等待掷\n骰」换行看着像排版坏了，不像屏宽不够
            Node {
                flex_shrink: 0.0,
                ..default()
            },
            phase,
            phase_font,
            phase_color,
        ))
        .with_children(|bar| {
            // 掷骰按钮按「常态」spawn，可用态全交给 `refresh_top_bar` 判：
            // `Palette` 因此恒等于主色，解禁时 `bg = palette.normal` 才是对的颜色
            button(
                bar,
                &font.0,
                ButtonSpec::new("掷骰", layout::ROLL_BUTTON, PRIMARY, UiAction::Roll),
            );
            // B5b 的两个入口。ATM 只在卡到手后亮（路过银行格才有），
            // 证券随时可看 —— 「能不能看」与「能不能下单」是两件事，分开灰
            button(
                bar,
                &font.0,
                ButtonSpec::new(
                    "ATM",
                    layout::FINANCE_BUTTON,
                    SECONDARY,
                    UiAction::ToggleAtm,
                ),
            );
            button(
                bar,
                &font.0,
                ButtonSpec::new(
                    "证券",
                    layout::FINANCE_BUTTON,
                    SECONDARY,
                    UiAction::ToggleStock,
                ),
            );
            // 手牌随时可看（同证券：「能不能看」与「能不能出牌」是两件事，分开灰）。
            // 手里没牌时面板会明写「手里没有卡」，所以入口不必等牌到手才亮 ——
            // 一个从没亮过的按钮，玩家不会知道它有这块面板
            button(
                bar,
                &font.0,
                ButtonSpec::new(
                    "手牌",
                    layout::FINANCE_BUTTON,
                    SECONDARY,
                    UiAction::ToggleHand,
                ),
            );
            // 道具栏（P5）：与手牌同一条 —— 随时能看，能不能动手写在面板里那行字上
            button(
                bar,
                &font.0,
                ButtonSpec::new(
                    "道具",
                    layout::FINANCE_BUTTON,
                    SECONDARY,
                    UiAction::ToggleItems,
                ),
            );
            // Godot 的暂停只有 ESC 一条路（`pause_menu.gd:19-25`），这里补一个按钮：
            // 只用键盘能找到的功能，鼠标玩家就找不到
            button(
                bar,
                &font.0,
                ButtonSpec::new(
                    "暂停",
                    layout::BACK_BUTTON,
                    SECONDARY,
                    UiAction::TogglePause,
                ),
            );
            button(
                bar,
                &font.0,
                ButtonSpec::new(
                    "返回主菜单",
                    layout::BACK_BUTTON,
                    SECONDARY,
                    UiAction::BackToMenu,
                ),
            );
        });
}

/// 顶栏要读的世界状态。收成一个参数包是房规：参数一多不挂 `#[allow(too_many_arguments)]`
/// 而是打包（§六 坑 8），否则「这个 system 真的需要知道这么多事」这个信号会被吃掉。
#[derive(SystemParam)]
pub(crate) struct TurnView<'w, 's> {
    phase: Res<'w, State<TurnPhase>>,
    order: Res<'w, TurnOrder>,
    session: Res<'w, GameSession>,
    paused: Res<'w, Paused>,
    pending: Res<'w, PendingDecision>,
    atm: Res<'w, AtmAccess>,
    clock: Res<'w, MarketClock>,
    names: Query<'w, 's, &'static PlayerName>,
    /// 当前这位身上有没有停留计时（P4④）：有则那颗「掷骰」写「停留」。
    /// 判据放在这里而不是按 `TurnPhase` 猜，是因为它跟人走、不跟阶段走。
    stays: Query<'w, 's, &'static StaySentence>,
    /// 此刻在行动的是不是一位**受雇的恶人**（P12）。旁白要说清这件事：
    /// 「请掷骰」对一位自己会掷骰的 NPC 是一句假指令，而玩家那侧真正想知道的是
    /// 「这是谁雇来的、替谁办事」——那半句话只存在于 `Hired` 里。
    hired: Query<'w, 's, &'static game_core::prelude::Hired>,
    /// 这一位是电脑还是等人点（P14e）。旁白要说清「请掷骰」是对人说的：
    /// 电脑座位自己会掷，玩家真正想知道的是「现在轮到谁在想」—— 而
    /// `AiProfile.who` 的第二处上屏也在这里（第一处是 HUD 那张玩家卡）
    controllers: Query<'w, 's, &'static Controller>,
}

/// 一颗按钮此刻该不该亮。三颗按钮三种判据，都从这里出，
/// 于是「看着能点、按下去没反应」只可能是这一处写错，而不是散在三个构造点。
///
/// 范围由 `TopBar` 圈定（见 `refresh_top_bar`），所以这里的输入只有顶栏那六颗。
/// 新加一颗**顶栏**按钮时必须在这里补一条判据 —— 漏了不会编译失败，只会永远亮着，
/// 症状与坑 31 反过来：灰该灰的时候不灰。浮层里的按钮不走这条路，别往这儿加。
fn button_allowed(action: &UiAction, turn: &TurnView) -> bool {
    let phase = *turn.phase.get();
    match action {
        UiAction::Roll => can_roll(phase, &turn.paused, &turn.pending),
        // ATM：卡在手上一路都开着，唯一挡它的是暂停与别人家的模态（模态的遮罩本来也压着）
        UiAction::ToggleAtm => {
            !turn.paused.0
                && turn.atm.0.is_some()
                && super::counter_of(&turn.pending).is_none()
                && turn.pending.active().is_none()
        }
        // 证券：随时能看行情，下单的限制写在面板里那八颗按钮上
        UiAction::ToggleStock => !turn.paused.0,
        // 手牌：同一条 —— 能不能看由暂停决定，能不能出牌写在面板里那行字上
        UiAction::ToggleHand => !turn.paused.0,
        // 道具栏同理：看是随时的事，放不放得下由面板里那排候选与规则层判
        UiAction::ToggleItems => !turn.paused.0,
        _ => true,
    }
}

/// 重着色要动的那两样：按钮名下的子节点，与子节点上的文字颜色。
///
/// 收成参数包是房规（§六 坑 8）：参数一多该问「这些是不是同一件事的输入」，
/// 而不是挂 `#[allow(clippy::too_many_arguments)]` 把那个信号吃掉。
#[derive(SystemParam)]
pub(crate) struct TopBarPaint<'w, 's> {
    kids: Query<'w, 's, &'static Children>,
    labels: Query<'w, 's, &'static mut TextColor>,
}

/// 改写按钮上那行字的颜色。
///
/// `button()` 在 spawn 那一刻就把灰字配好了，但顶栏的可用态是**每帧算出来的**：
/// ATM 卡到手/过期、暂停开合都发生在 spawn 之后。只换底色不换字色，禁用的按钮
/// 仍然顶着一行白字 —— 而底色 `BTN_DISABLED` 与 `BTN_SECONDARY` 差得并不远，
/// 玩家读不出「这颗按不动」。
fn paint_label(paint: &mut TopBarPaint<'_, '_>, button: Entity, disabled: bool) {
    let want = if disabled {
        col::BTN_TEXT_DISABLED
    } else {
        Color::WHITE
    };
    let Ok(children) = paint.kids.get(button) else {
        return;
    };
    for child in children {
        if let Ok(mut text) = paint.labels.get_mut(*child) {
            text.0 = want;
        }
    }
}

/// 阶段跳变时改写旁白与阶段名；按钮的可用态则跟着判据走。
///
/// 两条守卫分开是因为它们的驱动源不同。文本只关心阶段，用 `Local<Option<TurnPhase>>`
/// 而不是 `Ref<State<_>>::is_changed`：装配那一帧按钮还不在世界里（`Commands` 帧末才 flush），
/// 文本写空手时要把这一帧**记下来重来**，判据握在自己手里更好办。
/// 按钮**不设守卫**：暂停开合、待决事项清空、ATM 卡到手都不改阶段，只盯阶段的守卫会让它们
/// 停在错误的颜色上；而回主菜单再开一局时 `Local` 会带着上一局的判断活下来，新 spawn 的按钮
/// 就再也没人刷过。判据本身是幂等的（只在 `Disabled` 与判据不一致时才动手），逐帧跑也
/// 不影响悬停换色。
///
/// **只管顶栏名下那六颗**（P3b 真鼠标冒烟翻出来的账，§6.1 坑 31）。早先这里查的是
/// 「全工程任何带 `UiAction` 的按钮」，而 `button_allowed` 对不认识的动作一律放行 ——
/// 于是每块浮层里 spawn 期就该灰着的按钮，下一帧被顶栏剥掉 `Disabled`、标签刷成白字：
/// 灰底白字正是坑 26 说过的「灰了但读不出灰」，而面板重建时这些实体会被 despawn，
/// 排到帧末的 `remove::<Disabled>` 打在空实体上，日志里就是一串 `Entity despawned`。
/// 浮层的灰不灰由各块自己的 sync 按自己的签名决定（`finance`、`hand` 都是），顶栏不该代劳。
pub(crate) fn refresh_top_bar(
    turn: TurnView,
    bars: Query<&Children, With<TopBar>>,
    mut buttons: Query<(
        Entity,
        &mut BackgroundColor,
        &Palette,
        Option<&Disabled>,
        &UiAction,
    )>,
    mut paint: TopBarPaint<'_, '_>,
    mut texts: Query<(&mut Text, Option<&HintText>, Option<&PhaseText>)>,
    mut commands: Commands,
    mut seen_phase: Local<Option<TurnPhase>>,
) {
    let now = *turn.phase.get();
    // 这一回合轮到的人被停留按住吗（P4④）。与 `button_allowed` 分开算：按钮仍然可用，
    // 变的只是它代表哪个意图（「停留」），以及派发处发哪条消息
    let held = turn
        .order
        .current()
        .is_some_and(|p| turn.stays.get(p).is_ok());
    for entry in bars.iter() {
        for child in entry {
            // 顶栏名下还有旁白与阶段名两行文本，它们没有按钮的那套组件 → 跳过
            let Ok((entity, mut bg, palette, disabled, action)) = buttons.get_mut(*child) else {
                continue;
            };
            let allowed = button_allowed(action, &turn);
            match (allowed, disabled) {
                (true, Some(_)) => {
                    commands.entity(entity).remove::<Disabled>();
                    bg.0 = palette.normal;
                    paint_label(&mut paint, entity, false);
                }
                (false, None) => {
                    commands.entity(entity).insert(Disabled);
                    bg.0 = col::BTN_DISABLED;
                    paint_label(&mut paint, entity, true);
                }
                _ => {}
            }
            // 「掷骰」与「停留」是同一个按钮、同一个意图位（P4④）。写成每帧比对再写，
            // 免得反复赋同一个字符串让排版重来
            if matches!(action, UiAction::Roll) {
                let want = if held { "停留" } else { "掷骰" };
                if let Ok(kids) = paint.kids.get(entity) {
                    for child in kids {
                        if let Ok((mut text, ..)) = texts.get_mut(*child)
                            && text.0 != want
                        {
                            text.0 = want.to_string();
                        }
                    }
                }
            }
        }
    }

    if *seen_phase == Some(now) {
        return;
    }
    let hint = phase_hint(now, &turn);
    // 阶段标签要比旁白多两个读数：轮到的是谁、是不是电脑（P14e）。旁白会被事件流水
    // 盖掉，这一段不会 —— 「现在轮到谁在想」整个阶段都成立
    let who = turn.order.current();
    let who_name = who
        .and_then(|player| turn.names.get(player).ok())
        .map(|name| name.0.as_str());
    let who_ai = match who.and_then(|player| turn.controllers.get(player).ok()) {
        Some(Controller::Ai(profile)) => Some(profile.who),
        _ => None,
    };
    let mut wrote = false;
    for (mut text, is_hint, is_phase) in texts.iter_mut() {
        if is_phase.is_some() {
            *text = Text::new(phase_label_word(now, who_name, who_ai));
            wrote = true;
        } else if is_hint.is_some() {
            if let Some(line) = hint.as_deref() {
                *text = Text::new(line);
            }
            wrote = true;
        }
    }
    // 顶栏还没进世界时不能把这一帧记成「已刷新」，否则它永远停在「准备中」
    if wrote {
        *seen_phase = Some(now);
    }
}

/// 哪些阶段要说一句话。`TurnStart`/`Landed`/`WaitDecision`/`TurnEnd` 保持上一条
/// （Godot 的 `match phase_name` 同样没给这几个分支赋文案）。
///
/// `Over` 刻意**不**写文案：终局那句话是「胜者：X！」，由 [`hint_from_events`] 从
/// `GameOver` 带来。阶段旁白与事件流水差一帧到位（消息下一帧才读得到，转移也晚一帧），
/// 于是这里写「游戏结束」会把胜者那句盖掉 —— 而阶段标签已经说了「游戏结束」，重复没有价值。
///
/// 等待掷骰那一行带上日历与物价：行情每回合结束动一次、分红按「几月几日」发、
/// 地价租金按 [`game_core::GameSession::price_index`] 缩放，玩家要能在动手之前看见这三个数，
/// 否则「每月 15 号派息」与「地价翻了一倍」只是两句写在文档里的话。
fn phase_hint(phase: TurnPhase, turn: &TurnView) -> Option<String> {
    let text = match phase {
        TurnPhase::Boot => "准备中".to_string(),
        TurnPhase::WaitRoll => {
            let date = game_core::prelude::date_of(turn.clock.tick, turn.session.epoch);
            // 指数每回合末重算，而这一行在进入 WaitRoll 时才写 —— 两者同帧之后，
            // 所以这里读到的就是「今天」那个数（旁白不会显示昨天的物价）
            let index = turn.session.price_index;
            // 主语那一格分三种读数：玩家（请按骰）、受雇的恶人（他自己掷，替他报一声）、
            // 查不到名字（菜单态与装配前）
            let head = match turn.order.current() {
                Some(who) => {
                    let boss = turn.hired.get(who).ok().map(|hired| {
                        turn.names
                            .get(hired.employer)
                            .map(|n| n.0.clone())
                            .unwrap_or_else(|_| "?".into())
                    });
                    let ai = match turn.controllers.get(who).ok() {
                        Some(Controller::Ai(profile)) => Some(profile.who),
                        _ => None,
                    };
                    head_word(
                        turn.names.get(who).ok().map(|n| n.0.as_str()),
                        boss.as_deref(),
                        ai,
                    )
                }
                None => "请掷骰".to_string(),
            };
            format!(
                "{head}（第 {} 回合 · {date} · 物价 ×{index:.2}）",
                turn.session.turn_count
            )
        }
        TurnPhase::Walking => "移动中…".to_string(),
        _ => return None,
    };
    Some(text)
}

/// 旁白里「谁的回合」那一句。
///
/// 抽成纯函数是为了能在没有世界的场合钉住它（P14e）：**电脑座位那句「正在想…」与
/// 人类那句「请掷骰」是两句话** —— 对一位自己会掷骰的电脑说「请掷骰」是一句假指令，
/// 而玩家真正想知道的是「现在轮到谁在想」。旁白的优先级：受雇的恶人先说自己替谁办事，
/// 然后才轮到「电脑／人」。
fn head_word(name: Option<&str>, boss: Option<&str>, ai: Option<&str>) -> String {
    match (name, boss, ai) {
        (Some(name), Some(boss), _) => format!("{name} 替 {boss} 办事 — 他自己掷骰"),
        (Some(name), None, Some(who)) => format!("{name} 的回合 — 电脑（{who}）正在想…"),
        (Some(name), None, None) => format!("{name} 的回合 — 请掷骰"),
        (None, ..) => "请掷骰".to_string(),
    }
}

/// 阶段标签那一段字：轮到电脑时**把是谁、哪一档性格**也写上去（P14e）。
///
/// 为什么不上旁白那一行：旁白在同一个阶段里会被事件流水盖掉（「莱息 来到银行」），
/// 而「现在轮到谁在想」是**整个阶段都成立**的事实 —— 它该待在每帧都看得见的那一段字里。
/// 人类座位的回合不写名字：玩家自己的名字就在 HUD 上，重复一遍只是噪音。
fn phase_label_word(phase: TurnPhase, name: Option<&str>, ai: Option<&str>) -> String {
    match (name, ai) {
        (Some(name), Some(who)) => format!("{} · {name}（电脑·{who}）", phase_name(phase)),
        _ => phase_name(phase),
    }
}

/// 阶段名。Godot 直接把内部枚举名 `WAIT_ROLL` 打在屏幕上，这里换成中文（§5.1 差异表）。
fn phase_name(phase: TurnPhase) -> String {
    let name = match phase {
        TurnPhase::Boot => "准备中",
        TurnPhase::TurnStart => "回合开始",
        TurnPhase::WaitRoll => "等待掷骰",
        TurnPhase::Walking => "行走中",
        TurnPhase::Halted => "行走中断",
        TurnPhase::Landed => "落格结算",
        TurnPhase::WaitDecision => "等待决策",
        TurnPhase::TurnEnd => "回合结束",
        TurnPhase::Over => "游戏结束",
    };
    name.to_string()
}

/// 没有模态可看的那几条事件，写进提示行。
#[derive(SystemParam)]
pub(crate) struct Notices<'w, 's> {
    detention_skip: MessageReader<'w, 's, DetentionSkipped>,
    /// 人回来了（P4①）。三个去处是三句话，但在模型里是同一条凭据，所以这里只多一个动词：
    /// 「出狱」「出院」「回国」由 `DetentionPlace::back_verb` 给
    detention_back: MessageReader<'w, 's, DetentionLifted>,
    /// 保释成交（P4③）。与 `detention_back` 分开：那条说的是「期满」，这条说的是
    /// 「有人替他付了账」—— 屏幕上要同时说得清谁付、付了多少
    bailed: MessageReader<'w, 's, PlayerBailed>,
    /// 停留被跳过（P3d④c）。排在拘留之后：同时挂着两张计时的人，这一帧两条都会写，
    /// 而留下停留那句更对 —— 拘留是旧事，「被停留卡按住」才是刚刚发生的那件
    stay_skip: MessageReader<'w, 's, StaySkipped>,
    /// 停留期满。它与上一条不会同一帧同时出现（一个还在扣格、一个刚扣完），
    /// 但排在 `stay_skip` 之后：这一行说的是「从本回合起能动」，是那条流水的终点
    stay_lifted: MessageReader<'w, 's, StayLifted>,
    bankrupt: MessageReader<'w, 's, PlayerBankrupt>,
    bank: MessageReader<'w, 's, BankVisited>,
    bank_ops: MessageReader<'w, 's, BankOpDone>,
    stock: MessageReader<'w, 's, StockVisited>,
    traded: MessageReader<'w, 's, StockTraded>,
    points: MessageReader<'w, 's, PointsChanged>,
    /// 游乐场那一局的成绩（P7）。排在 `points` 之后：换到点券的那一帧两条都会写，
    /// 而这一条说得出「玩的是哪一款、拿了几分」—— 点券余数在 HUD 上一直看得见
    minigame: MessageReader<'w, 's, MinigameScore>,
    /// 乐透那三条（P8）。排在 `minigame` 之后，三条之间也是「买券 → 开奖 → 中奖」的因果序：
    /// 同一帧里后写的盖掉前写的，于是玩家最后读到的是**结果**（谁领了多少钱），
    /// 而流局那一句因为没有中奖凭据而留得住。
    ticket: MessageReader<'w, 's, LotteryBought>,
    drawn: MessageReader<'w, 's, LotteryDrawn>,
    won: MessageReader<'w, 's, LotteryWon>,
    // ===== 拍卖（P9）=====
    // 五条排在一起，顺序是**因果序**：开场 → 每一声喊价 → 某个人退出 → 落槌 → 流标。
    // 同一帧里后写的盖掉前写的，于是玩家最后读到的是结果（谁拍走了、多少钱），
    // 而一场拍卖在长跑里通常就压在同一帧上散场（自动玩家一帧喊一声）。
    //
    // 与乐透那三条同一个道理：拍卖没有持久的读数（面板随轮转每帧重建，下一帧问的就是
    // 另一个人），所以顶栏是它唯一的出口。
    auctions_started: MessageReader<'w, 's, AuctionStarted>,
    bids: MessageReader<'w, 's, BidPlaced>,
    passes: MessageReader<'w, 's, BidWithdrawn>,
    sold: MessageReader<'w, 's, AuctionWon>,
    unsold: MessageReader<'w, 's, AuctionFailed>,
    /// 百货成交（P3d②）。排在 `points` 之后：同一帧里那笔点券变动与这张成交凭据说的
    /// 是同一件事，而带卡名的那条更能回答「我刚按下去发生了什么」
    shop: MessageReader<'w, 's, ShopOpDone>,
    /// 抵押柜台的成交（P3d③）。排在 `shop` 之后、`bank_ops` 那一族之外单开一条：
    /// 它说的是「哪块地、过手多少」，而那四条银行操作只带金额。
    mortgage: MessageReader<'w, 's, MortgageDone>,
    /// 出牌的结果。手牌面板随手牌重建，把一次性结果烤进去下一次就没了 ——
    /// 持久的读数只有这一行（与点券那一条同一个道理）
    played: MessageReader<'w, 's, CardPlayed>,
    // ===== 四大恶人（P12b–d）=====
    // 六条排在一起，说的都是「雇来的那个家伙刚刚干了什么」。出场那两条（抢存款/偷点券）
    // 各只有一句汇总；环上的四条是逐次发生的，同一帧里后写的盖掉前写的。
    // 他们的名字从 `PlayerName` 来（恶人也挂这一位），所以 `name()` 直接可用。
    /// 出场分派（bail 窗口发的）。抢存款/偷点券的凭据在下面两条 —— 这一条基本被盖掉，
    /// 留着是「谁被放出来了」的那一帧记录
    villain_hired: MessageReader<'w, 's, VillainHired>,
    /// 强盗抢走全场存款（含雇主自己 —— 那是特性，播报要把这句话说全）
    robbed: MessageReader<'w, 's, DepositsRobbed>,
    /// 小偷偷走其他玩家的一半点券
    stolen: MessageReader<'w, 's, PointsStolen>,
    /// 小偷搬走一件路上放置物
    thieved: MessageReader<'w, 's, ItemThieved>,
    /// 强盗擦肩抢走一张卡
    snatched: MessageReader<'w, 's, CardSnatched>,
    /// 流氓勒索成功
    toll: MessageReader<'w, 's, TollExtorted>,
    /// 间谍截租
    rerouted: MessageReader<'w, 's, RentRerouted>,
    // ===== 神明（P6e①）=====
    // 这六条排在 `played` **之前**：请神符那句 `describe` 已经说了「请来哪位、他从哪一格
    // 过来」，再让 `GodPossessed` 盖上去就是同一件事说两遍；而**走路踩到神**时根本没有
    // `CardPlayed`，这几条照样播得出来。两条路各说一句，不互相覆盖。
    /// 一位神上路。它不吃鼠标、不弹模态，除了这一行没有任何出口（dump 那一栏要开面板才看得见）
    god_appeared: MessageReader<'w, 's, GodAppeared>,
    /// 上身。`days` 由这条带着，所以这一行与 HUD 那句「还剩几天」读的是同一个数
    god_possessed: MessageReader<'w, 's, GodPossessed>,
    /// 离场。三种走法三句话（`GodExit::label`），而它们共用一条凭据
    god_left: MessageReader<'w, 's, GodLeft>,
    /// 上身那一刻当场做的那件事（给钱／收钱／抢钱／送卡）。它排在 `god_possessed` 之后：
    /// 「附身」是状态，HUD 会一直显示；「给了 3,521 元」是这一帧刚发生的事，顶栏该说后者
    god_deed: MessageReader<'w, 's, GodDeed>,
    /// 卡**离开手牌但不是被打出**。玩家问的是「我那张卡去哪了」，只有这条答得出
    card_lost: MessageReader<'w, 's, CardLost>,
    /// 土地公把一块地直接拿走。它与「有人付钱买地」是两条凭据，那条带付款、这条不带
    land_taken: MessageReader<'w, 's, LandTaken>,
    // ===== 道路放置物与道具（P5）=====
    /// 放下了什么、放在哪一格。面板随道具数重建，一次性结果只有这一行能说
    placed: MessageReader<'w, 's, ItemPlaced>,
    /// 行走被道路放置物打断。这一行与落格结算互斥 —— 停在无主地上也不会弹买地面板
    halted: MessageReader<'w, 's, WalkHalted>,
    /// 定时炸弹换了主人。「换人」不是一次静默的状态改写，新主人得看见它跟过来了
    bomb_transferred: MessageReader<'w, 's, BombTransferred>,
    chest: MessageReader<'w, 's, ChestOpened>,
    remote: MessageReader<'w, 's, RemoteDiceUsed>,
    /// 乌龟卡夹步：骰面与实际步数两个数都要说，否则玩家会以为骰子坏了
    capped: MessageReader<'w, 's, MoveCapped>,
    mount: MessageReader<'w, 's, MountChanged>,
    // ===== 商业用地（P10c）=====
    /// 服务费成交。**收据面板下一刻就被关掉了**，而这一行是唯一持久的那句话 ——
    /// 它要说得出「哪座设施、算了多少」，否则玩家只看见钱包少了一截。
    /// 排在 P5 那一族之后：道具与收费不会同帧发生，而这一条更该留在最后
    service: MessageReader<'w, 's, ServiceCharged>,
    /// 公司到访（P10d①）。与 `service` 同一条理由：收据下一刻就被关掉，这一行是
    /// 唯一持久的那句话 —— 它要说得出「哪家公司、为什么是这个数（董事长免单）」
    company: MessageReader<'w, 's, CompanyCharged>,
    /// 设施建成。与 `land_taken` 同一形：只带一块地，不说办成了什么
    facility_built: MessageReader<'w, 's, FacilityBuilt>,
    // ===== 保险与董事长特权（P10d②）=====
    // 四条排在 `company` 之后：投保/赔付/白拿/融资与公司到访都是「过手一笔钱或货」的
    // 事件，而它们没有收据面板 —— 顶栏这一行是唯一持久的出口（dump 那一栏要开面板）。
    /// 投保成交。保单本身挂在身上（HUD 有「投保剩 N 天」那一段），这一行只播成交
    policies: MessageReader<'w, 's, PolicyBought>,
    /// 保险赔付。与 `policies` 分开：那是「付钱换保障」，这条是「出事领钱」，
    /// 两条不同帧，同一条词会让人分不清方向
    indemnities: MessageReader<'w, 's, IndemnityPaid>,
    /// 董事长白拿。**没有点券流水**（分文不动），不带这一行就是「按了没反应」
    freebies: MessageReader<'w, 's, ChairmanTook>,
    /// 融资成交（挪用与兜底共用一条）。与 `bank_ops` 分开：那条的借还占额度计息，
    /// 这条不计息 —— 说清「挪的是谁的存款」才不会当成贷款
    fin_drawn: MessageReader<'w, 's, FinancingDrawn>,
    /// 融资收回（身份丢了当场清算）。坏账那一格（现金不够还的部分）是玩家唯一
    /// 看得见的「银行吃了亏」，必须说出口
    fin_recalled: MessageReader<'w, 's, FinancingRecalled>,
    /// 新闻（P11）。排在全部结算之后、`over` 之前：新闻中心那一格在落格链的**末端**跑，
    /// 而它动的正是前面那些域管着的东西 —— 同一帧里「谁付了租」与「号外说全场缴税」都写了
    /// 的时候，留下号外那一句才对（它是这一趟最后发生的事）
    news: MessageReader<'w, 's, NewsFired>,
    // ===== 魔法屋（P13）=====
    /// 一道法术施完了。`targets` 是**实际生效**的承受者 —— 被免罪挡掉的那一位不在里面，
    /// 他有自己的凭据 [`MagicBlocked`]，两条各一份才有那四种说法
    magic_cast: MessageReader<'w, 's, MagicCast>,
    /// 免罪卡挡下一道法术（十二道里挡得住的那两道才会发这一条）
    magic_blocked: MessageReader<'w, 's, MagicBlocked>,
    // ===== 被动卡那道门（P13b②）=====
    /// 一张被动卡**真的花掉了**。它与上面那条 `MagicBlocked` 的分别：那一条只讲「这道法术
    /// 没落到他身上」，这一条讲的是**谁的钱包少了一张卡** —— 而门为五处来路都发这一条
    /// （卡、命运牌、新闻、法事、③ 的账），只接前一条就只有魔法屋那一路说得出来
    passive_used: MessageReader<'w, 's, PassiveUsed>,
    over: MessageReader<'w, 's, GameOver>,
}

pub(crate) fn hint_from_events(
    mut notices: Notices,
    names: Query<&PlayerName>,
    // 抵押与赎回的凭据带的是 `LandId`（与 `RentDue` 同一条口径：地块靠编号，卡才带卡面），
    // 所以这里要查一次定义才能说出「哪块地」。菜单态没有棋盘，故 `Option`
    board: Option<Res<ActiveBoard>>,
    mut hints: Query<&mut Text, With<HintText>>,
) {
    // 一帧里可能同时有多条（例如破产紧跟在回合结束旁白之后）：按流水顺序取最后一条，
    // 与 Godot 那些信号处理函数逐个覆盖 `hint_label.text` 的结果一致
    let mut line: Option<String> = None;
    for m in notices.detention_skip.read() {
        // 「剩几回合」在笼中那一条上没有答案（P12 的恶人：只有保释办得出来），
        // 而那句话说得出「等保释」—— 写成「剩 0 回合」正好是反的
        let left = match m.left {
            Some(n) => format!("剩 {n} 回合"),
            None => "等保释".to_string(),
        };
        line = Some(format!(
            "{} 在{}中（{left}），跳过本回合",
            name(&names, m.player),
            m.place.label()
        ));
    }
    for m in notices.detention_back.read() {
        line = Some(format!(
            "{} {}了，本回合照常行动",
            name(&names, m.player),
            m.place.back_verb()
        ));
    }
    // 停留与拘留共用「跳过本回合」这句话，但**不共用一条流水**：人在不在场上是两件事
    // （`StaySkipped` 的注释）。HUD 那段有持久的剩余回合数，这里只播这一次
    for m in notices.stay_skip.read() {
        line = Some(format!(
            "{} 原地停留（剩 {} 回合），跳过本回合",
            name(&names, m.player),
            m.turns_left
        ));
    }
    for m in notices.stay_lifted.read() {
        line = Some(format!(
            "{} 停留期满，下一回合起照常行动",
            name(&names, m.player)
        ));
    }
    for m in notices.bankrupt.read() {
        line = Some(format!("{} 破产了！", name(&names, m.player)));
    }
    for m in notices.bank.read() {
        line = Some(format!(
            "{} 来到银行（存款 ${}，贷款 ${}）",
            name(&names, m.player),
            comma(m.deposit),
            comma(m.loan)
        ));
    }
    for m in notices.bank_ops.read() {
        line = Some(format!(
            "{} {}${}",
            name(&names, m.player),
            op_verb(m.op),
            comma(m.amount)
        ));
    }
    for m in notices.stock.read() {
        line = Some(format!("{} 来到了证券交易所", name(&names, m.player)));
    }
    for m in notices.traded.read() {
        line = Some(format!(
            "{} {} {} 股 @ ${:.2}",
            name(&names, m.player),
            side_verb(m.side),
            m.shares,
            m.price
        ));
    }
    // 点券到账：它不进现金流水、也没有模态可看，这一行是唯一的播报口。
    // 和其余事件流水一样会被下一次阶段旁白冲掉（只活两三帧），持久的读数在 HUD 那一段
    for m in notices.points.read() {
        // 方向写在动词上而不是符号上：P3d② 起点券会**减少**（买卡），而「捡到 -30 点券」
        // 读起来像租金表坏了
        let (verb, amount) = if m.delta < 0 {
            ("花掉", -m.delta)
        } else {
            ("捡到", m.delta)
        };
        line = Some(format!(
            "{} {} {} 点券（共 {}）",
            name(&names, m.player),
            verb,
            amount,
            m.balance
        ));
    }
    // 游乐场（P7）：这一行要说得出「哪一款、几分、换了多少」，因为那一屏下一刻就收走了，
    // 而点券余数在 HUD 上一直看得见。`awarded` 那一位也得说出来 —— 练习局报的分是 0 点券，
    // 不写清楚就成了「玩了半天什么都没拿到」
    for m in notices.minigame.read() {
        line = Some(if !m.awarded {
            format!(
                "{} 玩「{}」得了 {} 分 —— 练习局，不计点券",
                name(&names, m.player),
                m.game.label(),
                m.score
            )
        } else {
            format!(
                "{} 玩「{}」得了 {} 分，换 {} 点券",
                name(&names, m.player),
                m.game.label(),
                m.score,
                m.points
            )
        });
    }
    // 乐透（P8）。买券那一句不带奖池余额（面板上一直看得见，而这行要说的是刚发生的事）；
    // 开奖那一句**两种结果各一句话** —— 「没人猜中」是一条真消息，不是什么都没发生，
    // 而滚存下去的那个数字是玩家下一期要不要再买一注的判断依据。
    for m in notices.ticket.read() {
        line = Some(format!(
            "{} 花 {} 买下一注 {} 号",
            name(&names, m.player),
            m.price,
            m.number
        ));
    }
    for m in notices.drawn.read() {
        // 开奖那一句**两种结果各一句话** —— 「没人猜中」是一条真消息，不是什么都没发生，
        // 而滚存下去的那个数字是玩家下一期要不要再买一注的判断依据。
        //
        // 有人猜中时这里就点名：号码独占（10-26 实报）之后一个号至多一位持有者，所以
        // 「谁独得了这一期」总说得出一个名字，而不再是一个人数。点名也让这一句在
        // **没有** [`LotteryWon`] 的那一格里仍然完整可读（池子为 0 时规则层不发中奖凭据，
        // 顶栏就只剩这一行）—— 两条同时出现的帧里，盖住它的是「谁拿到了钱」那一条。
        line = Some(match m.winner {
            None => format!(
                "乐透开出 {} 号 —— 没人猜中，奖池 {} 滚到下月",
                m.number, m.pot
            ),
            Some(winner) => format!(
                "乐透开出 {} 号 —— {} 独得奖池 {}",
                m.number,
                name(&names, winner),
                m.pot
            ),
        });
    }
    for m in notices.won.read() {
        line = Some(format!(
            "{} 猜中 {} 号，领到 {}",
            name(&names, m.player),
            m.number,
            m.amount
        ));
    }
    // ===== 拍卖（P9）=====
    // 「拍的是哪块地」每条都只带 `LandId`，所以这一族每条都要查一次地块名（同抵押那一条）。
    let plot_of = |land: LandId| -> String {
        board
            .as_ref()
            .and_then(|b| b.def.land(land.0))
            .map_or_else(|| "？".to_string(), |l| l.name.clone())
    };
    for m in notices.auctions_started.read() {
        // 「谁敲的槌」与「钱归谁」是同一句话的两半，而这两种入口在画面上长得一模一样：
        // 不说清的话玩家会以为某个玩家在卖自己的地，而那块地其实是刚死的人留下的
        let who = match m.by {
            Some(founder) => format!("{} 敲槌拍卖{}", name(&names, founder), plot_of(m.land)),
            None => format!("破产变卖：{} 挂拍", plot_of(m.land)),
        };
        line = Some(format!(
            "{who} —— 起拍 {}，一档 {}，{} 人能喊",
            comma(m.opening),
            comma(m.step),
            m.bidders
        ));
    }
    for m in notices.bids.read() {
        line = Some(format!(
            "{} 喊到 {}（{}）",
            name(&names, m.player),
            comma(m.amount),
            plot_of(m.land)
        ));
    }
    for m in notices.passes.read() {
        line = Some(format!(
            "{} 放弃争{}",
            name(&names, m.player),
            plot_of(m.land)
        ));
    }
    for m in notices.sold.read() {
        // 钱去了哪儿必须说：两种入口两个答案，而这一句是这一场拍卖里唯一持久的读数
        // （面板下一刻就在问下一个人了）
        line = Some(format!(
            "{} 以 {} 拍得{} —— 所得{}",
            name(&names, m.player),
            comma(m.price),
            plot_of(m.land),
            match m.by {
                Some(founder) => format!("归 {}", name(&names, founder)),
                None => "归银行".to_string(),
            }
        ));
    }
    for m in notices.unsold.read() {
        line = Some(format!(
            "{} 没人出价，流标（起拍 {}）",
            plot_of(m.land),
            comma(m.price)
        ));
    }
    // 保释：这一行要同时说清「谁付」「付了多少」「放了谁」。它排在通用点券流水之后 ——
    // 同一帧两条说的是同一件事，而带对象与价目的这条更能回答「我刚按下去发生了什么」
    for m in notices.bailed.read() {
        line = Some(format!(
            "{} 花 {} 点券保释了 {}（{}）",
            name(&names, m.by),
            m.points,
            name(&names, m.player),
            m.place.back_verb()
        ));
    }
    // ===== 四大恶人（P12b–d）=====
    // 排在保释之后：出场那两条与保释同源（保释成交的下一帧分派），放在一起读得出
    // 「保释 → 出场动手」的因果。恶人的名字走 `PlayerName`，与玩家同一条读法。
    for m in notices.villain_hired.read() {
        line = Some(format!(
            "{} 出笼，替 {} 办事",
            name(&names, m.villain),
            name(&names, m.employer)
        ));
    }
    for m in notices.robbed.read() {
        line = Some(format!(
            "{} 出手，抢走全场 {} 的存款（{} 个人，包括雇主自己）",
            name(&names, m.villain),
            comma(m.total),
            m.victims
        ));
    }
    for m in notices.stolen.read() {
        line = Some(format!(
            "{} 出手，偷走 {} 的 {} 点券给 {}",
            name(&names, m.villain),
            m.victims,
            m.total,
            name(&names, m.employer)
        ));
    }
    for m in notices.thieved.read() {
        line = Some(format!(
            "{} 把格{}的{}偷走送给了雇主",
            name(&names, m.thief),
            m.tile.0,
            m.item.label()
        ));
    }
    for m in notices.snatched.read() {
        line = Some(format!(
            "{} 擦肩抢走 {} 的「{}」",
            name(&names, m.robber),
            name(&names, m.victim),
            m.card.title
        ));
    }
    for m in notices.toll.read() {
        let plot = board
            .as_ref()
            .and_then(|b| b.def.land(m.land.0))
            .map_or("？", |l| l.name.as_str());
        line = Some(format!(
            "{} 路过{}，勒索 {} {}（钱归他自己）",
            name(&names, m.hoodlum),
            plot,
            name(&names, m.owner),
            comma(m.amount)
        ));
    }
    for m in notices.rerouted.read() {
        let plot = board
            .as_ref()
            .and_then(|b| b.def.land(m.land.0))
            .map_or("？", |l| l.name.as_str());
        line = Some(format!(
            "间谍探到{}有案底，{} 的租 {} 转进了 {} 的口袋",
            plot,
            name(&names, m.spy),
            comma(m.amount),
            name(&names, m.employer)
        ));
    }
    // 百货成交：这一行要说得出「哪张卡」与「几点券」，因为柜台面板下一刻就重建了。
    // 它排在点券那一条之后 —— 同一帧里两条说的是同一件事，而带卡名的这条更能回答
    // 「我刚按下去发生了什么」，点券余数在 HUD 上一直看得见。
    for m in notices.shop.read() {
        line = Some(match m.side {
            TradeSide::Buy => format!(
                "{} 花 {} 点券买了「{}」",
                name(&names, m.player),
                m.points,
                m.card.title
            ),
            TradeSide::Sell => format!(
                "{} 把「{}」卖回百货，收回 {} 点券（9 折）",
                name(&names, m.player),
                m.card.title,
                m.points
            ),
        });
    }
    // 抵押与赎回：这一行要说得出「哪块地」与「多少钱」，因为柜台下一刻就重建了。
    // 赎回那一条把「比到手多多少」写出来 —— 那一成手续费是玩家此刻唯一要看的数
    for m in notices.mortgage.read() {
        let plot = board
            .as_ref()
            .and_then(|b| b.def.land(m.land.0))
            .map_or("？", |l| l.name.as_str());
        line = Some(match m.op {
            MortgageOp::Take => format!(
                "{} 抵押了{}，到手 {}（不计息，赎回加一成）",
                name(&names, m.player),
                plot,
                comma(m.amount)
            ),
            MortgageOp::Redeem => format!(
                "{} 赎回了{}，付掉 {}",
                name(&names, m.player),
                plot,
                comma(m.amount)
            ),
        });
    }
    // ===== P6e①：神明 =====
    // 六句话各抽成一个纯函数：顶栏只有一行，而「这句话怎么说」与名字怎么解析无关
    // （与 `op_verb`／`passive_clause` 同一条分工）。抽出来还有一层用处 —— 那六条分支里
    // 有五条是「带不带某个尾巴」的形状，不抽出来就只能靠截图去撞它们
    for m in notices.god_appeared.read() {
        line = Some(god_appeared_word(m.god.label(), &tile_word(&board, m.tile)));
    }
    for m in notices.god_left.read() {
        line = Some(god_left_word(
            &name(&names, m.player),
            m.god.label(),
            m.reason,
        ));
    }
    for m in notices.card_lost.read() {
        line = Some(card_lost_word(
            m.by.label(),
            &name(&names, m.player),
            &m.card.title,
        ));
    }
    for m in notices.land_taken.read() {
        let plot = board
            .as_ref()
            .and_then(|b| b.def.land(m.land.0))
            .map_or("？", |l| l.name.as_str());
        line = Some(land_taken_word(
            &name(&names, m.player),
            plot,
            m.from.map(|prev| name(&names, prev)).as_deref(),
        ));
    }
    for m in notices.god_possessed.read() {
        line = Some(god_possessed_word(
            &name(&names, m.player),
            m.god.label(),
            m.days,
            m.replaced.map(|old| old.label()),
        ));
    }
    for m in notices.god_deed.read() {
        line = Some(god_deed_word(
            m.god.label(),
            &name(&names, m.player),
            m.cash,
            m.cards.len(),
        ));
    }
    // 出牌的结果必须在这儿说清楚，尤其要说「有没有生效」：一张陷害卡打在带护身的人头上
    // 是什么都没发生的，而手牌面板下一次重建就把那张卡的重置成了常态 —— 没有这一行，
    // 玩家只会看见「按了没反应」
    for m in notices.played.read() {
        let effect = m
            .card
            .outcomes
            .iter()
            .map(super::dialog::describe)
            .collect::<Vec<_>>()
            .join("，");
        let effect_text = if effect.is_empty() {
            String::new()
        } else {
            format!(" — {effect}")
        };
        line = Some(if m.player == m.victim {
            format!(
                "{} 使用了「{}」{effect_text}",
                name(&names, m.player),
                m.card.title
            )
        } else {
            format!(
                "{} 对 {} 使用了「{}」{effect_text}",
                name(&names, m.player),
                name(&names, m.victim),
                m.card.title
            )
        });
    }
    // ===== P5：道具与道路放置物 =====
    for m in notices.placed.read() {
        line = Some(format!(
            "{} 把{}放在了 {}",
            name(&names, m.player),
            m.item.label(),
            tile_word(&board, m.tile)
        ));
    }
    for m in notices.halted.read() {
        line = Some(format!(
            "{} 在 {} 踩到{}，行走中断",
            name(&names, m.player),
            tile_word(&board, m.tile),
            m.cause.label()
        ));
    }
    for m in notices.bomb_transferred.read() {
        line = Some(format!(
            "{} 在 {} 把定时炸弹交给了 {}（剩 {} 步）",
            name(&names, m.from),
            tile_word(&board, m.tile),
            name(&names, m.to),
            m.steps_left
        ));
    }
    for m in notices.chest.read() {
        line = Some(format!(
            "{} 在 {} 打开宝箱，得到「{}」",
            name(&names, m.player),
            tile_word(&board, m.tile),
            m.item.label()
        ));
    }
    for m in notices.remote.read() {
        line = Some(format!(
            "{} 用遥控骰子，这一回合走 {} 格",
            name(&names, m.player),
            m.steps
        ));
    }
    for m in notices.capped.read() {
        line = Some(format!(
            "{} 掷出 {} 点，只走 {} 格（乌龟卡剩 {} 回合）",
            name(&names, m.player),
            m.rolled,
            m.moved,
            m.turns_left
        ));
    }
    for m in notices.mount.read() {
        line = Some(format!(
            "{} 的载具变成{}",
            name(&names, m.player),
            m.mount.label()
        ));
    }
    // ===== 商业用地（P10c）=====
    // 建成那一句排在服务费之前：两条同帧的概率极低（一个发生在自己的空地上、
    // 一个是踩别人的设施），真撞上就让「刚被收了多少钱」留在最后
    for m in notices.facility_built.read() {
        // 商业地的设施挂在**地块**上（一块 2×2 的商业地绑两格），所以播报用 `plot_of`
        // 那个地块名 —— 说格号会有一半人说错
        line = Some(format!(
            "{} 在{}建起了{}(造价 {})",
            name(&names, m.player),
            plot_of(m.land),
            m.kind.label(),
            comma(m.cost)
        ));
    }
    for m in notices.service.read() {
        // 免单那笔是 0（大财神赖账，或门用免费卡免掉 —— 后者那条流水自带金额）：
        // 说「分文不取」而不是「付了 0」，后者读起来像租金表坏了
        let amount = if m.amount == 0 {
            "分文不取".to_string()
        } else {
            format!("{} 付 {}", name(&names, m.payer), comma(m.amount))
        };
        line = Some(if m.stay_days > 0 {
            format!("{amount} 在{}住了 {} 天", m.kind.label(), m.stay_days)
        } else if m.roll > 0 {
            format!("{amount} 在{}（摇出 {}）", m.kind.label(), m.roll)
        } else {
            format!("{amount} 在{}过路", m.kind.label())
        });
    }
    for m in notices.company.read() {
        // 公司名要从棋盘上查（消息里只有股票 id）：夹具没有棋盘，落回 id 本身
        let company = board
            .as_ref()
            .and_then(|b| {
                b.def
                    .tiles
                    .iter()
                    .find(|t| t.data.stock.as_deref() == Some(&m.stock))
                    .map(|t| t.name.clone())
            })
            .unwrap_or_else(|| m.stock.clone());
        // 董事长到访那笔金额是 0：「免费」要说得出为什么，不能读起来像租金表坏了
        line = Some(if m.amount == 0 && m.chairman == Some(m.payer) {
            format!("{} 是{}的董事长，免费通行", name(&names, m.payer), company)
        } else if m.stay_days > 0 {
            format!(
                "{} 付 {} 在{}住了 {} 天",
                name(&names, m.payer),
                comma(m.amount),
                company,
                m.stay_days
            )
        } else {
            format!(
                "{} 付 {} 在{}过路",
                name(&names, m.payer),
                comma(m.amount),
                company
            )
        });
    }
    // ===== 保险与董事长特权（P10d②）=====
    // 公司名从棋盘上查 —— 与上面 `company` 那一段同一个形状，抽成闭包两处共用
    let stock_company = |stock: &str| -> String {
        board
            .as_ref()
            .and_then(|b| {
                b.def
                    .tiles
                    .iter()
                    .find(|t| t.data.stock.as_deref() == Some(stock))
                    .map(|t| t.name.clone())
            })
            .unwrap_or_else(|| stock.to_string())
    };
    for m in notices.policies.read() {
        line = Some(format!(
            "{} 在{}投保 {} 天，保费 {}",
            name(&names, m.player),
            stock_company(&m.stock),
            m.days,
            comma(m.premium)
        ));
    }
    for m in notices.indemnities.read() {
        line = Some(format!(
            "{} 被拘留 {} 天，{}赔付 {}",
            name(&names, m.player),
            m.days,
            stock_company(&m.stock),
            comma(m.amount)
        ));
    }
    for m in notices.freebies.read() {
        line = Some(format!(
            "{} 是{}的董事长，白拿了一件「{}」",
            name(&names, m.player),
            stock_company(&m.stock),
            m.item
        ));
    }
    for m in notices.fin_drawn.read() {
        line = Some(format!(
            "{} 挪用储户存款 {}（融资债，不计息）",
            name(&names, m.chairman),
            comma(m.amount)
        ));
    }
    for m in notices.fin_recalled.read() {
        line = Some(format!(
            "{} 不再执掌银行保险，还清融资 {}",
            name(&names, m.player),
            comma(m.amount)
        ));
    }
    for m in notices.news.read() {
        line = Some(match &m.news.audience {
            NewsAudience::People(hits) => {
                let said: Vec<(String, String)> = hits
                    .iter()
                    .map(|hit| {
                        (
                            name(&names, hit.player),
                            hit.outcomes
                                .iter()
                                .map(describe)
                                .collect::<Vec<_>>()
                                .join("；"),
                        )
                    })
                    .collect();
                news_headline(&m.news.title, m.news.scope, &said)
            }
            NewsAudience::Ground {
                center,
                lands,
                outcomes,
            } => {
                let plot = |id: &LandId| {
                    board
                        .as_ref()
                        .and_then(|b| b.def.land(id.0))
                        .map_or_else(|| format!("L{}", id.0), |l| l.name.clone())
                };
                news_ground_headline(
                    &m.news.title,
                    center.as_ref().map(plot).as_deref(),
                    &lands.iter().map(plot).collect::<Vec<_>>(),
                    &outcomes.iter().map(describe).collect::<Vec<_>>().join("；"),
                )
            }
            // 机构档（P11c②）：没有人名列表也没有地块清单，主语是「哪一家」
            NewsAudience::Institution { what, days } => {
                news_institution_headline(&m.news.title, *what, *days)
            }
        });
    }
    // 魔法屋那一道法术（P13）。两条凭据先各收一份再决定这句怎么说：「落空」与「被挡下」
    // 在顶栏那一行上是两种不同的世界，而 `MagicCast.targets` 里没有挡住的那位 ——
    // 只读 cast 会把「有人用掉了一张免罪卡」这件事整条吞掉。
    // 排在新闻之后：同帧里两者都写的机会很小（新闻在落格链、法术在回答链），
    // 真要选一个的话，玩家刚点下去的那一步更该被回答
    let blocked: Vec<(Entity, MagicSpell, Entity)> = notices
        .magic_blocked
        .read()
        .map(|m| (m.player, m.spell, m.victim))
        .collect();
    for m in notices.magic_cast.read() {
        let hit: Vec<String> = m.targets.iter().map(|e| name(&names, *e)).collect();
        let held: Vec<String> = blocked
            .iter()
            .filter(|(caster, spell, _)| *caster == m.player && *spell == m.spell)
            .map(|(_, _, victim)| name(&names, *victim))
            .collect();
        line = Some(magic_headline(
            &name(&names, m.player),
            m.spell,
            &hit,
            &held,
        ));
    }
    // 门那一句（P13b②）。它**追加**在上面那几条后面而不是把它们换掉：顶栏只有一行、
    // 最后写的赢，而「事情落到谁头上」与「谁花了一张卡」是同一帧里的两半话 ——
    // 换掉就等于替玩家宣布「刚才那一下没发生」。
    for m in notices.passive_used.read() {
        line = Some(append_clause(
            line,
            &passive_clause(
                &name(&names, m.player),
                &m.card.title,
                m.event.label(),
                m.moved_to.map(|to| name(&names, to)).as_deref(),
                m.saved,
            ),
        ));
    }
    for m in notices.over.read() {
        line = Some(match m.winner.and_then(|w| names.get(w).ok()) {
            Some(winner) => format!("胜者：{}！", winner.0),
            None => "游戏结束".to_string(),
        });
    }

    if let Some(line) = line
        && let Ok(mut text) = hints.single_mut()
    {
        *text = Text::new(line);
    }
}

// ===== P11：新闻那一句 =====

/// 一条**人档**新闻在顶栏那一行怎么说。抽成纯函数（与 P6e 那六条同一个理由）：顶栏只有一行，
/// 而它有四种形状 —— 点到一个人、点到全场同样一下、点到全场各不相同、谁都没点到。
/// 不抽出来就只能靠截图去撞那四格，撞不上的那一格在验收里读起来和「没做」一样。
///
/// `hits` 是 `(人名, 效果句)`，由调用方从 `NewsHit` 查好名字再传进来（人名在表现层，
/// 规则层的凭据里只有 `Entity`）。
///
/// `scope` 只用来选那**两个名词**：「全场 2 人各…」里的「全场」，与「场上没有步行的人」里的
/// 「步行的人」。它必须进来而不是写死成「人」/「可点名的人」—— P11b② 之后一条筛过人的新闻
/// 播出来时，「场上没有可点名的人」是一句假话（它筛的是有车的人，而今天没人开车），
/// 而那句假话与「按钮坏了」在玩家眼里是同一件事。
fn news_headline(title: &str, scope: NewsScope, hits: &[(String, String)]) -> String {
    let head = format!("号外「{title}」");
    let (crowd, none) = match scope {
        NewsScope::Everyone => ("全场".to_string(), "人".to_string()),
        NewsScope::One(_) => ("全场".to_string(), "可点名的人".to_string()),
        // 筛过人的那一种：两句是同一个词（「步行的人 2 人各…」/「场上没有步行的人」）。
        // 这个词的映射住在规则层（`FieldFilter::label`），这里不抄一份 —— 与「地名→人话」
        // 只写在 `DetentionPlace` 同一条纪律
        NewsScope::EveryoneIf(filter) => (filter.label(), filter.label()),
        // 地档不从这里过（它走 [`news_ground_headline`]）。这一格写出来是为了这张表不许有 `_`：
        // 有人把人档的句子接到地档上时，症状得是一句念得出的话而不是编译期之外的空白
        NewsScope::Around => ("这一带".to_string(), "地可打".to_string()),
        // 机构档也不从这里过（它走 [`news_institution_headline`]）。同上：这一格存在的理由
        // 是这张表不许有 `_`，接错档时症状得是一句念得出的话
        NewsScope::Institution(_) => ("全场".to_string(), "机构可停".to_string()),
    };
    match hits.len() {
        // 一条新闻播出去而场上没人被点到：这不是「什么都没发生」的静默失效，而是
        // **点名点不到人**（只剩他一个人时的 `random_other`）或**筛到零个**（没人开车）。
        // 要说出来，否则玩家会以为按钮坏了 —— 与灰按钮必须自带一句为什么是同一条纪律
        0 => format!("{head}：场上没有{none}"),
        1 => format!("{head}：{} {}", hits[0].0, hits[0].1),
        n => {
            // 全场类新闻的绝大多数形状是「每个人都挨同样一下」，那一句就该念成一句
            let same = hits.iter().all(|(_, desc)| desc == &hits[0].1);
            if same {
                format!("{head}：{crowd} {n} 人各 {}", hits[0].1)
            } else {
                let who = hits
                    .iter()
                    .map(|(name, desc)| format!("{name} {desc}"))
                    .collect::<Vec<_>>()
                    .join("、");
                format!("{head}：{n} 人 —— {who}")
            }
        }
    }
}

/// 一条**地档**新闻在顶栏那一行怎么说（P11b①）。与人档分成两个函数而不是给上面那个
/// 加参数：两档要念的东西没有交集 —— 人档念「谁、各挨多少」，地档念「哪几块、有没有连地
/// 一起收」，合成一个带可选参数的函数会得到「人名列表与地块列表都为空」那种说不出话的形状。
///
/// 「哪几块」是这条的存在理由：PLAN §四 那句「外星人拆了哪几块」要的就是地块名列表，
/// 而 `Razed{lands: 2}` 那个数答不出「我家那块在不在里面」。
///
/// 两种单块形状分开走（P11c① 加的判据）：**锚点自己就在清单里**时（`One` 那一档的征收与
/// 山洪），它就是这句话的主语，直接说「西单 夷为平地…」；锚点只是中心、被打的是旁边那几块
/// 时（公有地拍卖拍的是**无主**的那些，中心那块常常正主人在场），主语得换成清单 ——
/// 否则念出来是「王府井 把西单挂上拍卖台」，一句把两个地块绑错的话。
fn news_ground_headline(title: &str, center: Option<&str>, lands: &[String], deed: &str) -> String {
    let head = format!("号外「{title}」");
    match center {
        // 锚点算不出来 = 这张盘上一块地都没有。与「场上没有可点名的人」同一条纪律：
        // 空手要说得出为什么空，否则玩家以为自己按错了
        None => format!("{head}：这一带没有地可打"),
        Some(at) if lands.is_empty() => format!("{head}：{at} 一带没有地"),
        Some(at) if lands.len() == 1 && lands[0] == at => format!("{head}：{at} {deed}"),
        Some(at) => format!(
            "{head}：{at} 一带 {} 块 —— {} {deed}",
            lands.len(),
            lands.join("、")
        ),
    }
}

/// 一条**机构档**新闻在顶栏那一行怎么说（P11c②）：「号外「股市临时停牌」：股市 停牌 5 天」。
///
/// 第三个函数而不是给上面两个加参数：人档念「谁、各挨多少」，地档念「哪几块」，
/// 这一档念「哪一家、停几天、哪天恢复」。三档要的东西没有交集，合成一个带可选参数的
/// 函数就会得到「人名、地块、机构全为空」那种说不出话的形状（与 [`news_ground_headline`]
/// 文件头那条同一条理由）。
///
/// 天数读的是**世界上**那一个（`NewsAudience::Institution::days`），所以叠加那天
/// 这句话与浮层那句「剩 N 天」是同一个数。
fn news_institution_headline(title: &str, what: Institution, days: u32) -> String {
    format!(
        "号外「{title}」：{} {} {days} 天",
        what.label(),
        what.halt_phrase()
    )
}

// ===== P6e①：神明那六句话 =====
// 抽成纯函数的理由：顶栏只有一行，读回来要起一整个 App；而其中五条都带一个「有没有
// 那个尾巴」的分叉（顶走了谁、原来在谁名下、有没有卡、钱是进还是出），不抽出来就只能
// 靠截图去撞那些分支 —— 撞不上的那一条在验收里读起来和「没做」一样。

/// 一位神上路。它不吃鼠标、不弹模态，顶栏这一行是唯一的出口（dump 那一栏要开面板才看得见）
fn god_appeared_word(god: &str, tile: &str) -> String {
    format!("{god} 上了路，站在 {tile}")
}

/// 三种走法三句话。core 那个 `label` 是「期满／顶走／送走」三个短动词，dump 与测试流水
/// 用它；这里要的是能独立读的一句人话，所以「被人弄走的」那两种要补一个「被」字 ——
/// 「顶走走了」不是话
fn god_left_word(who: &str, god: &str, reason: GodExit) -> String {
    let verb = match reason {
        GodExit::Expired => "期满走了",
        GodExit::Replaced => "被顶走了",
        GodExit::Banished => "被送走了",
    };
    format!("{who} 身上的{god}{verb}")
}

/// 上身。`days` 由这条凭据带着，所以这一行与 HUD 那句「还剩几天」读的是同一个数
fn god_possessed_word(who: &str, god: &str, days: u32, replaced: Option<&str>) -> String {
    let tail = replaced.map_or(String::new(), |old| format!("，顶走了{old}"));
    format!("{who} 被{god}附身（{days} 天）{tail}")
}

/// 丢卡。主语是**哪位神**而不是「系统」：这条消息存在的全部理由就是说得出是谁干的
fn card_lost_word(by: &str, who: &str, card: &str) -> String {
    format!("{by} 从 {who} 手里弄走了「{card}」")
}

/// 土地公把一块地直接拿走。主语写死在他身上：`LandTaken` 今天唯一的发源处是
/// `Books::take_land`，而它只被土地公那条落脚链调用。哪天另一位也能直接拿地，要回来给
/// 这条凭据加一个 `god` 字段，不是在这里补一个 `if`。
/// 「原来在谁名下」是这一句的全部信息量 —— 没有它，玩家只会看见地换了人
fn land_taken_word(who: &str, plot: &str, from: Option<&str>) -> String {
    let tail = from.map_or("这块地本来无主".to_string(), |prev| {
        format!("原来在 {prev} 名下")
    });
    format!("土地公 替 {who} 拿下{plot}（{tail}）")
}

/// 上身那一刻当场做的那件事。「给了」与「拿走了」分两句而不是带符号报数：负数写在屏幕上
/// 读起来像账算错了，而这里说的是一笔真的动过的钱（明细在 `CashChanged`，两边对得上）。
/// `cards` 只在钱那一格为零时才被念出来 —— 大福神是「送 2 张卡」，不送钱
fn god_deed_word(god: &str, who: &str, cash: i64, cards: usize) -> String {
    let tail = if cards == 0 {
        String::new()
    } else {
        format!("，还有 {cards} 张卡")
    };
    if cash > 0 {
        format!("{god} 给了 {who} ${}{}", comma(cash), tail)
    } else if cash < 0 {
        format!("{god} 从 {who} 手里拿走 ${}{}", comma(-cash), tail)
    } else {
        format!("{god} 塞给 {who} {cards} 张卡")
    }
}

/// 银行操作的中文动词。`BankOp` 在规则层是枚举，屏幕上要给人话，映射属于表现层
/// （与 `tile_info::kind_name` 同一条分工）。
fn op_verb(op: game_core::prelude::BankOp) -> &'static str {
    use game_core::prelude::BankOp;
    match op {
        BankOp::Deposit => "存了",
        BankOp::Withdraw => "取了",
        BankOp::Borrow => "借了",
        BankOp::Repay => "还了",
    }
}

fn side_verb(side: game_core::prelude::TradeSide) -> &'static str {
    use game_core::prelude::TradeSide;
    match side {
        TradeSide::Buy => "买入",
        TradeSide::Sell => "卖出",
    }
}

fn name(names: &Query<&PlayerName>, player: Entity) -> String {
    names
        .get(player)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| "玩家".to_string())
}

// ===== P13：魔法屋那一句 =====

/// 一道法术施完之后，顶栏那一行怎么说（P13）。
///
/// 抽成纯函数与 P6e／P11 那几条同一个理由：顶栏只有一行，而这一句有**四种**形状
/// （落到人头上／顺带有人挡下／全被挡下／一样都没落到），只有纯函数能把四种各钉一条。
/// 「落空」与「被挡下」不许共用一句话：前者是「这一道今天没东西可打」，后者是
/// 「有人花掉了一张免罪卡」—— 玩家听完这两句会做不同的下一手。
fn magic_headline(caster: &str, spell: MagicSpell, hit: &[String], blocked: &[String]) -> String {
    let asked = format!("{} 求的「{}」", caster, spell.label());
    match (hit.is_empty(), blocked.is_empty()) {
        (false, true) => format!("{asked} 落到 {} 头上", hit.join("、")),
        (false, false) => format!(
            "{asked} 落到 {} 头上；{} 用免罪卡挡下了那一下",
            hit.join("、"),
            blocked.join("、")
        ),
        (true, false) => format!("{asked} 被 {} 用免罪卡整笔挡下", blocked.join("、")),
        (true, true) => format!("{asked} 落空 —— 一样都没落到人头上"),
    }
}

/// 一句话往已有那行后面追加。`None` 时它就是这一行（这一帧没有别的事件在说）。
fn append_clause(prev: Option<String>, clause: &str) -> String {
    match prev {
        Some(line) => format!("{line}；{clause}"),
        None => clause.to_string(),
    }
}

/// 门那一句怎么说（P13b②，③ 补上钱那一半）。两个形状对应门上两个动作：
/// **抵消**（「挡下了」—— 这一次不落下来）与**嫁祸**（「转给」—— 这一次落到别人头上）。
/// 两句话的差别是下一手：挡下的那位自己脱身，转走的那位要知道**是谁**接住了这一下。
/// 放弃不发这一条（什么都不花，所以没有新事实要说 —— 事件照原样落那一句由各个来路
/// 自己的凭据讲）。抽成纯函数与 P13/P11 那几条同一个理由：一行话有两种形状，
/// 截图撞不出那两格。
///
/// 第三种形状是**价签**：`saved > 0` 只在账那一侧成立（坐牢没有价签），所以「这是事还是钱」
/// 不用再带一个位 —— 「他花了一张卡」这句话在两种东西上要说的是「挡下了那一次」还是
/// 「免掉了那笔多少」，而后者那句里的数是玩家判断下一张该不该留着的唯一依据。
fn passive_clause(
    who: &str,
    card: &str,
    event: &str,
    moved_to: Option<&str>,
    saved: i64,
) -> String {
    let (verb, what) = if saved > 0 {
        ("免掉", format!("那笔 {}", comma(saved)))
    } else {
        ("挡下", format!("那一次「{event}」"))
    };
    match moved_to {
        None => format!("{who} 花一张「{card}」{verb}了{what}"),
        Some(to) => format!("{who} 花一张「{card}」把{what}转给了 {to}"),
    }
}

/// 「5 南京路」—— 顶栏那几行要说得出格名，否则「放在 5 号格」还得玩家自己数。
/// 棋盘缺席时退化成格号，不猜名字。
fn tile_word(board: &Option<Res<ActiveBoard>>, tile: TileIndex) -> String {
    board
        .as_ref()
        .and_then(|b| b.def.tile(tile.0 as i64))
        .map_or_else(
            || format!("{} 号格", tile.0),
            |t| format!("{} {}", tile.0, t.name),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;
    /// 设施的类别只在用例里需要点名（生产代码从消息里读的就是它）
    use game_core::prelude::BuildingKind;
    use game_core::prelude::{
        CardKind, CardNotice, DetentionPlace, EffectOutcome, FieldFilter, GameKind, GameRules, God,
        LandId, NewsFired, NewsHit, NewsNotice, NewsScope, PassiveEvent, PlayerId, PlayerPick,
        StockId, TradeSide, TurnOrder,
    };

    /// P14e：阶段标签在电脑的回合里要多写「是谁、哪档性格」。
    ///
    /// 它比旁白重要：旁白在同一阶段里会被事件流水盖掉，而阶段标签每帧都在。
    #[test]
    fn the_phase_label_names_the_computer_on_its_turn() {
        assert_eq!(
            phase_label_word(TurnPhase::WaitDecision, Some("莱息"), Some("大佬奸")),
            "等待决策 · 莱息（电脑·大佬奸）"
        );
        assert_eq!(
            phase_label_word(TurnPhase::WaitRoll, Some("姬百年"), None),
            "等待掷骰"
        );
        assert_eq!(phase_label_word(TurnPhase::Walking, None, None), "行走中");
    }

    /// P14e：旁白要分得清「电脑正在想」与「请掷骰」。
    ///
    /// 两句话是给两种读者的：人类座位那句是一条指令，电脑座位那句是状态。
    /// 受雇的恶人仍然先说「替谁办事」—— 那不是电脑，是 NPC 的第三种身份。
    #[test]
    fn the_head_word_says_who_is_thinking() {
        assert_eq!(
            head_word(Some("莱息"), None, Some("大佬奸")),
            "莱息 的回合 — 电脑（大佬奸）正在想…"
        );
        assert_eq!(
            head_word(Some("姬百年"), None, None),
            "姬百年 的回合 — 请掷骰"
        );
        assert_eq!(
            head_word(Some("强盗"), Some("姬百年"), None),
            "强盗 替 姬百年 办事 — 他自己掷骰"
        );
        assert_eq!(head_word(None, None, None), "请掷骰");
    }

    /// 顶栏刷新只许动顶栏名下的按钮（§6.1 坑 31）。
    ///
    /// 这条判据是被真鼠标冒烟翻出来的：手牌浮层一重建，日志里就冒出
    /// `remove<Disabled>` 打在已 despawn 实体上的报错，而暂停时该灰的卡面按钮灰不住。
    /// 半个工程都不该由顶栏代劳 —— 所以断言两头：浮层那颗不许被碰，顶栏那颗该变还得变。
    /// 浮层那颗用证券下单的动作（B5b 就有，比拿新阶段的动作当道具更经得起回头跑历史）。
    #[test]
    fn the_top_bar_refresh_only_touches_the_top_bar() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StatesPlugin)
            .init_state::<TurnPhase>()
            .insert_resource(Paused(false))
            .insert_resource(PendingDecision::default())
            .insert_resource(GameSession {
                turn_count: 0,
                seed: 0,
                price_index: 1.0,
                epoch: game_core::prelude::Date::new(2048, 1, 1),
                last_gift_year: None,
                last_envelope_year: None,
                last_news: None,
                suspensions: Default::default(),
                bank_vault: 0,
            })
            .insert_resource(MarketClock::default())
            .insert_resource(GameRules::default())
            .insert_resource(AtmAccess::default())
            .add_systems(Update, refresh_top_bar);

        let player = app.world_mut().spawn(PlayerId(0)).id();
        app.world_mut()
            .insert_resource(TurnOrder::new(vec![player]));

        let bar = app.world_mut().spawn(TopBar).id();
        let atm = app
            .world_mut()
            .spawn((
                ChildOf(bar),
                UiAction::ToggleAtm,
                BackgroundColor(SECONDARY.0),
                Palette {
                    normal: SECONDARY.0,
                    hover: SECONDARY.1,
                },
            ))
            .id();
        // 浮层里的一颗：spawn 期就灰着（灰的理由由那块面板自己算），且不在顶栏名下
        let panel_btn = app
            .world_mut()
            .spawn((
                UiAction::Trade {
                    stock: StockId("stock_001".to_string()),
                    side: TradeSide::Buy,
                    shares: 10,
                },
                BackgroundColor(col::BTN_DISABLED),
                Palette {
                    normal: col::BTN_DISABLED,
                    hover: col::BTN_DISABLED,
                },
                Disabled,
            ))
            .id();

        for _ in 0..3 {
            app.update();
        }
        let world = app.world();
        assert!(
            world.get::<Disabled>(panel_btn).is_some(),
            "顶栏把浮层按钮的 `Disabled` 剥了 —— 查询范围又放宽到全工程了？"
        );
        assert!(
            world.get::<Disabled>(atm).is_some(),
            "没发卡时顶栏的 ATM 该是灰的 —— 刷新本身没跑？"
        );

        // 反向半边：卡到手，顶栏那颗必须亮起来（否则上面两条断言只是「什么都没做」）
        app.world_mut().resource_mut::<AtmAccess>().0 = Some(player);
        for _ in 0..3 {
            app.update();
        }
        let world = app.world();
        assert!(
            world.get::<Disabled>(atm).is_none(),
            "ATM 卡到手后顶栏按钮仍然灰着"
        );
        assert!(
            world.get::<Disabled>(panel_btn).is_some(),
            "浮层按钮又被顶栏动了"
        );
    }

    /// 三种走法都得是一句能独立读的话。core 那个 `label` 是短动词（期满／顶走／送走），
    /// 直接拼进「…走了」会得到「顶走走了」—— 而那正是照抄 label 时最容易写出来的一句
    #[test]
    fn the_three_exits_each_read_as_a_sentence() {
        assert_eq!(
            god_left_word("阿土伯", "大财神", GodExit::Expired),
            "阿土伯 身上的大财神期满走了"
        );
        assert_eq!(
            god_left_word("阿土伯", "小财神", GodExit::Replaced),
            "阿土伯 身上的小财神被顶走了"
        );
        assert_eq!(
            god_left_word("阿土伯", "小衰神", GodExit::Banished),
            "阿土伯 身上的小衰神被送走了"
        );
    }

    /// 附身那句的两个分支：身上原本没神 vs 顶走一位。少了后者，玩家不知道自己损失了谁
    #[test]
    fn possession_names_the_god_it_pushed_out() {
        assert_eq!(
            god_possessed_word("阿土伯", "大财神", 7, None),
            "阿土伯 被大财神附身（7 天）"
        );
        assert_eq!(
            god_possessed_word("阿土伯", "大财神", 7, Some("小财神")),
            "阿土伯 被大财神附身（7 天），顶走了小财神"
        );
    }

    /// 拿下那块地时「原来在谁名下」是这一句的全部信息量。写成 `None` 那一支也要有话说 ——
    /// 无主地被拿走的画面与有人名下被拿走长得一模一样，而玩家会以为是系统 bug
    #[test]
    fn a_taken_plot_says_whose_it_was_or_that_it_was_nobody_s() {
        assert_eq!(
            land_taken_word("孙小美", "汉口路", Some("阿土伯")),
            "土地公 替 孙小美 拿下汉口路（原来在 阿土伯 名下）"
        );
        assert_eq!(
            land_taken_word("孙小美", "汉口路", None),
            "土地公 替 孙小美 拿下汉口路（这块地本来无主）"
        );
    }

    /// 钱那一格永不出现负号：`GodDeed.cash` 是**净值**（进为正出为负），照抄到屏幕上会得到
    /// 「给了 -300」，读起来像账算错了而不是像被抢了
    #[test]
    fn a_deed_reads_the_direction_instead_of_the_sign() {
        assert_eq!(
            god_deed_word("大财神", "阿土伯", 3_521, 0),
            "大财神 给了 阿土伯 $3,521"
        );
        let taken = god_deed_word("大穷神", "阿土伯", -300, 0);
        assert_eq!(taken, "大穷神 从 阿土伯 手里拿走 $300");
        assert!(!taken.contains('-'), "负号漏到屏幕上了：{taken}");
    }

    /// 只送卡的那一位（大福神 2 张、小福神 1 张）钱那一格是 0，那句必须说卡而不是「给了 0 元」
    #[test]
    fn a_card_only_deed_says_cards_not_zero_dollars() {
        assert_eq!(
            god_deed_word("大福神", "阿土伯", 0, 2),
            "大福神 塞给 阿土伯 2 张卡"
        );
    }

    /// 上路与丢卡两句各钉一个主语：前者的主语是神（没有人），后者的主语是**哪位神**
    #[test]
    fn appearances_and_losses_name_their_actor() {
        assert_eq!(
            god_appeared_word("大衰神", "南京路"),
            "大衰神 上了路，站在 南京路"
        );
        assert_eq!(
            card_lost_word("衰神", "阿土伯", "购地卡"),
            "衰神 从 阿土伯 手里弄走了「购地卡」"
        );
    }

    // ===== 那六条真的接到顶栏上（ wiring ）=====
    // 上面那六条纯函数用例**测不到接线**：把某个 `for m in notices.xxx.read()` 整块删掉，
    // 它们照样全绿，而屏幕上那一句从此永远不出现 —— 这正是「空跑的断言等于没断言」的
    // 第六种形状。所以这里把消息真发一遍，读回顶栏那一行（0.20 的 `Text` 就是
    // `bevy_ui::widget::text::Text(pub String)`，读得回来）

    fn notices_app() -> (App, Entity, Entity, Entity) {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            // `Notices` 读几条就要注册几条 —— 少一条，参数校验直接 panic，
            // 而不是安静地少播一句
            .add_message::<DetentionSkipped>()
            .add_message::<DetentionLifted>()
            .add_message::<PlayerBailed>()
            .add_message::<StaySkipped>()
            .add_message::<StayLifted>()
            .add_message::<PlayerBankrupt>()
            .add_message::<BankVisited>()
            .add_message::<BankOpDone>()
            .add_message::<StockVisited>()
            .add_message::<StockTraded>()
            .add_message::<PointsChanged>()
            .add_message::<MinigameScore>()
            .add_message::<LotteryBought>()
            .add_message::<LotteryDrawn>()
            .add_message::<LotteryWon>()
            // 拍卖那五条（P9）。同一条纪律：少注册一条不是少一行字，是 `Notices` 参数
            // 校验直接 panic —— 而 panic 恰好是我们要的（它说明「接线」这件事真的被测着）
            .add_message::<AuctionStarted>()
            .add_message::<BidPlaced>()
            .add_message::<BidWithdrawn>()
            .add_message::<AuctionWon>()
            .add_message::<AuctionFailed>()
            .add_message::<ShopOpDone>()
            .add_message::<MortgageDone>()
            .add_message::<CardPlayed>()
            .add_message::<GodAppeared>()
            .add_message::<GodPossessed>()
            .add_message::<GodLeft>()
            .add_message::<GodDeed>()
            .add_message::<CardLost>()
            .add_message::<LandTaken>()
            .add_message::<ItemPlaced>()
            .add_message::<WalkHalted>()
            .add_message::<BombTransferred>()
            .add_message::<ChestOpened>()
            .add_message::<RemoteDiceUsed>()
            .add_message::<MoveCapped>()
            .add_message::<MountChanged>()
            // P10c：商业设施那两条（建成与收费）。同样属于「漏了整块播报建不出来」
            .add_message::<FacilityBuilt>()
            .add_message::<ServiceCharged>()
            // P10d①：公司到访那条
            .add_message::<CompanyCharged>()
            // P10d②：保险与董事长特权的五条播报源（漏任何一条 = Notices 参数包
            // 建不出来，症状是整个顶栏一行字都不更新）
            .add_message::<PolicyBought>()
            .add_message::<IndemnityPaid>()
            .add_message::<ChairmanTook>()
            .add_message::<FinancingDrawn>()
            .add_message::<FinancingRecalled>()
            // P11：新闻那一条（漏了它 = `Notices` 参数包建不出来 = 顶栏整行不更新）
            .add_message::<NewsFired>()
            // P12b–d：四大恶人的六条播报源（同一条理由 —— 漏了参数包建不出来）
            .add_message::<VillainHired>()
            .add_message::<DepositsRobbed>()
            .add_message::<PointsStolen>()
            .add_message::<ItemThieved>()
            .add_message::<CardSnatched>()
            .add_message::<TollExtorted>()
            .add_message::<RentRerouted>()
            // P13：魔法屋的两条凭据（施法与「被免罪挡下」各一条）。同一条坑：漏了
            // `Notices` 参数包建不出来，顶栏整行不再更新 —— 症状不是「那一句没播」
            .add_message::<MagicCast>()
            .add_message::<MagicBlocked>()
            // P13b②：门花掉一张被动卡那一条（「谁花了哪张、转给了谁」都在这一条上）。
            // 同一条坑的第五遍：漏了它 = `Notices` 参数包建不出来 = 顶栏整行不更新
            .add_message::<PassiveUsed>()
            .add_message::<GameOver>()
            .add_systems(Update, hint_from_events);
        let me = app.world_mut().spawn(PlayerName("阿土伯".into())).id();
        let foe = app.world_mut().spawn(PlayerName("孙小美".into())).id();
        let hint = app.world_mut().spawn((HintText, Text::new(""))).id();
        (app, me, foe, hint)
    }

    fn send<M: bevy::ecs::message::Message + Clone + Send + Sync + 'static>(app: &mut App, m: M) {
        app.world_mut().resource_mut::<Messages<M>>().write(m);
    }

    fn hint_of(app: &mut App, hint: Entity) -> String {
        app.update();
        app.world().get::<Text>(hint).expect("提示行在").0.clone()
    }

    fn notice(title: &str) -> CardNotice {
        CardNotice {
            id: "item_summon".into(),
            kind: CardKind::Item,
            title: title.into(),
            desc: "测试".into(),
            outcomes: vec![],
        }
    }

    /// 六条各播一句。逐条单帧发，不混在一起 —— 混了就分不清是哪条 reader 读到的
    #[test]
    fn each_god_notice_reaches_the_top_bar_line() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            GodAppeared {
                god: God::SmallDoom,
                tile: TileIndex(3),
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("小衰神 上了路"),
            "上路那条没接到顶栏"
        );

        send(
            &mut app,
            GodPossessed {
                player: me,
                god: God::BigRich,
                days: 7,
                tile: TileIndex(1),
                replaced: None,
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("阿土伯 被大财神附身（7 天）"),
            "附身那条没接到顶栏"
        );

        send(
            &mut app,
            GodLeft {
                player: me,
                god: God::BigRich,
                reason: GodExit::Banished,
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("大财神被送走了"),
            "离场那条没接到顶栏"
        );

        send(
            &mut app,
            GodDeed {
                player: me,
                god: God::BigRich,
                cash: 3_521,
                cards: vec![],
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("大财神 给了 阿土伯 $3,521"),
            "当场那件事没接到顶栏"
        );

        send(
            &mut app,
            CardLost {
                player: me,
                card: notice("购地卡"),
                by: God::SmallDoom,
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("小衰神 从 阿土伯 手里弄走了「购地卡」"),
            "丢卡那条没接到顶栏"
        );

        send(
            &mut app,
            LandTaken {
                player: foe,
                land: LandId(1),
                from: Some(me),
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("土地公 替 孙小美 拿下"),
            "拿地那条没接到顶栏"
        );
        // 无主那一支走同一句的另一头：`from: None` 必须说「本来无主」而不是漏掉那半句
        send(
            &mut app,
            LandTaken {
                player: foe,
                land: LandId(1),
                from: None,
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("这块地本来无主"),
            "无主那一支没接到顶栏"
        );
    }

    /// 顺序的两格：请神符那一帧同时有 `CardPlayed` 与 `GodPossessed`，留下的必须是出牌
    /// 那句（它已经说了请来谁）；而走路踩到神那一帧只有 `GodPossessed` 与 `GodDeed`，
    /// 留下的是当场那件事。两格各钉一头 —— 只钉一头的话，把整块挪到 `played` 之前
    /// 或之后都不会红
    #[test]
    fn the_played_line_wins_over_the_possession_while_the_deed_wins_within_the_gods() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            GodPossessed {
                player: me,
                god: God::SmallRich,
                days: 7,
                tile: TileIndex(6),
                replaced: None,
            },
        );
        send(
            &mut app,
            CardPlayed {
                player: me,
                victim: me,
                card: notice("请神符"),
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "阿土伯 使用了「请神符」",
            "出牌那句被神的状态盖掉了"
        );

        send(
            &mut app,
            GodPossessed {
                player: me,
                god: God::SmallRich,
                days: 7,
                tile: TileIndex(6),
                replaced: None,
            },
        );
        send(
            &mut app,
            GodDeed {
                player: me,
                god: God::SmallRich,
                cash: -300,
                cards: vec![],
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "小财神 从 阿土伯 手里拿走 $300",
            "同族里该留下更具体的那件事"
        );
    }

    /// 游乐场那一局（P7）：成绩行要说得出「哪一款、几分、换了多少」，而它排在通用点券
    /// 流水**之后** —— 同一帧两条都会写，留下的该是更具体的那条（与百货、保释同一条分工）。
    #[test]
    fn an_arcade_score_wins_over_the_generic_points_line() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            PointsChanged {
                player: me,
                delta: 48,
                balance: 148,
            },
        );
        send(
            &mut app,
            MinigameScore {
                player: me,
                game: GameKind::PopBalloons,
                score: 12,
                points: 48,
                awarded: true,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "阿土伯 玩「打气球」得了 12 分，换 48 点券",
            "点券那行盖住了成绩 —— 玩家看不出这 48 是玩出来的还是捡来的"
        );
    }

    /// 0 分也得播：那一屏下一刻就收走了，而「玩了半天一分没换到」正是玩家最要一句解释的
    /// 场合（沉默会被读成「按钮是死的」，§6.1 坑 27 那一族）。
    #[test]
    fn a_scoreless_round_still_says_so() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            MinigameScore {
                player: me,
                game: GameKind::PenguinDig,
                score: 0,
                points: 0,
                awarded: true,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "阿土伯 玩「企鹅挖宝」得了 0 分，换 0 点券"
        );
    }

    /// 乐透（P8）：开奖那一句要区分「有人猜中」与「流局」，因为流局不是一句「什么都没发生」——
    /// 那一期的钱还躺在池里，而玩家下一期要不要再买一注读的就是这个数。
    /// 有人猜中那一句**点名**：号码独占之后中奖号只有一位持有者，「独得」说的是他。
    #[test]
    fn a_draw_announces_both_outcomes() {
        let (mut app, _me, _foe, hint) = notices_app();
        send(
            &mut app,
            LotteryDrawn {
                number: 7,
                winner: None,
                pot: 3_000,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "乐透开出 7 号 —— 没人猜中，奖池 3000 滚到下月"
        );

        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            LotteryDrawn {
                number: 7,
                winner: Some(me),
                pot: 3_000,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "乐透开出 7 号 —— 阿土伯 独得奖池 3000"
        );
    }

    /// 中奖那一行盖在开奖那一行之上：同一帧两条都写，留下的该是「谁拿到了钱」——
    /// 与百货/保释盖过通用点券流水同一条分工。
    #[test]
    fn the_winner_line_wins_over_the_draw_line() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            LotteryDrawn {
                number: 11,
                winner: Some(me),
                pot: 2_000,
            },
        );
        send(
            &mut app,
            LotteryWon {
                player: me,
                number: 11,
                amount: 2_000,
            },
        );
        assert_eq!(hint_of(&mut app, hint), "阿土伯 猜中 11 号，领到 2000");
    }

    /// 买券那一句在**没有开奖**的帧里也得留得住（一注买下去到开奖之间隔着几十回合，
    /// 那几十回合里玩家回头要找的是「我到底买成过哪几注」）。
    #[test]
    fn a_ticket_purchase_says_which_number() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            LotteryBought {
                player: me,
                number: 23,
                price: 1_000,
            },
        );
        assert_eq!(hint_of(&mut app, hint), "阿土伯 花 1000 买下一注 23 号");
    }

    /// `awarded` 那一位必须在顶栏上看得出来。练习局走的是 UI 本地那条路（不发这条凭据），
    /// 但规则层这一支是真的、也被 `game_core/tests/minigame.rs` 钉着 —— 播报侧不能把它
    /// 说得跟真给钱了一样，否则「不计点券」那句就只活在结算屏上
    #[test]
    fn an_unawarded_score_is_announced_as_practice() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            MinigameScore {
                player: me,
                game: GameKind::CatchTreasure,
                score: 9,
                points: 0,
                awarded: false,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "阿土伯 玩「接财宝」得了 9 分 —— 练习局，不计点券"
        );
    }

    /// 商业设施那两句（P10c）。收据面板下一刻就被关掉，服务费那笔的**唯一持久读数**
    /// 就是这一行 —— 所以它要说得出「哪座设施、算了多少」，而不只是「钱包少了一截」。
    /// 两种算账（A 型摇档、B 型住店）各一句：合成一句会让「住了几天」消失，
    /// 而那是玩家下一次要不要绕开这座旅馆的唯一依据。
    #[test]
    fn a_service_charge_says_which_facility_and_how_it_was_priced() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            ServiceCharged {
                payer: me,
                owner: foe,
                land: LandId(2),
                kind: BuildingKind::Mall,
                amount: 600,
                roll: 3,
                stay_days: 0,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "阿土伯 付 600 在购物中心（摇出 3）"
        );

        // B 型：天数与金额是两件事，而这一行说的是前者（后者走现金流水与收据）
        send(
            &mut app,
            ServiceCharged {
                payer: me,
                owner: foe,
                land: LandId(5),
                kind: BuildingKind::Hotel,
                amount: 1_500,
                roll: 3,
                stay_days: 3,
            },
        );
        assert_eq!(hint_of(&mut app, hint), "阿土伯 付 1,500 在旅馆住了 3 天");

        // 免单那一笔（大财神/免费卡）不能说成「付 0」：那是一句谎话
        send(
            &mut app,
            ServiceCharged {
                payer: me,
                owner: foe,
                land: LandId(2),
                kind: BuildingKind::Mall,
                amount: 0,
                roll: 2,
                stay_days: 0,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "分文不取 在购物中心（摇出 2）",
            "金额 0 读起来像租金表坏了"
        );
    }

    /// 公司到访那几句（P10d①）。董事长免单那句必须说得出「为什么免费」——
    /// 金额 0 读起来像租金表坏了，与设施免单那一句同一条理由。
    /// 夹具没有棋盘，公司名落回股票 id：这里验的是三件事都在（人、免费/金额、去向）。
    #[test]
    fn a_company_visit_says_why_it_was_free_or_how_much() {
        let (mut app, me, _foe, hint) = notices_app();
        // 董事长免单：金额 0 + 董事长是自己
        send(
            &mut app,
            CompanyCharged {
                payer: me,
                stock: "stock_001".into(),
                chairman: Some(me),
                amount: 0,
                roll: 0,
                stay_days: 0,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("阿土伯") && said.contains("董事长") && said.contains("免费通行"),
            "免单收据那一行没接上：{said}"
        );

        // 访客付费：金额要说出来（按步数计费的那一类）
        send(
            &mut app,
            CompanyCharged {
                payer: me,
                stock: "stock_001".into(),
                chairman: Some(_foe),
                amount: 240,
                roll: 0,
                stay_days: 0,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("阿土伯") && said.contains("240"),
            "付费到访那一行没接上：{said}"
        );

        // 航空那类按天：天数与金额是两件事，这一行说的是前者
        send(
            &mut app,
            CompanyCharged {
                payer: me,
                stock: "stock_005".into(),
                chairman: None,
                amount: 1_200,
                roll: 3,
                stay_days: 3,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("1,200") && said.contains("住了 3 天"),
            "按天计费那一行没接上：{said}"
        );
    }

    /// 建成那一句要说得清「谁、在哪、盖了什么、花了多少」—— 四样缺一样，玩家第二天
    /// 就会来问「我这块地怎么突然有楼了」。
    #[test]
    fn a_new_facility_says_where_and_what_it_cost() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            FacilityBuilt {
                player: me,
                land: LandId(2),
                kind: BuildingKind::Hotel,
                cost: 2_200,
            },
        );
        // 夹具里没有棋盘，所以地块名落在「？」—— 这一句要验的是「四样都在」
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("阿土伯") && said.contains("建起了旅馆") && said.contains("造价 2,200"),
            "建成那一行没接上：{said}"
        );
    }

    /// 开场那一句要分得清两种入口：一个是「某人敲的槌、钱归他」，一个是「破产变卖、钱归银行」。
    /// 这两句在画面上长得一模一样（都是「一块地上拍」），而玩家下一步要不要抬价，
    /// 读的就是这个区别 —— 合成一句就是谎话。
    #[test]
    fn an_opening_auction_says_who_struck_the_hammer() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            AuctionStarted {
                land: LandId(3),
                by: Some(me),
                seller: Some(foe),
                opening: 5_700,
                step: 570,
                bidders: 2,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("阿土伯 敲槌拍卖")
                && said.contains("起拍 5,700")
                && said.contains("2 人能喊"),
            "开槌的那位与起拍、台阶、能喊的人数都要在：{said}"
        );

        send(
            &mut app,
            AuctionStarted {
                land: LandId(3),
                by: None,
                seller: Some(foe),
                opening: 7_600,
                step: 760,
                bidders: 2,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("破产变卖") && said.contains("起拍 7,600"),
            "没有开槌的人时不许说成「某玩家在卖地」：{said}"
        );
    }

    /// 每一声喊价都得看得见。拍卖面板问完一句就换人，顶栏是**唯一**留下「谁喊到哪」的地方 ——
    /// 少接这一条，一场拍卖在屏幕上就与一次点击长得一样。
    #[test]
    fn every_shout_and_every_pass_reaches_the_line() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            BidPlaced {
                player: me,
                land: LandId(3),
                amount: 6_270,
            },
        );
        assert_eq!(hint_of(&mut app, hint), "阿土伯 喊到 6,270（？）");

        send(
            &mut app,
            BidWithdrawn {
                player: me,
                land: LandId(3),
            },
        );
        assert_eq!(hint_of(&mut app, hint), "阿土伯 放弃争？");
    }

    /// 落槌那一句必须带上**钱归谁**（两种入口两个答案），而流标那一句要念得出起拍价 ——
    /// 玩家问的下一句就是「起拍多少都没人要」。
    #[test]
    fn the_hammer_says_where_the_money_went_and_a_pass_says_the_opening() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            AuctionWon {
                player: me,
                land: LandId(3),
                price: 6_840,
                by: Some(foe),
                seller: None,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("阿土伯 以 6,840 拍得") && said.contains("所得归 孙小美"),
            "拍款去向要说得出：{said}"
        );

        send(
            &mut app,
            AuctionWon {
                player: me,
                land: LandId(3),
                price: 7_600,
                by: None,
                seller: Some(foe),
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("所得归银行"),
            "清算那一场不许让人以为某个玩家收到了钱"
        );

        send(
            &mut app,
            AuctionFailed {
                land: LandId(3),
                by: None,
                seller: Some(foe),
                price: 7_600,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("没人出价，流标") && said.contains("起拍 7,600"),
            "流标那一句要念得出起拍价：{said}"
        );
    }

    /// 新闻那一句的四种形状（P11）。全走 `news_headline` 这个纯函数 + 一条整屏接线：
    /// 顶栏只有一行，而「点到一个人 / 全场同样一下 / 全场各不相同 / 谁都没点到」是四句
    /// 不同的话，截图只能撞见其中一格。
    #[test]
    fn a_named_news_hits_the_named_one_only() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_002".into(),
                    title: "稽查现金大户".into(),
                    desc: "付出 $3000".into(),
                    scope: NewsScope::One(PlayerPick::Richest),
                    audience: NewsAudience::People(vec![NewsHit {
                        player: foe,
                        outcomes: vec![EffectOutcome::Lost(3_000)],
                    }]),
                },
            },
        );
        // 被点名的是**别人**：这一行不出现播报者的名字，验收①在屏幕上也是这个形状
        assert_eq!(
            hint_of(&mut app, hint),
            "号外「稽查现金大户」：孙小美 支付 $3000"
        );
    }

    /// 一条**筛过人**的新闻放出一个人（P11b② 的「医院传出好消息」）。
    ///
    /// 这一句同时钉两样：`describe` 里那条新的 `Released`（用的是 `back_verb` ——
    /// 「提前出院」而不是「解除拘留 1 次」），以及一条只点到一个人的筛过类新闻
    /// 念起来还是一句人话（它走的是「1 人」那一格，不是「全场 N 人」）。
    #[test]
    fn a_news_that_opens_the_hospital_door_says_so_in_one_sentence() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_012".into(),
                    title: "医院传出好消息".into(),
                    desc: "住院的人立刻出院".into(),
                    scope: NewsScope::EveryoneIf(FieldFilter::Detained(DetentionPlace::Hospital)),
                    audience: NewsAudience::People(vec![NewsHit {
                        player: foe,
                        outcomes: vec![EffectOutcome::Released {
                            place: DetentionPlace::Hospital,
                            junction: game_core::prelude::TileIndex(20),
                        }],
                    }]),
                },
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "号外「医院传出好消息」：孙小美 提前出院"
        );
    }

    #[test]
    fn a_whole_field_news_reads_as_one_sentence() {
        let (mut app, me, foe, hint) = notices_app();
        let hits = |who: &[Entity]| {
            who.iter()
                .map(|p| NewsHit {
                    player: *p,
                    outcomes: vec![EffectOutcome::Gained(1_500)],
                })
                .collect::<Vec<_>>()
        };
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_001".into(),
                    title: "财政补贴发放".into(),
                    desc: "每人进账 $1500".into(),
                    scope: NewsScope::Everyone,
                    audience: NewsAudience::People(hits(&[me, foe])),
                },
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "号外「财政补贴发放」：全场 2 人各 获得 $1500"
        );
    }

    #[test]
    fn a_news_that_names_nobody_says_so_out_loud() {
        // 「谁都没点到」必须是**一句人话**而不是一行空白：这一格是空的与按钮坏了,
        // 在玩家眼里是同一件事（§6.1 坑 27 的 UI 版）
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_003".into(),
                    title: "没人接的电话".into(),
                    desc: "测试".into(),
                    scope: NewsScope::One(PlayerPick::RandomOther),
                    audience: NewsAudience::People(vec![]),
                },
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "号外「没人接的电话」：场上没有可点名的人"
        );
    }

    #[test]
    fn the_headline_lists_people_when_the_hits_differ() {
        // 纯函数那一侧补第四格：全场但**各不相同**（涨价卡那种「有人有地没人有」的形状）
        let hits = vec![
            ("阿土伯".to_string(), "支付 $1000".to_string()),
            ("孙小美".to_string(), "支付 $2000".to_string()),
        ];
        assert_eq!(
            news_headline("地价税开征", NewsScope::Everyone, &hits),
            "号外「地价税开征」：2 人 —— 阿土伯 支付 $1000、孙小美 支付 $2000"
        );
    }

    /// 筛过人的那三种作用域各两句（P11b②）：「N 个人挨同样一下」与「一个都没筛到」。
    /// 这一格之所以要专项钉：那两句的名词都来自规则层，写死成「可点名的人」也能让
    /// 全部旧用例绿，而屏幕上一句「场上没有可点名的人」在「没人开车」那一条上就是假话。
    #[test]
    fn a_filtered_crowd_says_which_crowd_it_hit() {
        for (scope, crowd, none) in [
            (
                NewsScope::EveryoneIf(FieldFilter::OnFoot),
                "步行的人",
                "步行的人",
            ),
            (
                NewsScope::EveryoneIf(FieldFilter::Riding),
                "有车的人",
                "有车的人",
            ),
            (
                NewsScope::EveryoneIf(FieldFilter::Detained(DetentionPlace::Hospital)),
                "在医院的人",
                "在医院的人",
            ),
        ] {
            let hits = vec![
                ("阿土伯".to_string(), "原地停留（1 回合）".to_string()),
                ("孙小美".to_string(), "原地停留（1 回合）".to_string()),
            ];
            assert_eq!(
                news_headline("豪雨袭城", scope, &hits),
                format!("号外「豪雨袭城」：{crowd} 2 人各 原地停留（1 回合）")
            );
            assert_eq!(
                news_headline("豪雨袭城", scope, &[]),
                format!("号外「豪雨袭城」：场上没有{none}")
            );
        }
    }

    /// 地档那三种形状（P11b①）。它们各自对应一句玩家会问的话：
    /// 「拆了哪几块」「我家那块在不在里面」「为什么什么都没拆」。
    #[test]
    fn the_ground_headline_names_the_plots_it_touched() {
        let deed = "夷为平地 2 块（土地收回 2 块）";
        assert_eq!(
            news_ground_headline(
                "外星人攻打地球",
                Some("南京路"),
                &["南京路".to_string(), "汉口路".to_string()],
                deed
            ),
            format!("号外「外星人攻打地球」：南京路 一带 2 块 —— 南京路、汉口路 {deed}")
        );
        // 一整条街在缺路段键的盘上可能只剩一块 —— 那一格不该念成「一带 1 块」，
        // 与 `Stamped{lands}` 那句「块数要说」同一条理由
        assert_eq!(
            news_ground_headline(
                "山洪爆发土地流失",
                Some("西单"),
                &["西单".to_string()],
                deed
            ),
            "号外「山洪爆发土地流失」：西单 夷为平地 2 块（土地收回 2 块）"
        );
        // 盘上一块地都没有：说清楚，别播一条静默的空标题
        assert_eq!(
            news_ground_headline("外星人攻打地球", None, &[], deed),
            "号外「外星人攻打地球」：这一带没有地可打"
        );
        assert_eq!(
            news_ground_headline("外星人攻打地球", Some("南京路"), &[], deed),
            "号外「外星人攻打地球」：南京路 一带没有地"
        );
    }

    /// 锚点**不在**清单里的那一种（P11c① 的公有地拍卖）。
    ///
    /// 上面那条「一块就说那块」的形状成立的前提是锚点自己就是被打的那一块（`One` 那一档）。
    /// 拍卖不成立：锚是播报者脚下那块地，而它**有主**，于是清单里只剩旁边那块无主的。
    /// 那一格要是继续走「{at} {deed}」，念出来是「王府井 把 L5 挂上拍卖台」——
    /// 一句话把两个地块绑错，而玩家只会以为自己的地被卖了。
    #[test]
    fn a_ground_news_whose_anchor_was_not_hit_names_the_list() {
        let deed = "把 L5 强制上架";
        assert_eq!(
            news_ground_headline(
                "公有土地公开拍卖",
                Some("王府井"),
                &["西单".to_string()],
                deed
            ),
            "号外「公有土地公开拍卖」：王府井 一带 1 块 —— 西单 把 L5 强制上架"
        );
        // 锚点就是那一块时仍旧走短的那一句（征收与山洪的形状不许被上面那条改坏）
        assert_eq!(
            news_ground_headline(
                "政府强制征收",
                Some("王府井"),
                &["王府井".to_string()],
                deed
            ),
            "号外「政府强制征收」：王府井 把 L5 强制上架"
        );
    }

    /// 机构档那一句（P11c②）：主语是「哪一家」，而「停的是哪项能力」必须上屏。
    /// 「银行 3 天」是一句答不出「那我还能不能借钱」的话，而那一问正是玩家下一手要问的。
    #[test]
    fn an_institution_news_names_the_house_and_the_capability() {
        use game_core::prelude::Institution;
        assert_eq!(
            news_institution_headline("股市临时停牌", Institution::StockMarket, 3),
            "号外「股市临时停牌」：股市 停牌 3 天"
        );
        assert_eq!(
            news_institution_headline("银行停止放款", Institution::Bank, 5),
            "号外「银行停止放款」：银行 停止放款 5 天"
        );
    }

    /// 机构档走完整条接线。这一条与上面那条纯函数用例的分别是它**读得到** `audience`
    /// 里那个天数：漏接线的形状是顶栏停在上一行（看着像新闻没播），而不是编译失败。
    #[test]
    fn an_institution_news_reaches_the_bar() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_017".into(),
                    title: "银行停止放款".into(),
                    desc: "三天不借新钱".into(),
                    scope: NewsScope::Institution(game_core::prelude::Institution::Bank),
                    audience: NewsAudience::Institution {
                        what: game_core::prelude::Institution::Bank,
                        days: 3,
                    },
                },
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "号外「银行停止放款」：银行 停止放款 3 天"
        );
    }

    /// 地档走完整条接线（顶栏那一行由 `hint_from_events` 拼，纯函数只覆盖它自己那一半）。
    /// 这份夹具没有棋盘资源，所以地块名回退成 `L2` —— 那正是「名字查不到时说什么」那一格，
    /// 而菜单态（`Option<Res<ActiveBoard>>` 是 `None`）真的会走到这里。
    #[test]
    fn a_ground_news_reads_its_plot_list_on_the_bar() {
        let (mut app, me, _foe, hint) = notices_app();
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_004".into(),
                    title: "外星人攻打地球".into(),
                    desc: "一整条街".into(),
                    scope: NewsScope::Around,
                    audience: NewsAudience::Ground {
                        center: Some(game_core::prelude::LandId(2)),
                        lands: vec![game_core::prelude::LandId(2), game_core::prelude::LandId(3)],
                        outcomes: vec![EffectOutcome::Razed {
                            lands: 2,
                            seized: 1,
                        }],
                    },
                },
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "号外「外星人攻打地球」：L2 一带 2 块 —— L2、L3 夷为平地 2 块（土地收回 1 块）"
        );
    }

    /// 同帧里新闻要赢过它前面那些 reader（链上它排在全部结算之后）。
    /// 这条是「接线」的用例：漏掉排序的话症状不是不显示，而是**看哪条先到**——
    /// 与 P8 那条「乐透必须排在 decision 之前」同族。
    #[test]
    fn the_extra_wins_over_the_lines_that_came_earlier_in_the_frame() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            PlayerBailed {
                player: foe,
                by: me,
                place: game_core::prelude::DetentionPlace::Jail,
                points: 30,
                junction: TileIndex(2),
            },
        );
        send(
            &mut app,
            NewsFired {
                broadcaster: me,
                news: NewsNotice {
                    id: "news_001".into(),
                    title: "财政补贴发放".into(),
                    desc: "每人进账 $1500".into(),
                    scope: NewsScope::Everyone,
                    audience: NewsAudience::People(vec![NewsHit {
                        player: me,
                        outcomes: vec![EffectOutcome::Gained(1_500)],
                    }]),
                },
            },
        );
        assert!(
            hint_of(&mut app, hint).starts_with("号外"),
            "同一帧两条，留下的该是最后发生的那一件"
        );
    }

    /// 一句话的四种形状（纯函数）。顶栏只有一行，混着发就分不清是哪条 reader 读到的，
    /// 而这一句的四格恰好是 P13 验收里最难念对的那四格。
    ///
    /// 「落空」与「被挡下」不许共用一句话：前者是「这一道今天没东西可打」，后者是
    /// 「有人花掉了一张免罪卡」—— 玩家听完这两句会做不同的下一手。
    #[test]
    fn a_spell_headline_has_four_shapes() {
        let row = |hit: &[&str], blocked: &[&str]| {
            let hit: Vec<String> = hit.iter().map(|s| s.to_string()).collect();
            let blocked: Vec<String> = blocked.iter().map(|s| s.to_string()).collect();
            magic_headline("阿土伯", MagicSpell::Jail, &hit, &blocked)
        };

        assert_eq!(
            row(&["孙小美"], &[]),
            "阿土伯 求的「坐牢 3 天」 落到 孙小美 头上"
        );
        assert_eq!(
            row(&["孙小美", "阿土伯"], &[]),
            "阿土伯 求的「坐牢 3 天」 落到 孙小美、阿土伯 头上",
            "一批两个人要都念出来，人数不能代替名字"
        );
        assert_eq!(
            row(&[], &["孙小美"]),
            "阿土伯 求的「坐牢 3 天」 被 孙小美 用免罪卡整笔挡下"
        );
        assert_eq!(
            row(&["阿土伯"], &["孙小美"]),
            "阿土伯 求的「坐牢 3 天」 落到 阿土伯 头上；孙小美 用免罪卡挡下了那一下",
            "挡下与挨中并存时两句都要说 —— 只说一边就是少记账"
        );
        assert_eq!(
            row(&[], &[]),
            "阿土伯 求的「坐牢 3 天」 落空 —— 一样都没落到人头上"
        );
    }

    /// 两条凭据都要接到那一行：只接 `MagicCast` 的话，「有人用掉了一张免罪卡」这件事
    /// 整条被吞掉，而那是这一局里唯一一张会被用掉的卡。
    #[test]
    fn the_witch_s_two_receipts_reach_the_bar() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            MagicCast {
                player: me,
                spell: MagicSpell::Hospital,
                targets: vec![foe],
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("落到 孙小美 头上"),
            "施法那一条没接到顶栏"
        );

        send(
            &mut app,
            MagicBlocked {
                player: me,
                spell: MagicSpell::Jail,
                victim: foe,
            },
        );
        send(
            &mut app,
            MagicCast {
                player: me,
                spell: MagicSpell::Jail,
                targets: vec![],
            },
        );
        assert!(
            hint_of(&mut app, hint).contains("被 孙小美 用免罪卡整笔挡下"),
            "全被挡下那一格没接到顶栏"
        );

        // 一批两个人：一个挨了、一个挡了 —— 同一句里两边都要说得出
        send(
            &mut app,
            MagicBlocked {
                player: me,
                spell: MagicSpell::TurnBack,
                victim: foe,
            },
        );
        send(
            &mut app,
            MagicCast {
                player: me,
                spell: MagicSpell::TurnBack,
                targets: vec![me],
            },
        );
        let line = hint_of(&mut app, hint);
        assert!(
            line.contains("落到 阿土伯 头上") && line.contains("孙小美 用免罪卡挡下了那一下"),
            "两种落点并存时少念了一半：{line}"
        );
    }

    /// 门那一句有四种形状（事／钱 × 抵消／嫁祸），而这一行是「我刚才花了一张卡」
    /// 唯一上屏的地方。钱那两种**必须带数**：不带数就分不清「免了一次坐牢」与
    /// 「免了一笔 2,400」，而后者正是玩家判断下一张该不该留着的唯一依据。
    #[test]
    fn the_gate_clause_reads_differently_for_money_and_for_events() {
        let (mut app, me, foe, hint) = notices_app();
        send(
            &mut app,
            PassiveUsed {
                player: me,
                card: notice("免费卡"),
                event: PassiveEvent::Rent,
                moved_to: None,
                saved: 2_400,
            },
        );
        let line = hint_of(&mut app, hint);
        assert!(
            line.contains("免掉了那笔 2,400"),
            "钱那一侧要说得出省了多少：{line}"
        );
        assert!(
            !line.contains("挡下"),
            "钱不是一件灾，别念事件的动词：{line}"
        );

        send(
            &mut app,
            PassiveUsed {
                player: me,
                card: notice("嫁祸卡"),
                event: PassiveEvent::Bill,
                moved_to: Some(foe),
                saved: 5_000,
            },
        );
        let line = hint_of(&mut app, hint);
        assert!(
            line.contains("那笔 5,000") && line.contains("转给了 孙小美"),
            "转嫁要说的是**那笔钱**去了谁那儿：{line}"
        );

        send(
            &mut app,
            PassiveUsed {
                player: me,
                card: notice("免罪卡"),
                event: PassiveEvent::JailTrip,
                moved_to: None,
                saved: 0,
            },
        );
        let line = hint_of(&mut app, hint);
        assert!(
            line.contains("挡下了那一次") && !line.contains('$'),
            "事那侧没有价签，`saved` 恒为 0：{line}"
        );

        send(
            &mut app,
            PassiveUsed {
                player: me,
                card: notice("嫁祸卡"),
                event: PassiveEvent::HospitalTrip,
                moved_to: Some(foe),
                saved: 0,
            },
        );
        let line = hint_of(&mut app, hint);
        assert!(
            line.contains("那一次「") && line.contains("转给了 孙小美"),
            "四种形状里最后这一种：{line}"
        );
    }
}
