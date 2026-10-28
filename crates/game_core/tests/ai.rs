//! P14a 的验收：**AI 是一个会自己发意图的座位**，而不是第二套规则。
//!
//! 这一刀没有打分（那是 P14b/c/d），所以这里钉的全是「座位」这件事的形状：
//!
//! - 每个配好的座位真的带上了 `Controller`，默认局是**一位真人 + 三档性格** →
//!   [`each_seat_carries_the_controller_it_was_configured_with`]、
//!   [`the_field_is_one_human_and_three_personalities`]；
//! - **没有人按按钮，局自己走**（掷骰、停留、答窗口三条出口都有人发） →
//!   [`the_ai_field_moves_with_nobody_at_the_keyboard`]、
//!   [`a_held_ai_says_stay_because_rolling_is_refused`]、
//!   [`an_ai_seat_answers_a_window_it_was_hung_without_a_click`]；
//! - 验收原话那几件具体的事：**在拍卖里出价** → [`an_ai_seat_bids_its_way_through_an_auction`]；
//! - **同一个窗口只收到一份回答**：core 的 AI 答电脑座位，那份脚手架答人类座位，
//!   分界是 `Controller` 这一个组件。它的牙长在
//!   [`an_ai_seat_answers_a_window_it_was_hung_without_a_click`] 上（夹具默认 `DeclineAll`，
//!   一旦它也开始替电脑座位答话，`Decline` 会盖掉 AI 的 `Accept` → 那条当场红，实测过）。
//!   拍卖那一条**不担这个责任**：它同时发生时价格照样只前进一档，分不出来；
//! - AI 不动别人的随机数（第 20 条子流只在该抽的时候抽） →
//!   [`the_ai_leaves_the_dice_and_the_deck_alone`]；
//! - 延迟与暂停这两道门真的挡得住 → [`a_held_clock_freezes_the_ai`]、
//!   [`a_paused_game_freezes_the_ai_too`]。
//!
//! 「延迟不改答案」这一条在这里只能钉住**形状**（同一颗种子两次跑出同一条轨迹），
//! 墙钟上的真实间隔在无头测试里量不到 —— `Time::elapsed_secs()` 不前进，所以 core 的默认
//! 必须是 0（`ai.rs` 里那条单测钉的就是它），而这里用「拧死的钟」当闸门而不是当秒表。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::{MessageReader, NextState, ResMut, Resource, Update};
use common::*;
use game_core::prelude::{
    ActiveBoard, AiCadence, AiClock, AiProfile, CardAim, Controller, Detention, GameRules,
    GameSeed, GameSession, GameSetup, Gender, Institution, LandId, LandStatus, PLAYER_COLORS,
    Paused, Pending, PendingDecision, PlayerSetup, PriceHistory, StaySentence, StockId, StockPrice,
    StockTradeRequested, Timed, TurnPhase,
};
use game_core::rng::Rng;

/// 徐家汇（地块 3）：与 `tests/auction.rs` 同一块标的，Lv2 起拍 7600、一档 760。
const LAND: u32 = 3;

/// 一位真人 + `ai` 位电脑（P14 给玩家看到的那个阵容），**AI 冻在原地**。
fn field(ai: usize) -> (App, Vec<Entity>) {
    field_with(GameSetup::seats(ai))
}

/// `field` 的那一层壳：摆任意阵容，装配完就把 AI 冻住。
///
/// 冻是两步而不是拧一个数：`begin_game_system` 装配时会把 `AiClock` 归零，所以在装配**之前**
/// 把间隔调大挡不住那第一次动作（`ready_at = 0` 意味着「现在就能动手」）。装配之后
/// 先 [`freeze`] 才算冻住 —— 这一条本来就是 [`thaw`] 那条用例翻出来的。
fn field_with(setup: GameSetup) -> (App, Vec<Entity>) {
    let mut app = boot(setup, GameRules::instant(), GameSeed::default());
    assert!(run_until_world(&mut app), "棋盘资产没能在时限内装配成世界");
    freeze(&mut app);
    let first = boot_to_wait_roll(&mut app);
    clear_gods(&mut app);
    let rows = roster(&mut app);
    assert_eq!(rows[0].0, first);
    let players = rows.iter().map(|(e, ..)| *e).collect();
    (app, players)
}

/// 三档先验都在，但**失误率统一拧成 0**。
///
/// 测打分时要按住噪声，否则红下来分不清是式子变了还是骰子变了 —— 而这两件事各自有用例
/// （[`a_fumble_trades_the_best_answer_for_the_next_one`] 测噪声本身）。
fn quiet_field(ai: usize) -> (App, Vec<Entity>) {
    let mut setup = GameSetup::seats(ai);
    for p in &mut setup.players {
        if let Controller::Ai(profile) = &mut p.controller {
            profile.fumble = 0.0;
        }
    }
    field_with(setup)
}

/// 把窗口挂在某位座位上、放松钟让他答一次、答完再冻回去。
///
/// 四帧而不是两帧：`hang` 写的 `NextState` 要到下一帧的 `StateTransition` 才落地，而 AI 那条
/// system 挂在 `PreUpdate`（排在它之前），所以第一帧读到的还是旧相位（§6.1 新增坑 39）。
fn ask(app: &mut App, seat: Entity, about: Pending) {
    hang(app, seat, about);
    thaw(app);
    settle(app, 4);
    freeze(app);
}

/// 全员皆机的阵容：`n` 位座位各领一档性格（没有人类座位，所以局只能靠 AI 往前走）。
fn all_ai(n: usize) -> GameSetup {
    GameSetup {
        players: (0..n)
            .map(|i| PlayerSetup {
                name: format!("机{i}"),
                color: PLAYER_COLORS[i % PLAYER_COLORS.len()],
                gender: Gender::Male,
                controller: Controller::Ai(AiProfile::THREE[i % AiProfile::THREE.len()]),
            })
            .collect(),
    }
}

/// 冻住 AI：把下一次可以动手的时刻推到永远之后。
fn freeze(app: &mut App) {
    app.insert_resource(AiClock {
        ready_at: f32::INFINITY,
    });
}

/// 放松 AI 的钟（core 的默认：即时）。
fn thaw(app: &mut App) {
    app.insert_resource(AiCadence::default());
    app.insert_resource(AiClock::default());
}

fn controller_of(app: &App, player: Entity) -> Controller {
    *app.world()
        .entity(player)
        .get::<Controller>()
        .expect("座位应有 `Controller`：它是必挂组件，`spawn_players` 是唯一的上身出口")
}

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

/// 现在被挂着的是谁、是什么。
fn window(app: &App) -> Option<(Entity, Pending)> {
    app.world().resource::<PendingDecision>().active()
}

/// 把回合机推到「等回答」并挂上窗口 —— 与 `tests/cards.rs`、`tests/villain.rs` 同一手：
/// 这一刀测的是**谁答**，不是**怎么挂上**，挂法由落格分派那些既有用例各自钉住。
fn hang(app: &mut App, player: Entity, about: Pending) {
    app.world_mut()
        .resource_mut::<PendingDecision>()
        .offer(player, about);
    app.world_mut()
        .resource_mut::<NextState<TurnPhase>>()
        .set(TurnPhase::WaitDecision);
}

#[test]
fn each_seat_carries_the_controller_it_was_configured_with() {
    // 配置里写了 AI 而世界里没有那一位 = `spawn_players` 漏挂组件，而漏挂的后果是
    // 「那个座位根本不动」（AI 的 system 读不到组件就当人类等按钮）—— 症状与
    // 「装配没跑」一模一样，所以这条要从**世界**里读而不是从 `GameSetup` 里读
    let (app, players) = field(3);
    assert_eq!(
        controller_of(&app, players[0]),
        Controller::Human,
        "第一位是坐在键盘前的那位"
    );
    for (i, who) in ["大佬奸", "普通人", "乖宝宝"].iter().enumerate() {
        let seat = players[i + 1];
        let Controller::Ai(profile) = controller_of(&app, seat) else {
            panic!("第 {} 位该是电脑开的座位", i + 1);
        };
        assert_eq!(profile.who, *who, "三档预设领错了名字");
    }
    // 而 `Default` 那两位**没有被顺手改成 AI**（几百条既有测试按「等人点」写）
    for p in GameSetup::default().players {
        assert_eq!(
            p.controller,
            Controller::Human,
            "{} 的座位默认必须等人点",
            p.name
        );
    }
}

#[test]
fn the_field_is_one_human_and_three_personalities() {
    let setup = GameSetup::ai_field();
    assert_eq!(setup.players.len(), 4, "玩家看到的一局是 1 人类 + 3 电脑");
    let ai: Vec<_> = setup
        .players
        .iter()
        .filter(|p| p.controller.is_ai())
        .collect();
    assert_eq!(ai.len(), 3);
    // 三档各一位：验收那句「进魔法屋按性格选」要有对照才拍得出来
    let mut names: Vec<&str> = ai
        .iter()
        .map(|p| p.controller.profile().unwrap().who)
        .collect();
    names.sort_unstable();
    assert_eq!(names, ["乖宝宝", "大佬奸", "普通人"]);
    // 四个座位四种色：`PLAYER_COLORS` 正好四格，撞色会让 HUD 与棋子读不出谁是谁
    let mut colors: Vec<_> = setup.players.iter().map(|p| p.color).collect();
    let before = colors.len();
    colors.dedup();
    assert_eq!(colors.len(), before, "座位撞色了");
    // `NM_AI=0` 那条路（纯人类局）仍然存在 —— 现有截图与真鼠标冒烟走的就是它
    assert_eq!(GameSetup::seats(0).players.len(), 1);
    assert!(
        GameSetup::seats(0)
            .players
            .iter()
            .all(|p| !p.controller.is_ai())
    );
    // 上限之外不多给：没有第四档性格可领
    assert_eq!(GameSetup::seats(9).players.len(), 4);
}

#[test]
fn the_ai_field_moves_with_nobody_at_the_keyboard() {
    // 这一条就是验收那句「可完局」的最小形式：**一次按钮都没按**，回合机自己往前走
    let mut app = boot(all_ai(3), GameRules::instant(), GameSeed::default());
    assert!(run_until_world(&mut app), "棋盘资产没能在时限内装配成世界");
    let start = turn_count(&app);
    settle(&mut app, 900);
    let moved = turn_count(&app) - start;
    assert!(
        moved >= 5,
        "三位电脑 {} 帧里只走了 {moved} 个回合 —— 有人不发意图，回合机停在某处不动了",
        900
    );
    // 而它走的是一局**正常的**棋：钱不出错、不凭空多也不凭空少
    assert_cash_audit(&mut app, GameRules::default().start_cash);
}

#[test]
fn a_held_ai_says_stay_because_rolling_is_refused() {
    // 被停留按住的人**只发「停留」**。少这一支是死锁而不是少走一步：
    // `roll_dice_system` 在规则层拒掉被按住者的掷骰，于是 `RequestRoll` 每帧被写、每帧被吞，
    // 回合机停在 `WaitRoll` 不动也不报错（P4④ 当年把这条写成「P14 的前置」等的就是它）
    let mut app = boot(all_ai(1), GameRules::instant(), GameSeed::default());
    assert!(run_until_world(&mut app));
    let seat = current(&app);
    app.world_mut()
        .entity_mut(seat)
        .insert(StaySentence { turns_left: 2 });
    let before = tile_of(&app, seat);
    let start = turn_count(&app);
    settle(&mut app, 60);
    assert!(
        turn_count(&app) > start,
        "被按住的 AI 把回合机卡住了：`RequestStay` 那条出口没接上"
    );
    assert_eq!(tile_of(&app, seat), before, "停留的人棋子不该动");
}

#[test]
fn an_ai_seat_answers_a_window_it_was_hung_without_a_click() {
    // 报价类窗口的正面形状：挂在电脑座位上、没有人点，他自己把它答完，回合交还。
    //
    // **这一条同时是「座位分界」那条纪律的牙**：夹具那份脚手架的策略刻意留在默认
    // `DeclineAll`，而 `decision_system` 对同帧的多条回答取**最后一条** —— 所以一旦有人
    // 摘掉 `auto_answer_system` 里那句 `Controller::Ai` 的排除，夹具的 `Decline` 就会把
    // AI 的 `Accept` 盖掉，这条当场红（实测过：把那句判据短路之后红的是过户那一条）。
    let (mut app, players) = field(1);
    let ai = players[1];
    set_cash(&mut app, ai, 30_000);
    own(&mut app, LAND, None, 0, false);
    hang(&mut app, ai, Pending::Buy(LandId(LAND)));
    thaw(&mut app);
    // 四帧而不是两帧：`hang` 写的 `NextState` 要到下一帧的 `StateTransition` 才落地，
    // 而 AI 那条 system 挂在 `PreUpdate`（排在 `StateTransition` 之前），
    // 所以它读到的相位晚一帧 —— 第二帧答、第三帧才被 `decision_system` 消费
    settle(&mut app, 4);
    assert_eq!(
        owner_of(&mut app, LAND),
        Some(1),
        "窗口没被 AI 自己答掉（夹具那份是 DeclineAll，答了也不会过户）"
    );
    assert_eq!(window(&app), None, "答完之后窗口该收掉");
}

#[test]
fn an_ai_seat_bids_its_way_through_an_auction() {
    // 验收原话「在拍卖里出价」那一格：一台轮转式的机器，一圈一圈问不同的人，
    // 电脑座位既不等人点也不被别人代答，一路加到没人跟，最后**他把这块地拍走**。
    //
    // 断的是**最后成交的是谁**而不是「价格前进一档 + 下一位是谁」：第一版断的是后两条，
    // 而它们在 red-check 里红不起来 —— 放弃与加档同时发生时价格同样只前进一档，
    // 轮转同样落到旁观者那一格，两个读数都分不开这两种世界。
    let mut setup = GameSetup::default();
    setup.players.push(PlayerSetup {
        name: "电脑".into(),
        color: PLAYER_COLORS[2],
        gender: Gender::Male,
        controller: Controller::Ai(AiProfile::SHARK),
    });
    setup.players.push(PlayerSetup {
        name: "王小二".into(),
        color: PLAYER_COLORS[3],
        gender: Gender::Female,
        controller: Controller::Human,
    });
    let mut app = boot(setup, GameRules::instant(), GameSeed::default());
    app.insert_resource(DecisionPolicy::Hold);
    assert!(run_until_world(&mut app), "前置：棋盘要装配起来");
    freeze(&mut app);
    boot_to_wait_roll(&mut app);
    clear_gods(&mut app);
    let rows = roster(&mut app);
    let (founder, victim, ai, bystander) = (rows[0].0, rows[1].0, rows[2].0, rows[3].0);
    assert!(
        controller_of(&app, ai).is_ai(),
        "前置：第三位是电脑开的座位"
    );
    assert!(
        controller_of(&app, founder).is_human()
            && controller_of(&app, victim).is_human()
            && controller_of(&app, bystander).is_human(),
        "前置：除第三位之外都该是等人点的座位 —— 少了这一句，「谁替他答的」就说不清"
    );

    // 开一槌：拍卖卡由 P0 打在 P1 的地上 —— 开槌的人不进名单，所以名单是 P1、AI、P3
    own(&mut app, LAND, Some(1), 2, false);
    set_cash(&mut app, ai, 1_000_000);
    set_cash(&mut app, bystander, 1_000);
    let card = item_index(&app, "item_auction");
    deal(&mut app, founder, card.0);
    play_card(&mut app, founder, card.0, Some(CardAim::Land(LandId(LAND))));
    settle(&mut app, 3);
    assert!(lot_of(&app).is_some(), "前置：拍卖已开场");

    // 让轮转走到 AI 那一格：替「此刻该出价的那一位」答放弃，直到轮到电脑
    // （与 `tests/auction.rs` 同一条纪律：先读 `lot_of` 里的 `bidder()` 再替**那一位**按，
    // 而不是按自己猜的座位号 —— 名单里谁排在前面由座位序与排除条件共同决定）
    let mut turns = 0;
    while bidder_of(&app) != ai {
        turns += 1;
        assert!(turns < 8, "轮转八圈还没走到电脑那一格：他不在名单上");
        let asked = bidder_of(&app);
        auction_ask(&mut app, asked, game_core::prelude::BidMove::Withdraw);
        settle(&mut app, 2);
    }

    // 现在把脚手架换成「什么都答」（它对拍卖的答案是**放弃**），放松 AI 的钟，让它自己跑完：
    // 正确的形状是 AI 一声一声加到没人跟，最后**他把这块地拍走**；
    // 两份回答都生效的话他在这一圈就被请出名单，成交的就不是他
    // （第一版在这里断的是「价格前进一档 + 下一位是旁观者」，那条**红不起来**：
    // 放弃与加档同时发生时价格也只前进一档，而轮转同样落到旁观者 —— 两个读数都分不开这两种世界）
    app.insert_resource(DecisionPolicy::AcceptAll);
    thaw(&mut app);
    settle(&mut app, 12);

    let lot = lot_of(&app);
    assert!(
        lot.is_none(),
        "拍卖该已经散场，还挂着说明没人把最后一声答完：{lot:?}"
    );
    assert_eq!(
        owner_of(&mut app, LAND),
        Some(2),
        "这块地该由电脑拍走 —— 不是他就是脚手架替他放弃了"
    );
    assert!(
        cash_of(&mut app, ai) < 1_000_000,
        "AI 的现金该因为这一槌少了一截（成交的是他，不是别人）"
    );
}

fn bidder_of(app: &App) -> Entity {
    lot_of(app)
        .expect("前置：拍卖正在进行")
        .bidder()
        .expect("名单上没人可问的那一帧不该被读到")
}

fn owner_of(app: &mut App, land: u32) -> Option<u32> {
    let w = app.world_mut();
    let mut q = w.query::<(&LandId, &game_core::prelude::Owner)>();
    q.iter(w)
        .find(|(id, _)| id.0 == land)
        .and_then(|(_, owner)| owner.0.map(|p| p.0))
}

#[test]
fn a_held_clock_freezes_the_ai() {
    // 伪延迟那道门真的有门：钟按住时**一步都不许走**，放松之后立刻走。
    // 少了这一条，「延迟」只是一个没人验证的注释，而它下一刀要和 `fumble` 共用同一条子流。
    // 按住的写法是 `freeze` 而不是「把间隔调大」：间隔只在**动作之后**才起作用，
    // 装配那一刻 `ready_at` 还是 0，第一次动作照样发得出去（这条用例的第一版就漏在这一格上）
    let mut app = boot(all_ai(2), GameRules::instant(), GameSeed::default());
    assert!(run_until_world(&mut app));
    freeze(&mut app);
    let start = turn_count(&app);
    settle(&mut app, 120);
    assert_eq!(turn_count(&app), start, "钟按住时 AI 不该动手");
    thaw(&mut app);
    settle(&mut app, 240);
    assert!(
        turn_count(&app) > start,
        "放松之后 AI 还是不动 —— 那道门没有开（停在 {:?}，窗口 {:?}）",
        phase(&app),
        window(&app)
    );
}

#[test]
fn a_paused_game_freezes_the_ai_too() {
    // 暂停时人类那一侧全停，AI 那一侧也必须全停：少了 `not_paused` 那道门，
    // 玩家会看见「暂停菜单开着而棋子自己在走、模态自己在自己答自己」
    let mut app = boot(all_ai(2), GameRules::instant(), GameSeed::default());
    assert!(run_until_world(&mut app));
    app.insert_resource(Paused(true));
    let start = turn_count(&app);
    settle(&mut app, 120);
    assert_eq!(turn_count(&app), start, "暂停中 AI 不该动手");
    app.insert_resource(Paused(false));
    settle(&mut app, 240);
    assert!(turn_count(&app) > start, "取消暂停之后 AI 该继续走");
}

#[test]
fn the_ai_leaves_the_dice_and_the_deck_alone() {
    // 第 20 条子流的两半（P14b 之后）：**别人的流一位都不许动，自己这条必须被走过**。
    // 这里比纯函数用例多钉一层——真的跑一局有 AI 的棋，让「装配期之外有没有人动过骰子流」
    // 变成一条世界级的断言（与 `turn_flow` 那批同一条纪律）
    let seed = GameSeed(0x5EED_1234);
    let expected: Vec<u32> = {
        let mut r = game_core::rng::Rng::new(seed.0);
        (0..6).map(|_| r.roll_die()).collect()
    };
    let mut app = boot(all_ai(3), GameRules::instant(), seed);
    assert!(run_until_world(&mut app));
    settle(&mut app, 900);

    let got: Vec<u32> = events(&app)
        .into_iter()
        .filter_map(|line| {
            // 流水那一行的形状是 `Roll(名字,点数,from->to)`（`tests/common` 的 `collect_flow`）
            let rest = line.strip_prefix("Roll(")?;
            rest.split(',').nth(1)?.trim().parse::<u32>().ok()
        })
        .collect();
    assert!(got.len() >= 6, "这一局没摇出六次骰子，前置不成立：{got:?}");
    assert_eq!(&got[..6], &expected[..6], "AI 的取数挪动了骰子流");

    // ⚠️ **10-28 P14b 把这半条的方向掉过来了**。原来断的是「AI 那条流一位都没被消耗」
    // （那时 `pick_ai` 只有 `AiCadence::gap` 一个读者，而 core 的默认间隔是 0 → 不抽）。
    // 今天它有第二个读者：每次决策掷一次失误率（`AiPace::fumble`）。于是「没被消耗」从一条
    // 保证变成一条**缺陷证据** —— 这一句红，说的正是「打分写完了却没人掷骰、`fumble` 是个死字段」。
    // 现在钉的是两件相反方向的事：**别人的流一位都不许动**（上面那条骰子），
    // **自己这条流必须被走过**（下面这一条）。而「间隔为 0 不抽数」那半条仍由纯函数用例
    // `the_headless_default_cadence_never_holds` 钉着，没有丢。
    let after = app.world_mut().resource_mut::<Rng>().pick_ai().to_bits();
    let fresh = Rng::new(seed.0).pick_ai().to_bits();
    assert_ne!(
        after, fresh,
        "跑完一局 AI 却没碰过自己那条流：`fumble` 没有读者，那一格是死字段"
    );
}

// ============================ P14b：钱 ============================
//
// 上面那一节钉的是「AI 是一个座位」，这一节钉的是「这个座位会算账」。三条判据各自有名：
//
// - **同一块地在三档先验上分出两个结果** → [`three_personalities_divide_on_the_same_plot`]；
// - **「值不值」读的是以后的收益而不是钱包** → [`the_monopoly_jump_is_what_makes_goody_reach_for_his_wallet`]。
//   同一档先验、同样两万块：差一块才成那一族时不买，补全得了就买 —— 这一条才是
//   「打分」与「有钱就买」的分界线（P14a 那份一律 `Accept` 的脚手架在这里必须被推翻）；
// - **留底那条闸门读的是下一笔要付的账**，不是「买不买得起」 → [`the_reserve_gate_is_the_next_bill`]；
// - **`fumble` 在世界里真的换动作** → [`a_fumble_trades_the_best_answer_for_the_next_one`]。
//
// 这一节的用例几乎都先把 `fumble` 拧成 0（[`quiet_field`]）：测式子的时候按住噪声，
// 否则红下来分不清是判据变了还是骰子变了。噪声本身另有那一条专项用例。

/// 西单（地块 5）与王府井（地块 4）：yellow 那一族的**两块**，一次租金 180／160，
/// 地价 3000／2800。用它们而不是用 `LAND` 做「值不值」那两条用例，是因为
/// **单地块的那一族（red 的徐家汇、orange 的外滩）一买下就是垄断**，租金当场翻倍，
/// 「差一块才成一组」那个跳变在它们身上根本写不出来。
const WEST: u32 = 5;
const PALACE: u32 = 4;

fn level_of(app: &mut App, land: u32) -> u8 {
    let w = app.world_mut();
    let mut q = w.query::<(&LandId, &game_core::prelude::BuildingLevel)>();
    q.iter(w)
        .find(|(id, _)| id.0 == land)
        .map(|(_, level)| level.0)
        .expect("地块实体不在世界里")
}

/// 一位真人 + 一位电脑（大佬奸），但失误率由调用侧给。
fn seat_with_fumble(fumble: f32) -> (App, Entity) {
    let mut setup = GameSetup::seats(1);
    let Controller::Ai(profile) = &mut setup.players[1].controller else {
        panic!("`seats(1)` 的第二位必须是电脑座位");
    };
    profile.fumble = fumble;
    let (app, players) = field_with(setup);
    (app, players[1])
}

#[test]
fn three_personalities_divide_on_the_same_plot() {
    // 西单（yellow 那一族的一块）：一次租金 180、地价 3000 —— **16.7 倍回本**。
    // 三档的倍数是 25.6 / 17.2 / 11.2，于是两端各差出一截：大佬奸余 1608、乖宝宝差 984。
    // 中间那一档（普通人 3096 对 3000）只余 3%，所以**不断他** —— 一条靠 3% 余量站住的
    // 断言会在任何人挪一次地价时红，而那不是它想守的东西。
    // 断的是**谁把手举起来**，不是谁付了多少钱：那是这一刀唯一的可见产物。
    let (mut app, players) = quiet_field(3);
    let (shark, goody) = (players[1], players[3]);
    let mut took = Vec::new();
    for (index, seat) in players[1..].iter().enumerate() {
        own(&mut app, WEST, None, 0, false);
        own(&mut app, PALACE, None, 0, false);
        set_cash(&mut app, *seat, 20_000);
        ask(&mut app, *seat, Pending::Buy(LandId(WEST)));
        if owner_of(&mut app, WEST) == Some(index as u32 + 1) {
            took.push(*seat);
        }
    }
    assert!(
        took.contains(&shark),
        "大佬奸（25.6 倍）该吃下这块 16.7 倍回本的地，实际举手的：{took:?}"
    );
    assert!(
        !took.contains(&goody),
        "乖宝宝（11.2 倍）不该买这块 16.7 倍回本的地 —— 三档在同一块地上读不出差别就是白建"
    );
    assert_cash_audit(&mut app, GameRules::default().start_cash);
}

#[test]
fn the_monopoly_jump_is_what_makes_goody_reach_for_his_wallet() {
    // **这一条是「打分」与「有钱就买」的分界线，也是「值不值」读的是以后的收益的证据**：
    // 同一块地、同一档先验、同样的两万块 —— 手上没有那一族时他不买（180 × 11.2 = 2016
    // 盖不住 3000），先把同族的另一块给他，再买这一块就补全该族、租金翻倍
    // （360 × 11.2 = 4032 > 3000）→ 买。
    //
    // 它顺带钉住一件更隐蔽的事：`rent_if_mine` 必须把**假想我名下**那份视图喂给
    // `calculate_rent`。若图省事改读此刻的 `rent_of`（无主地收 0），两种情况都是 0 分，
    // 于是「不买 / 买」这一对断言同时红 —— 而 P14a 那份一律 `Accept` 的脚手架反倒全绿，
    // 所以这条红的不只是实现，还替「打分比不打分强」这句话立了据。
    let (mut app, players) = quiet_field(3);
    let goody = players[3];

    own(&mut app, PALACE, None, 0, false);
    own(&mut app, WEST, None, 0, false);
    set_cash(&mut app, goody, 20_000);
    ask(&mut app, goody, Pending::Buy(LandId(WEST)));
    assert_eq!(
        owner_of(&mut app, WEST),
        None,
        "差一块才成那一族时，乖宝宝不该为一块 16.7 倍回本的地掏钱"
    );

    own(&mut app, PALACE, Some(3), 0, false);
    own(&mut app, WEST, None, 0, false);
    ask(&mut app, goody, Pending::Buy(LandId(WEST)));
    assert_eq!(
        owner_of(&mut app, WEST),
        Some(3),
        "同一块地，只因为买下它就补全了 yellow 那一族（租金翻倍），乖宝宝就该伸手"
    );
}

#[test]
fn the_reserve_gate_is_the_next_bill() {
    // 留底 ＝ 下一笔要付的账，而这一格里它是**别人名下最贵的那一笔租金**：
    // 让真人拿着王府井（yellow 那一族只有一块在他手上 → 不成垄断，租金就是表上那个 160）。
    // 于是「买完只剩 60」与「买完还剩 400」是两个世界，而两块地的价格完全一样。
    let (mut app, players) = quiet_field(1);
    let seat = players[1];
    own(&mut app, 4, Some(0), 0, false);

    // 买价 3800：60 的余量低于那笔 160 的账 → 不举手续
    own(&mut app, LAND, None, 0, false);
    set_cash(&mut app, seat, 3_860);
    ask(&mut app, seat, Pending::Buy(LandId(LAND)));
    assert_eq!(
        owner_of(&mut app, LAND),
        None,
        "付完之后连下一笔租金都盖不住，这一格该放弃"
    );
    assert_eq!(
        cash_of(&mut app, seat),
        3_860,
        "该放弃的那一笔不许动钱（「钱少了而地没拿到」是这一族最坏的形状）"
    );

    // 同一块地、同一份先验，只是余量越过那道账 → 举手续
    set_cash(&mut app, seat, 4_200);
    ask(&mut app, seat, Pending::Buy(LandId(LAND)));
    assert_eq!(
        owner_of(&mut app, LAND),
        Some(1),
        "400 的余量盖得住那笔 160 的账，该买 —— 这一半是防「留底被写成一条永远不放的闸门」"
    );
}

#[test]
fn a_fumble_trades_the_best_answer_for_the_next_one() {
    // 同一块地、同样现金、同一档先验，只差 `fumble`：0 那一头买，1 那一头不买。
    // **这一条是「次优」这个词在世界上的唯一证据** —— 没有它，`fumble` 就是一个没人读的字，
    // 而「大佬奸偶尔手滑」这句话在代码里根本不存在（与「空跑的断言等于没断言」同一条教训）。
    for (fumble, bought) in [(0.0_f32, true), (1.0_f32, false)] {
        let (mut app, seat) = seat_with_fumble(fumble);
        own(&mut app, LAND, None, 0, false);
        set_cash(&mut app, seat, 20_000);
        ask(&mut app, seat, Pending::Buy(LandId(LAND)));
        assert_eq!(
            owner_of(&mut app, LAND),
            if bought { Some(1) } else { None },
            "失误率 {fumble} 那一头的结果不对"
        );
    }

    // 而**只有一条候选时不许越界**：乖宝宝在 100% 失误下仍然「不买」，
    // 既不 panic 也不改成买 —— 表上第二名不存在，那一次手滑就没有代价可言
    let mut setup = GameSetup::seats(1);
    let Controller::Ai(_profile) = &mut setup.players[1].controller else {
        panic!("`seats(1)` 的第二位必须是电脑座位");
    };
    setup.players[1].controller = Controller::Ai(AiProfile {
        fumble: 1.0,
        ..AiProfile::GOODY
    });
    let (mut app, players) = field_with(setup);
    let seat = players[1];
    own(&mut app, LAND, None, 0, false);
    set_cash(&mut app, seat, 20_000);
    ask(&mut app, seat, Pending::Buy(LandId(LAND)));
    assert_eq!(
        owner_of(&mut app, LAND),
        None,
        "只有一条候选时，失误不该凭空变出一个动作来"
    );
    assert_eq!(
        window(&app),
        None,
        "窗口该照常收掉，回合不许停在没人答的地方"
    );
}

#[test]
fn an_ai_seat_upgrades_its_own_plot() {
    // 升级那一格读 `build`，而读的是**增量租金**（加盖之后少付/多收的那一段）。
    // 这一条只钉正路：等级 1→2、钱真的少了一笔。三档在升级上分不出档（演示盘的
    // 加盖比买地划算得多，10 倍与 23 倍都盖得过），所以「按 `build` 分叉」的证据
    // 长在买地那两条上，这里不假装。
    let (mut app, players) = quiet_field(1);
    let seat = players[1];
    own(&mut app, LAND, Some(1), 1, false);
    set_cash(&mut app, seat, 20_000);
    let before = cash_of(&mut app, seat);
    ask(&mut app, seat, Pending::Upgrade(LandId(LAND)));
    assert_eq!(level_of(&mut app, LAND), 2, "加盖没落地");
    let after = cash_of(&mut app, seat);
    assert!(
        after < before,
        "加盖是花钱的：现金分文没动就说明那一步没走账本"
    );
    assert_cash_audit(&mut app, GameRules::default().start_cash);
}

/// 直接摆上负债 —— 还款那一格的夹具（与 `set_cash`／`own`／`set_shares` 同一类「只改读数」）。
fn set_loan(app: &mut App, player: Entity, loan: i64) {
    let w = app.world_mut();
    let mut entity = w.entity_mut(player);
    let mut balance = entity
        .get_mut::<game_core::prelude::BankBalance>()
        .expect("玩家身上该有 `BankBalance`");
    balance.loan = loan;
}

/// 让某一档先验单独走一次柜台：把世界摆回同一条起跑线，挂上窗口，让他办。
///
/// 三档共用这一句而不是各写一遍，是为了让「同一块世界」这件事真的成立 —— 任何一处
/// 忘了重置（比如忘了把 `LAND` 的归属换回来），读出的差别就是夹具的而不是性格的。
fn counter_visit(app: &mut App, seat: Entity, index: u32) {
    own(app, 2, Some(0), 5, false); // 真人拿着外滩商业区、Lv5：一笔租金 18000
    own(app, LAND, Some(index), 0, false); // 他自己名下有一块，才撑得出可借额度
    set_cash(app, seat, 0);
    set_loan(app, seat, 0);
    hang(app, seat, Pending::BankTeller);
    ask(app, seat, Pending::BankTeller);
}

#[test]
fn the_three_ladders_at_the_counter_reach_different_amounts() {
    // 同一块世界、同样身无分文、同一个柜台：三档各自借一次，借到的数**必须不同**。
    // 这就是 PLAN §四・十五 的 ② 那句「按性格分叉」在世界上的样子 —— 而它不是一句
    // 「肯 / 不肯」：大佬奸与普通人都肯，差别在**借这一层的几成**（`debt` 是档位不是开关）。
    //
    // 乖宝宝那一档是这条用例的另一半：他 `debt = 0.10` 在门槛（0.25）之下，于是
    // **不借、也不押**，名下那块地照原样站着 —— 4 代实测那句「AI 从不抵押，直到破产」
    // 在这里被保留成一种性格，而不是一条全局规则。
    let (mut app, players) = quiet_field(3);
    for (index, seat) in players[1..].iter().enumerate() {
        counter_visit(&mut app, *seat, index as u32 + 1);
    }
    let (shark, plain, goody) = (players[1], players[2], players[3]);
    let (_, shark_loan) = bank_of(&mut app, shark);
    let (_, plain_loan) = bank_of(&mut app, plain);
    let (_, goody_loan) = bank_of(&mut app, goody);
    assert!(
        shark_loan > plain_loan && plain_loan > 0,
        "同一世界里两档都该自救，且借的额度按 `debt` 分档：大佬奸 {shark_loan} / 普通人 {plain_loan}"
    );
    assert_eq!(
        goody_loan, 0,
        "乖宝宝那一档不该动身家 —— 他宁可死扛（§七 那条实测的归宿）"
    );
    assert_eq!(
        mortgage_of(&app, LAND),
        None,
        "而且他也不押：名下那块地照原样站着（柜台上的两档筹钱业务他都不办）"
    );
    assert_cash_audit(&mut app, GameRules::default().start_cash);
}

#[test]
fn a_counter_visit_always_gives_the_turn_back() {
    // 就算**什么都不办**，柜台也得收摊。少了 `TellerMove::Idle` 那一句 `Acknowledge`，
    // 症状不是崩而是「整局静默停住、画面还在动」（坑 38 那一族），而乖宝宝那一档
    // 每一局都会走到这一格 —— 所以这一条不是补白，是这一刀的门。
    let (mut app, players) = quiet_field(3);
    let goody = players[3];
    counter_visit(&mut app, goody, 3);
    assert_eq!(
        window(&app),
        None,
        "柜台没关：他什么都没办，可回合还停在那儿"
    );
    assert_ne!(
        phase(&app),
        TurnPhase::WaitDecision,
        "窗口关了而相位还钉在 `WaitDecision` —— 交还回合由这一步做"
    );
}

#[test]
fn a_rich_seat_pays_down_its_loan_at_the_counter() {
    // 有余钱又欠着钱 → 还。分是「这一笔下个月不再长息」，与筹钱那两档同一把尺。
    let (mut app, players) = quiet_field(1);
    let seat = players[1];
    // 这里**不**给别人摆一块高价地：留底会把「余钱」吃掉 —— 一笔 18000 的最贵租金让
    // 两万现金变成缺口而不是余量，于是这一档根本不上桌（第一版就是这么红的）。
    // 留底只由那五千欠款的月息撑出来，正好够说明「还钱这一档读的是省下的息」。
    set_loan(&mut app, seat, 5_000);
    set_cash(&mut app, seat, 20_000);
    hang(&mut app, seat, Pending::BankTeller);
    ask(&mut app, seat, Pending::BankTeller);
    let (_, loan) = bank_of(&mut app, seat);
    assert_eq!(loan, 0, "两万现金压着五千欠款，这一档该还掉");
    assert!(
        cash_of(&mut app, seat) < 20_000,
        "还掉的钱是从现金里走的（不是凭空清零）"
    );
    assert_cash_audit(&mut app, GameRules::default().start_cash);
}

#[test]
fn a_rich_seat_redeems_the_plot_it_pawned() {
    // 押着的地**收不到租**（`calculate_rent` 里抵押那一格短路成 0），所以赎回换回的是
    // 「下一次有人踩它」那笔钱 —— 而这个数**不能**直接读 `rent_of`（读出来就是 0，
    // 于是这一档永远上不了桌，且画面上一切正常）。这一条钉的就是那个假想视图。
    let (mut app, players) = quiet_field(1);
    let seat = players[1];
    own(&mut app, LAND, Some(1), 0, true); // 自己的地，押着
    set_cash(&mut app, seat, 20_000);
    hang(&mut app, seat, Pending::BankTeller);
    ask(&mut app, seat, Pending::BankTeller);
    assert_eq!(
        mortgage_of(&app, LAND),
        None,
        "两万现金压着一块押着的地，该把它赎回来 —— 赎回那一档的分读的是「假想没押」的租金"
    );
    assert_eq!(owner_of(&mut app, LAND), Some(1), "赎回不改归属");
    assert!(cash_of(&mut app, seat) < 20_000, "赎回是要付钱的");
}

#[test]
fn a_fumble_at_the_counter_drops_from_one_rung_to_the_next() {
    // 「次优」在**多档表**上同样要成立：不失误时他借（补得最多的一档），
    // 失误时他掉到第二档 —— 押一块地出去。这一条与 P14b① 那条两候选的失误用例
    // 不是重复：那一格只有「买 / 不买」，这一格有四档，第二名是**另一种业务**而不是同一件的反面。
    let (mut app, seat) = seat_with_fumble(0.0);
    own(&mut app, 2, Some(0), 5, false);
    own(&mut app, LAND, Some(1), 0, false);
    set_cash(&mut app, seat, 0);
    hang(&mut app, seat, Pending::BankTeller);
    ask(&mut app, seat, Pending::BankTeller);
    let (_, loan) = bank_of(&mut app, seat);
    assert!(
        loan > 0,
        "不失误的大佬奸该先借（借来的那一档补得最多），实际 {loan}"
    );
    assert_eq!(mortgage_of(&app, LAND), None, "第一档成了，就不该再动地");

    let (mut app, seat) = seat_with_fumble(1.0);
    own(&mut app, 2, Some(0), 5, false);
    own(&mut app, LAND, Some(1), 0, false);
    set_cash(&mut app, seat, 0);
    hang(&mut app, seat, Pending::BankTeller);
    ask(&mut app, seat, Pending::BankTeller);
    let (_, loan) = bank_of(&mut app, seat);
    assert_eq!(
        loan, 0,
        "手滑了就不该借成：这一局他借到的钱该是 0，实际 {loan}"
    );
    assert!(
        mortgage_of(&app, LAND).is_some(),
        "手滑之后他办的是第二档：把名下那块地押出去"
    );
}

#[test]
fn one_counter_visit_does_one_business() {
    // 「一次访问只办一件」不是补白，它挡的是两件具体的事：
    // ① 业务与「办完」不同帧（那两条 system 与 `decision_system` 没排过序，坑 36）；
    // ② **押完一块地立刻有资格赎回它** —— 那是一台不需要外力就能永远转下去的循环。
    // 这一条钉 ① 与「只办一件」本身：大佬奸面对一个借不满的缺口（两块地撑出来的额度
    // 远小于留底），只借一次就收摊，名下两块地一块都没动。
    // 把 `AiErrands` 那一格去掉，他会借到额度见底、再把地押出去 —— 两条断言同时红。
    let (mut app, players) = quiet_field(1);
    let seat = players[1];
    own(&mut app, 2, Some(0), 5, false); // 别人那块 Lv5 让缺口大到一笔业务补不满
    own(&mut app, LAND, Some(1), 0, false);
    own(&mut app, PALACE, Some(1), 0, false);
    set_cash(&mut app, seat, 0);
    set_loan(&mut app, seat, 0);
    hang(&mut app, seat, Pending::BankTeller);
    ask(&mut app, seat, Pending::BankTeller);

    let (_, loan) = bank_of(&mut app, seat);
    assert!(loan > 0, "第一档该办成：借到了一笔，实际 {loan}");
    assert_eq!(
        mortgage_of(&app, LAND),
        None,
        "借过一笔就收摊：这一次访问不该顺手把地也押出去"
    );
    assert_eq!(
        mortgage_of(&app, PALACE),
        None,
        "同上（第二块）—— 循环的入口就是「押完还能赎、赎完还能押」"
    );
    assert_eq!(window(&app), None, "办完那一句在下一帧发，窗口该已经收了");
}

// ======================== P14b③：回合前理财那一步 ========================
//
// 这一节钉的是「AI 在掷骰之前先看一眼活钱」这一刀在**结构上**成立的四件事：
//
// - **每回合最多一只股** → [`the_housekeeping_step_sells_one_stock_per_turn`]。它不是约定，
//   而是 `AiErrands::rebalanced` 那一格：卖完之后要等下一帧才掷骰，中间没有这格闸门的话
//   「缺口还没补满」会让同一回合一只一只卖下去。
// - **变现读 `debt`、买股读 `stock`**，两格各归各的 → [`the_goody_would_rather_go_broke_than_sell`]、
//   [`only_the_shark_dabbles_in_stocks`]。
// - **有地可买时先买地、跌得不够不买** → [`the_shark_keeps_his_money_for_the_land`]、
//   [`a_flat_market_is_not_worth_a_trade`] —— 「AI 炒股要尽量少」的两半。
// - **不开盘不下单**，与 `trade_request_system` 读同一句 → [`a_closed_market_keeps_the_ai_out`]。

/// 一档性格的电脑 + 一位等人点的真人，且**电脑是当前回合**。
///
/// `field_with` 按「当前是真人」建好（推到 `WaitRoll` 就停），这里再把轮转指针拨到电脑
/// 那一格 —— 理财那一步只读「现在轮到谁 + 相位」，与 `begin_turn` 无关。
/// 失误率统一按住：这一节测的是判据，「手滑」另有专项用例。
fn solo_ai(profile: AiProfile) -> (App, Entity) {
    let mut setup = GameSetup::seats(1);
    setup.players[1].controller = Controller::Ai(AiProfile {
        fumble: 0.0,
        ..profile
    });
    let (mut app, players) = field_with(setup);
    make_current(&mut app, players[1]);
    (app, players[1])
}

/// 让 AI 只动一次：先把钟调到「现在就能动」，动完 `charge` 把他按到一小时之后。
///
/// core 的默认钟是即时（`0..=0`，且不抽流），所以「逐帧观察 AI」在这里写不出来 ——
/// 一次性夹具要的是「一步一停」，而不是「跑几帧碰运气」。
fn step_ai(app: &mut App) {
    app.insert_resource(AiCadence {
        min: 3600.0,
        max: 3600.0,
    });
    app.insert_resource(AiClock::default());
    settle(app, 2);
}

/// 让这一动**只可能理财、不掷骰**：给他一句停留。
///
/// 「什么都没办」之后 AI 会紧接着掷骰，而 `GameRules::instant()` 下骰子可能当帧就落地收租 ——
/// 那会让「现金没动」这条断言红在一笔与本用例无关的账上。停留把那一动换成「不掷骰」，
/// 而理财排在它前面（P4④ 的新口径：被按住的人也还在牌桌上），于是判据不受影响。
fn hold_the_roll(app: &mut App, seat: Entity) {
    app.world_mut()
        .entity_mut(seat)
        .insert(StaySentence { turns_left: 1 });
}

/// 摆一只股的现价与历史。理财那一步读的是「现价 ÷ 历史均价」，所以历史也得摆。
fn quote(app: &mut App, id: &str, price: f32, history: &[f32]) {
    let w = app.world_mut();
    let mut q = w.query::<(&StockId, &mut StockPrice, &mut PriceHistory)>();
    let mut found = false;
    for (stock, mut now, mut past) in q.iter_mut(w) {
        if stock.0 == id {
            now.0 = price;
            past.0.clear();
            past.0.extend(history.iter().copied());
            found = true;
            break;
        }
    }
    assert!(found, "stocks.json 里没有 {id}");
}

/// 记下 AI 发出去的每一条下单请求。
///
/// 停牌那一条不能用「股数没变」来断：`trade_request_system` 自己也会拒绝，于是
/// 「AI 发了一条注定被吞掉的请求」与「AI 根本没发」在账上长得一模一样 —— 那正是
/// 「空跑的断言等于没断言」的形状。只有把请求本身抓住，才分得开这两件事。
#[derive(Resource, Default)]
struct TradeLog(Vec<String>);

fn watch_trades(app: &mut App) {
    app.init_resource::<TradeLog>();
    app.add_systems(
        Update,
        |mut log: ResMut<TradeLog>, mut requests: MessageReader<StockTradeRequested>| {
            for r in requests.read() {
                log.0.push(r.stock.0.clone());
            }
        },
    );
}

/// 六块地全有主 —— 「有地可买时先买地」那道闸门放行的前提。
fn sell_out_the_board(app: &mut App) {
    for land in 0..6 {
        own(app, land, Some(0), 0, false);
    }
}

#[test]
fn the_housekeeping_step_sells_one_stock_per_turn() {
    // 缺口远大于任何一笔持仓（一笔 18000 的租金把水位撑到 21600），于是他会把最大那一笔
    // 清掉；清完之后活钱还是不够，**第二动**如果还能卖就会接着卖另一只 —— 标记是那道闸门。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    own(&mut app, 2, Some(0), 5, false);
    set_cash(&mut app, seat, 0);
    quote(&mut app, "stock_001", 80.0, &[80.0]);
    quote(&mut app, "stock_002", 60.0, &[60.0]);
    set_shares(&mut app, seat, "stock_001", 100);
    set_shares(&mut app, seat, "stock_002", 100);

    step_ai(&mut app);
    assert_eq!(shares_of(&mut app, seat, "stock_001"), 0, "第一只该被卖掉");
    assert_eq!(cash_of(&mut app, seat), 8_000, "卖 100 股 × 80 元");
    assert_eq!(
        shares_of(&mut app, seat, "stock_002"),
        100,
        "一只股：同一动里第二只不许跟着卖"
    );

    // 同一回合的第二动：标记已经写下了，他该掷骰而不是接着卖
    step_ai(&mut app);
    assert_eq!(
        shares_of(&mut app, seat, "stock_002"),
        100,
        "同一回合里第二只股也被卖了 —— 「每回合最多一只」那道闸门没关上"
    );
}

#[test]
fn the_goody_would_rather_go_broke_than_sell() {
    // 卖股是「动身家」，所以资格读 `debt` 那一格（与押地借钱同一个门槛），不读 `stock` ——
    // 乖宝宝缺钱也不动自己的资产，§七 那条 4 代实测的归宿在这里原样保留。
    let (mut app, seat) = solo_ai(AiProfile::GOODY);
    own(&mut app, 2, Some(0), 5, false);
    set_cash(&mut app, seat, 0);
    quote(&mut app, "stock_001", 80.0, &[80.0]);
    set_shares(&mut app, seat, "stock_001", 100);
    hold_the_roll(&mut app, seat);

    step_ai(&mut app);
    assert_eq!(
        shares_of(&mut app, seat, "stock_001"),
        100,
        "乖宝宝那一档不该动身家 —— 缺钱也不卖股"
    );
    assert_eq!(cash_of(&mut app, seat), 0, "没成交就不许动钱");
}

#[test]
fn only_the_shark_dabbles_in_stocks() {
    // 同一份世界、同样两万闲钱、同一只跌了两成的股：只有 `stock` 过门槛的那一档会买。
    // 三档在这里读出两个结果 —— 这就是「性格是数不是枚举」在股票那一格上的证据。
    for (profile, buys) in [
        (AiProfile::SHARK, true),
        (AiProfile::PLAIN, false),
        (AiProfile::GOODY, false),
    ] {
        let (mut app, seat) = solo_ai(profile);
        sell_out_the_board(&mut app);
        set_cash(&mut app, seat, 20_000);
        // 只有 001 真跌下去了（72 对均价 90 → 0.8 倍），其余四只是平的
        for (id, base) in [
            ("stock_002", 60.0),
            ("stock_003", 45.0),
            ("stock_004", 35.0),
            ("stock_005", 55.0),
        ] {
            quote(&mut app, id, base, &[base]);
        }
        quote(&mut app, "stock_001", 72.0, &[90.0]);
        hold_the_roll(&mut app, seat);

        step_ai(&mut app);
        assert_eq!(
            shares_of(&mut app, seat, "stock_001") > 0,
            buys,
            "{} 那一档对「跌到 0.8 倍」的反应不对",
            profile.who
        );
        if buys {
            assert!(cash_of(&mut app, seat) < 20_000, "买股是要掏钱的");
        } else {
            assert_eq!(cash_of(&mut app, seat), 20_000, "没买就不许动钱");
        }
        for id in ["stock_002", "stock_003", "stock_004", "stock_005"] {
            assert_eq!(
                shares_of(&mut app, seat, id),
                0,
                "{id} 不该被碰 —— 一动只动一只"
            );
        }
    }
}

#[test]
fn the_shark_keeps_his_money_for_the_land() {
    // 同一份富裕条件，只差「场上还有一块无主的地」：这时钱是留着买地的。
    // 这一格是「尽量少」的第一半 —— 它比任何「现金超过多少才投资」的常数都稳，
    // 因为地价与物价都会动，而「还剩几块无主地」是棋盘自己的读数。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    for land in 0..5 {
        own(&mut app, land, Some(0), 0, false);
    }
    set_cash(&mut app, seat, 20_000);
    quote(&mut app, "stock_001", 72.0, &[90.0]);
    hold_the_roll(&mut app, seat);

    step_ai(&mut app);
    assert_eq!(
        shares_of(&mut app, seat, "stock_001"),
        0,
        "还有地可买就去炒股 —— 那一步该等棋盘卖完"
    );
    assert_eq!(cash_of(&mut app, seat), 20_000, "没买就不许动钱");
}

#[test]
fn a_flat_market_is_not_worth_a_trade() {
    // 闲钱条件全满足，只是没有一只跌到 0.9 倍 → 一股也不买。
    // 「有闲钱」是一件事，「见着有闲钱就下单」是另一件 —— 这一条是「尽量少」的第二半。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    sell_out_the_board(&mut app);
    set_cash(&mut app, seat, 20_000);
    for (id, base) in [
        ("stock_001", 80.0),
        ("stock_002", 60.0),
        ("stock_003", 45.0),
        ("stock_004", 35.0),
        ("stock_005", 55.0),
    ] {
        quote(&mut app, id, base, &[base]);
    }
    hold_the_roll(&mut app, seat);

    step_ai(&mut app);
    for id in [
        "stock_001",
        "stock_002",
        "stock_003",
        "stock_004",
        "stock_005",
    ] {
        assert_eq!(shares_of(&mut app, seat, id), 0, "平盘也买：{id}");
    }
    assert_eq!(cash_of(&mut app, seat), 20_000, "没买就不许动钱");
}

#[test]
fn a_closed_market_keeps_the_ai_out() {
    // 下单处与 AI 读的是同一句 `market_halt_now`（停牌／休市）。少了这一句，AI 会在停牌那几天
    // 每回合发一条注定被静默吞掉的请求 —— 账上什么都没动，而「他为什么不理财」没人查得出来。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    own(&mut app, 2, Some(0), 5, false);
    set_cash(&mut app, seat, 0);
    quote(&mut app, "stock_001", 80.0, &[80.0]);
    set_shares(&mut app, seat, "stock_001", 100);
    run_op(&mut app, |books, _| {
        books
            .session
            .suspensions
            .suspend(Institution::StockMarket, 3);
    });
    hold_the_roll(&mut app, seat);
    watch_trades(&mut app);

    step_ai(&mut app);
    let sent = app.world().resource::<TradeLog>().0.clone();
    assert!(
        sent.is_empty(),
        "停牌时 AI 还是发了下单请求 {sent:?} —— 下单处会把它静默吞掉，账上什么都不会动"
    );
    assert_eq!(
        shares_of(&mut app, seat, "stock_001"),
        100,
        "停牌时不许下单"
    );
    assert_eq!(cash_of(&mut app, seat), 0, "没成交就不许动钱");
}

// ============================ P14c①：出牌 ============================
//
// 这一节钉「AI 会用卡」这句话里可测的四件事：
//
// - **购地卡买最值的那块** → [`an_ai_seat_plays_its_grab_card_on_the_best_plot`]（与买地那一问
//   同一个式子，所以「卡说买得起而账本说买不起」在结构上写不出来）；
// - **伤害那一路的分按 `harm` 打折** → [`the_harm_axis_decides_which_card_gets_played`]：
//   同一个世界、同一副手牌，大佬奸先查封、普通人先涨价 —— 「性格是数不是枚举」在这一刀上的证据；
// - **门槛之下根本不上桌** → [`the_goody_never_uses_a_harmful_card`]；
// - **一次只出一张** → [`one_card_per_turn`]，以及被动卡不走进这条路 →
//   [`a_passive_card_is_never_played`]。

/// 一块地上盖着的限时状态（`None` = 没挂着）。
fn status_of(app: &App, land: u32) -> Option<LandStatus> {
    let entity = *app
        .world()
        .resource::<ActiveBoard>()
        .lands
        .get(&LandId(land))
        .expect("prototype 里有这块地");
    app.world()
        .entity(entity)
        .get::<Timed<LandStatus>>()
        .map(|held| held.kind)
}

#[test]
fn an_ai_seat_plays_its_grab_card_on_the_best_plot() {
    // 演示盘六块空地，大佬奸（25.6 倍回本）算下来最值的是外滩商业区：
    // 单地块那一族一买下就是垄断（租金 280 → 560），560 × 25.6 − 4400 = 9936，全场最高。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    set_cash(&mut app, seat, 20_000);
    let grab = item_index(&app, "item_grab").0;
    deal(&mut app, seat, grab);

    step_ai(&mut app);
    assert_eq!(
        owner_of(&mut app, 2),
        Some(1),
        "购地卡该买下算下来最值的那块（外滩商业区 L2）"
    );
    assert!(
        hand_of(&app, seat).is_empty(),
        "打出去的卡该从手里消失（卡没了事没成是缺陷，事成了卡还在也是）"
    );
    assert_eq!(
        cash_of(&mut app, seat),
        15_600,
        "买地走的是 Books::buy 正路：地价 4400 从现金里扣"
    );
}

#[test]
fn the_harm_axis_decides_which_card_gets_played() {
    // 同一个世界、同一副手牌（涨价卡 + 查封卡）：
    // 自己那块西单一次收 180（yellow），对手那块南京路一次收 180（brown，Lv1）。
    // · 大佬奸：查封 180 × 0.90 = 162 > 涨价 180 × 0.80 = 144 → 先害人；
    // · 普通人：查封 180 × 0.30 = 54  < 涨价 180 × 0.50 = 90  → 先置业。
    // 这就是 `harm` 不只是门槛、还是**损害那一路的乘数**的直接后果。
    //
    // ⚠️ 两条街必须选**不同族**：限时状态盖的是整条街（`area: street`），
    // 所以在同一族的两块地上比对「哪块被盖了」是分不出「他打的是哪一块」的 ——
    // 第一版正是这么写的，把方向判据拆掉也照样绿（`status_aims` 那条 `mine` 检查
    // 被这次 red-check 抓出来是空跑的）。
    for (profile, sealed, hiked) in [
        (AiProfile::SHARK, true, false),
        (AiProfile::PLAIN, false, true),
    ] {
        let (mut app, seat) = solo_ai(profile);
        own(&mut app, 5, Some(1), 0, false); // 他自己：西单（yellow），一次收 180
        own(&mut app, 0, Some(0), 1, false); // 对手：南京路（brown Lv1），一次收 180
        let hike = item_index(&app, "item_hike").0;
        let seal = item_index(&app, "item_seal").0;
        deal(&mut app, seat, hike);
        deal(&mut app, seat, seal);
        hold_the_roll(&mut app, seat);

        step_ai(&mut app);
        assert_eq!(
            status_of(&app, 0) == Some(LandStatus::Sealed),
            sealed,
            "{} 该不该查封对手那条街？",
            profile.who
        );
        assert_eq!(
            status_of(&app, 5) == Some(LandStatus::PriceHike),
            hiked,
            "{} 该不该给自家的街涨价？",
            profile.who
        );
        assert_eq!(
            hand_of(&app, seat).len(),
            1,
            "{} 只该打出一张（另一张还在手里）",
            profile.who
        );
    }
}

#[test]
fn the_goody_never_uses_a_harmful_card() {
    // 乖宝宝的 `harm = 0.05` 在门槛（0.20）之下 —— 手里有一张查封卡也不打。
    // 这一格钉的是**门槛**而不是乘数：只乘不拦的话，一块够肥的地会让乖宝宝也伸手。
    let (mut app, seat) = solo_ai(AiProfile::GOODY);
    own(&mut app, 5, Some(0), 0, false);
    let seal = item_index(&app, "item_seal").0;
    deal(&mut app, seat, seal);
    hold_the_roll(&mut app, seat);

    step_ai(&mut app);
    assert_eq!(status_of(&app, 5), None, "乖宝宝查封了别人的地");
    assert_eq!(hand_of(&app, seat).len(), 1, "卡该还在手里");
}

#[test]
fn an_ai_seat_jails_the_richest_opponent() {
    // 陷害卡：目标按「他名下那一轮租金之和 × 害意」排，已经在里面的跳过。
    // 两张牌打在两个人身上会在世界状态上留下同一句话，所以这里只摆一个人 ——
    // 「挑谁」那一半由 `detain_aims` 的排序决定，与 `the_harm_axis...` 同一条纪律。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    let human = roster(&mut app)[0].0;
    own(&mut app, 5, Some(0), 0, false); // 真人的西单：他挡在我前面 180 块
    let frame = item_index(&app, "item_frame").0;
    deal(&mut app, seat, frame);
    hold_the_roll(&mut app, seat);

    step_ai(&mut app);
    let held = app
        .world()
        .entity(human)
        .get::<Detention>()
        .expect("真人该被关起来");
    assert_eq!(held.term.turns(), Some(5), "陷害卡写的是 5 天（卡面）");
    assert!(
        hand_of(&app, seat).is_empty(),
        "卡该被花掉 —— 一直在手里说明门或出牌那条路没接上"
    );
}

#[test]
fn one_card_per_turn() {
    // 两张购地卡、六块空地：第一动买下最值的那块，第二动**不许**接着买 ——
    // 标记（`AiErrands::played_card`）是那道闸门，不是「一张卡只能用一次」。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    set_cash(&mut app, seat, 20_000);
    let grab = item_index(&app, "item_grab").0;
    deal(&mut app, seat, grab);
    deal(&mut app, seat, grab);
    hold_the_roll(&mut app, seat);

    step_ai(&mut app);
    assert_eq!(hand_of(&app, seat).len(), 1, "第一张该被打出去");
    step_ai(&mut app);
    assert_eq!(
        hand_of(&app, seat).len(),
        1,
        "同一回合里第二张也被打出去了 —— 「一次只出一张」那道闸门没关上"
    );
    // 而且第二张没买成第二块地：名下只有外滩那一块
    assert_eq!(owner_of(&mut app, 3), None, "第二张把徐家汇也买了");
}

/// 一条**故意不写**的用例，理由记在这里：「被动卡不会被主动打出去」今天测不出来。
///
/// `item_cards.json` 里三张被动卡的 `effects` 都是空的，所以就算 `card_table` 不跳过它们，
/// `card_aims` 也会因为「一条效果都没有」返回空 —— 两种世界在**画面与账上**一模一样。
/// 真正守这条的是 `play_card_system` 里那句 `is_passive`（它在目标与落空两道门之前），
/// 而 AI 这一侧那句只是让这件事在类型/判据上说得通。等哪天出现一张**带效果的被动卡**
/// （`CardSet::validate` 今天不拦这种形状），这条用例才有牙 —— 那时补它，现在不写一条
/// 怎么改都绿的断言（「空跑的断言等于没断言」第六次的教训就在上一节）。
#[test]
fn the_grab_card_skips_a_plot_it_could_not_keep() {
    // 现金 4500：外滩 4400 买得起，但买完只剩 100 —— 连对手西单那笔 180 的租都盖不住。
    // 于是这一块不进候选表（「被拒的候选不进表」），退而买守得住的徐家汇（3800，剩 700）。
    let (mut app, seat) = solo_ai(AiProfile::SHARK);
    own(&mut app, 5, Some(0), 0, false);
    set_cash(&mut app, seat, 4_500);
    let grab = item_index(&app, "item_grab").0;
    deal(&mut app, seat, grab);

    step_ai(&mut app);
    assert_eq!(
        owner_of(&mut app, 2),
        None,
        "买完守不住留底的那块不该买（钱少了而地没拿到是这一族最坏的形状）"
    );
    assert_eq!(
        owner_of(&mut app, 3),
        Some(1),
        "该退而买买得起又守得住的那块（徐家汇 3800，剩 700）"
    );
}
