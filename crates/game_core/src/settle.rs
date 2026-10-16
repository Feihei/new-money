//! 结算层的账本：现金的唯一出口、破产清算、地产与银行的改账动作。
//!
//! 缺陷 #3 在这里没有落点：`Wallet` 只能经 [`Books::credit`] / [`Books::debit`] 改动，
//! 两者都必发 `CashChanged`；付不起时 `debit` 返回 `Err` 且**一分都不扣**，
//! 是撤销交易还是宣告破产由调用方显式选择 —— 不再可能出现
//! `card_effects.gd:34-41` 那种「文案写着破产，状态还是 active」。
//!
//! 为什么是**一个** SystemParam 而不是「地产包 / 银行包 / 股票包」三个：
//! 那三块都要 `&mut Wallet`，装进同一个 system 会在 schedule 建立期直接撞成借用冲突。
//! 集中一处顺带把「谁改过现金」收成一个出口 —— 组件级冲突没有了，审计点也只有一个
//! （B3 的现金回归断言就是「余额 == 起始现金 + Σ `CashChanged.delta`」）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::economy::{self, LandView};
use crate::land::{BuildingLevel, LandId, Mortgaged, Owner};
use crate::msg::{CashChanged, PlayerBankrupt, PointsChanged};
use crate::player::{BankBalance, Bankrupt, PlayerId, Points, Wallet};
use crate::rules::GameRules;
use crate::stock::StockHoldings;
use crate::world::ActiveBoard;

/// 付不起钱。带着「要付多少」「有多少」，调用方据此决定是撤销交易还是破产清算。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsufficientFunds {
    pub needed: i64,
    pub available: i64,
}

/// 一局对局的账本。落格结算与决策执行都通过它读写运行时状态。
#[derive(SystemParam)]
pub struct Books<'w, 's> {
    pub active: Res<'w, ActiveBoard>,
    pub rules: Res<'w, GameRules>,
    wallets: Query<'w, 's, &'static mut Wallet, With<PlayerId>>,
    points: Query<'w, 's, &'static mut Points, With<PlayerId>>,
    banks: Query<'w, 's, &'static mut BankBalance, With<PlayerId>>,
    holdings: Query<'w, 's, &'static mut StockHoldings, With<PlayerId>>,
    faces: Query<'w, 's, (Entity, &'static PlayerId, Option<&'static Bankrupt>)>,
    plots: Query<
        'w,
        's,
        (
            Entity,
            &'static LandId,
            &'static mut Owner,
            &'static mut BuildingLevel,
            Option<&'static Mortgaged>,
        ),
    >,
    cash: MessageWriter<'w, CashChanged>,
    points_changed: MessageWriter<'w, PointsChanged>,
    bankrupts: MessageWriter<'w, PlayerBankrupt>,
}

impl<'w, 's> Books<'w, 's> {
    /// 地块运行时快照（全表）。租金/垄断/等级差/贷款额度都要看整盘，六到四十块地
    /// 线性扫一遍比再挂一层索引划算。
    pub fn views(&self) -> Vec<LandView> {
        self.plots
            .iter()
            .map(|(_, id, owner, level, mortgaged)| {
                LandView::new(&self.active.def, *id, owner.0, level.0, mortgaged.is_some())
            })
            .collect()
    }

    /// 单块地的运行时快照。`None` = 棋盘里没有这块地（`validate` 已在加载期拦下）。
    pub fn view(&self, land: LandId) -> Option<LandView> {
        let entity = *self.active.lands.get(&land)?;
        let (_, id, owner, level, mortgaged) = self.plots.get(entity).ok()?;
        Some(LandView::new(
            &self.active.def,
            *id,
            owner.0,
            level.0,
            mortgaged.is_some(),
        ))
    }

    pub fn land_entity(&self, land: LandId) -> Option<Entity> {
        self.active.lands.get(&land).copied()
    }

    pub fn balance(&self, player: Entity) -> i64 {
        self.wallets.get(player).map(|w| w.cash).unwrap_or(0)
    }

    pub fn id_of(&self, player: Entity) -> Option<PlayerId> {
        self.faces.get(player).ok().map(|(_, id, _)| *id)
    }

    /// `PlayerId → Entity`。地块归属存的是 `PlayerId`，而消息里一律带 `Entity`
    /// （接收方直接拿它查组件），跨过去就靠这个。
    pub fn holder(&self, id: PlayerId) -> Option<Entity> {
        self.faces
            .iter()
            .find(|(_, pid, _)| **pid == id)
            .map(|(e, _, _)| e)
    }

    pub fn is_bankrupt(&self, player: Entity) -> bool {
        self.faces
            .get(player)
            .map(|(_, _, bankrupt)| bankrupt.is_some())
            .unwrap_or(false)
    }

    /// 还活着的玩家。股息、利息这类「全员结算」用它。
    pub fn solvent(&self) -> Vec<(Entity, PlayerId)> {
        self.faces
            .iter()
            .filter(|(_, _, bankrupt)| bankrupt.is_none())
            .map(|(e, id, _)| (e, *id))
            .collect()
    }

    /// 加钱的唯一入口。`amount <= 0` 什么都不做（与 `player_data.gd:50-55` 一致）。
    pub fn credit(&mut self, player: Entity, amount: i64) {
        if amount <= 0 {
            return;
        }
        let Ok(mut wallet) = self.wallets.get_mut(player) else {
            return;
        };
        wallet.cash += amount;
        self.cash.write(CashChanged {
            player,
            delta: amount,
            balance: wallet.cash,
        });
    }

    /// 扣钱的唯一入口。钱不够时**一分都不扣**并返回 `Err`
    /// （`player_data.gd:39-46` 的「不够时不扣」），要不要因此破产由调用方决定。
    pub fn debit(&mut self, player: Entity, amount: i64) -> Result<(), InsufficientFunds> {
        if amount <= 0 {
            return Ok(());
        }
        let available = {
            let wallet = self.wallets.get(player).map_err(|_| InsufficientFunds {
                needed: amount,
                available: 0,
            })?;
            wallet.cash
        };
        if available < amount {
            return Err(InsufficientFunds {
                needed: amount,
                available,
            });
        }
        let mut wallet = self.wallets.get_mut(player).expect("上面查过存在");
        wallet.cash -= amount;
        self.cash.write(CashChanged {
            player,
            delta: -amount,
            balance: wallet.cash,
        });
        Ok(())
    }

    /// 「该付而付不起 → 当场破产」的统一出口。返回 `Err` 时已经宣告过破产，
    /// 收款方一分钱也拿不到（Godot 的收租失败就是这个语义）。
    pub fn charge(
        &mut self,
        commands: &mut Commands,
        player: Entity,
        amount: i64,
    ) -> Result<(), InsufficientFunds> {
        let Err(short) = self.debit(player, amount) else {
            return Ok(());
        };
        self.bankrupt(commands, player, short.needed);
        Err(short)
    }

    /// 破产清算：现金清零、存款负债归零、持仓清空、名下地块放回市场。
    ///
    /// 与 Godot 的差别是刻意修的：`player_data.gd:84-91` 只清玩家自己的 `properties`
    /// 列表，`PropertyRegistry` 里的业主仍写着死者 —— 于是租金永远打给一个破产者、
    /// 同色组永远卡住别人的垄断与升级。这里把归属一并摘掉。
    pub fn bankrupt(&mut self, commands: &mut Commands, player: Entity, unpaid: i64) {
        if self.is_bankrupt(player) {
            return;
        }
        if let Ok(mut wallet) = self.wallets.get_mut(player) {
            let lost = wallet.cash;
            if lost != 0 {
                wallet.cash = 0;
                self.cash.write(CashChanged {
                    player,
                    delta: -lost,
                    balance: 0,
                });
            }
        }
        if let Ok(mut bank) = self.banks.get_mut(player) {
            *bank = BankBalance::default();
        }
        if let Ok(mut holdings) = self.holdings.get_mut(player) {
            holdings.0.clear();
        }
        if let Some(pid) = self.id_of(player) {
            for (entity, _, mut owner, mut level, mortgaged) in self.plots.iter_mut() {
                if owner.0 == Some(pid) {
                    owner.0 = None;
                    level.0 = 0;
                    if mortgaged.is_some() {
                        commands.entity(entity).remove::<Mortgaged>();
                    }
                }
            }
        }
        commands.entity(player).insert(Bankrupt);
        self.bankrupts.write(PlayerBankrupt { player, unpaid });
        info!(
            "玩家 {} 破产（付不出 {unpaid}）",
            self.id_of(player).map(|p| p.0).unwrap_or_default()
        );
    }

    /// 回合计息：存款生息进余额、贷款计息进负债，都不经过现金
    /// （`bank_system.gd:74-89`），所以这里刻意不发 `CashChanged`。
    pub fn apply_interest_all(&mut self) {
        let rules = *self.rules;
        for (player, _, bankrupt) in self.faces.iter() {
            if bankrupt.is_some() {
                continue;
            }
            let Ok(mut bank) = self.banks.get_mut(player) else {
                continue;
            };
            let earned = economy::deposit_interest(bank.deposit, &rules);
            let owed = economy::loan_interest(bank.loan, &rules);
            bank.deposit += earned;
            bank.loan += owed;
        }
    }

    pub fn bank_of(&self, player: Entity) -> BankBalance {
        self.banks.get(player).copied().unwrap_or_default()
    }

    /// 点券的唯一入口（捡点券箱、将来的小游戏得分）。与现金分账：这里只发
    /// [`PointsChanged`]，一次都不碰 `Wallet` —— 所以 B3 的现金审计看不见这笔账，
    /// 这正是「永不通兑」在代码里的形状（PLAN §一）。
    ///
    /// 返回 `None` = 世界里没有这位玩家；面额 > 0 由 `BoardDef::validate` 守住，
    /// 所以这里不再为「发 0 点」留分支。
    pub fn award_points(&mut self, player: Entity, amount: u32) -> Option<u32> {
        let mut points = self.points.get_mut(player).ok()?;
        points.0 += amount;
        self.points_changed.write(PointsChanged {
            player,
            delta: amount as i64,
            balance: points.0,
        });
        Some(points.0)
    }

    pub fn holdings_of(&self, player: Entity) -> Vec<(crate::stock::StockId, u32)> {
        self.holdings
            .get(player)
            .map(|h| h.0.iter().map(|(k, v)| (k.clone(), *v)).collect())
            .unwrap_or_default()
    }

    pub fn set_holdings(&mut self, player: Entity, stock: crate::stock::StockId, shares: u32) {
        let Ok(mut holdings) = self.holdings.get_mut(player) else {
            return;
        };
        // 归零即撤 key（`stock_market.gd:142-146`：卖空后 `stocks.erase`）
        if shares == 0 {
            holdings.0.remove(&stock);
        } else {
            holdings.0.insert(stock, shares);
        }
    }

    /// 存款：现金 → 余额。非法额或现金不足 → `false`，两边都不动。
    pub fn deposit(&mut self, player: Entity, amount: i64) -> bool {
        let Some(moved) = economy::deposit_plan(amount, self.balance(player)) else {
            return false;
        };
        let Some(mut bank) = self.banks.get_mut(player).ok() else {
            return false;
        };
        bank.deposit += moved;
        self.debit(player, moved)
            .expect("deposit_plan 已确认现金够");
        true
    }

    /// 取款：余额 → 现金。
    pub fn withdraw(&mut self, player: Entity, amount: i64) -> bool {
        let balance = self.bank_of(player).deposit;
        let Some(moved) = economy::withdraw_plan(amount, balance) else {
            return false;
        };
        let Some(mut bank) = self.banks.get_mut(player).ok() else {
            return false;
        };
        bank.deposit -= moved;
        self.credit(player, moved);
        true
    }

    /// 贷款：上限来自名下**未抵押地块**（缺陷 #1 的修复点，见 `economy::calc_max_loan`）。
    pub fn borrow(&mut self, player: Entity, amount: i64) -> bool {
        let Some(pid) = self.id_of(player) else {
            return false;
        };
        let views = self.views();
        let max = economy::calc_max_loan(&self.active.def, &views, pid, &self.rules);
        let now = self.bank_of(player).loan;
        let Some(got) = economy::loan_plan(amount, now, max) else {
            return false;
        };
        let Some(mut bank) = self.banks.get_mut(player).ok() else {
            return false;
        };
        bank.loan += got;
        self.credit(player, got);
        true
    }

    /// 还款：多还的部分不收。
    pub fn repay(&mut self, player: Entity, amount: i64) -> bool {
        let (cash, loan) = (self.balance(player), self.bank_of(player).loan);
        let Some(paid) = economy::repay_plan(amount, cash, loan) else {
            return false;
        };
        let Some(mut bank) = self.banks.get_mut(player).ok() else {
            return false;
        };
        bank.loan -= paid;
        self.debit(player, paid).expect("repay_plan 已确认现金够");
        true
    }

    /// 买地：扣地价 → 归属落到买家 → 等级置 1（`property_registry.gd:29-33` 的
    /// `set_owner_by_key` 顺带把空地升到 1 级）。
    /// 付不起或已有人持有 → `None`，什么都不动。
    pub fn buy(&mut self, player: Entity, land: LandId) -> Option<i64> {
        let view = self.view(land)?;
        if !economy::can_buy(&self.active.def, &view, self.balance(player)) {
            return None;
        }
        let price = economy::land_price(&self.active.def, &view)?;
        let entity = self.land_entity(land)?;
        let pid = self.id_of(player);
        self.debit(player, price).ok()?;
        let (_, _, mut owner, mut level, _) = self.plots.get_mut(entity).ok()?;
        owner.0 = pid;
        level.0 = 1;
        Some(price)
    }

    /// 升级：扣钱 + 等级 +1。判定全在 `economy::can_upgrade`（垄断 + 等级差 ≤1）。
    pub fn upgrade(&mut self, player: Entity, land: LandId) -> Option<u8> {
        let view = self.view(land)?;
        let pid = self.id_of(player)?;
        let views = self.views();
        let cash = self.balance(player);
        if !economy::can_upgrade(&self.active.def, &view, pid, &views, cash, &self.rules) {
            return None;
        }
        let cost = economy::upgrade_cost(&self.active.def, &view)?;
        let entity = self.land_entity(land)?;
        self.debit(player, cost).ok()?;
        let (_, _, _, mut level, _) = self.plots.get_mut(entity).ok()?;
        level.0 += 1;
        Some(level.0)
    }

    /// 该地块的租金（无主/抵押 → 0；垄断翻倍）。
    pub fn rent_of(&self, land: LandId) -> i64 {
        let Some(view) = self.view(land) else {
            return 0;
        };
        let all = self.views();
        economy::calculate_rent(&self.active.def, &view, &all, &self.rules)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 破产者名下的地块必须被摘干净：这是「归属是组件」才能一句话做到的事。
    /// （组件级的行为由 `tests/settle.rs` 的 App 用例覆盖，这里只守住账本类型本身。）
    #[test]
    fn insufficient_funds_carries_the_shortfall() {
        let short = InsufficientFunds {
            needed: 100,
            available: 50,
        };
        assert_eq!(short.needed - short.available, 50, "缺口本身是可审计的");
    }

    #[test]
    fn bank_balance_starts_at_zero_on_both_sides() {
        let bank = BankBalance::default();
        assert_eq!((bank.deposit, bank.loan), (0, 0));
    }
}
