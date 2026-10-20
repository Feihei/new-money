//! 规则层唯一的随机性来源。
//!
//! Godot 端骰子用全局 `randi_range`（`dice.gd:19`）、洗牌用 `_rng.randomize()`
//! （`card_deck.gd:53`），两者都不可复现，而 `roll_seeded()` 只是个 `assert` 后原样返回的
//! 假种子函数（缺陷 #6）。这里整局共用一个种子，于是「同种子 → 同对局」变成一条
//! 可以断言的性质 —— 回放与失败复现都从这里长出来。
//!
//! **同一个种子分成九条互不相干的子流**（骰子 / 牌堆 / 行情 / 百货 / 历法 / 礼物 / 红包 /
//! 宝箱位置 / 宝箱掉落）：抽一张卡不该让下一手骰点变样。若共用一条流，经济系统每多一次
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
        }
    }

    /// 宝箱刷在哪一格（宝箱位置子流）。
    pub fn pick_spawn(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.spawn.random_range(span)
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

    /// 闭区间取随机整数（牌堆流）：洗牌用的 `0..=i`、将来 AI 的随机化都走这条。
    pub fn pick(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.cards.random_range(span)
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
}
