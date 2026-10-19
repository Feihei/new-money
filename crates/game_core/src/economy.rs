//! 经济核心的纯函数层：地产（租金/垄断/升级）、银行（存贷/利息/额度）、股票（波动/交易）。
//!
//! 两条刻意的设计约束：
//! 1. **只吃数字与 `LandView`，不碰组件、不发消息。** 于是 Godot 的三套经济用例
//!    （`test_property_system`/`test_bank_system`/`test_stock_market`）可以不经过 ECS
//!    直接移植成表驱动单测，也不必为「测一条租金分支」而装配整个世界。
//! 2. **银行与股票函数返回「计划额」，不改钱包。** 现金的唯一出口在 `settle::Books`
//!    （改 `Wallet` 必发 `CashChanged`），所以这里刻意不接受 `&mut Wallet` ——
//!    否则「算账顺手扣钱」会绕过流水，B3 的现金审计就失去意义。
//!
//! 缺陷 #1 的防线在签名上：`calc_max_loan` 的输入是 `LandView`（内部是 `LandId`），
//! 想把主环 `TileIndex` 喂进来编译不过 —— Godot 的 `bank_system.gd:99-105` 就是
//! 拿 registry 返回的字符串 key 去调 `get_tile(int)`，贷款上限因此恒为 0。

use std::collections::VecDeque;

use crate::board::BoardDef;
use crate::land::LandId;
use crate::player::PlayerId;
use crate::rng::Rng;
use crate::rules::GameRules;
use crate::status::LandStatus;
use crate::stock::StockDef;

/// 土地运行时状态的只读快照。由结算层从组件收集（地块数量 ≤ 正式版 40，无需省）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LandView {
    pub id: LandId,
    pub group: Option<String>,
    pub owner: Option<PlayerId>,
    pub level: u8,
    pub mortgaged: bool,
    /// 限时状态（涨价 / 查封，P3c）。`None` = 没挂着。租金与加盖判定都读它，
    /// 所以它必须在快照里 —— 判定函数不去查世界，才能做到「报价与成交同一个数」。
    pub status: Option<LandStatus>,
}

impl LandView {
    pub fn new(
        def: &BoardDef,
        id: LandId,
        owner: Option<PlayerId>,
        level: u8,
        mortgaged: bool,
        status: Option<LandStatus>,
    ) -> Self {
        Self {
            id,
            group: def.land(id.0).and_then(|l| l.group.clone()),
            owner,
            level,
            mortgaged,
            status,
        }
    }
}

// ===== 地产（`property_system.gd`）=====

/// 购买判定：无主 + 现金够（`property_system.gd:12-23`）。
/// 「玩家状态是否 active」由调用方保证：回合轮到的人不可能是破产者。
/// 能不能买这块地：无主，且**实际要付的价**在现金之内。
///
/// `price` 由调用方从修正管线拿（`Books::quoted`）—— 物价指数与免费卡都会改它，
/// 判定函数不该自己再去读一遍世界。价为 0（免费）当然是买得起。
pub fn can_buy(land: &LandView, price: i64, cash: i64) -> bool {
    land.owner.is_none() && cash >= price
}

pub fn land_price(def: &BoardDef, land: &LandView) -> Option<i64> {
    def.land(land.id.0).map(|d| d.price)
}

pub fn upgrade_cost(def: &BoardDef, land: &LandView) -> Option<i64> {
    def.land(land.id.0).map(|d| d.upgrade_cost)
}

/// 租金：无主/抵押 → 0；等级不足 1 按 1 算（空地也收 `rent[0]`，`property_system.gd:59-61`）；
/// 等级超出租金表长度 → 0（`property_system.gd:70` 的越界分支）；
/// 持有同色组全部未抵押地产 → ×`monopoly_rent_multiplier`（`property_system.gd:73-77`）。
pub fn calculate_rent(def: &BoardDef, land: &LandView, all: &[LandView], rules: &GameRules) -> i64 {
    let Some(owner) = land.owner else {
        return 0;
    };
    if land.mortgaged {
        return 0;
    }
    // 查封：法院贴着条子，这一趟谁都不收钱。放在租金表之前短路，是因为「零」不是
    // 从某个数乘出来的 —— 涨价与查封同时挂在一块地上这件事，单槽位已经让它不可能发生
    if land.status == Some(LandStatus::Sealed) {
        return 0;
    }
    let Some(defn) = def.land(land.id.0) else {
        return 0;
    };
    let level = land.level.max(1) as usize;
    let base = defn.rent.get(level - 1).copied().unwrap_or(0);
    // 垄断只对「真有个组」的地块生效：无组地块 Godot 也不翻倍（`property_system.gd:74` 的 `group != ""`）
    let monopoly = land
        .group
        .as_deref()
        .is_some_and(|g| owns_full_group(all, owner, g));
    let rent = if monopoly {
        base * rules.monopoly_rent_multiplier
    } else {
        base
    };
    // 涨价乘在垄断之后：4 代读出来是「这条街的过路费翻倍」，而街价本身已经含垄断加成。
    // 反过来先加倍再垄断是同一个数（都是乘），但顺序写清楚，将来加「涨价不叠加垄断」
    // 这种规则时只有一处要改
    match land.status {
        Some(LandStatus::PriceHike) => rent * rules.hike_rent_multiplier,
        Some(LandStatus::Sealed) | None => rent,
    }
}

/// 垄断判定：同色组内所有地块都被 `player` 持有且未抵押。
///
/// 与 Godot 的差异：无 `group` 的地块不属于任何组 —— 这里返回 `false`（不可垄断、
/// 不可升级），而 Godot 的 `_owns_full_group` 拿空串去匹配会得到「没有任何格子，
/// 于是全都满足」的隐式 `true`（`property_system.gd:151-161`），坏数据上等于白送升级权。
/// prototype 的六块地都有组，两条路在这里不分歧。
pub fn owns_full_group(all: &[LandView], player: PlayerId, group: &str) -> bool {
    let mut any = false;
    for l in all.iter().filter(|l| l.group.as_deref() == Some(group)) {
        any = true;
        if l.owner != Some(player) || l.mortgaged {
            return false;
        }
    }
    any
}

/// 组内等级差 ≤ 1（经典大富翁规则，`property_system.gd:164-178`）。空组返回 `false`。
pub fn group_levels_within_one(all: &[LandView], group: &str) -> bool {
    let mut min = u8::MAX;
    let mut max = 0;
    let mut any = false;
    for l in all.iter().filter(|l| l.group.as_deref() == Some(group)) {
        any = true;
        min = min.min(l.level);
        max = max.max(l.level);
    }
    any && (max - min) <= 1
}

/// 升级判定（`property_system.gd:105-125`）：**由 `by` 发起** + 未抵押 + 未满级 +
/// 垄断同色组 + 组内等级差 ≤1 + 现金够。
///
/// `by` 是刻意多出来的参数：Godot 的 `can_upgrade(player, tile_index, ...)` 第一件事就是
/// `is_owned_by_key(key, player.player_id)`。只看 `land.owner` 虽然在此刻等价
/// （落格分派只在「业主本人踩到自己地块」时才问升级），但把「谁在操作」藏进数据里，
/// B6 的 AI 一有代打场景就会误判 —— 显式参数让这条判定不能被越权调用。
///
/// `cost` 同理，是**实际要付的那个数**（物价指数与免费卡都会改它）。判定函数不去读世界，
/// 于是「模态上写的价」与「扣掉的价」共用同一个入参。
pub fn can_upgrade(
    land: &LandView,
    by: PlayerId,
    all: &[LandView],
    cost: i64,
    cash: i64,
    rules: &GameRules,
) -> bool {
    if land.owner != Some(by) {
        return false;
    }
    if land.mortgaged || land.level >= rules.max_building_level {
        return false;
    }
    // 查封中的地块连加盖都不许 —— 条子贴在那儿，你不能往法院封着的地上再盖一层。
    // 涨价不挡加盖：那张卡动的是「别人付多少」，不是这块地能不能施工。
    if land.status == Some(LandStatus::Sealed) {
        return false;
    }
    let Some(group) = land.group.as_deref() else {
        return false;
    };
    owns_full_group(all, by, group) && group_levels_within_one(all, group) && cash >= cost
}

// ===== 银行（`bank_system.gd`）=====

/// 未抵押地产总价：可贷额度的基数。抵押中的地块不计（`bank_system.gd:97-105` 的语义，
/// 与「抵押就是把地产押给银行」一致）。
pub fn property_value(def: &BoardDef, all: &[LandView], player: PlayerId) -> i64 {
    all.iter()
        .filter(|l| l.owner == Some(player) && !l.mortgaged)
        .map(|l| land_price(def, l).unwrap_or(0))
        .sum()
}

/// 名下地产的**名义**账面价值：地价 + 已花出去的加盖费。
///
/// 与 [`property_value`] 是两个数，而且必须保持是两个：贷款基数按 Godot 口径只看地价，
/// 而「全场有多少财富」要把房子算进去 —— 否则玩家每加盖一层就从指数里蒸发一笔钱，
/// **开发越猛、物价越低**，那与物价指数存在的理由正好相反。
/// 买地即 Lv1（`settle::buy` 钳到 1），所以已购的加盖次数是 `level - 1`。
///
/// 这里给的是**名义**值；物价指数要的是「按今天的价它还值多少」，所以调用方再乘一次
/// 上一回合的指数（见 `Books::total_assets`）。
pub fn estate_value(def: &BoardDef, land: &LandView) -> i64 {
    let price = land_price(def, land).unwrap_or(0);
    let built = land.level.saturating_sub(1) as i64;
    price + upgrade_cost(def, land).unwrap_or(0) * built
}

/// 一个人的现资产：现金 + 存款 − 负债 + 地产账面 + 股票市值。物价指数的分子按人加它。
///
/// `loan` 那一格**含抵押本金**（`Books::total_assets` 把两者加在一起传进来）：押出去换到的
/// 现金进了分子，那块地却还在分子里（可赎回），所以必须有一笔债把它抵平 ——
/// 少了这笔，「谁缺钱了」就在替「所有人的房租」定价。
pub fn net_worth(cash: i64, deposit: i64, loan: i64, estate: i64, stock_value: i64) -> i64 {
    cash + deposit - loan + estate + stock_value
}

/// 物价指数 = 现资产总和 ÷ 初始总和（PLAN §三）。不设上限（PC 原版口径）。
///
/// **地产按今天的价算**（分子里那块由调用方乘过上一回合的指数），于是「花钱买地」对指数
/// 是中性的一笔交易：现金少多少，账面就多多少。反过来，如果按名义价算，物价 2 倍时买一块
/// 地就凭空蒸发掉一半的差额，**全场越买越通缩** —— 那与「物价」两个字正好相反。
/// 中性还有一条好处：指数只因为**系统里的钱变多**而涨（过起点薪水、股息、破产把现金清零），
/// 玩家能看懂它为什么动。乘的是上一回合的指数而不是联立解，一步滞后不会发散：
/// 买地花的钱永远来自现金，`estate × index` 的上界就是付掉的那笔现金。
///
/// 分母为非正（空阵容、起始现金配成 0）时返回 1.0 —— 一个没有参照物的比值不该冒充
/// 「全场免费」。分子为负（全场赤贫到负债超过一切）按 0 算：指数不该是负的，
/// 那会把每一笔账都翻成负数。
pub fn price_index(total: i64, initial: i64) -> f32 {
    if initial <= 0 {
        return 1.0;
    }
    (total.max(0) as f64 / initial as f64) as f32
}

/// 最大可贷 = 未抵押地产总价 × 系数（`bank_system.gd:10` 的 `MAX_LOAN_RATIO`）。
pub fn calc_max_loan(def: &BoardDef, all: &[LandView], player: PlayerId, rules: &GameRules) -> i64 {
    let value = property_value(def, all, player);
    (value as f32 * rules.max_loan_ratio).floor() as i64
}

/// 抵押一块地到手多少：账面价值 × `mortgage_percent` / 100，**向下取整**。
///
/// 基数是 [`estate_value`]（地价 + 已花的加盖费）而不是只按地价 —— 加盖花掉的钱不该在抵押
/// 那一刻蒸发。而抵押中的地不能加盖（[`can_upgrade`] 那条），所以这个数一旦定下就冻结，
/// [`redeem_cost`] 才有「按本金加一成」可算。
///
/// **不乘物价指数**：这是一笔债，不是一份资产。可贷额度那条也是按名义地价算的（[`calc_max_loan`]），
/// 两个窗口按同一口径才不会互相套利。
pub fn mortgage_cashout(def: &BoardDef, land: &LandView, rules: &GameRules) -> i64 {
    estate_value(def, land).saturating_mul(rules.mortgage_percent as i64) / 100
}

/// 赎回要付多少：本金 × `redeem_percent` / 100，**向下取整**（本金 750、一成 → 825）。
///
/// 收的是**当初到手那笔**而不是「今天重算一遍」：`Mortgaged::proceeds` 存的就是它。
/// 重算在今天的规则下会得到同一个数，但那个「今天」是靠 `can_upgrade` 挡着加盖换来的 ——
/// 一笔债的价不该寄存在另一条规则的不变式上（见 `land::Mortgaged` 的注释）。
pub fn redeem_cost(principal: i64, rules: &GameRules) -> i64 {
    principal.saturating_mul(rules.redeem_percent as i64) / 100
}

/// 存款合法额：`amount > 0` 且现金够（`bank_system.gd:14-24`）。
pub fn deposit_plan(amount: i64, cash: i64) -> Option<i64> {
    (amount > 0 && cash >= amount).then_some(amount)
}

/// 取款合法额：`amount > 0` 且余额够（`bank_system.gd:28-37`）。
pub fn withdraw_plan(amount: i64, deposit: i64) -> Option<i64> {
    (amount > 0 && deposit >= amount).then_some(amount)
}

/// 贷款合法额：累计负债不超上限（`bank_system.gd:41-53`）。
pub fn loan_plan(amount: i64, loan_now: i64, max_loan: i64) -> Option<i64> {
    (amount > 0 && loan_now + amount <= max_loan).then_some(amount)
}

/// 还款合法额：多还的部分不收，实还 `min(amount, loan)`（`bank_system.gd:57-70`）。
pub fn repay_plan(amount: i64, cash: i64, loan: i64) -> Option<i64> {
    if amount <= 0 || loan <= 0 {
        return None;
    }
    let actual = amount.min(loan);
    (cash >= actual).then_some(actual)
}

/// 存款利息（**每月**一次，月末那天）：进余额不进现金（`bank_system.gd:80-82`）。
pub fn deposit_interest(deposit: i64, rules: &GameRules) -> i64 {
    if deposit <= 0 {
        return 0;
    }
    (deposit as f32 * rules.deposit_rate).floor() as i64
}

/// 贷款利息（**每月**一次，与储金同一天）：进负债不进现金（`bank_system.gd:84-87`）。
pub fn loan_interest(loan: i64, rules: &GameRules) -> i64 {
    if loan <= 0 {
        return 0;
    }
    (loan as f32 * rules.loan_rate).floor() as i64
}

// ===== 股票（`stock_market.gd`）=====

/// 推进一只股一个 tick：`new = old × (1 + base_drift + noise)`，
/// clamp 进区间、四舍五入到分（`stock_market.gd:68-78`）。返回 `(old, new)`。
pub fn stock_tick(price: &mut f32, def: &StockDef, rng: &mut Rng, rules: &GameRules) -> (f32, f32) {
    let old = *price;
    let noise = rng.range_f32(-def.volatility..=def.volatility);
    let mut new = old * (1.0 + rules.stock_base_drift + noise);
    new = new.clamp(def.price_floor, def.price_ceiling);
    new = (new * 100.0).round() / 100.0;
    *price = new;
    (old, new)
}

/// 每股股息 = `floor(股价 × rate)`（`stock_market.gd:171`）。
pub fn dividend_per_share(price: f32, rules: &GameRules) -> i64 {
    (price * rules.dividend_rate).floor() as i64
}

/// 记录收盘价并维持历史上限（`stock_market.gd:194-200`）。
pub fn record_price(history: &mut VecDeque<f32>, price: f32, max: usize) {
    history.push_back(price);
    while history.len() > max {
        history.pop_front();
    }
}

/// 买入成交价：向上取整（`stock_market.gd:103` 的 `ceili`）。
pub fn buy_cost(price: f32, shares: u32) -> i64 {
    (price * shares as f32).ceil() as i64
}

/// 卖出市值：向下取整（`stock_market.gd:137` 的 `floori`）。
pub fn sell_proceeds(price: f32, shares: u32) -> i64 {
    (price * shares as f32).floor() as i64
}

/// 买入计划：`(实际股数, 花费)`。现金不足时按余额折算可买股数，一次算清 ——
/// 修缺陷 #5：Godot `stock_market.gd:105-110` 现金不足时递归调用自身且无深度保护。
/// 一股都买不起、`want == 0` 或价格非法 → `None`（钱包与持仓不动）。
pub fn buy_plan(price: f32, cash: i64, want: u32) -> Option<(u32, i64)> {
    if want == 0 || price <= 0.0 {
        return None;
    }
    let mut n = want;
    if buy_cost(price, n) > cash {
        // floor 折算可能仍差一分钱（浮点/向上取整的边界），逐股退回直到付得起
        n = (cash as f32 / price).floor() as u32;
        while n > 0 && buy_cost(price, n) > cash {
            n -= 1;
        }
    }
    (n > 0).then(|| (n, buy_cost(price, n)))
}

/// 卖出计划：`(实际股数, 入账)`。最多卖掉持有量（`stock_market.gd:125-131`）；
/// 一股都不持有 → `None`。
pub fn sell_plan(price: f32, owned: u32, want: u32) -> Option<(u32, i64)> {
    let actual = owned.min(want);
    (actual > 0).then(|| (actual, sell_proceeds(price, actual)))
}

// ===== 日历 =====

/// 把「第几个行情 tick」换算成「第几月第几日」，两者都从 1 起。
///
/// 一天就是一个回合结束（`market::market_tick_on_turn_end_system` 在那里推进 tick），
/// 所以日历不需要另一条时钟 —— 玩家听到的「每月 1 号派息」与看到的行情更新是同一个数。
/// `tick 0` 是开局那天，即 1 月 1 日。`days_per_month == 0` 退化成「每月一天」而不是除零。
pub fn date_of(tick: u32, days_per_month: u32) -> (u32, u32) {
    let span = days_per_month.max(1);
    (tick / span + 1, tick % span + 1)
}

/// 今天是这周的第几天（1 起，1 = 周一）。`tick 0` 是开局那天 = 1 月 1 日 = 周一。
pub fn weekday_of(tick: u32, days_per_week: u32) -> u32 {
    let span = days_per_week.max(1);
    tick % span + 1
}

/// 周末吗（每周最后两天）。股市与银行这两天都不开门。
///
/// 周长按 7、月长按 12，两者不整除 —— 于是周末在月内逐月漂移，「这个月哪天办不了业务」
/// 不是一张能背下来的表。这是要的：日历得真的被看一眼。
pub fn is_weekend(tick: u32, rules: &GameRules) -> bool {
    let span = rules.days_per_week.max(1);
    weekday_of(tick, span) + 2 > span
}

/// 月末吗 —— 储金与贷款结息的那一天（PLAN §三「月末发 10% 储金利息」）。
///
/// 口径说清：回合结束时 tick 已经推进到**新的一天**，所以这条问的是「今天开局是不是月末」，
/// 于是利息在月末那天的**开头**到账，而不是月末过完才补。一次月结一次，不会重也不会漏。
pub fn is_month_end(tick: u32, rules: &GameRules) -> bool {
    date_of(tick, rules.days_per_month).1 >= rules.days_per_month.max(1)
}

/// 今天发不发股息。`dividend_day` 落在月长之外时按月末收拢 —— 配置写错该表现为
/// 「派息日挪到月末」，而不是「这局永远不发钱」那种静默失效。
pub fn is_dividend_day(tick: u32, rules: &GameRules) -> bool {
    let span = rules.days_per_month.max(1);
    let wanted = rules.dividend_day.clamp(1, span);
    date_of(tick, span).1 == wanted
}

/// 下一个派息日（给 UI 说「距派息还有几天」）。同一天不返回 0 —— 刚发过一次，
/// 屏幕上要说的是下一次，而不是「今天」（今天的那笔已经在现金里了）。
pub fn days_until_dividend(tick: u32, rules: &GameRules) -> u32 {
    let span = rules.days_per_month.max(1);
    let wanted = rules.dividend_day.clamp(1, span);
    let (_, today) = date_of(tick, span);
    let gap = (wanted + span - today) % span;
    if gap == 0 { span } else { gap }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::BoardDef;

    const PROTOTYPE: &str = include_str!("../../../assets/data/boards/prototype.json");

    fn board() -> BoardDef {
        BoardDef::from_json(PROTOTYPE).expect("prototype.json 应能解析")
    }

    fn rules() -> GameRules {
        GameRules::default()
    }

    fn p(i: u32) -> Option<PlayerId> {
        Some(PlayerId(i))
    }

    /// 全场快照的唯一构造入口：`(地块, 归属, 等级, 抵押)` 覆盖若干地块，其余无主空地。
    /// 被考察的那块必须也从这里取，否则「focal 说 P0 持有、快照说 P1 持有」会各说各话。
    ///
    /// 限时状态一律 `None`：既有那二十来条表驱动用例没有一条关心涨价/查封，为它们各补一个
    /// 位置会把改动面铺满。要状态的用例走 [`stamped`]。
    fn world(def: &BoardDef, rows: &[(u32, Option<PlayerId>, u8, bool)]) -> Vec<LandView> {
        def.lands
            .iter()
            .map(|l| match rows.iter().find(|r| r.0 == l.id) {
                Some((_, owner, level, mortgaged)) => {
                    LandView::new(def, LandId(l.id), *owner, *level, *mortgaged, None)
                }
                None => LandView::new(def, LandId(l.id), None, 0, false, None),
            })
            .collect()
    }

    /// 在一份现成的快照上给某块地盖限时状态。
    fn stamped(mut all: Vec<LandView>, id: u32, status: Option<LandStatus>) -> Vec<LandView> {
        if let Some(view) = all.iter_mut().find(|v| v.id.0 == id) {
            view.status = status;
        }
        all
    }

    /// 从快照里取某块地（顺带保证 `world()` 的顺序就是 `def.lands` 的顺序）
    fn at(all: &[LandView], id: u32) -> LandView {
        all.iter()
            .find(|l| l.id == LandId(id))
            .expect("prototype 里有这块地")
            .clone()
    }

    /// 名义升级费。这些表驱动的用例不套物价，所以「名义＝实付」；
    /// `can_upgrade` 现在收的是**实付**那个数（见它的注释），测试侧就照这里这样递。
    fn cost(def: &BoardDef, land: &LandView) -> i64 {
        upgrade_cost(def, land).unwrap_or(0)
    }

    /// prototype 的六块地：
    /// 0 南京路 brown 1500 / 1 汉口路 brown 1500 / 2 外滩商业区 orange 4400 /
    /// 3 徐家汇 red 3800 / 4 王府井 yellow 2800 / 5 西单 yellow 3000
    #[test]
    fn rent_follows_the_level_table() {
        let def = board();
        let r = rules();
        // land 0 的租金表 [60,180,450,900,1500,2500]
        let table: [(u8, i64); 6] = [(1, 60), (2, 180), (3, 450), (4, 900), (5, 1500), (6, 2500)];
        for (level, want) in table {
            let all = world(&def, &[(0, p(0), level, false)]);
            let land = at(&all, 0);
            assert_eq!(
                calculate_rent(&def, &land, &all, &r),
                want,
                "{level} 级租金应取 rent[{}]",
                level - 1
            );
        }
    }

    #[test]
    fn rent_edge_branches_are_all_pinned() {
        let def = board();
        let r = rules();

        // 无主 → 0（`property_system.gd:48-50`）
        let all = world(&def, &[(0, None, 3, false)]);
        let land = at(&all, 0);
        assert_eq!(calculate_rent(&def, &land, &all, &r), 0, "无主地产租金为 0");

        // 抵押 → 0（`:56-58`，Godot 的 test_rent_mortgaged_is_zero）
        let all = world(&def, &[(0, p(0), 3, true)]);
        let land = at(&all, 0);
        assert_eq!(calculate_rent(&def, &land, &all, &r), 0, "抵押中租金为 0");

        // 空地（level 0）按 1 级起租（`:59-61` 的 level<1 归一）
        let all = world(&def, &[(0, p(0), 0, false)]);
        let land = at(&all, 0);
        assert_eq!(calculate_rent(&def, &land, &all, &r), 60, "空地也收一档");

        // 等级越出租金表 → 0（`:70` 的 `level <= rent_array.size()` 反向）
        let all = world(&def, &[(0, p(0), 7, false)]);
        let land = at(&all, 0);
        assert_eq!(
            calculate_rent(&def, &land, &all, &r),
            0,
            "租金表只有 6 档，越界不该拿最后一档凑数"
        );
    }

    #[test]
    fn monopoly_doubles_rent_and_needs_the_whole_group() {
        let def = board();
        let r = rules();
        // brown 组 = land 0 + land 1
        let all = world(&def, &[(0, p(0), 1, false)]);
        let land = at(&all, 0);
        assert_eq!(
            calculate_rent(&def, &land, &all, &r),
            60,
            "只持有一半 → 不翻倍"
        );

        let all = world(&def, &[(0, p(0), 1, false), (1, p(0), 1, false)]);
        let land = at(&all, 0);
        assert_eq!(
            calculate_rent(&def, &land, &all, &r),
            60 * r.monopoly_rent_multiplier,
            "整组在手 → ×{}（`property_system.gd:75-77`）",
            r.monopoly_rent_multiplier
        );

        // 组里另一块被抵押 → 垄断失效（`_owns_full_group` 的 `is_mortgaged_by_key`）
        let all = world(&def, &[(0, p(0), 1, false), (1, p(0), 1, true)]);
        let land = at(&all, 0);
        assert_eq!(
            calculate_rent(&def, &land, &all, &r),
            60,
            "组内有抵押 → 不翻倍"
        );

        // 组里另一块在别人手里 → 同样不翻倍
        let all = world(&def, &[(0, p(0), 1, false), (1, p(1), 1, false)]);
        let land = at(&all, 0);
        assert_eq!(
            calculate_rent(&def, &land, &all, &r),
            60,
            "组内有他人 → 不翻倍"
        );
    }

    #[test]
    fn a_hiked_street_doubles_and_a_sealed_one_collects_nothing() {
        let def = board();
        let r = rules();
        // 整组在手（垄断 60×2=120）之上再涨价。乘在垄断之后是 `calculate_rent` 里写死的
        // 顺序，这条把它钉住：两个倍数若共用一个数，这里先红
        let all = stamped(
            world(&def, &[(0, p(0), 1, false), (1, p(0), 1, false)]),
            0,
            Some(LandStatus::PriceHike),
        );
        assert_eq!(
            calculate_rent(&def, &at(&all, 0), &all, &r),
            120 * r.hike_rent_multiplier,
            "涨价乘在垄断加成之后"
        );
        assert_eq!(
            calculate_rent(&def, &at(&all, 1), &all, &r),
            120,
            "涨价是按块盖的，没盖到的那块照旧"
        );

        // 查封：零，而且是在查租金表之前短路 —— 于是「查封 × 涨价」这种该由单槽位挡掉的
        // 组合在这里读不出第三种数（真出现时它是零，不是「翻过倍再归零」的中间态）
        let all = stamped(
            world(&def, &[(0, p(0), 1, false), (1, p(0), 1, false)]),
            0,
            Some(LandStatus::Sealed),
        );
        assert_eq!(
            calculate_rent(&def, &at(&all, 0), &all, &r),
            0,
            "法院贴着条子的这一趟谁都不收钱"
        );
    }

    #[test]
    fn a_sealed_plot_cannot_be_built_on_while_a_hiked_one_can() {
        let def = board();
        let r = rules();
        let me = PlayerId(0);
        let rows = [(0, p(0), 1, false), (1, p(0), 1, false)];

        let sealed = stamped(world(&def, &rows), 0, Some(LandStatus::Sealed));
        assert!(
            !can_upgrade(
                &at(&sealed, 0),
                me,
                &sealed,
                cost(&def, &at(&sealed, 0)),
                99_999,
                &r
            ),
            "查封中的地块不许加盖"
        );

        let hiked = stamped(world(&def, &rows), 0, Some(LandStatus::PriceHike));
        assert!(
            can_upgrade(
                &at(&hiked, 0),
                me,
                &hiked,
                cost(&def, &at(&hiked, 0)),
                99_999,
                &r
            ),
            "涨价动的是「别人付多少」，不该挡住业主自己加盖"
        );
    }

    #[test]
    fn single_land_group_monopolises_once() {
        let def = board();
        let r = rules();
        // red 组只有 land 3 一块：整组在手即垄断，翻倍恰好一次（不是 ×2×2）
        let all = world(&def, &[(3, p(0), 1, false)]);
        let land = at(&all, 3);
        assert!(owns_full_group(&all, PlayerId(0), "red"));
        assert_eq!(calculate_rent(&def, &land, &all, &r), 240 * 2);
    }

    #[test]
    fn upgrade_needs_monopoly_and_level_spread_within_one() {
        let def = board();
        let r = rules();
        let max = r.max_building_level;
        let me = PlayerId(0);

        // 只拿到 brown 组的一半 → 不可升级（Godot 的 test_cant_upgrade_without_monopoly）
        let all = world(&def, &[(0, p(0), 1, false)]);
        let land = at(&all, 0);
        assert!(!can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r));

        // 整组在手 → 可升级；现金不够（land 0 的 upgrade_cost=800）→ 不可
        let all = world(&def, &[(0, p(0), 1, false), (1, p(0), 1, false)]);
        let land = at(&all, 0);
        assert!(
            can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r),
            "垄断 → 可升级"
        );
        assert!(
            !can_upgrade(&land, me, &all, cost(&def, &land), 799, &r),
            "差一口气也不可"
        );

        // 组内等级差 2 → 低的那块不许再拉开（`property_system.gd:164-178`）
        let all = world(&def, &[(0, p(0), 1, false), (1, p(0), 3, false)]);
        let land = at(&all, 0);
        assert!(
            !can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r),
            "等级差必须 ≤1"
        );

        // 差 1 时低的那块可升（升到与高的齐平）
        let all = world(&def, &[(0, p(0), 2, false), (1, p(0), 3, false)]);
        let land = at(&all, 0);
        assert!(
            can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r),
            "差 1 时低的那块可升"
        );

        // 满级不可再升（Godot 的 test_cant_upgrade_beyond_6）
        let all = world(&def, &[(0, p(0), max, false), (1, p(0), max, false)]);
        let land = at(&all, 0);
        assert!(
            !can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r),
            "已达 {max} 级 → 不可升级"
        );

        // 抵押中不可升
        let all = world(&def, &[(0, p(0), 3, true), (1, p(0), 3, false)]);
        let land = at(&all, 0);
        assert!(
            !can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r),
            "抵押中不可升级"
        );

        // 整组都在 P1 手里时，P0 说了不算（`is_owned_by_key`：越权的升级请求）
        let all = world(&def, &[(0, p(1), 1, false), (1, p(1), 1, false)]);
        let land = at(&all, 0);
        assert!(
            !can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r),
            "非业主不可升级"
        );
        assert!(
            can_upgrade(&land, PlayerId(1), &all, cost(&def, &land), 99_999, &r),
            "同一个人格检查换回真业主就该放行"
        );
    }

    #[test]
    fn groupless_land_gets_no_free_monopoly() {
        let def = board();
        let r = rules();
        let me = PlayerId(0);

        // 无主地块不可升级
        let all = world(&def, &[(0, None, 1, false)]);
        let land = at(&all, 0);
        assert!(!can_upgrade(&land, me, &all, cost(&def, &land), 99_999, &r));

        // 造一块没有 group 的地：Godot 会因「空串匹配不到任何格」隐式放行，
        // 这里明确拒绝（见 owns_full_group 的注释）
        let mut def2 = board();
        def2.lands[0].group = None;
        let all2 = world(&def2, &[(0, p(0), 1, false)]);
        let land2 = at(&all2, 0);
        assert!(
            !can_upgrade(&land2, me, &all2, cost(&def2, &land2), 99_999, &r),
            "无组地块不该白送升级权"
        );
        assert_eq!(
            calculate_rent(&def2, &land2, &all2, &r),
            60,
            "无组地块也不该翻倍的租金"
        );
    }

    #[test]
    fn buy_needs_an_unowned_land_and_enough_cash() {
        let def = board();
        let all = world(&def, &[]);
        let free = at(&all, 0);
        assert!(can_buy(&free, 1_500, 1_500), "地价 1500，刚好够");
        assert!(!can_buy(&free, 1_500, 1_499), "差一块钱也不算够");
        // 免费卡把价削到 0：付得起是必然的，归属判定不该反过来把它挡掉
        assert!(can_buy(&free, 0, 0), "价为 0 也是买得起");

        let all = world(&def, &[(0, p(1), 1, false)]);
        let taken = at(&all, 0);
        assert!(!can_buy(&taken, 1_500, 99_999), "已有人持有 → 不可买");
    }

    /// 买地中性：按**缩放价**付钱、按**缩放价**计值，全场总财富不该动。
    ///
    /// 这条是物价指数不自毁的关键。地产若按名义价计值，物价 1.5 时每成交一笔全场
    /// 就凭空蒸发半笔，指数一路往下掉 —— 「越买越通缩」与「物价」两个字正好相反。
    /// 放在纯函数层而不是集成测试里，是因为集成路径上机会/命运卡会从系统外掏钱进来，
    /// 那是合法的通胀来源，两种效应混在一条断言里就谁都证不了。
    #[test]
    fn buying_at_the_scaled_price_is_value_neutral() {
        let def = board();
        let all = world(&def, &[]);
        let free = at(&all, 0);
        let index = 1.5;

        let nominal = land_price(&def, &free).expect("prototype 的 L0 有地价");
        let paid = (nominal as f64 * index).round() as i64;
        assert_eq!(paid, 2_250, "1500 的地在物价 1.5 时该收 2250");

        let before = net_worth(20_000, 0, 0, 0, 0);
        let estate = (estate_value(&def, &free) as f64 * index).round() as i64;
        let after = net_worth(20_000 - paid, 0, 0, estate, 0);
        assert_eq!(before, after, "买地是现金换账面，不该创造也不该蒸发财富");
    }

    /// 加盖同理：付掉缩放后的建筑费，账面按同一口径多一层。
    #[test]
    fn upgrading_is_value_neutral_too() {
        let def = board();
        let all = world(&def, &[(0, p(1), 1, false)]);
        let one = at(&all, 0);
        let mut two = one.clone();
        two.level = 2;

        let index = 2.0;
        let cost = (upgrade_cost(&def, &one).unwrap() as f64 * index).round() as i64;
        let before = estate_value(&def, &one) as f64 * index;
        let after = estate_value(&def, &two) as f64 * index;
        assert_eq!(
            (after - before).round() as i64,
            cost,
            "加盖一层的账面增量必须等于实付，否则开发本身就在改指数"
        );
    }

    /// 指数的两条退化输入：没有参照物时不许冒充「全场免费」，负债超过一切时按 0 收。
    #[test]
    fn the_index_degrades_sensibly_on_empty_inputs() {
        assert_eq!(price_index(0, 0), 1.0, "分母为 0 不该给出 0 或 NaN");
        assert_eq!(price_index(0, -100), 1.0, "分母为负同理");
        assert_eq!(
            price_index(-5_000, 40_000),
            0.0,
            "全场赤贫到负债，指数收在 0 而不是负数"
        );
        assert_eq!(price_index(80_000, 40_000), 2.0);
    }

    /// 缺陷 #1 的回归网：贷款额度的基数是**土地**总价。
    /// Godot 拿 `get_owned_properties()` 返回的字符串 key 去调 `get_tile(int)`，
    /// 恒取不到地块 → 上限恒为 0 → 贷款功能实际不可用。
    #[test]
    fn max_loan_comes_from_land_prices() {
        let def = board();
        let r = rules();

        let all = world(&def, &[(0, p(0), 1, false), (1, p(0), 1, false)]);
        // brown 两块各 1500 → 3000 × 2.0
        assert_eq!(property_value(&def, &all, PlayerId(0)), 3_000);
        assert_eq!(calc_max_loan(&def, &all, PlayerId(0), &r), 6_000);

        // 抵押中的地块不计入基数（押给银行的东西不能再押一次）：1500 × 2.0
        let all = world(&def, &[(0, p(0), 1, false), (1, p(0), 1, true)]);
        assert_eq!(property_value(&def, &all, PlayerId(0)), 1_500);
        assert_eq!(calc_max_loan(&def, &all, PlayerId(0), &r), 3_000);

        // 别人的一块都不算；一块没有 → 上限 0（Godot 的 bug 是让所有人都得到 0）
        assert_eq!(calc_max_loan(&def, &all, PlayerId(7), &r), 0);
        let empty = world(&def, &[]);
        assert_eq!(calc_max_loan(&def, &empty, PlayerId(0), &r), 0);
    }

    #[test]
    fn bank_plans_reject_illegal_amounts() {
        assert_eq!(
            deposit_plan(0, 1_000),
            None,
            "存 0 元要拒绝（Godot 的 test_deposit_zero）"
        );
        assert_eq!(deposit_plan(-5, 1_000), None);
        assert_eq!(deposit_plan(5_000, 20_000), Some(5_000));
        assert_eq!(deposit_plan(20_000, 20_000), Some(20_000), "存空合法");
        assert_eq!(deposit_plan(20_001, 20_000), None, "现金不足 → 拒");

        assert_eq!(withdraw_plan(5_000, 3_000), None, "余额不足 → 拒");
        assert_eq!(withdraw_plan(3_000, 3_000), Some(3_000), "取空合法");
        assert_eq!(withdraw_plan(0, 3_000), None);

        assert_eq!(loan_plan(4_000, 0, 4_000), Some(4_000), "顶格可贷");
        assert_eq!(loan_plan(4_001, 0, 4_000), None, "超限应拒贷");
        assert_eq!(loan_plan(1, 4_000, 4_000), None, "已顶格再加一块也拒");
        assert_eq!(loan_plan(0, 0, 4_000), None);

        assert_eq!(
            repay_plan(2_000, 20_000, 1_000),
            Some(1_000),
            "多还部分不收"
        );
        assert_eq!(repay_plan(2_000, 500, 1_000), None, "现金不够还款 → 拒");
        assert_eq!(repay_plan(100, 20_000, 0), None, "无债可还 → 拒");
        assert_eq!(repay_plan(-1, 20_000, 1_000), None);
    }

    #[test]
    fn interest_compounds_into_the_balance_not_into_cash() {
        let r = rules();
        assert_eq!(deposit_interest(10_000, &r), 1_000, "月末储金 10% 进余额");
        assert_eq!(loan_interest(10_000, &r), 1_500, "月末贷款 15% 进负债");
        assert_eq!(deposit_interest(0, &r), 0);
        assert_eq!(loan_interest(-5, &r), 0);
        // 不足一分利息时向下取整 → 小额存款可能原地不动，这是 Godot 的 `floori` 语义
        // 月息 10% 下要存满 10 块才生得出 1 块，所以 9 是「一辈子拿不到利息」的那类存款
        assert_eq!(deposit_interest(9, &r), 0);
    }

    /// 贷款利率必须**高于**储金利率。两个数一旦倒过来，「借满再存进银行」就是无风险套利，
    /// 而规则层没有任何东西挡得住它 —— 玩家五分钟能发现，AI 第一天就会这么干。
    /// P2④ 把两条利率从「每回合」改成「每月」时正好把它们拧反过一次，所以这条留在网里。
    #[test]
    fn borrowing_to_deposit_earns_nothing() {
        let r = rules();
        assert!(
            r.loan_rate > r.deposit_rate,
            "贷款利率 {:?} 没高于储金利率 {:?}",
            r.loan_rate,
            r.deposit_rate
        );
        for principal in [1_000, 10_000, 100_000, 1_000_000] {
            assert!(
                loan_interest(principal, &r) >= deposit_interest(principal, &r),
                "{principal} 存进去赚的比借出来付的多 —— 白捡一条财路"
            );
        }
    }

    #[test]
    fn stock_price_stays_clamped_and_rounded() {
        let stock = StockDef {
            id: "s".into(),
            name: "测试股".into(),
            base_price: 80.0,
            volatility: 0.12,
            price_floor: 10.0,
            price_ceiling: 400.0,
            outstanding_shares: 1000,
        };
        let r = rules();
        let mut rng = Rng::new(7);
        let mut price = stock.base_price;
        for i in 0..500 {
            let (old, new) = stock_tick(&mut price, &stock, &mut rng, &r);
            assert!(
                (stock.price_floor..=stock.price_ceiling).contains(&new),
                "第 {i} 次 tick 后价格 {new} 出界（从 {old}）"
            );
            assert_eq!(
                new,
                (new * 100.0).round() / 100.0,
                "价格必须精确到分（`stock_market.gd:77`）"
            );
        }
    }

    #[test]
    fn clamp_holds_at_both_extremes() {
        // 一直涨到天花板 / 一直跌到地板：区间约束在极端值上仍然闭合
        let stock = StockDef {
            id: "s".into(),
            name: "测试股".into(),
            base_price: 399.0,
            volatility: 1.0,
            price_floor: 10.0,
            price_ceiling: 400.0,
            outstanding_shares: 1000,
        };
        let r = rules();
        let mut rng = Rng::new(1);
        let mut price = stock.base_price;
        for _ in 0..200 {
            let (_, new) = stock_tick(&mut price, &stock, &mut rng, &r);
            assert!((10.0..=400.0).contains(&new), "极端波动下出界：{new}");
        }
    }

    #[test]
    fn price_history_keeps_only_the_newest_window() {
        let mut hist = VecDeque::new();
        for i in 0..60 {
            record_price(&mut hist, i as f32, 50);
        }
        assert_eq!(hist.len(), 50, "历史上限 50 条（`stock_market.gd:17`）");
        assert_eq!(hist.front(), Some(&10.0), "丢的是最旧的");
        assert_eq!(hist.back(), Some(&59.0));
    }

    #[test]
    fn buy_plan_fills_partial_once_instead_of_recursing() {
        // 缺陷 #5 的回归网：现金只够一部分时一次算清，且绝不超付
        assert_eq!(buy_plan(80.0, 20_000, 10), Some((10, 800)));
        assert_eq!(buy_plan(80.0, 50, 10), None, "一股都买不起 → 0 股");
        assert_eq!(buy_plan(80.0, 800, 10), Some((10, 800)), "刚好够");
        assert_eq!(
            buy_plan(80.0, 750, 10),
            Some((9, 720)),
            "付不起 10 股就按余额买 9 股，不再递归"
        );
        assert_eq!(buy_plan(80.0, 20_000, 0), None, "买 0 股 → 无成交");
        assert_eq!(buy_plan(0.0, 20_000, 10), None, "非法价格不成交");
        assert_eq!(buy_plan(-1.0, 20_000, 10), None);
        // 向上取整的边界：10.5 × 2 = 21.0，现金 21 恰好够
        assert_eq!(buy_plan(10.5, 21, 2), Some((2, 21)));
        assert_eq!(buy_plan(10.5, 20, 2), Some((1, 11)), "差一角钱退到 1 股");
    }

    #[test]
    fn buy_plan_never_asks_for_more_than_the_wallet_has() {
        // 扫一片价格/现金组合，验证「花费 ≤ 现金」这条硬性质（超付 = 负现金 = 缺陷 #3 的源头）
        for cents in 1..40u32 {
            let price = cents as f32 / 3.0;
            for cash in [0i64, 1, 7, 99, 1_000, 19_999] {
                if let Some((shares, cost)) = buy_plan(price, cash, 500) {
                    assert!(shares > 0, "{price}/{cash} 成交 0 股？");
                    assert!(
                        cost <= cash,
                        "{price}×{shares} = {cost} 超付（现金 {cash}）"
                    );
                }
            }
        }
    }

    #[test]
    fn sell_plan_caps_at_holdings() {
        assert_eq!(sell_plan(80.0, 5, 100), Some((5, 400)), "最多只能卖 5 股");
        assert_eq!(sell_plan(80.0, 0, 10), None, "一股不持有 → 不成交");
        assert_eq!(sell_plan(80.0, 5, 0), None, "卖 0 股 → 无成交");
        assert_eq!(sell_plan(10.5, 3, 3), Some((3, 31)), "市值向下取整");
        assert_eq!(sell_proceeds(10.5, 1), 10);
        assert_eq!(
            buy_cost(10.5, 1),
            11,
            "买 ceil、卖 floor 的不对称是 Godot 原样（价差即交易成本）"
        );
    }

    #[test]
    fn dividend_is_floor_of_price_times_rate() {
        let r = rules();
        assert_eq!(dividend_per_share(80.0, &r), 1, "80×1.5%=1.2 → 1");
        assert_eq!(dividend_per_share(66.0, &r), 0, "不足一分不给");
        assert_eq!(dividend_per_share(400.0, &r), 6);
    }

    #[test]
    fn the_calendar_counts_months_and_days_from_day_one() {
        // 12 天一月：tick 0 就是 1 月 1 日，满一月翻到 2 月 1 日
        assert_eq!(date_of(0, 12), (1, 1));
        assert_eq!(date_of(11, 12), (1, 12));
        assert_eq!(date_of(12, 12), (2, 1));
        assert_eq!(date_of(25, 12), (3, 2));
        assert_eq!(date_of(3, 0), (4, 1), "月长写 0 退化成每月一天，不能除零");
    }

    #[test]
    fn dividend_day_is_a_date_not_a_tick_modulus() {
        let r = GameRules {
            days_per_month: 3,
            dividend_day: 2,
            ..rules()
        };
        // 3 天一月、2 号派息 → tick 1、4、7…（1 月 2 日、2 月 2 日…）
        assert!(!is_dividend_day(0, &r), "开局那天不算一次派息");
        assert!(is_dividend_day(1, &r));
        assert!(!is_dividend_day(2, &r));
        assert!(!is_dividend_day(3, &r), "3 号是月末");
        assert!(is_dividend_day(4, &r), "2 月 2 日");
    }

    #[test]
    fn a_dividend_day_outside_the_month_moves_to_month_end() {
        // 配置写成「1 月 45 号」：应挪到月末发，而不是这局永远不发钱
        let r = GameRules {
            days_per_month: 6,
            dividend_day: 45,
            ..rules()
        };
        assert!(is_dividend_day(5, &r), "应收拢成每月 6 号");
        assert!(is_dividend_day(11, &r));
        assert!(!is_dividend_day(4, &r));
    }

    #[test]
    fn countdown_never_reports_zero() {
        let r = GameRules {
            days_per_month: 12,
            dividend_day: 1,
            ..rules()
        };
        assert_eq!(days_until_dividend(0, &r), 12, "今天就是派息日也说下一次");
        assert_eq!(
            days_until_dividend(5, &r),
            7,
            "1 月 6 日 → 2 月 1 日还差 7 天"
        );
        assert_eq!(days_until_dividend(11, &r), 1, "1 月 12 日是明天就派息");
        assert_eq!(days_until_dividend(12, &r), 12, "2 月 1 日刚发过");
    }
}
