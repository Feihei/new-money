//! 建筑损害阶梯的 App 级测试（P2②）。纯算术与面积解析在 `damage.rs` 的单测里，
//! 这里跑的是**真棋盘**上那一遍：prototype 的六个地块、`Books::damage` 这条唯一入口、
//! 以及「拆完之后租金怎么变」这条只有接线才会露出来的后果。
//!
//! prototype 的地块对照（`assets/data/boards/prototype.json`）：
//! 0 南京路(col2 row0, brown) / 1 汉口路(col3 row0, brown) / 2 外滩商业区(col6 row2, **2×2**, orange) /
//! 3 徐家汇(col4 row5, red) / 4 王府井(col2 row5, yellow) / 5 西单(col0 row2, yellow)。
//! 挑这几块做用例正合适：0↔1 相邻（`Box(3)` 打得着、`Street` 也打得着，但理由不同）、
//! 4↔5 同路段却隔半张棋盘（只有 `Street` 打得着）、2 是唯一那块 2×2。

mod common;

use bevy::app::App;
use common::*;
use game_core::prelude::{DamageTally, LandDamage, LandId, PlotArea};

fn board_ready() -> App {
    let mut app = boot_default();
    assert!(run_until_world(&mut app), "棋盘资产没能在时限内装配成世界");
    app
}

#[test]
fn a_monster_card_levels_the_building_but_keeps_the_deed() {
    let mut app = board_ready();
    own(&mut app, 0, Some(1), 3, false);

    let tally = run_damage(
        &mut app,
        LandId(0),
        LandDamage::Clear { seize: false },
        PlotArea::One,
    );
    assert_eq!(tally, DamageTally { hit: 1, seized: 0 });
    assert_eq!(
        plot_state(&mut app, 0),
        (Some(1), 0, false),
        "怪兽卡那一档：房子归零、地还是他的 —— 夺地是另一档，不该顺手做"
    );
}

#[test]
fn forced_seizure_takes_the_deed_and_the_mortgage_with_it() {
    let mut app = board_ready();
    own(&mut app, 0, Some(1), 4, true);

    let tally = run_damage(
        &mut app,
        LandId(0),
        LandDamage::Clear { seize: true },
        PlotArea::One,
    );
    assert_eq!((tally.hit, tally.seized), (1, 1));
    assert_eq!(
        plot_state(&mut app, 0),
        (None, 0, false),
        "强制征收：房屋同灭、土地收回，抵押也必须跟着走 —— \
         无主地上留着 `Mortgaged` 就等于替一个不存在的主人挡租金"
    );
}

#[test]
fn tearing_down_a_building_does_not_write_off_the_debt() {
    let mut app = board_ready();
    own(&mut app, 0, Some(1), 3, true);

    run_damage(
        &mut app,
        LandId(0),
        LandDamage::Degrade { levels: 1 },
        PlotArea::One,
    );
    assert_eq!(
        plot_state(&mut app, 0),
        (Some(1), 2, true),
        "降一级：房子少一层，地与抵押照旧 —— 拆房不是免债，那是 P9 变卖池才有的账"
    );
}

#[test]
fn a_missile_dents_the_neighbourhood_and_nothing_else() {
    let mut app = board_ready();
    for land in [0, 1, 3, 4, 5] {
        own(&mut app, land, Some(0), 2, false);
    }
    own(&mut app, 2, Some(1), 2, false);

    let tally = run_damage(
        &mut app,
        LandId(0),
        LandDamage::Degrade { levels: 1 },
        PlotArea::Box { side: 3 },
    );
    assert_eq!(tally.hit, 2, "0 的 3×3 只罩得住紧挨着的 1：{tally:?}");
    assert_eq!(plot_state(&mut app, 0), (Some(0), 1, false));
    assert_eq!(plot_state(&mut app, 1), (Some(0), 1, false));
    // 中心的这块**没有**被夺走：`Degrade` 那一支压根没有 `seize` 这个开关
    assert_eq!(
        plot_state(&mut app, 5),
        (Some(0), 2, false),
        "隔两格的西单不该被 3×3 扫到"
    );
    assert_eq!(
        plot_state(&mut app, 2),
        (Some(1), 2, false),
        "另一头的 2×2 外滩也不该 —— 它大，但它不在半径里"
    );
}

#[test]
fn street_follows_the_group_not_the_geometry() {
    let mut app = board_ready();
    own(&mut app, 4, Some(0), 5, false); // 王府井 yellow
    own(&mut app, 5, Some(1), 5, false); // 西单 同一路段，但隔着半张棋盘

    let tally = run_damage(
        &mut app,
        LandId(4),
        LandDamage::Clear { seize: false },
        PlotArea::Street,
    );
    assert_eq!(tally.hit, 2, "「整路段夷为平地」= 同路段两块：{tally:?}");
    assert_eq!(plot_state(&mut app, 4), (Some(0), 0, false));
    assert_eq!(
        plot_state(&mut app, 5),
        (Some(1), 0, false),
        "别人的那一块照样拆 —— 拆的是路段，不是「我的地产」"
    );

    // 对照：同一个中心的 3×3 只打得到自己，两者不是一回事
    own(&mut app, 4, Some(0), 5, false);
    own(&mut app, 5, Some(1), 5, false);
    let near = run_damage(
        &mut app,
        LandId(4),
        LandDamage::Clear { seize: false },
        PlotArea::Box { side: 3 },
    );
    assert_eq!(near.hit, 1, "4 与 5 不同段但也不相邻");
    assert_eq!(plot_state(&mut app, 5).1, 5, "3×3 不该碰孤零零的那一块");
}

#[test]
fn a_nuke_clears_the_whole_board_and_every_deed() {
    let mut app = board_ready();
    for land in 0..6 {
        own(&mut app, land, Some(0), 6, land == 2);
    }

    let tally = run_damage(
        &mut app,
        LandId(0),
        LandDamage::Clear { seize: true },
        PlotArea::Whole,
    );
    assert_eq!(tally, DamageTally { hit: 6, seized: 6 });
    for land in 0..6 {
        assert_eq!(
            plot_state(&mut app, land),
            (None, 0, false),
            "全图夺地之后第 {land} 块还带着主人或抵押"
        );
    }
}

#[test]
fn an_empty_lot_still_counts_as_hit() {
    let mut app = board_ready();
    // 无主空地也被算进命中：范围就是范围，不为「这一拳没打到东西」而缩小。
    // 将来的新闻文案要说「拆了 9 块」，那个 9 得与玩家数到的格子数一致
    let tally = run_damage(
        &mut app,
        LandId(3),
        LandDamage::Clear { seize: false },
        PlotArea::One,
    );
    assert_eq!(tally, DamageTally { hit: 1, seized: 0 });
}

#[test]
fn a_land_that_is_not_on_the_board_damages_nothing() {
    let mut app = board_ready();
    own(&mut app, 0, Some(1), 3, false);
    let tally = run_damage(
        &mut app,
        LandId(99),
        LandDamage::Clear { seize: true },
        PlotArea::Whole,
    );
    assert_eq!(tally, DamageTally::default(), "中心地块不存在就该一拳打空");
    assert_eq!(plot_state(&mut app, 0), (Some(1), 3, false));
}

/// 拆完之后租金怎么变，是这套动词唯一「钱真的动」的后果 —— 而它只能从接线里长出来：
/// 建筑归零不等于这块地不收费，它退到 `rent[0]` 那一档（空地租金）。
#[test]
fn a_levelled_lot_still_charges_the_empty_ground_rent() {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll); // 1 号格 = 地块 0 南京路
    own(&mut app, 0, Some(1), 3, false);
    run_damage(
        &mut app,
        LandId(0),
        LandDamage::Clear { seize: false },
        PlotArea::One,
    );

    play_turns(&mut app, 1);
    let led = ledger(&app);
    assert!(
        led.contains(&"Rent(P0->P1@L0$60)".to_string()),
        "拆平后该收空地租金 60：{led:?}"
    );
    assert_eq!(cash_of(&mut app, p0), 19_940, "租金仍从现金走");
}

/// 拆过的地**不该**再触发垄断翻倍：`economy::rent_of` 读的是「同组全在有主且未抵押」，
/// 而夺地那一档把 owner 收走了 —— 这条把损害与租金两条线接在一起。
#[test]
fn a_seized_lot_opens_no_monopoly_for_whoever_owns_the_rest() {
    let (mut app, _) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 0, Some(1), 0, false);
    own(&mut app, 1, Some(1), 0, false); // brown 两块空地都在 P1 手里 → 本该翻倍
    run_damage(
        &mut app,
        LandId(1),
        LandDamage::Clear { seize: true },
        PlotArea::One,
    );

    play_turns(&mut app, 1);
    let led = ledger(&app);
    assert!(
        led.contains(&"Rent(P0->P1@L0$60)".to_string()),
        "另一块被征收后该回到单块租金 60：{led:?}"
    );
    assert!(
        !led.iter().any(|e| e.contains("$120")),
        "brown 组已经不完整，120 那档不该再出现：{led:?}"
    );
    assert_eq!(plot_state(&mut app, 1).0, None, "被征收的那块此刻无主");
}
