//! 规则层唯一的随机性来源。
//!
//! Godot 端骰子用全局 `randi_range`（`dice.gd:19`）、洗牌用 `_rng.randomize()`
//! （`card_deck.gd:53`），两者都不可复现，而 `roll_seeded()` 只是个 `assert` 后原样返回的
//! 假种子函数（缺陷 #6）。这里整局共用一个种子，于是「同种子 → 同对局」变成一条
//! 可以断言的性质 —— 回放与失败复现都从这里长出来。
//!
//! **同一个种子分成二十条互不相干的子流**（骰子 / 牌堆 / 行情 / 百货 / 历法 / 礼物 / 红包 /
//! 宝箱位置 / 宝箱掉落 / 神明 / 小游戏 / 乐透 / 拍卖 / 服务收费 / 董事长白拿 / 新闻 / 恶人 /
//! 魔法屋 / 开局落位 / AI）：抽一张卡不该让下一手骰点变样。若共用一条流，经济系统每多一次
//! 取随机数就会把 `turn_flow` 里「对局骰点 == 独立掷出的骰点」那批回归网整片打翻，
//! 回放也对不齐；分流之后每域的随机性各自稳定，加内容不互相扰动。
//!
//! 百货那条流是 P3d② 加的，理由与牌堆分流同一条：**「这一局进过几次店」不该改变下一张
//! 机会牌是谁**。把它并进牌堆流也能跑，只是换货会悄悄挪动抽卡顺序，而那正是分流要挡掉的
//! 那种耦合。P2b 的历法 / 礼物 / 红包三条同一条理由：开局随机到几月、圣诞发过几张卡、
//! 春节抽过几个红包，都不该改变牌序与骰点 —— 否则同一个 `NM_SEED` 在收礼物之后就走样，
//! 截图核对与回放双双失效。

use std::ops::RangeInclusive;

use bevy::prelude::*;
use rand::{Rng as _, SeedableRng as _};
use rand_chacha::ChaCha8Rng;

/// 单颗六面骰（`dice.gd:15`）。大富翁4 规则：无双骰、无双数额外回合。
pub const DIE_FACES: u32 = 6;

/// 子流的域标签：七条流要取到互不相关的密钥流，高位错开即可。
const CARDS_TAG: u64 = 0xCA4D_5EED_0000_0001;
const MARKET_TAG: u64 = 0x0A0E_7700_0000_0002;
const SHOP_TAG: u64 = 0x5B0F_5EED_0000_0003;
const EPOCH_TAG: u64 = 0xEC40_C000_0000_0004;
const GIFT_TAG: u64 = 0x61F7_0000_0000_0005;
const ENVELOPE_TAG: u64 = 0xE2E2_0000_0000_0006;
/// P5：宝箱刷在哪几格。与掉落分成两条 —— 开局撒几个宝箱不该改变第一个宝箱开出什么。
const SPAWN_TAG: u64 = 0x5BA0_0000_0000_0007;
/// P5：宝箱开出哪一种道具。
const LOOT_TAG: u64 = 0x1007_0000_0000_0008;
/// P6：刷哪位神、它落在哪一格、它自己往哪边走、财神给的那位数。
///
/// 神单独一条的理由与百货那一条同形：**「这一局撞见几位神」不该改变下一张机会牌是谁，
/// 也不该改变下一个红包是几**。附身还有一层别处没有的耦合 —— 神每回合都要挪一格，
/// 于是它是这条流上唯一一个「随回合稳定增加取数次数」的来源；把它并进骰子流，
/// 第 30 回合之后的骰点就会随「前面几位神各晃了几步」漂移，回放当场对不齐。
const GOD_TAG: u64 = 0x60D0_0000_0000_0009;
/// P7：游乐场这一趟玩哪一款、以及那一局的布局种子。
///
/// 单独一条的理由与百货、神明那两条同形：**「这一局进过几次游乐场」不该改变下一张机会牌
/// 是谁，也不该改变下一手骰点**。而布局种子那一半更要紧 —— 挖宝的 16 格里埋着几格宝
/// 直接决定得分、得分换点券、点券是账，所以它必须是可复现的（同一个 `NM_SEED` 走到同一格
/// 摆出同一局），同时又不能去挪动别的流。
const MINIGAME_TAG: u64 = 0xA4CA_D000_0000_000A;
/// P8：乐透开出的那个号。
///
/// 单独一条的理由与前面每一条同形，而这里多一层：**开奖不该改变任何别的东西**。把开奖号
/// 并进骰子流，「这个月开了几号」就会挪动下一手骰点 —— 而乐透是一月一次、骰子是一回合
/// 好几次，两者的取数节奏差着两个数量级，混在一条流上等于让月度事件去扰动逐回合的可复现性。
const LOTTERY_TAG: u64 = 0x1077_0000_0000_000B;
/// P9：一处破产清算里**拍哪几块地、按什么次序拍**。
///
/// 这条流单独开的原因与前面每一条同形，而 P9 多一层要防的东西：拍卖的次序是玩家会回头对的
/// 事（「我那块地为什么是第二场」），若让它跟着「这一局进过几次店」或「刷过几次神」漂移，
/// 同一颗种子的两次跑法会拍出两块不同的地 —— 那比拍哪几块更难查。
const AUCTION_TAG: u64 = 0xA71C_0000_0000_000C;
/// P10：服务收费原型摇出的那个数（购物中心掷几点、旅馆住几天、航空转盘停几段）。
///
/// 单独一条的理由与百货、神明那两条同形，而这里多一层：**这一摇发生在落格结算的那一帧**，
/// 与骰子、抽卡、神明挪格同帧并发 —— 并进任何一条已有的流，「这一趟踩没踩购物中心」
/// 都会挪动别人的随机数。而摇出几点是玩家会回头对的事（「我上次明明摇出 5」），
/// 同一颗种子必须两次走出同一个数。
const SERVICE_TAG: u64 = 0x5E3C_0000_0000_000E;
/// P10d②：董事长在百货白拿的那一件。
///
/// 单独一条的理由与百货、神明那两条同形：**「董事长进过几次店」不该改变下一张机会牌
/// 是谁，也不该改变下一次服务摇数**。白拿发生在落格结算的那一帧，与骰子、抽卡同帧
/// 并发 —— 并进任何一条已有的流，董事长逛一趟百货就会挪动别人的随机数。
const FREEBIE_TAG: u64 = 0xF3EE_0000_0000_000F;
/// P11：这一趟播哪一条新闻，以及「点名一位」那一档点到谁。
///
/// 单独一条的理由与前面每一条同形，而新闻这一条多一层**必须**：抽取算法是
/// 「全池随机 + 同场不连重」（PLAN §七 那条拍板），而「上一条是哪条」是一个**跨回合的状态**
/// （`GameSession::last_news`）。若让它读牌堆那条流，「这一局播过几条新闻」就会挪动
/// 下一张机会牌是谁 —— 那正是每条子流要防的事，而这里它同时还是回放的对齐点。
const NEWS_TAG: u64 = 0xBE5F_0000_0000_0010;
/// P12：恶人的随机行为（强盗擦肩抢走哪一张卡）。
///
/// 单独一条的理由与前面每一条同形，而这里多一层**行为稀疏性**：恶人的取数发生在
/// 别人走路的步进帧上（擦肩那一格），与骰子、抽卡、神明挪格同帧并发 —— 并进任何一条
/// 已有的流，「这一回合擦过谁的肩」都会挪动别人的随机数。而抢走哪张卡是玩家会回头对的事
/// （「我那张停在我手里好好的卡怎么没了」），同一颗种子必须两次抢走同一张。
const VILLAIN_TAG: u64 = 0x7A1A_0000_0000_0011;
/// P13：魔法屋女巫的抽签（「得一张卡」发下的是哪一张）。
///
/// 单独一条的理由与百货、神明那两条同形：**施法发生在别人回合的落格帧上**，
/// 并进任何一条已有的流，「这一趟踩没踩魔法屋」都会挪动别人的随机数。
/// 而发下哪张卡是玩家会回头对的事（「上次同种子也该是这张」），同一颗种子必须
/// 两次发下同一张。
const MAGIC_TAG: u64 = 0x9A61_C000_0000_0012;
/// 开局落位（10-26 起：4 代没有起点格，玩家开局在环上各抽一格）。
///
/// 单独一条的理由与前面每一条同形，而这里多一层**装配期**：取数发生在 `begin_game_system`
/// 里，比第一次掷骰还早，而且排在牌堆装配与宝箱刷出**之间** —— 并进任何一条已有的流，
/// 「这一局有几个人」都会挪动第一张机会牌、第一手骰点与宝箱位置。那三样都是截图核对与
/// 回放的对齐点，而「谁坐在哪」是这一串里最不该牵动别人的一个读数。
const START_TAG: u64 = 0x57A3_0000_0000_0013;
/// P14：AI 的读数（这一次想了多久，以及 P14b 起「这一次算没算错」）。
///
/// 单独一条的理由与前面每一条同形，而这里多一层**别的流都没有的性质**：这一条抽的是
/// **墙钟上的长度**，它随帧率与 `NM_AUTOGAP` 变。若让它与任何影响决策的读数同流，
/// 「这台机器想了 0.3 秒还是 1.4 秒」就会挪动下一张机会牌是谁 —— 那等于把整局的可复现性
/// 押在帧率上，而同一颗种子的两次跑法本来必须走出同一条轨迹。
const AI_TAG: u64 = 0xA1A1_0000_0000_0014;

/// 对局种子。测试与回放里显式给定；真实客户端从时钟取。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSeed(pub u64);

impl Default for GameSeed {
    /// 固定值：不显式给种子时（测试、CI）对局必须可复现。
    fn default() -> Self {
        Self(0x5EED_0000_0000_0001)
    }
}

/// 全局唯一的随机性来源。任何需要随机的 system 都从这里取，不要自己 new 一个 ——
/// 否则「同种子同对局」立刻失效。
#[derive(Resource)]
pub struct Rng {
    pub seed: u64,
    dice: ChaCha8Rng,
    cards: ChaCha8Rng,
    market: ChaCha8Rng,
    shop: ChaCha8Rng,
    epoch: ChaCha8Rng,
    gift: ChaCha8Rng,
    envelope: ChaCha8Rng,
    spawn: ChaCha8Rng,
    loot: ChaCha8Rng,
    god: ChaCha8Rng,
    minigame: ChaCha8Rng,
    lottery: ChaCha8Rng,
    auction: ChaCha8Rng,
    service: ChaCha8Rng,
    freebie: ChaCha8Rng,
    news: ChaCha8Rng,
    villain: ChaCha8Rng,
    magic: ChaCha8Rng,
    start: ChaCha8Rng,
    ai: ChaCha8Rng,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            // 骰子流直接吃种子本身：`Rng::new(s).roll_die()` 必须等于对局里的第一手点数
            dice: ChaCha8Rng::seed_from_u64(seed),
            cards: ChaCha8Rng::seed_from_u64(seed ^ CARDS_TAG),
            market: ChaCha8Rng::seed_from_u64(seed ^ MARKET_TAG),
            shop: ChaCha8Rng::seed_from_u64(seed ^ SHOP_TAG),
            epoch: ChaCha8Rng::seed_from_u64(seed ^ EPOCH_TAG),
            gift: ChaCha8Rng::seed_from_u64(seed ^ GIFT_TAG),
            envelope: ChaCha8Rng::seed_from_u64(seed ^ ENVELOPE_TAG),
            spawn: ChaCha8Rng::seed_from_u64(seed ^ SPAWN_TAG),
            loot: ChaCha8Rng::seed_from_u64(seed ^ LOOT_TAG),
            god: ChaCha8Rng::seed_from_u64(seed ^ GOD_TAG),
            minigame: ChaCha8Rng::seed_from_u64(seed ^ MINIGAME_TAG),
            lottery: ChaCha8Rng::seed_from_u64(seed ^ LOTTERY_TAG),
            auction: ChaCha8Rng::seed_from_u64(seed ^ AUCTION_TAG),
            service: ChaCha8Rng::seed_from_u64(seed ^ SERVICE_TAG),
            freebie: ChaCha8Rng::seed_from_u64(seed ^ FREEBIE_TAG),
            news: ChaCha8Rng::seed_from_u64(seed ^ NEWS_TAG),
            villain: ChaCha8Rng::seed_from_u64(seed ^ VILLAIN_TAG),
            magic: ChaCha8Rng::seed_from_u64(seed ^ MAGIC_TAG),
            start: ChaCha8Rng::seed_from_u64(seed ^ START_TAG),
            ai: ChaCha8Rng::seed_from_u64(seed ^ AI_TAG),
        }
    }

    /// 新闻那一条流（P11）：这一趟播哪一条、点名那一位点到谁。
    pub fn pick_news(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.news.random_range(span)
    }

    /// 恶人那一条流（P12）：强盗擦肩抢走受害者手牌里的第几张。
    pub fn pick_villain(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.villain.random_range(span)
    }

    /// 魔法屋那一条流（P13）：「得一张卡」那道法术发下货架上的第几张。
    pub fn pick_magic(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.magic.random_range(span)
    }

    /// 服务收费那一条流（P10）：购物中心掷几点、旅馆住几天、航空转盘停几段。
    /// 范围由载体自己给（`1..=sides`）—— 原型库只收数，不认识骰子。
    pub fn pick_service(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.service.random_range(span)
    }

    /// 董事长白拿那一条流（P10d②）：这一趟货架上拿走第几格。
    pub fn pick_freebie(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.freebie.random_range(span)
    }

    /// 乐透那一条流（P8）：这个月开出几号。
    pub fn pick_lottery(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.lottery.random_range(span)
    }

    /// 破产清算那一条流（P9）：从他名下的地里抽哪几块、按什么次序拍。
    pub fn pick_auction(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.auction.random_range(span)
    }

    /// 游乐场那一条流（P7）：这一趟玩哪一款，以及那一局的布局种子。
    pub fn pick_minigame(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.minigame.random_range(span)
    }

    /// 一整局小游戏的布局种子（64 位）。
    ///
    /// 为什么发一个**种子**而不是把布局抽好：布局长什么样是玩法本体的事（4×4 的冰洞、
    /// 气球从哪儿升起来），规则层刻意不知道（PLAN §五「core 不实现玩法本体」）。
    /// 发种子既保住了「同一 `NM_SEED` 走到同一格摆出同一局」，又没让 core 学会数格子。
    pub fn arcade_seed(&mut self) -> u64 {
        self.minigame.random::<u64>()
    }

    /// 神明那一条流（P6）：刷哪位神、落在哪一格、往哪边走、神给的那位数。
    pub fn pick_god(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.god.random_range(span)
    }

    /// 宝箱刷在哪一格（宝箱位置子流）。
    pub fn pick_spawn(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.spawn.random_range(span)
    }

    /// 开局把人放到环上哪一格（落位子流）。调用方给 `0..=环长-1`。
    ///
    /// **每人独立抽一次**，所以两个人落在同一格是合法结果（同格的站位偏移是表现层的事）。
    pub fn pick_start(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.start.random_range(span)
    }

    /// 宝箱开出哪一种道具（掉落子流）。
    pub fn pick_loot(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.loot.random_range(span)
    }

    /// 开局的月份（历法子流）。**专用子流是硬约束**：若从牌堆或骰子流里抽，
    /// 同一个 `NM_SEED` 的日历会随内容增删漂移，截图核对全部失效。
    pub fn pick_epoch_month(&mut self) -> u32 {
        self.epoch.random_range(1..=12)
    }

    /// 圣诞礼物的卡池下标（礼物子流）：`0..n`。空池由调用方先挡（那里才说得出为什么没发）。
    pub fn pick_gift(&mut self, cards: u32) -> u32 {
        self.gift.random_range(0..cards.max(1))
    }

    /// 红包的一位数字（红包子流）：0..=9。千位也可以是 0，0000 是空红包。
    pub fn pick_envelope_digit(&mut self) -> u32 {
        self.envelope.random_range(0..=9)
    }

    /// 掷一颗骰，返回 1..=6（骰子流）。
    pub fn roll_die(&mut self) -> u32 {
        self.dice.random_range(1..=DIE_FACES)
    }

    /// 闭区间取随机整数（牌堆流）：洗牌用的 `0..=i`。
    ///
    /// ⚠️ P14 的 AI **不走这条**（它有 [`Self::pick_ai`]）。这条原话写的是「将来 AI 的随机化
    /// 都走这条」，而那句话在 P14a 被推翻了：把「这台电脑想了多久」接到牌堆流上，等于让帧率
    /// 去挪动下一张机会牌是谁。
    pub fn pick(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.cards.random_range(span)
    }

    /// AI 那一条流（P14）：这一次想了多久（`0..=1`，调用侧按区间放大）。
    ///
    /// 只有 `AiCadence` 的区间**真的是一个区间**时才抽它：core 的默认与无头测试都是 0，
    /// 那时一次数都不消耗 —— 否则每回合白抽一次，等于给这条流加了一个随回合增长的取数节奏
    /// （与神明那条流单独拆出去的理由同形）。
    pub fn pick_ai(&mut self) -> f32 {
        self.ai.random_range(0.0..=1.0)
    }

    /// 闭区间取随机整数（百货流）：每次进店的货架抽签（`shop::restock`）。
    /// 与 [`Self::pick`] 分成两条是刻意的：买几次货不该改变下一张机会牌是谁。
    pub fn pick_shop(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.shop.random_range(span)
    }

    /// 闭区间取随机浮点（行情流）：股价每 tick 的噪声（`stock_market.gd:73`）。
    pub fn range_f32(&mut self, span: RangeInclusive<f32>) -> f32 {
        self.market.random_range(span)
    }
}

/// 四个数字拼成一笔钱：千位是 `leading`，后三位各 0–9。
///
/// 为什么要一个公共函数而不是两处各写一遍 `acc * 10 + 数字`：PLAN §三 神明行点名
/// 「大财神那笔四位数现金与春节红包共用同一个生成函数，别各写一套」。这两处的**位数形状**
/// 是同一条规则（四位、逐位取数），只有千位范围不同 —— 红包允许 0000（空红包），
/// 财神从 1000 起。逐位取数而不是 `rng(1000..=8999)` 一次取完，是因为表现层的老虎机
/// 要逐位停下来给玩家看（`ui/calendar_fx.rs`），一次取出的整数拆不出那四位。
pub fn four_digits(leading: u32, rest: [u32; 3]) -> i64 {
    debug_assert!(
        leading <= 9 && rest.iter().all(|d| *d <= 9),
        "四位数的一位上出现了 {leading} 或 {rest:?}"
    );
    leading as i64 * 1_000 + rest[0] as i64 * 100 + rest[1] as i64 * 10 + rest[2] as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draws(seed: u64, n: usize) -> Vec<u32> {
        let mut rng = Rng::new(seed);
        (0..n).map(|_| rng.roll_die()).collect()
    }

    #[test]
    fn same_seed_replays_the_same_sequence() {
        assert_eq!(draws(42, 50), draws(42, 50));
        assert_ne!(draws(42, 50), draws(43, 50));
    }

    #[test]
    fn die_stays_in_range() {
        for seed in 0..20 {
            for v in draws(seed, 200) {
                assert!((1..=DIE_FACES).contains(&v), "掷出 {v}（种子 {seed}）");
            }
        }
    }

    #[test]
    fn every_face_is_reachable() {
        let rolled: Vec<u32> = draws(7, 2000);
        for face in 1..=DIE_FACES {
            assert!(rolled.contains(&face), "2000 次里没出现过 {face}，分布可疑");
        }
    }

    /// 分流的意义：别的域取多少随机数，都不该挪动骰子流。
    /// 加了卡牌/行情/百货之后，「对局第一手 == `Rng::new(seed).roll_die()`」这类断言仍然成立。
    #[test]
    fn other_streams_do_not_shift_the_dice() {
        let mut busy = Rng::new(99);
        for _ in 0..50 {
            busy.pick(0..=6);
            busy.pick_shop(0..=6);
            busy.range_f32(-0.5..=0.5);
        }
        let mut fresh = Rng::new(99);
        assert_eq!(
            busy.roll_die(),
            fresh.roll_die(),
            "牌堆/行情取数污染了骰子流"
        );
    }

    /// 新闻那条流（P11）：这一局播过几条新闻、点到过谁，都不该改变下一张机会牌是谁、
    /// 下一手骰点是几。它与 `pick_shop` 那两条同族，而这里多一层要防的东西 ——
    /// 「同场不连重」让新闻的**取数次数**随池子大小变（见 `news::roll_span`），
    /// 所以这条流一旦被并进牌堆，池子从 2 条改成 8 条就会重排整局的抽卡顺序。
    #[test]
    fn a_news_broadcast_shifts_nothing_else() {
        let after_news = || {
            let mut r = Rng::new(7);
            for _ in 0..20 {
                r.pick_news(0..=7);
            }
            (0..10).map(|_| r.pick(0..=99)).collect::<Vec<_>>()
        };
        let untouched = || {
            let mut r = Rng::new(7);
            (0..10).map(|_| r.pick(0..=99)).collect::<Vec<_>>()
        };
        assert_eq!(after_news(), untouched(), "新闻流污染了牌堆流");

        let mut busy = Rng::new(99);
        for _ in 0..30 {
            busy.pick_news(0..=3);
        }
        let mut fresh = Rng::new(99);
        assert_eq!(busy.roll_die(), fresh.roll_die(), "新闻流污染了骰子流");
        // 而它自己是可复现的：同种子同序列（回放与截图核对的地基）
        let seq = |seed| {
            let mut r = Rng::new(seed);
            (0..12).map(|_| r.pick_news(0..=3)).collect::<Vec<_>>()
        };
        assert_eq!(seq(4), seq(4));
        assert_ne!(seq(4), seq(5), "新闻流与另一个域取了同一条密钥流");
        assert_ne!(
            seq(4),
            {
                let mut r = Rng::new(4);
                (0..12).map(|_| r.pick(0..=3)).collect::<Vec<_>>()
            },
            "新闻流与牌堆流取了同一条密钥流"
        );
    }

    #[test]
    fn each_stream_is_reproducible() {
        let cards = |seed: u64| {
            let mut r = Rng::new(seed);
            (0..30).map(|_| r.pick(0..=99)).collect::<Vec<_>>()
        };
        let drifts = |seed: u64| {
            let mut r = Rng::new(seed);
            (0..30)
                .map(|_| (r.range_f32(-0.5..=0.5) * 1000.0).round() as i32)
                .collect::<Vec<_>>()
        };
        assert_eq!(cards(5), cards(5), "同种子必须复现同一条抽牌序列");
        assert_ne!(cards(5), cards(6));
        assert_eq!(drifts(5), drifts(5), "同种子必须复现同一条行情序列");
        assert_ne!(drifts(5), drifts(6));
    }

    /// 域标签失效（三条流退化成同一条）时这条会响：拿牌堆流掷骰区间走 30 次，
    /// 结果不该等于骰子流的同种子序列。
    #[test]
    fn card_stream_is_not_the_dice_stream() {
        let mut r = Rng::new(11);
        let via_cards: Vec<u32> = (0..30).map(|_| r.pick(1..=DIE_FACES)).collect();
        assert_ne!(via_cards, draws(11, 30), "牌堆流与骰子流取了同一个域");
    }

    /// P3d② 分第四条流的**全部理由**：进几趟店、换几次货架，都不该改变下一张机会牌是谁。
    /// 这条一旦红了，说明有人把货架抽签并回了牌堆流 —— 症状是「买过货的那局，抽卡顺序变了」。
    #[test]
    fn a_shop_restock_does_not_shift_the_deck() {
        let after_shopping = || {
            let mut r = Rng::new(3);
            for _ in 0..20 {
                r.pick_shop(0..=4);
            }
            (0..10).map(|_| r.pick(0..=99)).collect::<Vec<_>>()
        };
        let untouched = || {
            let mut r = Rng::new(3);
            (0..10).map(|_| r.pick(0..=99)).collect::<Vec<_>>()
        };
        assert_eq!(after_shopping(), untouched(), "百货流污染了牌堆流");
    }

    #[test]
    fn shop_stream_is_not_the_card_stream() {
        let draw = |seed: u64, shop: bool| {
            let mut r = Rng::new(seed);
            (0..30)
                .map(|_| {
                    if shop {
                        r.pick_shop(0..=99)
                    } else {
                        r.pick(0..=99)
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_ne!(
            draw(11, true),
            draw(11, false),
            "百货流与牌堆流取了同一个域"
        );
        assert_eq!(draw(5, true), draw(5, true), "同种子必须复现同一条货架序列");
    }

    #[test]
    fn drift_stays_inside_the_amplitude() {
        let mut rng = Rng::new(3);
        for _ in 0..500 {
            let v = rng.range_f32(-0.12..=0.12);
            assert!((-0.12..=0.12).contains(&v), "噪声 {v} 跑出区间");
        }
    }

    /// P2b 的三条新流：历法、礼物、红包。各抽各的，谁也不许挪动牌序与骰点。
    #[test]
    fn calendar_and_holiday_streams_do_not_shift_the_rest() {
        let busy = || {
            let mut r = Rng::new(21);
            for _ in 0..20 {
                r.pick_epoch_month();
                r.pick_gift(6);
                r.pick_envelope_digit();
            }
            (
                (0..5).map(|_| r.roll_die()).collect::<Vec<_>>(),
                (0..5).map(|_| r.pick(0..=9)).collect::<Vec<_>>(),
            )
        };
        let fresh = || {
            let mut r = Rng::new(21);
            (
                (0..5).map(|_| r.roll_die()).collect::<Vec<_>>(),
                (0..5).map(|_| r.pick(0..=9)).collect::<Vec<_>>(),
            )
        };
        assert_eq!(busy(), fresh(), "历法/礼物/红包污染了骰子或牌堆流");
    }

    #[test]
    fn epoch_month_stays_inside_the_year() {
        let mut r = Rng::new(4);
        let months: Vec<u32> = (0..200).map(|_| r.pick_epoch_month()).collect();
        assert!(months.iter().all(|m| (1..=12).contains(m)));
        for month in 1..=12 {
            assert!(months.contains(&month), "200 次没抽到 {month} 月，分布可疑");
        }
    }

    #[test]
    fn envelope_digits_stay_in_range_and_can_be_zero() {
        let mut r = Rng::new(8);
        let digits: Vec<u32> = (0..500).map(|_| r.pick_envelope_digit()).collect();
        assert!(digits.iter().all(|d| *d <= 9));
        assert!(digits.contains(&0), "千位允许 0，0000 是合法红包");
    }

    /// P6 的第十条流。神每回合都要挪一格，是这条流上唯一一个「取数次数随回合数增长」的来源
    /// —— 所以它必须不能挪动骰点、牌序与红包。
    #[test]
    fn a_god_wandering_shifts_nothing() {
        let draws = |seed: u64, busy: bool| {
            let mut r = Rng::new(seed);
            if busy {
                for _ in 0..60 {
                    r.pick_god(0..=1);
                    r.pick_god(1..=12);
                }
            }
            (
                (0..6).map(|_| r.roll_die()).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick(0..=9)).collect::<Vec<_>>(),
                (0..4)
                    .map(|_| (0..4).map(|_| r.pick_envelope_digit()).collect::<Vec<_>>())
                    .collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            draws(77, true),
            draws(77, false),
            "神明流污染了骰子/牌堆/红包流"
        );
    }

    #[test]
    fn god_draws_are_reproducible_and_in_range() {
        let series = |seed: u64| {
            let mut r = Rng::new(seed);
            (0..40).map(|_| r.pick_god(0..=10)).collect::<Vec<_>>()
        };
        assert_eq!(series(5), series(5), "同种子必须复现同一条刷神序列");
        assert_ne!(series(5), series(6));
        assert!(series(5).iter().all(|v| (0..=10).contains(v)));
    }

    /// P7 的第十一条流。游乐场是这条流上唯一一个「一次取数就取走 64 位」的来源
    /// （布局种子），所以它尤其不能挪动骰点、牌序、货架与神明。
    #[test]
    fn an_arcade_visit_shifts_nothing() {
        let draws = |seed: u64, busy: bool| {
            let mut r = Rng::new(seed);
            if busy {
                for _ in 0..40 {
                    r.pick_minigame(0..=2);
                    r.arcade_seed();
                }
            }
            (
                (0..6).map(|_| r.roll_die()).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick(0..=9)).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick_shop(0..=9)).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick_god(1..=12)).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            draws(41, true),
            draws(41, false),
            "小游戏流污染了骰子/牌堆/百货/神明流"
        );
    }

    /// 布局种子要真的散开：三局全是同一个数的话，「随机一款 + 随机布局」那一半是假的
    /// （而症状只是「每次挖宝的宝都在同几格」，不报错、也不红）。
    #[test]
    fn arcade_seeds_spread_out() {
        let mut r = Rng::new(23);
        let seeds: Vec<u64> = (0..32).map(|_| r.arcade_seed()).collect();
        let mut uniq = seeds.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), seeds.len(), "布局种子撞车了：{seeds:?}");
    }

    /// 财神与红包共用的那一句（PLAN §三 神明行点名的「别各写一套」）。
    /// 两个边界要钉住：0000 是合法空红包，而千位为 1 的那一位是 1000 不是 1。
    #[test]
    fn four_digits_folds_place_values_not_a_string() {
        assert_eq!(four_digits(0, [0, 0, 0]), 0, "空红包");
        assert_eq!(four_digits(1, [0, 0, 0]), 1_000, "千位是千位，不是 1");
        assert_eq!(four_digits(9, [9, 9, 9]), 9_999, "四位数的上限");
        assert_eq!(four_digits(3, [0, 7, 5]), 3_075, "中间那位是 0 也要占位");
    }

    /// P10 的第十四条流。服务收费的摇数与骰子、抽卡、神明同帧发生，是这条流存在的
    /// 全部理由：踩没踩购物中心不该挪动别人的随机数，反过来也一样。
    #[test]
    fn a_service_roll_shifts_nothing() {
        let draws = |seed: u64, busy: bool| {
            let mut r = Rng::new(seed);
            if busy {
                for _ in 0..40 {
                    r.pick_service(1..=6);
                    r.pick_service(1..=4);
                }
            }
            (
                (0..6).map(|_| r.roll_die()).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick(0..=9)).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick_shop(0..=9)).collect::<Vec<_>>(),
                (0..6).map(|_| r.pick_god(1..=12)).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            draws(33, true),
            draws(33, false),
            "服务流污染了骰子/牌堆/百货/神明流"
        );
    }

    #[test]
    fn service_draws_are_reproducible_and_in_range() {
        let series = |seed: u64| {
            let mut r = Rng::new(seed);
            (0..40).map(|_| r.pick_service(1..=4)).collect::<Vec<_>>()
        };
        assert_eq!(series(5), series(5), "同种子必须复现同一条摇数序列");
        assert_ne!(series(5), series(6));
        assert!(series(5).iter().all(|v| (1..=4).contains(v)));
    }

    /// AI 那条流（P14）：这台电脑想了多久、算错了几次，都不该挪动骰点、牌序、百货与神明。
    /// 这里要防的比别条更硬一层 —— 这一条的**取数次数本来就由墙钟决定**（`NM_AUTOGAP` 拧一下，
    /// 一局里 AI 动手的次数就变了），所以它一旦被并进任何一条别的流，帧率与延迟旋钮会直接
    /// 重排整局的骰点与抽卡，而症状是「同一颗种子在我的机器上与你的机器上走得不一样」。
    #[test]
    fn the_ai_stream_shifts_nothing_else() {
        let draws = |busy: bool| {
            let mut r = Rng::new(31);
            if busy {
                for _ in 0..80 {
                    r.pick_ai();
                }
            }
            (
                (0..8).map(|_| r.roll_die()).collect::<Vec<_>>(),
                (0..8).map(|_| r.pick(0..=9)).collect::<Vec<_>>(),
                (0..8).map(|_| r.pick_shop(0..=9)).collect::<Vec<_>>(),
                (0..8).map(|_| r.pick_god(1..=12)).collect::<Vec<_>>(),
            )
        };
        assert_eq!(
            draws(true),
            draws(false),
            "AI 流污染了骰子/牌堆/百货/神明流"
        );
        let series = |seed: u64| {
            let mut r = Rng::new(seed);
            (0..12).map(|_| r.pick_ai()).collect::<Vec<f32>>()
        };
        assert_eq!(series(5), series(5), "同种子必须复现同一条 AI 序列");
        assert!(series(5).iter().all(|v| (0.0..=1.0).contains(v)));
        assert_ne!(series(5), series(6), "AI 流与另一个域取了同一条密钥流");
        assert_ne!(
            series(5),
            {
                let mut r = Rng::new(5);
                (0..12).map(|_| r.range_f32(0.0..=1.0)).collect::<Vec<_>>()
            },
            "AI 流与行情流取了同一条密钥流"
        );
    }
}
