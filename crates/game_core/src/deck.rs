//! 牌堆：机会 / 命运两副牌（`card_deck.gd`）。
//!
//! 卡定义是静态资产（`Assets<CardSet>`），牌堆实体只持 **下标**（`CardId`），
//! 于是「洗牌」是排下标，不复制卡面数据，也不会出现两份卡面真相。
//!
//! 与 Godot 的实质差别只有一处：洗牌用全局 `_rng.randomize()`（`card_deck.gd:53`）
//! 不可复现（缺陷 #6），这里改用 `Rng` 的**牌堆子流** —— 同种子复现同一副顺序，
//! 而抽卡不会挪动骰子流（见 `rng.rs` 的分流理由）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::afflict::{AfflictDomain, Affliction, Insulation};
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

/// 三副卡池的查询入口：「读哪三个文件」+「加载出来的资产」。
///
/// 这两样永远成对出现 —— 下标只有在**正确的那副池**里才指得对一张卡，所以打包成一个
/// 参数包而不是让每个调用方各拿两份（`CardSink` 同一条房规，§六 坑 8）。
/// `kind → handle` 这张分派表写在这里一次，抽卡与出牌两条入口共用它。
#[derive(SystemParam)]
pub struct Shelf<'w> {
    sets: Res<'w, Assets<CardSet>>,
    data: Res<'w, DataHandles>,
}

impl<'w> Shelf<'w> {
    /// 某副池的整副卡（开局校验、牌堆装配、货架用）。
    pub fn set(&self, kind: CardKind) -> Option<&CardSet> {
        let handle = self.handle(kind);
        self.sets.get(handle)
    }

    /// 某副池里的一张卡。
    pub fn get(&self, kind: CardKind, id: CardId) -> Option<&CardDef> {
        self.set(kind).and_then(|set| card_of(set, id))
    }

    /// 整副货架（`Item` 池）。百货公司列货位用的就是这一张。
    pub fn items(&self) -> &[CardDef] {
        self.set(CardKind::Item).map_or(&[], |set| &set.0)
    }

    fn handle(&self, kind: CardKind) -> &Handle<CardSet> {
        match kind {
            CardKind::Chance => &self.data.chance,
            CardKind::Fate => &self.data.fate,
            // 道具卡没有对应的格子（`domain_of` 不产出 `Card(Item)`），它的「池」是货架。
            // 这一支行文与上面两行同形，是为了让「哪天给道具卡开一格」这件事在
            // 分派表上留痕，而不是靠调用方记得去拿 `data.items`
            CardKind::Item => &self.data.items,
        }
    }
}

// ===== 效果的执行：抽到与打出共用同一条 =====

/// 结算一张卡要动的全部东西。打包成参数包是这仓库的房规（§六 坑 8），而这张包正好
/// 也是**两条入口共用的那一份**：踩到格子抽卡（[`settle_card_landing`]）与玩家主动
/// 打出一张卡（[`crate::hand::play_card_system`]）走的是同一个 [`apply_card`]。
///
/// 这不是顺手复用，是刻意的：一张卡的效果只有一套语义。两条入口各写一遍的话，
/// 「陷害卡把对手关进去」与「命运牌把自己关进去」就会在某一天开始各修一半。
#[derive(SystemParam)]
pub struct CardSink<'w, 's> {
    pub books: Books<'w, 's>,
    pub commands: Commands<'w, 's>,
    pub positions: Query<'w, 's, &'static mut TokenPosition, With<PlayerId>>,
    pub insulations: Query<'w, 's, &'static mut Insulation>,
    pub jailed: MessageWriter<'w, PlayerJailed>,
    pub teleported: MessageWriter<'w, PlayerTeleported>,
}

/// 把一张卡的全部效果打在 `victim` 头上，返回每条效果的凭据给 UI 拼文案。
///
/// `victim` 是参数而不是「抽卡的人」，是因为同一张卡面既可能落在自己身上（命运的
/// 「直接入狱」）也可能落在对手身上（陷害卡）—— 区别只在 `CardDef.target`，
/// 不在效果里。三处刻意的规则：
/// - **扣不到钱就真破产**（缺陷 #3）：`CouldNotPay` 与 `PlayerBankrupt` 同帧成对，
///   且卡上剩下的效果不再执行 —— 人已经离场。
/// - **`move_to` 不连锁结算新格子**：Godot 端踩到「回到起点」也不会领过路费奖金，
///   这里照做。连锁会引入「卡牌把玩家送到卡牌格」这类环，属于设计而非移植。
/// - **`go_to_jail` 先过护身**（P3a）：被抵消的那次不会留下半条入狱流水。
pub fn apply_card(
    sink: &mut CardSink,
    victim: Entity,
    def: &CardDef,
    aim: Option<crate::hand::CardAim>,
) -> Vec<EffectOutcome> {
    let mut outcomes = Vec::new();
    for effect in &def.effects {
        match effect {
            CardEffect::AddCash { amount } => {
                // 走管线：物价 2.5 时这张「横财」卡真的多给钱，而衰神的「得钱作废」
                // 是一条 Cancel 就能勾掉的账（所以 Bonus 的 `cancellable` 是开着的）
                let charge = Charge::new(ChargeKind::Bonus, None, Some(victim), *amount);
                let got = sink
                    .books
                    .settle(&mut sink.commands, charge)
                    .map(|done| done.amount)
                    .unwrap_or(0);
                outcomes.push(EffectOutcome::Gained(got));
            }
            CardEffect::DeductCash { amount } => {
                let charge = Charge::new(ChargeKind::Penalty, Some(victim), None, *amount);
                match sink.books.settle(&mut sink.commands, charge) {
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
                let affliction = Affliction::new(AfflictDomain::Detention, victim);
                if absorb(&mut sink.insulations, &mut sink.commands, &affliction) {
                    outcomes.push(EffectOutcome::Blocked {
                        domain: affliction.domain,
                    });
                    continue;
                }
                let board = &sink.books.active.def;
                let Some(junction) = board.branch_junction(BranchKind::Jail) else {
                    debug!("这张棋盘没有监狱分支，入狱卡落空");
                    continue;
                };
                let junction = TileIndex(junction.id);
                let end = board.branch_end(BranchKind::Jail).map(GridPos::of);
                jail_player(
                    &mut sink.commands,
                    &mut sink.positions,
                    victim,
                    junction,
                    *turns,
                );
                sink.jailed.write(PlayerJailed {
                    player: victim,
                    junction,
                    end,
                });
                outcomes.push(EffectOutcome::Jailed {
                    turns: *turns,
                    junction,
                });
            }
            CardEffect::MoveTo { tile_name } => {
                let target = sink
                    .books
                    .active
                    .def
                    .tiles
                    .iter()
                    .position(|t| t.name == *tile_name)
                    .map(|idx| TileIndex(idx as u32));
                match target {
                    Some(tile) => {
                        teleport(&mut sink.positions, victim, tile);
                        sink.teleported.write(PlayerTeleported {
                            player: victim,
                            tile,
                        });
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
            CardEffect::Insulate { charges } => {
                // 护身叠层：两张免罪卡 = 抵两次。`None`（神明那种不限次）不被加法吃掉
                let stacked = match sink.insulations.get_mut(victim) {
                    Ok(mut existing) => {
                        // `None`（不限次）保持 `None`：`map` 天生不做这件事，
                        // 而「神明附身被一张免罪卡变成只剩一次」是不可接受的
                        existing.charges =
                            existing.charges.map(|held| held.saturating_add(*charges));
                        existing.charges
                    }
                    Err(_) => {
                        sink.commands.entity(victim).insert(Insulation {
                            charges: Some(*charges),
                        });
                        Some(*charges)
                    }
                };
                outcomes.push(EffectOutcome::Shielded {
                    charges: stacked.unwrap_or(*charges),
                });
            }
            CardEffect::LandStatus { status, days, area } => {
                // 锚点来自**请求里的目标**，不在卡面上：同一张「涨价卡」打在南京路还是
                // 上园路是玩家按下去那一刻才定的。抽卡那条路（`settle_card_landing`）
                // 永远没有 aim，所以数据若把这种效果写进机会/命运池，这里就是落空 ——
                // `CardSet::validate` 已经在加载期拦了（要求 target = land），这一句
                // debug 是第二道：坏数据不该静默改着游戏状态。
                let Some(crate::hand::CardAim::Land(center)) = aim else {
                    debug!("{} 要盖地块状态，但这次没有地块目标", def.id);
                    continue;
                };
                let lands =
                    sink.books
                        .stamp_status(&mut sink.commands, center, *status, *area, *days);
                outcomes.push(EffectOutcome::Stamped {
                    status: *status,
                    lands,
                    days: *days,
                });
            }
        }
    }
    outcomes
}

/// 用掉一层护身去挡这件厄运。挡住了才把空护身收走 —— 留着 `Some(0)` 会让面板上
/// 出现一个「有免罪卡但什么都不挡」的玩家。
fn absorb(
    insulations: &mut Query<&mut Insulation>,
    commands: &mut Commands,
    affliction: &Affliction,
) -> bool {
    let Ok(mut shield) = insulations.get_mut(affliction.victim) else {
        return false;
    };
    if !shield.absorb(affliction) {
        return false;
    }
    if shield.charges == Some(0) {
        commands.entity(affliction.victim).remove::<Insulation>();
    }
    true
}

// ===== 落格那一段：抽卡并当场结算效果 =====

/// 踩到机会 / 命运格：抽一张、立刻结算效果、把卡面挂成「等玩家读完」。
///
/// 对应 Godot `turn_manager.gd:389-407` + `card_effects.gd`。实质差别都收在
/// [`apply_card`] 那一段的注释里；这里只剩「从哪副牌抽、抽到的发给谁」这一层。
pub fn settle_card_landing(
    mut sink: CardSink,
    mut decks: Query<'_, '_, &'static mut Deck>,
    mut rng: ResMut<'_, Rng>,
    shelf: Shelf<'_>,
    landing: Res<'_, Landing>,
    mut pending: ResMut<'_, PendingDecision>,
    mut drawn: MessageWriter<'_, CardDrawn>,
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
    // `Item` 单独挡在这里：道具卡**不该被抽到**（它没有格子，来源是货架），
    // 而这一行存在的意义就是让「哪天有人给道具卡开一格」在这儿编译停一下。
    let kind = match expect {
        CardKind::Chance | CardKind::Fate => expect,
        CardKind::Item => return,
    };
    let Some(set) = shelf.set(kind) else {
        return;
    };
    let Some(mut deck) = decks.iter_mut().find(|d| d.kind == kind) else {
        return;
    };
    let Some(drawn_id) = deck.draw(&mut rng) else {
        debug!("{kind:?} 牌堆是空的，本回合不抽卡");
        return;
    };
    let Some(def) = card_of(set, drawn_id) else {
        return;
    };

    // 抽到的卡没有目标可选（`aim: None`）：两副事件池的效果都是打在抽卡人自己身上的，
    // 而带 `target` 的道具卡压根不在池里（`settle_card_landing` 对 `Item` 直接 return）
    let outcomes = apply_card(&mut sink, player, def, None);

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
                    cost_points: None,
                    target: crate::card::CardTarget::SelfOnly,
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
