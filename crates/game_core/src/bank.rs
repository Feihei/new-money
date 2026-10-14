//! 银行格结算：踩到银行 → 全员计息。
//!
//! 存 / 取 / 借 / 还四个动作本身在 [`crate::settle::Books`] 上（它们是账本操作，
//! 与「踩到哪格」无关，B5 的银行面板和 B6 的 AI 都调同一组方法）。
//! 这里只有落格那一段 —— 对应 Godot `turn_manager.gd:381-386` 的 `_handle_bank_landing`。
//!
//! 缺陷 #1 的实测点在这里：`borrow` 的可贷额度来自名下**地块**价格
//! （`economy::calc_max_loan`），Godot 端把 `PropertyRegistry` 的字符串 key
//! 当成主环整数下标去查格子，于是所有玩家的额度恒为 0、贷款功能整个不可用。

use bevy::prelude::*;

use crate::board::TileKind;
use crate::landing::{Landed, Landing};
use crate::msg::BankVisited;
use crate::settle::Books;

/// 踩到银行格：全员结一次息，然后把访客自己的余额报给 UI。
///
/// 利息进余额/进负债，**不经过现金**（`bank_system.gd:74-89`），所以这一步不发
/// `CashChanged` —— B3 的现金审计据此把银行看成外部对手方，只有现金动作进流水。
pub fn settle_bank_landing(
    mut books: Books,
    landing: Res<Landing>,
    mut visited: MessageWriter<BankVisited>,
) {
    let Some(Landed {
        player,
        kind: TileKind::Bank,
        ..
    }) = landing.0
    else {
        return;
    };
    books.apply_interest_all();
    let bank = books.bank_of(player);
    visited.write(BankVisited {
        player,
        deposit: bank.deposit,
        loan: bank.loan,
    });
}
