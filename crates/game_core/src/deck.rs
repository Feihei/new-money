//! 牌堆：机会 / 命运两副牌（`card_deck.gd`）。
//!
//! 卡定义是静态资产（`Assets<CardSet>`），牌堆实体只持 **下标**（`CardId`），
//! 于是「洗牌」是排下标，不复制卡面数据，也不会出现两份卡面真相。
//!
//! 与 Godot 的实质差别只有一处：洗牌用全局 `_rng.randomize()`（`card_deck.gd:53`）
//! 不可复现（缺陷 #6），这里改用 `Rng` 的**牌堆子流** —— 同种子复现同一副顺序，
//! 而抽卡不会挪动骰子流（见 `rng.rs` 的分流理由）。

use bevy::prelude::*;

use crate::board::{BranchKind, GridPos, TileIndex};
use crate::card::{CardDef, CardEffect, CardKind, CardSet};
use crate::charge::{Charge, ChargeKind};
use crate::data::DataHandles;
use crate::landing::{
    Landed, Landing, LandingDomain, Pending, PendingDecision, jail_player, teleport,
};
use crate::msg::{CardDrawn, CardNotice, EffectOutcome, PlayerJailed, PlayerTeleported};
use crate::player::PlayerId;
use crate::player::TokenPosition;
use crate::rng::Rng;
use crate::settle::Books;

/// 牌堆里的一张牌 = 它在对应 `CardSet` 里的下标。
///
/// 用下标而不是 `String` id：卡池是加载后不变的静态数组，下标即身份，
/// 存盘时随 `GameSeed` 一起足够重建（§九）；而 `CardNotice` 给 UI 的是 id 字符串。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct CardId(pub u32);

/// 一副可循环抽的牌。存在即「这一堆牌」，机会 / 命运各一个实体。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub struct Deck {
    pub kind: CardKind,
    /// 还能抽的牌，`last` 是堆顶（与 `card_deck.gd:63` 的 `pop_back()` 一致）。
    pub draw_pile: Vec<CardId>,
    /// 已抽出的牌。抽穿后整副重洗（`card_deck.gd:59-62` 的自动洗牌）。
    pub discard: Vec<CardId>,
}

impl Deck {
    /// 按 `rng` 洗出一副完整牌堆（开局装配用）。
    pub fn shuffled(kind: CardKind, cards: &CardSet, rng: &mut Rng) -> Self {
        let mut deck = Self {
            kind,
            draw_pile: (0..cards.0.len() as u32).map(CardId).collect(),
            discard: Vec::new(),
        };
        deck.shuffle(rng);
        deck
    }

    pub fn remaining(&self) -> usize {
        self.draw_pile.len()
    }

    pub fn total(&self) -> usize {
        self.draw_pile.len() + self.discard.len()
    }

    /// 整副重洗：恢复成完整的 n 张再打乱（`card_deck.gd:48-54` 的 `shuffle()` 就是这个，
    /// 不是「弃牌堆倒回去」—— 弃牌堆只是记账，供存档与回放看「这局抽过哪些牌」。
    pub fn shuffle(&mut self, rng: &mut Rng) {
        let n = self.total();
        self.draw_pile = (0..n as u32).map(CardId).collect();
        self.discard.clear();
        fisher_yates(&mut self.draw_pile, rng);
    }

    /// 抽一张。牌堆空了先自动重洗（Godot 的 `test_deck_auto_reshuffle_when_empty`）。
    /// 空牌堆（一副都没有）返回 `None` —— 数据错误由 `CardSet::validate` 拦在加载期。
    pub fn draw(&mut self, rng: &mut Rng) -> Option<CardId> {
        if self.draw_pile.is_empty() {
            if self.total() == 0 {
                return None;
            }
            self.shuffle(rng);
        }
        let card = self.draw_pile.pop()?;
        self.discard.push(card);
        Some(card)
    }
}

/// 种子化 Fisher-Yates：从堆尾往前，每个位置与本机之前的某处交换。
fn fisher_yates(pile: &mut [CardId], rng: &mut Rng) {
    for i in (1..pile.len()).rev() {
        let j = rng.pick(0..=i as u32) as usize;
        pile.swap(i, j);
    }
}

/// 牌堆 → 卡定义的查询（牌堆只存下标，卡面在资产里）。
pub fn card_of(set: &CardSet, id: CardId) -> Option<&CardDef> {
    set.0.get(id.0 as usize)
}

// ===== 落格那一段：抽卡并当场结算效果 =====

/// 踩到机会 / 命运格：抽一张、立刻结算效果、把卡面挂成「等玩家读完」。
///
/// 对应 Godot `turn_manager.gd:389-407` + `card_effects.gd`。三处刻意的差别：
/// - **效果是枚举不是字符串**（`CardEffect`），新增 action 漏写分支直接编译失败（缺陷 #8）。
/// - **扣不到钱就真破产**（缺陷 #3）：`CouldNotPay` 这条凭据与 `PlayerBankrupt`
///   在同一帧成对出现，UI 文案与玩家状态不可能各说各话。
/// - **`move_to` 不连锁结算新格子**：Godot 端踩到「回到起点」也不会领过路费奖金，
///   这里照做 —— 连锁会引入「卡牌把玩家送到卡牌格」这类环，属于设计而非移植。
#[allow(clippy::too_many_arguments)]
pub fn settle_card_landing(
    mut commands: Commands,
    mut books: Books,
    mut decks: Query<'_, '_, &'static mut Deck>,
    mut rng: ResMut<'_, Rng>,
    sets: Res<'_, Assets<CardSet>>,
    data: Res<'_, DataHandles>,
    mut positions: Query<'_, '_, &'static mut TokenPosition, With<PlayerId>>,
    landing: Res<'_, Landing>,
    mut pending: ResMut<'_, PendingDecision>,
    mut drawn: MessageWriter<'_, CardDrawn>,
    mut jailed: MessageWriter<'_, PlayerJailed>,
    mut teleported: MessageWriter<'_, PlayerTeleported>,
) {
    let Some(Landed {
        player,
        domain: LandingDomain::Card(expect),
        ..
    }) = landing.0
    else {
        return;
    };

    // 牌堆与卡定义按种类配对：机会格抽机会牌，命运格抽命运牌。
    // 这条配对写在 `domain_of` 的分派表里，所以这里不必再 `match` 一遍格类型。
    let handle = match expect {
        CardKind::Chance => &data.chance,
        CardKind::Fate => &data.fate,
    };
    let Some(set) = sets.get(handle) else {
        return;
    };
    let Some(mut deck) = decks.iter_mut().find(|d| d.kind == expect) else {
        return;
    };
    let Some(drawn_id) = deck.draw(&mut rng) else {
        debug!("{expect:?} 牌堆是空的，本回合不抽卡");
        return;
    };
    let Some(def) = card_of(set, drawn_id) else {
        return;
    };

    let mut outcomes = Vec::new();
    for effect in &def.effects {
        match effect {
            CardEffect::AddCash { amount } => {
                // 走管线：物价 2.5 时这张「横财」卡真的多给钱，而衰神的「得钱作废」
                // 是一条 Cancel 就能勾掉的账（所以 Bonus 的 `cancellable` 是开着的）
                let charge = Charge::new(ChargeKind::Bonus, None, Some(player), *amount);
                let got = books
                    .settle(&mut commands, charge)
                    .map(|done| done.amount)
                    .unwrap_or(0);
                outcomes.push(EffectOutcome::Gained(got));
            }
            CardEffect::DeductCash { amount } => {
                let charge = Charge::new(ChargeKind::Penalty, Some(player), None, *amount);
                match books.settle(&mut commands, charge) {
                    Ok(done) => outcomes.push(EffectOutcome::Lost(done.amount)),
                    Err(short) => {
                        outcomes.push(EffectOutcome::CouldNotPay {
                            needed: short.needed,
                        });
                        // 人已破产，卡上剩下的效果不必再执行
                        break;
                    }
                }
            }
            CardEffect::GoToJail { turns } => {
                let board = &books.active.def;
                let Some(junction) = board.branch_junction(BranchKind::Jail) else {
                    debug!("这张棋盘没有监狱分支，入狱卡落空");
                    continue;
                };
                let junction = TileIndex(junction.id);
                let end = board.branch_end(BranchKind::Jail).map(GridPos::of);
                jail_player(&mut commands, &mut positions, player, junction, *turns);
                jailed.write(PlayerJailed {
                    player,
                    junction,
                    end,
                });
                outcomes.push(EffectOutcome::Jailed {
                    turns: *turns,
                    junction,
                });
            }
            CardEffect::MoveTo { tile_name } => {
                let target = books
                    .active
                    .def
                    .tiles
                    .iter()
                    .position(|t| t.name == *tile_name)
                    .map(|idx| TileIndex(idx as u32));
                match target {
                    Some(tile) => {
                        teleport(&mut positions, player, tile);
                        teleported.write(PlayerTeleported { player, tile });
                        outcomes.push(EffectOutcome::Teleported {
                            tile,
                            name: tile_name.clone(),
                        });
                    }
                    // 数据写了个棋盘上没有的格名：不猜、也不退回起点
                    // （`card_effects.gd:76-81` 已经修过一次「回退到 0=起点」的坑）
                    None => outcomes.push(EffectOutcome::UnknownTile {
                        name: tile_name.clone(),
                    }),
                }
            }
        }
    }

    drawn.write(CardDrawn {
        player,
        card: CardNotice {
            id: def.id.clone(),
            kind: def.kind,
            title: def.title.clone(),
            desc: def.desc.clone(),
            outcomes,
        },
    });
    pending.offer(player, Pending::Card);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::card::CardEffect;

    fn set(n: usize, kind: CardKind) -> CardSet {
        CardSet(
            (0..n)
                .map(|i| CardDef {
                    id: format!("{kind:?}_{i:03}"),
                    kind,
                    title: format!("卡{i}"),
                    desc: "测试".into(),
                    effects: vec![CardEffect::AddCash { amount: 100 }],
                })
                .collect(),
        )
    }

    #[test]
    fn shuffled_deck_holds_every_card_exactly_once() {
        let cards = set(10, CardKind::Chance);
        let mut rng = Rng::new(3);
        let deck = Deck::shuffled(CardKind::Chance, &cards, &mut rng);
        assert_eq!(deck.remaining(), 10, "总张数不能因洗牌增减");
        let mut sorted = deck.draw_pile.clone();
        sorted.sort();
        assert_eq!(
            sorted,
            (0..10).map(CardId).collect::<Vec<_>>(),
            "洗牌不该丢牌也不该重出一张"
        );
    }

    #[test]
    fn shuffle_is_seeded_and_seed_sensitive() {
        let cards = set(7, CardKind::Fate);
        let order =
            |seed: u64| Deck::shuffled(CardKind::Fate, &cards, &mut Rng::new(seed)).draw_pile;
        assert_eq!(order(21), order(21), "同种子必须复现同一副顺序");
        assert_ne!(order(21), order(22), "换种子却同一副顺序说明洗牌没读种子");
    }

    #[test]
    fn drawing_decrements_and_never_repeats_within_a_pass() {
        let cards = set(5, CardKind::Chance);
        let mut deck = Deck::shuffled(CardKind::Chance, &cards, &mut Rng::new(9));
        assert_eq!(deck.remaining(), 5);
        let mut drawn = Vec::new();
        for _ in 0..5 {
            drawn.push(deck.draw(&mut Rng::new(1)).expect("还有牌"));
        }
        assert_eq!(deck.remaining(), 0);
        assert_eq!(deck.discard.len(), 5, "抽出的牌进弃牌堆");
        let mut uniq = drawn.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), 5, "一轮之内不该重样：{drawn:?}");
    }

    #[test]
    fn empty_pile_reshuffles_and_keeps_going() {
        // Godot 的 test_deck_auto_reshuffle_when_empty：抽穿后下一张照样有
        let cards = set(3, CardKind::Chance);
        let mut deck = Deck::shuffled(CardKind::Chance, &cards, &mut Rng::new(4));
        let mut rng = Rng::new(4);
        let mut second_pass = Vec::new();
        for i in 0..6 {
            let c = deck
                .draw(&mut rng)
                .unwrap_or_else(|| panic!("第 {i} 次抽牌抽空了"));
            if i >= 3 {
                second_pass.push(c);
            }
        }
        assert_eq!(second_pass.len(), 3, "重洗后仍该发满 3 张");
        assert_eq!(deck.remaining(), 0);
    }

    #[test]
    fn empty_deck_reports_none_instead_of_panicking() {
        let cards = CardSet(Vec::new());
        let mut deck = Deck {
            kind: CardKind::Chance,
            draw_pile: Vec::new(),
            discard: Vec::new(),
        };
        assert_eq!(
            deck.draw(&mut Rng::new(0)),
            None,
            "一副空牌不该抽（也不该 panic）"
        );
        assert_eq!(deck.total(), 0);
        let _ = cards;
    }

    #[test]
    fn card_lookup_maps_index_to_definition() {
        let cards = set(4, CardKind::Chance);
        assert_eq!(card_of(&cards, CardId(2)).unwrap().title, "卡2");
        assert!(
            card_of(&cards, CardId(99)).is_none(),
            "越界下标应查不到而不是崩"
        );
    }
}
