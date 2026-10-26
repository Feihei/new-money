//! 百货公司（P3d②）的 App 级用例：真踩那一格、真按柜台上的买/卖按钮。
//!
//! 这一层要守的三件事，都是「画面与流水」而不是「算术」：
//! - **窗口**：只有站在柜台前的那个人办得成（别人的点击、按了「离开」之后的点击都不算）。
//! - **两套账**：买与卖只动点券，一次 `Wallet` 都不碰（PLAN §一「永不通兑」）。
//! - **上限挤档真的有人生产**：`CardGained.pushed_out` 从 P3a 起只是类型上的一个字段，
//!   它的第一个真用户就是这里的买入 —— 所以那条断言写在这个文件里，而不是 `hand.rs` 的单测里。
//!
//! prototype 的格子对照：**16 百货公司**（15 西单 → 掷 1 点即踩到）。
//! 换货是抽签（`shop::restock`），所以每个用例在进柜台之后用 `set_shelf` 把货架压平 ——
//! 断言「买这一张花 30」与「抽到哪三张」无关。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::Messages;
use common::*;
use game_core::prelude::{
    CardId, Decision, DecisionChosen, GameRules, GameSetup, Pending, TradeSide, TurnPhase,
};

/// prototype 上那一格百货。`board.rs::prototype_carries_a_shop_the_player_can_step_on`
/// 钉着这个编号，两处一起改才不会悄悄错位。（P10d① 两家公司插进环中段后它从 12 号顺移到
/// 14 号，P10d② 重排演示盘后落到 16 号，P11 的新闻中心插在 13 号、它再顺移到 17 号，
/// P13 的魔法屋插在 9 号、它跟着顺移到 18 号。）
const SHOP: u32 = 18;

/// 走到「柜台开着」的那一帧：p0 站在百货前一格（`SHOP - 1`）掷 1 点踩进去。
/// 用 `Hold` 策略 —— 自动应答会把窗口直接关掉，就什么都观察不到了（同 `finance.rs` 的柜台）。
fn open_counter() -> (App, Entity) {
    let (mut app, p0) = ready_on(SHOP - 1, 1, DecisionPolicy::Hold);
    roll(&mut app, p0);
    settle(&mut app, 8);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Shop)),
        "前置没做到：踩到百货格却没开出柜台"
    );
    (app, p0)
}

/// 柜台开着、点券调到 `points`、货架压成 `cards` 的前置。
/// 卡按 **id** 给（不是下标）：改 `item_cards.json` 的顺序不该让一批用例集体变红。
fn stocked(points: u32, cards: &[&str]) -> (App, Entity, Vec<CardId>) {
    let (mut app, p0) = open_counter();
    let ids: Vec<CardId> = cards.iter().map(|id| item_index(&app, id)).collect();
    set_points(&mut app, p0, points);
    set_shelf(&mut app, &ids.iter().map(|c| c.0).collect::<Vec<_>>());
    (app, p0, ids)
}

/// 同 [`stocked`]，但种子自选 —— 需要避开默认种子上神明的剧本时用
/// （P10d① 环长变了，默认种子的刷位跟着挪）。
fn stocked_seeded(
    points: u32,
    cards: &[&str],
    seed: game_core::prelude::GameSeed,
) -> (App, Entity, Vec<CardId>) {
    let (mut app, p0) = ready_on_seeded(SHOP - 1, DecisionPolicy::Hold, seed);
    roll(&mut app, p0);
    settle(&mut app, 8);
    let ids: Vec<CardId> = cards.iter().map(|id| item_index(&app, id)).collect();
    set_points(&mut app, p0, points);
    set_shelf(&mut app, &ids.iter().map(|c| c.0).collect::<Vec<_>>());
    (app, p0, ids)
}

fn shop_lines(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|line| line.starts_with("Shop("))
        .collect()
}

// ===== 踩上去开窗口 =====

#[test]
fn stepping_into_the_shop_opens_a_counter_with_a_shelf() {
    let (mut app, _) = open_counter();
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "回合该停在柜台前等玩家按「离开」"
    );

    let shelf = shelf_of(&app);
    assert_eq!(
        shelf.len(),
        app.world().resource::<GameRules>().shop_slots as usize,
        "货位数该等于 `GameRules::shop_slots`"
    );
    let mut uniq = shelf.clone();
    uniq.sort();
    uniq.dedup();
    assert_eq!(uniq.len(), shelf.len(), "抽签是不放回的: {shelf:?}");

    // 一格百货不发一分钱：它与现金那条链毫无交集，两套账在这里的形状就是「现金审计仍然对得上」
    assert_cash_audit(&mut app, 20_000);
    assert!(shop_lines(&app).is_empty(), "还没按按钮就该没有成交凭据");
}

// ===== 买入 =====

#[test]
fn buying_costs_points_and_takes_that_slot_off_the_shelf() {
    let (mut app, p0, ids) = stocked(50, &["item_frame"]);
    let frame = ids[0];

    shop_ask(&mut app, p0, TradeSide::Buy, frame.0);
    settle(&mut app, 3);

    assert_eq!(points_of(&app, p0), 20, "30 点券的卡该从 50 买到只剩 20");
    assert_eq!(hand_of(&app, p0), vec![frame], "卡该进手牌");
    assert!(
        shelf_of(&app).is_empty(),
        "货位买走就空了 —— 一次进店囤满同款的路不该存在"
    );
    assert_eq!(
        shop_lines(&app),
        vec!["Shop(P0买item_frame·30点券)".to_string()],
        "成交凭据要说动了几点券"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("Gain(P0:item_frame)")),
        "卡进手牌那半要另有一条凭据"
    );

    // 同一张再买一次：货架已经空了（`NotStocked`），三样读数都不许动
    shop_ask(&mut app, p0, TradeSide::Buy, frame.0);
    settle(&mut app, 3);
    assert_eq!(points_of(&app, p0), 20, "被拒的第二次还扣了钱");
    assert_eq!(hand_of(&app, p0), vec![frame], "被拒的第二次还发了货");
    assert_eq!(
        shop_lines(&app).len(),
        1,
        "被拒的那次也写了成交凭据: {:?}",
        shop_lines(&app)
    );
}

#[test]
fn a_card_too_expensive_is_refused_and_leaves_no_trace() {
    let (mut app, p0, ids) = stocked(10, &["item_free"]);
    let free = ids[0];

    shop_ask(&mut app, p0, TradeSide::Buy, free.0);
    settle(&mut app, 3);

    assert_eq!(points_of(&app, p0), 10, "点券不够却扣了钱");
    assert!(hand_of(&app, p0).is_empty(), "买不起还发货");
    assert_eq!(shelf_of(&app), vec![free], "买不成也不该把货下架");
    assert!(shop_lines(&app).is_empty(), "被拒的交易写了成交凭据");
}

/// 上限挤档（PLAN §三「满了替换最前面那张」）第一次有了真生产者：P3a 只有 `Hand::gain`
/// 的单测，而 `CardGained.pushed_out` 那天起没有任何 system 写过它。
#[test]
fn a_full_hand_pushes_out_the_oldest_card_and_says_which_one() {
    // `hand_limit: 1` 而不是真攒 15 张：要验的是「挤掉最前面」这条规则，不是卡牌数量
    let mut app = boot(
        GameSetup::default(),
        GameRules {
            hand_limit: 1,
            ..GameRules::instant()
        },
        seed_rolling_first(1),
    );
    set_policy(&mut app, DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    place(&mut app, p0, SHOP - 1);
    roll(&mut app, p0);
    settle(&mut app, 8);
    assert_eq!(pending_of(&app), Some((p0, Pending::Shop)));

    let frame_name = "item_frame";
    let insulate_name = "item_insulate";
    let frame = item_index(&app, frame_name);
    let insulate = item_index(&app, insulate_name);
    set_points(&mut app, p0, 100);
    set_shelf(&mut app, &[frame.0, insulate.0]);

    shop_ask(&mut app, p0, TradeSide::Buy, frame.0);
    settle(&mut app, 2);
    shop_ask(&mut app, p0, TradeSide::Buy, insulate.0);
    settle(&mut app, 3);

    assert_eq!(
        hand_of(&app, p0),
        vec![insulate],
        "上限 1 时第二张该把第一张挤出去"
    );
    // 「我明明买了卡，卡却不见了」是查不动的那类投诉，所以挤掉的那张必须点名
    assert!(
        ledger(&app).contains(&format!("Gain(P0:{insulate_name})挤掉{frame_name}")),
        "{:?}",
        ledger(&app)
    );
    // 两次都扣了钱：挤掉不退款（那是 4 代的口径，也是「赌下一张更有用」这个玩法的代价）
    assert_eq!(points_of(&app, p0), 100 - 30 - 20);
}

// ===== 回收 =====

#[test]
fn selling_a_card_back_pays_nine_tenths_and_removes_it() {
    let (mut app, p0) = open_counter();
    let insulate = item_index(&app, "item_insulate");
    set_shelf(&mut app, &[]);
    deal(&mut app, p0, insulate.0);

    shop_ask(&mut app, p0, TradeSide::Sell, insulate.0);
    settle(&mut app, 3);

    assert_eq!(points_of(&app, p0), 18, "20 点券的卡卖回该收 18（9 折）");
    assert!(hand_of(&app, p0).is_empty(), "点券到手而卡还在手里");
    assert_eq!(
        shop_lines(&app),
        vec!["Shop(P0卖item_insulate·18点券)".to_string()]
    );

    // 同一张再卖一次：手里没了（`NotHeld`），点券不许第二次进账
    shop_ask(&mut app, p0, TradeSide::Sell, insulate.0);
    settle(&mut app, 3);
    assert_eq!(points_of(&app, p0), 18, "被拒的第二次还付了钱");
    // 两套账：买与卖走完，现金一位没动
    assert_cash_audit(&mut app, 20_000);
    assert_points_audit(&mut app, 0);
}

/// 回收不问货架：那张卡本来就是从**某一次**的货架上买来的，而这一趟没摆它也要收。
/// 反过来（只在架上才收）会让「上次买的卡这次卖不掉」成为一句无法自证的话。
#[test]
fn the_shop_buys_a_card_it_is_not_currently_stocking() {
    let (mut app, p0) = open_counter();
    let hike = item_index(&app, "item_hike");
    set_shelf(&mut app, &[]);
    deal(&mut app, p0, hike.0);
    assert!(!shelf_of(&app).contains(&hike), "前置：这一趟不该摆这张");

    shop_ask(&mut app, p0, TradeSide::Sell, hike.0);
    settle(&mut app, 3);
    assert_eq!(points_of(&app, p0), 36, "40 点券的卡卖回该收 36");
}

// ===== 窗口：谁办得成、开到什么时候 =====

/// 柜台是关于「那个人」的：别人的点击不替他花钱，按了「离开」之后这一格也不再开门。
#[test]
fn somebody_else_and_the_period_after_leaving_are_both_refused() {
    let (mut app, p0, ids) = stocked(50, &["item_frame"]);
    let frame = ids[0];
    let p1 = roster(&mut app)[1].0;
    // 给乙备足点券：不给钱的话这一笔被拒的是「点券不够」而不是「他不在柜台前」，
    // 那条 `is_empty` 断言就成了白写的 —— 窗口判据一旦被人删掉，这条用例抓不到
    set_points(&mut app, p1, 500);

    shop_ask(&mut app, p1, TradeSide::Buy, frame.0);
    settle(&mut app, 3);
    assert!(hand_of(&app, p1).is_empty(), "别人的柜台还能替别人买东西？");
    assert!(hand_of(&app, p0).is_empty(), "乙的点击花掉了甲的点券？");
    assert_eq!(points_of(&app, p0), 50);
    assert_eq!(points_of(&app, p1), 500, "乙买成了（花的是他自己的点券）");

    // 「离开」= 答一句 Acknowledge（`decision_system` 对窗口只认这一种回答）。
    // 换成非 Hold 策略就是替玩家按那颗按钮。
    set_policy(&mut app, DecisionPolicy::AcceptAll);
    settle(&mut app, 6);
    assert_eq!(pending_of(&app), None, "答了「离开」柜台还挂着");

    shop_ask(&mut app, p0, TradeSide::Buy, frame.0);
    settle(&mut app, 3);
    assert_eq!(points_of(&app, p0), 50, "窗口关了还能买");
    assert!(hand_of(&app, p0).is_empty());
    assert_eq!(shelf_of(&app), vec![frame], "窗口关了还把货下架了");
    assert!(shop_lines(&app).is_empty(), "窗口关了还写成交凭据");
}

/// 每次进入货架重抽（PLAN §三）。断言的是「重抽」这条规则本身而不是「两次不一样」——
/// 池子只有 5 张、货位 3 个，两次撞上并不奇怪，那不该让这条用例红。
#[test]
fn the_next_visit_redraws_the_shelf_even_after_buying_out() {
    // 换第 3 颗种子：默认那颗的神明恰好刷在百货格上，回合结束小福神上身
    // 送一张转向卡 —— 卡进了手牌，这条「重抽的是货架不是手牌」的断言就被
    // 无关的神明剧本打红（P10d① 环长 15→17 改变了刷位）。
    let (mut app, p0, ids) = stocked_seeded(100, &["item_frame"], nth_seed_rolling_first(1, 3));
    let frame = ids[0];
    shop_ask(&mut app, p0, TradeSide::Buy, frame.0);
    settle(&mut app, 3);
    assert!(shelf_of(&app).is_empty(), "前置：这一趟已经空了");

    // 离开柜台（这一回合就此结束）→ 把骰子流倒回第一掷 → 换 p0 行动 → 再掷 1 点重新进店。
    // `rewind_dice` 是冷却类用例同一条手法：要的是「同一个人再次踩到同一格」，
    // 而不是真跑一整圈把利息、分红、行情全卷进来。
    set_policy(&mut app, DecisionPolicy::AcceptAll);
    settle(&mut app, 6);
    assert_eq!(pending_of(&app), None, "前置：「离开」没关掉柜台");
    // 换回 `Hold`：第二次进店的柜台要观察得到，否则自动应答立刻替玩家按了「离开」
    set_policy(&mut app, DecisionPolicy::Hold);
    rewind_dice(&mut app);
    make_current(&mut app, p0);
    place(&mut app, p0, SHOP - 1);
    roll(&mut app, p0);
    settle(&mut app, 8);

    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Shop)),
        "重新踩到百货格该再开一次柜台"
    );
    assert_eq!(
        shelf_of(&app).len(),
        app.world().resource::<GameRules>().shop_slots as usize,
        "第二次进店该重抽满货位（买空的那一格也要回来）"
    );
    // 手牌与点券都留着：重抽的是货架，不是这一局买过的东西
    assert_eq!(hand_of(&app, p0), vec![frame]);
    assert!(!shop_lines(&app).is_empty(), "第一次成交的凭据消失了");
}

/// 点券审计在百货这一侧同样成立：买与卖各有一笔 `PointsChanged`，而现金流水一条不多。
/// 这条是「永不通兑」的最后一道形状 —— 有人将来给买卖加现金找零，红的是这里。
#[test]
fn trading_in_points_never_touches_the_cash_ledger() {
    let (mut app, p0, ids) = stocked(50, &["item_frame"]);
    let frame = ids[0];
    shop_ask(&mut app, p0, TradeSide::Buy, frame.0);
    settle(&mut app, 2);
    shop_ask(&mut app, p0, TradeSide::Sell, frame.0);
    settle(&mut app, 3);

    assert_eq!(cash_of(&mut app, p0), 20_000, "点券账动了现金");
    assert_eq!(points_of(&app, p0), 50 - 30 + 27, "买 30 卖回 27");
    assert_cash_audit(&mut app, 20_000);
    assert_points_audit(&mut app, 0);
}

/// 柜台不是报价：`Decision::Accept` 对它不算回答（`decision_system` 里那一对臂）。
/// 这里直接写一条 `DecisionChosen` 而不是经 `DecisionPolicy` —— 策略对窗口一律答「已阅」
/// （那等于替玩家按「离开」），而这条要验的正是「答错了算不算数」。
#[test]
fn the_counter_only_closes_on_leaving() {
    let (mut app, p0) = open_counter();
    let before = shelf_of(&app);

    app.world_mut()
        .resource_mut::<Messages<DecisionChosen>>()
        .write(DecisionChosen {
            player: p0,
            action: Decision::Accept,
        });
    settle(&mut app, 3);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Shop)),
        "一句「接受」就把百货柜台关了 —— 面板上根本没有这颗按钮可对上"
    );
    assert_eq!(shelf_of(&app), before, "一句回答把货架动了");

    app.world_mut()
        .resource_mut::<Messages<DecisionChosen>>()
        .write(DecisionChosen {
            player: p0,
            action: Decision::Acknowledge,
        });
    settle(&mut app, 3);
    assert_eq!(pending_of(&app), None, "按了「离开」柜台还挂着");
}
