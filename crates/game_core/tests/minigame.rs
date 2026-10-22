//! 游乐场（P7）的 App 级用例：真踩那一格、真把成绩报上去。
//!
//! 这一层要守的四件事，都是「窗口与账」而不是玩法本身（玩法在表现层，规则层刻意不知道
//! 气球怎么飞）：
//! - **窗口**：踩上去才开着，只有窗口前那个人报的成绩算数，报了成绩窗口就关上、回合交还。
//! - **那张表是唯一的价格来源**：断言一律读世界里的 `GameRules::minigames`，不在测试里
//!   重抄一遍数字 —— 抄了就等于「表改了用例还绿」。
//! - **两套账**：点券进账，现金一条流水都不动（PLAN §一「永不通兑」）。
//! - **`awarded` 那一格真的有牙**：报 `false` 时凭据照记、点券一分不发。这是 PLAN §四 P7
//!   点名要的显式开关（「不靠主菜单不发消息隐式实现」），所以它必须能在**有窗口**的场合
//!   被翻过来 —— 只有练习局会用到它，而练习局不该是唯一能测到它的场合。
//!
//! prototype 上没有游乐场格（真棋盘归 P10），所以每个用例用 `arcade_tile` 把 8 号（机会）
//! 就地改成游乐场 —— 与 `coupon_tile`、`retile` 同一手法：棋盘是资源里的一份快照。
//! 演示盘上那一格由 `board.rs::prototype_carries_an_arcade_the_player_can_step_on` 钉住。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::ecs::message::Messages;
use common::*;
use game_core::prelude::{
    Decision, DecisionChosen, GameKind, GameRules, ItemKind, Payout, Pending, TurnPhase,
};

/// 用例里当游乐场使的那一格（prototype 的 8 号＝机会）。7 号掷 1 点即踩到。
const ARCADE: u32 = 8;

/// 走到「游乐场窗口该开着」的那一帧。`game` 为 `None` = 数据里那个「这一趟随机一款」的缺省。
///
/// 用 `Hold` 策略：自动应答会替玩家报一份成绩，窗口当场就关了，什么都观察不到
/// （与百货柜台、保释窗口同一条前置）。
fn at_arcade(game: Option<GameKind>) -> (App, Entity) {
    let (mut app, p0) = ready_on(ARCADE - 1, 1, DecisionPolicy::Hold);
    arcade_tile(&mut app, ARCADE as usize, game);
    roll(&mut app, p0);
    settle(&mut app, 8);
    (app, p0)
}

/// 窗口开着的前置：踩上去、并且断言它真的开了（前置不成立时后面每条断言都会变成假绿）。
fn open_arcade(game: GameKind) -> (App, Entity) {
    let (app, p0) = at_arcade(Some(game));
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Minigame(game))),
        "前置没做到：踩到游乐场却没开出窗口"
    );
    (app, p0)
}

/// 表里那一档。**读世界里的规则**，不在测试里重抄数字。
fn payout_of(app: &App, game: GameKind) -> Payout {
    app.world().resource::<GameRules>().minigames.of(game)
}

fn score_lines(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|line| line.starts_with("Arcade("))
        .collect()
}

fn point_lines(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|line| line.starts_with("Points("))
        .collect()
}

// ===== 踩上去开窗口 =====

#[test]
fn stepping_on_the_arcade_parks_the_turn_on_that_game() {
    let (app, p0) = open_arcade(GameKind::CatchTreasure);
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "游乐场是个窗口：回合要停在这儿等成绩"
    );
    // 格子上写死了哪一款，窗口里就是哪一款（不是「随便挑一个」）
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Minigame(GameKind::CatchTreasure)))
    );
}

/// 「这一趟随机一款」那条路：格子上没写 `data.game` 时由小游戏子流抽。
///
/// 断言写的是**跨种子会变**而不是「等于某一款」：一颗种子上「抽到打气球」与「写死打气球」
/// 长得一模一样，只有换几颗种子才分得出这两件事（`nth_seed_rolling_first` 就是为这条留的）。
#[test]
fn an_unpinned_tile_deals_a_game_and_the_deal_follows_the_seed() {
    let mut seen: Vec<GameKind> = Vec::new();
    for n in 0..8usize {
        let seed = nth_seed_rolling_first(1, n);
        let (mut app, p0) = ready_on_seeded(ARCADE - 1, DecisionPolicy::Hold, seed);
        arcade_tile(&mut app, ARCADE as usize, None);
        roll(&mut app, p0);
        settle(&mut app, 8);
        let Some((_, Pending::Minigame(game))) = pending_of(&app) else {
            panic!("第 {n} 颗种子上没开出游乐场窗口：{:?}", pending_of(&app));
        };
        assert!(
            GameKind::ALL.contains(&game),
            "抽出来的不是三种之一：{game:?}"
        );
        if !seen.contains(&game) {
            seen.push(game);
        }
    }
    assert!(
        seen.len() >= 2,
        "八颗种子抽出来全是一样的，那「随机一款」其实是写死的：{seen:?}"
    );
}

/// 写死了就照写的玩：与上一条对着看，两条一起才说明「随机」不是「数据没读到时的兜底」。
#[test]
fn a_pinned_tile_always_deals_what_the_data_says() {
    for game in GameKind::ALL {
        let (app, p0) = at_arcade(Some(game));
        assert_eq!(
            pending_of(&app),
            Some((p0, Pending::Minigame(game))),
            "{}：格子上写死了却开出别的游戏",
            game.label()
        );
    }
}

/// 每一趟都要发一手新的布局种子：挖宝埋在哪几格由它决定，两趟同一手就等于
/// 「第二次进游乐场，宝还在原来那几格」。
#[test]
fn two_visits_get_two_different_deals() {
    let (mut app, p0) = open_arcade(GameKind::PenguinDig);
    let first = deal_of(&app);
    report_score(&mut app, p0, GameKind::PenguinDig, 1);
    settle(&mut app, 8);
    assert!(pending_of(&app).is_none(), "报了成绩窗口就该关上");

    // 再走一趟。用遥控骰子把那一步定住：`seed_rolling_first` 只保证**第一掷**是 1，
    // 而 `rewind_dice` 会把整条流退回去 —— 连小游戏那条一起退，于是「两趟两手」就变成
    // 在测「同一条流重放两次」，什么也证明不了（那一格正是这条用例的牙）
    place(&mut app, p0, ARCADE - 1);
    make_current(&mut app, p0);
    grant_item(&mut app, p0, ItemKind::RemoteDice, 1);
    use_remote_dice(&mut app, p0, 1);
    settle(&mut app, 8);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Minigame(GameKind::PenguinDig))),
        "第二次踩上去该再开一局 —— 游乐场不是点券箱，没有「一次性」那条规则"
    );
    assert_ne!(deal_of(&app), first, "两趟发下来是同一手布局");
}

// ===== 成绩换点券 =====

#[test]
fn a_score_is_paid_by_the_table_and_by_nothing_else() {
    let (mut app, p0) = open_arcade(GameKind::PopBalloons);
    let payout = payout_of(&app, GameKind::PopBalloons);
    let cash_before = cash_of(&mut app, p0);
    let points_before = points_of(&app, p0);

    report_score(&mut app, p0, GameKind::PopBalloons, 7);
    settle(&mut app, 6);

    assert_eq!(
        points_of(&app, p0),
        points_before + payout.points(7),
        "7 分该按表换成 {} 点券",
        payout.points(7)
    );
    assert_eq!(
        cash_of(&mut app, p0),
        cash_before,
        "点券是另一套账：一分现金都不许动"
    );
    assert_eq!(
        score_lines(&app),
        vec![format!("Arcade(P0·打气球·7分→{}点券)", payout.points(7))],
        "凭据要读得出「哪一款、几分、换了几点券」"
    );
    // 两条审计各查一半：现金那一条查「没有凭空多出一笔钱」，点券那一条查「余额 == Σ流水」
    assert_cash_audit(&mut app, 20_000);
    assert_points_audit(&mut app, 0);
}

/// 满分是夹子而不是终点。打出三倍满分仍然只拿上限那一笔 —— 不夹的话
/// 「一个手快的人能刷出多少点券」就成了一个没有上界的数，而点券要给保释恶人（300）定价。
#[test]
fn the_cap_bites_past_perfect() {
    let (mut app, p0) = open_arcade(GameKind::CatchTreasure);
    let payout = payout_of(&app, GameKind::CatchTreasure);

    report_score(
        &mut app,
        p0,
        GameKind::CatchTreasure,
        payout.perfect * 3 + 5,
    );
    settle(&mut app, 6);

    assert_eq!(points_of(&app, p0), payout.best());
    assert_eq!(
        payout_of(&app, GameKind::CatchTreasure).points(payout.perfect),
        payout.best(),
        "满分那一档就该是上限"
    );
}

/// 0 分不发点券流水：与春节红包「0 元不发现金流水」同一条 —— 一条 `+0` 的流水
/// 会让审计里多出一行什么都没说的事。
#[test]
fn a_zero_score_leaves_no_points_trail() {
    let (mut app, p0) = open_arcade(GameKind::PenguinDig);
    report_score(&mut app, p0, GameKind::PenguinDig, 0);
    settle(&mut app, 6);
    assert_eq!(points_of(&app, p0), 0);
    assert!(
        point_lines(&app).is_empty(),
        "0 分不该留下点券流水：{:?}",
        point_lines(&app)
    );
    assert_eq!(
        score_lines(&app).len(),
        1,
        "凭据照记 —— 「玩了个 0 分」与「没玩」在流水上是两件事"
    );
}

// ===== `awarded` 那一格 =====

/// PLAN §四 P7 点名的显式开关：`awarded: false` 时凭据照记、点券一分不发。
/// 用例特意在**窗口开着**的场合翻它 —— 练习局没有窗口，而「没有窗口」不该是这个开关
/// 唯一能被按到的场合（那正是「靠不发消息隐式实现」的形状）。
#[test]
fn an_unawarded_score_is_recorded_but_pays_nothing() {
    let (mut app, p0) = open_arcade(GameKind::PopBalloons);
    report_score_awarded(&mut app, p0, GameKind::PopBalloons, 9, false);
    settle(&mut app, 6);

    assert_eq!(points_of(&app, p0), 0, "练习局不给点券");
    assert!(
        point_lines(&app).is_empty(),
        "一分没发就不该有点券流水：{:?}",
        point_lines(&app)
    );
    assert_eq!(
        score_lines(&app),
        vec!["Arcade(P0·打气球·9分→练习不计)".to_string()],
        "凭据要说得出「这一局不计」，而不是安静地什么都不留"
    );
    assert!(
        pending_of(&app).is_none(),
        "报了成绩就该交还回合，哪怕这一局不计点券"
    );
}

// ===== 窗口的那几道门 =====

/// 别人报的成绩不算：窗口挂在谁身上，就只有谁的成绩能换成点券。
/// 与百货柜台「别人的点击不能替他买东西」同一条纪律。
#[test]
fn a_score_from_someone_not_at_the_window_is_refused() {
    let (mut app, p0) = open_arcade(GameKind::PopBalloons);
    let foe = roster(&mut app)
        .into_iter()
        .map(|(e, ..)| e)
        .find(|e| *e != p0)
        .expect("演示盘上至少有两个人");

    report_score(&mut app, foe, GameKind::PopBalloons, 12);
    settle(&mut app, 6);

    assert_eq!(points_of(&app, foe), 0, "不在窗口前的人报成绩不该拿到点券");
    assert!(
        score_lines(&app).is_empty(),
        "被拒的那一份连凭据都不该有：{:?}",
        score_lines(&app)
    );
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Minigame(GameKind::PopBalloons))),
        "窗口不该被别人的成绩关掉"
    );
}

/// 报错了款（窗口开的是打气球，报上来的是挖宝）也不认：一款一价，
/// 否则「玩便宜的那一款、按贵的那一款结账」就成了漏洞。
#[test]
fn a_score_for_another_game_is_refused() {
    let (mut app, p0) = open_arcade(GameKind::PopBalloons);
    report_score(&mut app, p0, GameKind::PenguinDig, 6);
    settle(&mut app, 6);
    assert_eq!(points_of(&app, p0), 0);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Minigame(GameKind::PopBalloons))),
        "窗口还开着：他没玩那一局，不该被算成玩过了"
    );
}

/// 空手离开（不玩、或玩了不报成绩）：窗口关掉、一分不发、**也没有凭据**。
///
/// 这一条与 `an_unawarded_score_is_recorded_but_pays_nothing` 的区别就是它存在的全部理由：
/// 同样是「没拿到点券」，一个是「玩了、这一局不计」，一个是「压根没报成绩」。
/// 两者的流水必须不同，否则玩家看不出自己那一局到底算没算。
#[test]
fn walking_away_empty_handed_closes_the_window_silently() {
    let (mut app, p0) = open_arcade(GameKind::CatchTreasure);
    app.world_mut()
        .resource_mut::<Messages<DecisionChosen>>()
        .write(DecisionChosen {
            player: p0,
            action: Decision::Acknowledge,
        });
    settle(&mut app, 6);

    assert!(pending_of(&app).is_none(), "「不玩了」要能把窗口关掉");
    assert_eq!(points_of(&app, p0), 0);
    assert!(
        score_lines(&app).is_empty(),
        "没报成绩就不该有成绩凭据：{:?}",
        score_lines(&app)
    );
}

/// 报价式的回答在这个窗口上不认：`Accept`/`Decline` 对不上面板任何一颗按钮，
/// 认了就成了「一条消息既结账又关窗口」（`landing::decision_system` 里那一格的注释）。
#[test]
fn an_offer_style_answer_does_not_close_the_window() {
    for action in [Decision::Accept, Decision::Decline] {
        let (mut app, p0) = open_arcade(GameKind::PenguinDig);
        app.world_mut()
            .resource_mut::<Messages<DecisionChosen>>()
            .write(DecisionChosen { player: p0, action });
        settle(&mut app, 6);
        assert_eq!(
            pending_of(&app),
            Some((p0, Pending::Minigame(GameKind::PenguinDig))),
            "{action:?} 把游乐场窗口关掉了"
        );
    }
}
