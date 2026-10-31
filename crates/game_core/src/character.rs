//! 角色表（P2d）：首发十二人的静态档案。
//!
//! [CHARACTERS.md](../../../.agents/CHARACTERS.md) §九 第 3 条要的那份 `characters.json`。
//! **角色之间没有被动、没有主动、没有规则分支** —— 一行的机制差别只有表 B 那几格
//! 开局参数与一组 AI 先验（`AiProfile` 的五个偏好 + `fumble`），其余全是文案与外观。
//!
//! 今天规则层只消费其中三样：**名字、颜色、性别**（以及电脑座位的 `AiProfile`）。
//! `cash`／`deposit`／`loan`／`starting_lands` 是表 B 的原值，**四档起始资金统一覆盖
//! `cash`**（P2c 的口径），其余三格等角色经济落地 —— 它们先原样存着，免得届时要
//! 回头从文档里再抄一遍。
//!
//! 它是一份 **Asset**：主菜单的开局设置页要读它，与棋盘/牌堆/股票同一条加载路径。

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde::Deserialize;

use crate::ai::{AiProfile, Controller};
use crate::player::{Gender, PlayerColor, PlayerSetup};

/// 角色层级（CHARACTERS.md §三：资本 / 素人 / 怪人，各四人）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum CharacterTier {
    Capital,
    Amateur,
    Oddball,
}

impl CharacterTier {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Capital => "资本",
            Self::Amateur => "素人",
            Self::Oddball => "怪人",
        }
    }
}

/// 一位角色的档案。字段形状与 CHARACTERS.md §五 的表 A + 表 B 对齐。
#[derive(Debug, Clone, Deserialize)]
pub struct CharacterDef {
    /// 稳定键（kebab-case）。界面与将来的存档都认它，**不认名字** —— 名字是文案。
    pub id: String,
    /// 入境诨名，也就是棋盘上那个名字。
    pub name: String,
    /// 护照名（阿香那种没有护照的写空串）。
    pub passport: String,
    /// 注册地。
    pub home: String,
    pub tier: CharacterTier,
    /// 颜色名（酒红／沙金……），与下面的 RGB 成对；只上屏。
    pub color_name: String,
    /// 棋子/HUD 用的 RGB（0–255）。**不用十六进制字符串**：这里不需要给人手改，
    /// 而解析失败的回落值会把「颜色写错了」变成一颗白棋子。
    pub color: [u8; 3],
    /// 口头禅。
    pub quote: String,
    /// 剪影描述（将来接 sprite 的造型说明）。
    pub silhouette: String,
    /// 性别：魔法屋「所有女生／所有男生」两条筛选条件的唯一来源。
    pub gender: Gender,
    /// AI 先验。真人座位不读它。
    pub ai: AiProfile,
    /// 表 B 的起始现金。**今天被四档起始资金覆盖**，字段先存着。
    pub cash: i64,
    /// 表 B 的起始存款 / 贷款（角色经济未落地，字段先存着）。
    pub deposit: i64,
    pub loan: i64,
    /// 表 B 的起始地产块数（同上）。
    pub starting_lands: u32,
}

impl CharacterDef {
    pub fn player_color(&self) -> PlayerColor {
        let [r, g, b] = self.color;
        PlayerColor::srgb(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0)
    }

    /// 这个角色坐进一个座位时的开局配置。
    pub fn setup(&self, controller: Controller) -> PlayerSetup {
        PlayerSetup {
            name: self.name.clone(),
            color: self.player_color(),
            gender: self.gender,
            controller,
        }
    }
}

/// 整张角色表。JSON 顶层就是一个数组（与三副牌同一条形状）。
#[derive(Asset, TypePath, Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct CharacterSet(pub Vec<CharacterDef>);

impl CharacterSet {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn get(&self, id: &str) -> Option<&CharacterDef> {
        self.0.iter().find(|c| c.id == id)
    }

    /// 加载期能查的都在这儿查：重键、重色、AI 数值越界。
    ///
    /// **颜色必须两两不同**：棋子、建筑归属色与 HUD 圆点共用它，撞色之后
    /// 「谁的地」在小图上就读不出来（CHARACTERS.md §九 第 2 条）。
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.0.len() < 4 {
            errs.push(format!("角色表只有 {} 人，至少要 4 人", self.0.len()));
        }
        let mut ids = BTreeSet::new();
        let mut names = BTreeSet::new();
        let mut colors = BTreeSet::new();
        for c in &self.0 {
            if c.id.trim().is_empty() {
                errs.push("有角色没写 id".into());
            } else if !ids.insert(c.id.as_str()) {
                errs.push(format!("id 重复：{}", c.id));
            }
            if c.name.trim().is_empty() {
                errs.push(format!("{} 没写名字", c.id));
            } else if !names.insert(c.name.as_str()) {
                errs.push(format!("名字重复：{}", c.name));
            }
            if !colors.insert(c.color) {
                errs.push(format!("颜色重复：{} {:?}", c.name, c.color));
            }
            for (label, value) in [
                ("buy", c.ai.buy),
                ("debt", c.ai.debt),
                ("stock", c.ai.stock),
                ("build", c.ai.build),
                ("harm", c.ai.harm),
                ("fumble", c.ai.fumble),
            ] {
                if !(0.0..=1.0).contains(&value) {
                    errs.push(format!("{} 的 {label}={value} 不在 0..=1", c.name));
                }
            }
            if c.cash < 0 || c.deposit < 0 || c.loan < 0 {
                errs.push(format!("{} 的起始金额是负数", c.name));
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 仓库里那份真表：解析得动、校验得过、十二人、三层各四人、颜色两两不同。
    #[test]
    fn the_shipped_table_parses_and_passes_validation() {
        let set = CharacterSet::from_json(include_str!("../../../assets/data/characters.json"))
            .expect("characters.json 解析失败");
        set.validate().expect("characters.json 没过校验");
        assert_eq!(set.0.len(), 12, "首发十二人");
        for tier in [
            CharacterTier::Capital,
            CharacterTier::Amateur,
            CharacterTier::Oddball,
        ] {
            assert_eq!(
                set.0.iter().filter(|c| c.tier == tier).count(),
                4,
                "{} 层不是四人",
                tier.label()
            );
        }
    }

    /// 默认局的四个人必须在表里 —— 主菜单的设置页拿这四个 id 当默认选择。
    /// 名字是文案，比对的是 id：改名字不该把默认阵容弄丢。
    #[test]
    fn the_default_four_sit_in_the_table() {
        let set = CharacterSet::from_json(include_str!("../../../assets/data/characters.json"))
            .expect("解析");
        for id in ["ji-bainian", "lai-xi", "fan-yiwen", "duoji"] {
            assert!(set.get(id).is_some(), "默认阵容里的 {id} 不在表里");
        }
        assert_eq!(
            set.get("ji-bainian").unwrap().gender,
            Gender::Female,
            "姬百年的性别是魔法屋筛选的夹具之一"
        );
    }

    /// 校验器要真的有牙：重 id / 重色 / 越界的 `fumble` 各一条。
    #[test]
    fn validation_catches_duplicates_and_out_of_range_numbers() {
        let base = CharacterDef {
            id: "a".into(),
            name: "甲".into(),
            passport: "A".into(),
            home: "某地".into(),
            tier: CharacterTier::Capital,
            color_name: "红".into(),
            color: [255, 0, 0],
            quote: "…".into(),
            silhouette: "…".into(),
            gender: Gender::Female,
            ai: AiProfile::PLAIN,
            cash: 20_000,
            deposit: 0,
            loan: 0,
            starting_lands: 0,
        };
        let mut clone = base.clone();
        clone.id = "a".into();
        clone.name = "乙".into();
        clone.color = [0, 255, 0];
        let mut other = base.clone();
        other.id = "b".into();
        other.name = "丙".into();
        other.color = [0, 0, 255];
        let mut bad = base.clone();
        bad.id = "c".into();
        bad.name = "丁".into();
        bad.color = [1, 1, 1];
        bad.ai.fumble = 1.5;

        let errs = CharacterSet(vec![base.clone(), clone, other, bad])
            .validate()
            .expect_err("重 id 与越界该被拒");
        assert!(errs.iter().any(|e| e.contains("id 重复")), "{errs:?}");
        assert!(errs.iter().any(|e| e.contains("fumble")), "{errs:?}");
    }

    /// 颜色与性别从数据来，不从索引来：同一个人在任何座位都是同一种色。
    #[test]
    fn a_character_carries_its_own_color_and_gender_into_a_seat() {
        let def = CharacterDef {
            id: "x".into(),
            name: "某".into(),
            passport: String::new(),
            home: "某地".into(),
            tier: CharacterTier::Oddball,
            color_name: "深紫".into(),
            color: [91, 42, 134],
            quote: "…".into(),
            silhouette: "…".into(),
            gender: Gender::Male,
            ai: AiProfile::GOODY,
            cash: 8_000,
            deposit: 0,
            loan: 0,
            starting_lands: 0,
        };
        let seat = def.setup(Controller::Human);
        assert_eq!(seat.name, "某");
        assert_eq!(seat.gender, Gender::Male);
        assert_eq!(seat.color.0, [91.0 / 255.0, 42.0 / 255.0, 134.0 / 255.0]);
        assert_eq!(seat.controller, Controller::Human);
    }
}
