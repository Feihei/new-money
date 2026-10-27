//! P10① 的验收：收费原型库与管线的接缝。
//!
//! PLAN §四 P10 那行验收原话是「**同一原型换两个参数就得出旅馆与航空**」——
//! 原型那一侧（`assess` 的输入输出）在 `game_core/src/service.rs` 的单元测试里逐格钉过；
//! 这里补的是另一半：**原型算出的那笔账，过结算时是「过路费一族」待遇** ——
//! 物价指数缩它、大财神赖它（这条链），免它、转它（③ 之后那是被动卡那道门的两个问句，
//! 判据是「这笔账归表上哪一行」）。这几条如果有一条要为服务费单独写代码，
//! P2 那句「谁都不用回来改这条链」就破了。
//!
//! 「踩到商业格 → 摇数 → 出账 → 扣款 → B 型挪人」那一整趟要等 P10b 的
//! `building_kind` 与 P10c 的落格装配才存在，本文件只验收库里这两半。

use bevy::ecs::entity::Entity;
use game_core::prelude::{
    Adjustment, Charge, ChargeDomain, ChargeKind, Correction, Corrections, FeePrototype, LandId,
    Mount, PassiveEvent, Scope, ServiceBill, Source, assess_service, charge_domain_of,
    scales_with_price, stage, transferable,
};

fn player(n: u32) -> Entity {
    Entity::from_raw_u32(n).expect("索引不该撞上 Entity 的保留值")
}

/// 加油站：油费按步数、只对载具收（10-20 拍板的载体参数）。
fn gas_station() -> FeePrototype {
    FeePrototype::PerStep {
        rate: 100,
        requires_mount: true,
    }
}

/// 购物中心：骰 1–6，摇出几走第几档。
fn mall() -> FeePrototype {
    FeePrototype::RandomAmount {
        sides: 6,
        tiers: vec![100, 200, 300, 400, 500, 600],
    }
}

/// 原型出的账，装配层拿去 `Books::settle` 的那一句长什么样。这里固定它：
/// kind 是 `Service`、付方是踩格者、收方按载体是地主或董事长 —— 装配层（P10c/P10d）
/// 不该有自己的第二种写法，否则「免费卡为什么吃得到购物中心、吃不到买地」就说不清了。
fn service_charge(payer: Entity, payee: Entity, bill: ServiceBill) -> Charge {
    Charge::new(ChargeKind::Service, Some(payer), Some(payee), bill.base)
}

/// 三张表对 `Service` 的裁定：归「过路费」域（P2 写下那行注释时等的就是它）、
/// 受物价缩放、可嫁祸。改任何一位都是改平衡，不许在一次重构里顺手发生。
#[test]
fn service_fees_belong_to_the_rent_family_in_all_three_tables() {
    assert_eq!(charge_domain_of(ChargeKind::Service), ChargeDomain::Rent);
    assert!(scales_with_price(ChargeKind::Service));
    assert!(transferable(ChargeKind::Service));
}

/// 摇出的数 × 档位表出来的名义额，过完物价还乘得出整数 ——
/// 物价 2.0 时购物中心的 300 档收 600。
#[test]
fn the_price_index_scales_a_service_bill() {
    let bill = assess_service(&mall(), 3, 0, Mount::Foot).expect("购物中心必有账");
    let charge = service_charge(player(1), player(2), bill);
    assert_eq!(stage(charge, 2.0, &Corrections::default()).amount, 600);
}

/// 免费卡与嫁祸卡**不在门下的这条链上了**（③）：它们成了那道门的两个问句，而门问的是
/// 「这一笔账归表上哪一行」。这里钉的是换完家之后的那句话 —— 服务费不必为自己那两种卡
/// 多写一行：它与过路费读同一张表、两列答案相同。
///
/// 与下面那条「三张表」的测试合起来，就是 P2 那句「谁都不用回来改这条链」在③之后的形状。
#[test]
fn the_door_asks_a_service_bill_exactly_like_a_rent() {
    for kind in [
        ChargeKind::Service,
        ChargeKind::Rent { land: LandId(3) },
        ChargeKind::Penalty,
        ChargeKind::Levy,
    ] {
        let row = PassiveEvent::of_charge(kind);
        assert!(row.forgivable(), "{kind:?} 该被免费卡问一句");
        assert!(row.shiftable(), "{kind:?} 该被嫁祸卡问一句");
        let charge = Charge::new(kind, Some(player(1)), Some(player(2)), 3_000);
        assert!(charge.cancellable, "{kind:?} 大财神那一支要吃得到");
        assert!(charge.transferable, "{kind:?} 嫁祸那一支要转得出去");
    }
    // 答案相同是**两个数**而不是一个数：它是表上两行，改一种钱不该连坐另一种
    assert_ne!(
        PassiveEvent::of_charge(ChargeKind::Service),
        PassiveEvent::of_charge(ChargeKind::Rent { land: LandId(3) }),
        "服务费与过路费塌成一行，「下一句只改其中一种」就写不出来了"
    );
}

/// 大财神赖账那一支也吃得到：`Source::God` 的 `Cancel` 挂在付方身上，
/// 购物中心的档位账归零。神明的豁免表（`Source::God`）与卡片共用同一条链。
#[test]
fn the_money_god_dodges_a_service_bill_too() {
    let mut corrections = Corrections::default();
    corrections.register(Correction::always(
        player(1),
        Scope::Domain(ChargeDomain::Rent),
        Adjustment::Cancel,
        Source::God,
    ));
    let bill = assess_service(&mall(), 5, 0, Mount::Foot).unwrap();
    let charge = service_charge(player(1), player(2), bill);
    assert_eq!(stage(charge, 1.0, &corrections).amount, 0);

    // 别人的账他赖不掉：链按 bearer 找人，服务费在这条纪律上与租金没有差别
    let bill = assess_service(&mall(), 5, 0, Mount::Foot).unwrap();
    let charge = service_charge(player(2), player(1), bill);
    assert_eq!(stage(charge, 1.0, &corrections).amount, 500);
}

/// 嫁祸那一句在③之后**不在这条链上**：`Adjustment::TransferTo` 那一位删了，改付款人是
/// 门在花掉那张卡那一刻干的（[`crate::passive`]）。这里钉的是留给这条链的那半句 ——
/// 过管线的是**实际掏钱那位**的修正，而收款方不换（「他替我付」不是「钱给他」）。
/// 大财神附身的人替人付账时赖得掉，正是这一格的形状。
#[test]
fn a_shifted_bill_is_staged_for_whoever_actually_pays() {
    let mut corrections = Corrections::default();
    corrections.register(Correction::always(
        player(2),
        Scope::Domain(ChargeDomain::Rent),
        Adjustment::Cancel,
        Source::God,
    ));
    // 步行踩加油站没有账（载体参数那一格），开车踩了才有 —— 挑有账的那条
    let bill = assess_service(&gas_station(), 0, 3, Mount::Car).expect("开车才有油费");
    let mine = service_charge(player(1), player(3), bill);
    assert_eq!(
        stage(mine, 1.0, &corrections).amount,
        300,
        "P1 身上没有那位神，账照付"
    );

    let shifted = Charge {
        payer: Some(player(2)),
        ..mine
    };
    let done = stage(shifted, 1.0, &corrections);
    assert_eq!(done.amount, 0, "替他付的那位自己有豁免，这笔就走他那一遍");
    assert_eq!(
        done.payee,
        Some(player(3)),
        "收款方不换：他替我付，不是钱给他"
    );
}

/// 摇数进账、账进管线，两层各自的「不产生账」不许被中间层伪造出一条 0 元流水：
/// D 型与「步行踩加油站」在 `assess` 就是 `None`，装配层据此**不建账** ——
/// 这一格钉的是「`None` 与 0 元是两件事」，0 元流水是大财神那种豁免的形状。
#[test]
fn no_bill_means_no_charge_row_at_all() {
    assert_eq!(
        assess_service(&FeePrototype::Functional, 4, 5, Mount::Car),
        None
    );
    assert_eq!(assess_service(&gas_station(), 0, 5, Mount::Foot), None);
}
