//! 地产结算流水线那一段：无主地报价、他人地收租、自有地升级报价。
//!
//! 对应 Godot `turn_manager.gd:268-311`（`_handle_property_landing`）+ `property_system.gd`
//! 的四个动作。判定与算账都在 `economy` 的纯函数里，这里只做「查组件 → 调判定 →
//! 改组件 → 发消息」四步，所以 `test_property_system` 那 20 条 GUT 用例全部
//! 落在 `economy.rs` 的表驱动单测上，不依赖 ECS。

use bevy::prelude::*;

use crate::charge::{Charge, ChargeKind};
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
        //
        // 但**必败**那一格不给弹（P6b② 大衰神）：报价与成交读的是同一句
        // [`Books::buy_price`]，弹一张按下去必然失败的模态，比不弹更糟 ——
        // 玩家会以为是自己按错了，而不是身上那位神不让买。
        None if books.acquire_denied(player) => {
            let god = books.god_of(player).map(|held| held.god);
            debug!(
                "{} 想踩下这块无主地，但{}在场 —— 买地建屋必败",
                books.id_of(player).map(|p| p.0).unwrap_or_default(),
                god.map_or(String::new(), |g| g.label().to_string())
            );
        }
        None => {
            // 土地公踩到无主地**不问价**（判据在 `Books::takes_landing`，过户由排在后面的
            // `settle_land_deity_landing_system` 做）。这一句必须问在弹窗口之前：先弹一张
            // 「是否购买此地」、答完才发现地已经归他，那是一个答完就失效的窗口
            if books.takes_landing(player) {
                debug!("无主地 L{} 不问价：他身上是土地公", land.0);
            } else {
                pending.offer(player, Pending::Buy(land));
                offers.write(BuyOffered { player, land });
            }
        }
        // 自己的地 → 只在真能升级时提问（Godot 的 `can_upgrade` 分支）。
        // 报价走管线，所以模态上那个 `cost` 与买家按下「升级」时扣掉的数是同一个。
        Some(owner) if owner == self_id => {
            let base = economy::upgrade_cost(def, &view).unwrap_or(0);
            let cost = books.quoted(ChargeKind::Upgrade { land }, player, base);
            let views = books.views();
            if !books.acquire_denied(player)
                && economy::can_upgrade(&view, owner, &views, cost, books.balance(player), rules)
            {
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
            let charge = Charge::new(
                ChargeKind::Rent { land },
                Some(player),
                Some(owner_entity),
                rent,
            );
            // `payer` 取成交后那个：被嫁祸卡转手时，收据要说的是**实际付钱的人**
            if let Ok(done) = books.settle(&mut commands, charge) {
                rents.write(RentDue {
                    payer: done.payer.unwrap_or(player),
                    owner: owner_entity,
                    land,
                    amount: done.amount,
                });
            }
            pending.offer(player, Pending::RentNotice(land));
        }
    }
}
