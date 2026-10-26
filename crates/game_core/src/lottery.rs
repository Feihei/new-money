//! 乐透（P8）：奖池、买券、每月 15 号开奖。
//!
//! 四代口径（PLAN §三「乐透」行）：1000 元/注、1–36 选一号、一人可持多号、**一个号全场
//! 只有一张**（10-26 实报的号码独占，它才是「奖金必然某位玩家独得」的那条来路）、
//! 每月 15 号开奖。
//! 这一块真正的设计决定只有两条，都在「钱从哪来、到哪去」上：
//!
//! - **奖池只有两条来源**（10-22 拍板）：买券的钱**全额**进池 + 月末利息滚存。
//!   PLAN 原写的第三条「罚金／租金／破产变卖股款各抽比例进池」被这条换掉，理由不是省事：
//!   租金是有名有主的钱（房东收），抽它等于让房东少收，于是每一条 `RentDue` 都要额外说清
//!   「房东实收多少、池里进了多少」才审计得平 —— 那是把一个月度事件的成本摊到全盘每一笔
//!   租金上。真正「无主」的意外损失只有罚金那一种，而它一次也就几百块，池子因此会小得
//!   不值得开奖。买券钱 + 利息这两条自己就能滚：券是玩家主动买的，利息让「本月没人猜中」
//!   仍然有读数。
//! - **买券与发奖都不过结算管线**（[`crate::charge`]）。买券是账面搬运（同股票买卖与银行
//!   存取那一条边界：没有「减免」这回事 —— 免罪卡不该免掉彩票钱），发奖是池子给钱而不是
//!   谁欠的账单。而且池子里的钱已经是名义累计，再过一遍物价指数等于把通胀算两次。
//!
//! 一期一结：开奖那一刻只要**有人**猜中，全场所有人的券一起作废（不分中没中）—— 奖池被
//! 掏空了，留着老券等于让上一期白送下一期。反过来，**没人猜中**时池子与所有人的券都留着
//! （PLAN §三「号码留到下月」的操作化：你手上那几个号继续等它开出来）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::calendar::{date_of, is_dividend_day, is_month_end};
use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};
use crate::market::MarketClock;
use crate::msg::{
    BuyTicketRequested, JackpotChanged, JackpotWhy, LotteryBought, LotteryDrawn, LotteryWon,
};
use crate::player::PlayerId;
use crate::rng::Rng;
use crate::settle::Books;
use crate::turn::TurnPhase;

/// 奖池。它**不是**任何人的钱包，所以这里动的每一笔都发 [`JackpotChanged`] 而不是
/// [`crate::msg::CashChanged`] —— 现金审计据此把池子看成外部对手方（与银行同一条纪律）。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Jackpot {
    pub pot: i64,
    /// 最近一次开出的号。`None` = 本局还没开过奖。
    ///
    /// 留着它而不是只留池子余额，是因为「这个月开了几号」是玩家会回头问的事，
    /// 而它一旦只存在于顶栏那两三帧的播报里，就再也查不到了（`NM_DUMP` 要说得出话）。
    pub last_draw: Option<u8>,
}

/// 一个人手上这一期的彩票号。**一人可持多号**（4 代口径），而**一个号全场只有一张**
/// （10-26 实报的独占）：[`LotteryTickets::add`] 挡的是同一个人重复按同一个号，
/// 跨个人的那一半由 [`buy_allowed`] 的 [`LotteryDeny::Taken`] 挡。
///
/// 独占换来的东西就是这个模块的形状：中奖号**至多一位持有者**，于是「独得奖池」在结构上
/// 成立，不需要均分、不需要余数留池，而这两个分支也再也写不出来。
#[derive(Component, Debug, Default, Clone, PartialEq, Eq)]
pub struct LotteryTickets {
    pub numbers: Vec<u8>,
}

impl LotteryTickets {
    pub fn holds(&self, number: u8) -> bool {
        self.numbers.contains(&number)
    }

    /// 收下一张新号。已有同号时返回 `false` 且什么都不改。
    pub fn add(&mut self, number: u8) -> bool {
        if self.holds(number) {
            return false;
        }
        self.numbers.push(number);
        self.numbers.sort_unstable();
        true
    }
}

/// 这一注买不成的原因。灰按钮的文案与规则层的拒绝**共用这一张表**
/// （与 `shop::ShopDeny`、`mortgage::mortgage_op_allowed` 同一条纪律：判据只写一次）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LotteryDeny {
    /// 人不站在柜台前 —— 没踩到彩票格，或已经按了「离开」
    NoCounter,
    /// 号码不在 `1..=lottery_numbers` 上。写这一格而不是让越界静默通过，是因为号码是
    /// 从 UI 的按钮烤进请求里的：将来谁改那张表漏了一格，症状会是「凭空多出一个号」
    BadNumber,
    /// 这个号你已经有了
    AlreadyHeld,
    /// 这个号被**别人**占着（4 代的号码独占，10-26 实报）。它排在 `TooBroke` **之前**：
    /// 钱不够与号被占是两件事，而被占走的那个号即使有钱也买不成 —— 反过来写的话，
    /// 一个缺钱的人按到别人的号会得到一句与他该问的判断无关的「现金不够」。
    Taken,
    /// 现金不够
    TooBroke,
}

impl LotteryDeny {
    /// 每颗灰按钮都得带一句为什么，否则「按不动」与「坏了」在玩家眼里是同一件事
    /// （§6.1 坑 27 的 UI 版）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::NoCounter => "先踩到彩票中心",
            Self::BadNumber => "没有这个号",
            Self::AlreadyHeld => "这个号你已经买过了",
            Self::Taken => "这个号已经被别人买走了",
            Self::TooBroke => "现金不够",
        }
    }
}

/// 这一注此刻买不买得成。判据全部由调用方查好再传进来（同 `shop_op_allowed`），于是这张表
/// 既能被 system 用，也能在单测里逐格断言。
///
/// 顺序是刻意的：先问**窗口**，再问**号码本身**（在不在池里、归谁），最后才问钱。
/// 窗口不对时后面几条都不必查，而 UI 上也只有这一种情况需要把整块面板灰掉。
///
/// `taken` 是「这个号被**另一个还赢得到的人**占着」。这一格只数活着的人，与
/// [`draw_lottery_system`] 发奖时筛的是同一批人（[`Books::solvent`]）：破产者手里那张旧
/// 凭证既兑不出钱，也不该替全场锁死一个号码，否则「这一期的池子永远开不成」就是他离场
/// 留下的后遗症。两处读同一个 `is_bankrupt`，于是「一个号至多一位能中奖的持有者」这件事
/// 不会因为一个人死了而破。
pub fn buy_allowed(
    at_counter: bool,
    number: u32,
    numbers: u32,
    held: bool,
    taken: bool,
    cash: i64,
    price: i64,
) -> Result<(), LotteryDeny> {
    if !at_counter {
        return Err(LotteryDeny::NoCounter);
    }
    if number == 0 || number > numbers {
        return Err(LotteryDeny::BadNumber);
    }
    if held {
        return Err(LotteryDeny::AlreadyHeld);
    }
    if taken {
        return Err(LotteryDeny::Taken);
    }
    if cash < price {
        return Err(LotteryDeny::TooBroke);
    }
    Ok(())
}

/// 踩到彩票格：挂上柜台窗口。形状与 [`crate::shop::settle_shop_landing`] 同一条 ——
/// 不是一个报价，而是一个窗口，买券走 [`BuyTicketRequested`] 逐笔进来，回合停在
/// `WaitDecision` 直到玩家按「离开」。
pub fn settle_lottery_landing(landing: Res<Landing>, mut pending: ResMut<PendingDecision>) {
    let Some(Landed {
        player,
        domain: LandingDomain::Lottery,
        ..
    }) = landing.0
    else {
        return;
    };
    pending.offer(player, Pending::Lottery);
}

/// 买一注要动的全部东西。打包是房规（§六 坑 8）：钱、券、池子、窗口本来就是**一起成立
/// 或一起不成立**的，而散装到第九个参数时 clippy 的 `too_many_arguments` 会把
/// 「一个 system 要读这么多」这个信号吃掉。
#[derive(SystemParam)]
pub struct TicketDesk<'w, 's> {
    pub books: Books<'w, 's>,
    pub pending: Res<'w, PendingDecision>,
    pub phase: Res<'w, State<TurnPhase>>,
    pub jackpot: ResMut<'w, Jackpot>,
    /// 各人手里的号。带上 `Entity` 而不是只 `With<PlayerId>`：独占那一格要问的是
    /// 「**谁**占着这个号」，而 `Query::iter()` 交的是数据项本身、不给句柄。
    pub tickets: Query<'w, 's, (Entity, &'static mut LotteryTickets), With<PlayerId>>,
    pub bought: MessageWriter<'w, LotteryBought>,
    pub pot: MessageWriter<'w, JackpotChanged>,
}

impl<'w, 's> TicketDesk<'w, 's> {
    /// 这个号被**别的活着的人**占着没有 —— [`buy_allowed`] 那个 `taken` 位怎么查出来的。
    ///
    /// 这里刻意**不另立一张「号码 → 人」的表**：券本身就是唯一真相，占号的人就是手里
    /// 真有那张券的人。另立一张就多了第二份真相，而它失配的症状恰好是这条规则要防的那件事
    /// —— 两个人手里出现同一个号。
    fn taken(&self, player: Entity, number: u8) -> bool {
        self.tickets.iter().any(|(holder, tickets)| {
            holder != player && tickets.holds(number) && !self.books.is_bankrupt(holder)
        })
    }
}

/// 办一笔买券。**不改回合状态** —— 买券是玩家站在柜台前的自由动作，回合仍然由「离开」
/// 那一句交还（与 `shop_op_system`、`bank_op_system` 同一条 §4.6 镜像）。
///
/// 只认柜台前那个人：`Pending::Lottery` 挂的是谁，就花谁的钱。别人的点击不能替他买。
pub fn lottery_op_system(
    mut desk: TicketDesk<'_, '_>,
    mut requests: MessageReader<BuyTicketRequested>,
) {
    if matches!(desk.phase.get(), TurnPhase::Boot | TurnPhase::Over) {
        // 终局之后还在买券，回放与存档就对不上账了
        return;
    }
    let rules = *desk.books.rules;
    for r in requests.read() {
        let at_counter = desk.pending.active() == Some((r.player, Pending::Lottery));
        let held = desk
            .tickets
            .get(r.player)
            .is_ok_and(|(_, tickets)| tickets.holds(r.number));
        if let Err(why) = buy_allowed(
            at_counter,
            r.number as u32,
            rules.lottery_numbers,
            held,
            desk.taken(r.player, r.number),
            desk.books.balance(r.player),
            rules.ticket_price,
        ) {
            debug!(
                "乐透被拒：{:?} 想买 {} 号 —— {}",
                r.player,
                r.number,
                why.label()
            );
            continue;
        }
        buy(&mut desk, r.player, r.number, rules.ticket_price);
    }
}

/// 真正动手那一半。抽出来只为让「四道门都过了」与「扣钱、发券、进池」各自读得清楚。
///
/// 顺序与百货同一条纪律：**先扣钱**。[`Books::debit`] 不足额时一分都不扣，所以它排在
/// 发券与进池之前 —— 反过来写的话，钱没扣成而券已经到手、池子还多记了一笔。
fn buy(desk: &mut TicketDesk<'_, '_>, player: Entity, number: u8, price: i64) {
    if desk.books.debit(player, price).is_err() {
        return;
    }
    let Some((_, mut tickets)) = desk.tickets.get_mut(player).ok() else {
        // 没有这个组件的人买不了券（开局装配就带着它，这里只是不为不可能留分支）
        return;
    };
    if !tickets.add(number) {
        return;
    }
    desk.jackpot.pot += price;
    desk.bought.write(LotteryBought {
        player,
        number,
        price,
    });
    desk.pot.write(JackpotChanged {
        delta: price,
        balance: desk.jackpot.pot,
        why: JackpotWhy::Ticket,
    });
    info!(
        "{player:?} 买下一注 {number} 号，奖池 {} 元",
        desk.jackpot.pot
    );
}

/// 开奖与结息要动的东西。同样是参数包（§六 坑 8）。
#[derive(SystemParam)]
pub struct DrawDesk<'w, 's> {
    books: Books<'w, 's>,
    clock: Res<'w, MarketClock>,
    rng: ResMut<'w, Rng>,
    jackpot: ResMut<'w, Jackpot>,
    tickets: Query<'w, 's, &'static mut LotteryTickets, With<PlayerId>>,
    drawn: MessageWriter<'w, LotteryDrawn>,
    won: MessageWriter<'w, LotteryWon>,
    pot: MessageWriter<'w, JackpotChanged>,
}

/// 每月 15 号开奖（**读 `GameRules::dividend_day`，不新开一个日期旋钮** —— PLAN §四 P8 那句）。
///
/// 排在 `TurnEnd` 里、行情 tick **之后**：两者都要看「今天几号」，而 tick 就是在那一步推进的
/// （与 `apply_month_end_interest_system` 同一条顺序理由 —— 放在它前面会用昨天的日历判断
/// 今天该不该开奖，15 号那天正好错开）。
///
/// **中奖独得整池，没有均分**。这不是「实现了独得」而是「均分写不出来」：号码独占
/// （[`buy_allowed`] 的 `Taken`）保证一个号至多只有一位还活着的持有者，而这里筛的正是
/// 活着的那批 —— 所以那份名单在结构上只可能是空的或一个人的，`holders: Vec<_>` 与它的
/// 均分、余数留池一起换成了下面那一次 `find`。P8 当时那一套是替一个不该存在的分支写的
/// 代码，10-26 实报独占之后整条删掉，余数没有地方可留。
pub fn draw_lottery_system(mut desk: DrawDesk<'_, '_>) {
    let rules = *desk.books.rules;
    let today = date_of(desk.clock.tick, desk.books.session.epoch);
    if !is_dividend_day(today, rules.dividend_day) {
        return;
    }
    // 号码池被配成 0 时夹成 1：这个数一旦从设置或存档里读进来，`1..=0` 是一次当场崩掉的
    // 空区间，而它换来的只是「只有一个号」——与 `poor_cash_divisor` 同一条处理
    let number = desk.rng.pick_lottery(1..=rules.lottery_numbers.max(1)) as u8;
    let winner = desk
        .books
        .solvent()
        .into_iter()
        .map(|(player, _)| player)
        .find(|player| {
            desk.tickets
                .get(*player)
                .is_ok_and(|tickets| tickets.holds(number))
        });
    let pot = desk.jackpot.pot;

    if let Some(winner) = winner {
        // `credit` 对 `amount <= 0` 什么都不做，而 0 元也不该发一条「你赢了」的凭据 ——
        // 与「0 点券不发 PointsChanged」同一条（这一格只在 `ticket_price` 被配成 0 时碰得到）
        if pot > 0 {
            desk.books.credit(winner, pot);
            desk.won.write(LotteryWon {
                player: winner,
                number,
                amount: pot,
            });
            desk.jackpot.pot = 0;
            desk.pot.write(JackpotChanged {
                delta: -pot,
                balance: 0,
                why: JackpotWhy::PaidOut,
            });
        }
        // 一期一结：有人猜中就把**全场**的券清掉，不只是中奖那一张（模块头第三条决定）。
        // 清号同时也是独占的**换期**：这一期占着 7 号的人，下期不必继续占着它。
        for mut tickets in &mut desk.tickets {
            tickets.numbers.clear();
        }
    }
    desk.jackpot.last_draw = Some(number);
    desk.drawn.write(LotteryDrawn {
        number,
        winner,
        pot,
    });
    match winner {
        Some(winner) => {
            info!("乐透 {today} 开出 {number} 号：{winner:?} 独得整池 {pot} 元，全场券作废")
        }
        None => {
            info!("乐透 {today} 开出 {number} 号，没人猜中：奖池 {pot} 元与所有人的券都留到下月")
        }
    }
}

/// 月末给奖池结一次息（`GameRules::jackpot_rate`，与储金结息**同一条日子**）。
///
/// 这笔钱是凭空造的 —— 但它造在池子里而不是任何人的钱包上，所以 B3 的现金审计不受影响；
/// 它会经物价指数推高下一回合的租金（池子不是玩家资产，`total_assets` 读不到它），
/// 只有真正发奖那一刻才落到某个人的账上。
///
/// 0 元不发凭据：池子空着（本局第一个月）时每月发一条 `+0` 的流水，等于在审计里
/// 多写一行什么都没说的事。
pub fn accrue_jackpot_interest_system(mut desk: InterestDesk<'_, '_>) {
    let rules = *desk.books.rules;
    let today = date_of(desk.clock.tick, desk.books.session.epoch);
    if !is_month_end(today) {
        return;
    }
    let pot = desk.jackpot.pot;
    if pot <= 0 {
        return;
    }
    // 向下取整：利息不该因为四舍五入而凭空多出一块钱（与 `shop::buyback` 同一类整数纪律）
    let gain = (pot as f64 * rules.jackpot_rate as f64).floor() as i64;
    if gain <= 0 {
        return;
    }
    let prior = desk.jackpot.pot;
    desk.jackpot.pot += gain;
    desk.pot.write(JackpotChanged {
        delta: gain,
        balance: desk.jackpot.pot,
        why: JackpotWhy::Interest,
    });
    info!(
        "奖池月息 +{gain}（{today}），池子 {prior} → {}",
        desk.jackpot.pot
    );
}

/// 结息那一半要读的两样 + 要写的两样。与 [`DrawDesk`] 分开两个参数包而不是合成一个：
/// 它们读的是**两个日子**（15 号 vs 月末），合并就等于让一条 system 同时负责两件事，
/// 而 P2b 拍板时专门把派息与分红拆成两个日子，就是为了不被这样合回去。
#[derive(SystemParam)]
pub struct InterestDesk<'w, 's> {
    books: Books<'w, 's>,
    clock: Res<'w, MarketClock>,
    jackpot: ResMut<'w, Jackpot>,
    pot: MessageWriter<'w, JackpotChanged>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 号码的两种写法都要能被认出来：`1..=36` 是闭区间，越界与 0 都得拒。
    /// 这一格有牙是因为 `BadNumber` 排在 `TooBroke` **之前** —— 反过来写的话，
    /// 一个身无分文的人按 0 号会得到「现金不够」，而真正的原因是那个号不存在。
    ///
    /// `Taken` 同样排在钱之前，理由一样：别人占着的号，有钱也买不成。
    #[test]
    fn the_gate_asks_the_window_then_the_number_then_the_money() {
        let ok = |number| buy_allowed(true, number, 36, false, false, 5_000, 1_000);
        assert_eq!(ok(1), Ok(()));
        assert_eq!(ok(36), Ok(()));
        assert_eq!(ok(0), Err(LotteryDeny::BadNumber));
        assert_eq!(ok(37), Err(LotteryDeny::BadNumber));
        // 窗口不在时，号码与钱都不必问
        assert_eq!(
            buy_allowed(false, 99, 36, true, true, 0, 1_000),
            Err(LotteryDeny::NoCounter)
        );
        assert_eq!(
            buy_allowed(true, 7, 36, true, false, 5_000, 1_000),
            Err(LotteryDeny::AlreadyHeld)
        );
        assert_eq!(
            buy_allowed(true, 7, 36, false, true, 5_000, 1_000),
            Err(LotteryDeny::Taken)
        );
        // 一个又穷又被占的号：报的是「号没了」而不是「钱不够」
        assert_eq!(
            buy_allowed(true, 7, 36, false, true, 999, 1_000),
            Err(LotteryDeny::Taken)
        );
        assert_eq!(
            buy_allowed(true, 7, 36, false, false, 999, 1_000),
            Err(LotteryDeny::TooBroke)
        );
    }

    /// 同号一人一张：`add` 拒绝重复且**什么都不改**（不是「加了再排掉」那种自我安慰）。
    #[test]
    fn one_person_cannot_hold_the_same_number_twice() {
        let mut tickets = LotteryTickets::default();
        assert!(tickets.add(17));
        assert!(tickets.add(3));
        assert_eq!(tickets.numbers, vec![3, 17], "券要排好序，面板才念得顺");
        assert!(!tickets.add(17));
        assert_eq!(tickets.numbers, vec![3, 17]);
    }

    /// 每一格灰按钮都得有话可说 —— 一句空文案与「按不动但没理由」是同一个缺陷。
    /// 五格互不相同也是这一条：面板把理由写在方格阵下面那一行时，撞了就说不清是哪一种。
    #[test]
    fn every_denial_reads_as_a_sentence() {
        let mut labels: Vec<&str> = [
            LotteryDeny::NoCounter,
            LotteryDeny::BadNumber,
            LotteryDeny::AlreadyHeld,
            LotteryDeny::Taken,
            LotteryDeny::TooBroke,
        ]
        .iter()
        .map(|why| why.label())
        .collect();
        for why in labels.iter() {
            assert!(!why.is_empty(), "有的拒绝没有文案：{why:?}");
        }
        labels.sort_unstable();
        let all = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), all, "两种拒绝同名：{labels:?}");
    }

    /// 奖池那三格的文案互不相同：`NM_DUMP` 与顶栏都直接念它，撞了就没法说清一笔钱哪来的。
    #[test]
    fn the_pot_says_why_it_moved() {
        let mut labels: Vec<&str> = [
            JackpotWhy::Ticket,
            JackpotWhy::Interest,
            JackpotWhy::PaidOut,
        ]
        .iter()
        .map(|why| why.label())
        .collect();
        labels.sort_unstable();
        let all = labels.len();
        labels.dedup();
        assert_eq!(labels.len(), all, "两种来源同名：{labels:?}");
    }

    /// 默认值这一格看着像废话，它是**平衡三元组**的唯一现场：号码池、一注价钱、月息。
    /// 三个数散着改，「期望回报」这件事就没有任何一处能读出来。
    #[test]
    fn the_default_odds_are_a_negative_bet() {
        let rules = crate::rules::GameRules::default();
        assert_eq!(rules.lottery_numbers, 36);
        assert_eq!(rules.ticket_price, 1_000);
        // 一注的期望回报（不含利息）≈ 池子 / 号码数。池子只由买券滚成，所以**每一注的
        // 期望都是 1/36 的池子增长**，而买的人越多池子越大 —— 负期望这件事不需要一个
        // 抽头的比例，它由「36 个号里只中一个」本身给出。这里钉的是月息低于储金月息：
        // 池子不该比把钱正经存进银行更划算。
        assert!(
            rules.jackpot_rate < rules.deposit_rate,
            "奖池月息 {} 高过储金月息 {}，买券等息就成了无风险的存钱姿势",
            rules.jackpot_rate,
            rules.deposit_rate
        );
        // 开奖日与分红同一天是拍板过的（P2b ③），而它靠「复用 dividend_day」成立
        assert_eq!(rules.dividend_day, 15);
    }
}
