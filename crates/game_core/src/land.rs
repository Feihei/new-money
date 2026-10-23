//! 土地实体：静态定义留在 `BoardDef.lands`，运行时状态是这里的一组组件。
//!
//! Godot 端的 `PropertyRegistry`（128 行弱类型 `Dictionary`）整个消失 —— 归属/等级/抵押
//! 直接是 Land 实体上的组件，于是缺陷 #1 那类「key 类型搞错但运行时才发现」的错误
//! 没有了存在空间。

use std::collections::HashMap;

use bevy::prelude::*;

use crate::board::BoardDef;
use crate::player::PlayerId;
use crate::world::Gameplay;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LandId(pub u32);

/// 归属。`Owner(None)` = 无主、可买；`Owner(Some(p))` = p 的地产。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Owner(pub Option<PlayerId>);

/// 建筑等级：**0 = 空地**（无主、刚买下都算；10-23 级数重判，见 PLAN §三），
/// 1..=5 可升（空地→平房→店铺→商场→商业大楼→摩天大楼，加盖 5 次）；
/// 租金查 `rent[level]`，六行租金表 0..=5 正好对上。上限在 `GameRules.max_building_level`。
///
/// 为什么 0 起算而不是买地钳到 1：`rent[level]` 直接对下标、`estate_value` 的
/// 「建筑数 = level」不再差一格，而「无主却带楼」（破产残留、天使白给）与
/// 「无主的空地」从此是同一个数轴上的两个真状态，没有第三种「0 级其实是一级」
/// 的翻译。屏幕上的名字见 `game_app::ui::building_label`。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct BuildingLevel(pub u8);

/// 抵押中的本金。`proceeds > 0` 即「这块地押在银行」：租金为 0、不计入垄断判定与贷款
/// 额度、不可升级（对应 Godot `PropertyRegistry.is_mortgaged` 的三处检查）。
///
/// **为什么是「每块地都挂着、用 0 表示没押」而不是「存在即押」的单位标记**：赎回要收的不
/// 是「当初的价」而是「当初到手那笔 + 手续费」，那个数必须存下来 —— 按公式重算要把「抵押中
/// 的地永远不会变」这条不变式借给一次算术，而它只是*恰好*成立（`can_upgrade` 今天挡着加盖，
/// 明天未必）。既然要存数，就顺手把延迟写的那个洞一起堵上：`Option<&Mortgaged>` + `Commands`
/// 的写法下，同一帧里两条抵押同一块地的请求都读得到「还没押」，于是钱会付两遍；
/// `&mut Mortgaged` 就地改，第二条请求当场看见。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mortgaged {
    /// 当初抵押到手的那笔钱（名义值，不随物价指数走 —— 它是一笔债，不是一份资产）。
    pub proceeds: i64,
}

impl Mortgaged {
    /// 这块地此刻押在银行吗。
    pub fn held(&self) -> bool {
        self.proceeds > 0
    }
}

/// 从棋盘定义铺出 Land 实体，返回 `LandId → Entity` 索引。
///
/// 实体上不重复存 `LandDef`：价格/租金是静态配置，用 `active.def.land(id)` 查，
/// 两处都存迟早对不上。
pub fn spawn_lands(commands: &mut Commands, board: &BoardDef) -> HashMap<LandId, Entity> {
    board
        .lands
        .iter()
        .map(|l| {
            let id = LandId(l.id);
            let entity = commands
                .spawn((
                    id,
                    Owner::default(),
                    BuildingLevel::default(),
                    Mortgaged::default(),
                    Gameplay,
                ))
                .id();
            (id, entity)
        })
        .collect()
}
