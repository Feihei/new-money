//! 抵押与赎回（P3d③）：把名下的地块押给银行换一笔**不计息**的现金，连本带费赎回来。
//!
//! PLAN §三 那一行「抵押/赎回」在 4 代口径上记的是「—」（没人记得确切数字），所以这里的
//! 形状是按**结构**定的，两个数（`mortgage_percent` / `redeem_percent`）挂在 `GameRules`
//! 上等 P17 之前调。三条口径决定了这个文件长什么样：
//!
//! - **它是柜台业务，不是随时可办**。与借/还同一格：踩到银行、停在上面、按「办完」之前
//!   那一段才算，周末照样休。抵押是一笔债，而「什么时候允许负债」这条判据在规则层
//!   （[`crate::bank`] 的 ATM 分岔是银行独有的待遇，这里不复制）。
//! - **押出去不计息，所以赎回来要付费**。不计息是它区别于 [`crate::settle::Books::borrow`]
//!   的全部理由（代价是失去这块地的收租权、垄断资格与可贷额度）；若赎回再分文不加，
//!   「缺钱就押、宽裕就赎」就是一台免费的现金泵。
//! - **本金存在地块上**，赎回按「当初到手那笔 + 手续费」算，不按今天重算（见
//!   [`crate::land::Mortgaged`] 那条注释：重算在今天的规则下恰好相等，但那个「恰好」是
//!   借了 `can_upgrade` 挡着加盖的力，一笔债的价不该寄存在别人的不变式上）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::economy::{self, LandView};
use crate::land::LandId;
use crate::landing::{Pending, PendingDecision};
use crate::market::MarketClock;
use crate::msg::{MortgageDone, MortgageOp, MortgageRequested};
use crate::settle::Books;
use crate::status::LandStatus;
use crate::turn::TurnPhase;

/// 判一件抵押/赎回要读的八样东西。**调用方一律走 [`read_facts`]**，不许自己凑：
/// 面板与规则层读的是同一份事实，才会出现「灰着的按钮按下去确实没声音」，
/// 而不是两套判据各自演化（同 `shop_op_allowed` 的六个参数，只是这里要多两样）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MortgageFacts {
    /// 人站在银行柜台前吗（`Pending::BankTeller` 挂着他）
    pub at_counter: bool,
    /// 银行今天开门吗（周末休市，与四种 `BankOp` 同一条）
    pub open: bool,
    /// 棋盘里有这块地
    pub on_board: bool,
    /// 这块地是**请求者**的（不是别人的、也不是无主）
    pub owned: bool,
    /// 这块地此刻押在银行
    pub held: bool,
    /// 这块地被查封（法院贴着条子）
    pub sealed: bool,
    /// 这一笔要过手多少：`Take` 收钱所以是 0，`Redeem` 是本金 + 手续费
    pub cost: i64,
    pub cash: i64,
}

/// 办不成的原因。UI 的灰按钮文案与规则层的拒绝**共用这一张表**
/// （同 `shop::ShopDeny`：判据只写一次，面板只是把它翻成人话）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MortgageDeny {
    /// 人不站在银行柜台前
    NoCounter,
    /// 周末休市
    Closed,
    /// 棋盘里没有这块地（数据错误，`validate` 已在加载期拦下）
    NotOnBoard,
    /// 这块地不是你的
    NotYours,
    /// 已经押着了，不能再押一次
    AlreadyHeld,
    /// 没押着，赎什么
    NotHeld,
    /// 查封中的地块既押不出去也赎不回来
    Sealed,
    /// 现金不够赎回
    NoCash,
}

impl MortgageDeny {
    /// 灰按钮上要写的那句话。**每颗灰按钮都得带一句为什么**（§6.1 坑 27 的 UI 版）。
    pub fn label(self) -> &'static str {
        match self {
            Self::NoCounter => "要走进银行柜台",
            Self::Closed => "周末休市",
            Self::NotOnBoard => "棋盘里没有这块地",
            Self::NotYours => "这块地不是你的",
            Self::AlreadyHeld => "已经押在这里了",
            Self::NotHeld => "这块地没押着",
            Self::Sealed => "法院贴着条子",
            Self::NoCash => "现金不够赎回",
        }
    }
}

/// 这一件办不办得成。四问的顺序是刻意的：**先问窗口，再问对象，最后问钱**——
/// 窗口不对时不必查地块，而「这块地是谁的」又排在「它押没押」之前：别人的地块上
/// 那些状态根本不该被读到（否则一句 `NotHeld` 就把别人的地说了出去）。
pub fn mortgage_op_allowed(op: MortgageOp, f: &MortgageFacts) -> Result<(), MortgageDeny> {
    if !f.at_counter {
        return Err(MortgageDeny::NoCounter);
    }
    if !f.open {
        return Err(MortgageDeny::Closed);
    }
    if !f.on_board {
        return Err(MortgageDeny::NotOnBoard);
    }
    if !f.owned {
        return Err(MortgageDeny::NotYours);
    }
    // 查封挡两件：押出去是拿有纠纷的东西再借一笔，赎回来是银行替法院解押 ——
    // 两边都不该在这一格发生。挡在 `held` 之前，因为「查封中」是比「押没押」更外层的事实
    if f.sealed {
        return Err(MortgageDeny::Sealed);
    }
    match op {
        MortgageOp::Take => {
            if f.held {
                Err(MortgageDeny::AlreadyHeld)
            } else {
                Ok(())
            }
        }
        // 赎回先问钱：`Books::redeem` 在钱不够时一分不扣、地也还押着，这里提前挡掉，
        // 好让按钮灰着而不是按下去没声音
        MortgageOp::Redeem => {
            if !f.held {
                return Err(MortgageDeny::NotHeld);
            }
            if f.cash < f.cost {
                return Err(MortgageDeny::NoCash);
            }
            Ok(())
        }
    }
}

/// 把八样事实查齐。**面板与规则层都走这一条**，于是「这颗按钮为什么灰」与
/// 「这一笔为什么被拒」是同一个函数算出来的同一个数。
///
/// `cost` 只在赎回那一支有内容：押出去是收钱，没有「先问够不够」这一说。
pub fn read_facts(
    books: &Books,
    player: Entity,
    land: LandId,
    at_counter: bool,
    open: bool,
) -> MortgageFacts {
    let view = books.view(land);
    let pid = books.id_of(player);
    // 主语查不到时一律「不是他的」：`view.owner` 与 `pid` 同为 `Option`，直接比会让
    // 「无主地 + 不存在的玩家」相等，那正好是最不该放行的一格
    let owned = match (pid, view.as_ref().map(|v| v.owner)) {
        (Some(me), Some(owner)) => owner == Some(me),
        _ => false,
    };
    MortgageFacts {
        at_counter,
        open,
        on_board: view.is_some(),
        owned,
        held: view.as_ref().is_some_and(|v| v.mortgaged),
        sealed: matches!(
            view.as_ref().map(|v| v.status),
            Some(Some(LandStatus::Sealed))
        ),
        cost: books.redeem_cost_of(land).unwrap_or(0),
        cash: books.balance(player),
    }
}

/// 名下**可以摆上柜台**的地块（自己的地，含已押着的 —— 押着的那些长成「赎回」那一行）。
///
/// 顺序按 `LandId` 排：面板是按这个顺序一行一行建起来的，而 `views()` 来自实体迭代，
/// 不保证与棋盘定义同序。不排序的话每次重建可能换队，玩家刚看清的那一行就跳走了。
pub fn counter_plots(books: &Books, player: Entity) -> Vec<(LandId, LandView)> {
    let Some(pid) = books.id_of(player) else {
        return Vec::new();
    };
    let mut plots: Vec<(LandId, LandView)> = books
        .views()
        .into_iter()
        .filter(|v| v.owner == Some(pid))
        .map(|v| (v.id, v))
        .collect();
    plots.sort_by_key(|(id, _)| id.0);
    plots
}

/// 柜台上的抵押与赎回。不改回合状态、也不该改 —— 与四种 `BankOp` 同一格：
/// 它们是玩家站在柜台前的自由动作，回合仍然由「办完」那一句交还（§4.6 红线在 UI 侧的镜像）。
///
/// 拒绝一律走 `debug!` 之后静默返回：面板该把办不动的按钮亮成灰的（灰的理由由
/// [`MortgageDeny::label`] 给），而不是按下去没声音。
///
/// **同一帧里两条押同一块地的请求只成一次**：`Books::mortgage` 就地写 `Mortgaged`，
/// 第二条请求的 [`read_facts`] 当场读到 `held`。这条不是顺手 —— 若本金走 `Commands`
/// 的延迟写，两条都会看到「还没押」，于是同一块地付两遍钱。
pub fn mortgage_op_system(
    mut desk: MortgageDesk<'_, '_>,
    mut requests: MessageReader<MortgageRequested>,
) {
    if matches!(desk.phase.get(), TurnPhase::Boot | TurnPhase::Over) {
        // 终局之后还在改地产与负债，回放与存档就对不上账了
        return;
    }
    let open = !economy::is_weekend(desk.clock.tick, &desk.books.rules);
    for r in requests.read() {
        let at_counter = desk.pending.active() == Some((r.player, Pending::BankTeller));
        let facts = read_facts(&desk.books, r.player, r.land, at_counter, open);
        if let Err(why) = mortgage_op_allowed(r.op, &facts) {
            debug!(
                "抵押柜台被拒：{:?} 想{:?}第 {} 号地 —— {}",
                r.player,
                r.op,
                r.land.0,
                why.label()
            );
            continue;
        }
        let moved = match r.op {
            MortgageOp::Take => desk.books.mortgage(r.player, r.land),
            MortgageOp::Redeem => desk.books.redeem(r.player, r.land),
        };
        let Some(amount) = moved else {
            // 判据过了而账没成 —— `Books` 那一侧的二次认主语挡下了。真走到这里
            // 说明两侧判据分叉了，宁可留一行日志也不要静默
            warn!("抵押柜台：{:?} 的 {:?} 过了判据却没办成", r.player, r.op);
            continue;
        };
        desk.done.write(MortgageDone {
            player: r.player,
            op: r.op,
            land: r.land,
            amount,
        });
    }
}

/// 办一件抵押/赎回要动的东西（房规：打包成参数包，见 §六 坑 8）。
#[derive(SystemParam)]
pub struct MortgageDesk<'w, 's> {
    pub books: Books<'w, 's>,
    pub pending: Res<'w, PendingDecision>,
    pub phase: Res<'w, State<TurnPhase>>,
    pub clock: Res<'w, MarketClock>,
    pub done: MessageWriter<'w, MortgageDone>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一件「站在柜台前、银行开门、地是自己的、没押没查封」的普通请求要读到的事实。
    fn take() -> MortgageFacts {
        MortgageFacts {
            at_counter: true,
            open: true,
            on_board: true,
            owned: true,
            held: false,
            sealed: false,
            cost: 0,
            cash: 20_000,
        }
    }

    #[test]
    fn a_plain_mortgage_goes_through() {
        assert_eq!(mortgage_op_allowed(MortgageOp::Take, &take()), Ok(()));
    }

    /// 窗口排在最前：不在柜台前时，连「这块地是不是你的」都不必问。
    /// 这一条是「别人的点击不能替他押地」与「办完离开之后还能押」的共同拒绝路径。
    #[test]
    fn the_window_is_the_first_question_and_the_last_word() {
        for op in [MortgageOp::Take, MortgageOp::Redeem] {
            let mut f = take();
            f.at_counter = false;
            f.owned = false;
            assert_eq!(
                mortgage_op_allowed(op, &f),
                Err(MortgageDeny::NoCounter),
                "{op:?} 在没有柜台时还往下问了"
            );
            let mut g = take();
            g.at_counter = false;
            g.open = false;
            assert_eq!(mortgage_op_allowed(op, &g), Err(MortgageDeny::NoCounter));
        }
    }

    /// 休市挡两件，且**有柜台也挡** —— 周末走进银行，人站着、地是自己的，照样办不成。
    /// 与四种 `BankOp` 同一条日历判据（`bank::a_closed_bank_refuses_everything` 的地产版）。
    #[test]
    fn a_closed_bank_refuses_land_moves_too() {
        let mut f = take();
        f.open = false;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Take, &f),
            Err(MortgageDeny::Closed)
        );
        let mut g = take();
        g.open = false;
        g.held = true;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Redeem, &g),
            Err(MortgageDeny::Closed)
        );
    }

    /// 别人的地块连「押没押」都不该被读到：拒绝必须是 `NotYours`，
    /// 而不是替别人报出一种状态。
    #[test]
    fn somebody_elses_plot_is_not_even_asked_about() {
        for (op, held) in [
            (MortgageOp::Take, false),
            (MortgageOp::Take, true),
            (MortgageOp::Redeem, true),
        ] {
            let mut f = take();
            f.owned = false;
            f.held = held;
            assert_eq!(
                mortgage_op_allowed(op, &f),
                Err(MortgageDeny::NotYours),
                "{op:?} 押了别人的地"
            );
        }
    }

    #[test]
    fn sealing_blocks_both_directions() {
        let mut f = take();
        f.sealed = true;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Take, &f),
            Err(MortgageDeny::Sealed),
            "查封中的地还能再押一次 —— 银行收了有纠纷的抵押物"
        );
        let mut g = take();
        g.sealed = true;
        g.held = true;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Redeem, &g),
            Err(MortgageDeny::Sealed),
            "查封中赎回来也收不到租，这一笔只会让人以为解押了"
        );
    }

    #[test]
    fn one_plot_is_held_once_and_released_once() {
        // 押第二次
        let mut f = take();
        f.held = true;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Take, &f),
            Err(MortgageDeny::AlreadyHeld),
            "同一块地押两次 —— 第二笔是白印的钱"
        );
        // 赎一块没押的
        let mut g = take();
        g.held = false;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Redeem, &g),
            Err(MortgageDeny::NotHeld),
            "没押着的地也能赎回"
        );
    }

    /// 赎回问钱，押出去不问：押是收钱的那一支，若照抄「够不够」就成了永远为真的废话。
    #[test]
    fn redeeming_needs_the_cash_while_mortgaging_needs_none() {
        let mut held = take();
        held.held = true;
        held.cost = 825;
        held.cash = 824;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Redeem, &held),
            Err(MortgageDeny::NoCash),
            "差 1 块也算赎不起"
        );
        held.cash = 825;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Redeem, &held),
            Ok(()),
            "刚好够该放行"
        );
        let mut broke = take();
        broke.cash = 0;
        assert_eq!(
            mortgage_op_allowed(MortgageOp::Take, &broke),
            Ok(()),
            "身无分文正是来抵押的理由"
        );
    }

    #[test]
    fn every_denial_says_something_different() {
        let all = [
            MortgageDeny::NoCounter,
            MortgageDeny::Closed,
            MortgageDeny::NotOnBoard,
            MortgageDeny::NotYours,
            MortgageDeny::AlreadyHeld,
            MortgageDeny::NotHeld,
            MortgageDeny::Sealed,
            MortgageDeny::NoCash,
        ];
        let words: Vec<&str> = all.iter().map(|d| d.label()).collect();
        assert!(words.iter().all(|w| !w.is_empty()), "有句灰按钮的话是空的");
        let mut uniq = words.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), all.len(), "有两种拒绝说了同一句话: {words:?}");
    }
}
