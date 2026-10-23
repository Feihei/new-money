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
    GameSession, ItemKind, NewsId, NewsSet, Scope, Source, TileKind, TurnPhase,
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

/// 同一件事，但每轮把物价指数摆回 `index`（`Some` 那一条是金额类验收要的：
/// 指数本身会被补贴一路推高，而「卡面 × 指数」那个确切的数只有在钉住之后才说得出）。
fn step_onto_news(app: &mut App, p0: Entity, times: usize, index: Option<f32>) -> Vec<String> {
    let mut lines = Vec::new();
    for _ in 0..times {
        clear_occupancies(app);
        make_current(app, p0);
        place(app, p0, NEWS - 1);
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
            NEWS,
            "这一轮没踩到新闻中心（遥控骰子被拒了？）：{lines:?}"
        );
        lines = news_lines(app);
    }
    assert_eq!(lines.len(), times, "踩 {times} 次就该播 {times} 条");
    lines
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

    let lines = step_onto_news(&mut app, p0, 6, Some(1.0));
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

    let lines = step_onto_news(&mut app, p0, 8, Some(1.0));
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
        "八条里那两种点名各得撞一次，否则这两句是空跑的：{lines:?}"
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

    let lines = step_onto_news(&mut app, p0, 10, Some(1.0));
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

    let lines = step_onto_news(&mut app, p0, 6, Some(2.0));
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

/// 换一颗种子就换一串新闻，而几颗种子加起来**三条都露过面**。
///
/// 这一条不能只在一种子上问：池子只有三条，固定一条序列里「缺谁」都是可能的，那种红
/// 说的是种子而不是算法。五颗种子各播三轮（15 次）还凑不齐三条，才说明抽取真的偏了。
/// 它同时也钉住「新闻流读的是会话种子」：有人把 `Rng::new` 的 tag 换成常量或忘了接子流，
/// 这一条会红而 [`no_two_broadcasts_in_a_row_are_the_same`] 照绿。
#[test]
fn another_seed_broadcasts_another_show() {
    let mut firsts: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    for seed in 1..=5u64 {
        let (mut app, p0) =
            ready_on_seeded(NEWS - 1, DecisionPolicy::DeclineAll, GameSeed(seed * 7919));
        let lines = walk_onto_news(&mut app, p0, 3);
        firsts.push(parts(&lines[0]).0);
        seen.extend(lines.into_iter().map(|l| parts(&l).0));
    }
    let distinct: std::collections::HashSet<&String> = firsts.iter().collect();
    assert!(
        distinct.len() > 1,
        "五颗种子第一播全是同一条，说明抽签没读会话种子：{firsts:?}"
    );
    for want in ["news_001", "news_002", "news_003"] {
        assert!(
            seen.iter().any(|id| id == want),
            "十五次播映里没念过 {want}，「全池随机」是假的：{seen:?}"
        );
    }
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

    let lines = step_onto_news(&mut app, p0, 6, Some(1.0));
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

    let lines = step_onto_news(&mut app, p0, 6, Some(1.0));
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
