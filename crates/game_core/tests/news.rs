//! P11 新闻中心的验收：踩到那一格就**自动播一条**，而「挨打的通常不是踩格的人」。
//!
//! 三条验收的落点：
//! - **①「踩格者不挨打」** → [`a_news_that_names_somebody_else_never_touches_the_broadcaster`]：
//!   一条点名别人的新闻，触发者金额不变、被点名者金额变。播报者与承受者在凭据里是
//!   **两个主语**（`NewsFired::broadcaster` vs `NewsHit::player`），这条断言才有得问。
//!   边界另有 [`the_richest_pick_reads_wallets_not_seats`] 钉：「点名最富那位」读的是钱，
//!   播报者自己最富时那一刀就落他头上 —— 「挨打的不是踩格的人」说的是**点名别人**那一档。
//! - **②「全场类新闻影响到的每个人都留流水」** → [`a_whole_field_news_moves_every_wallet`]。
//! - **③「路段类新闻到期自动失效」** → 那条要 `NewsScope` 长出「一片地」那一档，归 **P11b**。
//!
//! 抽取算法（✅ 10-20 拍板：全池随机 + 同场不连重）在这里问的是**世界**而不是纯函数：
//! [`no_two_broadcasts_in_a_row_are_the_same`] 与 [`another_seed_broadcasts_another_show`]。
//! `news.rs` 的 `draw`/`roll_span` 单测钉的是映射本身，那两条红了说明算错了；
//! 这两条红了说明有人绕过它自己抽了一条。

mod common;

use bevy::app::App;
use bevy::asset::Assets;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    ActiveBoard, Adjustment, CardEffect, ChargeDomain, Correction, DataHandles, GameSeed,
    GameSession, ItemKind, LandStatus, NewsId, NewsSet, Scope, Source, TileKind, TurnPhase,
};

/// 演示盘上的新闻中心（`prototype.json` 的 13 号格，`(0,4)`）。
/// `board.rs::diagonal_segments_count_is_three` 与 `tools/validate_board.py` 钉着同一个数。
const NEWS: u32 = 13;
/// 起始现金（`GameRules::default`），现金审计的基准。
const START: i64 = 20_000;

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

/// 流水里那几条新闻（`News(新闻id,播P?,P?[..]|P?[..])`）。累计取整条而不是最后一条，
/// 于是「中间少播一次」与「最后播错一条」是两种不同的红法。
fn news_lines(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|l| l.starts_with("News("))
        .collect()
}

/// 一条新闻流水的前三段：`(id, 播报者, 「被点到的人」那一整段原文)`。
///
/// `splitn(3, …)` 那一位是有讲究的 —— 效果凭据的 `Debug` 里可能带逗号
/// （`Detained { place, turns, junction }`），整条切开会把一句话拆成三句。
/// 拆流水而不是往夹具里再塞一个 reader：断言要问的三件事（谁播、谁挨、挨了多少）
/// 在这条流水的形状里都读得出来。
fn parts(line: &str) -> (String, String, String) {
    let body = line
        .trim_start_matches("News(")
        .trim_end_matches(')')
        .trim();
    let mut it = body.splitn(3, ',');
    (
        it.next().unwrap_or_default().to_string(),
        it.next()
            .unwrap_or_default()
            .strip_prefix("播")
            .unwrap_or("")
            .to_string(),
        it.next().unwrap_or_default().to_string(),
    )
}

/// 「被点到的人」那一段里，各被点到的用户名（`P0[P..]|P1[P..]` → `["P0", "P1"]`）。
fn named(hits: &str) -> Vec<String> {
    hits.split('|')
        .map(|seg| seg.split('[').next().unwrap_or_default().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// 这一条是**地档**（第三段以 `地` 开头：`地L3[L2/L5][Razed { … }]`）而不是人档。
///
/// 金额类断言一律先筛掉地档那几条：它们没有钱，而「一条新闻播出去、里面没有钱」正是
/// P11b① 新增的那一档的形状。不筛的话红的是「补贴的数不对」，而真正该说的是「这条不归钱管」。
fn is_ground(line: &str) -> bool {
    parts(line).2.starts_with('地')
}

/// 一串流水里的人档那几条。
fn people_only(lines: &[String]) -> Vec<String> {
    lines.iter().filter(|l| !is_ground(l)).cloned().collect()
}

/// 地档那一段里的中心与清单：`地L3[L2/L5][…]` → `("L3", ["L2","L5"])`。
fn ground(line: &str) -> (String, Vec<String>) {
    let body = parts(line).2;
    let at = body
        .strip_prefix('地')
        .unwrap_or_default()
        .split('[')
        .next()
        .unwrap_or_default()
        .to_string();
    let list = body
        .split('[')
        .nth(1)
        .unwrap_or_default()
        .split(']')
        .next()
        .unwrap_or_default()
        .split('/')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    (at, list)
}

/// 一条效果凭据的带符号金额：`Gained(n)` 记正、`Lost(n)` 记负，其余（`CouldNotPay{…}`
/// 那种没有数的）记 0。
fn outcome_net(one: &str) -> i64 {
    let number = |tag: &str| {
        one.strip_prefix(tag)
            .and_then(|r| r.strip_suffix(')'))
            .and_then(|v| v.parse().ok())
    };
    if let Some(v) = number("Gained(") {
        v
    } else {
        -number("Lost(").unwrap_or(0)
    }
}

/// 那一段里 `who` 这一播的净收支（`P1[Gained(1500)+Lost(3000)]` → `-1500`），没被点到是 0。
///
/// **金额一律从流水里读，不写死卡面那个数**：新闻的钱走 P2 那条管线，而 `Bonus`／`Penalty`
/// 两族都随物价指数缩放 —— 一条补贴本身就把全场资产推高，下一播的数只会更大（缩放本身由
/// [`the_carded_amount_is_scaled_by_the_price_index`] 单独钉）。
/// 从流水读还换来一件更硬的事：断言变成「屏幕上写的那个数真的进了钱包」。
/// `assert_cash_audit` 抓不住「凭据写了钱、钱包没动」那种世界（两边都一动没动，照样对得平），
/// 只有这条正面断言抓得住。
fn net_for(hits: &str, who: &str) -> i64 {
    hits.split('|')
        .find(|seg| seg.split('[').next().unwrap_or_default() == who)
        .map(|seg| {
            seg.split('[')
                .nth(1)
                .unwrap_or_default()
                .trim_end_matches(']')
                .split('+')
                .map(outcome_net)
                .sum::<i64>()
        })
        .unwrap_or(0)
}

/// 这一串流水里 `who` 一共收支多少。
fn net_over(lines: &[String], who: &str) -> i64 {
    lines.iter().map(|l| net_for(&parts(l).2, who)).sum()
}

/// 一次正常的踩格：P0 站在 12 号，下一手是 1 点。
fn step_onto_once() -> (App, Entity) {
    ready_on(NEWS - 1, 1, DecisionPolicy::DeclineAll)
}

/// 让 P0 反复踩同一格新闻中心 `times` 次，返回每一播之后的全部流水。
///
/// **这里刻意不用 `rewind_dice`**（`tests/fate_order.rs` 的 `step_on_fate` 用的是它）：
/// 那一句把整条随机流退回开局，而「同场不连重」读的是「这一局播过哪一条」—— 流一退，
/// 每一轮都从同一个位置重抽同一个数（`last_news` 只负责把那个数翻成别的下标），
/// 于是验出来的是夹具的形状而不是世界的形状。改用 P5 的遥控骰子定步数：不动任何随机流，
/// 每一轮真的往前走一格、抽签也就真的往下走一位。每轮先发一颗道具，那是真路径
/// （`turn.rs::remote_dice_system`），不是绕过判据。
///
/// `clear_occupancies` 是前置而不是清洁癖：开局随机撒宝箱、每回合末还会补刷神，而一位神
/// 站在 13 号格上时踩上去的是**附身**不是新闻，那一笔钱的来路就说不清了。
fn walk_onto_news(app: &mut App, p0: Entity, times: usize) -> Vec<String> {
    step_onto_news(app, p0, times, None)
}

/// 踩 `times` 次，只留**人档**那几条，且每轮把物价指数钉在 `index`。
///
/// 金额类用例全走这一条，理由有两句：① P11b① 之后池子里有了地档（拆房、盖状态），
/// 它们身上没有钱，混进「补贴的数对不对」的循环里只会红在错误的那一行上；② 钉住指数
/// 之后「卡面那个数 == 钱包那个数」才说得出。`times` 给到 12 是因为池子已经六条 ——
/// 十二次里三种人档各得撞几次，那几条用例的「前置」断言才有牙（撞不到就是空跑的）。
fn step_onto_money_news(app: &mut App, p0: Entity, times: usize, index: f32) -> Vec<String> {
    people_only(&step_onto_news(app, p0, times, Some(index)))
}

/// 同一件事，但每轮把物价指数摆回 `index`（`Some` 那一条是金额类验收要的：
/// 指数本身会被补贴一路推高，而「卡面 × 指数」那个确切的数只有在钉住之后才说得出）。
fn step_onto_news(app: &mut App, p0: Entity, times: usize, index: Option<f32>) -> Vec<String> {
    for _ in 0..times {
        step_news_once(app, p0, index);
    }
    let lines = news_lines(app);
    assert_eq!(lines.len(), times, "踩 {times} 次就该播 {times} 条");
    lines
}

/// 让 P0 踩**一次**新闻中心（一播一条），不检查累计条数 —— 地档那几条用例要的是
/// 「一直踩到抽中某一条为止」，而累计条数那一断言属于按次数说话的用例。
/// `tile` 那一格是给「换一格当新闻中心」的用例留的（锚点跟着人走，得能把他摆到别处）。
fn step_news_once(app: &mut App, p0: Entity, index: Option<f32>) {
    step_news_on(app, p0, NEWS, index)
}

fn step_news_on(app: &mut App, p0: Entity, tile: u32, index: Option<f32>) {
    clear_occupancies(app);
    make_current(app, p0);
    place(app, p0, tile - 1);
    if let Some(at) = index {
        set_price_index(app, at);
    }
    grant_item(app, p0, ItemKind::RemoteDice, 1);
    let want = turn_count(app) + 1;
    use_remote_dice(app, p0, 1);
    // 先把遥控骰子被受理、走进行走的那几帧让出来：`play_turns` 看见 `WaitRoll`
    // 就会按普通掷骰，同一颗骰子上按两次会多走一步
    settle(app, 2);
    play_turns(app, want);
    assert_eq!(
        tile_of(app, p0),
        tile,
        "这一轮没踩到新闻中心（遥控骰子被拒了？）"
    );
}

/// 一直踩到 `want` 那一条播出来为止，返回那一条流水（最多 `tries` 次），
/// 每轮先跑一次 `rearm` 把夹具摆回「被测那一播该有的样子」。
///
/// 池子是「全池随机 + 同场不连重」，测试要问的是**某一条特定新闻**的行为（地档那几条），
/// 所以不能靠「第 N 次恰好抽到它」—— 那是一条只有代码知道的隐藏状态。撞不到就 panic：
/// 六条池子四十次还抽不出某一条，说明抽取偏了，那是比这条用例本身更响的红。
///
/// **只认 P0 播出去的那一条**：长跑里乙也会走路，她踩到新闻中心同样会抽中 `want`，
/// 而她的锚点、她的现金与这条用例要问的那件事无关 —— 不筛主语，用例就会停在错误的那一播上。
///
/// `rearm` 不是保险，是**必要**：中间那一播可能正是一条天灾（山洪的 3×3 罩得住王府井），
/// 不重摆地块，最后那一播读到的就是上一播留下的残局 —— 症状是「seized 少了 1」，
/// 而它跟被测的那条规则一点关系都没有。
fn walk_until_rearmed(
    app: &mut App,
    p0: Entity,
    tile: u32,
    want: &str,
    tries: usize,
    mut rearm: impl FnMut(&mut App),
) -> String {
    for _ in 0..tries {
        rearm(app);
        step_news_on(app, p0, tile, None);
        if let Some(line) = news_lines(app)
            .into_iter()
            .rev()
            .find(|l| parts(l).0 == want && parts(l).1 == "P0")
        {
            return line;
        }
    }
    panic!("{tries} 次播映里 P0 没抽到 {want}（「全池随机」那条是假的）");
}

/// 只给「不需要任何预置」的用例用的一份新盘 + 一串播映。
/// 要预置现金或修正的用例必须自己 `step_onto_once()` 之后再 [`walk_onto_news`] ——
/// 这一句**新起一个 world**，先前设在别的 app 上的前置一条都带不过来（这条学费见
/// `tests/shop.rs` 那个「点券 0 的拒绝理由」）。
fn step_on_news(times: usize) -> (App, Vec<String>) {
    let (mut app, p0) = step_onto_once();
    let lines = walk_onto_news(&mut app, p0, times);
    (app, lines)
}

/// 那一条流水里的新闻在池子里排第几（`NewsId` 存的就是这个数，不是 id 字符串）。
fn index_of(app: &App, id: &str) -> u32 {
    let set = app
        .world()
        .resource::<Assets<NewsSet>>()
        .get(&app.world().resource::<DataHandles>().news)
        .expect("新闻池没加载");
    set.0
        .iter()
        .position(|n| n.id == id)
        .unwrap_or_else(|| panic!("流水里那条 {id} 不在池子里")) as u32
}

/// 那条新闻卡面上写的那个金额（绝对值）。
///
/// 从资产里读而不是在测试里再抄一遍 —— 「改一行 JSON 该让哪几条用例红」这件事，
/// 只有从数据侧拿数才答得出来。
fn carded(app: &App, id: &str) -> i64 {
    let set = app
        .world()
        .resource::<Assets<NewsSet>>()
        .get(&app.world().resource::<DataHandles>().news)
        .expect("新闻池没加载");
    let def = set
        .0
        .iter()
        .find(|n| n.id == id)
        .expect("流水里那条在池子里");
    let [only] = def.effects.as_slice() else {
        panic!("第一批三条各带一条效果");
    };
    match only {
        CardEffect::AddCash { amount } | CardEffect::DeductCash { amount } => amount.abs(),
        other => panic!("这一条只该动钱，读到 {other:?}"),
    }
}

///
/// 机制阶段做出来的东西如果盘上没有载体，测试全绿也等于没演示过（P3d② 起每一步都在
/// 付的那条学费）。这一条走的是**普通掷骰**：遥控骰子那条路验的是分派与结算，
/// 这一条验的是「12 号掷 1 点真的落在新闻中心」这个棋盘事实。
#[test]
fn the_demo_board_carries_a_news_center_the_player_can_step_on() {
    let (app, _) = step_onto_once();
    let news: Vec<u32> = app
        .world()
        .resource::<ActiveBoard>()
        .def
        .tiles
        .iter()
        .filter(|t| t.kind == TileKind::News)
        .map(|t| t.id)
        .collect();
    assert_eq!(news, vec![NEWS], "演示盘该恰好一格新闻中心");

    let (mut app, p0) = step_onto_once();
    clear_occupancies(&mut app);
    let want = turn_count(&app) + 1;
    play_turns(&mut app, want);
    assert_eq!(tile_of(&app, p0), NEWS, "从 12 号掷 1 点该停在新闻中心");
    assert_eq!(
        news_lines(&app).len(),
        1,
        "踩到那一格就播出去了，不需要任何人再点一下：{:?}",
        news_lines(&app)
    );
}

/// 新闻**不挂窗口**：钱与地当场办完，回合直接收尾。
///
/// 它是 `landing::domain_of` 那张表里唯一一个「结算完连收据都不摆」的域，所以这一条
/// 钉的是那个决定的形状 —— 若哪天有人给它补一个 `Pending::News`，这条会红，而那时
/// 该问的是「要不要听」这件事到底有没有第二个答案。
#[test]
fn a_broadcast_holds_nobody_up() {
    let (mut app, _p0) = step_onto_once();
    clear_occupancies(&mut app);
    let want = turn_count(&app) + 1;
    play_turns(&mut app, want);
    assert!(
        !news_lines(&app).is_empty(),
        "前置：这一趟真播过一条（不然下面两句是空跑的）：{:?}",
        ledger(&app)
    );
    assert_eq!(
        pending_of(&app),
        None,
        "踩到新闻中心不该挂住回合（{:?}）",
        pending_of(&app)
    );
    assert_ne!(phase(&app), TurnPhase::WaitDecision, "没人在等回答");
    assert_cash_audit(&mut app, START);
}

/// 验收①：**一条点名别人的新闻，触发者金额不变、被点名者金额变**。
///
/// 物价指数每轮钉回 1.0（[`step_onto_news`] 的 `index` 那一格）：不钉的话补贴本身会把
/// 指数一路推高，下一播的数就不是数据上那个数了 —— 缩放那条由
/// [`the_carded_amount_is_scaled_by_the_price_index`] 专门问，这里要的是「数额对不对得上数据」。
///
/// 夹具把乙的现金顶到全场最高，于是两种点名在这一份前置下**给出同一个形状**：挨打的都是他。
/// 而那份前置只覆盖了一半 —— 「点名最富那位」读的是**钱**不是座位，播报者自己最富时
/// 那一刀就落在他头上，那是 [`the_richest_pick_reads_wallets_not_seats`] 单独钉的事。
/// 「挨打的不是踩格的人」（PLAN §三 那句）说的是**点名别人**那一档，它在这一条里是结构性的。
#[test]
fn a_news_that_names_somebody_else_never_touches_the_broadcaster() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p1, 90_000);
    let before = (cash_of(&mut app, p0), cash_of(&mut app, p1));
    assert_eq!(before.1, 90_000, "前置：乙确实是全场最高");

    let lines = step_onto_money_news(&mut app, p0, 12, 1.0);
    let mut paid_out = 0;
    for line in &lines {
        let (id, broadcaster, hits) = parts(line);
        let each = carded(&app, &id);
        assert_eq!(broadcaster, "P0", "P0 是唯一被摆到那一格的人：{line}");
        if id == "news_001" {
            // 全场类不含这层意思：他也在「全场」里（那句只在**点名**那一档成立）
            assert!(
                named(&hits).contains(&"P0".to_string()),
                "全场类该点到播报者：{line}"
            );
            assert_eq!(
                net_for(&hits, "P0"),
                each,
                "补贴的数就是数据上那个数：{line}"
            );
        } else {
            let who = named(&hits);
            assert_eq!(
                who,
                vec!["P1".to_string()],
                "这份前置下挨打的只能是乙：{line}"
            );
            if id == "news_003" {
                // 这一档的「别人」是构造出来的（`audience` 先把播报者从候选里摘掉），
                // 所以它与抽到哪条、谁有钱都无关
                assert!(
                    !who.contains(&"P0".to_string()),
                    "随机另一位不许是开广播的人：{line}"
                );
            }
            assert_eq!(net_for(&hits, "P0"), 0, "开广播的人分文不动：{line}");
            assert_eq!(
                net_for(&hits, "P1"),
                -each,
                "挨刀的数就是数据上那个数：{line}"
            );
            paid_out += each;
        }
    }
    assert!(
        paid_out > 0,
        "六条里没有一刀真的砍下去，那「被点名者金额变」是空跑的：{lines:?}"
    );
    // 流水里的数与钱包里的数必须是同一个数。这两句是「金额变」的**下半句**：
    // 上面那些断言读的是凭据，凭据写了钱而钱包没动的话 `assert_cash_audit` 也报不出来
    assert_eq!(
        cash_of(&mut app, p0) - before.0,
        net_over(&lines, "P0"),
        "播报者只领全场补贴，不挨点名的刀：{lines:?}"
    );
    assert_eq!(
        cash_of(&mut app, p1) - before.1,
        net_over(&lines, "P1"),
        "乙挨的每一刀都要在他的现金里数得出来：{lines:?}"
    );
    assert!(
        cash_of(&mut app, p1) < before.1,
        "挨打的那一位金额必须真的变了：{lines:?}"
    );
    assert_cash_audit(&mut app, START);
}

/// 「点名最富那位」点的是**钱**，不是「别人」—— 播报者自己最富时那一刀就落他自己头上。
///
/// 这一条是上一那条的**边界**，也是它的前置没能覆盖的那一半：`richest` 不排除播报者。
/// 把它也写成「排除播报者」会得到一条谁都不会挨打的新闻（全场只有两个人时尤其荒唐），
/// 而 PLAN §三 那句「挨打的不是踩格的人」管的是**点名别人**那一档 ——
/// [`a_news_that_names_somebody_else_never_touches_the_broadcaster`] 钉的就是那一档。
/// 同一份前置里两条各说各的话：`news_002` 砸到自己，`news_003` 永远砸不到自己。
#[test]
fn the_richest_pick_reads_wallets_not_seats() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p0, 90_000);
    let before = (cash_of(&mut app, p0), cash_of(&mut app, p1));

    // 二十四次而不是十二次：这一条要**两种点名都撞到**（`richest` 砸自己、`random_other`
    // 砸别人），而 6 条池子里 12 次播映只回来 7 条人档 —— 下面那句「各得撞一次」的
    // 前置断言在 12 次时真的红过一次，这不是保险，是它该有的牙
    let lines = step_onto_money_news(&mut app, p0, 24, 1.0);
    let mut taxed = 0;
    let mut other = 0;
    for line in &lines {
        let (id, _broadcaster, hits) = parts(line);
        let who = named(&hits);
        match id.as_str() {
            "news_002" => {
                assert_eq!(
                    who,
                    vec!["P0".to_string()],
                    "最富的那一位就是开广播的他，稽查该砸到他头上：{line}"
                );
                taxed += carded(&app, "news_002");
            }
            "news_003" => {
                assert_eq!(
                    who,
                    vec!["P1".to_string()],
                    "同一个人最富，随机另一位也不能是他：{line}"
                );
                other += carded(&app, "news_003");
            }
            _ => assert_eq!(who.len(), 2, "全场类点到两个人：{line}"),
        }
    }
    assert!(
        taxed > 0 && other > 0,
        "这二十四次里那两种点名各得撞一次，否则这两句是空跑的：{lines:?}"
    );
    assert_eq!(
        cash_of(&mut app, p0) - before.0,
        net_over(&lines, "P0"),
        "播报者挨的稽查要真的在他钱包里：{lines:?}"
    );
    assert_eq!(
        cash_of(&mut app, p1) - before.1,
        net_over(&lines, "P1"),
        "乙只领补贴：{lines:?}"
    );
    assert_cash_audit(&mut app, START);
}

/// 验收②：**全场类新闻影响到的每个人都留流水**。
///
/// 这一格特意不看「播了几条」而看「同一条里几个人、每人那一笔各是多少」——
/// 新闻与卡的分界就在这里：一张卡打一个人，一条新闻可以同时打全场。
#[test]
fn a_whole_field_news_moves_every_wallet() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    let before = (cash_of(&mut app, p0), cash_of(&mut app, p1));
    let subsidy = carded(&app, "news_001");

    let lines = step_onto_money_news(&mut app, p0, 12, 1.0);
    let field: Vec<&String> = lines.iter().filter(|l| l.contains("news_001")).collect();
    assert!(
        !field.is_empty(),
        "十次里一条全场补贴都没撞到，那「全池随机」是假的：{lines:?}"
    );
    for line in &field {
        let (_id, _broadcaster, hits) = parts(line);
        assert_eq!(named(&hits).len(), 2, "两人盘里全场类该点到两个人：{line}");
        for who in ["P0", "P1"] {
            assert_eq!(
                net_for(&hits, who),
                subsidy,
                "每个人那一笔都得单独看得见（{who} 那一份没在流水里）：{line}"
            );
        }
    }
    // 每个人的钱包都跟着动了，动的数就是流水里那几个数
    let only_field: Vec<String> = field.iter().map(|l| (*l).clone()).collect();
    for (who, entity, start) in [("P0", p0, before.0), ("P1", p1, before.1)] {
        assert_eq!(
            net_over(&only_field, who),
            subsidy * field.len() as i64,
            "{who} 在每一条全场类里都得有一笔自己的流水：{lines:?}"
        );
        assert_eq!(
            cash_of(&mut app, entity) - start,
            net_over(&lines, who),
            "{who} 的钱包没跟着流水走：{lines:?}"
        );
    }
    assert!(subsidy * field.len() as i64 > 0, "补贴不是一笔 0 元流水");
    assert_cash_audit(&mut app, START);
}

/// 新闻的钱吃**物价指数**，不吃另一套账。
///
/// 这一条是「效果动词全部复用 P2 那套」那句话的**金额**版本：前面几条只验了「钱动了、
/// 流水对得上」，而「动了多少」是另一件事 —— 有人给新闻单开一条不走 `charge::stage`
/// 的结算，那几条照样全绿，只有这一条会红。指数钉在 2.0：每一条的数都必须是数据上那个数
/// 的两倍，一条不多一条不少。
#[test]
fn the_carded_amount_is_scaled_by_the_price_index() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p1, 90_000);

    let lines = step_onto_money_news(&mut app, p0, 12, 2.0);
    assert!(lines.len() >= 2, "前置：这一串够长");
    for line in &lines {
        let (id, _broadcaster, hits) = parts(line);
        let doubled = carded(&app, &id) * 2;
        assert!(
            hits.contains(&format!("Gained({doubled})"))
                || hits.contains(&format!("Lost({doubled})")),
            "物价 2.0 时{id}该按两倍过手，读到的是：{line}"
        );
        for who in ["P0", "P1"] {
            let net = net_for(&hits, who);
            assert!(
                net == 0 || net.abs() == doubled,
                "{who} 在这一播里的数不对：{line}"
            );
        }
    }
    assert_cash_audit(&mut app, START);
}

/// **同场不连重**在世界里成立：连着两条播出去的新闻不能是同一条。
///
/// 纯函数那一侧（`news::draw`）钉的是映射；这一条钉的是「结算真的读了 `last_news`」。
/// 有人哪天改成每次自己 `pick_news` 而不看上一条，这一条会红 —— 那症状是玩家说的
/// 「今天怎么又是这条」。十轮不换 app、不重置流，所以这一串就是那一局的样子。
#[test]
fn no_two_broadcasts_in_a_row_are_the_same() {
    let (_app, lines) = step_on_news(10);
    let ids: Vec<String> = lines.iter().map(|l| parts(l).0).collect();
    for pair in ids.windows(2) {
        assert_ne!(pair[0], pair[1], "连着两条不该是同一新闻：{ids:?}");
    }
}

/// 换一颗种子就换一串新闻，而几颗种子加起来**每一条都露过面**。
///
/// 这一条不能只在一种子上问：固定一条序列里「缺谁」都是可能的，那种红说的是种子而不是
/// 算法。六颗种子各播五轮（30 次）还凑不齐池子，才说明抽取真的偏了。要覆盖的 id 名单
/// 从资产里读（[`pool_ids`]）而不是在测试里抄一遍 —— 「往 JSON 里加一条新闻」该红的是
/// 「新那条没被抽到」，而不是「测试里那三行字符串没改」。
///
/// 它同时也钉住「新闻流读的是会话种子」：有人把 `Rng::new` 的 tag 换成常量或忘了接子流，
/// 这一条会红而 [`no_two_broadcasts_in_a_row_are_the_same`] 照绿。
#[test]
fn another_seed_broadcasts_another_show() {
    let mut firsts: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut pool = Vec::new();
    for seed in 1..=6u64 {
        let (mut app, p0) =
            ready_on_seeded(NEWS - 1, DecisionPolicy::DeclineAll, GameSeed(seed * 7919));
        if pool.is_empty() {
            pool = pool_ids(&app);
        }
        let lines = walk_onto_news(&mut app, p0, 5);
        firsts.push(parts(&lines[0]).0);
        seen.extend(lines.into_iter().map(|l| parts(&l).0));
    }
    let distinct: std::collections::HashSet<&String> = firsts.iter().collect();
    assert!(
        distinct.len() > 1,
        "六颗种子第一播全是同一条，说明抽签没读会话种子：{firsts:?}"
    );
    assert!(
        pool.len() >= 3,
        "前置：池子至少三条，否则下面那句覆盖断言是空的：{pool:?}"
    );
    for want in &pool {
        assert!(
            seen.iter().any(|id| id == want),
            "三十次播映里没念过 {want}，「全池随机」是假的：{seen:?}"
        );
    }
}

/// 池子里所有 id（从资产读）。
fn pool_ids(app: &App) -> Vec<String> {
    app.world()
        .resource::<Assets<NewsSet>>()
        .get(&app.world().resource::<DataHandles>().news)
        .expect("新闻池没加载")
        .0
        .iter()
        .map(|n| n.id.clone())
        .collect()
}

/// `last_news` 存的是**下标**，而它必须在开局归零（P15 存档的那条账）。
/// 装配那一步造的是新的一份 `GameSession`（`world::begin_game_system`），所以这一条钉的是
/// 「新局第一播没有要躲的上一条」—— 留着上一局那一条，新局第一次踩会**避开一条它
/// 这局从没播过的新闻**。
#[test]
fn a_fresh_match_has_no_previous_news_to_avoid() {
    let (app, _) = step_onto_once();
    assert_eq!(
        app.world().resource::<GameSession>().last_news,
        None,
        "新局不该带着上一局的上一条"
    );

    let (mut app, _p0) = step_onto_once();
    clear_occupancies(&mut app);
    let want = turn_count(&app) + 1;
    play_turns(&mut app, want);
    let line = news_lines(&app)
        .first()
        .expect("这一播真发生了，否则下面那句是空跑的")
        .clone();
    assert_eq!(
        app.world().resource::<GameSession>().last_news,
        Some(NewsId(index_of(&app, &parts(&line).0))),
        "记住的该正是刚播出去那一条的**下标**：{line}"
    );
}

/// 新闻的罚款登记在 `Penalty` 那一族，所以**免罪那一族吃得到它**。
///
/// 这里单独钉的不是「审计对得上」（每条用例最后都问一遍），而是新闻没有各写一套结算：
/// `deck.rs` 里 `FREE_PASS_DOMAINS` 那条注释早就替 P11 写好了「税也归这里」，而
/// 「归这里」的可执行版本就是挂一条修正之后这一笔真的变了。
///
/// 前置与 [`a_news_that_names_somebody_else_never_touches_the_broadcaster`] **逐字相同**
/// （同一种子、同样六轮、指数同样钉在 1.0），只多挂那一条修正：于是那一串
/// `Lost(3000)`／`Lost(2000)` 在这里全部翻成 `Lost(0)` —— 同一份世界上「有卡」与「没卡」
/// 的两个读数，不是两份不可比的前置。
///
/// 两条断言是一对，少一条都松：**挨刀分文不动**说「新闻走的是管线」，
/// **补贴照领**说「这条修正没有越出它自己那一域」（越出去等于把整条新闻免了）。
#[test]
fn a_named_fine_is_a_penalty_so_a_cancel_waives_it() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p1, 90_000);
    let subsidy = carded(&app, "news_001");
    register(
        &mut app,
        Correction::always(
            p1,
            Scope::Domain(ChargeDomain::Penalty),
            Adjustment::Cancel,
            Source::Card,
        ),
    );
    let before = cash_of(&mut app, p1);

    let lines = step_onto_money_news(&mut app, p0, 12, 1.0);
    let fines: Vec<&String> = lines.iter().filter(|l| !l.contains("news_001")).collect();
    let field = lines.len() - fines.len();
    assert!(
        !fines.is_empty() && field > 0,
        "六条里点名与全场各得有一条，否则这上下半句都是空跑的：{lines:?}"
    );
    for line in &fines {
        assert!(
            line.contains("Lost(0)"),
            "免掉的罚款要在流水里写成 0：{line}"
        );
        assert_eq!(
            net_for(&parts(line).2, "P1"),
            0,
            "挂着 `Cancel` 的人每一刀都该是 0：{line}"
        );
    }
    assert_eq!(
        cash_of(&mut app, p1) - before,
        subsidy * field as i64,
        "乙：罚款分文不动、补贴照领：{lines:?}"
    );
    // **常开**的那一条不消耗：它没有 `Waive` 流水（那条说的是一张卡花掉了），
    // 寿命由它的来源管。这一点下一那条用例用对照钉
    assert_eq!(corrections_len(&app), 1, "常开的那条修正不该被新闻花掉");
    assert_cash_audit(&mut app, START);
}

/// 一次性那一族（真正的**免罪卡**）只挡一刀，第二刀照砍。
///
/// 上面那条挂的是常开的修正，而玩家手里那张是**一张卡**：它要能挡下新闻的罚款
/// （不然「免罪卡只对命运牌有效」这条隐含承诺就是假的），又只能挡一次。
/// 于是这一条问三件事：第一刀 `Lost(0)` 且有一条 `Waive`，登记处当场空掉，
/// 后面的刀分文不免。`Waive` 只可能来自 `stage_and_spend` 的「真的花掉了哪几条」回报 ——
/// 上一那条没有它，正是「常开／一次性」两种寿命在流水里的不同形状。
#[test]
fn a_one_shot_free_pass_waives_exactly_one_news_fine() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p1, 90_000);
    register(
        &mut app,
        Correction::one_shot(
            p1,
            Scope::Domain(ChargeDomain::Penalty),
            Adjustment::Cancel,
            Source::Card,
            0,
        ),
    );
    let before = cash_of(&mut app, p1);

    let lines = step_onto_money_news(&mut app, p0, 12, 1.0);
    let fines: Vec<String> = lines
        .iter()
        .filter(|l| !l.contains("news_001"))
        .cloned()
        .collect();
    assert!(
        fines.len() >= 2,
        "至少要挨两刀才问得出「只挡一次」，六条里不够：{lines:?}"
    );
    assert!(
        fines[0].contains("Lost(0)"),
        "第一刀该被那张卡挡下：{}",
        fines[0]
    );
    for line in &fines[1..] {
        let each = carded(&app, &parts(line).0);
        assert_eq!(
            net_for(&parts(line).2, "P1"),
            -each,
            "卡烧掉之后的每一刀都要真的砍下去：{line}"
        );
    }
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l.starts_with("Waive(P1$") && l.ends_with("·Card)")),
        "挡下的那一笔要说得出替谁挡了多少：{:?}",
        ledger(&app)
    );
    assert_eq!(corrections_len(&app), 0, "一张卡就是只烧一张");
    let subsidy = carded(&app, "news_001") * (lines.len() - fines.len()) as i64;
    let paid: i64 = fines[1..].iter().map(|l| carded(&app, &parts(l).0)).sum();
    assert_eq!(
        cash_of(&mut app, p1) - before,
        subsidy - paid,
        "补贴照领、第二刀照挨：{lines:?}"
    );
    assert_cash_audit(&mut app, START);
}

/// 回合机在新闻之后照常走：没有谁把 `Pending` 留在半空。
///
/// 三回合而不是四：演示盘默认两人，偶数回合正好绕回播报者自己，那句「早该换人」
/// 就会红在轮转规则上而不是红在新闻上。
#[test]
fn the_turn_machine_keeps_running_after_a_broadcast() {
    let (mut app, p0) = step_onto_once();
    clear_occupancies(&mut app);
    let want = turn_count(&app) + 3;
    play_turns(&mut app, want);
    assert!(!news_lines(&app).is_empty(), "前置：这一趟至少播过一条");
    assert_ne!(current(&app), p0, "三回合之后早该换人");
    assert_eq!(phase(&app), TurnPhase::WaitRoll, "回合机没被新闻卡住");
}

// ===== P11b①：地档（作用域「这一带」）=====
//
// 演示盘的路段：4 王府井 与 5 西单 同属 `group: yellow`（`prototype.json` 里那两格的
// 路段键），而 13 号新闻中心脚下不是地 —— 环上离它最近的一块地是 11 号格上的王府井
// （2 步）。同一个数由 `board.rs::nearest_land_walks_outward_from_a_tile_that_is_not_land`
// 钉住，这里只念它，不在测试里再算一遍环。

const ANCHOR: u32 = 4;
const STREET: [u32; 2] = [4, 5];

/// 把那一整条街重新摆成「阿土伯名下的三层楼」（中心那块还押着）。
fn arm_street(app: &mut App) {
    own(app, ANCHOR, Some(0), 3, true);
    own(app, 5, Some(0), 3, false);
}

/// 天灾落在**开广播那个人脚下**那一整条街：楼平了、地收回、抵押一起清，而钱一分不动。
#[test]
fn a_ground_news_razes_the_street_under_the_broadcaster() {
    let (mut app, p0) = step_onto_once();
    arm_street(&mut app);
    for land in STREET {
        assert_eq!(
            plot_state(&mut app, land),
            (Some(0), 3, land == ANCHOR),
            "前置：两块都是他的三层楼（中心那块还押着）"
        );
    }

    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_004", 40, arm_street);
    let (center, lands) = ground(&line);
    assert_eq!(center, format!("L{ANCHOR}"), "锚点是脚下最近那块地：{line}");
    assert_eq!(
        lands,
        vec!["L4".to_string(), "L5".to_string()],
        "整条街都要在清单里（凭据上那份清单就是「拆了哪几块」那句话）：{line}"
    );
    assert!(
        line.contains("Razed { lands: 2, seized: 2 }"),
        "两块都该连地收回：{line}"
    );
    for land in STREET {
        assert_eq!(
            plot_state(&mut app, land),
            (None, 0, false),
            "L{land} 该被夷平、收回、抵押一起清：{line}"
        );
    }
    // 拆房不赔钱：这一条上没有任何一笔现金。这是这条阶梯的定义（PLAN §三 那一行
    // 「仅拆建筑、所有权保留／连地收回」里没有「补偿」那一格），不是漏了结算
    assert!(
        !line.contains("Gained") && !line.contains("Lost"),
        "地档不该动钱：{line}"
    );
    assert_cash_audit(&mut app, START);
}

/// 清单里有**无主的那一块**，而 `seized` 只数有主的。
///
/// 这一条是「按业主分组会把无主的那块丢掉」的反例：若清单是从业主集合推出来的，
/// 这里会是 `["L4"]` + `lands: 1`，而街上明明少了两排房子 —— 那种世界画面上看不出来，
/// 只有清单与 `seized` 两个数并排放才读得出「扫到 2 块、其中 1 块本来没人要」。
#[test]
fn the_ground_list_includes_the_plot_that_had_no_owner() {
    let (mut app, p0) = step_onto_once();
    // 只有王府井是他的；西单无主（演示盘开局本来就是这样，摆一次是防上一轮残留）
    own(&mut app, ANCHOR, Some(0), 3, false);
    own(&mut app, 5, None, 2, false);
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_004", 40, |app| {
        own(app, ANCHOR, Some(0), 3, false);
        own(app, 5, None, 2, false);
    });
    let (_center, lands) = ground(&line);
    assert_eq!(
        lands,
        vec!["L4".to_string(), "L5".to_string()],
        "无主那块也要在清单里：{line}"
    );
    assert!(
        line.contains("Razed { lands: 2, seized: 1 }"),
        "扫到 2 块、只有 1 块有主可收：{line}"
    );
    assert_eq!(plot_state(&mut app, 5).1, 0, "无主的那块照样被拆：{line}");
    assert_cash_audit(&mut app, START);
}

/// 验收③：**路段类新闻盖上去的状态，到期自动失效**。
///
/// 「这一带地价调涨」盖的是 `Timed<LandStatus>`，与涨价卡同一枚组件、同一个到期钟
/// （`expire_land_status_system`，一天＝一个回合末）。所以这条验的不是「新闻能不能盖」
/// （上一条管），而是「新闻盖上去的那一枚会不会自己掉」—— 盖了不掉，租金就永久翻倍。
///
/// 盖完之后把 13 号格改成道路再跑回合：不这么做，路上还会再抽到新闻，
/// 而「再盖一次」与「到期没掉」在画面上是同一个读数（`status_of` 又有了）。
#[test]
fn a_street_news_expires_by_itself() {
    let (mut app, p0) = step_onto_once();
    arm_street(&mut app);
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_006", 40, arm_street);
    let (_center, lands) = ground(&line);
    assert_eq!(
        lands,
        vec!["L4".to_string(), "L5".to_string()],
        "整条街都盖上了：{line}"
    );
    assert!(line.contains("Stamped"), "盖状态的那条凭据要在：{line}");
    for land in STREET {
        let (status, days) = status_of(&mut app, land).expect("刚播完，这一格该挂着涨价");
        assert_eq!(status, LandStatus::PriceHike, "L{land} 盖的是涨价");
        assert!(
            days > 0 && days <= 5,
            "L{land} 的天数在卡面那一格之内：{days}"
        );
    }

    // 撤掉载体，让后面那几回合不可能再盖一次（改的是内存里那份棋盘快照）
    retile(&mut app, NEWS, TileKind::Road);
    let want = turn_count(&app) + 12;
    play_turns(&mut app, want);
    for land in STREET {
        assert_eq!(
            status_of(&mut app, land),
            None,
            "L{land} 的涨价早该自己掉了（十二个回合末 > 卡面 5 天）"
        );
    }
    assert_cash_audit(&mut app, START);
}

/// 锚点跟着**人**走，不是跟着新闻中心那一格走。
///
/// 这条是「脚下那块地」这五个字的全部依据：把 7 号格（它本身就是一块地 —— 外滩商业区
/// 的两格之一）改成新闻中心，人踩上去时锚点就该是 **L2**，而不是上次那个 L4。
/// 若锚点写成了「盘上第一块地」或「新闻中心在数据里绑的那块」，这一条会红。
#[test]
fn the_anchor_is_the_ground_under_the_broadcaster_not_the_tile_itself() {
    let (mut app, p0) = ready_on(6, 1, DecisionPolicy::DeclineAll);
    retile(&mut app, 7, TileKind::News);
    own(&mut app, 2, Some(0), 3, false);
    let line = walk_until_rearmed(&mut app, p0, 7, "news_005", 40, |app| {
        own(app, 2, Some(0), 3, false);
    });
    let (center, lands) = ground(&line);
    assert_eq!(center, "L2", "他脚下那一格就是那块地，锚点该是它：{line}");
    assert!(
        lands.iter().any(|l| l == "L2"),
        "3×3 以脚下为中心，中心那块必在清单里：{line}"
    );
    assert_eq!(plot_state(&mut app, 2).1, 0, "楼该被拆平：{line}");
    assert_cash_audit(&mut app, START);
}
