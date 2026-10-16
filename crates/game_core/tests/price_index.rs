//! P2 的验收：同一条 5000 的租金在四种状态下各收多少，以及物价指数本身的口径。
//!
//! 四条状态里「免费卡」与「大穷神」都还没出生（P3 的卡、P6 的神明），这里用
//! **测试侧往登记处塞一条修正** 的方式表达它们 —— 这正是 P2 这条链的验收点：
//! 加一种修正不必碰 `charge.rs`，也不必给管线开洞。等 P3/P6 真做出来，
//! 它们写的就是同样的几行，只不过发起方换成卡牌效果与附身组件。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    Adjustment, ChargeKind, Correction, GameSession, LandId, Scope, Source, TurnPhase,
};

fn me(app: &mut App) -> Entity {
    roster(app)[0].0
}
fn other(app: &mut App) -> Entity {
    roster(app)[1].0
}
fn index_of(app: &App) -> f32 {
    app.world().resource::<GameSession>().price_index
}
fn plot_owner(app: &mut App, land: u32) -> Option<u32> {
    holdings(app)
        .into_iter()
        .find(|(id, _, _)| *id == land)
        .map(|(_, owner, _)| owner)
        .expect("prototype 里有这块地")
}

/// 走完 0 号玩家脚下这一格。
fn land(app: &mut App) {
    play_turns(app, 1);
}

/// 造一条「P0 踩到 P1 的 L0，租金名义 5000」的对局，`tweak` 在落格之前改状态。
fn five_thousand(tweak: impl FnOnce(&mut App, Entity)) -> (i64, i64) {
    let (mut app, payer) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let owner = other(&mut app);
    own(&mut app, 0, Some(1), 1, false);
    flat_rent(&mut app, 0, 5_000);
    tweak(&mut app, payer);
    land(&mut app);
    (cash_of(&mut app, payer), cash_of(&mut app, owner))
}

/// PLAN §四 P2 的那句验收：无修正／有免费卡／有大穷神加倍／物价 2.5。
/// 四个数一次说完，因为它们必须来自**同一条链的同一处**，分开写就看不出串了没有。
#[test]
fn the_same_five_thousand_rent_in_four_states() {
    let (payer, owner) = five_thousand(|_, _| {});
    assert_eq!(
        (payer, owner),
        (15_000, 25_000),
        "无修正：5000 原样从付款人转到业主手上"
    );

    let (payer, owner) = five_thousand(cancel_for);
    assert_eq!(
        (payer, owner),
        (20_000, 20_000),
        "免费卡：整笔抵消，两边都不动 —— 抵消不是「付了 0 元再退回来」"
    );

    let (payer, owner) =
        five_thousand(|app, who| scale_for(app, who, 2.0, ChargeKind::Rent { land: LandId(0) }));
    assert_eq!(
        (payer, owner),
        (10_000, 30_000),
        "大穷神加倍：10000，而且加倍的钱真的到了业主手上（不是蒸发掉）"
    );

    let (payer, owner) = five_thousand(|app, _| set_price_index(app, 2.5));
    assert_eq!(
        (payer, owner),
        (7_500, 32_500),
        "物价 2.5：12500。缩放的是「付多少收多少」这一整笔，业主拿全额"
    );
}

/// 转嫁改的是**付款人**，收据要说实际付钱的那个人。
/// 走真落格路径（不是 `charge::stage` 的单测），所以它同时钉住接线。
///
/// 两人局里能接盘的不是业主本人没别人，于是这笔账变成「P1 付给 P1」—— 看着像脱裤子放屁，
/// 但它证的正是「付款人这个字段真的被换掉了」，而四人局里接盘的是第三方，同一条代码。
#[test]
fn a_shifted_bill_names_the_real_payer() {
    let (mut app, payer) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let owner = other(&mut app);
    own(&mut app, 0, Some(1), 1, false);
    flat_rent(&mut app, 0, 5_000);
    register(
        &mut app,
        Correction {
            bearer: payer,
            scope: Scope::Any,
            adjustment: Adjustment::TransferTo { to: owner },
            source: Source::Card,
        },
    );
    land(&mut app);

    assert_eq!(cash_of(&mut app, payer), 20_000, "被转嫁的人一分不付");
    assert_eq!(
        cash_of(&mut app, owner),
        20_000,
        "接盘的业主付了 5000 又收 of 5000，净零"
    );
    assert!(
        ledger(&app).contains(&"Rent(P1->P1@L0$5000)".to_string()),
        "流水要说实际付款人：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 指数的口径：**只因为系统里的钱变多而涨**。
///
/// 这里刻意**不**拿「再跑六回合指数不该动」当断言 —— 那六回合里机会与命运卡会从系统里
/// 往外掏钱（`AddCash` 的付款人是 `None`），那是**合法**的通胀来源，拿它验中性只会测到
/// 骰子序列。买地中性钉在 `economy.rs::buying_at_the_scaled_price_is_value_neutral`，
/// 那条是纯函数，没有随机流掺进来。
#[test]
fn the_index_follows_the_money_supply() {
    let mut app = boot_default();
    assert!(run_until_world(&mut app));
    assert!(
        (index_of(&app) - 1.0).abs() < 1e-6,
        "开局只有初始现金，指数必须是 1.0：{}",
        index_of(&app)
    );

    // 只给一个人注资：全场 40000 → 60000，比值 1.5。两个人都注会得到 2.0，
    // 那正好盖掉「按人加总」这条路径
    let p0 = me(&mut app);
    set_cash(&mut app, p0, 40_000);
    play_turns(&mut app, 1);
    let pushed = index_of(&app);
    assert!(
        (pushed - 1.5).abs() < 0.02,
        "全场钱从 40000 变成 60000，指数该到 1.5，实得 {pushed}"
    );
}

/// PLAN §三 点名的豁免：物价不碰股价。
///
/// 同种子跑两局，一局每回合把指数钉在 3.0、一局不动，行情序列必须逐字相同。
/// 今天它当然成立（股价压根不过管线），这条是留给将来的：谁要是哪天顺手把
/// `stock_tick` 乘上指数，这里立刻红。
#[test]
fn stock_prices_ignore_the_index() {
    let mut calm = boot_default();
    let mut hot = boot_default();
    assert!(run_until_world(&mut calm));
    assert!(run_until_world(&mut hot));

    for _ in 0..4 {
        set_price_index(&mut hot, 3.0);
        play_turns(&mut calm, 1);
        play_turns(&mut hot, 1);
        assert_eq!(
            price_of(&mut calm, "stock_001"),
            price_of(&mut hot, "stock_001"),
            "物价 3.0 的那局股价动了 —— 股价必须在豁免表之外"
        );
    }
}

/// 买地报价也过管线：物价 2 倍时模态上那个数与扣掉的数必须是同一个 3000。
/// 报价与成交读的是同一个纯函数（`Books::staged` 只看 `base`），这条钉住它没被绕开。
#[test]
fn the_buy_offer_carries_the_staged_price() {
    let (mut app, payer) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    set_price_index(&mut app, 2.0);
    land(&mut app);

    assert_eq!(
        cash_of(&mut app, payer),
        17_000,
        "1500 的地价按物价 2.0 收 3000"
    );
    assert_eq!(plot_owner(&mut app, 0), Some(0), "钱付对了，地也该到手");
    assert!(
        ledger(&app).contains(&"Bought(P0@0$3000)".to_string()),
        "成交流水要说实付价：{:?}",
        ledger(&app)
    );
}

/// 重算只发生在回合末。接线错了（比如挂到 Update 每帧跑）不会崩，只会让指数
/// 在回合中途就跟着玩家的每次花钱乱跳 —— 那玩家永远读不到「今天的物价」。
#[test]
fn the_index_is_recomputed_at_the_end_of_a_turn() {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    assert_eq!(phase(&app), TurnPhase::WaitRoll);

    set_cash(&mut app, p0, 26_000);
    settle(&mut app, 2);
    assert!(
        (index_of(&app) - 1.0).abs() < 1e-6,
        "没有走完一个回合末就不该重算：{}",
        index_of(&app)
    );

    play_turns(&mut app, 1);
    assert!(
        index_of(&app) > 1.14,
        "过了一个回合末，注的那 6000 该进指数：{}",
        index_of(&app)
    );
}
