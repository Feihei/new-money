//! 公司企业（P10d①）：不可拥有的固定格 + 绑定股票 + 当月损益账。
//!
//! PLAN §三 那行的设计指令：公司是「位置固定的实体建筑，不能买地，只能通过股市持股；
//! 持股最多者自动当董事长 → 本人踩格免费、他人付费」。落地成三件事：
//!
//! - **格子只是门面**。`TileKind::Company` 绑着一只股票（`TileData.stock`），
//!   持股、董事长、损益账全挂在股票上 —— 「公司的地是哪块」没有答案，所以它
//!   **不走** `land_id`/`building_kind` 那条路。10-20 实报的口径：分红分的是
//!   「该公司当月真账所得」，亏损月份是负数、按比例扣持股玩家的现金 ——
//!   因此本模块的本体是 [`CompanyLedgers`] 这本账，而不是「股东名单」：
//!   谁持多少股账本里现查（[`Books::holdings_of`]），抄一份名单就是第二份真相。
//! - **收费走原型库**（[`crate::service`]）：B 型（航空转盘，出国 N 天）与
//!   C 型（电脑维修费，按骰面步数）与商业设施完全同形 —— 「同一原型换两个参数
//!   得出两种载体」是 P10 的真验收，这里若再长一层皮就多一处会分叉的地方。
//!   收的仍是 [`crate::charge::ChargeKind::Service`]（归 Rent 域），免费卡、
//!   嫁祸卡、物价指数三条既有规则不改一行就吃得到它。
//! - **钱进公司账而不是谁的口袋**：[`Charge`] 的 `payee` 填 `None` —— 公司是
//!   外部对手方（与银行收罚金同一形），「公司有钱包」在持股模型里没有位置。
//!
//! 与 `service.rs` 模块头那句话对齐的一处分家：**「董事长免费」不是打折 ——
//! 是根本不产生这笔账**（与步行踩加油站同一格）：免单要过管线、留「被谁免掉」的
//! 流水，而董事长踩自己公司连流水都不该有 —— 他不是逃掉一笔债，是没这回事。
//!
//! 分红在 [`pay_company_dividends`]：分红日（15 号）把当月净额按持股比例摊给
//! 持股人，正数进钱包、负数扣现金（付不出当场破产，与租金同一条判据）。
//! 没有公司账的股票（盘上没有那家门面）回落到市场旧的「股价 × dividend_rate」
//! —— 兜底公式住在 [`crate::market`]，两边以「账本里有没有这一页」分家。
//!
//! P10d② 在这格骨架上长出三样，全挂在「董事长 = 持股最多者」这一条身份上：
//!
//! - **人寿保险**（[`CompanyFee::Insure`]）：B 型的兄弟分支 —— 转盘摇出的不是
//!   离场天数而是投保天数，人原地不动、付保费换一张 [`crate::player::Insured`]
//!   保单；投保期间坐牢/住院由 [`insurance_claim_system`] 按天赔现金，账本记
//!   负数（分红日就是亏损月）。**人照旧被拘留** —— 保险赔的是钱不是自由，
//!   4 代口径里没有「坐牢豁免券」这一档。
//! - **日产建设**（无 `fee` 的 D 型格）：踩到开远程升级窗口（[`Pending::RemoteBuild`]），
//!   升级正路是 [`crate::settle::Books::upgrade`]，判据（垄断、组内最低层、未满级）
//!   一分不改；建设公司的董事长走 [`crate::settle::Books::upgrade_chairman`] ——
//!   同一条判据、不花钱、一次升两层。
//! - **银行融资**（`bank.rs`）：银行保险（[`FINANCE_STOCK`]）的董事长可挪用他人
//!   存款（不计息）、他人取款储备不足时由他兜底、持股被反超的瞬间融资被收回。
//!
//! 百货那条「白拿一件」也读这个身份（百货格绑着 `stock_004` 那只股），
//! 但它动的是货架与手牌，落在 [`crate::shop`]。

/// 演示盘口径：银行保险那家公司的股票 id —— 融资三件套挂它的董事长身上。
/// 正式棋盘（P10 收尾）应把「哪家股是银行」数据化，演示盘先用常量钉住。
pub const FINANCE_STOCK: &str = "stock_003";

/// 演示盘口径：日产建设那家公司的股票 id —— 远程升级的「免费升两层」挂它。
pub const BUILD_STOCK: &str = "stock_002";

use std::collections::BTreeMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::calendar::Date;
use crate::charge::{Charge, ChargeKind};
use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};
use crate::market::MarketClock;
use crate::msg::{
    CompanyCharged, CompanyPayout, IndemnityPaid, LandUpgraded, PlayerDetained, PolicyBought,
    RemoteBuildOffered, RemoteUpgradeRequested,
};
use crate::player::{DetentionPlace, Insured, Mount};
use crate::property::ServiceDesk;
use crate::service;
use crate::settle::Books;

/// 一笔公司账。`amount` **带符号**：服务费是正的进账，P10d② 的保险赔付与
/// 新闻罚款会写负数 —— 「亏损月是负数」从入账那一刻就是数据事实，不是分红时才补算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LedgerEntry {
    pub date: Date,
    pub amount: i64,
    /// 人话标签（dump 与顶栏读它）。`&'static str`：标签的集合是代码里数得清的那几种，
    /// 不值得为它开一个 enum 再写一遍 Display。
    pub why: &'static str,
}

/// 某家公司**当月**的那一页账。上一页在上个分红日已经结清（[`CompanyLedgers::close_month`]），
/// 所以每家公司同时最多只有一页 —— 「查三个月前的账」没有玩法问过这个问题。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MonthLedger {
    pub year: i32,
    pub month: u32,
    pub entries: Vec<LedgerEntry>,
}

impl MonthLedger {
    /// 当月净额。正 = 盈利（分红进股东钱包），负 = 亏损（按比例扣股东现金）。
    pub fn net(&self) -> i64 {
        self.entries.iter().map(|e| e.amount).sum()
    }
}

/// 全体公司的当月账。**按股票 id 一家一页**：公司门面格可以搬走（换棋盘），账跟着
/// 股票走 —— 「暴富科技」这个实体在不在盘上有门面，与它这个月收了多少服务费是两件事。
#[derive(Resource, Debug, Default)]
pub struct CompanyLedgers {
    months: Vec<(String, MonthLedger)>,
}

impl CompanyLedgers {
    /// 这一页（没有就开新页，跨月就翻新页）。入账唯一入口。
    pub fn current_mut(&mut self, stock: &str, date: Date) -> &mut MonthLedger {
        if !self.months.iter().any(|(s, _)| s == stock) {
            // 先按值查一遍再 push：借用检查器看不出「find 的借用已经结束」，
            // match 两臂各自可变借用会让它当成同时借了两份
            self.months.push((stock.to_owned(), MonthLedger::default()));
        }
        let (_, slot) = self
            .months
            .iter_mut()
            .find(|(s, _)| s == stock)
            .expect("上面刚保证过这一页存在");
        // 跨月了就翻新的一页：旧账已在上个分红日结清，「跨月不清」会让上月的服务费
        // 混进这个月的分红 —— 那是 4 代股东们会当场对出来的数
        if slot.year != date.year || slot.month != date.month {
            *slot = MonthLedger {
                year: date.year,
                month: date.month,
                entries: Vec::new(),
            };
        }
        slot
    }

    /// 查账（dump 与测试用）。`None` = 这家公司从没收过一分钱 —— 分红走市场兜底。
    pub fn find(&self, stock: &str) -> Option<&MonthLedger> {
        self.months.iter().find(|(s, _)| s == stock).map(|(_, l)| l)
    }

    /// 结账：返回 `(年, 月, 净额)` 并清空这一页。一期一结 —— 净额摊不到一股的余数
    /// 留在被清掉的那一页里（与乐透奖池的「余数留池」同一口径，只是这里没有下页可带）。
    pub fn close_month(&mut self, stock: &str) -> Option<(i32, u32, i64)> {
        let slot = self.months.iter_mut().find(|(s, _)| s == stock)?;
        let net = slot.1.net();
        let (year, month) = (slot.1.year, slot.1.month);
        slot.1.entries.clear();
        Some((year, month, net))
    }
}

/// 持股最多者自动当董事长。**破产者不算**（他已离场，[`Books::solvent`] 把他筛掉了）；
/// 同股数时先到者留任 —— 「反超才换人」是这条规则的操作化，`>=` 把并列挡在门外。
///
/// 每次踩格现算而不缓存：董事长是持股数的**导出值**，缓存它就要在每条买卖股的路径上
/// 想起去刷新 —— 「反超了却还免着旧董事长的单」正是那种不崩、只让玩家觉得规则是假的错。
pub fn chairman_of(books: &Books, stock: &str) -> Option<Entity> {
    let mut best: Option<(u32, Entity)> = None;
    for (player, _) in books.solvent() {
        let shares = books
            .holdings_of(player)
            .iter()
            .find(|(id, _)| id.0 == stock)
            .map_or(0, |(_, n)| *n);
        if shares == 0 {
            continue;
        }
        match best {
            Some((top, _)) if top >= shares => {}
            _ => best = Some((shares, player)),
        }
    }
    best.map(|(_, e)| e)
}

/// 公司结算那张台子：日历钟、损益账与收据是同一件事的三个侧面。打包成参数包是
/// 房规（§六 坑 8）—— 散装就把 [`settle_company_landing`] 顶到 clippy 的参数上限，
/// 而那条警告在这里正是要吃掉「一条 system 要挂三样公司私产」这个信号。
#[derive(SystemParam)]
pub struct CompanyDesk<'w> {
    /// 入账日期的来源（损益账按自然月分页）。
    pub clock: Res<'w, MarketClock>,
    /// 全体公司的当月账。
    pub ledgers: ResMut<'w, CompanyLedgers>,
    /// 踩格收据（含董事长免单那张 0 元的）。
    pub charged: MessageWriter<'w, CompanyCharged>,
    /// 投保凭据（P10d②）：保费付了、保单生效了。
    pub policies: MessageWriter<'w, PolicyBought>,
    /// 远程建设窗口报到（P10d②，日产建设）。
    pub offered: MessageWriter<'w, RemoteBuildOffered>,
}

/// 踩到公司门面格：董事长免费（连账都不产生），其余人按原型收一笔进公司账。
///
/// 与 `property.rs` 的 `settle_service` 同形（摇数 → assess → settle → 收据），差别有三：
/// 收款方是 `None`（钱进账本不进口袋）、B 型送去 [`DetentionPlace::Away`]（出国旅游
/// 那个地方不在棋盘上）、结算完**记一笔账**（分红按它分）。
pub fn settle_company_landing(
    mut commands: Commands,
    mut books: Books,
    landing: Res<Landing>,
    mut pending: ResMut<PendingDecision>,
    mut desk: ServiceDesk<'_, '_>,
    mut company: CompanyDesk<'_>,
) {
    let Some(Landed {
        player,
        tile,
        domain: LandingDomain::Company,
        ..
    }) = landing.0
    else {
        return;
    };
    let Some(tile) = books.active.def.tile(tile.0 as i64) else {
        return;
    };
    // `validate` 保证公司格必带 stock；这里用 `?` 式退路而不是 expect：坏数据
    // 表现为一格白踩（与 Unimplemented 同一待遇），不该 panic 一局对局
    let Some(stock) = tile.data.stock.clone() else {
        return;
    };
    let date = crate::calendar::date_of(company.clock.tick, books.session.epoch);
    // D 型（日产建设，P10d②）：没有 fee —— 踩到不收费，只开远程升级窗口。
    // 窗口对谁都开（**包括董事长**：他的「免费升两层」就发生在这一格里，
    // 排在下面那条免单短路之后的话，特权在结构上就按不出来）；
    // 窗口里能升谁的地，判据全在 [`remote_build_system`]。
    let Some(fee) = tile.data.fee.as_ref() else {
        company.offered.write(RemoteBuildOffered { player });
        pending.offer(player, Pending::RemoteBuild);
        return;
    };
    let chairman = chairman_of(&books, &stock);
    if chairman == Some(player) {
        // 董事长踩自己公司：没这回事（见模块头）。收据照样弹 —— 面板得说得出
        // 「你是董事长，这一趟免费」，与免单要看得见「为什么没付钱」同一条理由
        company.charged.write(CompanyCharged {
            payer: player,
            stock,
            chairman,
            amount: 0,
            roll: 0,
            stay_days: 0,
        });
        pending.offer(player, Pending::CompanyNotice);
        return;
    }
    // 保险（P10d②）：转盘摇出的不是离场天数而是投保天数 —— 人原地不动，付保费
    // 换一张保单（[`crate::player::Insured`]）；「被拘留时赔钱」的另一半在
    // [`insurance_claim_system`]。它不进 `service::assess`：那个原型的产物是
    // 「住几天」这种离场，保险的产物是身上一个状态，硬塞进去就是第二处会分叉的地方。
    if let crate::board::CompanyFee::Insure {
        sides,
        premium_per_day,
        indemnity_per_day,
    } = fee
    {
        let (sides, premium_per_day, indemnity_per_day) =
            (*sides, *premium_per_day, *indemnity_per_day);
        let days = if sides > 0 {
            desk.rng.pick_service(1..=sides)
        } else {
            0
        };
        let premium = premium_per_day * days as i64;
        let charge = Charge::new(ChargeKind::Service, Some(player), None, premium);
        let Ok(done) = books.settle(&mut commands, charge) else {
            return; // 保费付不起 → 破产已宣告；人都没了，保单无从谈起
        };
        if done.amount != 0 {
            company
                .ledgers
                .current_mut(&stock, date)
                .entries
                .push(LedgerEntry {
                    date,
                    amount: done.amount,
                    why: "保费",
                });
        }
        // 免单（免费卡把保费吃成 0）不发保单：保费一分没掏，保单就是凭空印的。
        // 与上面「免单不记账」同一口径 —— 公司没收着钱，就不欠任何保障
        if done.amount > 0 {
            commands.entity(player).insert(Insured {
                days_left: days,
                insurer: stock.clone(),
                indemnity_per_day,
            });
            company.policies.write(PolicyBought {
                player,
                stock: stock.clone(),
                days,
                premium: done.amount,
            });
        }
        company.charged.write(CompanyCharged {
            payer: done.payer.unwrap_or(player),
            stock,
            chairman,
            amount: done.amount,
            roll: days,
            stay_days: 0,
        });
        pending.offer(player, Pending::CompanyNotice);
        return;
    }
    let proto = fee
        .to_prototype()
        .expect("validate 已保证 company 格的 fee 只剩 B/C 两型");
    // 摇数走服务子流（P10 立的那条）：SERVICE_TAG 的注释本来就点着「航空转盘停几段」，
    // 公司摇数与商业设施摇数是同一个域 —— 再开第十五条流反而是把一句话拆成两句
    let roll = if proto.sides() > 0 {
        desk.rng.pick_service(1..=proto.sides())
    } else {
        0
    };
    let steps = desk
        .last_roll
        .0
        .filter(|(who, _)| *who == player)
        .map_or(0, |(_, steps)| steps);
    let mount = desk.mounts.get(player).copied().unwrap_or(Mount::Foot);
    let Some(bill) = service::assess(&proto, roll, steps, mount) else {
        return; // C 型 + 步行 + requires_mount：没这回事（与加油站同一格）
    };
    // payee = None：公司是外部对手方，钱离开钱包进了它的账
    let charge = Charge::new(ChargeKind::Service, Some(player), None, bill.base);
    let Ok(done) = books.settle(&mut commands, charge) else {
        return; // 付不起 → 破产已由 settle 宣告；人都没了，账与出国都无从谈起
    };
    // 记真账：过完管线的实付额。免单（免费卡吃成 0）不记 —— 公司没收着钱，
    // 记 0 只会让分红日多算一笔空账
    if done.amount != 0 {
        company
            .ledgers
            .current_mut(&stock, date)
            .entries
            .push(LedgerEntry {
                date,
                amount: done.amount,
                why: "服务费",
            });
    }
    if bill.stay_days > 0 {
        // 航空：出国旅游。`Away` 没有死胡同分支，接入点就是他此刻那一格（P4① 的试金石）
        if let Some((sentence, junction, end)) = crate::landing::detain(
            &mut commands,
            &mut desk.positions,
            &mut books.detained,
            &books.active.def,
            player,
            DetentionPlace::Away,
            bill.stay_days,
        ) {
            desk.detained_note.write(PlayerDetained {
                player,
                place: sentence.place,
                junction,
                end,
            });
        }
    }
    company.charged.write(CompanyCharged {
        payer: done.payer.unwrap_or(player),
        stock,
        chairman,
        amount: done.amount,
        roll,
        stay_days: bill.stay_days,
    });
    pending.offer(player, Pending::CompanyNotice);
}

/// 保险赔付（P10d②）：投保人在保单效期内被拘留，保险公司按天赔现金。
///
/// 盯 `PlayerDetained` 而不是 `Added<Detention>`：四条拘留路径（落格、命运卡、
/// 放置物、公司 B 型）全都汇到同一条消息，在这里拦一次就全接上了；而
/// `Added<Detention>` 会在**回合跳过的重插**（`begin_turn_system` 把 `turns_left - 1`
/// 写回去）时再响一次，把「坐牢 3 天赔 3 天」变成每年白送一轮。拘留天数读的是
/// [`Detention`] 组件此刻那一份 —— 刚被 `landing::detain` 写满的那份。
///
/// 赔付是两件事同时做：公司账记**负数**（[`LedgerEntry`]，「亏损月是负数」从
/// 入账就是事实），玩家 [`Books::credit`] 现金（`CashChanged` 进流水）。保单
/// **不因此作废**：效期还剩几天照旧走表 —— 这是一段时期的保障，不是一次性赔案；
/// 效期内再被拘留，只要组件还在就继续赔。
pub fn insurance_claim_system(
    mut books: Books,
    mut desk: CompanyDesk<'_>,
    mut detained: MessageReader<PlayerDetained>,
    holds: Query<&'static Insured>,
    mut paid: MessageWriter<'_, IndemnityPaid>,
) {
    for notice in detained.read() {
        // 拘留天数从账本那条查询里读（`Books::detained` 是全工程唯一的 `&mut
        // Detention`，自己再开一条读查询会在参数建立期撞成 B0001 —— P4② 的坑）
        let Ok(days) = books.detained.get(notice.player).map(|d| d.turns_left) else {
            continue; // 消息到了组件已被放出来（同帧出狱）：没有可赔的天数
        };
        let Ok(policy) = holds.get(notice.player) else {
            continue; // 没保单：保险与他无关，消息读掉就丢
        };
        let amount = policy.indemnity_per_day * days as i64;
        let date = crate::calendar::date_of(desk.clock.tick, books.session.epoch);
        desk.ledgers
            .current_mut(&policy.insurer, date)
            .entries
            .push(LedgerEntry {
                date,
                amount: -amount,
                why: "保险赔付",
            });
        books.credit(notice.player, amount);
        paid.write(IndemnityPaid {
            player: notice.player,
            stock: policy.insurer.clone(),
            days,
            amount,
        });
    }
}

/// 保单效期逐回合扣表（P10d②）：只扣**当前回合玩家**的那一份 —— 一天 = 一个
/// 自己的回合，与拘留/停留同一把尺（`player::Insured` 模块注）。
///
/// 不并进 `turn.rs` 的 `Sentences` 参数包：那一份挂着拘留与停留两条 writers，
/// 每加一样都要过一遍「漏注册 = 回合机不跑」的雷区；保单到期是**静默的**
/// （HUD 读组件、没有凭据消息），一条独立的小 system 更便宜也更难接错。
pub fn tick_insurance_system(
    order: Res<crate::turn::TurnOrder>,
    mut commands: Commands,
    mut policies: Query<&mut Insured>,
) {
    let Some(current) = order.current() else {
        return;
    };
    if let Ok(mut policy) = policies.get_mut(current) {
        policy.days_left = policy.days_left.saturating_sub(1);
        if policy.days_left == 0 {
            commands.entity(current).remove::<Insured>();
        }
    }
}

/// 远程升级柜台（P10d②，日产建设）：`RemoteUpgradeRequested` 逐笔进来办，
/// 窗口照开 —— 交还回合是 `decision_system` 对「先不建」的回应（与
/// `property::build_op_system` 同一条纪律）。
///
/// 判据与扣款全在 [`crate::settle::Books::upgrade`]（正路：垄断、组内最低层、
/// 未满级、付得起）；唯一分叉是建设公司的董事长 —— 同一条判据换成
/// [`crate::settle::Books::upgrade_chairman`]（不花钱、一次升两层）。升不成的
/// 请求静默丢弃：面板该把办不动的按钮亮成灰的，而不是按下去没声音。
pub fn remote_build_system(
    mut books: Books,
    pending: ResMut<PendingDecision>,
    mut requests: MessageReader<RemoteUpgradeRequested>,
    mut upgraded: MessageWriter<LandUpgraded>,
) {
    let Some((asked, Pending::RemoteBuild)) = pending.active() else {
        return;
    };
    let chairman = chairman_of(&books, BUILD_STOCK);
    for r in requests.read() {
        if r.player != asked {
            continue;
        }
        let new_level = if chairman == Some(r.player) {
            books.upgrade_chairman(r.player, r.land)
        } else {
            books.upgrade(r.player, r.land)
        };
        if let Some(new_level) = new_level {
            upgraded.write(LandUpgraded {
                player: r.player,
                land: r.land,
                new_level,
            });
        }
    }
}

/// 分红日（15 号）把每家公司的当月净额按持股比例摊给持股人。
///
/// **正负同一条公式**（10-20 实报的口径）：盈利月 `per_share` 进钱包，亏损月按比例
/// 扣现金 —— 扣不出的人当场破产（[`Books::charge`] 的既判据），因为「分红欠着以后补」
/// 在持股模型里没有账可挂。`per_share = 净额 ÷ 总股数` 向零取整，摊不到一股（|净额|
/// 小于总股数）的那个月不发也不扣 —— 与乐透「余数留池」同一口径。
///
/// 只发「有公司账」的股票：盘上没有门面的那几只回落到市场的兜底公式（调用方
/// [`crate::market`] 负责那另一半），两边以 [`CompanyLedgers::find`] 有没有这一页分家。
pub fn pay_company_dividends(
    commands: &mut Commands,
    books: &mut Books,
    ledgers: &mut CompanyLedgers,
    date: Date,
    payouts: &mut MessageWriter<CompanyPayout>,
) {
    let rules = *books.rules;
    if !crate::calendar::is_dividend_day(date, rules.dividend_day) {
        return;
    }
    // 全体持股先收拢成一张小表再动账：迭代持仓时不该挂着 books 的可变借用
    // （与 market.rs 旧分红那半条同一纪律）
    let mut by_stock: BTreeMap<String, Vec<(Entity, u32)>> = BTreeMap::new();
    for (player, _) in books.solvent() {
        for (id, shares) in books.holdings_of(player) {
            if shares > 0 {
                by_stock.entry(id.0).or_default().push((player, shares));
            }
        }
    }
    // BTreeMap 的键序 = 股票 id 字典序：多家公司同日分红时，发放次序与持股人的
    // 座位序无关 —— 可复现性押在键序上，不押在 spawn 次序上
    for (stock, holders) in by_stock {
        let Some((_, _, profit)) = ledgers.close_month(&stock) else {
            continue; // 无公司账：市场兜底
        };
        if profit == 0 {
            continue;
        }
        let total_shares: u64 = holders.iter().map(|(_, n)| *n as u64).sum();
        let per_share = profit / total_shares as i64;
        if per_share == 0 {
            continue;
        }
        for (player, shares) in holders {
            let amount = per_share * shares as i64;
            if amount > 0 {
                books.credit(player, amount);
            } else if amount < 0 && !books.is_bankrupt(player) {
                // 亏损月扣现金。charge 的既判据：扣不出当场破产
                let _ = books.charge(commands, player, -amount);
            }
            payouts.write(CompanyPayout {
                stock: stock.clone(),
                player,
                shares,
                per_share,
                profit,
            });
        }
    }
}
