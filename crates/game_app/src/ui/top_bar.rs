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
    AtmAccess, BankOpDone, BankVisited, CardPlayed, ChargeWaived, GameOver, GameRules, GameSession,
    JailSkipped, MarketClock, Paused, PendingDecision, PlayerBankrupt, PlayerName, PointsChanged,
    ShopOpDone, StockTraded, StockVisited, TradeSide, TurnOrder, TurnPhase,
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
    rules: Res<'w, GameRules>,
    names: Query<'w, 's, &'static PlayerName>,
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
/// 否则「每月 1 号派息」与「地价翻了一倍」只是两句写在文档里的话。
fn phase_hint(phase: TurnPhase, turn: &TurnView) -> Option<String> {
    let text = match phase {
        TurnPhase::Boot => "准备中".to_string(),
        TurnPhase::WaitRoll => {
            let (month, day) =
                game_core::economy::date_of(turn.clock.tick, turn.rules.days_per_month);
            // 指数每回合末重算，而这一行在进入 WaitRoll 时才写 —— 两者同帧之后，
            // 所以这里读到的就是「今天」那个数（旁白不会显示昨天的物价）
            let index = turn.session.price_index;
            match turn.order.current().and_then(|p| turn.names.get(p).ok()) {
                Some(name) => format!(
                    "{} 的回合 — 请掷骰（第 {turn_no} 回合 · 第 {month} 月 {day} 日 · 物价 ×{index:.2}）",
                    name.0,
                    turn_no = turn.session.turn_count
                ),
                None => format!("请掷骰（第 {month} 月 {day} 日 · 物价 ×{index:.2}）"),
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
    bank_ops: MessageReader<'w, 's, BankOpDone>,
    stock: MessageReader<'w, 's, StockVisited>,
    traded: MessageReader<'w, 's, StockTraded>,
    points: MessageReader<'w, 's, PointsChanged>,
    /// 百货成交（P3d②）。排在 `points` 之后：同一帧里那笔点券变动与这张成交凭据说的
    /// 是同一件事，而带卡名的那条更能回答「我刚按下去发生了什么」
    shop: MessageReader<'w, 's, ShopOpDone>,
    /// 出牌的结果。手牌面板随手牌重建，把一次性结果烤进去下一次就没了 ——
    /// 持久的读数只有这一行（与点券那一条同一个道理）
    played: MessageReader<'w, 's, CardPlayed>,
    /// 一次性修正生效那一笔（免费卡花掉了）。**没有对应的 `CashChanged`**：钱一分没动而
    /// 卡没了，玩家能看见的只有这一行
    waived: MessageReader<'w, 's, ChargeWaived>,
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
    // 免付要说「免掉了多少」而不是「付了 0」：前者是一次事件的凭据，后者读起来像租金表坏了
    for m in notices.waived.read() {
        line = Some(format!(
            "{} 免付了一笔 ${}（{}）",
            name(&names, m.player),
            comma(m.saved),
            source_word(m.via)
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

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;
    use game_core::prelude::{PlayerId, StockId, TradeSide, TurnOrder};

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
}
