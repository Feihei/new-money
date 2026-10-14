//! 卡牌定义：机会 / 命运牌池。效果用内部标签枚举，新增 action 时编译器会强制补分支。
//!
//! 对应 Godot 端 `card_data.gd` + `card_effects.gd`（后者是 `Dictionary` 弱类型 match，
//! 且 `deduct_cash` 失败时只回文案不真的判破产 —— 见 `.agents/MIGRATION.md` §三 第 3 条）。

use bevy::asset::Asset;
use bevy::reflect::TypePath;
use serde::Deserialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Chance,
    Fate,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CardEffect {
    AddCash {
        amount: i64,
    },
    DeductCash {
        amount: i64,
    },
    GoToJail {
        #[serde(default = "default_jail_turns")]
        turns: u32,
    },
    /// 按格名瞬移。格名是数据与棋盘的隐式契约，`validate` 只能查非空，
    /// 真正「存不存在」要等 B3 拿到 BoardDef 才能校验。
    MoveTo {
        tile_name: String,
    },
}

const fn default_jail_turns() -> u32 {
    3
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CardDef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: CardKind,
    pub title: String,
    pub desc: String,
    #[serde(default)]
    pub effects: Vec<CardEffect>,
}

#[derive(Debug, Clone, PartialEq, Asset, TypePath, Deserialize)]
#[serde(transparent)]
pub struct CardSet(pub Vec<CardDef>);

impl CardSet {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// `expect_kind` 让「把机会卡写进命运文件」这类数据错误在加载期就暴露。
    pub fn validate(&self, expect_kind: CardKind) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();

        if self.0.is_empty() {
            return Err(vec!["牌池为空".into()]);
        }

        let mut seen = std::collections::HashSet::new();
        for c in &self.0 {
            if c.id.is_empty() {
                errs.push("有卡牌 id 为空".into());
            } else if !seen.insert(c.id.as_str()) {
                errs.push(format!("卡牌 id 重复: {}", c.id));
            }
            if c.kind != expect_kind {
                errs.push(format!(
                    "卡牌 {} 的 type={:?} 与牌池 {:?} 不符",
                    c.id, c.kind, expect_kind
                ));
            }
            if c.title.is_empty() {
                errs.push(format!("卡牌 {} 缺 title", c.id));
            }
            if c.desc.is_empty() {
                errs.push(format!("卡牌 {} 缺 desc", c.id));
            }
            if c.effects.is_empty() {
                errs.push(format!("卡牌 {} 没有任何效果", c.id));
            }
            for e in &c.effects {
                match e {
                    CardEffect::AddCash { amount } | CardEffect::DeductCash { amount } => {
                        if *amount <= 0 {
                            errs.push(format!("卡牌 {} 的现金效果数值非法: {amount}", c.id));
                        }
                    }
                    CardEffect::GoToJail { turns } => {
                        if *turns == 0 {
                            errs.push(format!("卡牌 {} 的关押回合数为 0", c.id));
                        }
                    }
                    CardEffect::MoveTo { tile_name } => {
                        if tile_name.is_empty() {
                            errs.push(format!("卡牌 {} 的 move_to 缺 tile_name", c.id));
                        }
                    }
                }
            }
        }

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANCE: &str = include_str!("../../../assets/data/cards/chance_cards.json");
    const FATE: &str = include_str!("../../../assets/data/cards/fate_cards.json");

    #[test]
    fn both_pools_parse_and_validate() {
        let chance = CardSet::from_json(CHANCE).expect("机会牌应能解析");
        let fate = CardSet::from_json(FATE).expect("命运牌应能解析");
        assert!(chance.validate(CardKind::Chance).is_ok());
        assert!(fate.validate(CardKind::Fate).is_ok());
        assert!(chance.0.len() >= 5, "机会牌数偏少: {}", chance.0.len());
        assert!(fate.0.len() >= 5, "命运牌数偏少: {}", fate.0.len());
    }

    #[test]
    fn all_four_effect_kinds_are_covered() {
        let all = CardSet::from_json(CHANCE)
            .unwrap()
            .0
            .into_iter()
            .chain(CardSet::from_json(FATE).unwrap().0)
            .flat_map(|c| c.effects)
            .collect::<Vec<_>>();

        let has = |pred: fn(&CardEffect) -> bool| all.iter().any(pred);
        assert!(has(|e| matches!(e, CardEffect::AddCash { .. })));
        assert!(has(|e| matches!(e, CardEffect::DeductCash { .. })));
        assert!(has(|e| matches!(e, CardEffect::GoToJail { turns: 3 })));
        assert!(has(
            |e| matches!(e, CardEffect::MoveTo { tile_name } if tile_name == "起点")
        ));
    }

    #[test]
    fn card_ids_are_unique_within_pool() {
        let mut set = CardSet::from_json(CHANCE).unwrap();
        let first = set.0[0].clone();
        set.0.push(first);
        assert!(set.validate(CardKind::Chance).is_err());
    }

    #[test]
    fn wrong_kind_is_rejected() {
        let mut set = CardSet::from_json(CHANCE).unwrap();
        set.0[0].kind = CardKind::Fate;
        let errs = set.validate(CardKind::Chance).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("不符")), "{errs:?}");
    }

    #[test]
    fn empty_effects_and_bad_amounts_are_rejected() {
        let mut set = CardSet::from_json(FATE).unwrap();
        set.0[0].effects.clear();
        assert!(set.validate(CardKind::Fate).is_err());

        let mut set = CardSet::from_json(FATE).unwrap();
        set.0[0].effects = vec![CardEffect::DeductCash { amount: 0 }];
        assert!(set.validate(CardKind::Fate).is_err());
    }
}
