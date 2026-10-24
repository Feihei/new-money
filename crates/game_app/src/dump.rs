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
    ActiveBoard, ArcadeDeal, AtmAccess, Auction, BankBalance, Bankrupt, BuildingLevel, CardKind,
    CompanyLedgers, Corrections, CouponBoxes, Deck, DeckOrder, Detention, Facility, GameRules,
    GameSeed, GameSession, Gavel, Hand, Heading, Hired, HolidaySet, Institution, Insulation,
    Insured, Inventory, Jackpot, LandId, LandStatus, Landing, LotteryTickets, MarketClock,
    Mortgaged, Mount, MoveCap, Occupancy, OccupancyAt, Owner, Paused, Pending, PendingDecision,
    PlayerId, PlayerName, Points, Possession, Shelf, ShopStock, StaySentence, StockHoldings, Timed,
    TokenPosition, TurnOrder, TurnPhase, Villain, Wallet, resists,
};

use crate::asset_root;

use crate::ui::finance::comma;

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

/// 面板上每个玩家要摊开的那一栏。命名出来是因为这种十几元素的查询元组会撞
/// clippy 的 `too_complex_types`，而那个 lint 给的「抽 `type`」正是对的处理。
/// P3a 加了手牌与护身两栏之后更是要一直挂着这个别名。
type PlayerRow = (
    Entity,
    &'static PlayerId,
    &'static PlayerName,
    &'static Wallet,
    &'static Points,
    &'static BankBalance,
    Option<&'static Detention>,
    // 停留卡的计时（P3d④c）。与拘留各占一格而不是合成一个「处境」：面板要能同时
    // 读出两张计时各自剩几回合，合成一个字段就只剩一个数了
    Option<&'static StaySentence>,
    Option<&'static Bankrupt>,
    Option<&'static Hand>,
    // 护身的**两个来源**成对读：卡给的层数（组件）与神给的免疫（现读 `Possession`，
    // P6b②）。合成一格而不是相邻两格有两个理由：语义上它们回答同一个问题（「这次陷害
    // 为什么没生效」），而 Bevy 的 `QueryData` 元组实现到 15 元为止 —— 第 16 格会在这里
    // 报「not valid to request as data in a Query」，症状看着像查询写坏了而不是行数超限。
    (Option<&'static Insulation>, Option<&'static Possession>),
    &'static Heading,
    // P5：道具栏、载具、乌龟夹步。三样都是「他这回合怎么走」的输入 / 结果，
    // 与手牌分开念 —— 查「遥控骰子为什么没生效」时，道具栏与夹步必须同时看得见
    &'static Inventory,
    &'static Mount,
    Option<&'static MoveCap>,
);

/// 面板上每块地要摊开的那一栏。P3c 加进限时状态之后，就地凑齐了 clippy 的
/// `too_complex_types` 阈值，处理方式与 [`PlayerRow`] 同一条：给它名字，不加 `#[allow]`。
type PlotRow = (
    &'static LandId,
    &'static Owner,
    &'static BuildingLevel,
    &'static Mortgaged,
    Option<&'static Timed<LandStatus>>,
    Option<&'static Facility>,
);

/// 面板要读的全部世界状态。收成一个参数包是这仓库的房规：需要知道的就是这么多，
/// 而 `#[allow(clippy::too_many_arguments)]` 会把这个信号吃掉。
// 恶人那一条查询要一次读八样（他是谁、在哪、替谁、被什么按住）—— 拆散就答不全，
// 而这张面板的职责本来就是「把世界摆出来给人扫读」。
#[allow(clippy::type_complexity)]
#[derive(SystemParam)]
pub struct DumpView<'w, 's> {
    session: Option<Res<'w, GameSession>>,
    seed: Res<'w, GameSeed>,
    order: Option<Res<'w, TurnOrder>>,
    active: Option<Res<'w, ActiveBoard>>,
    phase: Res<'w, State<TurnPhase>>,
    paused: Res<'w, Paused>,
    clock: Res<'w, MarketClock>,
    holidays: Res<'w, HolidaySet>,
    landing: Res<'w, Landing>,
    pending: Res<'w, PendingDecision>,
    /// 游乐场那一手的布局种子（P7）。它是「这一屏为什么埋在那几格」的唯一答案，
    /// 而挖宝那一格的分是账上的事 —— 查「他怎么就挖到了六件」时要能看见种子
    deal: Res<'w, ArcadeDeal>,
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
    /// 奖池（P8）。第三张账：它不是任何人的钱包，所以现金审计与 HUD 都读不到它，
    /// 而「池子为什么是这个数」正是查乐透时要问的那一句
    jackpot: Res<'w, Jackpot>,
    /// 一注多少钱、号码池有多大（P8）。池子那两个数不在池里，而在规则里 ——
    /// 查「买了一张怎么池子涨了 1000」要能看见这两个数
    rules: Res<'w, GameRules>,
    /// 谁手里有哪几个号（P8）。没并进 [`PlayerRow`] 是因为那张元组已经到 Bevy 的
    /// `QueryData` 15 元上限（见它的注释），第 16 格会在编译期炸
    tickets: Query<
        'w,
        's,
        (
            &'static PlayerId,
            &'static PlayerName,
            &'static LotteryTickets,
        ),
    >,
    /// 拍卖机（P9）。这台机器的状态**不在任何组件里**：正在拍哪块、喊到哪、还剩谁、后面排着几块，
    /// 全在这一个资源上 —— 而面板每帧都在换主语（问的是下一个出价的人），
    /// 顶栏那一行只活两三帧。查「拍卖为什么停在半空」只有这一条路。
    auction: Res<'w, Auction>,
    /// 公司损益账（P10d①）。分红按它分，查「这次分红为什么是这个数」先看这一页
    ledgers: Res<'w, CompanyLedgers>,
    /// 每位玩家持有什么股 —— 董事长那一半要从这里现算（持股最多者，破产者不算）
    holdings: Query<
        'w,
        's,
        (
            &'static PlayerId,
            &'static PlayerName,
            &'static StockHoldings,
            Option<&'static Bankrupt>,
        ),
    >,
    /// 道路放置物（P5）。要同时看得见两种位置：落在哪一格、以及在谁身上剩几步 —
    /// 「炸弹在哪」是查一局为什么突然有人住院时最先要问的事
    occupancies: Query<'w, 's, (Entity, &'static Occupancy)>,
    /// 新闻池（P11）。`GameSession::last_news` 存的是**下标**，而「上一条是哪一条」要说得出
    /// 标题就得读池子 —— 查「为什么连着两条一样」/「那条天灾这局到底播过没有」只有这一格
    news: Option<Res<'w, game_core::prelude::NewsSet>>,
    /// 保单（P10d②）。**不并进 [`PlayerRow`]**：那张元组已经顶到 Bevy `QueryData`
    /// 的 15 元上限（见它的注释），第 16 格会在编译期炸 —— 与乐透票、持股同一条处理：
    /// 另开一条查询，按实体对回去
    insured: Query<'w, 's, (Entity, &'static Insured)>,
    /// 四大恶人（P12）。同样**不并进 `PlayerRow`**，而且理由还前面一层：他们不是玩家，
    /// 进不了那张按 `PlayerId` 收的表。这一栏要说的是「谁在笼里等保释、谁在环上替谁走路、
    /// 走到第几格、被什么按住」—— 查「恶人为什么一动不动」与「保释窗口为什么没他」时，
    /// 这是唯一一处能同时看见 `Detention`/`Hired`/`TurnOrder` 三者关系的地方。
    villains: Query<
        'w,
        's,
        (
            Entity,
            &'static Villain,
            &'static TokenPosition,
            Option<&'static Detention>,
            Option<&'static Hired>,
            Option<&'static StaySentence>,
            Option<&'static MoveCap>,
            &'static PlayerName,
        ),
    >,
    /// 名字（雇主那一半从这里读）。恶人也挂 `PlayerName`，见那个组件的注释
    names: Query<'w, 's, &'static PlayerName>,
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
            row(ui, "新闻", format_news(&view));
            row(ui, "停摆", format_suspensions(&view));
            for line in format_villains(&view) {
                row(ui, "恶人", line);
            }
            row(ui, "待决", format_pending(&view));
            row(ui, "资格", format_access(&view));
            row(ui, "牌堆", format_decks(&view));
            row(ui, "货架", format_stock(&view));
            row(ui, "奖池", format_jackpot(&view));
            row(ui, "公司", format_companies(&view));
            row(ui, "拍卖", format_auction(&view));
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
            ui.separator();
            for line in format_occupancies(&view) {
                ui.monospace(line);
            }
        });
}

fn row(ui: &mut egui::Ui, key: &str, value: String) {
    ui.monospace(format!("{key:　<4}{value}"));
}

/// 新闻那一栏（P11）：上一条是哪一条、池子有几条、这一格抽不抽得到别人。
///
/// 「同场不连重」的判据住在 `news::draw` 里，而它读的就是这里那个下标 —— 面板把它翻成
/// 标题，是为了「查为什么连着两条一样」时不必回去数 JSON 的行。
fn format_news(view: &DumpView) -> String {
    let Some(session) = &view.session else {
        return "世界未装配".to_string();
    };
    let pool = view.news.as_ref().map(|set| set.0.len()).unwrap_or(0);
    let last = session
        .last_news
        .and_then(|id| {
            view.news
                .as_ref()
                .and_then(|set| game_core::prelude::news_of(set, id))
                .map(|def| format!("#{}「{}」", id.0, def.title))
        })
        .unwrap_or_else(|| "还没播过".to_string());
    format!("上一条 {last} · 池 {pool} 条")
}

/// 机构停摆那一栏（P11c②）：哪一家停着、还剩几天。
///
/// 全都没停也写一句「两家都开着」而不是空着 —— 这一栏的全部用途是回答「为什么点不动
/// 那颗按钮」，而空栏既可能是「没停」也可能是「面板没读到」，那两种在排查时差一整趟。
fn format_suspensions(view: &DumpView) -> String {
    let Some(session) = &view.session else {
        return "世界未装配".to_string();
    };
    let halted = [Institution::StockMarket, Institution::Bank]
        .iter()
        .filter_map(|what| {
            let days = session.suspensions.days_left(*what);
            (days > 0).then(|| format!("{}{}剩{days}天", what.label(), what.halt_phrase()))
        })
        .collect::<Vec<_>>()
        .join(" · ");
    if halted.is_empty() {
        "两家都开着".to_string()
    } else {
        halted
    }
}

/// 恶人那一栏（P12a）：谁在笼里等保释、谁在环上替谁走路、被什么按住。
///
/// **一人一行**而不是挤成一行：这一栏要说的是「他在哪、替谁、被什么按住」三段话，
/// 四个人接在一起会超出面板宽度被裁掉，而它存在的意义正是让人不必回去读组件。
/// 一个都不写也要出声（「场上没有恶人」），理由与「停摆」那一栏同一条：空栏分不清
/// 「装配没跑」与「面板没读到」，那两种在排查时差一整趟。
fn format_villains(view: &DumpView) -> Vec<String> {
    let current = view.order.as_ref().and_then(|o| o.current());
    let mut rows: Vec<(String, String)> = view
        .villains
        .iter()
        .map(|(entity, villain, at, held, hired, stay, cap, name)| {
            // 他在环上的哪一格 / 在笼里 —— 处境与格子写在一起：查「他为什么不走」时
            // 这两个数必须同时看得见（在笼里的人也有 `TokenPosition`，那是他的视觉停车位）
            let where_ = match held {
                Some(d) => crate::ui::dialog::detention_phrase(d),
                None => {
                    let tile = format!("格{}", at.tile.0);
                    match hired {
                        Some(job) => {
                            // 雇主的名字读不到就退回实体号（面板别的地方也这么念）：
                            // 这一栏要说的是「他替谁」，而「查不到」本身正是要看得见的那件事
                            let boss = view
                                .names
                                .get(job.employer)
                                .map(|n| n.0.clone())
                                .unwrap_or_else(|_| format!("{:?}", job.employer));
                            format!("{tile}·替 {boss} 办事")
                        }
                        // 没人雇却在环上走 = 那两个位分叉了，这一格就是把矛盾摊在屏幕上
                        None => format!("{tile}·无人雇用"),
                    }
                }
            };
            let mut flag: Vec<String> = Vec::new();
            if current == Some(entity) {
                flag.push("正在行动".to_string());
            }
            if let Some(sentence) = stay {
                flag.push(format!("停留剩{}回合", sentence.turns_left));
            }
            if let Some(cap) = cap {
                flag.push(format!("少动{}", cap.turns_left));
            }
            let flag = if flag.is_empty() {
                "-".to_string()
            } else {
                flag.join(" ")
            };
            (
                name.0.clone(),
                format!(
                    "{:<4} {:<18} {:<12} {:?}",
                    villain.label(),
                    where_,
                    flag,
                    entity
                ),
            )
        })
        .collect();
    if rows.is_empty() {
        return vec!["场上没有恶人（装配缺位）".to_string()];
    }
    rows.sort();
    rows.into_iter().map(|(_, row)| row).collect()
}

fn format_session(view: &DumpView) -> String {
    let Some(session) = &view.session else {
        return format!("世界未装配（种子 {:#x}）", view.seed.0);
    };
    let date = game_core::prelude::date_of(view.clock.tick, session.epoch);
    let closed = if view.holidays.is_market_closed(date) {
        " · 股市休市"
    } else {
        ""
    };
    let holiday = view
        .holidays
        .holiday_name(date)
        .map(|name| format!(" · {name}"))
        .unwrap_or_default();
    let cover = view
        .holidays
        .coverage()
        .map(|(first, last)| format!("{first}–{last}"))
        .unwrap_or_else(|| "空表".to_string());
    format!(
        "第 {} 回合 · 种子 {:#06x} · 日历 {date}{holiday}（行情 tick {}）{closed} · 物价 ×{:.2} · 节日表 {cover}",
        session.turn_count, session.seed, view.clock.tick, session.price_index
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
        // 游乐场那个窗口多带一个数：布局种子决定了挖宝埋在哪几格（P7）
        Some((player, about @ Pending::Minigame(_))) => {
            format!("{about:?} ← {player:?} · 布局种子 {}", view.deal.seed)
        }
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

/// 公司那一栏（P10d①）：`暴富科技 当月 1,200 · 董事长 P0阿土伯`。
///
/// 账页只对「有过进出」的公司开（董事长免单连账都不产生），所以盘上的公司格也要扫一遍 ——
/// 有格没账写「当月 0」，那一行空着读起来像面板坏了。董事长在这里现算而不是缓存：
/// 与规则层同一条口径（持股最多、破产者不算），并列取先遇到的（持股人查询顺序稳定）。
fn format_companies(view: &DumpView) -> String {
    let Some(active) = view.active.as_ref() else {
        return "没有棋盘".to_string();
    };
    let companies: Vec<&game_core::prelude::TileDef> = active
        .def
        .tiles
        .iter()
        .filter(|t| t.kind == game_core::prelude::TileKind::Company)
        .collect();
    if companies.is_empty() {
        return "盘上没有公司格".to_string();
    }
    companies
        .iter()
        .map(|tile| {
            let stock = tile.data.stock.clone().unwrap_or_default();
            let net = view.ledgers.find(&stock).map_or(0, |page| page.net());
            // 董事长：持股最多的在世者。与 core 的 `chairman_of` 同一条判据 ——
            // 这里不调它是因为账本面板不开第二条 `Books`（那边全是 ResMut）
            let chairman = view
                .holdings
                .iter()
                .filter(|(.., bankrupt)| bankrupt.is_none())
                .max_by_key(|(.., held, _)| held.get(&game_core::prelude::StockId(stock.clone())))
                .filter(|(.., held, _)| held.get(&game_core::prelude::StockId(stock.clone())) > 0)
                .map(|(id, name, ..)| format!("P{}{}", id.0, name.0));
            format!(
                "{} 当月 {} · {}",
                tile.name,
                comma(net),
                chairman.unwrap_or_else(|| "无董事长".to_string())
            )
        })
        .collect::<Vec<_>>()
        .join(" / ")
}

/// 奖池那一栏（P8）：`池 3000 · 上期 7 号 · 一注 1000/36 号 · P0阿土伯[3,17]`。
///
/// 三个来源要写在一起，因为这一栏回答的是「池子为什么是这个数」：进池只有买券与月息
/// 两条路（10-22 拍板），而这两条都在别处的流水里 —— 面板上只有把「谁持有什么」摊开，
/// 玩家才对得上账。一个号都没人买时也照写「没人买」：那一行空着读起来像面板坏了。
fn format_jackpot(view: &DumpView) -> String {
    let drawn = match view.jackpot.last_draw {
        Some(n) => format!("上期 {n} 号"),
        None => "还没开过奖".to_string(),
    };
    let mut held: Vec<String> = view
        .tickets
        .iter()
        .filter(|(.., tickets)| !tickets.numbers.is_empty())
        .map(|(id, name, tickets)| {
            format!(
                "P{}{}[{}]",
                id.0,
                name.0,
                tickets
                    .numbers
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            )
        })
        .collect();
    // 按玩家序排：查询的迭代序是 archetype 里的行序，与 `P0/P1` 的编号无关，
    // 不排就会在同一份世界上截出两种顺序（截图核对读不出差别，而人会以为世界变了）
    held.sort();
    format!(
        "池 {} · {drawn} · 一注 {}/{} 号 · {}",
        view.jackpot.pot,
        view.rules.ticket_price,
        view.rules.lottery_numbers,
        if held.is_empty() {
            "没人买".to_string()
        } else {
            held.join(" ")
        }
    )
}

fn format_auction(view: &DumpView) -> String {
    let machine = &view.auction;
    let queued = machine.standby.len() + machine.liquidation.len();
    let Some(lot) = machine.lot.as_ref() else {
        return format!(
            "没在拍{}",
            if queued > 0 {
                format!(" · 还排着 {queued} 块")
            } else {
                String::new()
            }
        );
    };
    // 「槌是谁敲的」与「从谁手里拍走」两句话把三种入口分开：清算与公有地都没有开槌的人，
    // 而它们的钱去的是同一个地方（银行）却来自不同的事（死者留下 / 政府挂牌）——
    // 省一句就分不清，而这一栏的全部用途就是让人看出「钱为什么是这个数」
    let struck = match lot.gavel {
        Gavel::Card(founder) => format!("槌 {}", who_are(view, founder)),
        Gavel::Liquidation(_) => "破产变卖".to_string(),
        Gavel::Public => "公有地挂牌".to_string(),
    };
    let seller = lot
        .seller
        .map_or_else(|| "无主".to_string(), |former| who_are(view, former));
    let leader = lot.leader.map_or_else(
        || "还没人认价".to_string(),
        |front| format!("最高 {}({})", lot.high, who_are(view, front)),
    );
    let fighting = lot
        .pool
        .iter()
        .map(|e| who_are(view, *e))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{} · 从 {seller} 手里 · {struck} · {leader} · 该 {} · 还在争 {fighting}{}",
        plot_of(view, lot.land),
        lot.bidder()
            .map_or_else(|| "没人".to_string(), |e| who_are(view, e)),
        if queued > 0 {
            format!(" · 后面排着 {queued} 块")
        } else {
            String::new()
        },
    )
}

/// 地块名。菜单态没有棋盘，所以每一处查名字都要走 `Option<Res<ActiveBoard>>` 这一层。
fn plot_of(view: &DumpView, land: LandId) -> String {
    view.active
        .as_ref()
        .and_then(|board| board.def.land(land.0))
        .map_or_else(
            || format!("L{}", land.0),
            |plot| format!("L{} {}", plot.id, plot.name),
        )
}

/// 一个 `Entity` 是谁。写成「P1孙小美」而不是只有名字：面板上四个玩家可能重名，
/// 而 dump 是拿来对账的，编号才是别的路径上也查得到的那个键。
fn who_are(view: &DumpView, player: Entity) -> String {
    view.players
        .get(player)
        .map(|row| format!("P{}{}", row.1.0, row.2.0))
        .unwrap_or_else(|_| "?".into())
}

fn format_players(view: &DumpView) -> Vec<String> {
    // 保单按实体对回去（见 DumpView.insured 的注释）：身上有单的人才写「投保剩N天」
    let insured: std::collections::HashMap<Entity, &Insured> = view.insured.iter().collect();
    let mut rows: Vec<(u32, String)> = view
        .players
        .iter()
        .map(
            |(
                entity,
                id,
                name,
                wallet,
                points,
                bank,
                held,
                stay,
                bankrupt,
                hand,
                (shield, god),
                heading,
                inventory,
                mount,
                cap,
            )| {
                // 处境这一栏可以同时写两件事：拘留与停留各有自己的计时（`begin_turn_system`
                // 两条各扣各的），面板只写「更重的那一个」就会让人以为早的那张卡没挂上。
                // 拘留那一格带着他在哪儿 —— 三个去处在规则上是一族，在读数上是三句话
                let mut flag: Vec<String> = Vec::new();
                if bankrupt.is_some() {
                    flag.push("破产".to_string());
                } else {
                    if let Some(d) = held {
                        // 一句人话两处共用（顶栏与这张面板），所以映射不在这里重写一遍
                        flag.push(crate::ui::dialog::detention_phrase(d));
                    }
                    if let Some(sentence) = stay {
                        flag.push(format!("停留剩{}回合", sentence.turns_left));
                    }
                    if let Some(cap) = cap {
                        flag.push(format!("少动剩{}", cap.turns_left));
                    }
                    // 保单（P10d②）：天数是他的回合数（与拘留同族），剩几天一眼对得上
                    // 「投保 4 天、走了自己 3 个回合还剩 1」。赔付的那一半看流水里的 Indemnity
                    if let Some(policy) = insured.get(&entity) {
                        flag.push(format!("投保剩{}天", policy.days_left));
                    }
                    // 融资债（P10d②）：银行保险董事长的挪用账。挂在存款上但**不是贷款**
                    // （不计息、反超即收回），所以不能并进「贷」那一格让人误当借款
                    if bank.financing > 0 {
                        flag.push(format!("融资债{}", bank.financing));
                    }
                    if *mount != Mount::Foot {
                        flag.push(mount.label().to_string());
                    }
                }
                // 方向与处境共用这一栏（正向什么都不写，一个永远亮着的标记等于没有）：
                // 查「他这一回合为什么往回走」时，那一行得同时看得见处境与方向
                if *heading == Heading::Backward {
                    flag.push("逆向".to_string());
                }
                // 神带着**天数**，而天数数的是他自己的回合（与拘留同族、与地块的涨价不同），
                // 所以这一格写的必须是「剩几回合」而不是「第几天」—— 查验收③
                // 「坐牢期间神的 7 天照算」时，两个数要在同一行里对得上
                if let Some(held_god) = god {
                    flag.push(format!("{}剩{}", held_god.god.label(), held_god.days_left));
                }
                let flag = if flag.is_empty() {
                    "-".to_string()
                } else {
                    flag.join(" ")
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
                // 看的就是这一栏 —— 它在手牌面板上只有一行小字，容易被当成装饰。
                // 两个来源分开念：`免疫` 是那位神给的（不消耗、随神消失），数字是卡给的层数
                let guard = match (
                    god.map(|held| held.god).is_some_and(resists),
                    shield.map(|s| s.charges).filter(|n| *n > 0),
                ) {
                    (true, Some(n)) => format!("免疫+卡{n}"),
                    (true, None) => "免疫".to_string(),
                    (false, Some(n)) => n.to_string(),
                    (false, None) => "-".to_string(),
                };
                // 道具栏按 kind 数：与手牌分开念 —— 这两条库存的规则本来就不一样
                let items = if inventory.is_empty() {
                    "-".to_string()
                } else {
                    inventory
                        .entries()
                        .iter()
                        .map(|(kind, n)| format!("{}{n}", kind.label()))
                        .collect::<Vec<_>>()
                        .join("·")
                };
                (
                    id.0,
                    format!(
                        "P{:<2} {:<6} 现金 {:>7} 点券 {:>4} 存 {:>6} 贷 {:>6} {:<18} 牌 {:<12} 护 {:<4} 具 {:<12} 载 {:<4} {:?}",
                        id.0,
                        name.0,
                        wallet.cash,
                        points.0,
                        bank.deposit,
                        bank.loan,
                        flag,
                        cards,
                        guard,
                        items,
                        mount.label(),
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
    // 谁被关着（P4②）。地块那一栏要说得出「业主在住院，所以这一趟不收租」—— 查「这个 0
    // 是怎么来的」正是这个面板存在的理由。读的是 `players` 那一栏的同一个来源，
    // 不在这里再查一次组件（那会出现第二种问法）
    let locked: Vec<(u32, &str)> = view
        .players
        .iter()
        .filter_map(|(_, pid, _, _, _, _, held, ..)| held.map(|d| (pid.0, d.place.label())))
        .collect();
    let mut rows: Vec<(u32, String)> = view
        .plots
        .iter()
        .map(|(id, owner, level, mortgage, status, facility)| {
            let name = def
                .and_then(|d| d.land(id.0))
                .map_or("", |l| l.name.as_str());
            (
                id.0,
                format!(
                    "L{:<2} {:<8} owner {:?} {} {} {} {} {}",
                    id.0,
                    name,
                    owner.0.map(|p| p.0),
                    // 等级念 `building_label` 那一套（内部 Lv1＝空地，屏幕上写「空地」；
                    // 内部 Lv2 才是「建筑 Lv1」）—— 这个面板是给人查账的，读出来的编号
                    // 必须与 UI 那四块屏是同一个数，否则「为什么这块 Lv2 的地只值 1,500」又要问一遍
                    crate::ui::building_label(level.0),
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
                    owner
                        .0
                        .and_then(|p| {
                            locked
                                .iter()
                                .find(|(id, _)| *id == p.0)
                                .map(|(_, label)| format!("业主在{label}·不收租"))
                        })
                        .unwrap_or_default(),
                    // 商业设施（P10c）。空商业地读出来是空的（那一格本来就该是空），
                    // 而「这块地为什么收了 600」要能一眼看见立的是什么
                    facility.map_or_else(String::new, |f| f.0.label().to_string()),
                ),
            )
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, row)| row).collect()
}

/// 道路放置物那一栏（P5）。地上的按格号排，携带中的排最后 —— 后者不是拓扑上的东西，
/// 它是一条「还会走几步」的计时。「炸弹在谁身上、还剩几步」是这一栏存在的全部理由。
fn format_occupancies(view: &DumpView) -> Vec<String> {
    if view.occupancies.is_empty() {
        return vec!["道路放置物：无".to_string()];
    }
    let def = view.active.as_ref().map(|a| &a.def);
    let mut rows: Vec<(u32, String)> = view
        .occupancies
        .iter()
        .map(|(_, occ)| match occ.at {
            OccupancyAt::OnTile(tile) => {
                let name = def
                    .and_then(|d| d.tile(tile.0 as i64))
                    .map_or("?", |t| t.name.as_str());
                (
                    tile.0,
                    format!("格{:<2} {:<8} {}", tile.0, name, occ.kind.label()),
                )
            }
            OccupancyAt::Carried { by, steps_left } => {
                let pid = view
                    .players
                    .iter()
                    .find(|(entity, ..)| *entity == by)
                    .map_or("?".to_string(), |(_, id, ..)| format!("P{}", id.0));
                (
                    u32::MAX,
                    format!("定时炸弹 在 {pid} 身上 剩 {steps_left} 步"),
                )
            }
        })
        .collect();
    rows.sort();
    rows.into_iter().map(|(_, row)| row).collect()
}
