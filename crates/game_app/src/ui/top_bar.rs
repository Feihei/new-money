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
    ActiveBoard, AtmAccess, BankOpDone, BankVisited, BombTransferred, CardLost, CardPlayed,
    ChargeWaived, ChestOpened, DetentionLifted, DetentionSkipped, GameOver, GameSession,
    GodAppeared, GodDeed, GodExit, GodLeft, GodPossessed, ItemPlaced, LandTaken, LotteryBought,
    LotteryDrawn, LotteryWon, MarketClock, MinigameScore, MortgageDone, MortgageOp, MountChanged,
    MoveCapped, Paused, PendingDecision, PlayerBailed, PlayerBankrupt, PlayerName, PointsChanged,
    RemoteDiceUsed, ShopOpDone, StayLifted, StaySentence, StaySkipped, StockTraded, StockVisited,
    TileIndex, TradeSide, TurnOrder, TurnPhase, WalkHalted,
};

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
    let mut wrote = false;
    for (mut text, is_hint, is_phase) in texts.iter_mut() {
        if is_phase.is_some() {
            *text = Text::new(phase_name(now));
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
            match turn.order.current().and_then(|p| turn.names.get(p).ok()) {
                Some(name) => format!(
                    "{} 的回合 — 请掷骰（第 {turn_no} 回合 · {} · 物价 ×{index:.2}）",
                    name.0,
                    date,
                    turn_no = turn.session.turn_count
                ),
                None => format!("请掷骰（{date} · 物价 ×{index:.2}）"),
            }
        }
        TurnPhase::Walking => "移动中…".to_string(),
        _ => return None,
    };
    Some(text)
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
    /// 百货成交（P3d②）。排在 `points` 之后：同一帧里那笔点券变动与这张成交凭据说的
    /// 是同一件事，而带卡名的那条更能回答「我刚按下去发生了什么」
    shop: MessageReader<'w, 's, ShopOpDone>,
    /// 抵押柜台的成交（P3d③）。排在 `shop` 之后、`bank_ops` 那一族之外单开一条：
    /// 它说的是「哪块地、过手多少」，而那四条银行操作只带金额。
    mortgage: MessageReader<'w, 's, MortgageDone>,
    /// 出牌的结果。手牌面板随手牌重建，把一次性结果烤进去下一次就没了 ——
    /// 持久的读数只有这一行（与点券那一条同一个道理）
    played: MessageReader<'w, 's, CardPlayed>,
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
    /// 一次性修正生效那一笔（免费卡花掉了）。**没有对应的 `CashChanged`**：钱一分没动而
    /// 卡没了，玩家能看见的只有这一行
    waived: MessageReader<'w, 's, ChargeWaived>,
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
        line = Some(format!(
            "{} 在{}中（剩 {} 回合），跳过本回合",
            name(&names, m.player),
            m.place.label(),
            m.turns_left
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
        line = Some(if m.winners == 0 {
            format!(
                "乐透开出 {} 号 —— 没人猜中，奖池 {} 滚到下月",
                m.number, m.pot
            )
        } else {
            format!(
                "乐透开出 {} 号 —— {} 人猜中，各领 {}",
                m.number, m.winners, m.each
            )
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
    // 免付要说「免掉了多少」而不是「付了 0」：前者是一次事件的凭据，后者读起来像租金表坏了
    for m in notices.waived.read() {
        line = Some(format!(
            "{} 免付了一笔 ${}（{}）",
            name(&names, m.player),
            comma(m.saved),
            source_word(m.via)
        ));
    }
    // ===== P6e①：神明 =====
    // 六句话各抽成一个纯函数：顶栏只有一行，而「这句话怎么说」与名字怎么解析无关
    // （与 `op_verb`／`source_word` 同一条分工）。抽出来还有一层用处 —— 那六条分支里
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

/// 免付是谁给的。今天只有卡，但这一栏不能省：将来神明（大财神赖账）与规则（P10 的
/// 董事长免费）都走同一条凭据，而玩家有权知道自己这次是靠什么少付的。
fn source_word(via: game_core::prelude::Source) -> &'static str {
    use game_core::prelude::Source;
    match via {
        Source::Card => "卡片",
        Source::God => "神明",
        Source::Rule => "规则",
    }
}

fn name(names: &Query<&PlayerName>, player: Entity) -> String {
    names
        .get(player)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| "玩家".to_string())
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
    use game_core::prelude::{
        CardKind, CardNotice, GameKind, GameRules, God, LandId, PlayerId, StockId, TradeSide,
        TurnOrder,
    };

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
            .add_message::<ShopOpDone>()
            .add_message::<MortgageDone>()
            .add_message::<CardPlayed>()
            .add_message::<GodAppeared>()
            .add_message::<GodPossessed>()
            .add_message::<GodLeft>()
            .add_message::<GodDeed>()
            .add_message::<CardLost>()
            .add_message::<LandTaken>()
            .add_message::<ChargeWaived>()
            .add_message::<ItemPlaced>()
            .add_message::<WalkHalted>()
            .add_message::<BombTransferred>()
            .add_message::<ChestOpened>()
            .add_message::<RemoteDiceUsed>()
            .add_message::<MoveCapped>()
            .add_message::<MountChanged>()
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
    #[test]
    fn a_draw_announces_both_outcomes() {
        let (mut app, _me, _foe, hint) = notices_app();
        send(
            &mut app,
            LotteryDrawn {
                number: 7,
                winners: 0,
                each: 0,
                pot: 3_000,
            },
        );
        assert_eq!(
            hint_of(&mut app, hint),
            "乐透开出 7 号 —— 没人猜中，奖池 3000 滚到下月"
        );

        let (mut app, _me, _foe, hint) = notices_app();
        send(
            &mut app,
            LotteryDrawn {
                number: 7,
                winners: 2,
                each: 1_500,
                pot: 3_000,
            },
        );
        let said = hint_of(&mut app, hint);
        assert!(
            said.contains("2 人猜中") && said.contains("各领 1500"),
            "均分那一句得说得出人数与每人多少：{said}"
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
                winners: 1,
                each: 2_000,
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
}
