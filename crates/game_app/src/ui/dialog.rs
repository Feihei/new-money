//! 四个模态：买地、升级、租金收据、卡面通知。
//!
//! 对应 Godot `property_dialog.gd` / `upgrade_dialog.gd` / `rent_notice.gd` / `card_notice.gd`
//! （合计约 900 行，四个文件里 `_build_ui` 与 `_style_button` 几乎逐字重复）。
//!
//! 弹出时机不靠消息、靠 [`PendingDecision`]：规则层已经把「现在等谁回答什么」写成系统状态，
//! UI 只跟着它重建。用消息驱动会有一类漏弹 —— 弹窗 system 排在建报价的 system 之后时，
//! 本帧的消息要下一帧才读得到，而玩家在那一帧就已经能点东西了。
//!
//! 内容在弹出那一刻**烤死**，之后不随世界状态更新：待决事项被回答时 `PendingDecision`
//! 一定先清空再挂上下一个，所以每次报价都是一整块新面板，没有「忘了刷新」的余地。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, Ask, BailDeny, BailFacts, BankOp, BidDeny, BidFacts, BidMove, Blocker, Books,
    BuildingKind, BuildingLevel, CardDef, CardDrawn, CardId, CardKind, CardNotice, ChargeKind,
    CompanyCharged, Decision, Detention, DetentionPlace, EffectOutcome, Facility, Gate, Gavel,
    Hand, Institution, Jackpot, LandId, LandStatus, LandType, Lot, LotteryDeny, LotteryTickets,
    MagicFilter, MagicRitual, MagicSpell, MortgageDeny, MortgageOp, Owner, PassiveCard,
    PassiveEvent, PassiveMove, Pending, PendingDecision, PlayerId, PlayerName, Rng, ServiceCharged,
    Shelf, ShopDeny, ShopStock, TradeSide, TurnOrder, bail_op_allowed, bid_allowed, bidders,
    blame_candidates, buy_allowed, buyback, counter_plots, filter_targets, mortgage_op_allowed,
    read_facts, restock, shop_op_allowed,
};

use super::finance::{comma, money_row};
use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, SECONDARY, UiAction, building_label, button, info_row, label, overlay,
    panel, separator,
};
use crate::art;
use crate::render::{DevFont, palette};

/// 模态根节点，兼作「现在挂着的是哪个问题」与「上次写成什么样」两份记录。
///
/// 键带 `Entity` 而不是只带 `Pending`：同一块地连续两次挂牌（甲拒买、乙又踩到）时
/// `about` 相等的，只有玩家不同才会重建 —— 少带这一半就会给乙看甲的现金。
///
/// `stamp` 只有柜台用得上（其余模态是空的）：柜台是一个**窗口**而不是一个报价，玩家能在
/// 里面连办四笔，每次余额都变。没有这份签名就会出现「已经存下 1,000，面板还显示存之前
/// 的现金」—— 而那正是让人按错第二次的形状。
///
/// 为什么不存成 resource（先前是 `OpenModal`）：回主菜单时面板随 `BoardUi` 一起被收走，
/// 而资源里的 `Entity` 句柄与「上次挂的是什么」会活下来。下一局第一次报价若与上一局
/// 最后一次恰好相同（`Buy(LandId(0))` 很常见），判据一比就相等 → 直接 return →
/// **模态永远不再出现**，回合机停在 `WaitDecision` 等一个不存在的按钮。挂在实体上，
/// 实体没了键就没了，不存在需要重置的残留。
#[derive(Component)]
pub(crate) struct ModalRoot {
    pub key: (Entity, Pending),
    pub stamp: String,
}

/// 卡面正文。`Pending::Card` 只说「有一张牌要读」，不说是哪张 —— 规则层刻意不把字符串
/// 放进系统状态（`msg.rs` 的 `CardDrawn` 带 `CardNotice`），所以 UI 自己记最近一张。
/// 这一份**必须**在回菜单时清掉：它是消息留下的记忆，实体上没有对应物。
#[derive(Resource, Default)]
pub(crate) struct RecentCard(pub(crate) Option<CardNotice>);

/// 最近一笔服务费（P10c）。`Pending::ServiceNotice` 只挂着「哪块地」，摇了几点、住了几天、
/// 收了多少全在 [`ServiceCharged`] 这条消息里 —— 与 [`RecentCard`] 同一条分工，
/// 也同一条纪律：它必须回主菜单时清掉（消息留下的记忆，实体上没有对应物）。
#[derive(Resource, Default)]
pub(crate) struct RecentService(pub(crate) Option<ServiceCharged>);

/// 最近一笔公司到访（P10d①）。`Pending::CompanyNotice` 只说「到过一家公司的门面」，
/// 摇了几点、住了几天、付了多少全在 [`CompanyCharged`] 里 —— 与 [`RecentService`]
/// 同一条分工，也同一条纪律：它必须回主菜单时清掉（消息留下的记忆，实体上没有对应物）。
#[derive(Resource, Default)]
pub(crate) struct RecentCompany(pub(crate) Option<CompanyCharged>);

/// 开发用：`NM_DECISION=buy|upgrade|rent|card|bank` 直接把那个模态挂出来。
///
/// 五个模态各有各的触发条件（升级要凑齐同色组、收据要有人踩过别人的地、柜台要正好停在
/// 银行格上），一局里未必碰得到，而 agent 没有鼠标、截图按墙钟 —— 与 `NM_PAUSE`、
/// `NM_PICK_TILE` 同一条理由：进不了截图的界面等于没做完。只写 [`PendingDecision`]，
/// 不碰回合机：模态由状态驱动，挂上就显示。点掉它会再挂出来，这条开关只为核对界面存在。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalKind {
    Buy,
    Upgrade,
    Rent,
    Card,
    Bank,
}

#[derive(Resource)]
pub(crate) struct ForceModal(Option<ModalKind>);

impl ForceModal {
    pub(crate) fn from_env() -> Self {
        let name = match std::env::var("NM_DECISION") {
            Ok(name) => name,
            Err(_) => return Self(None),
        };
        let kind = match name.as_str() {
            "buy" => ModalKind::Buy,
            "upgrade" => ModalKind::Upgrade,
            "rent" => ModalKind::Rent,
            "card" => ModalKind::Card,
            "bank" => ModalKind::Bank,
            _ => {
                warn!("NM_DECISION={name} 不认识，应为 buy|upgrade|rent|card|bank");
                return Self(None);
            }
        };
        Self(Some(kind))
    }
}

/// 把 `NM_DECISION` 要的那个待决事项挂上。排在 [`sync_modal`] 之前，同帧就显示。
pub(crate) fn force_decision(
    force: Res<ForceModal>,
    active: Res<ActiveBoard>,
    order: Res<TurnOrder>,
    mut pending: ResMut<PendingDecision>,
) {
    let Some(kind) = force.0 else { return };
    if pending.active().is_some() {
        return;
    }
    let Some(player) = order.current() else {
        return;
    };
    // 柜台不关于任何对象，直接挂
    if kind == ModalKind::Bank {
        pending.offer(player, Pending::BankTeller);
        return;
    }
    // 其余四个都要一块地：挑一块租金表最长、带同色组的，模态内容才不会是空壳
    let Some(land) = active
        .def
        .lands
        .iter()
        .filter(|l| l.group.is_some() && l.rent.len() > 1)
        .max_by_key(|l| l.rent.len())
        .map(|l| LandId(l.id))
    else {
        return;
    };
    let about = match kind {
        ModalKind::Buy => Pending::Buy(land),
        ModalKind::Upgrade => Pending::Upgrade(land),
        ModalKind::Rent => Pending::RentNotice(land),
        ModalKind::Card => Pending::Card,
        ModalKind::Bank => unreachable!("柜台在上面就已经挂好了"),
    };
    pending.offer(player, about);
}

pub(crate) fn remember_drawn_card(
    mut drawn: MessageReader<CardDrawn>,
    mut memory: ResMut<RecentCard>,
) {
    for card in drawn.read() {
        memory.0 = Some(card.card.clone());
    }
}

/// 开发用：`NM_DECISION=card` 时补一条真卡的 [`CardDrawn`]。
///
/// 那条开关的契约是「只写 [`PendingDecision`]」，于是卡面模态弹出来的是 [`fallback_note`]
/// 那一张空壳 —— 而这一屏正是卡面图（`assets/cards/`）唯一的非鼠标入口。判据同 `NM_SHOP`：
/// 进不了截图的界面等于没做完。
///
/// 发**机会池第一张**而不是随机一张：这一屏要核对的是图与文各占哪一栏，换一张卡不该
/// 连图一起换掉。`outcomes` 留空 —— 它是排版前置，不是一笔结算，伪造一条会让顶栏流水说谎。
///
/// 不必与 [`remember_drawn_card`]、[`sync_modal`] 争先后：卡 id 进了模态签名（见
/// [`sync_modal`] 里那一段），所以这一条晚一帧到屏幕上就只是重建一次那一屏。
pub(crate) fn dev_seed_card(
    order: Option<Res<TurnOrder>>,
    shelf: Shelf<'_>,
    mut drawn: MessageWriter<CardDrawn>,
    mut done: Local<bool>,
) {
    if *done || std::env::var("NM_DECISION").ok().as_deref() != Some("card") {
        return;
    }
    let Some(player) = order.as_ref().and_then(|o| o.current()) else {
        return;
    };
    let Some(def) = shelf.set(CardKind::Chance).and_then(|set| set.0.first()) else {
        return;
    };
    drawn.write(CardDrawn {
        player,
        card: CardNotice {
            id: def.id.clone(),
            kind: def.kind,
            title: def.title.clone(),
            desc: def.desc.clone(),
            outcomes: Vec::new(),
        },
    });
    *done = true;
}

/// 记下最近一笔服务费。排在 [`sync_modal`] 之前 —— 收据弹出来的那一帧就要读得到它
/// （与 [`remember_drawn_card`] 同一条时序理由）。
pub(crate) fn remember_service(
    mut charged: MessageReader<ServiceCharged>,
    mut memory: ResMut<RecentService>,
) {
    for bill in charged.read() {
        memory.0 = Some(*bill);
    }
}

/// 记下最近一笔公司到访。排在 [`sync_modal`] 之前 —— 收据弹出来的那一帧就要读得到它
/// （与 [`remember_service`] 同一条时序理由）。
pub(crate) fn remember_company(
    mut charged: MessageReader<CompanyCharged>,
    mut memory: ResMut<RecentCompany>,
) {
    for bill in charged.read() {
        memory.0 = Some(bill.clone());
    }
}

/// 模态这一屏要读的世界状态。打包成参数包是这仓库的房规：`sync_modal` 本来就要同时看
/// 「等谁回答」「上一张卡」「今天几号」「现在挂着的是哪块面板」，
/// 而 `#[allow(clippy::too_many_arguments)]` 会把「一个 system 要读八样东西」这个信号吃掉
/// （`.agents/ARCHITECTURE.md` §六 坑 8）。
#[derive(SystemParam)]
pub(crate) struct ModalView<'w, 's> {
    pending: Res<'w, PendingDecision>,
    memory: Res<'w, RecentCard>,
    /// 最近一笔服务费（P10c）。收据的读数全在这里：摇了几点、住了几天、收了多少
    bill: Res<'w, RecentService>,
    /// 最近一笔公司到访（P10d①）。收据的读数全在这里：哪家公司、董事长是谁、收了多少
    company_bill: Res<'w, RecentCompany>,
    /// 百货柜台这一趟的货架（P3d②）。它是资源而不是世界状态：同一时刻只有柜台前那位看得见
    stock: Res<'w, ShopStock>,
    /// 卡面文案在货架上（`ShopStock` 里只有下标）
    shelf: Shelf<'w>,
    /// 卖回那一排要读手牌。只有柜台用得上，其余模态不看它
    hands: Query<'w, 's, &'static Hand, With<PlayerId>>,
    /// 彩票柜台要读的第三张账（P8）。它是资源而不是世界状态：同一时刻只有柜台前那位看得见
    jackpot: Res<'w, Jackpot>,
    /// 被动卡那道门（P13b②）。这一屏念的是「门在问哪一句」，而那句话只有门自己说得出 ——
    /// 面板不许从 `Pending::Passive` 猜（那一格不带问句，也不带「已经转过一次」那位）
    gate: Res<'w, Gate>,
    /// 魔法屋这一场法事的草稿（P13）：第二步那一屏念的是「女巫按这条条件筛出了谁」，
    /// 而那份名单只在资源里 —— 面板不许自己再筛一遍，两处各筛迟早漂
    ritual: Res<'w, MagicRitual>,
    /// 回合机此刻在问谁。第一步那一屏的「走路的人」要按规则层同一个读数筛
    /// （`magic_filter_system` 读的是同一句 `order.current()`）
    order: Res<'w, TurnOrder>,
    /// 他手里有哪几个号。与 `hands` 同一形：只有彩票柜台用得上。
    /// 带 `Entity` 是因为独占那一格要问「**谁**占着这个号」，而 `iter()` 交的是数据项本身
    tickets: Query<'w, 's, (Entity, &'static LotteryTickets), With<PlayerId>>,
    open: Query<'w, 's, (Entity, &'static ModalRoot)>,
}

/// 跟着 `PendingDecision` 建/换/收模态。排在 [`remember_drawn_card`] 之后，
/// 于是卡面消息与面板内容在同一帧到位，不会出现「先弹一张空白卡」。
///
/// 重建判据是「问题 + 签名」两半：问题变了当然重建，而柜台那种**窗口**（一个待决事项里
/// 能办四笔）问的是同一个问题、数字却一直在动 —— 只看问题就会出现「钱已经存进去了，
/// 面板还停在存之前」。签名只对柜台有意义，其余四个模态的内容弹出即定稿。
pub(crate) fn sync_modal(
    mut commands: Commands,
    font: Res<DevFont>,
    server: Res<AssetServer>,
    desk: game_core::magic::FilterDesk<'_, '_>,
    names: Query<&PlayerName>,
    view: ModalView<'_, '_>,
) {
    // `Books` 从 `desk` 里拿，而不是再开一条参数：`FilterDesk` 本身就揣着一份（筛人要用账本），
    // 同一 system 里两条对 `GameSession` 的可变借用会在参数建立期直接撞
    // （§6.1 坑 33 的资源版，`magic.rs` 里那条 `CardSink` 的注写的是同一件事）
    let books = &desk.books;
    let want = view.pending.active();
    // 柜台前那位手里有哪几张（卖回那一排的内容，也进签名）
    let held: Vec<CardId> = want
        .and_then(|(player, _)| view.hands.get(player).ok())
        .map(|hand| hand.cards.iter().copied().collect())
        .unwrap_or_default();
    // 银行柜台那一节「你的地产」：只有柜台开着才算，别的模态不读地块表
    let plots: Vec<PlotRow> = match want {
        Some((player, Pending::BankTeller)) => plot_rows(books, player),
        _ => Vec::new(),
    };
    // 融资一排候选（P10d②）：只有银行保险的董事长在柜台前才算。
    // 名字在这里查好 —— 与 `bails`/`service` 同一条分工，收屏那一侧只管画
    let fins: Vec<(Entity, String, i64)> = match want {
        Some((player, Pending::BankTeller)) => {
            if game_core::company::chairman_of(books, game_core::company::FINANCE_STOCK)
                != Some(player)
            {
                Vec::new()
            } else {
                books
                    .solvent()
                    .into_iter()
                    .filter(|(e, _)| *e != player)
                    .map(|(e, _)| {
                        (
                            e,
                            names
                                .get(e)
                                .map(|n| n.0.clone())
                                .unwrap_or_else(|_| "？".to_string()),
                            books.bank_of(e).deposit,
                        )
                    })
                    .filter(|(.., d)| *d > 0)
                    .collect()
            }
        }
        _ => Vec::new(),
    };
    // 保释窗口的一排候选：只有保释窗口才算（P4③）
    let bails: Vec<BailRow> = match want {
        Some((player, Pending::Bail(place))) => bail_rows(books, &names, player, place),
        _ => Vec::new(),
    };
    // 服务费收据的读数（P10c）：只有那张收据才算。名字与地块名在这里查好，
    // 收据那一屏拿到的就是一份可以直接上屏的东西 —— 与 `bails` 同一条分工
    let service: Option<ServiceRow> = match want {
        Some((_player, Pending::ServiceNotice(land))) => {
            service_row(books, &names, &view.bill, land)
        }
        _ => None,
    };
    // 公司收据的读数（P10d①）：只有那张收据才算。与 `service` 同一条分工
    let company: Option<CompanyRow> = match want {
        Some((_player, Pending::CompanyNotice)) => company_row(books, &names, &view.company_bill),
        _ => None,
    };
    // 彩票柜台的读数：只有它开着才算（P8）。判据要的现金、号码池、已有的号都在这里，
    // 奖池那一半从 `view.jackpot` 读 —— 面板与规则层读的是同一份事实
    let lottery: Option<LotteryDesk> = match want {
        Some((player, Pending::Lottery)) => Some(LotteryDesk {
            cash: books.balance(player),
            price: books.rules.ticket_price,
            numbers: books.rules.lottery_numbers,
            held: view
                .tickets
                .get(player)
                .map(|(_, tickets)| tickets.numbers.clone())
                .unwrap_or_default(),
            taken: tickets_by_others(books, &names, &view.tickets, player),
            pot: view.jackpot.pot,
            last_draw: view.jackpot.last_draw,
        }),
        _ => None,
    };
    // 拍卖台（P9）的读数：只有窗口开着、而且机器里真的有一场时才填。
    //
    // 读的是 `books.auction` 而不是给 [`ModalView`] 另加一条 `Res<Auction>` —— `Books` 自己
    // 就带着那一格（破产那一刻要就地记下待拍的料），同一参数里两条对同一资源的可变借用会在
    // 参数建立期直接撞（`auction::AuctionDesk` 上面那段纪律，本轮踩过）。
    //
    // 窗口开着而 `lot` 是空的那一帧确实存在（散场时 `auction_hold_system` 先清窗口，
    // 但两者之间隔着一次消息投递），所以这里当它可能没有 —— 不 unwrap。
    let bid: Option<BidDesk> = match (want, books.auction.lot.clone()) {
        (Some((player, Pending::Auction)), Some(lot)) => Some(BidDesk {
            plot: books
                .active
                .def
                .land(lot.land.0)
                .map_or_else(|| "？".to_string(), |l| l.name.clone()),
            cash: books.balance(player),
            denied_by_god: books.acquire_denied(player),
            queued: books.auction.standby.len() + books.auction.liquidation.len(),
            eligible: bidders(books, lot.by()),
            lot,
        }),
        _ => None,
    };
    // 魔法屋第一步（P13）：11 条条件各自筛出谁。名单与规则层算的是**同一个函数**
    // （`magic::filter_targets`），所以「按钮亮着却筛不出人」与「面板说三个人、法术落到
    // 两个人」这两种形状都写不出来。空名单那几条灰着，理由就写在行右。
    // 「走路的人」按 `order.current()` 算 —— `magic_filter_system` 读的是同一句。
    let magic_rows: Vec<MagicRow> = match want {
        Some((player, Pending::Magic)) => {
            let walker = view.order.current().unwrap_or(player);
            MagicFilter::ALL
                .iter()
                .map(|f| MagicRow {
                    filter: *f,
                    who: filter_targets(*f, walker, &desk)
                        .iter()
                        .filter_map(|e| names.get(*e).ok().map(|n| n.0.clone()))
                        .collect(),
                })
                .collect()
        }
        _ => Vec::new(),
    };
    // 第二步念的是这场法事的草稿（规则层筛完存进去的那一份），不是再筛一遍：
    // 从「答完条件」到「点开法术」之间世界可能动了（有人破产、有人被新闻搬走），
    // 再筛一次会给玩家一份与真正挨那一下的名单不同的名单
    let magic_draft: Option<(MagicFilter, Vec<String>)> = match want {
        Some((_, Pending::MagicSpell(_))) => view.ritual.draft.as_ref().map(|(f, found)| {
            (
                *f,
                found
                    .iter()
                    .filter_map(|e| names.get(*e).ok().map(|n| n.0.clone()))
                    .collect(),
            )
        }),
        _ => None,
    };
    // 门这一屏的读数（P13b②）。窗口挂着而门里没有问句的那一帧确实存在（答完一问与队列空
    // 之间隔着一次消息投递），所以这里当它可能没有 —— 与拍卖台那一格同一个形状
    let gate_ask: Option<GateAsk> = match (want, view.gate.asking()) {
        (Some((_, Pending::Passive)), Some((who, ask, event))) => {
            // 这一问由哪张卡答，与门里 `next_question` 读的是**同一句**（表上的 `offset_card`）：
            // 面板自己挑一张就会出现「按钮写着嫁祸卡、门问的是免罪」
            let card = match ask {
                Ask::Offset => event.offset_card(),
                Ask::Shift => Some(PassiveCard::Shift),
            };
            // 手里的张数按 `passive` 那一位数（与 `GateDesk::holds`／`count` 同一格），
            // 不许按卡 id 数 —— 数据改一次行序，面板就会写着「2 张」而门只吃掉那一张
            let held = view
                .hands
                .get(who)
                .map(|hand| {
                    hand.cards
                        .iter()
                        .filter(|id| {
                            view.shelf
                                .get(CardKind::Item, **id)
                                .is_some_and(|def| def.passive == card)
                        })
                        .count() as u32
                })
                .unwrap_or(0);
            Some(GateAsk {
                ask,
                event,
                card,
                held,
                blame: match ask {
                    // 候选与门里「这一问要不要挂出来」用的是同一句（[`blame_candidates`]）：
                    // 两处各筛一遍的形状是「按钮亮着而门说不能转」。
                    // 名字在这里查好 —— 收屏那一侧只管画（与 `bails`/`magic` 同一条分工）
                    Ask::Shift => blame_candidates(
                        &books
                            .solvent()
                            .into_iter()
                            .map(|(other, _)| other)
                            .collect::<Vec<_>>(),
                        who,
                    )
                    .into_iter()
                    .map(|other| (other, seat(&names, other)))
                    .collect(),
                    Ask::Offset => Vec::new(),
                },
                // 「后面还排着几份」与拍卖台那句「后面还排着几块」同一条读数：答完这一问
                // 未必回到回合机，还有一份在等**下一个人**答（主语会跳动）
                queued: view.gate.queue.len().saturating_sub(1),
                // 回合主人是谁由轮转指针读，与窗口上那位比对 —— 「替别人答」那一句
                // 要有两个人才说得出（同拍卖台读 `lot.bidder()` 那一条分工）
                owner: view
                    .order
                    .current()
                    .map_or_else(|| "没人".to_string(), |who| seat(&names, who)),
            })
        }
        _ => None,
    };
    let stamp = want.map_or(String::new(), |(player, about)| {
        let mut stamp = modal_stamp(
            about,
            books,
            player,
            ModalExtra {
                stock: &view.stock,
                held: &held,
                plots: &plots,
                bails: &bails,
                lottery: lottery.as_ref(),
                bid: bid.as_ref(),
                service: service.as_ref(),
                company: company.as_ref(),
                magic: &magic_rows,
                draft: magic_draft.as_ref(),
                gate: gate_ask.as_ref(),
            },
        );
        // 卡面这一屏的内容不在 `Pending` 里，而在 `RecentCard` 那份记忆里（消息驱动的
        // 收据都是这个形状）。签名里带上卡 id，于是「记忆比模态晚一帧到」不会把兜底说明
        // 钉在屏幕上 —— 重建判据本来就有这一格，只是卡面以前是空的
        if about == Pending::Card
            && let Some(card) = view.memory.0.as_ref()
        {
            stamp.push_str(&card.id);
        }
        stamp
    });
    // 两边都是 `None` 时相等：没有待决事项、也没有模态，正是常态
    if view
        .open
        .iter()
        .next()
        .is_some_and(|(_, root)| Some(root.key) == want && root.stamp == stamp)
    {
        return;
    }
    // 一次只挂一个：换问题时先把旧的整块收走（`despawn` 递归，遮罩是子树根）
    for (stale, _) in view.open.iter() {
        commands.entity(stale).despawn();
    }
    if let Some((player, about)) = want {
        let stamp = stamp.clone();
        // 返回值一律丢掉：每个 builder 自己把 `ModalRoot` 挂上去（见 `_modal_root`），
        // 这里要的只是「建了哪一屏」这个副作用
        match about {
            Pending::Buy(land) => {
                buy_modal(&mut commands, &font, books, player, land, stamp);
            }
            Pending::Upgrade(land) => {
                upgrade_modal(&mut commands, &font, books, player, land, stamp);
            }
            Pending::RentNotice(land) => {
                rent_modal(&mut commands, &font, books, &names, player, land, stamp);
            }
            Pending::Card => {
                card_modal(&mut commands, &font, &server, &view.memory, player, stamp);
            }
            Pending::BankTeller => {
                bank_modal(&mut commands, &font, books, player, stamp, &plots, &fins);
            }
            Pending::Shop => {
                shop_modal(
                    &mut commands,
                    &font,
                    books,
                    &shop_rows(books, &view.shelf, &view.stock, player, &held),
                    player,
                    stamp,
                );
            }
            Pending::Bail(place) => {
                bail_modal(&mut commands, &font, books, player, place, &bails, stamp);
            }
            // 建设窗口（P10c）。候选与灰按钮的判据全在 `Books::build_probe` 里 ——
            // 面板不在自己这一侧另算一遍价（P9 那条「写的与扣的必须同一个表达式」）。
            // 建成之后窗口照开（`decision_system` 那一格的注）：这一屏会重建成
            // 「这里已经立着设施」的样子，玩家按「离开」才走
            Pending::Build(land) => {
                build_modal(&mut commands, &font, books, player, land, stamp);
            }
            // 远程建设窗口（P10d②）。与 `build_modal` 同一条分工：候选与灰按钮的判据
            // 全在 `economy::can_upgrade`（就是 `Books::upgrade` 认的那张表）；
            // 升完窗口照开，按「先不建」才收摊
            Pending::RemoteBuild => {
                remote_build_modal(&mut commands, &font, books, player, stamp);
            }
            // 服务费收据（P10c）。与租金收据同一族：钱已经扣完（或被吃成 0），
            // 这一屏只负责把「摇了几点、住了几天」说出来。读数在 `sync_modal` 那边
            // 就像 `bails` 一样先摊好（`ServiceRow`）—— 这一屏的五个参数已经是上限了
            Pending::ServiceNotice(land) => {
                if let Some(row) = service.as_ref() {
                    service_modal(&mut commands, &font, player, row, stamp);
                } else {
                    // 拿不到读数（注入挂的窗口、或消息还没到）：说得出「结算过了」，
                    // 与 `card_modal` 的 `fallback_note` 同一条退路，不 panic
                    fallback_note(
                        &mut commands,
                        &font,
                        (player, Pending::ServiceNotice(land)),
                        "服务费已结算",
                    );
                }
            }
            // 公司收据（P10d①）。与租金/服务费同一族：钱已经结完（董事长免单也是
            // 「结完了 0 元」），这一屏把「哪家公司、怎么算的、谁在当董事长」说出来
            Pending::CompanyNotice => {
                if let Some(row) = company.as_ref() {
                    company_modal(&mut commands, &font, player, row, stamp);
                } else {
                    // 拿不到读数（注入挂的窗口、或消息还没到）：说得出「结算过了」，
                    // 与 `service_modal` 那条退路同一条，不 panic
                    fallback_note(
                        &mut commands,
                        &font,
                        (player, Pending::CompanyNotice),
                        "公司到访已结算",
                    );
                }
            }
            // 彩票柜台（P8）。`lottery` 在这一屏必然是 `Some`（上面那个 match 只在
            // `Pending::Lottery` 开着时才填它），但这里不 unwrap 一个「编译器不知道」的
            // 事实：拿默认读数建一屏比整块不建更难查。
            Pending::Lottery => {
                if let Some(desk) = lottery.as_ref() {
                    lottery_modal(&mut commands, &font, books, desk, player, stamp);
                }
            }
            // 拍卖台（P9）。它与彩票柜台同一条形状：一个窗口、两颗动作按钮。
            // `bid` 在这一屏几乎必然是 `Some`（上面只在窗口开着且有 `lot` 时才填它），
            // 但散场与下一场之间那一帧真的会是「窗口还没被清、机器已经空了」——
            // 那种时候整块不重建，下一帧 `auction_hold_system` 自己会把窗口收走
            Pending::Auction => {
                if let Some(desk) = bid.as_ref() {
                    auction_modal(&mut commands, &font, &names, desk, player, stamp);
                }
            }
            // 魔法屋的两步（P13）：两个窗口两个 builder，因为**候选来源不同** ——
            // 第一步读 `MagicFilter::ALL` 加现筛的名单，第二步读的是规则层存进草稿的那一份。
            // 合成一屏的话，「答完条件之后名单变了」这件事就没有地方说
            // 被动卡那道门（P13b②）。这一屏与拍卖台同一条形状：**主语每帧都可能换人**，
            // 而拿不到读数那一帧（问句与窗口之间隔着一次投递）整块不重建，
            // 下一帧 `gate_system` 自己会把问句挂上来 —— 与 `fallback_note` 不同的是
            // 这里没有一句假话可说（「门在问人」而问的是什么读不出）
            Pending::Passive => {
                if let Some(desk) = gate_ask.as_ref() {
                    passive_modal(&mut commands, &font, &names, desk, player, stamp);
                }
            }
            Pending::Magic => {
                magic_filter_modal(&mut commands, &font, player, &magic_rows, stamp);
            }
            Pending::MagicSpell(filter) => {
                if let Some((f, who)) = magic_draft.as_ref() {
                    magic_spell_modal(&mut commands, &font, player, *f, who, stamp);
                } else {
                    // 草稿与窗口分叉了（规则层那句 `warn!` 的画面对照）：宁可说一句「名单没了」，
                    // 也不摆一排点下去落向未知名单的法术
                    fallback_note(
                        &mut commands,
                        &font,
                        (player, Pending::MagicSpell(filter)),
                        "这场法事的名单不在了",
                    );
                }
            }
            // 游乐场（P7）**不在这里建面板**：模态的内容是「弹出那一刻烤死」的，
            // 而那一屏上有气球在飞、秒数在跳，每帧都要动 —— 那是 `ui/minigame.rs` 的活，
            // 它读的同样是 `PendingDecision`。这一格存在的意义是「旧的模态要被收走」：
            // 从百货柜台那一格走到游乐场，柜台不能还压在屏幕上（上面那个 despawn 循环
            // 已经做完了），而这一屏由 `sync_arcade` 建、不由这里建
            Pending::Minigame(_) => {}
        }
    }
}

/// 这一屏「该写成什么样」的签名。只有两种**窗口**需要，报价与收据返回空串。
///
/// 写成对 `Pending` 的穷尽匹配：新增一种待决事项时必须当场回答「它的屏幕上有什么是由
/// 判据决定的」，漏了不会编译失败、只会静默过期。
/// 签名要读的「这一屏的附加内容」。收成一个结构是房规（§六 坑 8）：
/// 四位数据本来就是同一件事 —— 这一屏上由判据决定的可见内容。
struct ModalExtra<'a> {
    stock: &'a ShopStock,
    held: &'a [CardId],
    plots: &'a [PlotRow],
    bails: &'a [BailRow],
    /// 彩票柜台的读数（P8），奖池那笔账也在里面 —— 它是「这一屏上由判据决定的可见内容」
    /// 里唯一一块**不属于任何玩家**的账。`None` = 这一屏不是彩票柜台
    lottery: Option<&'a LotteryDesk>,
    /// 拍卖台的读数（P9）：这一场喊到哪、还在争谁、后面还排着几块。
    /// 「现在轮到谁」不在这里 —— 窗口挂在谁身上就是谁，由 `PendingDecision` 带
    bid: Option<&'a BidDesk>,
    /// 服务费收据的读数（P10c）。它读的是 `RecentService`（消息留下的那一笔），
    /// 所以**内容本身就是判据**：消息晚到一帧时这一屏要先重建成真读数，而不是停在
    /// 「服务费已结算」那句兜底话上
    service: Option<&'a ServiceRow>,
    /// 公司收据的读数（P10d①）。与 `service` 同一条理由：`RecentCompany` 是消息留下的，
    /// 内容本身就是判据
    company: Option<&'a CompanyRow>,
    /// 魔法屋第一步的候选（P13）：每条条件筛出谁 —— 「哪几颗该灰、行右写谁」都由它决定，
    /// 所以它是判据而不是文案。`Pending::Magic` 之外那一屏传空表
    magic: &'a [MagicRow],
    /// 门这一屏的读数（P13b②）。哪一问、由哪张卡答、手里几张、能点名给谁 ——
    /// 这四样每一样都决定屏幕上有一颗按钮，所以它们是判据而不是文案（坑 30 同一条）
    gate: Option<&'a GateAsk>,
    /// 魔法屋第二步的草稿（P13）：条件 + 已筛出的名单。名单变了这一屏就得重建 ——
    /// 否则玩家对着一份「还写着三个人」的名单点出一道只落到两个人头上的法术
    draft: Option<&'a (MagicFilter, Vec<String>)>,
}

/// 魔法屋第一步的一行（P13）：一条条件与它筛出的人名。
///
/// 名字在这里查好（与 `bails`/`service` 同一条分工）：收屏那一侧只管画，不回头开查询。
pub(crate) struct MagicRow {
    pub filter: MagicFilter,
    pub who: Vec<String>,
}

fn modal_stamp(about: Pending, books: &Books, player: Entity, extra: ModalExtra<'_>) -> String {
    match about {
        Pending::BankTeller => {
            let bank = books.bank_of(player);
            // 融资那一节（P10d②）也要重烤：挪一笔 = 他的债与别人的存款同时动，
            // 身份反超 = 整节出现/消失。少进一位就会出现「已经挪到手、按钮还亮着」
            let chair = game_core::company::chairman_of(books, game_core::company::FINANCE_STOCK)
                == Some(player);
            let deposits = if chair {
                books
                    .solvent()
                    .iter()
                    .filter(|(e, _)| *e != player)
                    .map(|(e, _)| books.bank_of(*e).deposit)
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            // 末段挪出来拼（外层 format! 里再嵌 format! 会被 clippy 打回）：
            // 「董事长在不在场 + 每位储户的存款」—— 反超收回靠逐帧对账，没有消息可等，
            // 面板能看到的只有这一帧的身份
            let fins = format!(
                "{}:{}",
                chair as u8,
                deposits
                    .iter()
                    .map(|d| d.to_string())
                    .collect::<Vec<_>>()
                    .join(",")
            );
            format!(
                "{}|{}|{}|{}|{}|{}|{}|{}",
                books.balance(player),
                bank.deposit,
                bank.loan,
                loan_room(books, player),
                // 地产那一节的每一行都是「由判据决定」的可见东西：押没押、封没封、
                // 赎回要多少。少进签名一位，玩家就会对着一个「已经押掉了、按钮还亮着」
                // 的柜台按第二次（§6.1 坑 22/30 同族）
                extra
                    .plots
                    .iter()
                    .map(|row| format!("{}:{}:{}", row.land.0, row.held as u8, row.sealed as u8))
                    .collect::<Vec<_>>()
                    .join(","),
                bank.financing,
                fins,
                // 停摆（P11c②）也必须进签名：柜台是一个窗口而不是一个报价，新闻在开着
                // 的柜台之后播出来是常事 —— 少这一位，那两颗「借」会一直绿到玩家按下去
                // 才发现借不动（截图就是这么翻出来的）
                books.session.suspensions.days_left(Institution::Bank),
            )
        }
        // 百货柜台与银行同一条理由：它是一个窗口，每一笔买卖都要重烤三位 ——
        // 点券余额、货架还剩哪几格、手里有哪几张。少任何一位，玩家就会对着一个
        // 「已经买掉了、按钮还亮着」的面板按第二次。
        Pending::Shop => format!(
            "{}|{}|{}",
            books.points_of(player),
            extra
                .stock
                .slots
                .iter()
                .map(|c| c.0.to_string())
                .collect::<Vec<_>>()
                .join(","),
            extra
                .held
                .iter()
                .map(|c| c.0.to_string())
                .collect::<Vec<_>>()
                .join(","),
        ),
        // 保释窗口同一条：点券、还关着谁、谁刚被放走 —— 少一位就是一次静默过期
        Pending::Bail(_) => format!(
            "{}|{}",
            books.points_of(player),
            extra
                .bails
                .iter()
                .map(|row| row.target.to_bits().to_string())
                .collect::<Vec<_>>()
                .join(",")
        ),
        // 彩票柜台同一条：现金（买不买得起）、已有哪几个号（哪几颗该带 ✓）、别人占走的
        // 哪几个号（哪几颗该灰、名字写在下面那一行）、奖池（那句「奖池 N」的读数）。
        // 少一位就是一次静默过期 —— 买完一注之后号还亮着，玩家会以为那张券没买成，
        // 再按一次才是真买成了
        Pending::Lottery => {
            let desk = extra.lottery;
            format!(
                "{}|{}|{}|{}",
                desk.map(|d| d.cash).unwrap_or(0),
                desk.map(|d| d
                    .held
                    .iter()
                    .map(|n| n.to_string())
                    .collect::<Vec<_>>()
                    .join(","))
                    .unwrap_or_default(),
                desk.map(|d| d
                    .taken
                    .iter()
                    .map(|(n, _)| n.to_string())
                    .collect::<Vec<_>>()
                    .join(","))
                    .unwrap_or_default(),
                desk.map(|d| d.pot).unwrap_or(0),
            )
        }
        // 拍卖台同一条：拍的是哪块地、喊到哪、名单上还剩谁、后面还排着几块、被问的这位
        // 有多少钱。少任何一位都会出现「已经加过价，面板还停在上一档」——
        // 而那一档正是让人按下第二次的形状
        Pending::Auction => extra.bid.map_or(String::new(), |desk| {
            format!(
                "{}|{}|{}|{}|{}|{}|{}",
                desk.lot.land.0,
                desk.lot.high,
                desk.lot.turn,
                desk.lot.leader.map_or(0u64, |e| e.to_bits()),
                desk.lot
                    .pool
                    .iter()
                    .map(|e| e.to_bits().to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                desk.queued,
                desk.cash,
            )
        }),
        // 魔法屋第一步（P13）：签名是**每条条件筛出谁**。名单一动（有人破产、被新闻搬走、
        // 换了载具）这一屏就得重建 —— 少这一位，玩家会对着一个「还亮着却已经筛不出人」的
        // 按钮点下去，而规则层当场落空（`magic_filter_system` 那条空名单直接收摊的分支）
        Pending::Magic => extra
            .magic
            .iter()
            .map(|row| format!("{}:{}", row.filter.label(), row.who.join("+")))
            .collect::<Vec<_>>()
            .join(","),
        // 门（P13b②）：问序两问的内容、手里的张数、可点名的名单、后面还排着几份 ——
        // 四样都会变，而**主语每一帧都可能换人**（同拍卖台那条：A 转给 B 之后是 B 在答）。
        // 少任何一位都会出现「那张卡已经花掉了、按钮还亮着」或「名单上写着三个人、
        // 点下去落到两个人头上」
        Pending::Passive => extra.gate.map_or(String::new(), |desk| {
            format!(
                "{}|{}|{}|{}|{}|{}",
                desk.ask as u8,
                desk.event as u8,
                desk.card.map_or(0u8, |c| c as u8),
                desk.held,
                desk.blame
                    .iter()
                    .map(|(e, _)| e.to_bits().to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                desk.queued,
            )
        }),
        // 第二步（P13）：条件 + 草稿名单。条件是窗口自己带的（换条件＝换一场法事），
        // 名单是资源里的那一份 —— 两位都进签名，因为面板上那行字就是这两样
        Pending::MagicSpell(_) => extra.draft.map_or(String::new(), |(f, who)| {
            format!("{}|{}", f.label(), who.join("+"))
        }),
        // 游乐场那一屏不由 `sync_modal` 建（见下面那个空分支），所以这里没有要签名的东西：
        // 它的可见内容每帧都在动，由 `ui/minigame.rs` 自己就地改，不走「签名变了才重建」那条路
        Pending::Minigame(_) => String::new(),
        // 报价与收据的内容弹出即定稿（买地面板上的「你的现金」是那一刻的现金，
        // 而回合停在那几帧里谁也花不了钱）—— **但买地与升级两屏例外，签名里带地块的层数**。
        // 理由与它们面板上那些数同源：10-22 之后无主地可以立着楼（破产留下的那一手），
        // 于是「买价」「当前等级／升级后／两行租金」都不再由地块数据单独决定，还由世界上
        // 此刻的 `BuildingLevel` 决定；而那一格在报价挂出之后仍可能被改写（天使盖的那一层、
        // `NM_PLOTS`／`NM_HOUSE` 这类注入晚到一帧 —— 后者实测就拍出过一张「空地 → 空地」的升级）。
        // 不带这一格就会出现「面板写着 3,000、收据写着 6,000」，而那正是这两块屏的反面
        Pending::Buy(land) | Pending::Upgrade(land) => books
            .view(land)
            .map_or(String::new(), |view| format!("Lv{}", view.level)),
        Pending::RentNotice(_) | Pending::Card => String::new(),
        // 服务费收据的「签名」是它自己那份读数（P10c）：这笔钱已经结完、内容不会再变，
        // 唯一的变动来源是**消息晚到**（`remember_service` 与这一屏在两个 system 里）。
        // 不带这一格，晚到的那一帧会建出一屏「服务费已结算」，而下一帧签名不变、
        // 不复建 —— 一张说得出金额的收据永远上不了屏（本阶段做 `NM_SERVICE` 时踩到）
        Pending::ServiceNotice(_) => extra.service.map_or_else(
            || "空".to_string(),
            |row| {
                format!(
                    "{}|{}|{}|{}",
                    row.amount, row.roll, row.stay_days, row.kind as u8
                )
            },
        ),
        // 公司收据的签名与租金/服务费同一条（P10d①）：内容弹出即定稿，唯一变动是消息晚到。
        // 不带这一格，晚到的那一帧会建出一屏兜底话、下一帧签名不变不复建 ——
        // `NM_SERVICE` 当年踩过的坑在这里重演一遍
        Pending::CompanyNotice => extra.company.map_or_else(
            || "空".to_string(),
            |row| {
                format!(
                    "{}|{}|{}|{}",
                    row.stock, row.amount, row.roll, row.stay_days
                )
            },
        ),
        // 建设窗口是本仓库第二种「同一块地、数字会动」的报价：建完之后这一屏要翻成
        // 「这里已经立着设施」，而现金也会少一截 —— 少一位就会出现「已经建好了、
        // 按钮还亮着」那种让人按第二次的面板。层数与 `Buy`/`Upgrade` 同一条理由带上
        Pending::Build(land) => books.view(land).map_or(String::new(), |view| {
            format!(
                "{}|{}|{}",
                books.balance(player),
                view.level,
                // `facility_of` 那一格是「建没建成」的唯一读数。用 `bool` 而不是名字：
                // 这一屏重建要看的是「空 → 有」这一次翻转，建的是哪一类不影响判据
                books.facility_of(land).is_some() as u8,
            )
        }),
        // 远程建设窗口（P10d②）：升一块地 = 层数与现金同时动，与 `Build` 同一条理由。
        // 签名带**全部**地块的层数 —— 候选列表本来就画着它们，谁变了这一屏都得重烤
        Pending::RemoteBuild => format!(
            "{}|{}",
            books.balance(player),
            books
                .views()
                .iter()
                .map(|v| format!("{}:{}", v.id.0, v.level))
                .collect::<Vec<_>>()
                .join(",")
        ),
    }
}

/// 名下未抵押地块的可贷上限（缺陷 #1 修的就是这个数在 Godot 里恒为 0）。
fn loan_room(books: &Books, player: Entity) -> i64 {
    let Some(pid) = books.id_of(player) else {
        return 0;
    };
    game_core::economy::calc_max_loan(&books.active.def, &books.views(), pid, &books.rules)
}

/// 遮罩 + 居中面板的外壳。遮罩节点是整棵子树的根，`despawn` 它即收走全部内容；
/// `key` 是这一屏在回答哪个待决事项，[`sync_modal`] 靠它判断要不要重建。
fn frame(
    commands: &mut Commands,
    key: (Entity, Pending),
    stamp: String,
    spec: [f32; 3],
    accent: Color,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    let (overlay_node, overlay_bg, overlay_z) = overlay(3);
    let (panel_node, panel_transform, panel_bg, panel_border) = panel(spec[0], spec, accent);
    commands
        .spawn((
            BoardUi,
            ModalRoot { key, stamp },
            overlay_node,
            overlay_bg,
            overlay_z,
        ))
        .with_children(|mask| {
            mask.spawn((panel_node, panel_transform, panel_bg, panel_border))
                .with_children(|p| body(p));
        })
        .id()
}

/// 面板里居中的那一行（标题、副标题、现金）。
///
/// 节点仍然拉满面板宽度，靠 `Justify::Center` 让文字在行内居中 —— 与 Godot 的
/// `Label` + `HORIZONTAL_ALIGNMENT_CENTER` 同形。改成父级 `align_items: Center` 会
/// 连带把信息行也居中，而 Godot 的信息行是左对齐的。
fn center_label(
    font: &Handle<Font>,
    text: impl Into<String>,
    px: f32,
    color: Color,
) -> (Node, TextLayout, Text, TextFont, TextColor) {
    let (t, f, c) = label(font, text, px, color);
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

/// 一行按钮（Godot 的 `btn_box`：居中、间距 20）。
fn button_row(parent: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(layout::BTN_ROW_GAP),
            ..default()
        })
        .with_children(|line| f(line));
}

/// 无主地的报价那一屏。
///
/// **面板上那个数就是 `Books::buy` 要扣的那一个**：基数取 `economy::estate_value`
/// （地价 + 建筑费 × 花钱盖起来的那几层），再过一遍物价与修正，与 `buy_price`／`market_value`
/// 同一条式子。只写名义地价在这里曾经是安全的（无主地就是 Lv0），而 10-22 之后「无主却立着楼」
/// 是真状态 —— 破产留下的那一手、天使白给的那一层。那时按地价写就成了「写着 1,500、扣掉 3,100」，
/// 而这一块屏是唯一让玩家决定掏不掏这笔钱的地方。
/// 「地价」那一行照旧留着（它是这块地的固有属性，租金表按它排），多出来的那一截由
/// 「楼上已有」与「买价」两行自己说清楚 —— 灰按钮带理由那条纪律在这儿的形式是**价带理由**。
fn buy_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let def = books.active.def.land(land.0);
    let name = def.map(|l| l.name.clone()).unwrap_or_default();
    let group = def.and_then(|l| l.group.clone()).unwrap_or_default();
    let accent = if group.is_empty() {
        col::TITLE_GOLD
    } else {
        palette::group(&group)
    };
    let nominal = def.map(|l| l.price).unwrap_or(0);
    let view = books.view(land);
    let storeys = view.as_ref().map_or(0, |v| v.level);
    let base = view.as_ref().map_or(0, |v| {
        game_core::economy::estate_value(&books.active.def, v)
    });
    let price = books.quoted(ChargeKind::BuyLand { land }, player, base);
    let rent: Vec<i64> = def.map(|l| l.rent.clone()).unwrap_or_default();
    let type_text = match def.map(|l| l.land_type) {
        Some(LandType::Commercial) => "商业用地",
        Some(LandType::Insurance) => "保险用地",
        _ => "住宅用地",
    };
    let cash = books.balance(player);
    let affordable = cash >= price;
    let buy_text = if affordable {
        format!("购买 (${})", comma(price))
    } else {
        "现金不足".to_string()
    };
    let cash_text = format!("你的现金: ${cash}");

    frame(
        commands,
        (player, Pending::Buy(land)),
        stamp,
        layout::BUY_PANEL,
        col::BORDER_BUY,
        |p| {
            p.spawn(center_label(&font.0, name, size::BUY_TITLE, accent));
            p.spawn(center_label(
                &font.0,
                "是否购买此地？",
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            // 信息行组套一个竖排容器（Godot 的 `InfoPanel` VBox，行距 4）
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::INFO_ROW_GAP),
                ..default()
            })
            .with_children(|rows| {
                let key_w = layout::INFO_KEY_W;
                if !group.is_empty() {
                    info_row(rows, &font.0, "颜色组", &group, accent, key_w);
                }
                info_row(
                    rows,
                    &font.0,
                    "地价",
                    &format!("${}", comma(nominal)),
                    col::COST,
                    key_w,
                );
                // 地上已经有楼：那几层不是赠品，而是买价里最贵的一截，必须自己占一行说话。
                // 等级念的是 `building_label` 那一套（内部 Lv3 ＝「建筑 Lv2」），
                // 而这一行恰好是那条映射最有用的地方 —— 「楼上已有 Lv3」会被读成盖了三层
                if storeys >= 2 {
                    info_row(
                        rows,
                        &font.0,
                        "楼上已有",
                        &format!("{}，连地一起过户", building_label(storeys)),
                        col::GAIN,
                        key_w,
                    );
                    info_row(
                        rows,
                        &font.0,
                        "买价",
                        &format!("${}", comma(price)),
                        col::COST,
                        key_w,
                    );
                }
                info_row(rows, &font.0, "类型", type_text, col::VALUE, key_w);
                if !rent.is_empty() {
                    rows.spawn(label(&font.0, "租金表", size::SMALL, col::KEY));
                    for (i, amount) in rent.iter().enumerate() {
                        info_row(
                            rows,
                            &font.0,
                            &building_label((i + 1) as u8),
                            &format!("${amount}"),
                            col::VALUE,
                            key_w,
                        );
                    }
                }
            });
            p.spawn(separator());
            p.spawn(center_label(
                &font.0,
                cash_text,
                size::NAME,
                if affordable {
                    col::CASH_OK
                } else {
                    col::CASH_BAD
                },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        &buy_text,
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Accept),
                    )
                    .disabled(!affordable),
                );
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "取消",
                        layout::MODAL_BUTTON,
                        SECONDARY,
                        UiAction::Decide(Decision::Decline),
                    ),
                );
            });
        },
    )
}

fn upgrade_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let Some(view) = books.view(land) else {
        return fallback_note(commands, font, (player, Pending::Upgrade(land)), "升级报价");
    };
    let def = books.active.def.land(land.0);
    let name = def.map(|l| l.name.clone()).unwrap_or_default();
    let accent = def
        .and_then(|l| l.group.as_deref())
        .map(palette::group)
        .unwrap_or(col::TITLE_GOLD);
    let cost = game_core::economy::upgrade_cost(&books.active.def, &view).unwrap_or(0);
    let rent: Vec<i64> = def.map(|l| l.rent.clone()).unwrap_or_default();
    let level = view.level as usize;
    let current_rent = rent.get(level.saturating_sub(1)).copied().unwrap_or(0);
    let new_rent = rent.get(level).copied().unwrap_or(0);
    let cash = books.balance(player);
    let affordable = cash >= cost;
    let title = format!("升级 {name}");
    let buy_text = if affordable {
        format!("升级 (${cost})")
    } else {
        "现金不足".to_string()
    };
    let cash_text = format!("你的现金: ${cash}");

    frame(
        commands,
        (player, Pending::Upgrade(land)),
        stamp,
        layout::UPGRADE_PANEL,
        col::BORDER_UPGRADE,
        |p| {
            p.spawn(center_label(&font.0, title, size::DIALOG_TITLE, accent));
            p.spawn(center_label(
                &font.0,
                "是否升级此地？",
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            // 信息行组套一个竖排容器（Godot 的 `InfoPanel` VBox，行距 4）
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::INFO_ROW_GAP),
                ..default()
            })
            .with_children(|rows| {
                let key_w = layout::INFO_KEY_W_WIDE;
                info_row(
                    rows,
                    &font.0,
                    "当前等级",
                    &building_label(level as u8),
                    col::VALUE,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "升级后",
                    &building_label(level as u8 + 1),
                    col::GAIN,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "升级费用",
                    &format!("${cost}"),
                    col::COST,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "当前租金",
                    &format!("${current_rent}"),
                    col::VALUE,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "升级后租金",
                    &format!("${new_rent}"),
                    col::GAIN,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "租金增幅",
                    &format!("+${}", new_rent - current_rent),
                    col::GAIN,
                    key_w,
                );
            });
            p.spawn(separator());
            p.spawn(center_label(
                &font.0,
                cash_text,
                size::NAME,
                if affordable {
                    col::CASH_OK
                } else {
                    col::CASH_BAD
                },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        &buy_text,
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Accept),
                    )
                    .disabled(!affordable),
                );
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "取消",
                        layout::MODAL_BUTTON,
                        SECONDARY,
                        UiAction::Decide(Decision::Decline),
                    ),
                );
            });
        },
    )
}

fn rent_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    names: &Query<&PlayerName>,
    payer: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let def = books.active.def.land(land.0);
    let land_name = def.map(|l| l.name.clone()).unwrap_or_default();
    let amount = books.rent_of(land);
    let owner = books
        .view(land)
        .and_then(|v| v.owner)
        .and_then(|id| books.holder(id))
        .and_then(|e| names.get(e).ok())
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "无主".to_string());
    let payer_name = names.get(payer).map(|n| n.0.clone()).unwrap_or_default();
    // 「先扣钱后通知」是规则层的既有顺序（`property.rs:62-82`），所以破产在这里
    // 已经是既成事实，收据只是把它说出来 —— 与 Godot 的 `went_bankrupt` 同一个语义
    let bankrupt = books.is_bankrupt(payer);
    let title = if bankrupt {
        "破产！"
    } else {
        "支付租金"
    };
    let title_color = if bankrupt {
        col::BANKRUPT
    } else {
        col::TITLE_GOLD
    };
    let desc = if bankrupt {
        format!(
            "{payer_name} 踩到 {owner} 的地（{land_name}）\n租金 ${amount} 无法支付，宣告破产！"
        )
    } else {
        format!("{payer_name} 踩到 {owner} 的地（{land_name}）")
    };
    let ok_text = if bankrupt { "确认破产" } else { "确认" };

    frame(
        commands,
        (payer, Pending::RentNotice(land)),
        stamp,
        layout::RENT_PANEL,
        col::BORDER_RENT,
        |p| {
            p.spawn(center_label(
                &font.0,
                title,
                size::DIALOG_TITLE,
                title_color,
            ));
            p.spawn(center_label(&font.0, desc, size::NAME, col::BODY));
            p.spawn(center_label(
                &font.0,
                format!("- ${amount}"),
                size::RENT_AMOUNT,
                if bankrupt { col::BANKRUPT } else { col::COST },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        ok_text,
                        layout::OK_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

fn card_modal(
    commands: &mut Commands,
    font: &DevFont,
    server: &AssetServer,
    memory: &RecentCard,
    player: Entity,
    stamp: String,
) -> Entity {
    let Some(card) = memory.0.as_ref() else {
        return fallback_note(commands, font, (player, Pending::Card), "卡牌效果已结算");
    };
    let (type_text, accent) = match card.kind {
        CardKind::Chance => ("机会", col::CHANCE),
        CardKind::Fate => ("命运", col::FATE),
        // 道具卡不该出现在这个模态里：它是玩家主动打出的，反馈走手牌浮层与提示行。
        // 这一支只为穷尽存在 —— 写成 `_ => unreachable!()` 会让「哪天有人把它挂成
        // `Pending::Card`」变成一次对局中途的 panic，而这里只欠一个名字和一个颜色
        CardKind::Item => ("道具", col::CHANCE),
    };
    let effect = outcome_line(&card.outcomes);
    let face = art::card_face(&card.id).map(|path| server.load(path));

    frame(
        commands,
        (player, Pending::Card),
        stamp,
        if face.is_some() {
            layout::CARD_PANEL_ART
        } else {
            layout::CARD_PANEL
        },
        col::BORDER_CARD,
        |p| {
            if let Some(face) = face {
                // 卡面在左、文案在右。不叠在图上是因为面板底色是半透明的
                // （`col::PANEL`），字压在画上两头都读不清
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(layout::CARD_ART_GAP),
                    align_items: AlignItems::FlexStart,
                    ..default()
                })
                .with_children(|line| {
                    line.spawn((
                        ImageNode {
                            image: face,
                            image_mode: NodeImageMode::Stretch,
                            ..default()
                        },
                        Node {
                            width: Val::Px(layout::CARD_ART[0]),
                            height: Val::Px(layout::CARD_ART[1]),
                            ..default()
                        },
                    ));
                    line.spawn(Node {
                        flex_direction: FlexDirection::Column,
                        flex_grow: 1.0,
                        row_gap: Val::Px(10.0),
                        ..default()
                    })
                    .with_children(|words| {
                        card_words(words, font, type_text, accent, card, &effect)
                    });
                });
            } else {
                card_words(p, font, type_text, accent, card, &effect);
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec {
                        text: "确定",
                        size: layout::CARD_OK_BUTTON,
                        font: size::CARD_OK,
                        palette: super::PRIMARY,
                        action: UiAction::Decide(Decision::Acknowledge),
                        disabled: false,
                    },
                );
            });
        },
    )
}

/// 卡面模态的文字那一叠。有图与没图两条路共用它 —— 差别只在它是面板的直接子节点，
/// 还是横排里图右边的那一列。
fn card_words(
    p: &mut ChildSpawnerCommands,
    font: &DevFont,
    type_text: &str,
    accent: Color,
    card: &CardNotice,
    effect: &str,
) {
    p.spawn(center_label(
        &font.0,
        format!("【{type_text}】"),
        size::BODY,
        accent,
    ));
    p.spawn(center_label(
        &font.0,
        card.title.clone(),
        size::CARD_TITLE,
        accent,
    ));
    p.spawn(separator());
    if !card.desc.is_empty() {
        p.spawn(label(&font.0, card.desc.clone(), size::NAME, col::BODY));
    }
    if !effect.is_empty() {
        p.spawn(label(&font.0, effect.to_string(), size::BODY, col::RESULT));
    }
}

/// 一张兜底说明。`Pending` 与消息对不上时（测试里没抽过牌却被挂着、土地数据缺行）
/// 也要有个能点掉的东西 —— 模态挡着掷骰，弹不出内容就会把这一局卡死。
fn fallback_note(
    commands: &mut Commands,
    font: &DevFont,
    key: (Entity, Pending),
    text: &str,
) -> Entity {
    let text = text.to_string();
    frame(
        commands,
        key,
        String::new(),
        layout::RENT_PANEL,
        col::BORDER_CARD,
        |p| {
            p.spawn(label(&font.0, text, size::DIALOG_TITLE, col::BODY));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "确定",
                        layout::OK_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 银行柜台：**踩到**银行格才开（路过只给 ATM 卡，见 [`super::finance`]）。
///
/// 与其余四个模态最大的分别是它不是一次报价，而是一个窗口：玩家可以连办四笔，
/// 每笔之后数字都要重算 —— 所以 [`sync_modal`] 的重建判据里带了金额签名这一半。
/// 「办完」才把回合交还给结束流程；中途按别的（`Accept`/`Decline`）规则层一律不认。
///
/// 窗口判据在这里只剩金额一条：柜台既然开着，四种操作的主语与地点都已经成立，
/// 剩下「这笔金额办不办得动」。**银行全年无休**（P2b）—— 周末与节假日走进柜台照办，
/// 日历只管股市。
fn bank_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    stamp: String,
    plots: &[PlotRow],
    fins: &[(Entity, String, i64)],
) -> Entity {
    let cash = books.balance(player);
    let bank = books.bank_of(player);
    let room = loan_room(books, player);
    // 还能借多少 = 上限 - 现有负债（`economy::loan_plan` 判的就是这个累计口径）
    let headroom = (room - bank.loan).max(0);
    // 还清要一次掏够：现金不够就灰着，别让「还 3000」在只有 2000 时静默失败
    let clear_debt = bank.loan.min(cash);
    let rules = *books.rules;
    // 新闻「银行停止放款」（P11c②）：只挡借这一颗，存取与还款照旧亮着 ——
    // 「还可借」那一行因此不是报一个借不到的数，而是说出为什么借不到
    let halted = books.session.suspensions.days_left(Institution::Bank);
    let lending_open = halted == 0;

    frame(
        commands,
        (player, Pending::BankTeller),
        stamp,
        layout::BANK_PANEL,
        col::BORDER_BANK,
        |p| {
            p.spawn(center_label(
                &font.0,
                "银行柜台",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                "存取款、借贷都在这里办，办完才继续",
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::INFO_ROW_GAP),
                ..default()
            })
            .with_children(|rows| {
                let key_w = layout::INFO_KEY_W_WIDE;
                info_row(rows, &font.0, "现金", &comma(cash), col::CASH_OK, key_w);
                info_row(
                    rows,
                    &font.0,
                    "存款余额",
                    &comma(bank.deposit),
                    col::VALUE,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "现有负债",
                    &comma(bank.loan),
                    col::COST,
                    key_w,
                );
                // 融资债（P10d②）：与贷款分家 —— 不占额度、不计息，挪用与兜底都记它
                info_row(
                    rows,
                    &font.0,
                    "融资债",
                    &comma(bank.financing),
                    if bank.financing > 0 {
                        col::COST
                    } else {
                        col::KEY
                    },
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "还可借",
                    // 停摆时这一行不报那个借不到的数，报的是「为什么借不到、哪天再来」：
                    // 一个 0 会被读成「你没地产可押」，那是另一件完全不同的事
                    &if lending_open {
                        comma(headroom)
                    } else {
                        format!("停止放款（剩 {halted} 天）")
                    },
                    if lending_open && headroom > 0 {
                        col::GAIN
                    } else {
                        col::BANKRUPT
                    },
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "利率",
                    &format!(
                        "存 {:.1}% / 贷 {:.1}%，每月末各结一次",
                        rules.deposit_rate * 100.0,
                        rules.loan_rate * 100.0
                    ),
                    col::KEY,
                    key_w,
                );
            });
            p.spawn(separator());
            money_row(
                p,
                font,
                BankOp::Deposit,
                true,
                ("存", 1_000, cash >= 1_000),
                ("存全部", cash, cash > 0),
            );
            money_row(
                p,
                font,
                BankOp::Withdraw,
                true,
                ("取", 1_000, bank.deposit >= 1_000),
                ("取全部", bank.deposit, bank.deposit > 0),
            );
            money_row(
                p,
                font,
                BankOp::Borrow,
                // 四颗里只有这一颗问停摆（其余三颗传 `true`，柜台既然开着就办得动）
                lending_open,
                ("借", 5_000, headroom >= 5_000),
                ("借到上限", headroom, headroom > 0),
            );
            money_row(
                p,
                font,
                BankOp::Repay,
                true,
                ("还", 5_000, bank.loan >= 5_000 && cash >= 5_000),
                ("还清", clear_debt, bank.loan > 0 && clear_debt >= bank.loan),
            );
            // 融资（P10d②）：只有银行保险的董事长看得见这一节。它不是借 ——
            // 不占额度、不计息：钱从别人的存款划进你的现金，债记在 `financing` 上。
            // 整节只在有候选时出现 —— 空着的一节读起来像坏了
            if !fins.is_empty() {
                p.spawn(separator());
                p.spawn(label(
                    &font.0,
                    "融资（挪用储户存款，不计息）",
                    size::SMALL,
                    col::KEY,
                ));
                for (of, name, deposit) in fins {
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
                                format!("从 {name}（存 {deposit}）"),
                                size::ROW,
                                col::BODY,
                            ),
                        ));
                        button(
                            line,
                            &font.0,
                            ButtonSpec::new(
                                "挪 1,000",
                                layout::SHOP_BUTTON,
                                super::PRIMARY,
                                UiAction::Financing {
                                    of_player: *of,
                                    amount: 1_000.min(*deposit),
                                },
                            )
                            .disabled(*deposit < 1),
                        );
                        button(
                            line,
                            &font.0,
                            ButtonSpec::new(
                                "挪全部",
                                layout::SHOP_BUTTON,
                                SECONDARY,
                                UiAction::Financing {
                                    of_player: *of,
                                    amount: *deposit,
                                },
                            )
                            .disabled(*deposit < 1),
                        );
                    });
                }
            }
            p.spawn(separator());
            p.spawn(label(
                &font.0,
                "你的地产（押出去不计息，赎回来加一成）",
                size::SMALL,
                col::KEY,
            ));
            if plots.is_empty() {
                // 一句话而不是一片空白：这块面板的高度是固定的，空着读起来像坏了
                p.spawn(label(&font.0, "名下没有地块", size::ROW, col::KEY));
            }
            for row in plots {
                plot_row(p, font, row);
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "办完",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 远程建设窗口（P10d②）：踩到日产建设格弹的那一块。与 [`build_modal`] 同一条分工 ——
/// 候选与灰按钮的判据全在规则层（[`game_core::economy::can_upgrade`] 就是
/// `Books::upgrade` 认的那一张表），面板不自己另算一遍「能不能升」；升完窗口照开，
/// 一趟可以连升几块地，「先不建」才是收摊。
fn remote_build_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    stamp: String,
) -> Entity {
    let Some(pid) = books.id_of(player) else {
        // 没有身份的人开不出这扇窗 —— 与柜台同一条守门
        return fallback_note(commands, font, (player, Pending::RemoteBuild), "远程建设");
    };
    let free =
        game_core::company::chairman_of(books, game_core::company::BUILD_STOCK) == Some(player);
    let def = &books.active.def;
    let views = books.views();
    let cash = books.balance(player);
    frame(
        commands,
        (player, Pending::RemoteBuild),
        stamp,
        layout::SHOP_PANEL,
        col::BORDER_SHOP,
        |p| {
            p.spawn(center_label(
                &font.0,
                "日产建设 — 远程升级",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                if free {
                    "董事长特权：升级自己的地免费一次升两层"
                } else {
                    "只能升级名下的地块，照价付费"
                },
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            for view in &views {
                let land = view.id;
                let cost = game_core::economy::upgrade_cost(def, view).unwrap_or(i64::MAX);
                let name = def.land(land.0).map_or("？", |l| l.name.as_str());
                let deny: Option<&'static str> = if view.owner != Some(pid) {
                    Some(if view.owner.is_none() {
                        "无主的地"
                    } else {
                        "不是你的地"
                    })
                } else if view.mortgaged {
                    Some("抵押中的地")
                } else if view.level >= books.rules.max_building_level {
                    Some("已经封顶")
                } else if view.status == Some(LandStatus::Sealed) {
                    Some("查封中的地")
                } else if !game_core::economy::can_upgrade(
                    view,
                    pid,
                    &views,
                    cost,
                    cash,
                    &books.rules,
                ) {
                    // 剩下的拒绝只可能是组内那两条与钱 —— 归成一句人话
                    Some("组内还有别的地没齐，或带的钱不够")
                } else {
                    None
                };
                let text = format!("{}  {}", name, building_label(view.level));
                let buy = if free {
                    "免费升两层".to_string()
                } else {
                    format!("升级 ${}", comma(cost))
                };
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    ..default()
                })
                .with_children(|line| {
                    let (label_text, ink) = if deny.is_some() {
                        (text, col::KEY)
                    } else {
                        (text, col::BODY)
                    };
                    line.spawn((
                        Node {
                            flex_grow: 1.0,
                            ..default()
                        },
                        super::label(&font.0, label_text, size::ROW, ink),
                    ));
                    button(
                        line,
                        &font.0,
                        ButtonSpec::new(
                            &buy,
                            layout::SHOP_BUTTON,
                            super::PRIMARY,
                            UiAction::RemoteBuild { land },
                        )
                        .disabled(deny.is_some()),
                    );
                    if let Some(reason) = deny {
                        line.spawn(super::label(&font.0, reason, size::ROW, col::JAILED));
                    }
                });
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "先不建",
                        layout::MODAL_BUTTON,
                        SECONDARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 柜台「你的地产」一行的全部读数（形状同 [`ShopRow`]，理由也同一条：`plot_row` 本来要
/// 七八份参数，而其中五份本来就是「这一行长什么样」这一件事）。
pub(crate) struct PlotRow {
    pub land: LandId,
    pub op: MortgageOp,
    /// 地块名 + 等级（按钮旁边那句）
    pub title: String,
    /// 按钮上的字**带数额**：`押 750` / `赎 825`
    pub button: String,
    /// 灰按钮自带一句为什么，取自规则层那张表（同 [`ShopRow::blocked`]）
    pub blocked: Option<&'static str>,
    /// 下面两位不上屏，只进 [`modal_stamp`]：它们变了这一行就得重烤
    pub held: bool,
    pub sealed: bool,
}

/// 把「这个人名下的地块」翻成一排按钮。
///
/// 判据一律走规则层那张 [`mortgage_op_allowed`]，而**读数走 [`read_facts`]** —— 面板不许
/// 自己发明一套「能不能办」，也不许自己乘一遍 `mortgage_percent`（那样一来规则改一次、
/// 面板就悄悄过期一次）。`at_counter` 在这里恒为 `true`：柜台既然开着，`NoCounter`
/// 那一格在这一屏永远碰不到，而它在派发那一侧是真的会挡（见 `counter_of`）。
fn plot_rows(books: &Books, player: Entity) -> Vec<PlotRow> {
    let def = &books.active.def;
    counter_plots(books, player)
        .into_iter()
        .map(|(land, view)| {
            let op = if view.mortgaged {
                MortgageOp::Redeem
            } else {
                MortgageOp::Take
            };
            let (verb, money) = match op {
                MortgageOp::Take => (
                    "押",
                    game_core::economy::mortgage_cashout(def, &view, &books.rules),
                ),
                MortgageOp::Redeem => ("赎", books.redeem_cost_of(land).unwrap_or(0)),
            };
            let facts = read_facts(books, player, land, true);
            PlotRow {
                land,
                op,
                title: format!(
                    "{}  {}",
                    def.land(land.0).map_or("？", |l| l.name.as_str()),
                    building_label(view.level)
                ),
                button: format!("{verb} {money}"),
                blocked: mortgage_op_allowed(op, &facts)
                    .err()
                    .map(MortgageDeny::label),
                held: view.mortgaged,
                sealed: matches!(view.status, Some(LandStatus::Sealed)),
            }
        })
        .collect()
}

/// 柜台地产一行：「地块名 + 一颗带数额的按钮 + 灰掉时的那句话」。形状照 [`shop_row`]。
fn plot_row(p: &mut ChildSpawnerCommands, font: &DevFont, row: &PlotRow) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        row_gap: Val::Px(layout::SHOP_ROW_GAP),
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
                &row.title,
                size::ROW,
                if row.blocked.is_some() {
                    col::KEY
                } else {
                    col::BODY
                },
            ),
        ));
        button(
            line,
            &font.0,
            ButtonSpec::new(
                &row.button,
                layout::SHOP_BUTTON,
                super::PRIMARY,
                UiAction::MortgageOp {
                    land: row.land,
                    op: row.op,
                },
            )
            .disabled(row.blocked.is_some()),
        );
        if let Some(reason) = row.blocked {
            line.spawn(label(&font.0, reason, size::ROW, col::JAILED));
        }
    });
}

/// 保释窗口的一行：关在这里的一个人（或一个恶人），和「他现在能不能被赎出去」。
#[derive(Debug, Clone)]
pub(crate) struct BailRow {
    pub target: Entity,
    pub name: String,
    /// 这一行的价目（P12）。**一行一个数而不是一个窗口一个数**：同一处牢房里可以同时
    /// 关着服刑的玩家（30）与等保释的恶人（300），而 `Books::bail_price` 就是那个唯一来源
    pub cost: u32,
    /// 花这笔钱买到的是哪件事。两行都写着「关在里面」的话，玩家看不出 300 那行买的是武器
    pub note: &'static str,
    /// `Some(为什么)` = 这颗按钮灰着
    pub blocked: Option<&'static str>,
}

/// 柜台一行的全部读数。买与卖共用同一个形状，所以共用同一个结构而不是两套参数表 ——
/// 这是这仓库的房规（同 `ButtonSpec`）：`shop_row` 本来要八份参数，而其中五份
/// 本来就是「这一行长什么样」这一件事（§六 坑 8）。
pub(crate) struct ShopRow {
    pub card: CardId,
    pub side: TradeSide,
    pub title: String,
    pub desc: String,
    /// 按钮上的字**带数额**（`money_row` 同一条）：按下去要花多少、收回多少，
    /// 是玩家在这一屏唯一要做的那个判断
    pub button: String,
    /// 灰按钮必须自带一句为什么（§6.1 坑 27 的 UI 版）。这句话直接取规则层那张表的
    /// [`ShopDeny::label`] —— 「按不动的原因」与「被拒的原因」从此是同一句话。
    pub blocked: Option<&'static str>,
}

/// 柜台的两排：货架在上、卖回在下（[`shop_rows`] 的产出）。
pub(crate) struct ShopRows {
    pub buys: Vec<ShopRow>,
    pub sells: Vec<ShopRow>,
}

/// 把「这一趟的货架 + 这个人的手牌」翻成两排按钮。
///
/// 判据一律走规则层那张 [`shop_op_allowed`]，面板不许自己发明一套「能不能办」。
/// `at_counter` 在这里恒为 `true` —— 柜台既然开着，`NoCounter` 那一格在这一屏永远碰不到，
/// 而它在派发那一侧是真的会挡（见 `ui::mod` 的 `shop_customer`）。
fn shop_rows(
    books: &Books,
    shelf: &Shelf<'_>,
    stock: &ShopStock,
    player: Entity,
    held: &[CardId],
) -> ShopRows {
    let points = books.points_of(player);
    let buys: Vec<ShopRow> = stock
        .slots
        .iter()
        .map(|card| {
            let def = shelf.get(CardKind::Item, *card);
            let price = def.and_then(|d| d.cost_points);
            ShopRow {
                card: *card,
                side: TradeSide::Buy,
                title: card_title(def),
                desc: def.map(|d| d.desc.clone()).unwrap_or_default(),
                button: format!("买 {}", price.unwrap_or(0)),
                blocked: shop_op_allowed(TradeSide::Buy, true, price, true, false, points)
                    .err()
                    .map(ShopDeny::label),
            }
        })
        .collect();
    // 卖回那一排按**不同的卡**聚合（两张一样的并成一行带 ×2）：两颗粒看起来完全相同的
    // 按钮读起来像面板坏了，而它们办的是同一件事
    let mut counts: Vec<(CardId, u32)> = Vec::new();
    for card in held {
        match counts.iter_mut().find(|(id, _)| id == card) {
            Some((_, n)) => *n += 1,
            None => counts.push((*card, 1)),
        }
    }
    let sells: Vec<ShopRow> = counts
        .iter()
        .map(|(card, n)| {
            let def = shelf.get(CardKind::Item, *card);
            let price = def.and_then(|d| d.cost_points);
            let title = card_title(def);
            ShopRow {
                card: *card,
                side: TradeSide::Sell,
                title: if *n > 1 {
                    format!("{title} ×{n}")
                } else {
                    title
                },
                desc: def.map(|d| d.desc.clone()).unwrap_or_default(),
                button: format!(
                    "卖 {}",
                    buyback(price.unwrap_or(0), books.rules.shop_buyback)
                ),
                // 柜台开着、手里有（这一排的存在本身就是）→ 这里还可能红的只有「没有售价」那一格
                blocked: shop_op_allowed(TradeSide::Sell, true, price, false, true, points)
                    .err()
                    .map(ShopDeny::label),
            }
        })
        .collect();
    ShopRows { buys, sells }
}

/// 卡名。查不到卡定义时**如实说数据错误**，而不是把这一行省掉 ——
/// 「少一行」是唯一线索，而那读不出为什么（同手牌面板那一条）。
fn card_title(def: Option<&CardDef>) -> String {
    def.map(|d| d.title.clone())
        .unwrap_or_else(|| "<数据里没有这张卡>".to_string())
}

/// 百货公司柜台（P3d②）：**踩到**百货格才开（路过什么都不给，见 `shop::shop_op_allowed`）。
///
/// 与银行柜台同一条形状：不是一次报价而是一个窗口，所以 [`modal_stamp`] 给它算了签名 ——
/// 每一笔买卖之后点券、货架、手牌都要重烤。
fn shop_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    rows: &ShopRows,
    player: Entity,
    stamp: String,
) -> Entity {
    let points = books.points_of(player);

    frame(
        commands,
        (player, Pending::Shop),
        stamp,
        layout::SHOP_PANEL,
        col::BORDER_SHOP,
        |p| {
            p.spawn(center_label(
                &font.0,
                "百货公司",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!("点券 {points} — 买卡、卖回打 9 折，办完才继续"),
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            p.spawn(label(&font.0, "这一趟的货架", size::SMALL, col::KEY));
            if rows.buys.is_empty() {
                // 只说「没有货」不说「被谁买空的」：面板读得到货架空，读不到它为什么空 ——
                // `NM_SHOP` 开第二局时这里也是空的，那句「被买空了」就是一句假话
                p.spawn(label(
                    &font.0,
                    "这一趟没有货 —— 下次进店再摆",
                    size::ROW,
                    col::KEY,
                ));
            }
            for row in &rows.buys {
                shop_row(p, font, row);
            }
            p.spawn(separator());
            p.spawn(label(&font.0, "卖回（打 9 折）", size::SMALL, col::KEY));
            if rows.sells.is_empty() {
                p.spawn(label(&font.0, "手里没有卡", size::ROW, col::KEY));
            }
            for row in &rows.sells {
                shop_row(p, font, row);
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "离开",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 柜台的一行：「卡名 + 效果 + 一颗带数额的按钮 + 灰掉时的那句话」。
fn shop_row(p: &mut ChildSpawnerCommands, font: &DevFont, row: &ShopRow) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        row_gap: Val::Px(layout::SHOP_ROW_GAP),
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
                format!("{}  {}", row.title, row.desc),
                size::ROW,
                if row.blocked.is_some() {
                    col::KEY
                } else {
                    col::BODY
                },
            ),
        ));
        button(
            line,
            &font.0,
            ButtonSpec::new(
                &row.button,
                layout::SHOP_BUTTON,
                super::PRIMARY,
                UiAction::ShopOp {
                    side: row.side,
                    card: row.card,
                },
            )
            .disabled(row.blocked.is_some()),
        );
        // 灰掉的话那句话就写在按钮旁边 —— 面板上只有一处解释会读漏（同手牌浮层）
        if let Some(reason) = row.blocked {
            line.spawn(label(&font.0, reason, size::ROW, col::JAILED));
        }
    });
}

/// 彩票柜台那一屏的读数。收成一块而不是十个参数（房规 §六 坑 8），而且这几样本来就是
/// 「这一屏该亮哪几颗按钮」这一件事：现金、一注的价、号码池、已有哪几个号、别人占了哪几个。
/// 奖池与上次开出的号只用于说明文字，不进判据。
pub(crate) struct LotteryDesk {
    pub cash: i64,
    pub price: i64,
    pub numbers: u32,
    pub held: Vec<u8>,
    /// 别人手里占着的号 → 占着它的人（P8 的号码独占，10-26 实报）。
    ///
    /// 这是**公开信息**，而且必须公开：独占之后的判据是「这颗号属于别人，买不成」，
    /// 玩家按下之前有权知道是哪一颗、被谁抢了。P8 初版写过「谁买了什么在这一局里从来
    /// 不是公开信息」—— 那句话在没有独占的旧规则下成立（各买各的，说出来没有用），
    /// 而它随那条规则一起作废。
    pub taken: Vec<(u8, String)>,
    /// 奖池现在多少。它是这一屏的标题级读数（「奖池 N」），不是判据 —— 池子动多少
    /// 由规则层算，面板只念
    pub pot: i64,
    /// 上期开出的号。`None` = 这一局还没开过奖，那一行得说这句而不是空着
    pub last_draw: Option<u8>,
}

/// 别人手里占着的号 → `(号, 持有人)`。判据与 `lottery_op_system` 里那次扫描是同一条：
/// **只数还活着的人**（`Books::is_bankrupt`）—— 破产者兑不出钱，他那张旧凭证既不该被
/// 面板说成「占着」，也不该真的替全场锁死一个号码（两处读同一个 `is_bankrupt`，
/// 灰按钮与扣钱因此不可能分叉）。
fn tickets_by_others(
    books: &Books,
    names: &Query<&PlayerName>,
    tickets: &Query<(Entity, &LotteryTickets), With<PlayerId>>,
    player: Entity,
) -> Vec<(u8, String)> {
    let mut rows: Vec<(u8, String)> = Vec::new();
    for (holder, held) in tickets.iter() {
        if holder == player || books.is_bankrupt(holder) {
            continue;
        }
        let who = names
            .get(holder)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| "？".to_string());
        for number in &held.numbers {
            rows.push((*number, who.clone()));
        }
    }
    rows.sort_by_key(|(number, _)| *number);
    rows
}

impl LotteryDesk {
    /// 这颗号此刻灰着的那句话。
    ///
    /// 判据一律走规则层那张 [`buy_allowed`]，面板不许自己发明一套「能不能买」——
    /// `Taken` 那一格读的就是上面那份公开读数，所以「这颗号买不成」在面板与规则层是
    /// 同一个来源。`at_counter` 恒为 `true` —— 柜台既然开着，`NoCounter` 在这一屏永远
    /// 碰不到，而它在派发那一侧是真的会挡（见 `ui::mod` 的 `lottery_customer`）。
    fn deny(&self, number: u32) -> Option<LotteryDeny> {
        let n = number as u8;
        buy_allowed(
            true,
            number,
            self.numbers,
            self.held.contains(&n),
            self.taken.iter().any(|(t, _)| *t == n),
            self.cash,
            self.price,
        )
        .err()
    }

    fn rows(&self) -> Vec<(u8, Option<LotteryDeny>)> {
        (1..=self.numbers.max(1))
            .map(|n| (n as u8, self.deny(n)))
            .collect()
    }

    /// 是不是**所有**买不成的号都因为同一句话。是的话这一句写在方格阵下面一次，
    /// 而不是在 36 颗按钮旁边各贴一遍 —— 贴 36 遍等于没贴。
    ///
    /// 「每颗灰按钮都得带一句为什么」这条（§6.1 坑 27）在这里换成了三种可见形状：
    /// 「已经买过」写在那颗号上（带 ✓，而他的号全列在下面一行），
    /// 「被别人买走」**一格一个人**、说不出全池通用的一句，所以单独成行（`taken` 那一行，
    /// 号与名字都在）；剩下的 `BadNumber` 在这一屏碰不到 —— 按钮本来就是按号码池建的。
    /// 于是这一行实际只可能念出「现金不够」那一句，而它管的还是**没有主的那些号**。
    fn one_reason(&self) -> Option<&'static str> {
        let mut only = None;
        for number in 1..=self.numbers.max(1) {
            let why = match self.deny(number) {
                // 有一颗是亮的 → 「为什么按不动」就不止一句话了
                None => return None,
                Some(LotteryDeny::Taken) => continue,
                Some(why) => why.label(),
            };
            if only.is_some() && only != Some(why) {
                return None;
            }
            only = Some(why);
        }
        only
    }

    /// 「已被买走：3（姬百年）· 17（范一文）」。一个号都没被占时这行整条不写 ——
    /// 空着的解释行读起来像面板坏了（与百货那句「被买空了」同一课）。
    fn taken_line(&self) -> Option<String> {
        if self.taken.is_empty() {
            return None;
        }
        Some(format!(
            "已被买走：{}",
            self.taken
                .iter()
                .map(|(n, who)| format!("{n}（{who}）"))
                .collect::<Vec<_>>()
                .join(" · ")
        ))
    }
}

/// 彩票中心柜台（P8）：**踩到**彩票格才开，与百货同一个形状。
///
/// 号码是 6×6 的方格阵而不是 36 行清单：这一屏要做的是「挑一个号」，
/// 读起来该像一张奖券。已经买过的号**亮着带 ✓**而不是从池里消失 —— 它自己那张券
/// 是他这一期的凭据，从池里抹掉就成了「我的号怎么没了」。
///
/// 别人占走的号**灰着**，而「谁占了哪个号」写在方格阵下面那一行（见 [`LotteryDesk::taken`]
/// 那条）：号码独占之后这句话必须公开，否则一颗按不动的按钮只能让人猜规则。
fn lottery_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    desk: &LotteryDesk,
    player: Entity,
    stamp: String,
) -> Entity {
    let rows = desk.rows();
    let blocked = desk.one_reason();
    let taken_line = desk.taken_line();
    frame(
        commands,
        (player, Pending::Lottery),
        stamp,
        layout::LOTTERY_PANEL,
        col::BORDER_LOTTERY,
        |p| {
            p.spawn(center_label(
                &font.0,
                "彩票中心",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!(
                    "现金 {} — 一注 {}，每月 15 号开奖",
                    books.balance(player),
                    desk.price
                ),
                size::ROW,
                col::MUTED,
            ));
            p.spawn(center_label(
                &font.0,
                format!("奖池 {}", desk.pot),
                size::ROW,
                col::TITLE_GOLD,
            ));
            p.spawn(separator());
            p.spawn(label(&font.0, "挑一个号", size::SMALL, col::KEY));
            for line in rows.chunks(layout::LOTTERY_COLUMNS) {
                p.spawn(Node {
                    flex_direction: FlexDirection::Row,
                    justify_content: JustifyContent::Center,
                    column_gap: Val::Px(layout::LOTTERY_CELL_GAP),
                    row_gap: Val::Px(layout::LOTTERY_CELL_GAP),
                    ..default()
                })
                .with_children(|row| {
                    for (number, why) in line {
                        let held = desk.held.contains(number);
                        let text = if held {
                            format!("{number} ✓")
                        } else {
                            number.to_string()
                        };
                        button(
                            row,
                            &font.0,
                            ButtonSpec::new(
                                &text,
                                layout::LOTTERY_NUMBER_BUTTON,
                                super::PRIMARY,
                                UiAction::BuyTicket(*number),
                            )
                            .disabled(why.is_some()),
                        );
                    }
                });
            }
            if let Some(reason) = blocked {
                p.spawn(label(&font.0, reason, size::ROW, col::JAILED));
            }
            // 独占的那几颗各带一个名字，塞不进「全池同一句」那一条，所以单独一行
            if let Some(line) = taken_line {
                p.spawn(label(&font.0, line, size::ROW, col::JAILED));
            }
            p.spawn(separator());
            p.spawn(label(
                &font.0,
                if desk.held.is_empty() {
                    "手里还没有一注".to_string()
                } else {
                    format!(
                        "你的号：{}",
                        desk.held
                            .iter()
                            .map(|n| n.to_string())
                            .collect::<Vec<_>>()
                            .join(" · ")
                    )
                },
                size::ROW,
                col::KEY,
            ));
            p.spawn(label(
                &font.0,
                match desk.last_draw {
                    Some(n) => format!("上期开出 {n} 号"),
                    None => "这一局还没开过奖".to_string(),
                },
                size::SMALL,
                col::MUTED,
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "离开",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 门这一屏的读数（P13b②）。与 [`BidDesk`] 同一条形状：几样判据收在一块，面板不许自己
/// 发明「这张能不能用」—— 张数与候选都是门里那两个函数（`holds`／`shift_candidates`）
/// 读的同一份事实。
pub(crate) struct GateAsk {
    /// 问到哪一问（免罪？→ 嫁祸给谁？）。问序在规则层（PLAN §五 第 3 条），这里只念
    pub ask: Ask,
    /// 表上哪一行（「坐牢」）。玩家按下之前必须知道自己要挡掉的是哪一件事
    pub event: PassiveEvent,
    /// 这一问由哪张卡答。`None` = 表上这一行没有抵消卡 —— 门不会为它挂出第一问，
    /// 所以这一格走不到；真走到了就念一个问号而不是替规则层编一张卡
    pub card: Option<PassiveCard>,
    /// 他手里有几张这一种。门问的就是这一位，所以这一格与「有资格」是同一句话
    pub held: u32,
    /// 可以点名给谁（转嫁那一问的候选，已排除自己，名字在这里查好）
    pub blame: Vec<(Entity, String)>,
    /// 这一问之后还排着几份信封。主语会跳动，而这一格是「答完这一句为什么还没回到回合机」
    /// 的唯一说明（与拍卖台那句「后面还排着几块」同一条读数）
    pub queued: usize,
    /// 回合主人叫什么。这一格必须念出来：一张卡从甲手上打到乙身上时，这一屏问的是乙，
    /// 而按钮按下去改的是乙的处境 —— 玩家有权知道自己在替谁答（PLAN §五 第 2 条的画面对照）
    pub owner: String,
}

/// 门这一屏（P13b②）：一句问话、一排动作，而**主语每一帧都可能换人**（同拍卖台那条）。
///
/// 三格动作与门里 `PassiveMove` 的三臂逐字对应：用（花一张）、不用（放弃）、嫁祸给谁
/// （点名 + 花一张）。**放弃不消耗卡**是验收⑤，所以那颗按钮旁边永远写着那一句 ——
/// 玩家在两问之间按「不用」时，要知道手里那几张一张都没少。
fn passive_modal(
    commands: &mut Commands,
    font: &DevFont,
    names: &Query<&PlayerName>,
    desk: &GateAsk,
    player: Entity,
    stamp: String,
) -> Entity {
    // 「谁在答」与「谁是回合主人」两句都要念得出，否则替别人答那一下看着像自己做了决定
    let seat_of = seat(names, player);
    frame(
        commands,
        (player, Pending::Passive),
        stamp,
        layout::AUCTION_PANEL,
        col::BORDER_AUCTION,
        |p| {
            p.spawn(center_label(
                &font.0,
                "被动卡",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            // 问话原文在规则层（`Ask::label`）：面板只念，不改写 —— 两处各写一遍就会出现
            // 「门问『要用免罪卡吗』而按钮写着『挡掉这次』」
            p.spawn(center_label(&font.0, desk.ask.label(), size::ROW, col::KEY));
            p.spawn(center_label(
                &font.0,
                format!("{seat_of} 正挨一次「{}」", desk.event.label()),
                size::ROW,
                col::MUTED,
            ));
            if desk.owner != seat_of {
                p.spawn(center_label(
                    &font.0,
                    // 两个名字都要念出来，且「他」不许只有一个先行词：写成「现在是甲的回合 ——
                    // 这一问挂在他身上」时，那个「他」在屏幕上读起来像甲，而问的是乙
                    format!(
                        "现在是 {} 的回合，而这一问挂在 {} 身上 —— 答完也由他挨",
                        desk.owner, seat_of
                    ),
                    size::SMALL,
                    col::JAILED,
                ));
            }
            p.spawn(separator());
            match desk.ask {
                Ask::Offset => {
                    // 卡名与张数都来自门里那一句判据读的同一格（`PassiveEvent::offset_card`
                    // ＋`GateDesk::holds`），所以「写着 2 张、门吃掉 1 张」这种分叉画不出来
                    let card = desk.card.map(PassiveCard::label).unwrap_or("?");
                    p.spawn(label(
                        &font.0,
                        format!("手里有 {card} {held} 张", held = desk.held),
                        size::ROW,
                        col::KEY,
                    ));
                    p.spawn(label(
                        &font.0,
                        // 「同类多张只问一次」（PLAN §五 第 7 条）：这句话要说得出按一次
                        // 只花一张、剩下的那张还在
                        format!(
                            "用一张就把这一次「{}」抵消掉；剩下的留着挨下一次",
                            desk.event.label()
                        ),
                        size::SMALL,
                        col::MUTED,
                    ));
                    button_row(p, |line| {
                        button(
                            line,
                            &font.0,
                            ButtonSpec::new(
                                &format!("用{card}（手里 {} 张）", desk.held),
                                layout::AUCTION_BUTTON,
                                super::PRIMARY,
                                UiAction::Gate(PassiveMove::Use),
                            ),
                        );
                        button(
                            line,
                            &font.0,
                            ButtonSpec::new(
                                "不用（不花卡）",
                                layout::AUCTION_BUTTON,
                                super::SECONDARY,
                                UiAction::Gate(PassiveMove::Decline),
                            ),
                        );
                    });
                }
                Ask::Shift => {
                    p.spawn(label(
                        &font.0,
                        format!(
                            "把这一次「{}」推给点名的那一位；他自己手里有卡，还能接着挡",
                            desk.event.label()
                        ),
                        size::ROW,
                        col::KEY,
                    ));
                    p.spawn(label(
                        &font.0,
                        // 「链最长两跳」在屏幕上的那一格：接住的那位不能再往下转
                        "他转到别人之后，别人只可以被挡、不可以再往下转 —— 一条链最多两跳",
                        size::SMALL,
                        col::MUTED,
                    ));
                    if desk.blame.is_empty() {
                        // 门不会为「没有可点名的人」挂出这一问（`next_question` 那句判据），
                        // 走到这里只可能是问出之后场上少了一个人。说一句而不是摆一排空按钮
                        p.spawn(label(
                            &font.0,
                            "场上没有可点名的别人了",
                            size::ROW,
                            col::JAILED,
                        ));
                    }
                    for (other, name) in &desk.blame {
                        let _ = other;
                        button_row(p, |line| {
                            button(
                                line,
                                &font.0,
                                ButtonSpec::new(
                                    &format!("嫁祸给{name}（花一张嫁祸卡）"),
                                    layout::AUCTION_BUTTON,
                                    super::PRIMARY,
                                    UiAction::Gate(PassiveMove::Shift { to: *other }),
                                ),
                            );
                        });
                    }
                    button_row(p, |line| {
                        button(
                            line,
                            &font.0,
                            ButtonSpec::new(
                                "不转（不花卡）",
                                layout::AUCTION_BUTTON,
                                super::SECONDARY,
                                UiAction::Gate(PassiveMove::Decline),
                            ),
                        );
                    });
                }
            }
            if desk.queued > 0 {
                // 同拍卖台那句：不说清楚，玩家会把「答完这一句回合还没动」当成面板卡住了
                p.spawn(label(
                    &font.0,
                    format!("后面还排着 {} 份，一份答完才轮到下一个", desk.queued),
                    size::SMALL,
                    col::MUTED,
                ));
            }
        },
    )
}

/// 拍卖这一屏的读数（P9）。与 [`LotteryDesk`] 同一条形状：判据要读的几样收在一块，
/// 面板不许自己发明「能不能喊」。
///
/// 名字不叫 `AuctionDesk`：规则层已经有一个同名的参数包（`game_core::auction::AuctionDesk`），
/// 而这一屏读的正是它怀里那个资源 —— 两个同名类型在同一份 import 列表里会互相遮蔽。
pub(crate) struct BidDesk {
    /// 正在进行的这一场（从 `books.auction.lot` 抄下来的一份，不是引用 —— 面板要在
    /// 同一次读数里既看 `pool` 又看 `leader`，留着借用的话下面每一步判据都得再借一次）
    pub lot: Lot,
    /// 这块地叫什么。`LandId(3)` 玩家读不出声，而拍卖台上「拍的是哪块」是第一句话
    pub plot: String,
    /// 被问的这位现在有多少钱
    pub cash: i64,
    /// 他身上那位让置产必败（大衰神／小衰神）。判据走 `Books::acquire_denied`，
    /// 与规则层拒绝他那一声时问的是**同一句**
    pub denied_by_god: bool,
    /// 这一场之外还排着几块（只有破产清算会有：死者名下通常不止一块）
    pub queued: usize,
    /// 有资格出价的全部人（含已经放弃的）。「谁退出了」由 `pool` 与这一份相减得到 ——
    /// 场上没有第二处记着这件事，而面板该念出「已退出」而不是让那个人凭空消失
    /// （消失了玩家会以为他破产离场了，那是另一条凭据会说的事）
    pub eligible: Vec<Entity>,
}

impl BidDesk {
    /// 下一档要多少。**只有这一句**：按钮上写的数与规则层扣的数出自同一个表达式
    /// （`Lot::next_amount`），所以「按上去显示的」与「真付的」不可能分叉。
    pub fn needed(&self) -> i64 {
        self.lot.next_amount()
    }

    /// 这颗「加到 N」此刻按不按得动，连同它灰着的那句话。
    ///
    /// `at_lot` 与 `yours` 恒为 `true`：窗口就挂在这位身上（`auction_hold_system` 每一帧
    /// 把 `lot.bidder()` 挂上去），所以这一屏上碰不到那两条 —— 它们在派发那一侧是真的
    /// 会挡（见 `ui::mod` 的 `auction_customer`）。
    fn deny(&self) -> Option<BidDeny> {
        bid_allowed(&BidFacts {
            at_lot: true,
            yours: true,
            denied_by_god: self.denied_by_god,
            cash: self.cash,
            needed: self.needed(),
        })
        .err()
    }

    /// 已经退出这一场的人。
    fn out(&self) -> Vec<Entity> {
        self.eligible
            .iter()
            .copied()
            .filter(|e| !self.lot.pool.contains(e))
            .collect()
    }
}

/// 一个人的名字。查不到时给一个问号而不是空串 —— 拍卖台上每一行都在说「谁」，
/// 空串读起来像这一位不存在。
fn seat(names: &Query<&PlayerName>, player: Entity) -> String {
    names
        .get(player)
        .map(|n| n.0.clone())
        .unwrap_or_else(|_| "?".into())
}

/// 一排人写成「甲 · 乙 · 丙」。空名单写成 `None` 让调用方自己决定那句话怎么说 ——
/// 「还在争：（空）」与「已经没有人还在争」是两句话。
fn seats(names: &Query<&PlayerName>, list: &[Entity]) -> Option<String> {
    (!list.is_empty()).then(|| {
        list.iter()
            .map(|e| seat(names, *e))
            .collect::<Vec<_>>()
            .join(" · ")
    })
}

/// 拍卖台（P9）：一处窗口、两颗按钮，而**主语每一帧都可能换人**。
///
/// 它与其余五个窗口的差别全写在 `Pending::Auction` 那段上：柜台从头到尾问的是踩格的
/// 那个人，这里一圈一圈问不同的人 —— 所以这一屏的内容**不能**是「弹出那一刻烤死」的，
/// 签名里带上了价钱与名单（见 [`modal_stamp`] 那一格）。
///
/// 两颗按钮而不是一排金额：英式拍卖里玩家要决定的从来不是「喊多少」而是「敢不敢再加一档」，
/// 而任意喊价会把「同种子复现同一串拍卖」变成一件做不到的事。
fn auction_modal(
    commands: &mut Commands,
    font: &DevFont,
    names: &Query<&PlayerName>,
    desk: &BidDesk,
    player: Entity,
    stamp: String,
) -> Entity {
    let lot = &desk.lot;
    let blocked = desk.deny();
    let fighting = seats(names, &lot.pool);
    let quit = seats(names, &desk.out());
    frame(
        commands,
        (player, Pending::Auction),
        stamp,
        layout::AUCTION_PANEL,
        col::BORDER_AUCTION,
        |p| {
            p.spawn(center_label(
                &font.0,
                "拍卖台",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!(
                    "{} — 起拍 {}，一档 {}",
                    desk.plot,
                    comma(lot.opening),
                    comma(lot.step)
                ),
                size::ROW,
                col::MUTED,
            ));
            // 槌是谁敲的、钱最后归谁：这三句在三种入口上是不同的事，而玩家按下之前必须知道
            // 自己是在替别人抬价、在抢一块死者留下的地，还是在抢一块政府挂牌的公有地
            p.spawn(center_label(
                &font.0,
                match lot.gavel {
                    Gavel::Card(founder) => format!(
                        "{} 敲的槌 —— 他不能参与，而谁拍走那笔钱就归他",
                        seat(names, founder)
                    ),
                    Gavel::Liquidation(dead) => format!(
                        "{} 破产变卖 —— 所得归银行，不发给任何玩家",
                        seat(names, dead)
                    ),
                    // 「公有地」那一句要说清**没有原主**：面板上其余两处都念得出一个卖家，
                    // 这一处念不出，而念不出与漏印是两种读感
                    Gavel::Public => "公有地政府挂牌 —— 没有原主，所得归银行".to_string(),
                },
                size::SMALL,
                col::KEY,
            ));
            p.spawn(separator());
            p.spawn(label(
                &font.0,
                match lot.leader {
                    Some(leader) => {
                        format!("已喊到 {}（{}）", comma(lot.high), seat(names, leader))
                    }
                    None => format!("还没人认价 —— 认下就是 {}", comma(lot.opening)),
                },
                size::ROW,
                col::KEY,
            ));
            p.spawn(label(
                &font.0,
                fighting.map_or_else(
                    || "已经没有人还在争".to_string(),
                    |who| format!("还在争：{who}"),
                ),
                size::ROW,
                col::MUTED,
            ));
            if let Some(who) = quit {
                p.spawn(label(
                    &font.0,
                    format!("已退出：{who}"),
                    size::SMALL,
                    col::JAILED,
                ));
            }
            if desk.queued > 0 {
                // 一处清算通常不止一块。这句不是装饰：不说清楚，玩家会把这一场当成最后一场,
                // 而散场之后回合还压在拍卖机上，看着像「拍完了却不动」
                p.spawn(label(
                    &font.0,
                    format!("后面还排着 {} 块，一块拍完才轮到下一块", desk.queued),
                    size::SMALL,
                    col::MUTED,
                ));
            }
            p.spawn(separator());
            p.spawn(label(
                &font.0,
                format!("下一档 {} —— 现在轮到你出价", comma(desk.needed())),
                size::ROW,
                col::KEY,
            ));
            p.spawn(label(
                &font.0,
                format!("你的现金 {}", comma(desk.cash)),
                size::SMALL,
                col::MUTED,
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        &format!("加到 {}", comma(desk.needed())),
                        layout::AUCTION_BUTTON,
                        super::PRIMARY,
                        UiAction::Bid(BidMove::Raise),
                    )
                    .disabled(blocked.is_some()),
                );
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "放弃这一场",
                        layout::AUCTION_BUTTON,
                        super::SECONDARY,
                        UiAction::Bid(BidMove::Withdraw),
                    ),
                );
            });
            // 灰按钮必须自己说得出为什么（§6.1 坑 27）：这里只有一颗会灰，而它灰的原因
            // 只有两种 —— 身上那位，或者钱不够那一档
            if let Some(why) = blocked {
                p.spawn(label(&font.0, why.label(), size::ROW, col::JAILED));
            }
        },
    )
}

/// 保释窗口要读的那一排：这一处关着谁、这笔钱够不够。与规则层读同一份
/// [`bail_op_allowed`]，所以「灰着按不动」与「按下去办不成」不会分叉。
fn bail_rows(
    books: &Books,
    names: &Query<&PlayerName>,
    player: Entity,
    place: DetentionPlace,
) -> Vec<BailRow> {
    let points = books.points_of(player);
    books
        .held(place)
        .into_iter()
        .map(|(target, held)| {
            // 价钱与「他是什么」都问账本那一张表（`bail_price` 的判据是身上有没有 `Villain`），
            // 面板这里一个字都不判断 —— 灰按钮的理由与真正扣的钱因此不可能分叉
            let cost = books.bail_price(target);
            let facts = BailFacts {
                at_window: true,
                window: place,
                target_place: Some(held.place),
                points,
                cost,
            };
            let villain = books.is_villain(target);
            BailRow {
                target,
                name: names
                    .get(target)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|_| "?".into()),
                cost,
                note: if villain {
                    "放出来替你干活"
                } else {
                    "替他减刑"
                },
                blocked: bail_op_allowed(&facts).err().map(|d: BailDeny| d.label()),
            }
        })
        .collect()
}

/// 保释窗口（P4③）。与百货柜台同一条形状：一个窗口、一排候选、一颗「离开」。
/// 之所以不是「选一个回答」，是因为同一处可以关着好几个人，玩家可能想一次放两个。
fn bail_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    place: DetentionPlace,
    rows: &[BailRow],
    stamp: String,
) -> Entity {
    let points = books.points_of(player);

    frame(
        commands,
        (player, Pending::Bail(place)),
        stamp,
        layout::SHOP_PANEL,
        col::BORDER_SHOP,
        |p| {
            p.spawn(center_label(
                &font.0,
                format!("{}门口", place.label()),
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!("点券 {points} — 把里面的人放出来，各行多少钱写在按钮上"),
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            if rows.is_empty() {
                // 分派层在「没人可保」时不会摆这个窗口（见 `settle_bail_landing_system`），
                // 走到这里只可能是保释刚把最后一个人放走、面板还没重建的那一帧
                p.spawn(label(&font.0, "这里没有关着的人", size::ROW, col::KEY));
            }
            for row in rows {
                bail_row(p, font, row);
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "离开",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 保释窗口的一行：「名字 + 他关着这件事 + 一颗带价目的按钮 + 灰掉时的那句话」。
///
/// 价钱写在**按钮上**而不是标题里（P12）：同一排里 30 与 300 并存，而一句话只能有一个数。
fn bail_row(p: &mut ChildSpawnerCommands, font: &DevFont, row: &BailRow) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        row_gap: Val::Px(layout::SHOP_ROW_GAP),
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
                format!("{} 关在里面 · {}", row.name, row.note),
                size::ROW,
                if row.blocked.is_some() {
                    col::KEY
                } else {
                    col::BODY
                },
            ),
        ));
        button(
            line,
            &font.0,
            ButtonSpec::new(
                &format!("保释 {} {}", row.name, row.cost),
                layout::SHOP_BUTTON,
                super::PRIMARY,
                UiAction::Bail { target: row.target },
            )
            .disabled(row.blocked.is_some()),
        );
        if let Some(reason) = row.blocked {
            line.spawn(label(&font.0, reason, size::ROW, col::JAILED));
        }
    });
}

// ===== 魔法屋（P13）=====

/// 魔法屋**第一步**：女巫给的筛选条件（11 条）。
///
/// 「玩家不能指定具体人」是玩法不是限制（PLAN §三 那行）—— 指定人会把「害人」变成一次
/// 查名操作，条件把它变成一次博弈。所以这一屏不摆一排名字，而摆 11 条条件、每条右边跟着
/// **它此刻筛出谁**；筛不出人的那几条灰掉，理由就写在行右（这一屏 11 行的理由各不相同，
/// 逐行写才是省的 —— 与彩票那种「36 颗共用一句总理由」正好相反）。
fn magic_filter_modal(
    commands: &mut Commands,
    font: &DevFont,
    player: Entity,
    rows: &[MagicRow],
    stamp: String,
) -> Entity {
    frame(
        commands,
        (player, Pending::Magic),
        stamp,
        layout::MAGIC_PANEL,
        col::BORDER_MAGIC,
        |p| {
            p.spawn(center_label(
                &font.0,
                "魔法屋",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                "女巫只按条件指人 —— 挑一条，右边是她筛出的谁",
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            magic_list(p, |list| {
                for row in rows {
                    magic_filter_row(list, font, row);
                }
            });
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "离开",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 两步各自的那一截列表，收进一个自己的容器里排。
///
/// **不是美化**：面板的 `row_gap` 是 10，那是给「标题 / 分隔线 / 按钮行」这种异质块留的节奏。
/// 第二步直接铺 12 行时总高 686，唯一的出口「离开」落到 y≈732 —— 720 高的客户区里点不到，
/// 而这颗是「这一趟不作法」的唯一路径（十二颗法术各点下去都必定收窗口）。截图量过：
/// 那一屏一个主色像素都没有。列表自己拿 2px 行距之后那颗量回来在 y=641..680（第一步的
/// 在 y=569..609）—— 两个数都是从 PNG 上量的，不是按布局常量算的。
fn magic_list(p: &mut ChildSpawnerCommands, rows: impl FnOnce(&mut ChildSpawnerCommands)) {
    p.spawn(Node {
        flex_direction: FlexDirection::Column,
        row_gap: Val::Px(layout::MAGIC_LIST_GAP),
        ..default()
    })
    .with_children(|list| rows(list));
}

/// 第一步的一行：「一颗条件按钮 + 这一条筛出谁」。
///
/// 名单在 `sync_modal` 那侧就算好（`MagicRow.who`），这里一个字不判断 —— 「灰不灰」读的
/// 是同一条 `filter_targets` 的结果，面板与规则层因此不可能各说各话。
fn magic_filter_row(p: &mut ChildSpawnerCommands, font: &DevFont, row: &MagicRow) {
    let empty = row.who.is_empty();
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        row_gap: Val::Px(layout::MAGIC_ROW_GAP),
        ..default()
    })
    .with_children(|line| {
        button(
            line,
            &font.0,
            ButtonSpec::new(
                row.filter.label(),
                layout::MAGIC_BUTTON,
                super::SECONDARY,
                UiAction::MagicFilter(row.filter),
            )
            .disabled(empty),
        );
        line.spawn((
            Node {
                flex_grow: 1.0,
                ..default()
            },
            label(
                &font.0,
                if empty {
                    "没人命中 —— 这一条落空".to_string()
                } else {
                    row.who.join("、")
                },
                size::ROW,
                if empty { col::KEY } else { col::BODY },
            ),
        ));
    });
}

/// 魔法屋**第二步**：12 道法术，挑一道落到草稿那份名单上。
///
/// 名单是资源里的那一份（`MagicRitual.draft`），不是这里再筛一次 —— 从答完条件到点开法术
/// 之间世界可能动了，再筛一次会给玩家一份与真正挨那一下不同的名单。
///
/// 十二颗按钮**全亮着**：这一屏不灰任何一道。落空是这一格的合法结果（脚下没有可加盖的地、
/// 手里本来没卡、道具栏是空的），那一句「什么都没发生」由顶栏念出来（`MagicCast.targets`
/// 是空的）。反过来把「会不会落空」的十二判据各写一遍进面板，就是让灰按钮与规则层分叉 ——
/// 与保释/柜台那种「一张拒绝表两处读」不同，这里没有拒绝表，只有一条「落不落得空」的返回值。
fn magic_spell_modal(
    commands: &mut Commands,
    font: &DevFont,
    player: Entity,
    filter: MagicFilter,
    who: &[String],
    stamp: String,
) -> Entity {
    frame(
        commands,
        (player, Pending::MagicSpell(filter)),
        stamp,
        layout::MAGIC_PANEL,
        col::BORDER_MAGIC,
        |p| {
            p.spawn(center_label(
                &font.0,
                format!("女巫按「{}」筛出 {}", filter.label(), who.join("、")),
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!("挑一道 —— 落到这 {} 个人头上", who.len()),
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            magic_list(p, |list| {
                for spell in MagicSpell::ALL {
                    magic_spell_row(list, font, filter, spell);
                }
            });
            p.spawn(separator());
            p.spawn(label(
                &font.0,
                "只有坐牢与住院两道挡得住（免罪卡）—— 其余十道不受任何卡影响",
                size::SMALL,
                col::KEY,
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "离开",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 第二步的一行：「一颗法术按钮 + 它落到人头上是什么」。
///
/// 说明取自 core 的 `MagicSpell::desc` —— 文案在规则层，面板只念（与卡面 `desc` 同一条分工）。
fn magic_spell_row(
    p: &mut ChildSpawnerCommands,
    font: &DevFont,
    filter: MagicFilter,
    spell: MagicSpell,
) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        row_gap: Val::Px(layout::MAGIC_ROW_GAP),
        ..default()
    })
    .with_children(|line| {
        button(
            line,
            &font.0,
            ButtonSpec::new(
                spell.label(),
                layout::MAGIC_BUTTON,
                super::SECONDARY,
                UiAction::MagicSpell(filter, spell),
            ),
        );
        line.spawn((
            Node {
                flex_grow: 1.0,
                ..default()
            },
            label(&font.0, spell.desc(), size::SMALL, col::BODY),
        ));
    });
}

/// 建设窗口（P10c）：自己的**空商业地**上立哪一类设施。
///
/// 五类候选、各自一颗按钮，价钱**只有一行** —— 与升级同一格数据
/// （`economy::upgrade_cost`）：「在这块地上立一座购物中心」与「加盖一级」是同一笔
/// 建设费，价格不随类别变。每颗按钮的灰与理由都来自 [`Books::build_probe`]：
/// 面板算的那一笔就是账本要扣的那一笔（同一条表达式，不是两份同样的 if）。
///
/// 建完之后这一屏会重建成「这里已经立着设施」的样子（窗口照开，理由见
/// `landing::decision_system` 里 `Pending::Build` 那一格的注）。
fn build_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let name = books
        .active
        .def
        .land(land.0)
        .map_or_else(String::new, |l| l.name.clone());
    let built = books.facility_of(land);
    // 五类各问一次：`Ok(cost)` 顺手给出这一屏要写的价，`Err(why)` 就是那颗按钮上的灰
    let rows: Vec<(BuildingKind, Result<i64, String>)> = BuildingKind::ALL
        .iter()
        .map(|kind| {
            (
                *kind,
                books
                    .build_probe(player, land, *kind)
                    .map_err(|why| why.label().to_string()),
            )
        })
        .collect();
    let cost = rows
        .iter()
        .find_map(|(_, probe)| probe.as_ref().ok().copied())
        .unwrap_or(0);

    frame(
        commands,
        (player, Pending::Build(land)),
        stamp,
        layout::SHOP_PANEL,
        col::BORDER_SHOP,
        |p| {
            p.spawn(center_label(
                &font.0,
                format!("建设 {name}"),
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                if built.is_some() {
                    // 建成之后窗口还在：这一行就是「已经办完了」的凭据，
                    // 玩家按下面的「离开」才交还回合
                    format!("这里已经立着{}", built.map(|k| k.label()).unwrap_or("设施"))
                } else {
                    format!("造价 ${} — 立起之后路过的人按设施收费", comma(cost))
                },
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            for (kind, probe) in &rows {
                build_row(p, font, land, *kind, probe);
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        // 建成之后这一颗是正经出口；没建则是「这一趟不作」——
                        // 两条意思同一个动作（`decision_system` 只认 `Acknowledge`）
                        "离开",
                        layout::MODAL_BUTTON,
                        SECONDARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 建设窗口的一行：「设施名 + 一颗带价目的按钮 + 灰掉时的那句话」。
/// 与 [`bail_row`] 同一形 —— 五颗按钮都不在主行里，理由要写在自己旁边。
fn build_row(
    p: &mut ChildSpawnerCommands,
    font: &DevFont,
    land: LandId,
    kind: BuildingKind,
    probe: &Result<i64, String>,
) {
    p.spawn(Node {
        flex_direction: FlexDirection::Row,
        align_items: AlignItems::Center,
        column_gap: Val::Px(8.0),
        ..default()
    })
    .with_children(|line| {
        let (text, ink) = match probe {
            Ok(cost) => (format!("{} ${}", kind.label(), comma(*cost)), col::BODY),
            Err(_) => (kind.label().to_string(), col::KEY),
        };
        line.spawn((
            Node {
                flex_grow: 1.0,
                ..default()
            },
            super::label(&font.0, text, size::ROW, ink),
        ));
        button(
            line,
            &font.0,
            ButtonSpec::new(
                &format!("建{}", kind.label()),
                layout::SHOP_BUTTON,
                super::PRIMARY,
                UiAction::Build { land, kind },
            )
            .disabled(probe.is_err()),
        );
        if let Err(reason) = probe {
            line.spawn(super::label(&font.0, reason, size::ROW, col::JAILED));
        }
    });
}

/// 服务费收据那一屏要摊开的全部读数（P10c）。它是 `sync_modal` 在开窗那一刻从
/// `RecentService` 与世界状态里查好的 —— 「谁付、付给谁、在哪、怎么算的、多少钱」，
/// 五样都是消息里带着的事实，这一屏自己不再查一遍。
struct ServiceRow {
    land: LandId,
    kind: BuildingKind,
    payer_name: String,
    owner_name: String,
    land_name: String,
    amount: i64,
    roll: u32,
    stay_days: u32,
}

/// 查一屏收据的读数。`memory` 里那一笔必须**正是这块地**的：收据弹出来之前
/// 中间不会夹别的服务费（落格结算一帧一笔），但注入挂窗口时可能压根没有。
fn service_row(
    books: &Books,
    names: &Query<&PlayerName>,
    memory: &RecentService,
    land: LandId,
) -> Option<ServiceRow> {
    let bill = memory.0.filter(|b| b.land == land)?;
    Some(ServiceRow {
        land,
        kind: bill.kind,
        payer_name: names
            .get(bill.payer)
            .map(|n| n.0.clone())
            .unwrap_or_default(),
        owner_name: names
            .get(bill.owner)
            .map(|n| n.0.clone())
            .unwrap_or_default(),
        land_name: books
            .active
            .def
            .land(land.0)
            .map_or_else(String::new, |l| l.name.clone()),
        amount: bill.amount,
        roll: bill.roll,
        stay_days: bill.stay_days,
    })
}

/// 服务费收据（P10c）：钱已经结完（免单就是 0），这一屏只把「怎么算出来的」说清楚。
///
/// A 型要说摇出几点、B 型要说住了几天 —— 与租金收据同一个位置、同一条纪律：
/// 免单也要弹，否则玩家看不出「为什么没被收钱」。
fn service_modal(
    commands: &mut Commands,
    font: &DevFont,
    payer: Entity,
    row: &ServiceRow,
    stamp: String,
) -> Entity {
    // 怎么算出来的：A 型写档位、B 型写天数，C 型不写掷骰（它按步数，而步数不在消息里 ——
    // `roll` 那一格对 C 型是 0，写出来就是「摇出 0」这句假话）
    let detail = if row.stay_days > 0 {
        format!("摇出 {}，住 {} 天", row.roll, row.stay_days)
    } else if row.roll > 0 {
        format!("摇出 {}", row.roll)
    } else {
        "按这一回合的步数计费".to_string()
    };

    frame(
        commands,
        (payer, Pending::ServiceNotice(row.land)),
        stamp,
        layout::RENT_PANEL,
        col::BORDER_SHOP,
        |p| {
            p.spawn(center_label(
                &font.0,
                format!("{} 服务费", row.kind.label()),
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!(
                    "{} 踩到 {} 的{}",
                    row.payer_name, row.owner_name, row.land_name
                ),
                size::NAME,
                col::BODY,
            ));
            p.spawn(center_label(&font.0, detail, size::ROW, col::MUTED));
            p.spawn(center_label(
                &font.0,
                format!("- ${}", comma(row.amount)),
                size::RENT_AMOUNT,
                if row.amount == 0 {
                    col::MUTED
                } else {
                    col::COST
                },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "确认",
                        layout::OK_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 公司到访收据那一屏要摊开的全部读数（P10d①）。「谁付、付给哪家、怎么算的、多少钱、
/// 董事长是谁」，五样都是消息里带着的事实，这一屏自己不再查一遍。
struct CompanyRow {
    stock: String,
    company_name: String,
    payer_name: String,
    amount: i64,
    roll: u32,
    stay_days: u32,
    /// 保险格（P10d②）：转盘摇出的天数是**投保天数**，收据上「摇出 N」那半句
    /// 得跟着换说法；免单那句也要从「免费通行」换成「保费分文不取」。
    insure: bool,
    /// 董事长的名字。`None` = 这家公司还没有持股人，或付款的这位**就是**董事长
    /// —— 后一种时 [`company_modal`] 要把「为什么没被收钱」说出口。
    chairman_name: Option<String>,
}

/// 查一屏公司收据的读数。公司与地块不同：它不是任何人的产业，账本与格子
/// 各自有名字，在这里各查各的。
fn company_row(
    books: &Books,
    names: &Query<&PlayerName>,
    memory: &RecentCompany,
) -> Option<CompanyRow> {
    let bill = memory.0.as_ref()?;
    let tile = books
        .active
        .def
        .tiles
        .iter()
        .find(|t| t.data.stock.as_deref() == Some(&bill.stock));
    let company_name = tile
        .map(|t| t.name.clone())
        .unwrap_or_else(|| bill.stock.clone());
    let insure = matches!(
        tile.and_then(|t| t.data.fee.as_ref()),
        Some(game_core::board::CompanyFee::Insure { .. })
    );
    Some(CompanyRow {
        company_name,
        payer_name: names
            .get(bill.payer)
            .map(|n| n.0.clone())
            .unwrap_or_default(),
        chairman_name: bill
            .chairman
            .filter(|c| *c != bill.payer)
            .and_then(|c| names.get(c).ok())
            .map(|n| n.0.clone()),
        stock: bill.stock.clone(),
        amount: bill.amount,
        roll: bill.roll,
        stay_days: bill.stay_days,
        insure,
    })
}

/// 公司到访收据（P10d①）：钱已经结完（董事长免单那张收据金额就是 0），这一屏只把
/// 「为什么是这个数」说清楚 —— 免单也要弹，否则玩家看不出「为什么没被收钱」。
fn company_modal(
    commands: &mut Commands,
    font: &DevFont,
    payer: Entity,
    row: &CompanyRow,
    stamp: String,
) -> Entity {
    // 怎么算出来的：航空那类按天（摇出几点住几天）、科技那类按步数 —— 与 `service_modal`
    // 同一张嘴，但 C 型公司把步数算进了 `amount`，这里不再重复「按步数计费」那半句。
    // 保险格（P10d②）摇出的天数是**投保天数**，措辞跟着换：人没走，买的是一张保单
    let detail = if row.insure {
        format!("摇出 {}，投保 {} 天", row.roll, row.roll)
    } else if row.stay_days > 0 {
        format!("摇出 {}，住 {} 天", row.roll, row.stay_days)
    } else if row.roll > 0 {
        format!("摇出 {}", row.roll)
    } else {
        "免费通行".to_string()
    };
    // 董事长那一行：免单的收据必须说得出为什么免，不能只写一个「$0」。
    // 判定用的是「收据上还有没有别人当董事长」—— `company_row` 已经把
    // 董事长==付款人的那一格归成 `None`，所以 `Some(name)` 一定意味着
    // 「另有人当家」，这时哪怕金额是 0 也不许写成「你是董事长」
    // （`NM_COMPANY` 注入的凭据就踩过这一格：董事长是孙小美、金额 0）。
    let chairman_line = match &row.chairman_name {
        Some(name) => format!("董事长：{name}"),
        None => {
            if row.amount == 0 && row.insure {
                "你是董事长，保费分文不取（未投保）".to_string()
            } else if row.amount == 0 {
                "你是董事长，免费通行".to_string()
            } else {
                "这家公司还没有董事长".to_string()
            }
        }
    };

    frame(
        commands,
        (payer, Pending::CompanyNotice),
        stamp,
        layout::RENT_PANEL,
        col::BORDER_SHOP,
        |p| {
            p.spawn(center_label(
                &font.0,
                format!("{} 到访", row.company_name),
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                format!("{} 到访门面", row.payer_name),
                size::NAME,
                col::BODY,
            ));
            p.spawn(center_label(&font.0, detail, size::ROW, col::MUTED));
            p.spawn(center_label(&font.0, chairman_line, size::ROW, col::MUTED));
            p.spawn(center_label(
                &font.0,
                format!("- ${}", comma(row.amount)),
                size::RENT_AMOUNT,
                if row.amount == 0 {
                    col::MUTED
                } else {
                    col::COST
                },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "确认",
                        layout::OK_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 开发用：`NM_BUILD=1` 把演示盘那块商业地（外滩）划给当前回合玩家、摘掉预建的购物中心，
/// 再挂出建设窗口。
///
/// 摘除是 `Commands`（延迟写），所以开出来的第一帧面板可能还读到「已经立着设施」——
/// 下一帧签名翻转，`sync_modal` 自己会重建成空的建设窗口（那一帧的闪烁比「两帧才开窗」
/// 少一次往返，而两者都只在开发旋钮下出现）。
/// 判据由头上的 `Local` 收着，只在**这一局**做一次（与 `dev_grant_plots` 同一条代价）。
pub(crate) fn dev_open_build(
    mut commands: Commands,
    order: Option<Res<TurnOrder>>,
    active: Option<Res<ActiveBoard>>,
    faces: Query<&PlayerId>,
    mut lands: Query<(Entity, &LandId, &mut Owner)>,
    mut pending: ResMut<PendingDecision>,
    mut done: Local<bool>,
) {
    // 只在**这一局**做一次，而且窗口已经挂着别人时什么都不碰。两道门都必须在
    // **任何世界写入之前**：挡在后面的话，这个旋钮每帧都会重写 `Owner`，
    // 而 `render::land::sync_buildings` 是变化驱动的 —— 它于是每帧往那几块 slab
    // 上写 `Visibility`，撞上拆场那一帧就是一条「Entity despawned」的 panic
    // （本阶段实测：`NM_BUILD` 与 `NM_SERVICE` 同时打开、`NM_MENULOOP` 换场景时必崩）
    if *done || pending.active().is_some() {
        return;
    }
    let (Some(order), Some(active)) = (order, active) else {
        return;
    };
    let Some(player) = order.current() else {
        return;
    };
    let Some(pid) = faces.get(player).ok().copied() else {
        return;
    };
    let Some(def) = active
        .def
        .lands
        .iter()
        .find(|l| l.land_type == LandType::Commercial && l.building_kind.is_none())
        .or_else(|| {
            active
                .def
                .lands
                .iter()
                .find(|l| l.land_type == LandType::Commercial)
        })
    else {
        return;
    };
    let land = LandId(def.id);
    for (entity, id, mut owner) in &mut lands {
        if *id != land {
            continue;
        }
        owner.0 = Some(pid);
        // 预建设施要摘掉，否则这扇窗口开不出来（`Pending::Build` 的前提就是空地）
        commands.entity(entity).remove::<Facility>();
    }
    if pending.active().is_some() {
        return;
    }
    pending.offer(player, Pending::Build(land));
    *done = true;
}

/// 开发用：`NM_SERVICE=1` 把外滩那块商业地划给**另一位**玩家（预建的购物中心留着），
/// 再挂出一张服务费收据。
///
/// 为什么需要它：收据的正经入口是「有人踩到别人的设施」—— 那一掷由骰子说了算，而这一屏
/// **只有两三秒的窗口期**（`NM_AUTOROLL` 的自动玩家下一帧就把它答掉），截图按墙钟撞不上
/// （与 `NM_SHOP`／`NM_BUILD` 同一条判据：每个界面都得有一条不靠鼠标的入口）。
///
/// 它发一条**真凭据**（`ServiceCharged`，走 `remember_service` 那条正路）而不是只挂窗口：
/// 收据的内容全在这条消息里，不发它，这一屏只写得出「服务费已结算」那句兜底话。
/// 代价是这条流水没有对应的现金变动 —— 它不参与任何审计（那是 `tests/` 的事），
/// 而这个旋钮**只在开发时打开**。
pub(crate) fn dev_open_service(
    order: Option<Res<TurnOrder>>,
    active: Option<Res<ActiveBoard>>,
    faces: Query<&PlayerId>,
    mut lands: Query<(&LandId, &mut Owner)>,
    mut pending: ResMut<PendingDecision>,
    mut charged: MessageWriter<ServiceCharged>,
    mut done: Local<bool>,
) {
    if *done || pending.active().is_some() {
        return;
    }
    let (Some(order), Some(active)) = (order, active) else {
        return;
    };
    // 地主与来客各挑一个：收据那张纸上要写两个名字
    let (Some(owner), Some(guest)) = (order.order.first().copied(), order.order.get(1).copied())
    else {
        return;
    };
    let Ok(owner_id) = faces.get(owner) else {
        return;
    };
    let Some(def) = active
        .def
        .lands
        .iter()
        .find(|l| l.land_type == LandType::Commercial)
    else {
        return;
    };
    let land = LandId(def.id);
    for (id, mut holder) in &mut lands {
        if *id == land {
            holder.0 = Some(*owner_id);
        }
    }
    // 摇出 3 → A 型第 3 档。数写死在这里是有意的：这一屏要验的是「读得出怎么算的」
    charged.write(ServiceCharged {
        payer: guest,
        owner,
        land,
        kind: BuildingKind::Mall,
        amount: 600,
        roll: 3,
        stay_days: 0,
    });
    pending.offer(guest, Pending::ServiceNotice(land));
    *done = true;
}

/// 开发用：`NM_COMPANY=1` 挂出一张公司到访收据（董事长免单那种，0 元那张最有说的）。
///
/// 为什么需要它：与 [`dev_open_service`] 同一条判据 —— 收据的正经入口是踩到公司门面格，
/// 那一掷由骰子说了算，而 `NM_AUTOROLL` 的自动玩家下一帧就把它答掉，截图按墙钟撞不上。
///
/// 它发一条**真凭据**（`CompanyCharged`，走 [`remember_company`] 那条正路）。董事长
/// 指到第二位玩家，收据上就能写出「董事长：孙小美」与「0 元」这对兄弟 —— 免单那张
/// 才是这一屏最要紧的可读数。
pub(crate) fn dev_open_company(
    order: Option<Res<TurnOrder>>,
    faces: Query<&PlayerId>,
    mut pending: ResMut<PendingDecision>,
    mut charged: MessageWriter<CompanyCharged>,
    mut done: Local<bool>,
) {
    if *done || pending.active().is_some() {
        return;
    }
    let Some(order) = order else {
        return;
    };
    let (Some(guest), Some(chairman)) = (order.order.first().copied(), order.order.get(1).copied())
    else {
        return;
    };
    if faces.get(guest).is_err() {
        return;
    }
    // stock_001 = 暴富科技（演示盘 10 号格），amount 0 = 董事长免单那张
    charged.write(CompanyCharged {
        payer: guest,
        stock: "stock_001".to_string(),
        chairman: Some(chairman),
        amount: 0,
        roll: 0,
        stay_days: 0,
    });
    pending.offer(guest, Pending::CompanyNotice);
    *done = true;
}

/// 开发用：`NM_SHOP=1` 挂出百货柜台，并把「进得了店」需要的两样备齐 —— 一次换货与一笔点券。
/// 理由与 `NM_ATM`/`NM_HAND` 同一条：这块面板的正经入口是踩到那一格，而那一掷由骰子说了算，
/// 截图脚本控制不了（判据：每个界面都得有一条不靠鼠标的入口）。
///
/// 与 [`force_decision`] 分成两条 system 是有意的：那条的契约是「只写 [`PendingDecision`]」，
/// 而这里还要发点券与换货 —— 那是规则层的账，不该塞进那条开关的语义里。
///
/// 货架与点券由 `Local` 保证只补一次（同 `dev_deal_sample`），代价是同一个进程里开第二局
/// 不会重新摆货 —— 它是截图旋钮，不是跑完一局的手段（要跑一局单开 `NM_AUTOROLL`，
/// 而这一格会被自动玩家按「离开」走过）。
pub(crate) fn dev_open_shop(
    order: Option<Res<TurnOrder>>,
    mut pending: ResMut<PendingDecision>,
    mut books: Books,
    shelf: Shelf<'_>,
    mut stock: ResMut<ShopStock>,
    mut rng: ResMut<Rng>,
    mut stocked: Local<bool>,
) {
    let Some(player) = order.as_ref().and_then(|o| o.current()) else {
        return;
    };
    if !*stocked {
        // 100 点券：够买走货架上最贵的一张（`item_cards.json` 里 45 封顶），
        // 又让「点券不够」那颗灰按钮在买过两三次之后拍得到
        books.award_points(player, 100);
        stock.slots = restock(
            shelf.items().len(),
            books.rules.shop_slots as usize,
            &mut rng,
        );
        *stocked = true;
    }
    if pending.active().is_none() {
        pending.offer(player, Pending::Shop);
    }
}

/// 开发用：`NM_PLOTS=<n>` 把棋盘上前 n 块地划给当前回合玩家（Lv1、未抵押）。
///
/// 为什么需要它：银行柜台新长出的那一节「你的地产」要有地块才拍得到，而「名下有地」在
/// 正经玩法里要靠踩中一块无主地再回答「买」—— 那一掷同样由骰子说了算（判据同 `NM_SHOP`：
/// 每个界面都得有一条不靠鼠标的入口）。配 `NM_DECISION=bank` 一起用就是柜台那一屏。
///
/// **不走 [`Books::buy`]**：那条要扣现金、还要有 `Pending::Buy` 的报价在前，而这个旋钮只想
/// 改归属。于是它也不发 `LandBought` —— 它是世界状态的前置，不是一笔交易，硬套交易入口
/// 反倒会让「现金审计」凭空多出一笔没花出去的钱。
///
/// 与 `dev_open_shop` 同一条代价：`Local` 保证只划一次，同一进程里开第二局不会重新划。
pub(crate) fn dev_grant_plots(
    mut commands: Commands,
    order: Option<Res<TurnOrder>>,
    active: Option<Res<ActiveBoard>>,
    faces: Query<&PlayerId>,
    mut granted: Local<bool>,
) {
    if *granted {
        return;
    }
    let want = match std::env::var("NM_PLOTS") {
        Ok(spec) => match spec.trim().parse::<usize>() {
            Ok(n) => n,
            Err(_) => {
                warn!("NM_PLOTS={spec} 不是个数，应为 NM_PLOTS=2");
                *granted = true;
                return;
            }
        },
        // 没设这个旋钮：什么都不做，也**不**置 `granted` —— 它是每帧查环境变量的开关，
        // 与 `dev_open_shop` 一样只在设了的时候才有事
        Err(_) => return,
    };
    let (Some(player), Some(active)) = (order.as_ref().and_then(|o| o.current()), active.as_ref())
    else {
        return;
    };
    let Ok(pid) = faces.get(player) else {
        return;
    };
    *granted = true;
    // 按 `LandId` 取前 n 块：`HashMap` 的迭代序不稳定，不排序的话每次跑划到的地会换
    let mut plots: Vec<LandId> = active.lands.keys().copied().collect();
    plots.sort_unstable_by_key(|id| id.0);
    for land in plots.into_iter().take(want) {
        let Some(entity) = active.lands.get(&land).copied() else {
            continue;
        };
        // Lv1 = 「刚购入即 1 级」（`Books::buy` 同一条），留成空地拍出来的读数最干净
        commands
            .entity(entity)
            .insert((Owner(Some(*pid)), BuildingLevel(1)));
    }
    info!("NM_PLOTS：把 {want} 块地划给玩家 {pid:?}（只为截图，未走交易入口）");
}

/// 卡面效果行。规则层只发凭据、不拼文案（`msg.rs` 的注释），字符串在这里生成。
fn outcome_line(outcomes: &[EffectOutcome]) -> String {
    let parts: Vec<String> = outcomes.iter().map(describe).collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("效果：{}", parts.join("；"))
}

/// 拘留那一格的人话：有钟的写「剩 N 回合」，笼中的写「等保释」。
///
/// 顶栏、dump、保释窗口三处要说同一句话，所以这一句的映射只有一份。写成两句而不是
/// 「剩 0 回合」：0 在回合机上意思是「现在就放人」，而那正是笼中那一条**不会**发生的事。
pub fn detention_phrase(held: &Detention) -> String {
    match held.term.turns() {
        Some(n) => format!("{}剩{}回合", held.place.label(), n),
        None => format!("{}·等保释", held.place.label()),
    }
}

pub(crate) fn describe(outcome: &EffectOutcome) -> String {
    match outcome {
        EffectOutcome::Gained(amount) => format!("获得 ${amount}"),
        EffectOutcome::Lost(amount) => format!("支付 ${amount}"),
        EffectOutcome::CouldNotPay { needed } => format!("现金不足，欠 ${needed}"),
        EffectOutcome::Detained { place, turns, .. } => {
            format!("被关进{}（{turns} 回合）", place.label())
        }
        // 与上面那条是一对（P11b② 的新闻「医院传出好消息」）。用的是 `back_verb` 而不是
        // 「放出」：出狱／出院／回国／退房是四句话，而它们在模型里是同一条凭据 ——
        // 那句人话的映射已经有一份，别再抄第二份
        EffectOutcome::Released { place, .. } => format!("提前{}", place.back_verb()),
        EffectOutcome::Teleported { name, .. } => format!("移动到「{name}」"),
        // 拍卖那一槌（卡 P9 / 新闻「公开拍卖公有土地」P11c①）。卡面上不写价钱也不写谁会拍走 ——
        // 那两个数在 `AuctionStarted` 与 `AuctionWon` 上，而这条凭据要说的是
        // 「这一槌敲下去了、拍的是哪几块地」。零块是一句要说得出的人话：那条新闻播了，
        // 而这一带的地全都有主（不写「0 块」那种读数，玩家会去找那块被拍走的地）
        EffectOutcome::Auctioned { lands } => match lands.len() {
            0 => "这一带没有可拍的公有地".to_string(),
            1 => format!("把 L{} 强制上架", lands[0].0),
            n => format!("把 {n} 块公有地挂上拍卖台"),
        },
        // 数据写了个棋盘上不存在的格名：如实报出来，别退成一句漂亮话把数据错误藏起来
        EffectOutcome::UnknownTile { name } => format!("数据错误：找不到「{name}」"),
        // 表上那一行的人话（`PassiveEvent::label`）就是这句话的内容，所以 UI 不再自己
        // 摆一张「族 → 词」的映射 —— 那是 P13b① 删掉的第一份冗余：五行词与表上十五行
        // 各有一份答案，而加一行时只有表会被编译器逼着表态。
        // 「被谁挡的」由 P13b② 的门带上来（[`Blocker`]）：**卡挡的要花一张卡、神挡的分文不花**，
        // 两句混成一条「被抵消」就等于替玩家宣布少了一张卡（或少了一层永不消耗的免疫）
        EffectOutcome::Blocked { event, by } => match by {
            Blocker::Card => format!("一次{}被免罪卡抵消", event.label()),
            Blocker::God(god) => format!("一次{}被{}豁免", event.label(), god.label()),
        },
        // 块数要说：「整街」在缺路段键的盘上可能只盖到一块，玩家有权知道自己办成了多大一片
        EffectOutcome::Stamped {
            status,
            lands,
            days,
        } => format!("{} {lands} 块地 {days} 天", status.label()),
        // 与 `Stamped` 同一条：块数要说，而**哪几块**在新闻那句里由 `NewsAudience::Ground`
        // 那份清单给出（这条凭据只说「办成了多大一片」，它不带地块名字）。
        // 「收回几块」也要单独念：拆房与夺地是阶梯上两个格子，只看块数分不出来
        EffectOutcome::Razed { lands, seized } => format!(
            "夷为平地 {lands} 块{}",
            if *seized > 0 {
                format!("（土地收回 {seized} 块）")
            } else {
                String::new()
            }
        ),
        // 打出去那一刻**什么都不发生**，所以这一句必须说清「从今天起可以免一次」——
        // 否则玩家以为自己按错了。门槛要连着物价一起写，因为「现在值不值得用」正是这张卡
        // 唯一需要判断的地方
        EffectOutcome::FreePassArmed { floor } => {
            format!("存下一次免付（超过 {floor}×物价 的账才免）")
        }
        // 地块名字不在凭据里（`LandId` 只是个下标），而这一句是打在「刚按下去的那颗按钮」
        // 对应的卡面上 —— 玩家刚选的就是这块地，这里要说的是「付了多少」，那是唯一会变数的数
        EffectOutcome::Purchased { price, .. } => format!("买下这块地，付 ${price}"),
        // 说的是翻完之后朝哪边走，不是「翻了一次」：玩家下一手要用的就是这个数
        EffectOutcome::Reversed { heading } => format!("掉头，改为{}绕行", heading.label()),
        // 要说「原地」而不是「不能动」：他还在自己踩到的那一格上，地也照样收租
        EffectOutcome::Stayed { turns } => format!("原地停留（{turns} 回合）"),
        // 与停留是两句话：还在这条路上走，只是走不远
        EffectOutcome::Slowed { turns } => format!("接下来 {turns} 回合只走 1 格"),
        // 请来的那位是谁、从哪一格、离几步 —— 三个都要写：这张卡的赌性就在「由不得你挑」，
        // 而「最近」是玩家事后可以核对的一句话（不写格号就没法核对）
        EffectOutcome::Summoned { god, from, steps } => format!(
            "请来{}（他从 {} 号格、离你 {steps} 步的位置过来）",
            god.label(),
            from.0
        ),
        EffectOutcome::Banished { god, bomb } => {
            let sent = format!("送走{}", god.label());
            // 炸弹那一半不写就等于瞒着玩家：他刚把一样要命的东西和一位神一起丢了
            if *bomb {
                format!("{sent}，身上的定时炸弹也一起送走")
            } else {
                sent
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{LotteryDeny, LotteryDesk};

    /// 柜台前那位口袋里有钱、号码池是默认的 36 个、一注 1000。
    /// 面板这一侧的判据全部由 `LotteryDesk` 的字段决定，所以构造它不需要世界。
    fn desk(taken: Vec<(u8, String)>) -> LotteryDesk {
        LotteryDesk {
            cash: 20_000,
            price: 1_000,
            numbers: 36,
            held: Vec::new(),
            taken,
            pot: 1_000,
            last_draw: None,
        }
    }

    /// 别人占走的号：那颗按钮灰着、那句话**来自规则层那张拒绝表**（这一格是「判据只写一次」
    /// 在表现层的落点 —— 面板断到的就是柜台真会拒的），而「是谁占的」写在方格阵下面那一行。
    /// `one_reason` 在这里必须**不**把那句话当成全池通用的一句：一格一个人，而其余三十五颗还亮着。
    #[test]
    fn a_taken_number_is_grey_while_the_rest_stay_lit() {
        let d = desk(vec![(7, "姬百年".into())]);
        let rows = d.rows();
        assert_eq!(rows[0].1, None, "1 号没人占，该是亮的");
        assert_eq!(
            rows[6].1.map(LotteryDeny::label),
            Some("这个号已经被别人买走了"),
            "7 号该灰着并说出为什么"
        );
        assert_eq!(d.one_reason(), None, "还有亮的号 → 没有全池共用的一句");
        let line = d.taken_line().expect("被占走的号要单独成行");
        assert!(
            line.contains('7') && line.contains("姬百年"),
            "那一行要说得出是哪一颗、被谁：{line}"
        );
    }

    /// 独占之后「现金不够」那一行**仍然要说得出**：`one_reason` 对 `Taken` 是 `continue`
    /// 而不是 `return None`。写成后者会同时弄丢两句话 —— 那既不是「一句通用的话」也说不清
    /// 那两颗被点名的号，玩家看到的就是一整片灰按钮与一句沉默（§6.1 坑 27 的 UI 版）。
    #[test]
    fn broke_and_taken_still_read_as_one_sentence_for_the_untaken_numbers() {
        let mut d = desk(vec![(7, "姬百年".into()), (8, "范一文".into())]);
        d.cash = 999;
        assert_eq!(
            d.one_reason(),
            Some("现金不够"),
            "没有主的那些号，理由只有一个"
        );
        let rows = d.rows();
        for n in [6, 7] {
            assert_eq!(
                rows[n].1.map(LotteryDeny::label),
                Some("这个号已经被别人买走了"),
                "{} 号被占着，它不进上面那一行",
                n + 1
            );
        }
        assert_eq!(
            d.taken_line().unwrap().matches('（').count(),
            2,
            "两个号两个人，都得点名"
        );
    }

    /// 一个号都没被占时那一行**整条不写**（百货那次「空着的解释行读起来像面板坏了」同一课）。
    #[test]
    fn an_empty_take_list_says_nothing() {
        assert!(desk(Vec::new()).taken_line().is_none());
    }
}
