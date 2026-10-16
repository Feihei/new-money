//! 两块常驻浮层：ATM（存/取）与证券（行情与买卖）。
//!
//! 与四个模态的分别是三条，都是刻意的：
//! - **不阻断回合**。ATM 与证券都是「本回合把钱安排一下」的自由动作，回合仍然由
//!   掷骰 → 行走 → 落格那条链推进；模态则是规则层在等一个回答。
//! - **内容跟着世界状态刷新**，而不是弹出那一刻烤死。行情每回合结束动一次、现金每笔
//!   操作动一次，烤死的面板会在玩家眼前停在过期数字上。刷新判据是挂在面板实体上的
//!   [`FinanceStamp`]（§6.1 坑 22 的那条：键挂实体，实体没了键就没了）。
//! - **窗口由判据给**，不由「按钮存在」给。证券只在 `WaitRoll` 可交易，ATM 只在卡还在
//!   手里时亮着 —— 与规则层同构，判据集中在 [`super::can_trade`]/[`super::can_bank_op`]。
//!
//! 布局：ATM 左上、证券左下、详情卡右上、HUD 底部通栏，四块互不透叠（1024×768）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    AtmAccess, BankOp, Books, DataHandles, GameRules, MarketClock, Paused, PendingDecision,
    PriceHistory, StockId, StockPrice, StockSet, TradeSide, TurnOrder, TurnPhase,
};

use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, SECONDARY, UiAction, border_color, button, can_bank_op, can_trade,
    counter_of, info_row, is_open_day, label,
};
use crate::render::DevFont;

/// ATM 浮层开着吗。顶栏那颗按钮的唯一记忆。
///
/// 卡过期时**不**跟着清这个标志：面板随持卡人消失，但「我把它开过」这件事留着，
/// 下次路过银行格会自己弹回来。反过来清掉的写法试过，代价更大 —— 装配前那几帧
/// 还没有持卡人，标志会被抹成 false，于是 `NM_ATM=1` 这张截图永远拍不到面板
/// （与 §6.1 坑 22 同族：状态被一条「看起来无害」的清理吃掉，且不报错）。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AtmPanelOpen(pub bool);

impl AtmPanelOpen {
    /// `NM_ATM=1`：既开面板也把卡发到当前玩家手里（见 [`dev_grant_atm`]）。
    /// 这张卡的正常来源是「路过银行格」，而路过哪一格由骰子说了算 —— 截图脚本控制不了，
    /// 就得给它一条不靠运气的入口（判据：每个界面都必须有一条不依赖鼠标的入口）。
    pub(crate) fn from_env() -> Self {
        Self(std::env::var_os("NM_ATM").is_some())
    }
}

/// 证券浮层开着吗。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct StockPanelOpen(pub bool);

impl StockPanelOpen {
    pub(crate) fn from_env() -> Self {
        Self(std::env::var_os("NM_STOCK").is_some())
    }
}

/// 面板根上的「上次写成什么样」。整块重建，而不是逐段改写：
/// 金额按钮上的数字随余额变（「存全部」的数额每一笔都不同），逐段改写要缓存十几个节点，
/// 漏一段就是过期数字 —— 而面板一共十几个节点，重建一次的成本看不见。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct FinanceStamp(pub String);

#[derive(Component)]
pub(crate) struct AtmPanel;

#[derive(Component)]
pub(crate) struct StockPanel;

/// 开发用：把 ATM 卡塞给当前回合玩家（`NM_ATM=1` 才装配这条 system）。
/// 只写规则层的资源，不碰回合机 —— 与「面板由状态驱动」是同一条纪律。
pub(crate) fn dev_grant_atm(order: Option<Res<TurnOrder>>, mut atm: ResMut<AtmAccess>) {
    let Some(order) = order else { return };
    if let Some(player) = order.current() {
        atm.0 = Some(player);
    }
}

/// ATM 浮层要读的世界状态。打包成参数包是这仓库的房规：需要知道的就是这么多，
/// 而 `#[allow(clippy::too_many_arguments)]` 会把「一个 system 要读八样东西」这个信号吃掉
/// （`.agents/ARCHITECTURE.md` §六 坑 8）。
#[derive(SystemParam)]
pub(crate) struct AtmView<'w> {
    atm: Res<'w, AtmAccess>,
    open: Res<'w, AtmPanelOpen>,
    pending: Res<'w, PendingDecision>,
    paused: Res<'w, Paused>,
    clock: Res<'w, MarketClock>,
    rules: Res<'w, GameRules>,
}

/// ATM 浮层：现金与存款，四颗金额按钮，一个关闭。
pub(crate) fn sync_atm_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    books: Books,
    view: AtmView<'_>,
    shown: Query<(Entity, &AtmPanel, &FinanceStamp)>,
) {
    // 卡在谁手里：柜台的客人优先（两者通常同一个人，分开写是为了「卡过期了但柜台还开着」
    // 那种同帧交接的时刻仍然有钱可办）
    let holder = counter_of(&view.pending).or(view.atm.0);
    let want = if view.open.0 { holder } else { None };
    // 周末银行不开门。写进 stamp 是为了「日子一变面板就重建」——只按余额建，
    // 休市那天余额没动的玩家会看着四颗亮按钮按下去没声音（§6.1 坑 27 的 UI 版）
    let banking_hours = is_open_day(&view.clock, &view.rules);
    let stamp = want.map(|player| {
        let bank = books.bank_of(player);
        format!(
            "{player:?}|{}|{}|{banking_hours}",
            books.balance(player),
            bank.deposit
        )
    });

    if shown
        .iter()
        .next()
        .is_some_and(|(_, _, s)| Some(&s.0) == stamp.as_ref())
    {
        return;
    }
    for (entity, _, _) in shown.iter() {
        commands.entity(entity).despawn();
    }
    let (Some(player), Some(stamp)) = (want, stamp) else {
        return;
    };
    let cash = books.balance(player);
    let deposit = books.bank_of(player).deposit;
    // 窗口判据在外面算好：`with_children` 的闭包拿不到 `Books` 那类借用，
    // 而这两句本来也只依赖状态资源，不依赖余额
    let deposit_window = can_bank_op(
        BankOp::Deposit,
        &view.paused,
        &view.pending,
        &view.atm,
        banking_hours,
    );
    let withdraw_window = can_bank_op(
        BankOp::Withdraw,
        &view.paused,
        &view.pending,
        &view.atm,
        banking_hours,
    );

    let mut root = commands.spawn((
        BoardUi,
        AtmPanel,
        FinanceStamp(stamp),
        // 根节点参与命中测试的话，整块面板会把背后的点格点击吃掉（同顶栏那一坑）
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(layout::ATM_PANEL[1]),
            top: Val::Px(layout::ATM_PANEL[2]),
            width: Val::Px(layout::ATM_PANEL[0]),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(layout::OP_ROW_GAP),
            padding: UiRect::axes(Val::Px(16.0), Val::Px(14.0)),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        BackgroundColor(col::PANEL),
        border_color(col::BORDER_BANK),
        ZIndex(4),
    ));
    root.with_children(|p| {
        p.spawn(label(&font.0, "ATM", size::NAME, col::TITLE_GOLD));
        info_row(p, &font.0, "现金", &format!("${cash}"), col::CASH_OK, 60.0);
        info_row(p, &font.0, "存款", &format!("${deposit}"), col::VALUE, 60.0);
        // 灰按钮必须自带一句为什么，否则「按不动」与「坏了」在玩家眼里是同一件事
        if !banking_hours {
            p.spawn(label(
                &font.0,
                "周末休市 · 下周再来",
                size::ROW,
                col::BANKRUPT,
            ));
        }
        // 「全部」的数额在建按钮这一刻算：面板随余额重建，所以它永远是准的
        money_row(
            p,
            &font,
            BankOp::Deposit,
            deposit_window,
            ("存", 1_000, cash >= 1_000),
            ("存全部", cash, cash > 0),
        );
        money_row(
            p,
            &font,
            BankOp::Withdraw,
            withdraw_window,
            ("取", 1_000, deposit >= 1_000),
            ("取全部", deposit, deposit > 0),
        );
        button(
            p,
            &font.0,
            ButtonSpec::new(
                "关闭",
                layout::PANEL_CLOSE_BUTTON,
                SECONDARY,
                UiAction::ToggleAtm,
            ),
        );
    });
}

/// 一行两个金额按钮。`window` 是「窗口层面准不准办」（ATM 卡 / 柜台），
/// `enough` 是「这笔金额够不够」—— 两条都得亮成灰，但只有前者能解释「为什么按不动」，
/// 所以前者为假时把整行都灰掉，而不是只灰掉那一个金额。
///
/// `pub(crate)` 是给柜台模态用的：那八颗按钮与 ATM 这四颗是同一个形状，
/// 分两份写就会有一份忘了灰。
pub(crate) fn money_row(
    p: &mut ChildSpawnerCommands,
    font: &DevFont,
    op: BankOp,
    window: bool,
    small: (&str, i64, bool),
    big: (&str, i64, bool),
) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        column_gap: Val::Px(layout::BTN_ROW_GAP / 2.0),
        ..default()
    })
    .with_children(|line| {
        for (text, amount, enough) in [small, big] {
            // 动词与数额分开传：数额永远要写出来（「取全部」不写数字，玩家就不知道按下去是多少钱）
            let label = format!("{text} {}", comma(amount));
            button(
                line,
                &font.0,
                ButtonSpec::new(
                    &label,
                    layout::AMOUNT_BUTTON,
                    super::PRIMARY,
                    UiAction::BankOp { op, amount },
                )
                .disabled(!window || !enough),
            );
        }
    });
}

/// 证券浮层要读的世界状态（打包理由同 [`AtmView`]）。这一块的字段数不是设计出来的，
/// 是「一张行情表」本来就要同时看窗口、日历、股票实体与静态定义 —— 少一样就得少一列。
#[derive(SystemParam)]
pub(crate) struct StockView<'w, 's> {
    open: Res<'w, StockPanelOpen>,
    paused: Res<'w, Paused>,
    pending: Res<'w, PendingDecision>,
    phase: Res<'w, State<TurnPhase>>,
    order: Option<Res<'w, TurnOrder>>,
    clock: Res<'w, MarketClock>,
    rules: Res<'w, GameRules>,
    sets: Res<'w, Assets<StockSet>>,
    data: Res<'w, DataHandles>,
    stocks: Query<'w, 's, (&'static StockId, &'static StockPrice, &'static PriceHistory)>,
}

/// 证券浮层：五只股的现价、涨跌、持股，每行两颗买卖按钮。
pub(crate) fn sync_stock_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    books: Books,
    view: StockView<'_, '_>,
    shown: Query<(Entity, &StockPanel, &FinanceStamp)>,
) {
    if !view.open.0 {
        for (entity, _, _) in &shown {
            commands.entity(entity).despawn();
        }
        return;
    }
    let player = view.order.as_ref().and_then(|o| o.current());
    let trading_hours = is_open_day(&view.clock, &view.rules);
    let tradable = player.is_some_and(|p| {
        can_trade(
            *view.phase.get(),
            &view.paused,
            &view.pending,
            trading_hours,
        ) && same_actor(p, &view.pending)
    });
    let set = view.sets.get(&view.data.stocks);
    let mut rows: Vec<(String, String, f32, f32, u32)> = view
        .stocks
        .iter()
        .map(|(id, price, history)| {
            let prev = history.0.iter().rev().nth(1).copied().unwrap_or(price.0);
            let name = set
                .and_then(|s| s.find(&id.0))
                .map(|d| d.name.clone())
                .unwrap_or_else(|| id.0.clone());
            let held = player.map_or(0, |p| {
                books
                    .holdings_of(p)
                    .iter()
                    .find(|(held, _)| held.0 == id.0)
                    .map_or(0, |(_, n)| *n)
            });
            (id.0.clone(), name, price.0, prev, held)
        })
        .collect();
    // 按 id 排序：实体的迭代顺序是生成顺序，虽在本局稳定，但断言与截图不该依赖它。
    // 只比 id 不比价格 —— `f32` 不满足 `Ord`，而排序键本来就该是那个稳定的字符串
    rows.sort_by(|a, b| a.0.cmp(&b.0));

    let cash = player.map_or(0, |p| books.balance(p));
    let stamp = format!(
        "{}|{cash}|{tradable}|{}",
        view.clock.tick,
        rows.iter()
            .map(|(id, _, price, _, held)| format!("{id}:{price}:{held}"))
            .collect::<Vec<_>>()
            .join(",")
    );
    if shown.iter().next().is_some_and(|(_, _, s)| s.0 == stamp) {
        return;
    }
    for (entity, _, _) in shown.iter() {
        commands.entity(entity).despawn();
    }

    let (month, day) = game_core::economy::date_of(view.clock.tick, view.rules.days_per_month);
    let until = game_core::economy::days_until_dividend(view.clock.tick, &view.rules);
    let mut root = commands.spawn((
        BoardUi,
        StockPanel,
        FinanceStamp(stamp),
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(layout::STOCK_PANEL[1]),
            bottom: Val::Px(layout::STOCK_PANEL[2]),
            width: Val::Px(layout::STOCK_PANEL[0]),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(6.0),
            padding: UiRect::axes(Val::Px(16.0), Val::Px(14.0)),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        BackgroundColor(col::PANEL),
        border_color(col::BORDER_STOCK),
        ZIndex(4),
    ));
    root.with_children(|p| {
        p.spawn(label(
            &font.0,
            format!("证券 · 第 {month} 月 {day} 日（距派息 {until} 天）"),
            size::NAME,
            col::TITLE_GOLD,
        ));
        p.spawn(label(
            &font.0,
            // 三句各有各的「不能」，混成一句就会把休市说成「你不会操作」
            format!(
                "现金 ${cash} — {}",
                if !trading_hours {
                    "周末休市，下单不受理"
                } else if tradable {
                    "掷骰前可买卖"
                } else {
                    "只在掷骰前可买卖"
                }
            ),
            size::ROW,
            if tradable { col::MUTED } else { col::BANKRUPT },
        ));
        for (id, name, price, prev, held) in &rows {
            // 拷出三个数字再算：迭代的是 `&rows`，留着一路 `*` 会把算术写成谜
            let (price, prev, held) = (*price, *prev, *held);
            let drift = if prev > 0.0 {
                (price - prev) / prev * 100.0
            } else {
                0.0
            };
            let line_text = format!(
                "{name}  ${}  {sign}{drift:.1}%  持 {held}",
                format_price(price),
                sign = if drift >= 0.0 { "+" } else { "" }
            );
            p.spawn(Node {
                flex_direction: FlexDirection::Row,
                align_items: AlignItems::Center,
                column_gap: Val::Px(8.0),
                ..default()
            })
            .with_children(|line| {
                line.spawn((
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                    label(
                        &font.0,
                        line_text,
                        size::ROW,
                        if drift >= 0.0 { col::GAIN } else { col::COST },
                    ),
                ));
                let shares = 10u32;
                // 买不起与一股不卖都直接灰掉：判据与 `economy::buy_plan`/`sell_plan` 同一条
                let affordable = (price * shares as f32).ceil() as i64 <= cash;
                for side in [TradeSide::Buy, TradeSide::Sell] {
                    let enough = match side {
                        TradeSide::Buy => affordable,
                        TradeSide::Sell => held >= shares,
                    };
                    let text = match side {
                        TradeSide::Buy => format!("买 {shares}"),
                        TradeSide::Sell => format!("卖 {shares}"),
                    };
                    button(
                        line,
                        &font.0,
                        ButtonSpec::new(
                            &text,
                            layout::PANEL_CLOSE_BUTTON,
                            super::PRIMARY,
                            UiAction::Trade {
                                stock: StockId(id.clone()),
                                side,
                                shares,
                            },
                        )
                        .disabled(!tradable || !enough),
                    );
                }
            });
        }
        button(
            p,
            &font.0,
            ButtonSpec::new(
                "关闭",
                layout::PANEL_CLOSE_BUTTON,
                SECONDARY,
                UiAction::ToggleStock,
            ),
        );
    });
}

/// 证券浮层的按钮能不能替「正在被模态占住的那位」下单。
/// `can_trade` 已经要求没有待决事项，这一句只是把那个前提说清楚。
fn same_actor(current: Entity, pending: &PendingDecision) -> bool {
    pending.active().is_none_or(|(player, _)| player == current)
}

fn format_price(price: f32) -> String {
    // 一分以下的价格（跌破 1 元）也要读得出，所以不能用千分位那条路
    if price >= 1000.0 {
        comma(price.round() as i64)
    } else {
        format!("{price:.2}")
    }
}

/// `12345` → `12,345`。金额不带分隔符时，六位数的租金要按两遍才读得准。
pub(crate) fn comma(amount: i64) -> String {
    let text = amount.abs().to_string();
    let mut out = String::with_capacity(text.len() + text.len() / 3 + 1);
    if amount < 0 {
        out.push('-');
    }
    for (i, ch) in text.chars().enumerate() {
        if i > 0 && (text.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}
