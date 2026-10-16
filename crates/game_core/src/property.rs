//! 地产结算流水线那一段：无主地报价、他人地收租、自有地升级报价。
//!
//! 对应 Godot `turn_manager.gd:268-311`（`_handle_property_landing`）+ `property_system.gd`
//! 的四个动作。判定与算账都在 `economy` 的纯函数里，这里只做「查组件 → 调判定 →
//! 改组件 → 发消息」四步，所以 `test_property_system` 那 20 条 GUT 用例全部
//! 落在 `economy.rs` 的表驱动单测上，不依赖 ECS。

use bevy::prelude::*;

use crate::economy;
use crate::msg::{BuyOffered, RentDue, UpgradeOffered};
use crate::settle::Books;

use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};

/// 踩到地产格：三种归属三种走法（无主 / 自己 / 别人），与 Godot 完全一致。
pub fn settle_property_landing(
    mut commands: Commands,
    mut books: Books,
    landing: Res<Landing>,
    mut pending: ResMut<PendingDecision>,
    mut offers: MessageWriter<BuyOffered>,
    mut upgrades: MessageWriter<UpgradeOffered>,
    mut rents: MessageWriter<RentDue>,
) {
    // 只有地产格带土地；其余格（含 `land_id` 为 `None` 的坏数据）直接跳过
    let Some(Landed {
        player,
        land: Some(land),
        domain: LandingDomain::Property,
        ..
    }) = landing.0
    else {
        return;
    };
    let Some(view) = books.view(land) else {
        return;
    };
    let Some(self_id) = books.id_of(player) else {
        return;
    };
    let def = &books.active.def;
    let rules = &books.rules;

    match view.owner {
        // 无主 → 挂牌待决。Godot 也是「不管买不买得起都弹一张」（`confirm_buy` 里
        // 才做二次判定），差别在于这里买不起就是 `None`，不会静默扣成负数。
        None => {
            pending.offer(player, Pending::Buy(land));
            offers.write(BuyOffered { player, land });
        }
        // 自己的地 → 只在真能升级时提问（Godot 的 `can_upgrade` 分支）
        Some(owner) if owner == self_id => {
            let views = books.views();
            if economy::can_upgrade(def, &view, owner, &views, books.balance(player), rules) {
                let cost = economy::upgrade_cost(def, &view).unwrap_or(0);
                pending.offer(player, Pending::Upgrade(land));
                upgrades.write(UpgradeOffered { player, land, cost });
            }
        }
        // 别人的地 → 当场收租，再弹一张收据（顺序照搬 Godot：先扣钱后通知，
        // 所以「破产」这件事在通知出现之前就已经是真的了）
        Some(owner) => {
            let Some(owner_entity) = books.holder(owner) else {
                return;
            };
            let rent = books.rent_of(land);
            if rent <= 0 {
                return;
            }
            if books.charge(&mut commands, player, rent).is_ok() {
                books.credit(owner_entity, rent);
                rents.write(RentDue {
                    payer: player,
                    owner: owner_entity,
                    land,
                    amount: rent,
                });
            }
            pending.offer(player, Pending::RentNotice(land));
        }
    }
}
