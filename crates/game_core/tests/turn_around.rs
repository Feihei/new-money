//! P3d④b 的验收：转向卡翻的是**玩家身上那一位**，不是棋盘。
//!
//! ADR-001 行为规则 3 说主环是单向环，而 `GLOSSARY-board.md` 给转向卡的定义是
//! 「仍沿同一环，只是反向」。这两句合起来是一条很硬的结构约束：**环只有一份**
//! （`BoardDef::tiles`），方向是 `Heading`，落格分派、归属、租金、点券冷却全都读
//! 主环下标，所以掉头之后每一格的规则都自动成立 —— 这一批用例有一半是在钉这件事：
//!
//! - 倒着走踩到的格，与正着走踩到的格，走的是同一条落格流水线（不是第二套规则）；
//! - 骰面还是骰面：`Roll(P0,3,5->2)` 里那个 3 不该变成 -3（方向不是点数的一部分）；
//! - 回绕在负方向上同样成立（越过起点回到环尾），而起点不发薪水所以这里没有套利；
//! - 翻两次 = 原样（这一位不是「层数」）；
//! - 它是「事」不是「钱」，所以免罪挡得掉它，且挡掉时不留半条流水。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{CardAim, Heading};

/// 这位此刻朝哪边走。**缺组件就 panic**：`Heading` 是必挂的那一格（`spawn_players`），
/// 读不到就是装配漏了，而这条读数本身也是这条不变式的检查点之一。
fn heading_of(app: &App, player: Entity) -> Heading {
    *app.world()
        .entity(player)
        .get::<Heading>()
        .expect("每个玩家都该挂着 Heading（spawn_players 漏挂了？）")
}

/// 装配到「P0 站在 `tile` 上、下一掷恰好 `steps` 点、掷骰之前」。
fn standing_at(tile: u32, steps: u32) -> (App, Entity, Entity) {
    let (mut app, p0) = ready_on(tile, steps, DecisionPolicy::DeclineAll);
    let p1 = roster(&mut app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家");
    (app, p0, p1)
}

/// 把转向卡发到 `player` 手里并对 `aim` 打出去。
fn turn_around(app: &mut App, player: Entity, aim: Entity) {
    let card = item_index(app, "item_turn");
    deal(app, player, card.0);
    play_card(app, player, card.0, Some(CardAim::Player(aim)));
    settle(app, 2);
}

#[test]
fn every_player_starts_out_going_forward() {
    // 这一条看着像废话，它守的是「方向是玩家身上的位」这条模型的地基：
    // 谁哪天把 `Heading` 改成 `Option` 挂载或开局随机，第一条红的是它
    let (app, p0, p1) = standing_at(0, 1);
    assert_eq!(heading_of(&app, p0), Heading::Forward);
    assert_eq!(heading_of(&app, p1), Heading::Forward);
}

#[test]
fn a_reversed_player_walks_the_same_ring_backwards_and_the_die_stays_positive() {
    let (mut app, p0, p1) = standing_at(5, 3);
    turn_around(&mut app, p0, p0);
    assert_eq!(
        heading_of(&app, p0),
        Heading::Backward,
        "对自己打的那一张该生效"
    );

    play_turns(&mut app, 1);
    assert_eq!(tile_of(&app, p0), 2, "从 5 号倒着走三步该落在 2 号");
    let log = events(&app);
    // 骰面是 3，不是 -3：方向是玩家的事，不是这次掷骰的结果。把它折进 `steps`
    // 会让「掷出几点」这条流水从此读不出骰子，而 P5 的载具还要按骰数收费
    assert!(
        log.contains(&"Roll(P0,3,5->2)".to_string()),
        "骰面要留在流水里：{log:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|l| l.starts_with("Step(P0,"))
            .cloned()
            .collect::<Vec<_>>(),
        vec!["Step(P0,4)", "Step(P0,3)", "Step(P0,2)"],
        "逐格推进也要按倒序发，表现层插值读的就是这几条"
    );
    assert_eq!(
        heading_of(&app, p1),
        Heading::Forward,
        "别人没被打到，方向不该跟着变"
    );
}

#[test]
fn walking_backwards_wraps_past_the_start_to_the_other_end_of_the_ring() {
    // 从 1 号倒着走五步：0（起点）→ 14（彩票中心）→ 13（游乐场）→ 12（百货公司）→ 11（西单）。
    // 回绕用的是同一个 `rem_euclid`，而「倒着越过起点」不发薪水（`domain_of(Start) = Nothing` 早就拍板过），
    // 所以这里没有「反复擦过起点捞一笔」的套利。
    // 五步而不是四步（环每加一格就要重数一次）：四步落在 12 百货公司，那会开出柜台窗口；
    // 三步落在 13 游乐场，开一局小游戏。两条窗口各属于 `tests/shop.rs`／`tests/minigame.rs`，
    // 不该混进这条只问「绕不绕得回去」的用例里 —— 终点选在一块无主地产上，报价被 `DeclineAll` 拒掉。
    let (mut app, p0, _p1) = standing_at(1, 5);
    turn_around(&mut app, p0, p0);
    // 清场：装配随机刷在路上的神这一趟正好挡在他要经过的格上，附身那一刻小财神从别人钱包里
    // 抽了一笔（`GodDeed`）。下面那句「一分钱都不发」断的是**移动**，不是背景事件 ——
    // 补刷在回合末，所以他走路这一路上不会再有神。
    clear_gods(&mut app);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 11, "倒着越过起点该绕到环尾，不是停在 0");
    assert_eq!(cash_of(&mut app, p0), 20_000, "擦过起点一分钱都不发");
}

#[test]
fn two_reversals_put_the_direction_back_because_it_is_a_bit_not_a_stack() {
    // 「转向卡叠两层」是这套模型里最容易写错的一种可能：如果 `Heading` 存的是层数，
    // 第二张就会变成「逆向 ×2」。翻两次是原样 —— 这也是卡面文案那句「再转一次就转回来」
    //
    // 起点选 4 号而不是 5 号：倒着三步是 1 号（地产格，报价被拒），正着三步是 7 号（同样是
    // 地产格）。换成 5 号的话正着三步踩的是 8 号机会格，那条用例就在测一张随机牌了
    let (mut app, p0, _p1) = standing_at(4, 3);
    turn_around(&mut app, p0, p0);
    turn_around(&mut app, p0, p0);
    assert_eq!(heading_of(&app, p0), Heading::Forward, "两张转向卡 = 没转");

    play_turns(&mut app, 1);
    assert_eq!(tile_of(&app, p0), 7, "该照旧往前走");
}

#[test]
fn a_reversal_aimed_at_an_opponent_turns_him_and_not_the_caster() {
    // `CardTarget::Anybody` 与 `Player` 那一格只差「能不能选自己」，所以这一条同时
    // 在钉另一件事：打给别人时，出牌人自己的方向**不该**被顺手翻掉
    let (mut app, p0, p1) = standing_at(5, 3);
    turn_around(&mut app, p0, p1);

    assert_eq!(heading_of(&app, p1), Heading::Backward, "被点的那个人掉头");
    assert_eq!(heading_of(&app, p0), Heading::Forward, "出手的人照旧往前走");
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_turn)".to_string()),
        "流水要说清这是对谁用的：{:?}",
        ledger(&app)
    );
}

#[test]
fn an_insulated_victim_keeps_his_heading_and_both_sides_still_keep_a_trail() {
    // 与陷害卡同一条反制形状：挡掉了就没有这回事（不留半个掉头流水），
    // 但两边各看得见一次 —— 打的人要知道这张卡没了，被挡的人要知道自己省了一张
    let (mut app, p0, p1) = standing_at(5, 3);
    let insulate = item_index(&app, "item_insulate");
    make_current(&mut app, p1);
    deal(&mut app, p1, insulate.0);
    play_card(&mut app, p1, insulate.0, None);
    settle(&mut app, 2);
    make_current(&mut app, p0);

    turn_around(&mut app, p0, p1);

    assert_eq!(
        heading_of(&app, p1),
        Heading::Forward,
        "抵消了就没有掉头这回事"
    );
    assert_eq!(shield_of(&app, p1), None, "用掉的那一层要跟着收走");
    assert!(
        ledger(&app).contains(&"Play(P0->P1:item_turn挡1)".to_string()),
        "被挡掉也要留痕：{:?}",
        ledger(&app)
    );
}

#[test]
fn the_direction_survives_the_turn_boundary_because_it_is_not_a_turn_state() {
    // 掉头是这个人接下来的走法，不是「本回合的一个效果」。谁哪天在 `begin_turn_system`
    // 里顺手清一遍玩家状态，这条会红 —— 而那正是「转向卡玩起来莫名其妙只生效一次」的样子
    let (mut app, p0, _p1) = standing_at(5, 3);
    turn_around(&mut app, p0, p0);
    play_turns(&mut app, 1);
    assert_eq!(tile_of(&app, p0), 2, "第一回合就该倒着走");
    assert_eq!(
        heading_of(&app, p0),
        Heading::Backward,
        "回合结束不该把方向清回正向"
    );

    // 再跑两个回合（乙一回、甲一回）：那一步的骰面不再由种子定，所以这里只问方向还在不在，
    // 不问落点 —— 断言一个猜出来的格号，只会让这条用例在换种子时莫名其妙地红
    play_turns(&mut app, 3);
    assert_eq!(
        heading_of(&app, p0),
        Heading::Backward,
        "跨过两个回合之后仍然是逆向"
    );
}

#[test]
fn a_reversed_player_still_pays_rent_wherever_he_happens_to_land() {
    // 这一条是整批用例的重点：**落格规则不认识方向**。倒着踩进别人的地，与正着踩进去
    // 是同一条流水线（`classify_landing_system` 只看 `TokenPosition`），
    // 所以「转向」不需要在任何一处结算里加一个分支
    //
    // 5 号倒着走三步落在 **2 号格**，而那块地是 `L1`（汉口路）—— 格号与地块号不是一回事
    let (mut app, p0, p1) = standing_at(5, 3);
    own(&mut app, 1, Some(1), 1, false);
    flat_rent(&mut app, 1, 500);
    turn_around(&mut app, p0, p0);
    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 2, "倒着走的终点就是 2 号那块地");
    assert_eq!(cash_of(&mut app, p0), 19_500, "该付的租一分不少");
    assert_eq!(
        cash_of(&mut app, p1),
        20_500,
        "收租的人也不关心他从哪个方向来"
    );
    assert!(
        ledger(&app).contains(&"Rent(P0->P1@L1$500)".to_string()),
        "租金流水照旧：{:?}",
        ledger(&app)
    );
}
