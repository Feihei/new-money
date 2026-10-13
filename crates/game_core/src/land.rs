//! 土地实体：静态定义留在 `BoardDef.lands`，运行时状态是这里的一组组件。
//!
//! Godot 端的 `PropertyRegistry`（128 行弱类型 `Dictionary`）整个消失 —— 归属/等级/抵押
//! 直接是 Land 实体上的组件，于是缺陷 #1 那类「key 类型搞错但运行时才发现」的错误
//! 没有了存在空间。

use std::collections::HashMap;

use bevy::prelude::*;

use crate::board::BoardDef;
use crate::player::PlayerId;

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LandId(pub u32);

/// 归属。`Owner(None)` = 无主、可买；`Owner(Some(p))` = p 的地产。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Owner(pub Option<PlayerId>);

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
            let entity = commands.spawn((id, Owner::default())).id();
            (id, entity)
        })
        .collect()
}
