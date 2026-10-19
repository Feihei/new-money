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

use crate::charge::{self, Charge, ChargeKind, Correction, Corrections};
use crate::damage::{self, DamageTally, LandDamage};
use crate::economy::{self, LandView};
use crate::land::{BuildingLevel, LandId, Mortgaged, Owner};
use crate::msg::{CashChanged, ChargeWaived, PlayerBankrupt, PointsChanged};
use crate::player::{BankBalance, Bankrupt, PlayerId, Points, Wallet};
use crate::plot::{PlotArea, area_of};
use crate::rules::GameRules;
use crate::status::{LandStatus, Timed};
use crate::stock::StockHoldings;
use crate::turn::GameSession;
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
    /// 物价指数的来源（[`crate::charge`] 的管线读它）。与 `ActiveBoard` 同批插入，
    /// 所以「棋盘在则它在」这条已经成立，不需要额外的门禁。回合末由
    /// `turn::update_price_index_system` 写一次。
    pub session: ResMut<'w, GameSession>,
    /// 修正登记处。P2 只有物价指数这一个常驻修正；P3 的卡、P6 的神明各自往这里 push。
    ///
    /// 要是**可变**的，为的不只是登记：一次性修正（免费卡）在真正扣款那一笔上被花掉，
    /// 而花掉它的动作只能发生在这里 —— 报价侧走 [`Self::quoted`]，那条路拿不到下标，
    /// 结构上就花不了卡（P3a 给免罪卡立的同一条纪律）。
    pub corrections: ResMut<'w, Corrections>,
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
    /// 地块的限时状态（涨价 / 查封）。**单独一条查询而不是塞进 `plots`**：它是只读的
    /// （写走 `stamp_status` + `Commands`），而 `plots` 带着两个 `mut`；混在一起会让
    /// 「谁在改地块状态」这件事从类型上看不出来。
    statuses: Query<'w, 's, Option<&'static Timed<LandStatus>>, With<LandId>>,
    cash: MessageWriter<'w, CashChanged>,
    /// 一次性修正生效那一笔的凭据（免费卡免掉了多少）。与 `cash` 分开写，
    /// 因为这里没有钱动 —— 见 [`ChargeWaived`]。
    waived: MessageWriter<'w, ChargeWaived>,
    points_changed: MessageWriter<'w, PointsChanged>,
    bankrupts: MessageWriter<'w, PlayerBankrupt>,
}

impl<'w, 's> Books<'w, 's> {
    /// 地块运行时快照（全表）。租金/垄断/等级差/贷款额度都要看整盘，六到四十块地
    /// 线性扫一遍比再挂一层索引划算。
    pub fn views(&self) -> Vec<LandView> {
        self.plots
            .iter()
            .map(|(entity, id, owner, level, mortgaged)| {
                // 状态查不到 = 这块地没挂限时状态。`single` 那类查询在实体缺席时才是错，
                // 这里缺席是常态（绝大多数时候街上没有条子）
                let status = self.statuses.get(entity).ok().flatten().map(|t| t.kind);
                LandView::new(
                    &self.active.def,
                    *id,
                    owner.0,
                    level.0,
                    mortgaged.is_some(),
                    status,
                )
            })
            .collect()
    }

    /// 单块地的运行时快照。`None` = 棋盘里没有这块地（`validate` 已在加载期拦下）。
    pub fn view(&self, land: LandId) -> Option<LandView> {
        let entity = *self.active.lands.get(&land)?;
        let (_, id, owner, level, mortgaged) = self.plots.get(entity).ok()?;
        let status = self.statuses.get(entity).ok().flatten().map(|t| t.kind);
        Some(LandView::new(
            &self.active.def,
            *id,
            owner.0,
            level.0,
            mortgaged.is_some(),
            status,
        ))
    }

    pub fn land_entity(&self, land: LandId) -> Option<Entity> {
        self.active.lands.get(&land).copied()
    }

    /// 这块地的业主是哪个实体。`None` = 无主，或棋盘里压根没有这块地。
    ///
    /// 地块类卡牌的承受者、手牌面板上那颗「→ 南京路」按钮旁边的名字，都读这一句。
    /// 它把 `PlayerId`（组件里存的）翻成 `Entity`（消息与查询用的），中间那步就是 [`Self::holder`]。
    pub fn owner_of(&self, land: LandId) -> Option<Entity> {
        self.holder(self.view(land)?.owner?)
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

    /// 登记一条修正。P3c 的免费卡、P6 的神明都从这里进，管线本身不必认识它们。
    pub fn register(&mut self, correction: Correction) {
        self.corrections.register(correction);
    }

    /// 把一笔账过一遍修正管线，**不动钱也不花卡**。
    ///
    /// 报价走这一条：`amount` 只从 `base` 算（见 `charge::stage`），所以同一个入参调多少次
    /// 都是同一个数 —— 「模态上写的价」与「扣掉的价」不可能不一致，除非中间的登记变了
    /// （卡被打掉、神明到期），那时候以成交这次为准。成交走 [`Books::settle`]。
    pub fn staged(&self, charge: Charge) -> Charge {
        charge::stage(charge, self.session.price_index, &self.corrections)
    }

    /// 某个名义价对这位玩家来说实际是多少。UI 报价走这一条。
    pub fn quoted(&self, kind: ChargeKind, player: Entity, base: i64) -> i64 {
        self.staged(Charge::new(kind, Some(player), None, base))
            .amount
    }

    /// 走管线的一笔收付 —— 「询问 → 修正 → 扣」的第三步，也是**有名目的钱**的唯一入口。
    ///
    /// 返回最终那笔账（`amount` 已定，`payer` 可能已被转嫁给别人）。付不起时与
    /// [`Books::charge`] 同语义：当场破产、收款方一分拿不到，返回 `Err`。
    ///
    /// **只有这一条路会花掉一次性修正**：它走 [`charge::stage_and_spend`] 并注销回报的条目，
    /// 而报价侧的 [`Self::quoted`] 走 [`Self::staged`]，连下标都拿不到。
    ///
    /// `debit`/`credit` 仍然留在 `pub`：银行存取借还那条路是**自己左口袋换右口袋**，
    /// 不是一笔债，没有可修正的对象；硬套进管线只会让「存款被物价指数放大」这种
    /// 荒谬事变成可能。边界写在 [`crate::charge`] 模块头。
    pub fn settle(
        &mut self,
        commands: &mut Commands,
        charge: Charge,
    ) -> Result<Charge, InsufficientFunds> {
        // bearer 在过链**之前**取：转嫁那一支会把 `payer` 换人，而花掉那张卡的是登记它的
        // 那个人，不是最后付钱的那个人。
        let bearer = charge.bearer();
        let (charge, spent) =
            charge::stage_and_spend(charge, self.session.price_index, &self.corrections);
        if !spent.is_empty() {
            let at: Vec<usize> = spent.iter().map(|s| s.at).collect();
            for (idx, gone) in self.corrections.spend(&at) {
                let Some(spend) = spent.iter().find(|s| s.at == idx) else {
                    continue;
                };
                if let Some(player) = bearer {
                    self.waived.write(ChargeWaived {
                        player,
                        saved: spend.saved,
                        via: gone.source,
                    });
                }
            }
        }
        let Some(payer) = charge.payer else {
            // 纯收入（奖金、股息）：系统出钱，没有「付不起」这一说
            if let Some(payee) = charge.payee {
                self.credit(payee, charge.amount);
            }
            return Ok(charge);
        };
        if let Err(short) = self.debit(payer, charge.amount) {
            self.bankrupt(commands, payer, short.needed);
            return Err(short);
        }
        if let Some(payee) = charge.payee {
            self.credit(payee, charge.amount);
        }
        Ok(charge)
    }

    /// 全场现资产总和（物价指数的分子）。破产者不计 —— 他们已经离场，账也清零了。
    ///
    /// `index` 是**上一回合**的指数，用来把名义地产折成「按今天的价值多少」；理由与
    /// 不发散性见 [`economy::price_index`]。传 1.0 就是纯名义口径。
    ///
    /// `stock_price` 由调用方给（只有 [`crate::market::Market`] 查得到股价）：账本管的是
    /// 「谁有多少钱与地」，行情不是它的职责，为了一个乘法去挂股票实体会把边界弄脏。
    pub fn total_assets(
        &self,
        index: f32,
        stock_price: impl Fn(&crate::stock::StockId) -> f32,
    ) -> i64 {
        let def = &self.active.def;
        let views = self.views();
        self.solvent()
            .into_iter()
            .map(|(player, pid)| {
                let cash = self.wallets.get(player).map(|w| w.cash).unwrap_or(0);
                let bank = self.banks.get(player).copied().unwrap_or_default();
                let nominal = views
                    .iter()
                    .filter(|l| l.owner == Some(pid) && !l.mortgaged)
                    .map(|l| economy::estate_value(def, l))
                    .sum::<i64>();
                let estate = (nominal as f64 * index as f64).round() as i64;
                let stocks = self
                    .holdings
                    .get(player)
                    .map(|h| {
                        h.0.iter()
                            .map(|(id, n)| (stock_price(id) * *n as f32) as i64)
                            .sum()
                    })
                    .unwrap_or(0);
                economy::net_worth(cash, bank.deposit, bank.loan, estate, stocks)
            })
            .sum()
    }

    /// 全场初始资产总和（物价指数的分母）：开局人数 × 起始现金，整局不动。
    ///
    /// 破产的人**仍然留在分母里** —— 于是把对手打破产会让全场通缩。这是刻意的：
    /// 「清算别人并不能让自己的房子更值钱」是这条指数唯一被玩家看得见的性质。
    pub fn initial_assets(&self) -> i64 {
        self.faces.iter().count() as i64 * self.rules.start_cash
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

    /// 打一次建筑损害（P2② 的「拆房 / 征收」动词，形状见 [`crate::damage`]）。
    ///
    /// `Mortgaged` 只在**夺地**那一刻摘：抵押中的地被 `Degrade` 或 `Clear{seize:false}`
    /// 打过之后仍然是他的、仍然欠着 —— 房子被拆不该让债主免单（那是 P9 变卖池才有的账）。
    /// 反过来若地都无主了还留着抵押，`economy` 那三处「抵押中」判定就会替一个不存在的主人
    /// 挡掉租金。
    ///
    /// 不发消息：表现层读的是 `Owner`/`BuildingLevel` 的**变化**（`render::land::sync_buildings`
    /// 用 `Ref<T>` 检测），组件改了它就跟着改。等 P11 的新闻要播「外星人拆了哪几块」时
    /// 再补一条带地块清单的消息，而不是现在就为它准备一个没人读的字段。
    pub fn damage(
        &mut self,
        commands: &mut Commands,
        center: LandId,
        depth: LandDamage,
        area: PlotArea,
    ) -> DamageTally {
        let mut tally = DamageTally::default();
        for land in area_of(&self.active.def, center, area) {
            let Some(entity) = self.active.lands.get(&land).copied() else {
                continue;
            };
            let Ok((_, _, mut owner, mut level, mortgaged)) = self.plots.get_mut(entity) else {
                continue;
            };
            tally.hit += 1;
            let before = owner.0;
            damage::strike(&mut level.0, &mut owner.0, depth);
            if before.is_some() && owner.0.is_none() {
                tally.seized += 1;
                if mortgaged.is_some() {
                    commands.entity(entity).remove::<Mortgaged>();
                }
            }
        }
        info!(
            "建筑损害 {depth:?} × {area:?}（中心 {center:?}）：命中 {} 块、收回 {} 块",
            tally.hit, tally.seized
        );
        tally
    }

    /// 给一片地块盖上限时状态（P3c 的涨价 / 查封）。返回被盖到的块数。
    ///
    /// **单槽位、后来的赢**：同一块地上不会同时挂着「涨价」与「查封」——`insert` 直接覆盖
    /// 上一枚 `Timed`，于是 PLAN §三 那对「抵消对」在数据结构上就是替换，不需要一张
    /// 「谁压过谁」的优先级表（见 [`crate::status`] 模块头那条理由）。
    ///
    /// 面积解析与损害共用 [`crate::plot::area_of`]：涨价卡的「整街」与飞弹的「整路段」
    /// 必须是同一条街，两处各算一遍迟早会分叉。
    ///
    /// 无主的地也照样盖（返回的块数含它们）：范围是范围，不为「没打着有主的地」缩小 ——
    /// 与 [`Self::damage`] 同一条口径。今天能不能盖由出牌那一侧判（`aim_is_legal` 要求
    /// 锚点那块地有业主），而不是由这里偷偷过滤。
    pub fn stamp_status(
        &self,
        commands: &mut Commands,
        center: LandId,
        status: LandStatus,
        area: PlotArea,
        days: u32,
    ) -> u32 {
        let mut hit = 0;
        for land in area_of(&self.active.def, center, area) {
            let Some(entity) = self.active.lands.get(&land).copied() else {
                continue;
            };
            commands.entity(entity).insert(Timed::new(status, days));
            hit += 1;
        }
        info!(
            "地块状态 {} × {area:?}（中心 {center:?}）：盖了 {hit} 块、{days} 天",
            status.label()
        );
        hit
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

    /// 点券的唯一**收入**入口（捡点券箱、百货卖回、将来的小游戏得分）。与现金分账：这里只发
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

    /// 某人现在有多少点券（百货面板给灰按钮找理由要读它）。查不到按 0 —— 没有点券组件的
    /// 实体本来就买不了东西，这里不该为它开一条「未知」的分支。
    pub fn points_of(&self, player: Entity) -> u32 {
        self.points.get(player).map(|p| p.0).unwrap_or(0)
    }

    /// 点券的唯一支出入口（百货买卡、P12 保释 300）。与 [`Self::award_points`] 对称：
    /// 只发 [`PointsChanged`]（`delta` 为负），一次都不碰 `Wallet`。
    ///
    /// 余额不足时**一分都不扣**并返回 `false` —— 与 [`Self::debit`] 同一条形状：撤销交易还是
    /// 宣告破产由调用方显式选，账本不替它决定「扣一半」。
    pub fn spend_points(&mut self, player: Entity, amount: u32) -> bool {
        let Ok(mut points) = self.points.get_mut(player) else {
            return false;
        };
        if points.0 < amount {
            return false;
        }
        points.0 -= amount;
        self.points_changed.write(PointsChanged {
            player,
            delta: -(amount as i64),
            balance: points.0,
        });
        true
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

    /// 买地：扣**实付**地价 → 归属落到买家 → 等级置 1（`property_registry.gd:29-33` 的
    /// `set_owner_by_key` 顺带把空地升到 1 级）。
    /// 付不起或已有人持有 → `None`，什么都不动。
    ///
    /// 价过一遍管线（物价指数 + 小福神「买地免费」那一类 `Cancel`），所以这里返回的
    /// 是实付额而不是表上的地价 —— 免费时是 0，`LandBought.price` 因此说的是「这笔账」。
    pub fn buy(&mut self, player: Entity, land: LandId) -> Option<i64> {
        let view = self.view(land)?;
        let base = economy::land_price(&self.active.def, &view)?;
        let price = self.quoted(ChargeKind::BuyLand { land }, player, base);
        if !economy::can_buy(&view, price, self.balance(player)) {
            return None;
        }
        let entity = self.land_entity(land)?;
        let pid = self.id_of(player);
        self.debit(player, price).ok()?;
        let (_, _, mut owner, mut level, _) = self.plots.get_mut(entity).ok()?;
        owner.0 = pid;
        level.0 = 1;
        Some(price)
    }

    /// 升级：扣**实付**建筑费 + 等级 +1。结构判定（业主本人、未抵押、未满级、垄断、
    /// 组内等级差 ≤1）全在 `economy::can_upgrade`。
    pub fn upgrade(&mut self, player: Entity, land: LandId) -> Option<u8> {
        let view = self.view(land)?;
        let pid = self.id_of(player)?;
        let base = economy::upgrade_cost(&self.active.def, &view)?;
        let cost = self.quoted(ChargeKind::Upgrade { land }, player, base);
        let views = self.views();
        let cash = self.balance(player);
        if !economy::can_upgrade(&view, pid, &views, cost, cash, &self.rules) {
            return None;
        }
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
