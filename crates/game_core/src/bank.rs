//! 银行：落格结息、ATM 解锁、柜台操作的风控。
//!
//! 存 / 取 / 借 / 还四个动作本身在 [`crate::settle::Books`] 上（它们是账本操作，
//! 与「踩到哪格」无关，面板和 B6 的 AI 调的是同一组方法）。这个文件管的是
//! **「谁此刻有资格办哪一种」**：
//!
//! | 窗口 | 怎么拿到 | 能办 | 活到 |
//! |---|---|---|---|
//! | [`AtmAccess`] | **经过**银行格（路过或停下都算） | 存款、取款 | 自己的下一回合开始 |
//! | `Pending::BankTeller` | **踩到并停在**银行格 | 存、取、借、还 | 玩家按「办完」 |
//!
//! 借钱比取钱危险（利息会一直滚），所以要多一个「人真的走进柜台」的条件。这条判据
//! 落在规则层而不是按钮的 `disabled` 上：面板会算错、AI 会绕过，编译器不会。
//!
//! 缺陷 #1 的实测点在 `Books::borrow`：可贷额度来自名下**地块**价格
//! （`economy::calc_max_loan`），Godot 端把 `PropertyRegistry` 的字符串 key
//! 当成主环整数下标去查格子，于是所有玩家的额度恒为 0、贷款功能整个不可用。

use bevy::prelude::*;

use crate::board::{TileIndex, TileKind};
use crate::company::{FINANCE_STOCK, chairman_of};
use crate::institution::Institution;
use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};
use crate::market::MarketClock;
use crate::msg::{
    BankOp, BankOpDone, BankOpRequested, BankVisited, FinancingDrawRequested, FinancingDrawn,
    FinancingRecalled, TileStepped,
};
use crate::settle::Books;
use crate::turn::TurnPhase;
use crate::world::ActiveBoard;

/// 谁手里有 ATM 卡（`None` = 没人）。路过后**一直有效到他自己的下一回合开始** ——
/// 判据是「路过银行格」这种地理事实，但它要落在一个真能点到的时间窗里才有意义。
///
/// 存的是 `Entity` 而不是布尔：资格是「谁的」，别人递进来的操作必须认得出主语。
/// 换局不需要额外清理：`Entity` 带 generation，新一局的玩家是新句柄，`allows` 一比就否。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AtmAccess(pub Option<Entity>);

impl AtmAccess {
    pub fn allows(&self, player: Entity) -> bool {
        self.0 == Some(player)
    }
}

/// 哪种操作要在哪个窗口才准办。纯函数，表驱动测试打在 [`op_windows`] 上。
///
/// **银行全年无休**（10-20 实报更正）：周末与节假日只挡股市，只要停到银行格就能办。
/// P2④ 那版在这里还有一个 `open` 位，现已整条退役 —— 「有卡就能办」因此没有日历上的例外。
///
/// ⚠️ 10-24（P11c②）开的那个例外**不是**日历例外，而是一条新闻：`lending_open` 为假时
/// 只有 [`BankOp::Borrow`] 被挡，**存取款与还款照办**。这两格原来合成一条 `Borrow | Repay`，
/// 拆开写就是这次的设计本身：「停止放款」说的是「不借新的」，不是「不许还钱」——
/// 把还款一起挡掉等于凭空给欠债的人多记一笔逾期，而那句惩罚没有任何一条规则配得上它。
pub fn bank_op_allowed(op: BankOp, atm: bool, counter: bool, lending_open: bool) -> bool {
    match op {
        BankOp::Deposit | BankOp::Withdraw => atm || counter,
        BankOp::Repay => counter,
        BankOp::Borrow => counter && lending_open,
    }
}

/// 踩到银行格：把这一格变成「柜台」—— 挂上待决事项，回合停在 `WaitDecision`
/// 直到玩家按「办完」。
///
/// **不再在这里结息**（P2④）：Godot 的 `bank_system.gd:74-89` 把利息挂在「踩到银行格」上，
/// 于是「多久结一次息」取决于骰子踩不踩这一格 —— 一局不路过银行就一辈子没有利息。
/// 现在息由 [`apply_month_end_interest_system`] 按日历发，这一格只剩柜台。
pub fn settle_bank_landing(
    books: Books,
    landing: Res<Landing>,
    mut pending: ResMut<PendingDecision>,
    mut visited: MessageWriter<BankVisited>,
) {
    let Some(Landed {
        player,
        domain: LandingDomain::Bank,
        ..
    }) = landing.0
    else {
        return;
    };
    let bank = books.bank_of(player);
    visited.write(BankVisited {
        player,
        deposit: bank.deposit,
        loan: bank.loan,
    });
    pending.offer(player, Pending::BankTeller);
}

/// 月末结息（P2④）：储金计息进余额、贷款计息进负债，都不经过现金
/// （`bank_system.gd:74-89` 的这条语义留着），所以这里刻意不发 `CashChanged` ——
/// B3 的现金审计据此把银行看成外部对手方，只有现金动作进流水。
///
/// 排在 `TurnEnd` 里、行情 tick **之后**：两者都要看「今天几号」，而 tick 就是在那一步
/// 推进的。放在它前面会用昨天的日历判断今天该不该结息，月末那天正好错开。
pub fn apply_month_end_interest_system(mut books: Books, clock: Res<MarketClock>) {
    let today = crate::calendar::date_of(clock.tick, books.session.epoch);
    if !crate::calendar::is_month_end(today) {
        return;
    }
    books.apply_interest_all();
    info!("月末结息（{today}）");
}

/// 走过银行格即解锁 ATM。挂 `TileStepped` 上，所以「路过」与「停下」都算 ——
/// 停下那条还额外开出柜台，两个窗口不冲突。
///
/// 消息下一帧才读得到（坑 12），而 `instant()` 规则下整个行走一帧走完，
/// 于是 `AtmAccess` 在落格的那一帧就已经就位。
pub fn grant_atm_access_system(
    mut steps: MessageReader<TileStepped>,
    active: Option<Res<ActiveBoard>>,
    mut atm: ResMut<AtmAccess>,
) {
    let Some(active) = active else {
        // 菜单或装配前的空帧：把读到的一律作废，别让上一局的残留活着
        for _ in steps.read() {}
        return;
    };
    for step in steps.read() {
        if is_bank(&active, step.tile) {
            atm.0 = Some(step.player);
        }
    }
}

fn is_bank(active: &ActiveBoard, tile: TileIndex) -> bool {
    active
        .def
        .tile(tile.0 as i64)
        .is_some_and(|t| t.kind == TileKind::Bank)
}

/// 办一笔银行操作。不改回合状态、也不该改 —— 存取款是玩家手里的自由动作，
/// 回合仍然由「掷骰 → 走 → 落格 → 结束」那条链推进（§4.6 的红线在 UI 侧的镜像）。
///
/// **ATM 认人不认回合**：卡是谁的就谁用，别人回合里也能存取 —— 路过那一刻回合机最忙，
/// 若还要求「必须是当前行动者」，这张卡就一辈子按不到。
/// 借与还问的是柜台（`Pending::BankTeller`），而柜台随本人回合结束就关，
/// 所以那条天然只对自己开。
///
/// 三种拒绝都**静默返回**：资格不对、金额非法（`Books` 的 plan 函数判的）、终局之后。
/// 面板该把办不动的按钮点亮成灰的，而不是按下去没声音。
pub fn bank_op_system(
    mut books: Books,
    mut requests: MessageReader<BankOpRequested>,
    atm: Res<AtmAccess>,
    pending: Res<PendingDecision>,
    phase: Res<State<TurnPhase>>,
    mut done: MessageWriter<BankOpDone>,
) {
    if matches!(phase.get(), TurnPhase::Boot | TurnPhase::Over) {
        // 终局之后钱还在动，回放与存档就对不上了
        return;
    }
    for r in requests.read() {
        let at_counter = pending.active() == Some((r.player, Pending::BankTeller));
        // 停摆读的是**世界此刻**那一位（与面板同一个函数），不是请求里带的数：
        // 请求由表现层递进来，让它自带「银行开不开」等于让它替规则层做决定
        let lending_open = !books.session.suspensions.halted(Institution::Bank);
        if !bank_op_allowed(r.op, atm.allows(r.player), at_counter, lending_open) {
            debug!(
                "银行操作被拒：{:?} 此刻办不了 {:?}（放款{}）",
                r.player,
                r.op,
                if lending_open {
                    "开着"
                } else {
                    "被新闻停了"
                }
            );
            continue;
        }
        let moved = match r.op {
            BankOp::Deposit => books.deposit(r.player, r.amount),
            BankOp::Withdraw => books.withdraw(r.player, r.amount),
            BankOp::Borrow => books.borrow(r.player, r.amount),
            BankOp::Repay => books.repay(r.player, r.amount),
        };
        if moved {
            done.write(BankOpDone {
                player: r.player,
                op: r.op,
                amount: r.amount,
            });
        }
    }
}

/// 融资三件套（P10d②，银行保险[`FINANCE_STOCK`]董事长的特权）的规则层。
/// 三个动作各自的判据与账目全在 `Books`（`financing_draw` / `financing_cover` /
/// `financing_recall_all`），这里只管**谁此刻有资格**与**该发哪条凭据**：
///
/// - **收回（③）每帧先查**：身份是持股数的导出值，反超可能发生在任何路径上
///   （买卖成交、破产变卖、神明抽股）—— 没有一条「身份变了」的消息可等，
///   逐帧对账才是有效判据（与 [`crate::company::chairman_of`] 不缓存同一条理由）。
/// - **挪用（①）**：只认当前的董事长本人，别人的请求读掉就丢（同 `bank_op_system`
///   的认人纪律）。余额不足按实挪数成交。
/// - **兜底（②）最后办**：储备透支只可能来自取款，而取款在 [`bank_op_system`] 里 ——
///   本 system 排在它之后（同一条 Update chain），到这一帧储备是正是负已有定论。
pub fn financing_system(
    mut books: Books,
    mut requests: MessageReader<FinancingDrawRequested>,
    mut drawn: MessageWriter<FinancingDrawn>,
    mut recalled: MessageWriter<FinancingRecalled>,
) {
    // ③ 收回：不再是董事长却还背着融资债的，当场清算（消息带实还数 ——
    // 现金不够的部分是银行的坏账，顶栏那句要说得清「还上了多少」）
    let current = chairman_of(&books, FINANCE_STOCK);
    for (player, paid) in books.financing_recall_all(current) {
        recalled.write(FinancingRecalled {
            player,
            amount: paid,
        });
    }
    let Some(chairman) = chairman_of(&books, FINANCE_STOCK) else {
        // 盘上没有银行保险的董事长：挪用请求全部作废（兜底也无从谈起）
        for _ in requests.read() {}
        return;
    };
    // ① 挪用：认人只认董事长本人
    for r in requests.read() {
        if r.player != chairman {
            debug!("融资被拒：{:?} 不是银行保险的董事长", r.player);
            continue;
        }
        let moved = books.financing_draw(chairman, r.of_player, r.amount);
        if moved > 0 {
            drawn.write(FinancingDrawn {
                chairman,
                amount: moved,
            });
        }
    }
    // ② 兜底：储备被取穿了（负数），董事长补差额 —— 差额同样记成他的融资债
    let covered = books.financing_cover(chairman);
    if covered > 0 {
        drawn.write(FinancingDrawn {
            chairman,
            amount: covered,
        });
    }
}

/// 四种操作各自的窗口，写成表是给「新增一种操作却忘了想它该在哪能办」留的镜子。
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn borrowing_needs_the_counter_while_atm_covers_cash_moves() {
        let op = BankOp::Deposit;
        // ATM 或柜台任一即可（踩到银行格的人本来也路过过它）。银行不看日历 ——
        // 「周末与节假日只挡股市」那条由 `tests/finance.rs` 的集成用例钉着
        for cash_op in [BankOp::Deposit, BankOp::Withdraw] {
            assert!(
                bank_op_allowed(cash_op, true, false, true),
                "{cash_op:?} 路过 ATM 该能办"
            );
            assert!(
                bank_op_allowed(cash_op, false, true, true),
                "{cash_op:?} 柜台里该能办"
            );
            assert!(
                !bank_op_allowed(cash_op, false, false, true),
                "{cash_op:?} 两个窗口都没有还放行 —— 这就是凭空印钱"
            );
            // 停摆也挡不住存取：新闻说的是「不借新的」，不是「银行关门」
            assert!(
                bank_op_allowed(cash_op, false, true, false),
                "{cash_op:?} 被停摆挡住了 —— 停止放款不该管储户的钱"
            );
        }
        // 借只在柜台且只在没停摆时；还**只**看柜台（停摆不许挡还款）
        for credit_op in [BankOp::Borrow, BankOp::Repay] {
            assert!(
                !bank_op_allowed(credit_op, true, false, true),
                "{credit_op:?} 不该在 ATM 办成"
            );
            assert!(bank_op_allowed(credit_op, false, true, true));
            assert!(bank_op_allowed(credit_op, true, true, true));
            assert!(!bank_op_allowed(credit_op, false, false, true));
        }
        assert!(
            !bank_op_allowed(BankOp::Borrow, false, true, false),
            "停止放款还放得出钱，这条新闻就白播了"
        );
        assert!(
            bank_op_allowed(BankOp::Repay, false, true, false),
            "停摆期间不许还钱 —— 那是替欠债的人多记一笔逾期"
        );
        assert!(matches!(op, BankOp::Deposit));
    }

    #[test]
    fn atm_access_is_per_player_not_global() {
        let a = Entity::from_raw_u32(1).expect("索引不该撞上保留值");
        let b = Entity::from_raw_u32(2).expect("索引不该撞上保留值");
        let none = AtmAccess::default();
        assert!(!none.allows(a));
        let mine = AtmAccess(Some(a));
        assert!(mine.allows(a));
        assert!(
            !mine.allows(b),
            "甲路过银行，乙不能跟着一起刷取款 —— 资格得认人"
        );
    }
}
