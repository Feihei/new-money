//! P10① 的验收：收费原型库与管线的接缝。
//!
//! PLAN §四 P10 那行验收原话是「**同一原型换两个参数就得出旅馆与航空**」——
//! 原型那一侧（`assess` 的输入输出）在 `game_core/src/service.rs` 的单元测试里逐格钉过；
//! 这里补的是另一半：**原型算出的那笔账，过修正管线时是「过路费一族」待遇** ——
//! 物价指数缩它、免费卡免它、大财神赖它、嫁祸卡转它。这四条如果有一条要为服务费
//! 单独写代码，P2 那句「谁都不用回来改这条链」就破了。
//!
//! 「踩到商业格 → 摇数 → 出账 → 扣款 → B 型挪人」那一整趟要等 P10b 的
//! `building_kind` 与 P10c 的落格装配才存在，本文件只验收库里这两半。

use bevy::ecs::entity::Entity;
use game_core::prelude::{
    Adjustment, Charge, ChargeDomain, ChargeKind, Correction, Corrections, FeePrototype, Mount,
    Scope, Source, ServiceBill, assess_service, charge_domain_of, scales_with_price, stage,
    stage_and_spend, transferable,
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
    Charge::new(
        ChargeKind::Service,
        Some(payer),
        Some(payee),
        bill.base,
    )
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

/// 免费卡免服务费：免罪卡挂在踩格者身上，一笔够得着门槛（2000）的旅馆账被整笔吃掉，
/// 且**只烧一张** —— 与免租金共用同一条判据，没有为服务费新写一行。
/// 顺带钉住门槛那半条：不够门槛的账不免、也不烧卡。
#[test]
fn a_free_pass_cancels_a_service_bill_and_burns_exactly_one_card() {
    let pricey = FeePrototype::RandomStay {
        sides: 4,
        per_day: 1_500,
    };
    let mut corrections = Corrections::default();
    corrections.register(Correction::one_shot(
        player(1),
        Scope::Any,
        Adjustment::Cancel,
        Source::Card,
        2_000,
    ));

    // 摇出 1 天：名义 1500，没过门槛 —— 不免、也不烧卡
    let bill = assess_service(&pricey, 1, 0, Mount::Foot).expect("旅馆必有账");
    let (staged, spent) = stage_and_spend(service_charge(player(1), player(2), bill), 1.0, &corrections);
    assert_eq!(staged.amount, 1_500, "没过门槛的账照付");
    assert!(spent.is_empty(), "没生效就不该烧卡");

    // 摇出 2 天：名义 3000，过了门槛 —— 整笔吃掉、烧一张
    let bill = assess_service(&pricey, 2, 0, Mount::Foot).unwrap();
    let (staged, spent) = stage_and_spend(service_charge(player(1), player(2), bill), 1.0, &corrections);
    assert_eq!(staged.amount, 0, "免费卡该吃掉这笔服务费");
    assert_eq!(spent.len(), 1, "一笔账只烧一张卡");

    // 第二笔（另一名玩家踩了同一间旅馆）不再有卡可烧
    let bill = assess_service(&pricey, 1, 0, Mount::Foot).unwrap();
    let (staged, spent) = stage_and_spend(service_charge(player(2), player(1), bill), 1.0, &corrections);
    assert_eq!(staged.amount, 1_500, "卡已经花了，第二笔照付");
    assert!(spent.is_empty());
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

/// 嫁祸卡能把服务费转出去：`transferable` 那一位放开了它，`TransferTo` 就该真转 ——
/// 转出去之后付款人换了，而金额是原账的（转嫁不走新付款人的修正）。
#[test]
fn a_service_bill_can_be_shifted_to_someone_else() {
    let mut corrections = Corrections::default();
    corrections.register(Correction::always(
        player(1),
        Scope::Any,
        Adjustment::TransferTo { to: player(2) },
        Source::Card,
    ));
    // 步行踩加油站没有账（载体参数那一格），开车踩了才有 —— 挑有账的那条
    let bill = assess_service(&gas_station(), 0, 3, Mount::Car).expect("开车才有油费");
    let charge = service_charge(player(1), player(2), bill);
    let staged = stage(charge, 1.0, &corrections);
    assert_eq!(staged.payer, Some(player(2)), "账转给了被嫁祸的人");
    assert_eq!(staged.amount, 300, "转嫁不改金额");
}

/// 摇数进账、账进管线，两层各自的「不产生账」不许被中间层伪造出一条 0 元流水：
/// D 型与「步行踩加油站」在 `assess` 就是 `None`，装配层据此**不建账** ——
/// 这一格钉的是「`None` 与 0 元是两件事」，0 元流水是大财神那种豁免的形状。
#[test]
fn no_bill_means_no_charge_row_at_all() {
    assert_eq!(assess_service(&FeePrototype::Functional, 4, 5, Mount::Car), None);
    assert_eq!(assess_service(&gas_station(), 0, 5, Mount::Foot), None);
}
