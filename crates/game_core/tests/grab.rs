//! P3d④a 的验收：购地卡买的是**不用走到那一格**。
//!
//! 这张卡的形状与涨价/查封只差一格 `CardTarget`（`Land` 与 `UnownedLand`），而那一格
//! 差的是两件相反的事：涨价的承受者是那块地的业主，购地的承受者是出牌人自己。
//! 所以这批用例真正在钉的是三件事：
//!
//! 1. **走的是买地的正路**（`Books::buy`）：地价过物价指数、过修正登记处，扣完发
//!    `LandBought` —— 与踩到那一格买下来完全同一条，不是卡牌自己另写的一次转账。
//! 2. **候选判据与账本同源**（`Books::buy_price`）：有主的地、买不起的地、棋盘上没有的
//!    地，出牌那三道门一律静默拒绝，卡留在手里 —— 因为面板那颗按钮用的就是这同一句判据，
//!    「按下去没声音」与「按钮亮着却办不成」都不该出现。
//! 3. **回合不被推进**：出牌是玩家在自己回合里的自由动作（P3a 那条不改）。
//!
//! 「抽到即静默失效」那一半不在这里测：`buy_land` 的效果与 `unowned_land` 的目标在
//! `CardSet::validate` 里互相绑定（`card.rs` 的单测各测一个反例），坏数据到不了运行时。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    Adjustment, CardAim, ChargeDomain, ChargeKind, Correction, LandId, Scope, Source, TurnPhase,
    charge_domain_of,
};

/// 装配到「P0 的回合、掷骰之前」，返回 app 与两名玩家（P0 是当前回合玩家）。
fn two_players_before_the_roll() -> (App, Entity, Entity) {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = roster(&mut app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家");
    (app, p0, p1)
}

/// 把购地卡发到 `player` 手里并按下「→ 某块地」那颗按钮。
fn grab(app: &mut App, player: Entity, land: u32) {
    let card = item_index(app, "item_grab");
    deal(app, player, card.0);
    play_card(app, player, card.0, Some(CardAim::Land(LandId(land))));
    settle(app, 2);
}

/// prototype 盘上 L3（绑在 7 号格上）的名义地价 —— 用例里的数都从这条来，
/// 改数据时这里先红，而不是让六条用例各猜一个数。
const L3_PRICE: i64 = 3800;
/// L3 加盖一层的名义费（同一份地块数据里的 `upgrade_cost`）。
const L3_STOREY: i64 = 1900;

#[test]
fn buying_a_plot_from_the_other_side_of_the_ring_costs_exactly_a_normal_purchase() {
    // P0 站在 0 号格（夹具钉住的座位），买下的是 7 号格上那块地 —— 中间隔着一整条路。
    // 这张卡买的就是这个「不用走过去」，而价钱、归属、等级、流水都该与走过去买一模一样
    let (mut app, p0, _p1) = two_players_before_the_roll();
    grab(&mut app, p0, 3);

    assert_eq!(
        plot_state(&mut app, 3),
        (Some(0), 0, false),
        "刚买下的空地：归 P0、还是 Lv0（级数重判：0=空地，`Books::buy` 不再钳层）、没押着"
    );
    assert_eq!(cash_of(&mut app, p0), 20_000 - L3_PRICE, "扣的是名义地价");
    // 人还在 0 号格：这张卡买的就是「不必走过去」
    assert_eq!(tile_of(&app, p0), 0);
    assert!(hand_of(&app, p0).is_empty(), "打出去就该把占的那格还回来");
    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "出牌不推进回合（§4.6 红线）"
    );

    let log = ledger(&app);
    assert!(
        log.contains(&format!("Bought(P0@3${L3_PRICE})")),
        "归属变了要留正路那条流水：{log:?}"
    );
    assert!(
        log.contains(&"Play(P0->P0:item_grab)".to_string()),
        "出牌也要留流水：{log:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 无主而**立着楼**的地，购地卡也一样连楼收钱（10-22 实报：破产之后剩下的地归无主而建筑保留，
/// 走到那一格的人要「一次买下地+上面的建筑」）。
///
/// 这条不是补一个边缘情况，而是堵住一张套利券：判据若退回「只看有没有主人 + 只按地价」，
/// 那么「先打一槌拍卖把某人打成破产、再花 30 点券用购地卡白拿他三层的楼」就是全场最便宜的开发
/// 路径，而 40 点券那张卡本来买的是「不必走到那一格」，不是折扣。
/// 等级那一半同样要断：收了楼的钱却把楼钳回 Lv1，等于把买主刚付的那两层当场拆掉。
#[test]
fn a_grab_over_a_house_pays_for_the_house_and_keeps_it() {
    let (mut app, p0, _p1) = two_players_before_the_roll();
    own(&mut app, 3, None, 3, false); // 无主的徐家汇，立着三层

    grab(&mut app, p0, 3);

    assert_eq!(
        plot_state(&mut app, 3),
        (Some(0), 3, false),
        "买到的是一栋三层的楼，不是一块被抹平的空地"
    );
    let want = L3_PRICE + 3 * L3_STOREY; // 级数重判后建筑数 = level：三层 = 三次加盖
    assert_eq!(cash_of(&mut app, p0), 20_000 - want, "地价 + 已盖的每一层");
    assert!(
        ledger(&app).contains(&format!("Bought(P0@3${want})")),
        "收据上那个数得与扣的钱是同一个：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn the_price_paid_is_the_one_the_pipeline_quotes_not_the_one_the_card_says() {
    // 卡面上没有价，这是重点：物价 2 倍时这张卡该多掏一倍，正如过路费会翻倍。
    // 若把 3800 写死在卡上，它就成了一张「永远按开局地价成交」的套利券
    let (mut app, p0, _p1) = two_players_before_the_roll();
    set_price_index(&mut app, 2.0);
    grab(&mut app, p0, 3);

    assert_eq!(plot_state(&mut app, 3).0, Some(0), "贵一倍还是要买得下来");
    assert_eq!(cash_of(&mut app, p0), 20_000 - 2 * L3_PRICE);
    assert!(
        ledger(&app).contains(&format!("Bought(P0@3${})", 2 * L3_PRICE)),
        "流水上的数也得是翻倍之后的：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_blessing_that_makes_acquisitions_free_makes_this_card_free_too() {
    // 「买地免费」是 P6 大福神的特权（PLAN §三），而它落在修正登记处那一侧 ——
    // 购地卡不必认识它。这条用例证明这张卡真的走管线，而不是自己 `debit` 一次
    let (mut app, p0, _p1) = two_players_before_the_roll();
    assert_eq!(
        charge_domain_of(ChargeKind::BuyLand { land: LandId(3) }),
        ChargeDomain::Acquire
    );
    register(
        &mut app,
        Correction::always(
            p0,
            Scope::Domain(ChargeDomain::Acquire),
            Adjustment::Cancel,
            Source::God,
        ),
    );
    grab(&mut app, p0, 3);

    assert_eq!(plot_state(&mut app, 3).0, Some(0), "地照样到手");
    assert_eq!(cash_of(&mut app, p0), 20_000, "这笔被整笔免掉了");
    assert!(
        ledger(&app).contains(&"Bought(P0@3$0)".to_string()),
        "免费也要留一笔 0 的成交，否则「谁买的」这条链就断了：{:?}",
        ledger(&app)
    );
}

#[test]
fn a_plot_that_already_has_an_owner_is_not_a_candidate() {
    // 与涨价卡互为反面：那块卡只认有主的地，这张只认无主的。
    // 拒得很安静（三道门都静默），而卡**留在手里** —— 花了一张卡却什么都没发生
    // 是这套规则里最难查的那种投诉，所以宁可连按下去的机会都不给
    let (mut app, p0, p1) = two_players_before_the_roll();
    own(&mut app, 3, Some(1), 1, false);
    grab(&mut app, p0, 3);

    assert_eq!(plot_state(&mut app, 3).0, Some(1), "别人买下的地不该被抢走");
    assert_eq!(cash_of(&mut app, p0), 20_000, "一分都不该扣");
    assert_eq!(hand_of(&app, p0).len(), 1, "办不成的那次不该把卡打掉");
    assert!(!ledger(&app).iter().any(|l| l.starts_with("Bought")));
    let _ = p1;
}

#[test]
fn a_plot_beyond_the_wallet_is_not_a_candidate_either() {
    // 买不起的地连按钮都不该亮。这条同时钉住另一件事：卡**不是**一条赊账的入口 ——
    // `DeductCash` 付不起会当场破产，而买地付不起就该只是买不成
    let (mut app, p0, _p1) = two_players_before_the_roll();
    set_cash(&mut app, p0, L3_PRICE - 1);
    grab(&mut app, p0, 3);

    assert_eq!(plot_state(&mut app, 3).0, None, "钱不够就不该换手");
    assert_eq!(cash_of(&mut app, p0), L3_PRICE - 1, "现金分文不动");
    assert_eq!(hand_of(&app, p0).len(), 1, "卡留着，等下一笔钱来了再打");
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "买不成不等于破产离场");
}

#[test]
fn a_land_that_is_not_on_the_board_cannot_be_bought() {
    // 面板列不出这块地（候选排是从地块表数出来的），但请求消息是公开入口：
    // 一个越界的 `LandId` 该被同一句判据挡掉，而不是在账本里悄悄变成一次空转账
    let (mut app, p0, _p1) = two_players_before_the_roll();
    grab(&mut app, p0, 99);

    assert_eq!(cash_of(&mut app, p0), 20_000);
    assert_eq!(hand_of(&app, p0).len(), 1, "棋盘上没有这块地，卡就还是他的");
}

#[test]
fn somebody_elses_grab_card_does_not_move_the_land() {
    // 窗口那一半（`PlayWindow::opener`）与别的卡同一条：只在掷骰前、只认当前回合玩家。
    // 这里 p1 手里有卡、钱够（开局人人 20,000）、L3 也确实无主 —— 唯一拦住他的就是
    // 「现在不是他的回合」。少给任何一样，这条就在测别的东西（P3d② 那次 red-check 的教训）
    let (mut app, p0, p1) = two_players_before_the_roll();
    let card = item_index(&app, "item_grab");
    deal(&mut app, p1, card.0);
    play_card(&mut app, p1, card.0, Some(CardAim::Land(LandId(3))));
    settle(&mut app, 2);

    assert_eq!(
        plot_state(&mut app, 3).0,
        None,
        "别人的回合替他决定买不买是越权"
    );
    assert_eq!(cash_of(&mut app, p1), 20_000, "乙买成了的话现金会少一截");
    assert_eq!(hand_of(&app, p1).len(), 1, "卡还在他手里");
    let _ = p0;
}

#[test]
fn two_grab_cards_in_one_frame_take_two_different_plots() {
    // 同帧两条请求是 `MessageReader` 的真实形状（面板一次只发一条，测试与将来的 AI 会）。
    // 这里要的是「第二块地读得到第一块已经换手」—— 归属是就地写的组件，
    // 若走 `Commands` 的延迟写，两块地会被同一个人按同一个价买两次（P3d③ 同一条坑）
    let (mut app, p0, _p1) = two_players_before_the_roll();
    let card = item_index(&app, "item_grab");
    deal(&mut app, p0, card.0);
    deal(&mut app, p0, card.0);
    play_card(&mut app, p0, card.0, Some(CardAim::Land(LandId(0))));
    play_card(&mut app, p0, card.0, Some(CardAim::Land(LandId(3))));
    settle(&mut app, 2);

    assert_eq!(plot_state(&mut app, 0).0, Some(0), "第一块该到手");
    assert_eq!(plot_state(&mut app, 3).0, Some(0), "第二块也该到手");
    assert_eq!(
        cash_of(&mut app, p0),
        20_000 - 1_500 - L3_PRICE,
        "两块地的价各扣一次，不多不少"
    );
    assert!(hand_of(&app, p0).is_empty(), "两张卡都打掉了");
    assert_cash_audit(&mut app, 20_000);
}
