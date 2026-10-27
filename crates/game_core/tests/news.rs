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
use bevy::ecs::query::With;
use common::*;
use game_core::prelude::{
    ActiveBoard, Adjustment, BankOp, BidMove, CardEffect, ChargeDomain, CompanyLedgers, Correction,
    DataHandles, Date, Detention, DetentionPlace, GameSeed, GameSession, Gavel, HolidaySet,
    Institution, ItemKind, LandStatus, MarketClock, Mount, NewsId, NewsSet, Pending, Scope, Source,
    StaySentence, StockSet, Term, TileKind, TradeSide, TurnPhase, date_of,
};

/// 演示盘上的新闻中心（`prototype.json` 的 14 号格，`(0,4)`；P13 插了魔法屋之后从 13 顺移）。
/// `board.rs::diagonal_segments_count_is_three` 与 `tools/validate_board.py` 钉着同一个数。
const NEWS: u32 = 14;
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

/// 这一条流水里**有没有钱**（`Gained(`／`Lost(`／`CouldNotPay` 三种凭据之一）。
///
/// 金额类断言一律先筛掉没有钱的那几条。两批新增各自带来一种「没有人挨钱」的形状：
/// P11b① 的地档（`地L3[L2/L5][Razed { … }]`，身上根本没有钱），P11b② 的停走与放人
/// （人档，但挨的是「原地停留 1 回合」不是钱）。用「有没有钱」当判据而不是「是不是地档」：
/// 后者是前者的一个子集，而将来再来一种不打钱的动词时，按地档筛会漏。
fn carries_money(line: &str) -> bool {
    let hits = parts(line).2;
    hits.contains("Gained(") || hits.contains("Lost(") || hits.contains("CouldNotPay")
}

/// 一串流水里带钱的那几条。
fn money_only(lines: &[String]) -> Vec<String> {
    lines.iter().filter(|l| carries_money(l)).cloned().collect()
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

/// 这一串流水里 `who` **真的掏出去**了多少（只加 `Lost(...)` 那几笔，取正数）。
///
/// 它存在的唯一理由是把「净额」与「动过」分开：P11c① 把池子加到 14 条之后，一条用例撞上
/// 「三条 +1500 补贴配一条 −3000 罚金」—— 净额是 **+1500**，于是那句 `cash < before` 红了，
/// 而红的是池子的运气而不是这条要钉的那件事。这条要挡的世界是
/// 「凭据写了钱、钱包没动」（两边都一动没动照样对得平，见 [`net_for`] 上面那段），
/// 那一问只有**支出总额**答得住，净额答不住。
fn paid_over(lines: &[String], who: &str) -> i64 {
    lines
        .iter()
        .map(|line| {
            parts(line)
                .2
                .split('|')
                .filter(|seg| seg.split('[').next().unwrap_or_default() == who)
                .flat_map(|seg| {
                    seg.split('[')
                        .nth(1)
                        .unwrap_or_default()
                        .trim_end_matches(']')
                        .split('+')
                })
                .filter_map(|one| {
                    one.strip_prefix("Lost(")?
                        .strip_suffix(')')?
                        .parse::<i64>()
                        .ok()
                })
                .sum::<i64>()
        })
        .sum()
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
/// 之后「卡面那个数 == 钱包那个数」才说得出。`times` 给到 12 是因为池子已经十二条 ——
/// 十二次里三种人档各得撞几次，那几条用例的「前置」断言才有牙（撞不到就是空跑的）。
fn step_onto_money_news(app: &mut App, p0: Entity, times: usize, index: f32) -> Vec<String> {
    money_only(&step_onto_news(app, p0, times, Some(index)))
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
    release_grips(app);
    pin_calendar(app);
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

/// 把每个人身上的「停留」计时摘掉 —— **夹具复位**，不是被测判据。
///
/// 为什么必要：P11b② 之后池子里有两条按载具筛人的停走新闻（豪雨袭城／交通堵塞），
/// 它们写的正是 `StaySentence`，而身上挂着那一格的人，规则层会拒掉他的掷骰（P4④ 那颗
/// 「停留」按钮）。于是这批用例下一轮走不动，症状是上面那句「遥控骰子被拒了？」——
/// 与它要问的那件事（新闻播出去之后世界变成什么样）毫无关系。
///
/// 摘掉它不是绕过一条规则：「被按住的人要按哪颗按钮、按下去扣不扣」由 `tests/stay.rs` 钉，
/// 那里才是这一格的战场。这条夹具只保证「新闻这一批用例的每一步都能真的走一步」。
fn release_grips(app: &mut App) {
    let held: Vec<Entity> = {
        let world = app.world_mut();
        let mut q = world.query_filtered::<Entity, With<StaySentence>>();
        q.iter(world).collect()
    };
    for player in held {
        app.world_mut().entity_mut(player).remove::<StaySentence>();
    }
}

/// 把历法锚点钉在 2048-04-01 —— **夹具隔离**，不是被测判据。
///
/// 为什么必要：P2b 之后「今天是什么日子」由会话种子抽，而 P11b② 之后这批用例要跑几十轮
/// 才撞得齐两种点名（池子 12 条了）。跑过正月初一那天，每个在场玩家领一个四位红包 ——
/// 那是**凭空进钱包的一笔现金**，于是最后那句「钱包差额 == 新闻流水净额」会红在红包上，
/// 而它问的是新闻。（实测：第 43 轮 `Envelope(P0$5216)`，差 5216。）
///
/// 钉在 4/1 之后，一百来轮里只会路过月末结息与月中分红，而这两笔在这个夹具里都是 0
/// （没人存过钱、没人持有股票）。要问日历事件本身的是 `tests/calendar.rs` 那批用例。
fn pin_calendar(app: &mut App) {
    app.world_mut().resource_mut::<GameSession>().epoch = Date::new(2048, 4, 1);
}

/// 一直踩到 `want` 那一条播出来为止，返回那一条流水（最多 `tries` 次），
/// 每轮先跑一次 `rearm` 把夹具摆回「被测那一播该有的样子」。
///
/// 池子是「全池随机 + 同场不连重」，测试要问的是**某一条特定新闻**的行为（地档那几条），
/// 所以不能靠「第 N 次恰好抽到它」—— 那是一条只有代码知道的隐藏状态。撞不到就 panic：
/// 十二条池子四十次还抽不出某一条，说明抽取偏了，那是比这条用例本身更响的红。
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

/// 那条新闻卡面上写的那个金额（绝对值），**只在那个数是写死的**时候有答案。
///
/// 从资产里读而不是在测试里再抄一遍 —— 「改一行 JSON 该让哪几条用例红」这件事，
/// 只有从数据侧拿数才答得出来。
///
/// `Option` 而不是 `panic`：P11b② 之后池子里有了两条**税**（`levy`），它们的数是从世界里
/// 现读的（他手上有多少钱、名下那些地值多少），卡面上只有百分比。拿「卡面那个数」去问一条税
/// 是问错了对象，而问错对象的症状不该是夹具崩掉 —— 调用方按 `None` 跳过，
/// 各留一句自己的断言（税那一句是 [`a_tax_is_not_scaled_twice_and_a_landless_one_is_zero`]）。
fn fixed_cash_of(app: &App, id: &str) -> Option<i64> {
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
        panic!("第一批那三条各带一条效果");
    };
    match only {
        CardEffect::AddCash { amount } | CardEffect::DeductCash { amount } => Some(amount.abs()),
        _ => None,
    }
}

/// 同一条，只问「卡面定额」那三条，问不到就响。
///
/// 保留这个硬版本是给那些**明知在问哪一条**的调用方（`carded(&app, "news_001")` 那几处）：
/// 数据哪天把补贴改成税率，这里当场 panic，而不是让那几条用例静默地少问一件事。
fn carded(app: &App, id: &str) -> i64 {
    fixed_cash_of(app, id).unwrap_or_else(|| panic!("{id} 卡面上没有定额的数"))
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

    let lines = step_onto_money_news(&mut app, p0, 24, 1.0);
    let mut paid_out = 0;
    for line in &lines {
        let (id, broadcaster, hits) = parts(line);
        // 两条税（`news_007`／`news_008`）跳过：它们打在**所有人**头上，包括开广播的那位，
        // 而这一条问的是「点名 vs 全场」那两种形状。它们各自的形状由 P11b② 那几条钉，
        // 而最后那两句钱包断言读的仍是全部 `lines`（不跳过）—— 跳过只是不问，不算数
        let Some(each) = fixed_cash_of(&app, &id) else {
            continue;
        };
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
        "十二条里没有一刀真的砍下去，那「被点名者金额变」是空跑的：{lines:?}"
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

    // 四十八次而不是十二次：这一条要**两种点名都撞到**（`richest` 砸自己、`random_other`
    // 砸别人），而 P11b② 之后池子十二条、其中只有两条是这两档 —— 十二次撞不齐，
    // 二十四次也撞不齐（这一改的前置断言在 24 次时真的红过一次，那不是保险，是牙）
    let lines = step_onto_money_news(&mut app, p0, 48, 1.0);
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
        "这四十八次里那两种点名各得撞一次，否则这两句是空跑的：{lines:?}"
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
    let mut asked = 0;
    for line in &lines {
        let (id, _broadcaster, hits) = parts(line);
        // 税不在这一串里问：它**刻意不随物价缩**（基数已经是今天的价），
        // 那一条反着钉在 [`a_tax_is_not_scaled_twice_and_a_landless_one_is_zero`]
        let Some(each) = fixed_cash_of(&app, &id) else {
            continue;
        };
        asked += 1;
        let doubled = each * 2;
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
    assert!(
        asked >= 2,
        "这一串里至少两条是卡面定额的，否则整条是空跑的：{lines:?}"
    );
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
/// 算法。要覆盖的 id 名单从资产里读（[`pool_ids`]）而不是在测试里抄一遍 —— 「往 JSON 里加
/// 一条新闻」该红的是「新那条没被抽到」，而不是「测试里那三行字符串没改」。
///
/// 每颗种子播「池子条数 × 2」轮而不是写死五次：凑齐全池要抽的次数随池子变大（期望是
/// n·H(n) ≈ 12 条时要 37 次），写死一个数就是在赌池子不再长 —— 而 P11 正是那条池子还在长的阶段。
///
/// 它同时也钉住「新闻流读的是会话种子」：有人把 `Rng::new` 的 tag 换成常量或忘了接子流，
/// 这一条会红而 [`no_two_broadcasts_in_a_row_are_the_same`] 照绿。
#[test]
fn another_seed_broadcasts_another_show() {
    let mut firsts: Vec<String> = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let mut pool: Vec<String> = Vec::new();
    for seed in 1..=6u64 {
        let (mut app, p0) =
            ready_on_seeded(NEWS - 1, DecisionPolicy::DeclineAll, GameSeed(seed * 7919));
        if pool.is_empty() {
            pool = pool_ids(&app);
        }
        let lines = walk_onto_news(&mut app, p0, pool.len() * 2);
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
            "{} 次播映里没念过 {want}，「全池随机」是假的",
            pool.len() * 2 * 6
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
/// 罚款与税都归 [`ChargeDomain::Penalty`]（`charge.rs` 那张归域表里 `Levy` 与 `Penalty`
/// 同域那一句），而「归这里」的可执行版本就是挂一条修正之后这一笔真的变了。
///
/// 前置与 [`a_news_that_names_somebody_else_never_touches_the_broadcaster`] **逐字相同**
/// （同一种子、同样六轮、指数同样钉在 1.0），只多挂那一条修正：于是那一串
/// `Lost(3000)`／`Lost(2000)` 在这里全部翻成 `Lost(0)` —— 同一份世界上「有链」与「没链」
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
            Source::God,
        ),
    );
    let before = cash_of(&mut app, p1);

    let lines = step_onto_money_news(&mut app, p0, 12, 1.0);
    let fines: Vec<&String> = lines.iter().filter(|l| !l.contains("news_001")).collect();
    let field = lines.len() - fines.len();
    assert!(
        !fines.is_empty() && field > 0,
        "十二条里点名与全场各得有一条，否则这上下半句都是空跑的：{lines:?}"
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
    // **常开**的那一条不消耗：③ 之后这条链上根本没有「一次」这个位（`Used` 那条流水说的
    // 是一张**卡**没了，而卡住在门里，不住在这里）。寿命由它的来源管 —— 神离场才撤。
    assert_eq!(corrections_len(&app), 1, "常开的那条修正不该被新闻花掉");
    assert_cash_audit(&mut app, START);
}

/// 验收②（P13b③）：一笔**账**在**别人的回合**落到我头上 —— 窗口挂到我这儿、回合停住，
/// 而我答「用」之后它才免掉，而那张卡只够挡这一笔。
///
/// 夹具是 `news_008`（全场现金税 10%）：税按每个人自己的钱包抽，所以「谁欠多少」在这一播里
/// 是**分开算**的 —— 这是全工程唯一一条「钱的承受者不是当前回合玩家」的来路（租金、服务费、
/// 公司到访的付款人都是踩格的那位，也就是他自己）。10-27 拍的那句「决策主体要能在回合内
/// 跳动」在钱这一侧只有这里问得出来。
///
/// 钱摆成 P0 十万 / P1 三万，而免费卡只在 P1 手里 —— 于是整条池子里**只有一问**问得出来：
/// - `news_002`（最富那位罚 3000）点到的是 P0，他身上没卡，门连问都不问就落；
/// - `news_008` 对 P0 是 10000（同样没卡），对 P1 是 3000（过了 2000 那道门槛且有卡）→ 问；
/// - `news_003`（2000 定额）卡在「**超过**门槛」那一句上，谁都不问。
///
/// 这样摆不是洁癖：`DecisionPolicy::Hold` 底下每一问都会把回合机押住，多一种问就多一次空转；
/// 而「只有这一问」本身就是那条门槛判据的一个读数。
#[test]
fn a_news_tax_asks_its_bearer_while_the_broadcaster_holds_the_turn() {
    let (mut app, p0) = ready_on(NEWS - 1, 1, DecisionPolicy::Hold);
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p0, 100_000);
    set_cash(&mut app, p1, 30_000);
    // 卡**留在手里**就是资格（③ 之后没有「打出去挂着」那一步：`Insulation` 组件删了）
    let free = item_index(&app, "item_free");
    deal(&mut app, p1, free.0);
    let before = cash_of(&mut app, p1);

    walk_until_the_gate_asks(&mut app, p0, 40);

    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::Bill)),
        "账落在谁头上，那一问就挂到谁身上"
    );
    assert_eq!(
        gate_source(&app).as_deref(),
        Some("news_008"),
        "问出来的那一封是从现金税来的"
    );
    assert_eq!(current(&app), p0, "回合还是播报者的");
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "而回合停在他答完为止 —— 不是「P1 答完，P0 才能继续」"
    );

    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(cash_of(&mut app, p1), before, "免掉了就一分不动");
    assert_eq!(free_in_hand(&app, p1), 0, "那张卡花掉了");
    let log = ledger(&app);
    assert!(
        log.iter()
            .any(|l| l.starts_with("Used(P1") && l.contains("$3000")),
        "「花掉一张卡」与「这张卡值多少」要同一条流水说得出来：{log:?}"
    );
    assert_eq!(gate_asking(&app), None, "答完就没有人在等了");
    assert_cash_audit(&mut app, START);
}

/// 把 P0 一次次送上新闻中心，直到门问出一句为止（最多 `tries` 播）。
///
/// 为什么不复用 [`walk_until_rearmed`]（那句「一直踩到某一条播出来为止」）：它按名字挑的
/// 是**流水**，而这里要停在**问出去的那一刻** —— 回合被门押住之后 `play_turns` 只剩空转到
/// 帧数上限，那条走法在这里会在正确的那一播上走过头。
fn walk_until_the_gate_asks(app: &mut App, p0: Entity, tries: usize) {
    for _ in 0..tries {
        // 每一播都把现金摆回「只有 P1 会被问」那一份前置：上一播的税与补贴已经动过钱包
        set_cash(app, p0, 100_000);
        let p1 = roster(app)[1].0;
        set_cash(app, p1, 30_000);
        step_news_on(app, p0, NEWS, Some(1.0));
        if gate_asking(app).is_some() {
            return;
        }
    }
    panic!("{tries} 播里门一次都没问出来（现金税没撞上？「全池随机」那条是假的）");
}

/// 他说「不用」：账照原样落下，而那张卡**分文不动地留在手里** —— ⑤ 那句「放弃不消耗」。
///
/// 这一条走 `DeclineAll`：自动应答替 P1 答「不用」，回合机因此一路跑到底（上一条要停在
/// 那一问上，这一条要跑到落下来之后，两种走法不能互相冒充）。上面那条证明了「问了」，
/// 这一条证明的是「问而不用」与「不问」在原样落下这件事上是同一个出口。
#[test]
fn a_declined_tax_is_paid_in_full_while_the_card_and_the_windfalls_stay_untouched() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_cash(&mut app, p0, 100_000);
    set_cash(&mut app, p1, 30_000);
    let free = item_index(&app, "item_free");
    deal(&mut app, p1, free.0);
    let tax = 3_000;

    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_008", 40, |app| {
        set_cash(app, p0, 100_000);
        set_cash(app, p1, 30_000);
    });
    assert_eq!(
        net_for(&parts(&line).2, "P1"),
        -tax,
        "答「不用」的那一笔要真的砍下去：{line}"
    );
    assert_eq!(free_in_hand(&app, p1), 1, "放弃不该消耗那张卡");
    assert_eq!(
        gate_spends(&app),
        0,
        "整串流水里门一张卡都没花：{:?}",
        ledger(&app)
    );
    // 「凭据写了钱、钱包没动」是审计抓不住的（两边都一动没动照样对得平），所以要正面问一句
    let lines = news_lines(&app);
    assert!(
        paid_over(&lines, "P1") > 0,
        "P1 这一趟一分都没掏过，上面那句「砍下去」就成了空跑的：{lines:?}"
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

// ===== P11b②：按人筛的作用域、两条税、第一大地主、提前出院 =====
//
// 这一批的共同形状是**筛子**：作用域不再问「哪一位」或「全场」，而问「全场里哪一类人」。
// 每一条都配一对：筛到的那一格 + 筛不到的那一格。只写成功那一格的话，
// 「按载具筛」换成 `Everyone`、「按处筛」换成 `Everyone` 都照样绿 —— 那是一条空转的判据。

/// 把某人摆成「关在某一处、站在第 `at` 格、还剩 `days` 天」——**夹具前置**，写的是组件本体。
///
/// 为什么不走真路径（踩命运格抽那张水沟牌）：那要押运气与回合数，而这一批问的是
/// 「新闻放不放得出人、按处筛是不是真的在筛」，不是「住院怎么发生」（后者由
/// `tests/detention.rs` 钉）。与 `own`／`set_mount` 同一类：这是世界状态的前置，
/// 不是一笔交易，所以它不动现金、不发凭据。
fn hold(app: &mut App, player: Entity, at: u32, room: DetentionPlace, days: u32) {
    common::place(app, player, at);
    app.world_mut().entity_mut(player).insert(Detention {
        place: room,
        term: Term::Turns(days),
    });
}

/// 豪雨按住**走路的**那几位（P11b② 的第一条按载具筛人）。
///
/// 它落的是 `StaySentence` 而不是自己实现一套「跳过回合」—— P4④ 之后停留是 `WaitRoll`
/// 上那颗按钮，所以新闻侧只写状态。这一条因此同时是「新闻不另建回合机」的证据。
#[test]
fn the_rain_news_holds_the_walkers_and_spares_the_drivers() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_mount(&mut app, p1, Mount::Car);
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_010", 60, |app| {
        set_mount(app, p1, Mount::Car);
    });
    assert_eq!(
        named(&parts(&line).2),
        vec!["P0".to_string()],
        "开车的那位不该在这条名单里：{line}"
    );
    assert_eq!(
        stay_of(&app, p0),
        Some(1),
        "走路的那位被按住 1 回合（字面口径，P4④）：{line}"
    );
    assert_eq!(stay_of(&app, p1), None, "有车的人不受豪雨影响：{line}");
    assert!(
        line.contains("Stayed { turns: 1 }"),
        "凭据要说得出按住几回合：{line}"
    );
    assert_cash_audit(&mut app, START);
}

/// 交通堵塞按住**有车的**那几位 —— 上面那条的反面，两位的角色对调。
///
/// 两条各钉一半：只有这两句同时成立，「按载具筛」才不是一句装饰。合成一条 `Everyone`
/// 停走的话，上面那条照样绿（两人都被按住 → 名单里两个名字 → 红在名单上，但红在
/// **这一条**才是它该有的红法）。
#[test]
fn the_traffic_jam_holds_the_drivers_and_spares_the_walkers() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    set_mount(&mut app, p1, Mount::Bike);
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_011", 60, |app| {
        set_mount(app, p1, Mount::Bike);
    });
    assert_eq!(
        named(&parts(&line).2),
        vec!["P1".to_string()],
        "机车也算有车，走路的这位不该在名单里：{line}"
    );
    assert_eq!(stay_of(&app, p1), Some(1), "她该被按住：{line}");
    assert_eq!(stay_of(&app, p0), None, "步行的人反而照常前进：{line}");
    assert_cash_audit(&mut app, START);
}

/// 所得税抽的是**手上的钱**，不是「钱 × 物价」。
///
/// 这一条钉的是 `ChargeKind::Levy` 在 `scales_with_price` 那一格填的那个「不」：
/// 税的基数是从世界里现读的（他今天有多少钱），再乘一次指数就成了「物价 2 倍时
/// 10% 的税悄悄变 20%」—— 税率是新闻写死的，不该被另一条规则放大。
/// 与 [`a_land_tax_is_scaled_once_because_its_basis_is_todays_value`] 是一对：
/// 那条说「缩一次」，这条说「不缩」，两句合起来才是「基数已经含今天的价」这一句。
#[test]
fn an_income_tax_is_a_share_of_cash_not_of_the_price_index() {
    let probe = |index: f32| -> (i64, i64) {
        let (mut app, p0) = step_onto_once();
        let line = walk_until_rearmed(&mut app, p0, NEWS, "news_008", 60, |app| {
            set_price_index(app, index);
        });
        let hits = parts(&line).2;
        assert!(
            hits.contains("P0[Lost(") && hits.contains("P1[Lost("),
            "全场类的税该两个人都在名单里：{line}"
        );
        (-net_for(&hits, "P0"), cash_of(&mut app, p0))
    };
    for index in [1.0f32, 2.0] {
        let (tax, after) = probe(index);
        assert!(
            tax > 0,
            "物价 {index} 时这一播真的抽到了钱（否则下面那句是空跑的）"
        );
        // 抽完剩下的钱是税前的 90% ⟺ `after ∈ [9·tax, 9·tax + 9]`（税率 10%，整数向下取整
        // 最多差 9）。这一句把三种错法全抓住：**再乘一次物价**（税=20% → after=4·tax）、
        // **完全不缩**（基数读成名义钱数 → 物价 2.0 时税仍是 10% 但下面那句在 2.0 上照样对，
        // 所以这一句真正钉的是前者）、以及**税率写错**（比例不对，band 直接不成立）
        assert!(
            after >= tax * 9 && after < tax * 9 + 10,
            "物价 {index} 时所得税仍是「手上钱的 10%」，读到 tax={tax} 抽完之后={after}"
        );
    }
}

/// 地价税抽的是**名下地产的现值**，所以物价 2 倍时它恰好翻倍（一次，不是两次）。
///
/// A/B 两条而不是「等于某个写死的数」：夹具是同一份前置（同一块地、同一级楼），
/// 只有指数不同。若实现**再**乘一次物价，2.0 那一次会是四倍；若它**完全不缩**
/// （基数读成名义价），2.0 那一次与 1.0 相同。两种错法都被这一句抓住。
///
/// 顺带钉住「无地的那一位是 0 元税」：基数为零不是新闻落空，流水里那条 `Lost(0)`
/// 说的就是「这一播轮到你了，而你没地」。
#[test]
fn a_land_tax_is_scaled_once_because_its_basis_is_todays_value() {
    let tax_at = |index: f32| -> i64 {
        let (mut app, p0) = step_onto_once();
        own(&mut app, ANCHOR, Some(0), 3, false);
        let line = walk_until_rearmed(&mut app, p0, NEWS, "news_007", 60, |app| {
            own(app, ANCHOR, Some(0), 3, false);
            set_price_index(app, index);
        });
        let hits = parts(&line).2;
        assert_eq!(
            net_for(&hits, "P1"),
            0,
            "乙一块地都没有，地价税该是 0 而不是缺席：{line}"
        );
        -net_for(&hits, "P0")
    };
    let flat = tax_at(1.0);
    assert!(
        flat > 0,
        "前置：这一播真的抽到了钱（否则下面那句相等是两边都空跑）"
    );
    assert_eq!(
        flat * 2,
        tax_at(2.0),
        "物价翻倍时地价税恰好翻倍（一次），不是四倍也不是一倍"
    );
}

/// 「第一大地主」先数**块数**再比现值：两块空地赢一块三层楼。
///
/// 这一格是拍板过的（见 `PlayerPick::Landlord` 的注）：那个词说的是地多。
/// 夹具让阿土伯拿一块三层楼（现值 5,600）、乙拿两块空地（现值 3,000）——
/// 若实现按现值排，名单里会是 P0；按块数排才是 P1。
#[test]
fn the_landlord_award_goes_to_the_one_with_the_most_plots() {
    let (mut app, p0) = step_onto_once();
    let arm = |app: &mut App| {
        own(app, ANCHOR, Some(0), 3, false);
        own(app, 0, Some(1), 1, false);
        own(app, 1, Some(1), 1, false);
        set_price_index(app, 1.0);
    };
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_009", 60, arm);
    assert_eq!(
        named(&parts(&line).2),
        vec!["P1".to_string()],
        "两块空地该赢一块三层楼：{line}"
    );
    assert_eq!(
        net_for(&parts(&line).2, "P1"),
        carded(&app, "news_009"),
        "奖金是卡面那个数（指数钉在 1.0）：{line}"
    );
    assert_eq!(
        net_for(&parts(&line).2, "P0"),
        0,
        "不是地主就一分没有：{line}"
    );
    assert_cash_audit(&mut app, START);
}

/// 场上还没人买到地时，这一条**谁都不点名**。
///
/// 「表扬一位零地主」会把这条新闻变成「给当前回合的某个人发钱」，而那不是它说的话。
/// 空名单要说得出为什么空（顶栏那句「场上没有可点名的人」由表现层那几条用例钉，
/// 这一条钉的是规则层真的没点到人）。
#[test]
fn the_landlord_award_names_nobody_when_nobody_has_land() {
    let (mut app, p0) = step_onto_once();
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_009", 60, |_| {});
    assert!(
        parts(&line).2.is_empty(),
        "没人有地时这一条该是空名单（而不是「那就表扬第一个在场的人」）：{line}"
    );
    assert!(!line.contains("Gained("), "空名单不许动钱：{line}");
    assert_cash_audit(&mut app, START);
}

/// 医院那条新闻放出**关在医院**的人，走的是与期满同一条出口。
///
/// 它同时钉三件事：筛子真的在筛（下面那条反面）、组件真的被摘掉（不是把天数写成 0，
/// 那会让「他还在不在牌桌上」读成两个答案）、以及凭据带回了接入点。
#[test]
fn the_hospital_news_releases_the_hospitalized() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_012", 60, |app| {
        hold(app, p1, 20, DetentionPlace::Hospital, 3);
    });
    assert_eq!(
        named(&parts(&line).2),
        vec!["P1".to_string()],
        "名单里该只有住院的那位：{line}"
    );
    assert!(
        line.contains("Released { place: Hospital"),
        "凭据要说得出放的是哪一处：{line}"
    );
    assert_eq!(
        detain_of(&app, p1),
        None,
        "放出来就该真的没有计时了（写 0 天不算放）：{line}"
    );
}

/// 同一颗旋钮，关在**监狱**的人不许被放出 —— 「按处筛」那一格的全部牙齿在这条反面。
///
/// 只看上面那条，把 `EveryoneIf(Detained(Hospital))` 换成 `Everyone` 照样绿（他确实被放出了）。
/// 这一条把它换成真的：医院的好消息与监狱里的人无关，而他必须还在里面。
#[test]
fn a_hospital_news_does_not_empty_the_jail() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    let line = walk_until_rearmed(&mut app, p0, NEWS, "news_012", 60, |app| {
        hold(app, p1, 19, DetentionPlace::Jail, 3);
    });
    assert!(
        parts(&line).2.is_empty(),
        "他关在监狱，这条新闻的名单里不该有他：{line}"
    );
    assert_eq!(
        detain_of(&app, p1).map(|d| d.place),
        Some(DetentionPlace::Jail),
        "医院的消息不放监狱的人 —— 他还得待在里面：{line}"
    );
    assert_cash_audit(&mut app, START);
}

// ===== P11c①：公有地拍卖与强制征收 =====
//
// PLAN §四 第二批那五条里依赖 P9 的两条。它们共用「这一带」那一档做锚点，而答案正好相反：
// 一条把**无主**的地挂上台（钱离开牌桌，✅ 10-24 拍板归银行），一条把**有主**的地连楼收回
// （资产从物价指数的分子里被摘掉）。后者就是那句「增长不上限，但要有一点防止过快的平衡」
// 在这批里的落点 —— 它是这池新闻里目前唯一的通缩闸门。
//
// 这一组全走 `common::broadcast`（点名播那一条，不碰随机流）：这里断言的都是**前后差值**
// （少了哪一块、谁付了多少），而「反复踩到抽中」会把前面那几轮的副作用一起算进差值里。

/// 全体公司的当月账页读数 `(股票id, 条目数, 净额)` —— 股票名单从资产里读，不在测试里抄。
///
/// 它问的是 ✅ 10-24 那句拍板的后半句：公有地拍卖的所得**不进损益账**，所以它不会在 15 号
/// 按持股比例变成一笔分红。进了的话「归银行」就悄悄变成「归股东」，而那笔钱没人记得是谁的。
fn company_pages(app: &App) -> Vec<(String, usize, i64)> {
    let ids: Vec<String> = app
        .world()
        .resource::<Assets<StockSet>>()
        .get(&app.world().resource::<DataHandles>().stocks)
        .expect("股票池没加载")
        .0
        .iter()
        .map(|s| s.id.clone())
        .collect();
    let ledgers = app.world().resource::<CompanyLedgers>();
    ids.into_iter()
        .map(|id| {
            let page = ledgers.find(&id);
            (
                id,
                page.map_or(0, |p| p.entries.len()),
                page.map_or(0, |p| p.net()),
            )
        })
        .collect()
}

/// 那一整条街摆成「中心那块是他的两层楼、街尾那块无主」。
fn one_public_one_private(app: &mut App) {
    own(app, ANCHOR, Some(0), 2, false);
    own(app, 5, None, 0, false);
}

/// 两块都无主（要拍两场那一条用）。
fn two_public(app: &mut App) {
    own(app, ANCHOR, None, 1, false);
    own(app, 5, None, 0, false);
}

/// 点名播一条，返回它那一行流水。`broadcast` 返回 `false` 就是 id 写错 —— 必须当场响，
/// 否则症状是「后面每一条断言都绿」：什么都没播，所以什么都没变。
fn fired(app: &mut App, p0: Entity, id: &str) -> String {
    assert!(
        broadcast(app, p0, id),
        "{id} 不在新闻池里（改名没改到这条用例）"
    );
    settle(app, 2);
    news_lines(app).pop().expect("刚播过一条")
}

/// 此刻该出价的那一位（与 `tests/auction.rs` 同名那条同一形状：先读机器，再替**那一位**按按钮）。
fn bidder(app: &App) -> Entity {
    lot_of(app)
        .expect("前置：拍卖正在进行")
        .bidder()
        .expect("名单上没人可问的那一帧不该被读到")
}

/// 替「此刻该出价的那一位」喊一声加一档，返回他这一刻认下的数。
fn bid(app: &mut App) -> i64 {
    let amount = lot_of(app).expect("前置：拍卖正在进行").next_amount();
    let player = bidder(app);
    auction_ask(app, player, BidMove::Raise);
    amount
}

/// 公有地那一槌只拍**无主**的那几块：中心那块是他的，所以清单里只有街尾那块。
///
/// 最容易写成的那一版是「范围内的每一块都挂上台」，而它的症状是玩家看见自己的地被挂牌。
/// 这条新闻不替业主卖地 —— `OpenDeny::Private` 是同一句话在拍卖机那一侧的另一半，
/// 而这里钉的是新闻这一侧：**凭据那份清单与真挂上台的是同一个集合**。
#[test]
fn a_public_auction_lists_only_the_ownerless_plots() {
    let (mut app, p0) = step_onto_once();
    one_public_one_private(&mut app);
    let line = fired(&mut app, p0, "news_013");
    let (center, lands) = ground(&line);
    assert_eq!(center, format!("L{ANCHOR}"), "锚点是脚下最近那块地：{line}");
    assert_eq!(
        lands,
        vec!["L5".to_string()],
        "有主的 L4 不该出现在公有地名单上：{line}"
    );
    assert!(
        line.contains("Auctioned"),
        "凭据要说得出这一槌敲下去了：{line}"
    );

    let opened: Vec<String> = ledger(&app)
        .into_iter()
        .filter(|l| l.starts_with("Auction(L5:"))
        .collect();
    assert_eq!(opened.len(), 1, "无主那块该开出一场：{opened:?}");
    // `by bank` = 没有开槌的人，`from -` = 没有原主。两格各挡一种冒充：破产清算那条是
    // `by bank,from P?`（钱同样归银行，但它说的是「死者留下的」而不是「政府挂牌」）
    assert!(
        opened[0].contains("by bank,from -"),
        "公有地的开场行要念得出「公家挂牌、无原主」：{}",
        opened[0]
    );
    assert_eq!(
        plot_state(&mut app, ANCHOR),
        (Some(0), 2, false),
        "他那块地连层数都没动：{line}"
    );
    assert_cash_audit(&mut app, START);
}

/// 一块公有地真被拍走时：**买家的钱少了、没有别人的钱多、那块地归他、公司账页一行没变**。
///
/// 这四句合起来才是 ✅ 10-24 那两句拍板的可执行形式。「归银行」在代码里不是一个收款人，
/// 而是一笔**没有 credit 的 debit** —— 所以断言不能只写「买家付了」（那笔钱也可能进了某位
/// 玩家或某个公司账页，而那两种都不是这条新闻说的事）。
#[test]
fn the_public_auction_money_leaves_the_table() {
    let (mut app, p0) = step_onto_once();
    let rows = roster(&mut app);
    let ps: Vec<Entity> = rows.iter().map(|(e, ..)| *e).collect();
    let ids: Vec<u32> = rows.iter().map(|(_, id, ..)| *id).collect();
    one_public_one_private(&mut app);
    // 脚手架的自动玩家对拍卖一律答「放弃」（长跑用例靠它不被挂死），而这一条要真拍走一块地
    set_policy(&mut app, DecisionPolicy::Hold);
    let line = fired(&mut app, p0, "news_013");

    let lot = lot_of(&app).expect("公有地该开出一场拍卖：{line}");
    assert_eq!(lot.gavel, Gavel::Public, "这一槌是公家敲的：{line}");
    assert_eq!(
        (lot.seller, lot.by()),
        (None, None),
        "既没有原主也没有开槌的人 —— 两种冒充都不许：{line}"
    );
    let price = lot.opening;
    let before: Vec<i64> = ps.iter().map(|e| cash_of(&mut app, *e)).collect();
    let pages = company_pages(&app);

    let buyer = bidder(&app);
    let paid = bid(&mut app);
    assert_eq!(paid, price, "没人认价时，第一个合法喊价就是起拍本身");
    // 其余每一位放弃，直到落槌（两人桌：P0 认下起拍、P1 放弃，当场成交）
    for _ in 0..6 {
        if lot_of(&app).is_none() {
            break;
        }
        let other = bidder(&app);
        auction_ask(&mut app, other, BidMove::Withdraw);
    }
    assert!(lot_of(&app).is_none(), "这一场该已经散了");
    settle(&mut app, 2);

    let at = ps.iter().position(|e| *e == buyer).expect("买家在名单上");
    let after: Vec<i64> = ps.iter().map(|e| cash_of(&mut app, *e)).collect();
    assert_eq!(after[at], before[at] - price, "买家付的就是落槌那一个数");
    for i in 0..ps.len() {
        if i == at {
            continue;
        }
        assert_eq!(
            after[i], before[i],
            "除买家之外没有人的钱变多 —— 那笔钱离开了牌桌（归银行＝没有 credit 的那一步）"
        );
    }
    assert_eq!(plot_state(&mut app, 5).0, Some(ids[at]), "那块公有地归买家");
    assert_eq!(
        company_pages(&app),
        pages,
        "这笔所得没进任何公司的当月账：它不会在 15 号变成一笔分红"
    );
    let want = format!("Sold(P{}:L5,{price},to bank)", ids[at]);
    assert!(
        ledger(&app).contains(&want),
        "流水要说得出这笔钱去了哪：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, START);
}

/// 两块公有地是**两场**拍卖，一场散完才开下一场；机器空了才把回合交出去。
///
/// 这条钉的是「新闻挂上来的那几块走的是同一台轮转式机器」：一次挂两块而机器只开一场，
/// 第二块就永远站在台上没人问。`Auction::holds_turn` 那条判据同时挡着「拍完就走人」那种
/// 收尾（坑 37 那一族：两处写同一个 `NextState` 时不许赌顺序，要让占着回合的那一侧先表态）。
#[test]
fn two_public_plots_go_up_one_at_a_time_and_the_turn_survives_it() {
    let (mut app, p0) = step_onto_once();
    two_public(&mut app);
    let line = fired(&mut app, p0, "news_013");
    let (_center, lands) = ground(&line);
    assert_eq!(
        lands,
        vec![format!("L{ANCHOR}"), "L5".to_string()],
        "两块都无主，两块都该在名单上：{line}"
    );

    // 一帧一帧喂：机器一空、且回合机已经把「等一个人回答」那一格交出去才停。
    // 判据里带上 phase 而不是多给几帧：把交还会晚一帧这件事写成 sleep，那条 hang 就只在
    // 帧数不够时红 —— 而这里要问的恰恰是「它到底交不交还」。
    // 多出来的帧会让下一位玩家开始走路、踩到别的东西，而这一条要数的是「这台机器开了几场」。
    let mut handed_back = false;
    for _ in 0..60 {
        settle(&mut app, 1);
        let idle = auction_of(&app);
        if idle.lot.is_none()
            && idle.standby.is_empty()
            && idle.liquidation.is_empty()
            && phase(&app) != TurnPhase::WaitDecision
        {
            handed_back = true;
            break;
        }
    }
    assert!(
        handed_back,
        "60 帧之后这台机器还没把回合交出去：{:?}",
        ledger(&app)
    );
    let lines = ledger(&app);
    let opened: Vec<&String> = lines.iter().filter(|l| l.starts_with("Auction(")).collect();
    assert_eq!(opened.len(), 2, "两块就该开两场（一次一块）：{opened:?}");
    assert_eq!(
        lines.iter().filter(|l| l.starts_with("Unsold(")).count(),
        2,
        "全员放弃＝两块都流标：{lines:?}"
    );
    let idle = auction_of(&app);
    assert!(
        idle.lot.is_none() && idle.standby.is_empty() && idle.liquidation.is_empty(),
        "这台机器转完就该彻底空着：{idle:?}"
    );
    for land in [ANCHOR, 5] {
        assert_eq!(
            plot_state(&mut app, land).0,
            None,
            "流标不转手：L{land} 还是无主"
        );
    }
    assert_ne!(
        phase(&app),
        TurnPhase::WaitDecision,
        "拍完两块之后回合不该还停在「等一个人回答」上（回合机被新闻挂死的形状）"
    );
    assert_cash_audit(&mut app, START);
}

/// 强制征收：**房屋同灭、土地收回、业主一分补偿都拿不到**，而全场现资产正好少那一块的价钱。
///
/// 最后那半句是这条用例的全部理由，也是 ✅ 10-24 那句「有一点平衡防止过快」的落点：
/// 物价指数读的是「全员现资产总和」（`economy::price_index`），而这里动的是**资产本身** ——
/// 楼平了、业主换成无主，那一块就从分子里消失了。它不是转账（没有 credit），也不是税
/// （税收的是钱，地还在他名下）。差值按**确切的一个数**断言而不是「小了一点」：那个数由
/// `market_value` 现读（起拍价同一条式子），改地块数据时先在这里红。
#[test]
fn a_requisition_takes_the_house_and_the_land_and_pays_nothing() {
    let (mut app, p0) = step_onto_once();
    let p1 = roster(&mut app)[1].0;
    own(&mut app, ANCHOR, Some(0), 3, false);
    own(&mut app, 5, Some(1), 2, false);
    // 把指数摆回 1.0：`market_value` 与 `total_assets` 两处要按同一条单位读，
    // 而前面那几轮的补贴会把指数推高（缩放本身另有用例钉）
    set_price_index(&mut app, 1.0);
    let price = market_value(&mut app, ANCHOR);
    let total_before = total_assets(&mut app, 1.0);
    let owner_cash = cash_of(&mut app, p0);
    let other_cash = cash_of(&mut app, p1);

    let line = fired(&mut app, p0, "news_014");
    let (center, lands) = ground(&line);
    assert_eq!(center, format!("L{ANCHOR}"), "锚点是脚下最近那块地：{line}");
    assert_eq!(
        lands,
        vec![format!("L{ANCHOR}")],
        "「一栋」就是阶梯上 `One` 那一格，隔壁那块不在范围内：{line}"
    );
    assert!(
        line.contains("Razed { lands: 1, seized: 1 }"),
        "一块、且那一块连地收回：{line}"
    );
    assert_eq!(
        plot_state(&mut app, ANCHOR),
        (None, 0, false),
        "楼平了、地收回：{line}"
    );
    assert_eq!(
        plot_state(&mut app, 5),
        (Some(1), 2, false),
        "范围只那一块，隔壁那位不动：{line}"
    );
    assert!(
        !line.contains("Gained") && !line.contains("Lost"),
        "征收不赔钱也不收款 —— 这一条上不该有任何一笔现金：{line}"
    );
    assert_eq!(cash_of(&mut app, p0), owner_cash, "业主的钱包一分没动");
    assert_eq!(cash_of(&mut app, p1), other_cash, "旁人更没动");
    assert_eq!(
        total_assets(&mut app, 1.0),
        total_before - price,
        "全场现资产该正好少了那一块地的价钱（这就是那条通缩闸门的单位）：{line}"
    );
    assert_cash_audit(&mut app, START);
}

// ===== P11c②：机构档（停牌 / 停止放款）与地价调跌 =====

/// 世界上这一家还剩几天停摆。读的是 `Books`，所以与规则层那句判据同一个来源 ——
/// 测试自己数天数就会与世界上那个数各漂一次。
fn suspended_for(app: &mut App, what: Institution) -> u32 {
    let out = std::sync::Arc::new(std::sync::Mutex::new(None));
    let sink = out.clone();
    run_op(app, move |books, _| {
        *sink.lock().unwrap() = Some(books.session.suspensions.days_left(what));
    });
    out.lock().unwrap().expect("Books 读得到停摆那一格")
}

/// 把锚点往前挪，直到「今天」是个开市日。
///
/// 跑回合必然撞上周末与节假日（P2b 之后日历是真的），而这条用例问的是
/// 「停摆到期之后放不放行」—— 被日历挡在外面的那一笔答不了这个问题。
/// 挪不到就响：那是夹具或节假日表错了，不是规则错了（比悄悄少一条断言好）。
fn open_the_market(app: &mut App) {
    for _ in 0..10 {
        let tick = app.world().resource::<MarketClock>().tick;
        let epoch = app.world().resource::<GameSession>().epoch;
        let closed = app
            .world()
            .resource::<HolidaySet>()
            .is_market_closed(date_of(tick, epoch));
        if !closed {
            return;
        }
        app.world_mut().resource_mut::<GameSession>().epoch = epoch.add_days(-1);
    }
    panic!("往前挪了十天还找不到一个开市日");
}

/// 停牌这几天：**下单不受理、行情也不往前走**，而到期自己复牌。
///
/// 两条都要，因为「停牌」在玩家耳朵里是两件事合成的一件：他按不动那颗买按钮，
/// 而复牌时看到的价不该是他没参与却变了的价。所以这里挡的不只是窗口，还有 `advance_prices`。
///
/// 前置那两笔是**非空跑守卫**（本项目犯过五次的那条）：不先证明「开盘时这一笔真的成交、
/// 一个 tick 价格真的会动」，下面那两条否定式断言就是永远不响的断言。
#[test]
fn a_suspended_market_refuses_orders_and_freezes_the_prices() {
    let (mut app, p0) = step_onto_once();
    // 日历钉死：锚点由会话种子抽，随机落在周末或节假日时**前置那一笔**就会被休市挡掉，
    // 症状看着像「停牌这条新闻没生效」，而其实是夹具根本没开盘（P11b② 那条 `pin_calendar`
    // 的同一条学费，这次是为了开盘而不是为了躲红包）
    pin_calendar(&mut app);
    let id = "stock_001";
    make_current(&mut app, p0);

    trade(&mut app, p0, id, TradeSide::Buy, 1);
    settle(&mut app, 3);
    assert_eq!(shares_of(&mut app, p0, id), 1, "前置：开盘时这一笔该成交");
    let live = price_of(&mut app, id);
    run_market_tick(&mut app);
    assert_ne!(
        price_of(&mut app, id),
        live,
        "前置：一个 tick 的价格会走随机游走（否则下面那条「冻住」是空的）"
    );

    let line = fired(&mut app, p0, "news_016");
    // 机构档的「办成了什么」全在这一格里（哪一家、剩几天），它不带 `EffectOutcome` 清单 ——
    // 那一族没有承受者也没有地块，再列一份就是同一个数说两遍
    assert!(line.contains("机构StockMarket[剩3天]"), "{line}");
    assert_eq!(suspended_for(&mut app, Institution::StockMarket), 3);

    trade(&mut app, p0, id, TradeSide::Buy, 1);
    settle(&mut app, 3);
    assert_eq!(
        shares_of(&mut app, p0, id),
        1,
        "停牌那天还受理了一笔下单 —— 规则层没读停摆那一格"
    );

    let frozen = price_of(&mut app, id);
    for _ in 0..3 {
        run_market_tick(&mut app);
    }
    assert_eq!(
        price_of(&mut app, id),
        frozen,
        "停牌期间行情动了：玩家复牌时看到的是一张他没参与却变了的价"
    );

    // 到期那一半：撤掉载体（不然路上还会再抽到新闻，把停摆重新续上），再跑过三个回合末
    retile(&mut app, NEWS, TileKind::Road);
    let want = turn_count(&app) + 5;
    play_turns(&mut app, want);
    assert_eq!(
        suspended_for(&mut app, Institution::StockMarket),
        0,
        "五个回合末之后早该复牌（一天＝一个回合末，与涨价同一把尺）"
    );
    make_current(&mut app, p0);
    open_the_market(&mut app);
    trade(&mut app, p0, id, TradeSide::Buy, 1);
    settle(&mut app, 3);
    assert_eq!(
        shares_of(&mut app, p0, id),
        2,
        "复牌之后买不动 —— 到期没真的放行"
    );
    assert_cash_audit(&mut app, START);
}

/// 停止放款**只挡借**：存取款与还款照办。
///
/// 这条用例的重心在后半：一条「银行停摆」的新闻很容易整块短路掉柜台，而那样一来
/// 欠着钱的人平白多一笔还不上的逾期 —— 没有任何一条规则配得上那笔惩罚。
/// 所以「借不动」与「还得掉」两条必须同时红过才算有牙（red-check 里真的一起翻过）。
#[test]
fn a_suspended_bank_stops_lending_but_not_repaying_or_cash_moves() {
    let (mut app, p0) = ready_on(6, 2, DecisionPolicy::Hold);
    own(&mut app, ANCHOR, Some(0), 2, false);
    roll(&mut app, p0);
    settle(&mut app, 8);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::BankTeller)),
        "前置没做到：踩到银行格却没开出柜台"
    );

    bank_ask(&mut app, p0, BankOp::Borrow, 5_000);
    settle(&mut app, 3);
    assert_eq!(bank_of(&mut app, p0).1, 5_000, "前置：开盘时借得动");

    let line = fired(&mut app, p0, "news_017");
    assert!(
        line.contains("机构Bank[剩3天]"),
        "凭据要说得出停的是银行、几天：{line}"
    );

    bank_ask(&mut app, p0, BankOp::Borrow, 5_000);
    settle(&mut app, 3);
    assert_eq!(
        bank_of(&mut app, p0).1,
        5_000,
        "停止放款还放得出钱，这条新闻就白播了"
    );

    bank_ask(&mut app, p0, BankOp::Repay, 5_000);
    settle(&mut app, 3);
    assert_eq!(
        bank_of(&mut app, p0).1,
        0,
        "还款被一起挡了 —— 停摆期间不许还钱是惩罚"
    );

    bank_ask(&mut app, p0, BankOp::Deposit, 1_000);
    settle(&mut app, 3);
    assert_eq!(
        bank_of(&mut app, p0).0,
        1_000,
        "存款也被挡了 —— 银行没有关门"
    );
    assert_cash_audit(&mut app, START);
}

/// 同一家再挨一条：**不累加**，答案还是那一个数。
///
/// 「更久的那一张赢」这一格由 `institution.rs` 的单测钉（那里能摆出 5 与 3 两个数），
/// 这一条只钉另一件在集成上才会显形的事：凭据与世界上那个数是同一个,
/// 播两次不会把停摆读成 6 天，也不会把第二条播成一句静默的空标题。
#[test]
fn two_suspensions_of_the_same_institution_do_not_stack() {
    let (mut app, p0) = step_onto_once();
    let first = fired(&mut app, p0, "news_016");
    let second = fired(&mut app, p0, "news_016");
    assert!(first.contains("剩3天"), "{first}");
    assert!(
        second.contains("剩3天"),
        "同一家挨两条不该累加成 6 天：{second}"
    );
    assert_eq!(
        suspended_for(&mut app, Institution::StockMarket),
        3,
        "凭据念 3 而世界上是别的数 —— 两处各算了一遍"
    );
    // 另一家不受牵连：两条停摆写在同一份资源里，串台是最省事的写错方式
    assert_eq!(suspended_for(&mut app, Institution::Bank), 0);
}

/// 地价调跌：整条街盖上 `PriceDrop`，而**后播的那条盖掉前一条**（单槽位，不留优先级表）。
///
/// 租金那个数（÷2）由 `economy.rs` 的单测钉；这里钉的是新闻这一侧的两件事：
/// 清单是脚下那一整条街，以及「涨价 ⇄ 调跌」在结构上就是替换。
#[test]
fn a_price_drop_news_stamps_the_street_and_a_later_hike_replaces_it() {
    let (mut app, p0) = step_onto_once();
    own(&mut app, ANCHOR, Some(1), 3, false);
    own(&mut app, 5, Some(1), 3, false);

    let line = fired(&mut app, p0, "news_015");
    let (_center, lands) = ground(&line);
    assert_eq!(
        lands,
        vec!["L4".to_string(), "L5".to_string()],
        "整条街都调跌：{line}"
    );
    for land in STREET {
        let (status, days) = status_of(&mut app, land).expect("刚播完，这一格该挂着调跌");
        assert_eq!(status, LandStatus::PriceDrop, "L{land} 盖的是调跌");
        assert!(
            days > 0 && days <= 5,
            "L{land} 的天数在条目那一格之内：{days}"
        );
    }

    // 后盖前：隔壁那条街刚涨过价，这一条把它换掉而不是叠上去
    fired(&mut app, p0, "news_006");
    assert_eq!(
        status_of(&mut app, ANCHOR).map(|(s, _)| s),
        Some(LandStatus::PriceHike),
        "涨价没盖掉调跌 —— 那就是两张状态表在租金那一格里打架"
    );
    assert_cash_audit(&mut app, START);
}
