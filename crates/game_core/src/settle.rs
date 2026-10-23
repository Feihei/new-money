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
use crate::player::{BankBalance, Bankrupt, Detention, DetentionPlace, PlayerId, Points, Wallet};
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
            &'static mut Mortgaged,
        ),
    >,
    /// 地块的限时状态（涨价 / 查封）。**单独一条查询而不是塞进 `plots`**：它是只读的
    /// （写走 `stamp_status` + `Commands`），而 `plots` 带着两个 `mut`；混在一起会让
    /// 「谁在改地块状态」这件事从类型上看不出来。
    statuses: Query<'w, 's, Option<&'static Timed<LandStatus>>, With<LandId>>,
    /// 拘留计时（P4）。**放在账本里而不是 `CardSink` 上**：「谁被关着」这件事账本必须读得到
    /// —— 收租权剥夺走的是 `LandView.owner_detained`（P4②），而那份快照只能由账本自己填。
    /// 写它的两条路（命运牌、陷害卡）也就在这里，`CardSink` 改用它（同一 system 里两条
    /// 对 `Detention` 的一读一写会在参数建立期直接撞成借用冲突，与本文件头那句同一条理由）。
    ///
    /// 就地写而不是 `Commands`：同一帧两张拘留卡，第二条要看得见第一条（见
    /// [`crate::landing::detain`]）。
    ///
    /// **只有 `Detention` 这一族**：P3d④c 的停留（`StaySentence`）不在这里 —— 被停留按住的
    /// 人还在场上，名下的地照收租，而卡面上没写「顺便收不到租」这一条（`tests/stay.rs` 钉着）。
    pub detained: Query<'w, 's, &'static mut Detention, With<PlayerId>>,
    /// 商业设施（P10c）。只读：建造的插入走 `Commands`（空商业地身上**没有**这个组件，
    /// 就地写无处可写）。与 `plots` 不同组件，借用相容 —— 账本读得到它，`Books::build`
    /// 的「一格一设施」判据才能与扣款在同一处判，UI 不必再开第二份查询。
    facilities: Query<'w, 's, Option<&'static crate::land::Facility>, With<LandId>>,
    /// 身上挂着哪位神（P6b②）。账本要读得到它，是因为「买地建屋必败」与「加盖多盖一层」
    /// 这两条**必须与扣款在同一处判**：判据放在调用方（落格那段、购地卡那条），
    /// 迟早会有一边忘了问 —— 而「衰神照样买到了地」这种错不会崩，只会在玩家眼里变成
    /// 「这神是假的」。与 `detained` 同一族：账本读得出，UI 才不必自己开第二份查询。
    possessions: Query<'w, 's, &'static crate::gods::Possession, With<PlayerId>>,
    /// 拍卖机（P9）。账本必须读得到它，是因为**破产那一刻**要就地记下待拍的料 ——
    /// 那几块地与其层数此刻还在死者名下，下一帧就查不到了（`render`/`Commands` 的延迟写
    /// 会让同一帧两处破产各看到「还没记」，与 P3d③ 那条「同一帧押两遍钱」同一条坑）。
    ///
    /// 连带一条纪律：任何想同时碰账本与拍卖机的 system 必须**借这一格**，不能再拿一条
    /// `ResMut<Auction>` —— 同一个 system 参数里两条对同一资源的可变借用，会在参数建立期
    /// 直接撞（§6.1 坑 33 的同族，只是碰的是资源而不是查询）。
    pub auction: ResMut<'w, crate::auction::Auction>,
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
        let locked = self.locked();
        self.plots
            .iter()
            .map(|(entity, id, owner, level, mortgaged)| {
                // 状态查不到 = 这块地没挂限时状态。`single` 那类查询在实体缺席时才是错，
                // 这里缺席是常态（绝大多数时候街上没有条子）
                let status = self.statuses.get(entity).ok().flatten().map(|t| t.kind);
                let who = owner.0;
                LandView::new(
                    &self.active.def,
                    *id,
                    who,
                    level.0,
                    mortgaged.held(),
                    status,
                    who.is_some_and(|o| locked.contains(&o)),
                )
            })
            .collect()
    }

    /// 被关着的人的名单，一次算好给整盘快照用。地块 ≤40、人 ≤4，线性 `contains` 比再挂
    /// 一层集合划算，也比每块地查一次组件少一次借用。
    ///
    /// 从 `faces` 那一侧走：它是账本里唯一同时拿着实体与 `PlayerId` 的查询，而 `detained`
    /// 的键是实体。名单要的是编号 —— 地块快照手上只有 `Owner.0` 那个 `PlayerId`。
    fn locked(&self) -> Vec<PlayerId> {
        self.faces
            .iter()
            .filter(|(entity, _, _)| self.detained.get(*entity).is_ok())
            .map(|(_, id, _)| *id)
            .collect()
    }

    /// 被关在 `place` 的人（实体 + 计时），按 `faces` 的顺序。
    ///
    /// 保释窗口要一排候选，而面板手上只有 `&Books`：这里把「谁是实体」与「他被关在哪」
    /// 两半合起来给一次，UI 就不必再开一条读 `Detention` 的查询（那会与账本里的
    /// `&mut Detention` 在参数建立期撞成借用冲突 —— 与 P4② 那条同一个坑）。
    pub fn held(&self, place: DetentionPlace) -> Vec<(Entity, Detention)> {
        self.faces
            .iter()
            .filter_map(|(entity, _, _)| {
                self.detained
                    .get(entity)
                    .ok()
                    .filter(|d| d.place == place)
                    .map(|d| (entity, *d))
            })
            .collect()
    }

    /// 单块地的运行时快照。`None` = 棋盘里没有这块地（`validate` 已在加载期拦下）。
    pub fn view(&self, land: LandId) -> Option<LandView> {
        let entity = *self.active.lands.get(&land)?;
        let (_, id, owner, level, mortgaged) = self.plots.get(entity).ok()?;
        let status = self.statuses.get(entity).ok().flatten().map(|t| t.kind);
        let who = owner.0;
        let locked = self.locked();
        Some(LandView::new(
            &self.active.def,
            *id,
            who,
            level.0,
            mortgaged.held(),
            status,
            who.is_some_and(|o| locked.contains(&o)),
        ))
    }

    /// 这个人押在银行的地块上，总共欠着多少本金（P3d③）。
    ///
    /// 物价指数要把它当负债扣掉：抵押到手的那笔现金进了分子，而那块地已经从分子里出去了，
    /// 不扣本金就等于「抵押一次、全场通缩一点」—— 那笔钱并没有离开游戏，只是换了个口袋。
    pub fn mortgage_debt(&self, player: PlayerId) -> i64 {
        self.plots
            .iter()
            .filter(|(_, _, owner, _, _)| owner.0 == Some(player))
            .map(|(_, _, _, _, mortgage)| mortgage.proceeds)
            .sum()
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
    /// **抵押中的地块照样算资产，同时把本金当负债扣掉**，于是「押一块地换现金」对指数是
    /// 中性的一笔：现金进 750、债记 750，那块地还在（它是*可赎回*的，不是卖掉了）。反过来
    /// 若把地踢出分子又不记债，抵押一次就凭空通缩一点全场物价 —— 而物价指数是下一回合
    /// 每一笔租金的乘数，那等于让「谁缺钱了」去决定「所有人的房租」。
    /// 赎回那一笔付掉的手续费**是**真的离开游戏（付给银行这个外部对手方），指数随之降一点 ——
    /// 与罚款、贷款利息同一条口径。
    ///
    /// 注意这与 [`crate::economy::property_value`] 是两回事：那条问的是「这块地还能不能替
    /// 你担保借款」，押出去当然就不担保了；这里问的是「你有多少财富」。同一个 `mortgaged`
    /// 位回答两个问题，两个答案相反，所以两处各自写清楚。
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
                    .filter(|l| l.owner == Some(pid))
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
                // 抵押本金与贷款同格：都是「欠银行的钱」。不扣它，抵押就成了把钱从指数里
                // 抹掉的动作（地块出分子、现金进分子，而债哪儿都不在）
                economy::net_worth(
                    cash,
                    bank.deposit,
                    bank.loan + self.mortgage_debt(pid),
                    estate,
                    stocks,
                )
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
    /// **楼留在地上**（10-22 实报）。摘掉的只有归属与那笔抵押债，所以街上留下的是
    /// 「无主但立着楼」的地，而下一位走到那一格的人**连楼一起付**（[`Books::buy_price`]
    /// 的基数含已盖的每一层）。同一条消息也定了「拍卖的那三格连着建筑一起拍」——
    /// 两处读的是同一个世界状态，于是拍卖也不必再「先摘再还」：
    /// 旧版在这里清零层数、由 `auction::Listing` 把被摘之前那个数带进起拍价、成交那一刻还回去，
    /// 那一整趟的动机是「拍不走的地上不该留一栋凭空回来的楼」，而真正的答案是**根本别摘**。
    ///
    /// **放回去之前先留一份料**（P9）：他名下每一块地被记进
    /// [`crate::auction::Auction::liquidation`]，由拍卖机挑 `liquidation_lots` 块逐块挂拍 ——
    /// 10-22 拍板「破产时从持有地产中随机选 3 处逐项进行拍卖，其余归无主，股票归无主」。
    /// 之所以是「记一份料」而不是「当场开场」：开场要座位序（`TurnOrder`）与随机流，
    /// 而账本两条都不该拿（那会把每一条扣钱的路都变成拍卖机的借用冲突者）；
    /// 也之所以在这里记**全部**而不是只记那三块：抽哪三块是抽出来的，先替它保留全集。
    /// 记的是**地块号**而不是「地块号 + 层数」：层数此刻（以及此后）都还在世界上。
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
            let mut held: Vec<LandId> = Vec::new();
            for (_, id, mut owner, _, mut mortgage) in self.plots.iter_mut() {
                if owner.0 == Some(pid) {
                    held.push(*id);
                    owner.0 = None;
                    // 抵押跟着人走：人离场了，押在银行的地与那笔债一起清零
                    // （与 [`Self::damage`] 的夺地同一口径 —— 无主地上留着抵押等于替一个
                    // 不存在的主人挡租金）
                    mortgage.proceeds = 0;
                }
            }
            // 待拍的料按地块号排一遍再交出去：`plots` 那条查询的**迭代顺序**是按实体行号，
            // 而「哪一块排第一」是玩家会回头对的事（同一颗种子该拍出同一串拍卖）。
            // 让迭代顺序冒充拍卖顺序，等于把可复现性押在 spawn 次序上。
            held.sort_unstable_by_key(|land| land.0);
            // 名下有地才留料：一处空的清算会让拍卖机白转一圈，而那一圈末尾那句 `warn!`
            // 说的是「开不成场」—— 玩家听见的是假消息
            if !held.is_empty() {
                self.auction
                    .liquidation
                    .push(crate::auction::LiquidationPlan {
                        dead: player,
                        plots: held,
                    });
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
    /// 抵押本金只在**夺地**那一刻清：抵押中的地被 `Degrade` 或 `Clear{seize:false}` 打过
    /// 之后仍然是他的、仍然欠着 —— 房子被拆不该让债主免单。P9 在另一头把这条收干净了：
    /// **押着的地根本进不了拍卖**（`auction::OpenDeny::Mortgaged`），所以「债跟着地走」
    /// 这种说法在这里从来没有成立过。
    /// 反过来若地都无主了还留着抵押，`economy` 那三处「抵押中」判定就会替一个不存在的主人
    /// 挡掉租金。
    ///
    /// 不发消息：表现层读的是 `Owner`/`BuildingLevel` 的**变化**（`render::land::sync_buildings`
    /// 用 `Ref<T>` 检测），组件改了它就跟着改。等 P11 的新闻要播「外星人拆了哪几块」时
    /// 再补一条带地块清单的消息，而不是现在就为它准备一个没人读的字段。
    pub fn damage(&mut self, center: LandId, depth: LandDamage, area: PlotArea) -> DamageTally {
        let lands = area_of(&self.active.def, center, area);
        let tally = self.damage_lands(&lands, depth);
        info!(
            "建筑损害 {depth:?} × {area:?}（中心 {center:?}）：命中 {} 块、收回 {} 块",
            tally.hit, tally.seized
        );
        tally
    }

    /// 对**一组地块**打同一拳。P5 的定时炸弹走这一条：它的爆心是道路格，
    /// 3×3 要按网格算（`plot::lands_around_tile`），算完仍然只把地块集合交给账本 ——
    /// 范围算术只有一份，`damage` 与炸弹共用同一段写组件。
    pub fn damage_lands(&mut self, lands: &[LandId], depth: LandDamage) -> DamageTally {
        let mut tally = DamageTally::default();
        for &land in lands {
            let Some(entity) = self.active.lands.get(&land).copied() else {
                continue;
            };
            let Ok((_, _, mut owner, mut level, mut mortgage)) = self.plots.get_mut(entity) else {
                continue;
            };
            tally.hit += 1;
            let before = owner.0;
            damage::strike(&mut level.0, &mut owner.0, depth);
            if before.is_some() && owner.0.is_none() {
                tally.seized += 1;
                mortgage.proceeds = 0;
            }
        }
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

    /// 抵押一块名下的地（P3d③）：到手 `账面 × mortgage_percent / 100`，本金写在地块上。
    ///
    /// 与 [`Self::borrow`] 的分工：这条**不计息也不占可贷额度**，代价是这块地不收租、不算
    /// 垄断、也不再是抵押品（`economy::property_value` 把它排除，于是可贷额度同时掉下去）。
    /// 所以「先借后押」是显然的顺序，押是留给「额度借满了、或不想再滚利息」那一个人的出口。
    ///
    /// 窗口与资格的判据在调用方（`mortgage::mortgage_op_allowed`），这里只再认一次主语：
    /// 不是你的地、或已经押着 → `None`，钱与地块都不动。
    pub fn mortgage(&mut self, player: Entity, land: LandId) -> Option<i64> {
        let view = self.view(land)?;
        let pid = self.id_of(player)?;
        if view.owner != Some(pid) || view.mortgaged {
            return None;
        }
        let got = economy::mortgage_cashout(&self.active.def, &view, &self.rules);
        if got <= 0 {
            // 押不出钱的地若真写进 `Mortgaged`，就成了「押着、本金 0、还挡租金」的怪状态
            return None;
        }
        let entity = self.land_entity(land)?;
        let (_, _, _, _, mut mortgage) = self.plots.get_mut(entity).ok()?;
        mortgage.proceeds = got;
        self.credit(player, got);
        Some(got)
    }

    /// 赎回：付「本金 × `redeem_percent` / 100」，地块回到未抵押。
    ///
    /// 价按**当初到手那笔**算而不是今天重算一遍（理由见 [`crate::land::Mortgaged`]）。
    /// 钱不够 → `None`，**地还押着、一分不扣**（与 [`Self::debit`] 同一条不许半途的纪律）。
    pub fn redeem(&mut self, player: Entity, land: LandId) -> Option<i64> {
        let view = self.view(land)?;
        let pid = self.id_of(player)?;
        if view.owner != Some(pid) || !view.mortgaged {
            return None;
        }
        let entity = self.land_entity(land)?;
        let principal = self.plots.get(entity).ok()?.4.proceeds;
        let cost = economy::redeem_cost(principal, &self.rules);
        self.debit(player, cost).ok()?;
        let (_, _, _, _, mut mortgage) = self.plots.get_mut(entity).ok()?;
        mortgage.proceeds = 0;
        Some(cost)
    }

    /// 这块地押着多少本金；没押 → `None`。表现层与测试都问这一句，别去查组件。
    pub fn mortgage_principal(&self, land: LandId) -> Option<i64> {
        let entity = self.land_entity(land)?;
        let mortgage = self.plots.get(entity).ok()?.4;
        mortgage.held().then_some(mortgage.proceeds)
    }

    /// 赎回这块地要付多少（本金 × `redeem_percent` / 100）；没押 → `None`。
    ///
    /// 挂在账本上而不是让调用方各算一遍：本金只在 `Mortgaged` 组件里，而「谁能查组件」
    /// 这件事的边界就是这里（`mortgage::read_facts` 与手牌面板读的是同一个数）。
    pub fn redeem_cost_of(&self, land: LandId) -> Option<i64> {
        let principal = self.mortgage_principal(land)?;
        Some(economy::redeem_cost(principal, &self.rules))
    }

    /// 买地：扣**实付**价（地价 + 地上已有的每一层，见 [`Self::buy_price`]）→ 归属落到买家 →
    /// 等级**不动**（级数重判之后 0 = 空地：无主空地买下来还是 0；「无主却带楼」的
    /// 那些层（[`Self::bankrupt`] 只摘归属、不摘楼、P6c 天使白给的那一层）原样过户 ——
    /// 买主刚刚按 `estate_value` 为每一层付过钱，「置 0」等于把那栋楼抹平）。
    /// 付不起或已有人持有 → `None`，什么都不动。
    ///
    /// 价过一遍管线（物价指数 + 小福神「买地免费」那一类 `Cancel`），所以这里返回的
    /// 是实付额而不是表上的地价 —— 免费时是 0，`LandBought.price` 因此说的是「这笔账」。
    pub fn buy(&mut self, player: Entity, land: LandId) -> Option<i64> {
        let price = self.buy_price(player, land)?;
        let entity = self.land_entity(land)?;
        let pid = self.id_of(player);
        self.debit(player, price).ok()?;
        let (_, _, mut owner, _, _) = self.plots.get_mut(entity).ok()?;
        owner.0 = pid;
        Some(price)
    }

    /// 他身上挂着哪位神、还剩几格（`None` = 没被附身）。
    ///
    /// 挂在账本上而不是让面板自己开一条 `Query<&Possession>`：HUD 那一行「跟着谁、剩几天」
    /// 与顶栏那句播报要读的就是同一份事实（P4② 给 `Detention` 立的那条同一条理由）。
    pub fn god_of(&self, player: Entity) -> Option<crate::gods::Possession> {
        self.possessions.get(player).ok().copied()
    }

    /// 置产这一族的事（买地、加盖）此刻对他**办不成**吗 —— 大衰神／小衰神／死神的「必败」。
    ///
    /// 唯一读点。判据本身在 [`crate::gods::denies_acquire`] 那张表上，这里只负责把它接到
    /// 账本上：[`Books::buy_price`] 与 [`Books::upgrade`] 都问这一句，所以「报价的按钮」与
    /// 「成交的那一步」不可能一个说行一个说不行（P3d④a 给 `buy_price` 立的同一条纪律）。
    pub fn acquire_denied(&self, player: Entity) -> bool {
        self.possessions
            .get(player)
            .is_ok_and(|held| crate::gods::denies_acquire(held.god))
    }

    /// 这次加盖多盖几层（大福神＝1）。封顶那一位由调用方夹（见 [`Books::upgrade`]）。
    pub fn extra_levels(&self, player: Entity) -> u8 {
        self.possessions
            .get(player)
            .map_or(0, |held| crate::gods::extra_levels(held.god))
    }

    /// 这块地**此刻**对这位玩家而言买得下来吗？买下来实付多少？`None` = 买不了
    /// （棋盘上没这块地 / 已经有人持有 / 过了物价与修正之后的价付不起 / 他身上有衰神）。
    ///
    /// [`Self::buy`] 扣的就是这个数，所以它同时是「能不能买」与「买了要付多少」两份事实的
    /// 唯一来源：手牌浮层那颗 `→ 南京路 $1,200` 的按钮与购地卡的合法性判据读的都是这一句，
    /// 「按钮亮着而规则层拒了」与「按钮上的价与实际扣的价不一样」在结构上不可能发生。
    /// 报价侧照旧不花钱（`quoted` 只算不调账本）。
    ///
    /// **基数含地上已有的楼**（10-22 实报：无主地也会立着楼 —— 破产留下的那一手、天使白给
    /// 的那一层 —— 而「走到那格可以一次买下地+上面的建筑，地价和所有建筑升级费用相加」）。
    /// 用的就是 [`Books::market_value`] 那个基数 `economy::estate_value`：Lv0/Lv1 的地
    /// 它等于地价，所以普通空地的报价分文没动；差一层楼就差一笔加盖费，
    /// 而这笔钱本来就被算进这块地的账面（抵押、资产、起拍价三处早已这么读）。
    pub fn buy_price(&self, player: Entity, land: LandId) -> Option<i64> {
        let view = self.view(land)?;
        // 「必败」问在算价之前：它不是折扣，是这件事根本不给做
        if self.acquire_denied(player) {
            return None;
        }
        let base = economy::estate_value(&self.active.def, &view);
        let price = self.quoted(ChargeKind::BuyLand { land }, player, base);
        economy::can_buy(&view, price, self.balance(player)).then_some(price)
    }

    /// 升级：扣**实付**建筑费 + 等级 +1（大福神在场则 +2，见 [`Books::extra_levels`]）。
    /// 结构判定（业主本人、未抵押、未满级、垄断、这块地是组内最低层）全在 `economy::can_upgrade`。
    ///
    /// 多盖的那一层**不多收一次钱**：卡面那句话是「加盖多盖一层」，不是「一次买两层」——
    /// 付的是这一级的费，得到两级。这也是它与 [`Self::buy`] 一样只在账本里判一次的理由。
    pub fn upgrade(&mut self, player: Entity, land: LandId) -> Option<u8> {
        let view = self.view(land)?;
        let pid = self.id_of(player)?;
        if self.acquire_denied(player) {
            return None;
        }
        let base = economy::upgrade_cost(&self.active.def, &view)?;
        let cost = self.quoted(ChargeKind::Upgrade { land }, player, base);
        let views = self.views();
        let cash = self.balance(player);
        if !economy::can_upgrade(&view, pid, &views, cost, cash, &self.rules) {
            return None;
        }
        let entity = self.land_entity(land)?;
        self.debit(player, cost).ok()?;
        let max = self.rules.max_building_level;
        let extra = self.extra_levels(player);
        let (_, _, _, mut level, _) = self.plots.get_mut(entity).ok()?;
        // 封顶那一位上多出来的那一层少掉，不是缺陷：4 代也没有「盖穿顶」这件事
        level.0 = level.0.saturating_add(1 + extra).min(max);
        Some(level.0)
    }

    /// 建商业设施（P10c）：扣**实付**造价，返回它。设施本体的插入（`Facility` 组件）
    /// 在 `property::build_op_system` 里用 `Commands` —— 空商业地身上没有这个组件，
    /// 就地写无处可写，这是它与 [`Self::upgrade`]（`BuildingLevel` 恒在）的结构差异。
    ///
    /// 判据唯一读点是 [`crate::property::build_op_allowed`]（UI 的灰按钮喂同一张表）。
    /// 造价与升级同一格数据（`upgrade_cost`）—— 4 代里「在这块地上立一座购物中心」
    /// 与「加盖一级」是同一笔建设费，两条分叉只在判据与产物上，不在价格上。
    pub fn build(
        &mut self,
        player: Entity,
        land: LandId,
        kind: crate::board::BuildingKind,
    ) -> Result<i64, crate::property::BuildDeny> {
        let cost = self.build_probe(player, land, kind)?;
        self.debit(player, cost)
            .map_err(|_| crate::property::BuildDeny::ShortOfCash)?;
        Ok(cost)
    }

    /// 「这块地上建这一类的具体价钱」与「建不建得成」的同一个问法 —— 与 [`Self::build`]
    /// 只差最后那一下扣款。**UI 的灰按钮必须走这里**：判据与金额各写一遍，
    /// 就会出现「按钮写着 2,200、扣掉 2,600」那种面板与账本各说各话的面板
    /// （P9 拍卖台上那句「写的与扣的必须同一个表达式」是同一纪律）。
    pub fn build_probe(
        &self,
        player: Entity,
        land: LandId,
        kind: crate::board::BuildingKind,
    ) -> Result<i64, crate::property::BuildDeny> {
        let Some(view) = self.view(land) else {
            return Err(crate::property::BuildDeny::NoWindow);
        };
        let Some(pid) = self.id_of(player) else {
            return Err(crate::property::BuildDeny::NotOwner);
        };
        if view.owner != Some(pid) {
            return Err(crate::property::BuildDeny::NotOwner);
        }
        let Some(def_land) = self.active.def.land(land.0) else {
            return Err(crate::property::BuildDeny::NoWindow);
        };
        let occupied = self.facility_of(land).is_some();
        // 收费型设施（购物中心/旅馆/加油站）必须配了参数才能建 —— 建出一座
        // 「永远不收钱的旅馆」比拒绝它更糟；研究所/公园不收费，不需要条目
        let has_fee = match kind {
            crate::board::BuildingKind::Mall
            | crate::board::BuildingKind::Hotel
            | crate::board::BuildingKind::GasStation => {
                self.active.def.facility_fee(land.0, kind).is_some()
            }
            crate::board::BuildingKind::Lab | crate::board::BuildingKind::Park => true,
        };
        let base = economy::upgrade_cost(&self.active.def, &view).unwrap_or(0);
        let cost = self.quoted(ChargeKind::Build { land }, player, base);
        crate::property::build_op_allowed(
            view,
            def_land.land_type == crate::board::LandType::Commercial,
            occupied,
            has_fee,
            self.acquire_denied(player),
            cost,
            self.balance(player),
        )?;
        Ok(cost)
    }

    /// 这块地上现在立着什么（P10c）。空商业地**没有** `Facility` 组件，
    /// 所以 `None` 是「还没有设施」而不是查不到 —— 与 `Option<&T>` 那一条同形。
    pub fn facility_of(&self, land: LandId) -> Option<crate::board::BuildingKind> {
        self.land_entity(land)
            .and_then(|e| self.facilities.get(e).ok().flatten())
            .map(|f| f.0)
    }

    /// 土地公那一格：**直接占有**这块地（P6c）。返回是否办成。
    ///
    /// 与 [`Self::buy`] 分开写，理由与 [`Self::bless`] 同一条：买地那一整套（报价、物价、
    /// 修正、扣款、`LandBought`）说的都是「一笔交易成立」，而这里根本没有交易 —— 他踩上去，
    /// 地就归他。**过路费不在这里收**，由 `settle_property_landing` 照原样收
    /// （PLAN §三 那句「过路费照付原主」），所以这条只动归属那一半。
    ///
    /// 三种办不成，各有各的理由：
    /// - 棋盘上没这块地 / 已经是他的：无事可做（也不发凭据，免得顶栏播一句「他拿走了自己的地」）；
    /// - **押给银行的地不给占**：押到手那笔钱在原主钱包里，债不该跟着地走 ——
    ///   「资产与负债一起过户」这件事 P9 也没做（押着的地进不了拍卖，
    ///   见 `auction::OpenDeny::Mortgaged`）—— 两条入口同一条口径，债从不跟着地走；
    /// - 查封中**照样能占**：那条子挡的是施工与收租（见 `can_upgrade` 与 `calculate_rent`），
    ///   而土地公收的是这块地本身。
    pub fn take_land(&mut self, player: Entity, land: LandId) -> bool {
        let (Some(view), Some(pid)) = (self.view(land), self.id_of(player)) else {
            return false;
        };
        if view.owner == Some(pid) || view.mortgaged {
            return false;
        }
        let Some(entity) = self.land_entity(land) else {
            return false;
        };
        let Ok((_, _, mut owner, _, _)) = self.plots.get_mut(entity) else {
            return false;
        };
        owner.0 = Some(pid);
        true
    }

    /// 这块地**此刻值多少** —— 拍卖的起拍价（P9）。
    ///
    /// 与 [`Books::buy_price`] 是两句话。那一句问的是「**这位玩家**现在买它要付多少」，
    /// 含他身上的神与卡；这一句问的是「这块地本身值多少」。拍卖里把地卖出去的是**对手**
    /// 而不是系统，所以任何人的免费与折扣都不参与 —— 小福神在场时若按 `buy_price` 起拍，
    /// 一场拍卖会从 0 元开始喊，而那句「起拍 $12,000」就成了假话。
    ///
    /// 实现上是把 `Charge` 的 payer 留 `None`：管线里 `bearer()` 读不到人，于是只有
    /// 物价指数那一层乘得动（`charge::stage_and_spend` 里 `let Some(bearer) = ... else`
    /// 那一格），这不是取巧，是「指数是全场事实、修正是个人的事」这条分工的现成出口。
    ///
    /// 基数与 [`Books::buy_price`] 是**同一条式子**（`economy::estate_value`：地价 +
    /// 已盖的每一层），10-22 那句「起拍=买下它本来要付的钱」在代码里就落成两处读同一个函数。
    /// 而它含楼在 10-22 之后有了第二个理由：破产只摘归属、不摘楼（[`Self::bankrupt`]），
    /// 死者名下的楼会跟着地一起站在街上 —— 起拍价若不读世界上那几层，
    /// 就有人按空地价拍走一栋三层楼。旧版在这里额外接一个 `level` 参数、把「被摘之前」
    /// 那个数盖进只读快照，而那一格跟着「先清零再还」那一趟一起退掉了。
    pub fn market_value(&self, land: LandId) -> Option<i64> {
        let view = self.view(land)?;
        let base = economy::estate_value(&self.active.def, &view);
        Some(
            self.staged(Charge::new(ChargeKind::BuyLand { land }, None, None, base))
                .amount,
        )
    }

    /// 拍卖落槌那一步：扣买家的钱、把归属换给他。**楼什么都不必做**（10-22）：
    /// 破产只摘归属（[`Self::bankrupt`]），那块地上的楼从头到尾都立在世界上，
    /// 所以「建筑随土地一起过户」在这里只需要改 `Owner` 这一格 ——
    /// 旧版要在成交那一刻把 `Listing` 记着的层数还回去，那是上一版「先清零再还」
    /// 自己给自己造的活。
    ///
    /// `None` 返回 = 付不出，于是这一场流标（与 [`Books::buy`] 那条「报价已经给过而付不起就
    /// 什么也不发生」同一条口径 —— 而不是「先扣钱再退钱」那种两段式）。
    ///
    /// 刻意**不过结算管线**（理由见 [`crate::auction`] 模块头第三条）：价由竞价产生，
    /// 不是账单，物价再乘一次等于把别人喊出的数改写。
    ///
    /// 抵押进不来：开场那道闸门（`OpenDeny::Mortgaged`）已经把它挡掉了，所以这里不必再判一次
    /// —— 而 [`Self::take_land`] 那句「资产与负债一起过户是 P9 的账」到此有了答案：
    /// **不一起过**，押着的地根本不上拍。
    pub fn auction_sale(&mut self, buyer: Entity, land: LandId, price: i64) -> Option<i64> {
        let pid = self.id_of(buyer)?;
        self.debit(buyer, price).ok()?;
        let Some(entity) = self.land_entity(land) else {
            return Some(price);
        };
        if let Ok((_, _, mut owner, _, _)) = self.plots.get_mut(entity) {
            owner.0 = Some(pid);
        }
        Some(price)
    }

    /// 他身上那位神是不是「踩到的地直接归他」（土地公）。唯一读点，与 [`Self::acquire_denied`]
    /// 同一条纪律：判据在 [`crate::gods::takes_landing`] 那张表上，这里只把它接到账本上 ——
    /// 于是「弹不弹那张购买报价」与「过户不过户」不可能一个说行一个说不行。
    pub fn takes_landing(&self, player: Entity) -> bool {
        self.god_of(player)
            .is_some_and(|held| crate::gods::takes_landing(held.god))
    }

    /// 天使那一格：给这块地**白加一层楼**（P6c）。返回加完之后的等级，`None` = 没加成。
    ///
    /// 与 [`Self::upgrade`] 分开写，而不是给它加一个 `force: bool`：`upgrade` 那五道结构判定
    /// （业主本人、未抵押、未满级、垄断、组内最低层）加一次扣款，合起来是**一笔交易**的成立
    /// 条件，而祝福根本不是一笔交易 —— 它没有付款人，也不该有。共用一个函数迟早要为塞进管线
    /// 给某道判定开一个「例外」口子，而那条口子就是玩家白拿一层楼的入口（与 P6b② 那条
    /// 「必败不能写成 `Cancel`」是同一种「别把两件事塞进同一条判定」）。
    ///
    /// 只保留两道，且都不是「谁买得起」那种：
    /// - **封顶**（`max_building_level`）：与 `upgrade` 同一条理由，没有「盖穿顶」这件事；
    /// - **查封**：法院贴着条子的地不许施工，读的是 `can_upgrade` 里那同一条。
    ///
    /// **抵押不挡**：天使不给任何人发钱，也不替任何人解押 —— 押给银行的地长高一层，
    /// 债还是原额，那是原主的账，不是这次祝福的分支。
    pub fn bless(&mut self, land: LandId) -> Option<u8> {
        let view = self.view(land)?;
        if view.status == Some(LandStatus::Sealed) {
            return None;
        }
        let entity = self.land_entity(land)?;
        let max = self.rules.max_building_level;
        let (_, _, _, mut level, _) = self.plots.get_mut(entity).ok()?;
        level.0 = level.0.saturating_add(1).min(max);
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
