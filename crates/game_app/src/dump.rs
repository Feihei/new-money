//! 只读 dump 面板（T1）：把「这一局现在到底是什么状态」原样摊开。
//!
//! 三条约束决定了它的形状：
//! - **只挂 `game_app`**，`game_core` 一条依赖都不加 —— 为了让调试面板去改规则层，方向是错的。
//! - **手写 `Debug` 打印**，不走 `World::inspect_entity` 反射：core 的类型本来就全
//!   `derive(Debug)`，而反射要反过来给二十几个组件补 `Reflect` derive。
//! - **`NM_DUMP=1` 才装载**。egui 与 `bevy_ui` + `bevy_picking` 同场会抢输入，默认装载
//!   等于给「按钮点不到」那一族坑（ARCHITECTURE §6.1 坑 23/27）新增一个来源。
//!
//! 面板从 P2 起有「物价指数」与「修正登记处」两行，因为这两样正是那条管线唯一的输入。
//! 「`Detention`」还没有（P4），到时候往 [`dump_panel`] 里再加一栏就行。

use std::sync::Arc;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, EguiPrimaryContextPass, PrimaryEguiContext, egui};
use game_core::prelude::{
    ActiveBoard, AtmAccess, BankBalance, Bankrupt, BuildingLevel, CardKind, Corrections,
    CouponBoxes, Deck, DeckOrder, GameRules, GameSeed, GameSession, Hand, Insulation, JailSentence,
    LandId, LandStatus, Landing, MarketClock, Mortgaged, Owner, Paused, PendingDecision, PlayerId,
    PlayerName, Points, Shelf, ShopStock, Timed, TurnOrder, TurnPhase, Wallet,
};

use crate::asset_root;

pub struct DumpPlugin;

impl Plugin for DumpPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(EguiPlugin::default())
            .add_systems(Update, attach_context)
            .add_systems(EguiPrimaryContextPass, dump_panel);
    }
}

/// 把主上下文钉在**当时活着的那台相机**上，并给它装中文字体。
///
/// 两件事必须写在同一个系统里，因为它们是同一个故障的两半：
/// - bevy_egui 0.43 的自动创建只认「第一台相机」，而我们的相机是随场景长出来的 —— 菜单那台
///   `Camera2d` 先出生、被它拿走上下文，进对局时随 `MenuPiece` 一起收走，上下文跟着实体一起没了，
///   此后再也不会重建（它内部用一个 `Local<bool>` 记账）。症状是「插件加载了、渲染器也起来了，
///   面板一帧都不画」。
/// - 补上去的那个上下文是**全新**的 `egui::Context`，字体表是默认的。只在第一次装字体不够：
///   实测菜单→对局之后面板整片变成方框。所以字体跟着宿主实体走，换宿主就重装。
///
/// 判据是「没有上下文才补」，所以正常帧里它什么都不做，也不会和自动创建抢同一台相机。
///
/// 这里直接查 `&mut EguiContext` 而不是用 `EguiContexts`：后者内部已经握着同一个组件的可变
/// 查询，两个参数同场会撞借用冲突，而我们要的正好是「宿主实体 + 它的上下文」这一对。
fn attach_context(
    mut commands: Commands,
    mut hosts: Query<(Entity, &mut bevy_egui::EguiContext)>,
    cams: Query<Entity, With<Camera>>,
    mut font_on: Local<Option<Entity>>,
) {
    let Some((host, mut egui_ctx)) = hosts.iter_mut().next() else {
        if let Some(cam) = cams.iter().next() {
            commands.entity(cam).insert(PrimaryEguiContext);
        }
        return;
    };
    if *font_on == Some(host) {
        return;
    }
    *font_on = Some(host);
    // 字体是**运行时读盘**而不是 `include_bytes!`：`assets/font/dev_*` 是开发授权字体且已
    // gitignore，编进二进制会让一份没有字体的克隆直接编译不过。读不到就照旧用默认字体。
    let path = asset_root().join("font/dev_SourceHanSansSC-Regular.otf");
    let Ok(bytes) = std::fs::read(&path) else {
        warn!("dump 面板没加载中文字体（{} 不存在）", path.display());
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "nm-dev".to_owned(),
        Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .insert(0, "nm-dev".to_owned());
    }
    egui_ctx.get_mut().set_fonts(fonts);
}

/// `NM_DUMP=1` 开了才装载。
///
/// 判据写在入口而不是插件内部：`add_plugins` 一旦调用，egui 就开始吃输入事件了 ——
/// 「装了但没画面板」与「没装」在抢输入这件事上不是同一回事。
pub fn install_if_requested(app: &mut App) {
    if std::env::var("NM_DUMP").as_deref() == Ok("1") {
        info!("只读 dump 面板已开启（NM_DUMP=1）");
        app.add_plugins(DumpPlugin);
    }
}

/// 面板上每个玩家要摊开的那一栏。命名出来是因为八元素的查询元组会撞
/// clippy 的 `too_complex_types`，而那个 lint 给的「抽 `type`」正是对的处理。
/// P3a 加了手牌与护身两栏之后更是要一直挂着这个别名。
type PlayerRow = (
    Entity,
    &'static PlayerId,
    &'static PlayerName,
    &'static Wallet,
    &'static Points,
    &'static BankBalance,
    Option<&'static JailSentence>,
    Option<&'static Bankrupt>,
    Option<&'static Hand>,
    Option<&'static Insulation>,
);

/// 面板上每块地要摊开的那一栏。P3c 加进限时状态之后，就地凑齐了 clippy 的
/// `too_complex_types` 阈值，处理方式与 [`PlayerRow`] 同一条：给它名字，不加 `#[allow]`。
type PlotRow = (
    &'static LandId,
    &'static Owner,
    &'static BuildingLevel,
    &'static Mortgaged,
    Option<&'static Timed<LandStatus>>,
);

/// 面板要读的全部世界状态。收成一个参数包是这仓库的房规：需要知道的就是这么多，
/// 而 `#[allow(clippy::too_many_arguments)]` 会把这个信号吃掉。
#[derive(SystemParam)]
pub struct DumpView<'w, 's> {
    session: Option<Res<'w, GameSession>>,
    seed: Res<'w, GameSeed>,
    order: Option<Res<'w, TurnOrder>>,
    active: Option<Res<'w, ActiveBoard>>,
    phase: Res<'w, State<TurnPhase>>,
    paused: Res<'w, Paused>,
    rules: Res<'w, GameRules>,
    clock: Res<'w, MarketClock>,
    landing: Res<'w, Landing>,
    pending: Res<'w, PendingDecision>,
    atm: Res<'w, AtmAccess>,
    boxes: Res<'w, CouponBoxes>,
    corrections: Res<'w, Corrections>,
    /// 手牌只有下标，可读的名字在货架上 —— 面板摊的是「手上是哪两张卡」，不是两个数
    shelf: Shelf<'w>,
    players: Query<'w, 's, PlayerRow>,
    plots: Query<'w, 's, PlotRow>,
    /// 两副牌各发到哪了。「为什么抽到这张」在定序那副牌上是能从行序算出来的，
    /// 前提是这个面板说得出它发到第几张
    decks: Query<'w, 's, &'static Deck>,
    /// 百货这一趟的货架（P3d②）。查「我买了卡却没拿到」时要能看见这一格摆过哪几张、
    /// 还剩哪几张 —— 那份事实只在这个资源里
    stock: Res<'w, ShopStock>,
}

fn dump_panel(mut contexts: EguiContexts, view: DumpView) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Window::new("NM_DUMP")
        // 硬钉左上角：`default_pos` 只在没有记忆时生效，实测会被自动布局挪到右边缘裁掉。
        // 代价是面板不能拖着走 —— 它是给人扫读的调试窗，不是要交互的界面
        .anchor(egui::Align2::LEFT_TOP, [28.0, 96.0])
        .default_size([440.0, 640.0])
        .show(ctx, |ui| {
            row(ui, "回合", format_session(&view));
            row(ui, "状态", format_state(&view));
            row(ui, "落格", format_landing(&view));
            row(ui, "待决", format_pending(&view));
            row(ui, "资格", format_access(&view));
            row(ui, "牌堆", format_decks(&view));
            row(ui, "货架", format_stock(&view));
            ui.separator();
            for line in format_corrections(&view) {
                ui.monospace(line);
            }
            ui.separator();
            for line in format_players(&view) {
                ui.monospace(line);
            }
            ui.separator();
            for line in format_lands(&view) {
                ui.monospace(line);
            }
        });
}

fn row(ui: &mut egui::Ui, key: &str, value: String) {
    ui.monospace(format!("{key:　<4}{value}"));
}

fn format_session(view: &DumpView) -> String {
    let Some(session) = &view.session else {
        return format!("世界未装配（种子 {:#x}）", view.seed.0);
    };
    let (month, day) = game_core::economy::date_of(view.clock.tick, view.rules.days_per_month);
    let closed = if game_core::economy::is_weekend(view.clock.tick, &view.rules) {
        " · 周末休市"
    } else {
        ""
    };
    format!(
        "第 {} 回合 · 种子 {:#06x} · 日历 第 {} 月 {} 日（行情 tick {}）{closed} · 物价 ×{:.2}",
        session.turn_count, session.seed, month, day, view.clock.tick, session.price_index
    )
}

fn format_state(view: &DumpView) -> String {
    format!(
        "{:?}{} · 暂停 {}",
        view.phase.get(),
        if view.order.is_some() {
            ""
        } else {
            " · 未开局"
        },
        view.paused.0
    )
}

/// 落格事实整条打出来：`Landed` 里「踩到哪格、是什么、归哪个域、绑哪块地」四样
/// 都有 Debug，分派表对不对一眼就能看出来。
fn format_landing(view: &DumpView) -> String {
    match view.landing.0 {
        Some(landed) => format!("{landed:?}"),
        None => "本帧没有落格事实".to_string(),
    }
}

fn format_pending(view: &DumpView) -> String {
    match view.pending.active() {
        Some((player, about)) => format!("{about:?} ← {player:?}"),
        None => "没人挂着待决事项".to_string(),
    }
}

fn format_access(view: &DumpView) -> String {
    let atm = match view.atm.0 {
        Some(player) => format!("ATM 在 {player:?} 手上"),
        None => "ATM 没人".to_string(),
    };
    format!("{atm} · 点券箱已取走 {} 格", view.boxes.taken())
}

/// 修正登记处整表打出来。P2 的管线只有两个输入 —— 物价指数（上一行的 `物价 ×`）
/// 和这张表 —— 有了这两样，屏幕上任何一笔金额都能手算回去。
///
/// 门槛与「一次性」必须跟着打出来：免费卡那种账「今天没免掉」的原因就是这两个位，
/// 少了它们，这一栏只说得出「挂着一条 Cancel」，而玩家问的是「为什么这一笔还是 2400」。
fn format_corrections(view: &DumpView) -> Vec<String> {
    if view.corrections.is_empty() {
        return vec!["修正登记处：空（没有人挂着可抵消/转嫁/加倍的账）".to_string()];
    }
    let mut lines = vec![format!("修正登记处：{} 条", view.corrections.len())];
    lines.extend(view.corrections.iter().map(|(at, c)| {
        format!(
            "  #{at} {:?} {:?} {:?} ←{:?}{}{}",
            c.bearer,
            c.scope,
            c.adjustment,
            c.source,
            if c.floor > 0 {
                format!(" 门槛{}", c.floor)
            } else {
                String::new()
            },
            if c.one_shot { " 一次性" } else { "" },
        )
    }));
    lines
}

/// 牌堆那一行。定序与洗牌在盘面上是两件事：前者「下一张是谁」能从数据行序算出来，
/// 后者只能看这一局抽到了什么。少了这个位，「命运怎么每次都发同一张」就只能回去读代码。
fn format_decks(view: &DumpView) -> String {
    let mut rows: Vec<(u8, String)> = view
        .decks
        .iter()
        .map(|d| {
            (
                match d.kind {
                    CardKind::Chance => 0,
                    CardKind::Fate => 1,
                    CardKind::Item => 2,
                },
                format!(
                    "{:?}{} {}/{}",
                    d.kind,
                    match d.order {
                        DeckOrder::Fixed => " 定序",
                        DeckOrder::Shuffled => " 洗牌",
                    },
                    d.remaining(),
                    d.total(),
                ),
            )
        })
        .collect();
    // 查询顺序是生成顺序，截图与断言不该依赖它（与玩家、地块那两栏同一条）
    rows.sort();
    if rows.is_empty() {
        return "世界未装配".to_string();
    }
    rows.into_iter()
        .map(|(_, line)| line)
        .collect::<Vec<_>>()
        .join(" · ")
}

/// 百货柜台这一趟摆的货：`免罪卡20 · 涨价卡40`（后面的数是点券价）。
/// 空货架写「空」而不是省略这一行：还没进店、被买空、商品池是空的 —— 这三种在截图上
/// 本来就该长得一样，而「这一行不见了」读起来像面板坏了。
fn format_stock(view: &DumpView) -> String {
    if view.stock.slots.is_empty() {
        return "空（没进过店，或这一趟被买空了）".to_string();
    }
    view.stock
        .slots
        .iter()
        .map(|card| {
            view.shelf.get(CardKind::Item, *card).map_or_else(
                || format!("<缺{}>", card.0),
                |def| format!("{}{}", def.title, def.cost_points.unwrap_or(0)),
            )
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

fn format_players(view: &DumpView) -> Vec<String> {
    let mut rows: Vec<(u32, String)> = view
        .players
        .iter()
        .map(
            |(entity, id, name, wallet, points, bank, jail, bankrupt, hand, shield)| {
                let flag = if bankrupt.is_some() {
                    "破产".to_string()
                } else if let Some(sentence) = jail {
                    format!("服刑剩{}回合", sentence.turns_left)
                } else {
                    "-".to_string()
                };
                // 手牌打中文名：查「这张卡为什么没生效」时，下标还得再翻一次数据文件
                let cards = hand.map_or("-".to_string(), |h| {
                    if h.is_empty() {
                        return "-".to_string();
                    }
                    h.cards
                        .iter()
                        .map(|c| {
                            view.shelf
                                .get(CardKind::Item, *c)
                                .map_or_else(|| format!("<缺{}>", c.0), |d| d.title.clone())
                        })
                        .collect::<Vec<_>>()
                        .join("·")
                });
                // 护身是「打过免罪卡之后」唯一留下的状态。查陷害为什么没生效，
                // 看的就是这一栏 —— 它在手牌面板上只有一行小字，容易被当成装饰
                let guard = shield.map_or("-".to_string(), |s| match s.remaining() {
                    None => "免疫".to_string(),
                    Some(n) => n.to_string(),
                });
                (
                    id.0,
                    format!(
                        "P{:<2} {:<6} 现金 {:>7} 点券 {:>4} 存 {:>6} 贷 {:>6} {:<12} 牌 {:<12} 护 {:<4} {:?}",
                        id.0,
                        name.0,
                        wallet.cash,
                        points.0,
                        bank.deposit,
                        bank.loan,
                        flag,
                        cards,
                        guard,
                        entity
                    ),
                )
            },
        )
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, row)| row).collect()
}

fn format_lands(view: &DumpView) -> Vec<String> {
    let def = view.active.as_ref().map(|a| &a.def);
    let mut rows: Vec<(u32, String)> = view
        .plots
        .iter()
        .map(|(id, owner, level, mortgage, status)| {
            let name = def
                .and_then(|d| d.land(id.0))
                .map_or("", |l| l.name.as_str());
            (
                id.0,
                format!(
                    "L{:<2} {:<8} owner {:?} Lv{} {} {}",
                    id.0,
                    name,
                    owner.0.map(|p| p.0),
                    level.0,
                    // 本金要说得出数：「抵押」与「押了 750、赎回要 825」在查账时是两件事
                    if mortgage.held() {
                        format!("抵押{}", mortgage.proceeds)
                    } else {
                        String::new()
                    },
                    // 限时状态要说得出还剩几天：「涨价封住了这条街」和「明天就到期」在盘面上是两件事
                    status.map_or_else(String::new, |t| format!(
                        "{}剩{}天",
                        t.kind.label(),
                        t.days_left
                    )),
                ),
            )
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, row)| row).collect()
}
