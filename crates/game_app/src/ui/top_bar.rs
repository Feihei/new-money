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
    BankVisited, GameOver, GameSession, JailSkipped, Paused, PendingDecision, PlayerBankrupt,
    PlayerName, StockVisited, TurnOrder, TurnPhase,
};

use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, Disabled, PRIMARY, Palette, SECONDARY, UiAction, button, can_roll, label,
};
use crate::render::DevFont;

#[derive(Component)]
pub(crate) struct HintText;

#[derive(Component)]
pub(crate) struct PhaseText;

/// 顶栏。装进 `UiSet::Boot`：与棋盘同批出生，回菜单时随 [`BoardUi`] 一起收走。
pub(crate) fn spawn_top_bar(mut commands: Commands, font: Res<DevFont>) {
    let (hint, hint_font, hint_color) = label(&font.0, "准备中", size::BODY, col::HINT);
    let (phase, phase_font, phase_color) = label(&font.0, "", size::BODY, col::PHASE);
    let bar = commands
        .spawn((
            BoardUi,
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
        .with_child((PhaseText, phase, phase_font, phase_color))
        .with_children(|bar| {
            // 掷骰按钮按「常态」spawn，可用态全交给 `refresh_top_bar` 判：
            // `Palette` 因此恒等于主色，解禁时 `bg = palette.normal` 才是对的颜色
            button(
                bar,
                &font.0,
                ButtonSpec::new("掷骰", layout::ROLL_BUTTON, PRIMARY, UiAction::Roll),
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
    names: Query<'w, 's, &'static PlayerName>,
}

/// 阶段跳变时改写旁白与阶段名；掷骰按钮的可用态则跟着判据走。
///
/// 两条守卫分开是因为它们的驱动源不同。文本只关心阶段，用 `Local<Option<TurnPhase>>`
/// 而不是 `Ref<State<_>>::is_changed`：装配那一帧按钮还不在世界里（`Commands` 帧末才 flush），
/// 文本写空手时要把这一帧**记下来重来**，判据握在自己手里更好办。
/// 按钮**不设守卫**：暂停开合、待决事项清空都不改阶段，只盯阶段的守卫会让它停在错误的颜色上；
/// 而回主菜单再开一局时 `Local` 会带着上一局的判断活下来，新 spawn 的按钮就再也没人刷过。
/// 判据本身是幂等的（只在 `Disabled` 与 `can_roll` 不一致时才动手），逐帧跑也不影响悬停换色。
pub(crate) fn refresh_top_bar(
    turn: TurnView,
    mut buttons: Query<(
        Entity,
        &mut BackgroundColor,
        &Palette,
        Option<&Disabled>,
        &UiAction,
    )>,
    mut texts: Query<(&mut Text, Option<&HintText>, Option<&PhaseText>)>,
    mut commands: Commands,
    mut seen_phase: Local<Option<TurnPhase>>,
) {
    let now = *turn.phase.get();
    let can = can_roll(now, &turn.paused, &turn.pending);
    for (entity, mut bg, palette, disabled, action) in buttons.iter_mut() {
        if *action != UiAction::Roll {
            continue;
        }
        match (can, disabled) {
            (true, Some(_)) => {
                commands.entity(entity).remove::<Disabled>();
                bg.0 = palette.normal;
            }
            (false, None) => {
                commands.entity(entity).insert(Disabled);
                bg.0 = col::BTN_DISABLED;
            }
            _ => {}
        }
    }

    if *seen_phase == Some(now) {
        return;
    }
    let hint = phase_hint(now, &turn.order, &turn.session, &turn.names);
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
fn phase_hint(
    phase: TurnPhase,
    order: &TurnOrder,
    session: &GameSession,
    names: &Query<&PlayerName>,
) -> Option<String> {
    let text = match phase {
        TurnPhase::Boot => "准备中".to_string(),
        TurnPhase::WaitRoll => match order.current().and_then(|p| names.get(p).ok()) {
            Some(name) => format!(
                "{} 的回合 — 请掷骰（第 {} 回合）",
                name.0, session.turn_count
            ),
            None => "请掷骰".to_string(),
        },
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
    jail_skip: MessageReader<'w, 's, JailSkipped>,
    bankrupt: MessageReader<'w, 's, PlayerBankrupt>,
    bank: MessageReader<'w, 's, BankVisited>,
    stock: MessageReader<'w, 's, StockVisited>,
    over: MessageReader<'w, 's, GameOver>,
}

pub(crate) fn hint_from_events(
    mut notices: Notices,
    names: Query<&PlayerName>,
    mut hints: Query<&mut Text, With<HintText>>,
) {
    // 一帧里可能同时有多条（例如破产紧跟在回合结束旁白之后）：按流水顺序取最后一条，
    // 与 Godot 那些信号处理函数逐个覆盖 `hint_label.text` 的结果一致
    let mut line: Option<String> = None;
    for m in notices.jail_skip.read() {
        line = Some(format!(
            "{} 在监狱中（剩 {} 回合），跳过本回合",
            name(&names, m.player),
            m.turns_left
        ));
    }
    for m in notices.bankrupt.read() {
        line = Some(format!("{} 破产了！", name(&names, m.player)));
    }
    for m in notices.bank.read() {
        line = Some(format!(
            "{} 来到银行（存款 ${}，贷款 ${}）",
            name(&names, m.player),
            m.deposit,
            m.loan
        ));
    }
    for m in notices.stock.read() {
        line = Some(format!("{} 来到了证券交易所", name(&names, m.player)));
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

fn name(names: &Query<&PlayerName>, player: Entity) -> String {
    names
        .get(player)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| "玩家".to_string())
}
