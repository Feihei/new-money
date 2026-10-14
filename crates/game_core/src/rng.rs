//! 规则层唯一的随机性来源。
//!
//! Godot 端骰子用全局 `randi_range`（`dice.gd:19`）、洗牌用 `_rng.randomize()`
//! （`card_deck.gd:53`），两者都不可复现，而 `roll_seeded()` 只是个 `assert` 后原样返回的
//! 假种子函数（缺陷 #6）。这里整局共用一个 `ChaCha8Rng`，种子是 resource，
//! 于是「同种子 → 同对局」变成一条可以断言的性质 —— 回放与失败复现都从这里长出来。

use std::ops::RangeInclusive;

use bevy::prelude::*;
use rand::{Rng as _, SeedableRng as _};
use rand_chacha::ChaCha8Rng;

/// 单颗六面骰（`dice.gd:15`）。大富翁4 规则：无双骰、无双数额外回合。
pub const DIE_FACES: u32 = 6;

/// 对局种子。测试与回放里显式给定；真实客户端从时钟取。
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct GameSeed(pub u64);

impl Default for GameSeed {
    /// 固定值：不显式给种子时（测试、CI）对局必须可复现。
    fn default() -> Self {
        Self(0x5EED_0000_0000_0001)
    }
}

/// 全局唯一的随机流。任何需要随机的 system 都从这里取，不要自己 new 一个 ——
/// 否则「同种子同对局」立刻失效。
#[derive(Resource)]
pub struct Rng {
    pub seed: u64,
    inner: ChaCha8Rng,
}

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self {
            seed,
            inner: ChaCha8Rng::seed_from_u64(seed),
        }
    }

    /// 掷一颗骰，返回 1..=6。
    pub fn roll_die(&mut self) -> u32 {
        self.range(1..=DIE_FACES)
    }

    /// 闭区间取随机数（洗牌、股价波动共用同一条流）。
    pub fn range(&mut self, span: RangeInclusive<u32>) -> u32 {
        self.inner.random_range(span)
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
}
