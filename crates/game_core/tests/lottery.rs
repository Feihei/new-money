//! P8 乐透的验收：买券进池、号码独占、每月 15 号开奖、没人猜中就滚到下个月。
//!
//! PLAN §四 P8 那行的验收原话是三句，这个文件一格一句：
//!
//! - **跨月累积正确** → [`an_unwon_month_rolls_the_pot_and_it_earns_interest`]：
//!   流局那个月池子一分不动、月末照样结息，下个月开出中奖号时发的就是滚存后的那个数；
//! - **中奖独得奖池** → [`the_winner_takes_the_whole_pot_and_every_ticket_is_void`]：
//!   一个人持有中奖号时整池归他，而**别人的券也一起作废**（一期一结）；
//! - **每笔进池的钱都能从流水追溯** → 几乎每条用例末尾一句 [`assert_pot_audit`]：
//!   它是第三张账（池子不是任何人的钱包，`assert_cash_audit` 读不到它）。
//!
//! 第四句是 10-26 实报补进来的，而它改的就是上面第二句的**来路**：
//!
//! - **一个号全场只有一张** → [`a_number_another_player_holds_cannot_be_bought`] 与
//!   [`only_one_player_can_hold_the_winning_number`]：独占才是「独得」的真正保证 ——
//!   没有它，中奖号可以被两个人同时持有，「独得」就得让位给均分与余数留池。
//!   [`a_taken_number_is_free_again_after_the_period_closes`] 钉的是这件事的另一半：
//!   占号的是**券**而不是别张表，所以一期一结把券清掉的那一刻，号就重新能买了。
//!
//! 号码怎么控制得住：开奖号来自 `Rng` 的乐透子流，而**开局装配一个数都不取这条流**，
//! `draw_lottery_system` 是它唯一的读者 —— 所以「这个月开几号」在对局开始前就定了，
//! 用例先把它算出来，再买一个必中或必不中的号。这与 `seed_rolling_first` 挑骰点、
//! `stack_card` 压牌顶是同一条手法，不是新发明；赌随机的那类断言（P3d① 翻过的两个坑）
//! 在这里从源头避开。
//!
//! 钱一律掐**差值**不掐绝对余额：这一路上会撞见附身的神与命运牌的罚金，
//! 20 000 那个数在 45 个回合之后不该被当成参照物。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::NextState;
use common::*;
use game_core::prelude::{
    ActiveBoard, Bankrupt, Date, GameRules, GameSeed, GameSetup, Jackpot, PLAYER_COLORS, Pending,
    PlayerSetup, Rng, StaySentence, TurnPhase,
};

/// 主环格数与彩票中心那一格（`prototype.json` 的 19 号；P8 追加在环尾，
/// P10d① 两家公司插进环中段后它从 14 号顺移到 16 号，P10d② 重排演示盘后落到 18 号，
/// P11 的新闻中心插在 13 号、它跟着顺移到 19 号）。
/// 两个数写在一起是因为前置要算「走 face 步正好落到 19 号」—— 环一变长这条就得重数，
/// 而 `board.rs` 的环断言与 `tools/validate_board.py` 会一起响。
const LOOP: u32 = 21;
const COUNTER: u32 = 20;

/// 默认那套规则：36 个号、一注 1000、月息 5%。
fn rules() -> GameRules {
    GameRules::instant()
}

/// 另一名玩家。
fn other(app: &mut App, p0: Entity) -> Entity {
    *roster(app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| e)
        .expect("默认阵容有两名玩家")
}

/// 这颗种子上第 `n` 次开奖（1 起）会开几号。见模块头那条「乐透流只有开奖会取」。
fn draw_number(seed: GameSeed, numbers: u32, n: usize) -> u8 {
    let mut rng = Rng::new(seed.0);
    for _ in 1..n {
        rng.pick_lottery(1..=numbers.max(1));
    }
    rng.pick_lottery(1..=numbers.max(1)) as u8
}

/// 这颗种子上前两次开奖号 `(第一次, 第二次)`。
fn draws(seed: GameSeed, numbers: u32) -> (u8, u8) {
    (draw_number(seed, numbers, 1), draw_number(seed, numbers, 2))
}

/// 找一颗「前两次开奖号不同」的种子。
///
/// 跨月那条要的是「这个月不中、下个月中」，而同一个号连开两次是真会发生的（36 分之 1）。
/// 写死一颗种子等于把用例钉在一个数上，所以照 [`seed_rolling_first`] 的手法现搜 ——
/// 它顺带也在断言「这条流真的在动」：一万颗种子里若前两次都相同，那是随机流的缺陷。
fn seed_two_draws() -> (GameSeed, u8, u8) {
    for s in 0..10_000u64 {
        let seed = GameSeed(s);
        let (a, b) = draws(seed, 36);
        if a != b {
            return (seed, a, b);
        }
    }
    panic!("一万颗种子里没有一颗前两次开奖不同，乐透那条流可疑");
}

/// 装配到「P0 刚踩上彩票中心、柜台窗口挂着没人答」，阵容由调用方给。
///
/// 种子由调用方给（用例要控制开奖号），所以前置不靠 `ready_on` 挑骰点，而是反过来：
/// 先读这颗种子的第一掷，再把他摆到「走 face 步正好落到 14 号」那一格。踩到柜台这一步
/// 走的仍是真走路、真落格分派 —— 夹具没有替他 `offer` 那个窗口。
///
/// 开局那两位神在这里清掉：他们会站在路上某格，而「走过去踩到柜台」这一路上撞见财神的话，
/// 附身当场搬一笔钱，下面那些现金差值就不再只等于票价。清场不影响奖池（两条账互不相干）。
fn at_the_setup(seed: GameSeed, rules: GameRules, setup: GameSetup) -> (App, Entity, Entity) {
    let mut app = boot(setup, rules, seed);
    app.insert_resource(DecisionPolicy::Hold);
    pin_epoch(&mut app, Date::new(2048, 1, 1));
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = other(&mut app, p0);
    clear_gods(&mut app);
    let face = Rng::new(seed.0).roll_die();
    place(&mut app, p0, (LOOP + COUNTER - face) % LOOP);

    roll(&mut app, p0);
    settle(&mut app, 6);

    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Lottery)),
        "前置该把他送到柜台前：骰面 {face}，该落在 {COUNTER} 号，流水 {:?}",
        ledger(&app)
    );
    (app, p0, p1)
}

/// 默认那套规则下的柜台前置（双人盘）。
fn at_the_counter(seed: GameSeed, rules: GameRules) -> (App, Entity, Entity) {
    at_the_setup(seed, rules, GameSetup::default())
}

/// 三人盘的柜台前置。
///
/// 为什么要第三人：`turn.rs` 里 `living <= 1` 一到就终局，两人盘上「让一个人破产」等于
/// 「这一局结束了」—— 回合机再也不回掷骰前，而破产那一条用例要走的正是
/// **他死了之后**另一个人重新踩到柜台。第三人买不到任何东西，只是让局继续转。
fn three_at_counter() -> (App, Entity, Entity) {
    let mut setup = GameSetup::default();
    setup.players.push(PlayerSetup {
        name: "王小二".into(),
        color: PLAYER_COLORS[2],
        gender: game_core::player::Gender::Male,
    });
    at_the_setup(GameSeed::default(), rules(), setup)
}

/// 默认种子（`GameSeed::default()`）的柜台前置。
fn at_counter() -> (App, Entity, Entity) {
    at_the_counter(GameSeed::default(), rules())
}

/// 替 `player` 把 `number` 按下去，并空跑两帧让柜台办这一笔。
fn buy(app: &mut App, player: Entity, number: u8) {
    lottery_ask(app, player, number);
    settle(app, 2);
}

/// 按「离开」：换成 `DeclineAll`，下一帧自动答一句「办完了」，回合交还给状态机。
fn leave_the_counter(app: &mut App) {
    set_policy(app, DecisionPolicy::DeclineAll);
    settle(app, 2);
    assert_eq!(pending_of(app), None, "离开之后不该还挂着窗口");
    assert_ne!(phase(app), TurnPhase::WaitDecision);
}

/// 把 `player` 重新送到柜台前：先等回合机回到「掷骰之前」，再倒回骰子流，
/// 让他从「走 face 步正好到 14 号」那一格出发。
///
/// 等那一跳不是保险丝而是必需：`leave_the_counter` 只空跑了帧数刚够关掉窗口，
/// 而 WaitDecision → TurnEnd → TurnStart → WaitRoll 还要几帧 —— 在别的阶段按掷骰会被
/// `roll_dice_system` 直接拒掉，症状正是「他没能踩到柜台」。
///
/// `rewind_dice` 换掉的是整个 `Rng`（十二条流一起回到开局），而这几条用例都在**第一次**
/// 开奖之前把券买完 —— 乐透流回到起点恰好还是同一个号，[`draw_number`] 的预测继续成立。
/// 反过来，若哪天有人把「rewind 之后再等一次开奖」写进用例，那个预测就得跟着改。
fn back_to_the_counter(app: &mut App, player: Entity) {
    assert!(
        run_until_phase(app, TurnPhase::WaitRoll, 30),
        "回合机没回到掷骰前，停在 {:?}",
        phase(app)
    );
    // 窗口重新挂成「没人答」：`leave_the_counter` 把策略换成了 `DeclineAll`，而那正是
    // 「踩到柜台的那一帧就被自动答掉」—— 断言就读不到窗口了
    set_policy(app, DecisionPolicy::Hold);
    rewind_dice(app);
    make_current(app, player);
    let seed = *app.world().resource::<GameSeed>();
    let face = Rng::new(seed.0).roll_die();
    place(app, player, (LOOP + COUNTER - face) % LOOP);
    roll(app, player);
    settle(app, 6);
    assert_eq!(
        pending_of(app),
        Some((player, Pending::Lottery)),
        "他也该踩得到柜台：{:?}",
        ledger(app)
    );
}

/// 一帧一帧把日历推到第 `want` 次 tick（= `epoch + want 天`），一跨过就立刻收手。
///
/// 不用 `play_turns(app, 14)` 那种「跑到第 14 回合为止」：有人被关着时那一回合不掷骰、
/// 整回合被跳过，一次调用能跨**两个**回合末。多出来的那一帧里命运牌会再动一次钱，
/// 「他最后一条现金流水就是那笔奖金」这条断言就不再成立 —— 而它正是这条用例要问的那句话。
///
/// 判据用 tick 不用「第几回合」：`tick` 就是日子，而前置里柜台、离开各吃掉几帧，
/// 没人该去记那是第几回合。日子算错了会 panic 在这一句上，而不是让下面的开奖断言去猜。
fn play_until_tick(app: &mut App, want: u32) {
    let mut pending = false;
    for _ in 0..20_000 {
        if tick_of(app) >= want {
            return;
        }
        assert_ne!(
            phase(app),
            TurnPhase::Over,
            "对局提前结束，推不到第 {want} 天"
        );
        match phase(app) {
            // `pending` 是必需的：从按下到真正掷出要跨几帧（消息帧末投递），期间状态仍是
            // `WaitRoll`，不加守卫会一次排进好几个请求（与 `play_turns` 同一条坑）。
            // 换到别的阶段就把它撤掉 —— 忘了撤就等于此后再也不掷骰，整圈空转到超时。
            TurnPhase::WaitRoll if !pending => {
                let cur = current(app);
                // 被停留按住的人那颗按钮翻成了「停留」（P4④）：脚手架要像真人一样按它，
                // 否则规则层拒掉掷骰、这里就原地空转到超时。新闻池每长一条，抽到
                // 「豪雨袭城」的种子的分布就挪一次 —— P11c② 就是把这一格从 `play_turns`
                // 抄过来补上的（症状是「推到第 45 天失败，停在 tick 35」）
                if app.world().entity(cur).get::<StaySentence>().is_some() {
                    stay(app, cur);
                } else {
                    roll(app, cur);
                }
                pending = true;
            }
            _ => pending = false,
        }
        app.update();
    }
    panic!("推到第 {want} 天失败，停在 tick {}", tick_of(app));
}

/// PLAN §四 P8 验收第三句的最小形状：一注的钱**全额**进池，而且两边各留一条流水。
#[test]
fn a_ticket_moves_cash_into_the_pot_and_leaves_two_lines() {
    let (mut app, p0, _p1) = at_counter();
    let price = rules().ticket_price;
    let before = cash_of(&mut app, p0);

    buy(&mut app, p0, 7);

    assert_eq!(
        cash_of(&mut app, p0),
        before - price,
        "买券动的是现金，不是点券（两套账永不通兑）"
    );
    assert_eq!(pot_of(&app), price, "票价全额进池，规则层不抽头");
    assert_eq!(tickets_of(&app, p0), vec![7]);
    let led = ledger(&app);
    assert!(
        led.iter().any(|l| l == &format!("Ticket(P0,7号,{price})")),
        "买券该留一条凭据：{led:?}"
    );
    assert!(
        led.iter()
            .any(|l| l == &format!("Pot(买券+{price}→{price})")),
        "池子那一步要说得出为什么动：{led:?}"
    );
    assert_pot_audit(&mut app, 0);
    // 买完还在柜台前：回合不由这一笔交还
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Lottery)),
        "买一注不该把窗口顺手关掉"
    );
}

/// 一次柜台可以买好几注（窗口而不是报价），而号码是**升序**存的 —— 面板直接念它。
#[test]
fn several_tickets_stack_up_and_sort_themselves() {
    let (mut app, p0, _p1) = at_counter();
    let price = rules().ticket_price;
    let before = cash_of(&mut app, p0);

    for number in [23u8, 4, 11] {
        buy(&mut app, p0, number);
    }

    assert_eq!(tickets_of(&app, p0), vec![4, 11, 23]);
    assert_eq!(pot_of(&app), price * 3);
    assert_eq!(cash_of(&mut app, p0), before - price * 3);
    assert_pot_audit(&mut app, 0);
}

/// 窗口是唯一入口：离开之后再按，钱、券、池子三样都不动。
#[test]
fn walking_away_closes_the_gate() {
    let (mut app, p0, _p1) = at_counter();
    buy(&mut app, p0, 7);
    let pot = pot_of(&app);
    let before = cash_of(&mut app, p0);
    let held = tickets_of(&app, p0);

    leave_the_counter(&mut app);
    buy(&mut app, p0, 8);

    assert_eq!(cash_of(&mut app, p0), before, "办不成的那一笔不该扣钱");
    assert_eq!(tickets_of(&app, p0), held, "券也没多发一张");
    assert_eq!(pot_of(&app), pot, "池子一分没进");
    assert_pot_audit(&mut app, 0);
}

/// 站在柜台前的是甲，乙的点击不能替他买 —— 与 ATM 的「认人不认回合」相反，
/// 这一格没有长期资格，窗口就是全部的判据。
#[test]
fn an_opponent_cannot_buy_on_my_behalf() {
    let (mut app, p0, p1) = at_counter();
    let before = cash_of(&mut app, p1);

    buy(&mut app, p1, 7);

    assert_eq!(cash_of(&mut app, p1), before, "乙的钱一分没动");
    assert!(tickets_of(&app, p1).is_empty(), "乙也没拿到券");
    assert!(tickets_of(&app, p0).is_empty(), "更不该记到甲头上");
    assert_eq!(pot_of(&app), 0, "池子也没进钱");
}

/// 号码池之外的那个号，扣钱之前就被拦下（顺序写在 `buy_allowed`，这里验它的后果）。
#[test]
fn a_number_outside_the_pool_costs_nothing() {
    let (mut app, p0, _p1) = at_counter();
    let before = cash_of(&mut app, p0);

    for number in [0u8, 37, 99] {
        buy(&mut app, p0, number);
    }

    assert_eq!(cash_of(&mut app, p0), before);
    assert!(tickets_of(&app, p0).is_empty());
    assert_eq!(pot_of(&app), 0);
}

/// 同一个号一人只有一张：第二张拒掉，池子也不重复进钱。
/// 这一格挡的是**同一个人**重复按同一个号，而「另一个人来按这个号」是另一条拒绝
/// （`LotteryDeny::Taken`，下面那两条）。P8 初版给这一格的理由是「均分按人，多买同号
/// 没有玩法意义」，那个理由已经随独占一起没了 —— 留下来的理由更硬：规则层那条 `buy` 是先扣钱
/// 再发券，判据若漏了这一格，重复按号的人**钱照样被扣**而券一张不多。
#[test]
fn the_same_number_cannot_be_bought_twice_by_one_person() {
    let (mut app, p0, _p1) = at_counter();
    let price = rules().ticket_price;

    buy(&mut app, p0, 7);
    let before = cash_of(&mut app, p0);
    buy(&mut app, p0, 7);

    assert_eq!(cash_of(&mut app, p0), before, "第二次没扣钱");
    assert_eq!(tickets_of(&app, p0), vec![7], "手里还是那一张");
    assert_eq!(pot_of(&app), price, "池子只进了第一注");
    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|l| l.starts_with("Ticket(P0,7号"))
            .count(),
        1,
        "凭据也只有一条 —— 两条就成了「池子里有两注的钱」"
    );
    assert_pot_audit(&mut app, 0);
}

/// **号码独占**（4 代口径，10-26 实报）：甲买走的号，乙就买不成了 —— 而乙**换个号照买**。
/// 后面那半句是这条用例的牙齿：只断「乙没买到 7 号」的话，它与「乙压根没站在柜台前」
/// 「乙被规则层随便一条拒绝挡在门外」三种缺陷长得分不清（P3d② 那次翻出的假用例就是
/// 这个形状 —— 拒的是钱而不是窗口，短路掉判据也不会红）。
#[test]
fn a_number_another_player_holds_cannot_be_bought() {
    let (mut app, p0, p1) = at_counter();
    let price = rules().ticket_price;
    buy(&mut app, p0, 7);
    leave_the_counter(&mut app);

    back_to_the_counter(&mut app, p1);
    let before = cash_of(&mut app, p1);
    buy(&mut app, p1, 7);
    assert_eq!(
        cash_of(&mut app, p1),
        before,
        "被占走的这一注不能扣钱：{:?}",
        ledger(&app)
    );
    assert!(tickets_of(&app, p1).is_empty(), "乙手里不该多出那张 7 号");
    assert_eq!(tickets_of(&app, p0), vec![7], "甲那一张仍然是他的");
    assert_eq!(pot_of(&app), price, "池子只进了甲那一注");

    // 同一趟柜台里换一个号：买得成。这一句把上面三条从「乙被挡在门外」里分出来
    buy(&mut app, p1, 8);
    assert_eq!(tickets_of(&app, p1), vec![8], "8 号没人占，乙该买得到");
    assert_eq!(cash_of(&mut app, p1), before - price);
    assert_eq!(pot_of(&app), 2 * price);
    assert_pot_audit(&mut app, 0);
}

/// 独占是「还赢得到的人之间」的独占：破产的人兑不出钱，他手里那张旧凭证不该替全场
/// 锁死一个号码 —— 否则他死后那个号就永远买不成，池子也就永远开不出去。
///
/// 直插 `Bankrupt` 组件而不走完整清算（与 `company.rs` 那条同一手法）：这一条验的只是
/// `taken` 那道筛子，而清算要扯上拍卖机与回合终局，会把要问的那句话淹掉。
#[test]
fn a_bankrupt_player_does_not_hold_a_number_hostage() {
    let (mut app, p0, p1) = three_at_counter();
    let price = rules().ticket_price;
    buy(&mut app, p0, 7);
    leave_the_counter(&mut app);
    insert(&mut app, p0, Bankrupt);

    back_to_the_counter(&mut app, p1);
    buy(&mut app, p1, 7);

    assert_eq!(
        tickets_of(&app, p1),
        vec![7],
        "死者占着的号该重新能买：{:?}",
        ledger(&app)
    );
    assert_eq!(
        pot_of(&app),
        2 * price,
        "甲那一注的钱早就进了池，不随他退场"
    );
    assert_pot_audit(&mut app, 0);
}

/// 钱不够时一分不扣：`debit` 那条「不够就不扣」在这里就是「券也不发、池子也不进」。
#[test]
fn a_player_who_cannot_pay_gets_nothing_for_free() {
    let (mut app, p0, _p1) = at_counter();
    let price = rules().ticket_price;
    set_cash(&mut app, p0, price - 1);

    buy(&mut app, p0, 7);

    assert_eq!(cash_of(&mut app, p0), price - 1, "差一块钱也不能赊");
    assert!(tickets_of(&app, p0).is_empty());
    assert_eq!(pot_of(&app), 0);
}

/// **中奖独得奖池**（PLAN 验收第二句），外加「一期一结」：没中的那些券一起作废。
#[test]
fn the_winner_takes_the_whole_pot_and_every_ticket_is_void() {
    let seed = GameSeed::default();
    let winning = draw_number(seed, 36, 1);
    let losing = if winning == 1 { 2 } else { winning - 1 };
    let price = rules().ticket_price;
    let (mut app, p0, p1) = at_the_counter(seed, rules());

    buy(&mut app, p0, winning);
    leave_the_counter(&mut app);
    // 乙也买了一注，但他买的不是中奖号 —— 他那一张要在开奖那一刻一起作废
    back_to_the_counter(&mut app, p1);
    buy(&mut app, p1, losing);
    leave_the_counter(&mut app);
    assert_eq!(pot_of(&app), 2 * price);

    // 走到本月 15 号。这里不等「第 14 回合」而等第 14 次 tick，也不掐开奖前后的钱包差值：
    // 有人被关着时那一回合不用掷骰就被跳过，一次 `play_turns` 能跨两个回合末，
    // 差值会掐到开奖之后的日子上 —— 到账那一句改用他**最后一条现金流水**来问（见下）。
    play_until_tick(&mut app, 14);

    assert_eq!(
        last_draw_of(&app),
        Some(winning),
        "这个月开的就该是他手里那个号"
    );
    assert_eq!(pot_of(&app), 0, "一个人持有中奖号 → 整池归他");
    assert!(
        paid_him(&app, p0, 2 * price),
        "「奖金到账」要有现金流水，不能只有一条凭据：{:?}",
        last_cash_move(&app, p0)
    );
    assert!(
        tickets_of(&app, p0).is_empty(),
        "赢家的券也作废（这一期结完了）"
    );
    assert!(
        tickets_of(&app, p1).is_empty(),
        "没中的人也一起清 —— 留着就是白送下一期"
    );
    let led = ledger(&app);
    assert!(
        led.iter()
            .any(|l| l == &format!("Draw({winning}号,P0,池{})", 2 * price)),
        "开奖那条流水要说得出号、谁独得、池中原有多少：{led:?}"
    );
    assert!(
        led.iter()
            .any(|l| l == &format!("Win(P0:{winning}号,+{})", 2 * price)),
        "中奖该有一条凭据：{led:?}"
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Win(P1")),
        "拿着输号的人不该有中奖凭据：{led:?}"
    );
    assert_pot_audit(&mut app, 0);
    assert_cash_audit(&mut app, 20_000);
}

/// **独占就是「独得」的来路**（10-26 实报）：两个人抢同一个中奖号时第二个人买不成，于是
/// 开奖那一刻在结构上只可能有一位持有者 —— 整池归他，连对手那张陪跑号的钱也一起归他。
///
/// P8 在这一格写的是「整数均分、`ticket_price` 配成 999 专门走到余数那一支」。独占之后
/// 那个分支不是「不必再测」，而是**写不出来**：`holders` 换成 `find`，池子整个清零，
/// 没有「每人多少」也没有「余数留给谁」。这里留着两个号、两笔钱，就是为了证明
/// 那个形状真的消失了 —— 池子空得干干净净，而钱只进一个人的钱包。
#[test]
fn only_one_player_can_hold_the_winning_number() {
    let seed = GameSeed::default();
    let price = rules().ticket_price;
    let winning = draw_number(seed, 36, 1);
    let spare = if winning == 36 { 1 } else { winning + 1 };
    let (mut app, p0, p1) = at_the_counter(seed, rules());

    buy(&mut app, p0, winning);
    leave_the_counter(&mut app);
    back_to_the_counter(&mut app, p1);
    let before = cash_of(&mut app, p1);
    buy(&mut app, p1, winning);
    assert_eq!(
        cash_of(&mut app, p1),
        before,
        "抢同一个号的那一笔不该扣钱：{:?}",
        ledger(&app)
    );
    assert!(tickets_of(&app, p1).is_empty(), "乙手里没多出那张中奖号");
    // 换个号照买 —— 这一句把「被独占挡住」与「这一趟他什么都买不成」分清楚
    buy(&mut app, p1, spare);
    assert_eq!(
        cash_of(&mut app, p1),
        before - price,
        "陪跑那一张是买成了的"
    );
    leave_the_counter(&mut app);
    assert_eq!(pot_of(&app), 2 * price, "池里是甲的中奖号 + 乙的陪跑号");

    play_until_tick(&mut app, 14);
    assert_eq!(today(&app), Date::new(2048, 1, 15), "前置的日子算错了");

    let led = ledger(&app);
    assert!(
        led.iter()
            .any(|l| l == &format!("Draw({winning}号,P0,池{})", 2 * price)),
        "开奖那一条写的是「谁独得、池里多少」，不再是人数与每人多少：{led:?}"
    );
    assert!(
        paid_him(&app, p0, 2 * price),
        "整池（含乙那张陪跑号的钱）该进甲一个人的钱包：{:?}",
        last_cash_move(&app, p0)
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Win(P1")),
        "乙手里那个号不是中奖号，不该有中奖凭据：{led:?}"
    );
    assert_eq!(pot_of(&app), 0, "独得之后池子是空的 —— 没有余数可留");
    assert!(tickets_of(&app, p0).is_empty() && tickets_of(&app, p1).is_empty());
    assert_pot_audit(&mut app, 0);
}

/// 占号的就是**券**，不是另立的一张「号码 → 人」登记表。所以一期一结把券清掉的那一刻，
/// 那个号重新能买。
///
/// 这一条钉的是「两处真相只有一份」：规则层若另存一张登记，清券那一步就会漏了它，
/// 症状是奖池都发完了、面板上那个号还写着「已被 甲 买下」而谁也买不成。
#[test]
fn a_taken_number_is_free_again_after_the_period_closes() {
    let seed = GameSeed::default();
    let price = rules().ticket_price;
    let winning = draw_number(seed, 36, 1);
    let (mut app, p0, p1) = at_the_counter(seed, rules());

    buy(&mut app, p0, winning);
    leave_the_counter(&mut app);
    back_to_the_counter(&mut app, p1);
    buy(&mut app, p1, winning);
    assert!(
        tickets_of(&app, p1).is_empty(),
        "前置：这个号此刻被甲占着，乙该买不成"
    );
    leave_the_counter(&mut app);

    play_until_tick(&mut app, 14);
    assert!(
        tickets_of(&app, p0).is_empty(),
        "前置：开奖把甲那张券清掉了（一期一结）"
    );

    back_to_the_counter(&mut app, p1);
    buy(&mut app, p1, winning);
    assert_eq!(
        tickets_of(&app, p1),
        vec![winning],
        "换了期，同一个号该重新能买：{:?}",
        ledger(&app)
    );
    assert_eq!(pot_of(&app), price, "新一期的池子从零开始，只这一注");
    assert_pot_audit(&mut app, 0);
}

/// **跨月累积正确**（PLAN 验收第一句）：这个月没人猜中 → 池子与券都留下；月末照样结息；
/// 下个月开出他手里那个号时，发的就是滚存后的那个数。
#[test]
fn an_unwon_month_rolls_the_pot_and_it_earns_interest() {
    let (seed, first, second) = seed_two_draws();
    let rules = rules();
    let price = rules.ticket_price;
    let (mut app, p0, _p1) = at_the_counter(seed, rules);
    // 手里这一张是**下个月**的号，所以这个月一定流局
    buy(&mut app, p0, second);
    leave_the_counter(&mut app);

    // 这个月的 15 号
    play_until_tick(&mut app, 14);
    assert_eq!(today(&app), Date::new(2048, 1, 15));
    assert_eq!(last_draw_of(&app), Some(first), "这个月开的是第一个号");
    assert_eq!(pot_of(&app), price, "没人猜中 → 池子一分不动");
    assert_eq!(
        tickets_of(&app, p0),
        vec![second],
        "号码留到下月（PLAN §三 那句）"
    );
    let led = ledger(&app);
    assert!(
        led.iter()
            .any(|l| l == &format!("Draw({first}号,没人,池{price})")),
        "流局也要有一条开奖流水（写「没人」而不是 0 人：这一行说的是没有一个人独得）：{led:?}"
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Win(")),
        "没人猜中就不该有中奖凭据：{led:?}"
    );

    // 月末：1 月 31 日 = 第 30 次 tick。空转的池子照吃一次月息（5% × 1000 = 50）
    play_until_tick(&mut app, 30);
    assert_eq!(today(&app), Date::new(2048, 1, 31), "1 月有 31 天");
    assert_eq!(
        pot_of(&app),
        price + price / 20,
        "流局的池子月末照样吃息：{:?}",
        ledger(&app)
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l == &format!("Pot(月息+{}→{})", price / 20, price + price / 20)),
        "结息那一步也要留流水：{:?}",
        ledger(&app)
    );
    assert_eq!(
        tickets_of(&app, p0),
        vec![second],
        "结息不动券，也不许偷偷把券清掉"
    );

    // 下个月 15 号 = 第 45 次 tick（1 月有 31 天 → tick 31 是 2 月 1 日）
    play_until_tick(&mut app, 45);
    assert_eq!(
        today(&app),
        Date::new(2048, 2, 15),
        "2 月 15 日该是第二次开奖"
    );
    assert_eq!(last_draw_of(&app), Some(second));
    assert_eq!(pot_of(&app), 0, "两个月的累积一次发完");
    assert!(
        paid_him(&app, p0, price + price / 20),
        "独得滚存后的整池（票价 + 月息）那一条到账流水没找到：{:?}",
        last_cash_move(&app, p0)
    );
    assert!(tickets_of(&app, p0).is_empty());
    assert_pot_audit(&mut app, 0);
}

/// 空池子开奖：不发钱、不发中奖凭据，也不许写一条 0 元的池子流水
/// （与「0 点券不发 `PointsChanged`」同一条纪律）。
#[test]
fn a_draw_with_no_tickets_says_nothing_and_costs_nothing() {
    let seed = GameSeed::default();
    let (mut app, _p0, _p1) = at_the_counter(seed, rules());
    leave_the_counter(&mut app);

    play_until_tick(&mut app, 14);
    assert_eq!(today(&app), Date::new(2048, 1, 15), "日子该走到开奖那天");

    assert_eq!(pot_of(&app), 0);
    assert_eq!(last_draw_of(&app), Some(draw_number(seed, 36, 1)));
    let led = ledger(&app);
    assert!(
        led.iter().any(|l| l.starts_with("Draw(")),
        "开过奖这件事总得看得见：{led:?}"
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Pot(")),
        "池子没动就不该有流水：{led:?}"
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Win(")),
        "没人中奖就不该有凭据：{led:?}"
    );
    assert_pot_audit(&mut app, 0);
    assert_cash_audit(&mut app, 20_000);
}

/// 上一局的池子不该跟到下一局：`begin_game_system` 的重置清单里必须有它
/// （跨局残留那一族缺陷的根，见 ARCHITECTURE §6.1 坑 22/25 —— 与点券箱、货架同一条）。
///
/// 走的是 `tests/coupon.rs` 那套「只重装 core 这一侧」的重开手法。
#[test]
fn a_rebooted_world_starts_with_an_empty_pot() {
    let (mut app, p0, _p1) = at_counter();
    buy(&mut app, p0, 7);
    assert!(pot_of(&app) > 0, "前置：刚往池子里放了钱");

    app.world_mut().remove_resource::<ActiveBoard>();
    app.world_mut()
        .resource_mut::<NextState<TurnPhase>>()
        .set(TurnPhase::Boot);
    assert!(run_until_world(&mut app), "世界没能重装");

    let pot = app.world().resource::<Jackpot>();
    assert_eq!(pot.pot, 0, "新局开局即空池");
    assert_eq!(pot.last_draw, None, "「上一期开几号」不是这一局的事实");
}
