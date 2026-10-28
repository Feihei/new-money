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
use bevy::prelude::NextState;
use common::*;
use game_core::prelude::{
    AiCadence, AiClock, AiProfile, CardAim, Controller, GameRules, GameSeed, GameSession,
    GameSetup, Gender, LandId, PLAYER_COLORS, Paused, Pending, PendingDecision, PlayerSetup,
    StaySentence, TurnPhase,
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
