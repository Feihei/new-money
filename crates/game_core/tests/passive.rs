//! P13b② 的验收：那道门自己问、自己落、自己把回合交回去。
//!
//! 这份文件只管**门自己**的那几条性质（问序、链长、拒绝的形状、回合押住与交还）。
//! 「事件交给门」那五处来路各自的验收留在它们本来的文件里（`tests/detention.rs` 的命运、
//! `tests/magic.rs` 的法事矩阵、`tests/stay.rs`/`turn_around.rs`/`occupancy.rs`
//! 的「不可免那一行连问都不问」、`tests/cards.rs` 的免罪那一问），因为一条来路的收据
//! 形状归那条来路管，而门在这里要问的是**另一件事**：同一句判据在世界上只有一个位置。
//!
//! 嫁祸那一族在这里第一次有真用户，所以它也是「决策主体能在回合内跳动」那句话
//! （PLAN §五 第 2 条，10-27 拍板）唯一的可执行形态：窗口挂在**不是当前回合玩家**的人身上，
//! 而回合真的停在他答完为止。

mod common;

use bevy::app::App;
use bevy::asset::Assets;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    Bankrupt, CardAim, CardSet, DataHandles, Detention, DetentionPlace, GameRules, GameSeed,
    GameSession, GameSetup, PLAYER_COLORS, Pending, PlayerSetup, Term, TurnPhase,
};

/// prototype 上的命运格（5 号，P10d② 重排后从 3 顺移过来）。写死而不是现读，与
/// `tests/detention.rs` 同一条口径：那一格是这张盘的**夹具**，不是被测试的那件事。
const FATE: u32 = 5;

/// 三人阵容。链最长两跳那条用例要「接住事的那一位」与「第三个人」是两个不同的人，
/// 而默认两人盘上 P1 只能把事转回出牌的 P0（那也是一种形状，另有用例走它）。
fn trio() -> GameSetup {
    GameSetup {
        players: ["甲", "乙", "丙"]
            .iter()
            .enumerate()
            .map(|(i, name)| PlayerSetup {
                name: (*name).to_string(),
                color: PLAYER_COLORS[i],
                gender: game_core::player::Gender::Male,
            })
            .collect(),
    }
}

/// 命运池里某个 id 的下标（与 `item_index` 同一条理由：按名字拿卡，不靠数组位置）。
fn fate_index(app: &App, id: &str) -> u32 {
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().fate;
    sets.get(handle)
        .expect("命运牌没加载")
        .0
        .iter()
        .position(|c| c.id == id)
        .unwrap_or_else(|| panic!("fate_cards.json 里没有 {id}")) as u32
}

/// 装配到「P0 的回合、掷骰之前」，并按座位发好手里的被动卡。
///
/// 自动应答策略摆成 `Hold`：那一位没答之前，脚手架不许替他答一句 —— 而 `Hold` 也意味着
/// 命运收据那一格要自己清（只有走到那一步的用例才需要，这里多数停在门那一问上）。
/// 发牌一律走 [`deal`]（`deal_to` 那条公开入口），于是手牌上限那条规则也在前置里。
fn table(holds: &[(usize, &[&str])]) -> (App, [Entity; 3]) {
    let mut app = boot(trio(), GameRules::instant(), GameSeed::default());
    app.insert_resource(DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    let rows = roster(&mut app);
    let seats: [Entity; 3] = [p0, rows[1].0, rows[2].0];
    for (seat, ids) in holds {
        for id in ids.iter() {
            let card = item_index(&app, id);
            deal(&mut app, seats[*seat], card.0);
        }
    }
    make_current(&mut app, p0);
    settle(&mut app, 2);
    (app, seats)
}

/// 替 P0 把一张陷害卡打到 `target` 头上 —— 真出牌入口，不是直接构造 `Affliction`。
fn frame(app: &mut App, from: Entity, target: Entity) {
    let card = item_index(app, "item_frame");
    deal(app, from, card.0);
    play_card(app, from, card.0, Some(CardAim::Player(target)));
    settle(app, 2);
}

/// 还在被拘留就返回剩余回合数。
fn turns_left(app: &App, player: Entity) -> Option<u32> {
    detention_of(app, player).and_then(|d| d.term.turns())
}

fn shift_left(app: &App, player: Entity) -> u32 {
    passive_in_hand(app, player, PassiveCard::Shift)
}

#[test]
fn the_offset_is_asked_first_and_a_shift_card_survives_that_question() {
    // 验收④（问序可测）在「事」这一侧的形状：手里同时有免罪与嫁祸时**先问免罪**，
    // 而免掉之后嫁祸那张不消耗。PLAN §五 第 4 条的原话是「先抵消后转嫁」，
    // 它在账那一侧的同一条用例（免费卡 ⇄ 嫁祸）归 ③。
    let (mut app, [p0, p1, _p2]) = table(&[(1, &["item_insulate", "item_shift"])]);
    frame(&mut app, p0, p1);

    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::JailTrip)),
        "两张卡都在手里时，第一问必须是抵消那一问"
    );
    assert_eq!(gate_depth(&app), 1, "一张卡的一件事只有一份信封");

    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None, "免掉了就没有坐牢这回事");
    assert_eq!(forgive_in_hand(&app, p1), 0, "免罪那一张花掉了");
    assert_eq!(
        shift_left(&app, p1),
        1,
        "第二问没被问到，那张嫁祸就该原样留着"
    );
    let log = ledger(&app);
    assert!(
        !log.iter().any(|l| l.starts_with("Blame(")),
        "没有转嫁就不该有一条换人的流水：{log:?}"
    );
}

#[test]
fn a_shift_moves_the_frame_onto_the_one_he_names_and_the_trail_says_so() {
    // 验收②的前半（事这一侧）+ ③的第一跳：收据说的必须是**实际挨那一下的人**。
    // `CardPlayed.victim` 在出牌那一刻填的是 P1，而门答完之后 `finish_played` 拿的是
    // `job.landed()` —— 这一条就是那一句的实现证据。
    let (mut app, [p0, p1, p2]) = table(&[(1, &["item_shift"])]);
    frame(&mut app, p0, p1);

    // 他手里只有嫁祸卡：第一问（抵消）由表答「手里没有那张」而**跳过**，直接问第二问
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Shift, PassiveEvent::JailTrip)),
        "没有免罪卡时那一问不该挂出来，而该直接问嫁祸 —— 先验前置，不然是空跑"
    );

    answer_gate(&mut app, p1, PassiveMove::Shift { to: p2 });
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), None, "转出去的那一位不该再挨它");
    assert_eq!(turns_left(&app, p2), Some(5), "挨那一下的是被点名的那位");
    assert_eq!(shift_left(&app, p1), 0, "嫁祸那一张花掉了");
    let log = ledger(&app);
    assert!(
        log.contains(&"Blame(P1→P2的坐牢)".to_string()),
        "事换人要单独留一条：{log:?}"
    );
    assert!(
        log.contains(&"Play(P0->P2:item_frame)".to_string()),
        "出牌那条流水的主语是出牌人，承受者必须是**真挨的那位**：{log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("Detain(P2@")),
        "拘留那条凭据也要落在被点名的人身上：{log:?}"
    );
}

#[test]
fn the_chain_stops_at_two_hops_while_the_second_still_shrugs_off_a_forgive() {
    // 验收③的全部内容：A 转给 B 之后，B 的嫁祸**不再被问**（[`Job::shifted`] 那一格），
    // 而 B 的免罪**照样吃**。PLAN §五 第 3 条的原话是「这条不是例外，是门的自然行为」，
    // 所以这里断言的是**问出来的那一串**，而不是某一句特例话。
    let (mut app, [p0, p1, p2]) =
        table(&[(1, &["item_shift"]), (2, &["item_insulate", "item_shift"])]);
    frame(&mut app, p0, p1);
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Shift, PassiveEvent::JailTrip)),
        "前置：P1 手里只有嫁祸卡，所以第一问就是它"
    );

    answer_gate(&mut app, p1, PassiveMove::Shift { to: p2 });
    settle(&mut app, 2);

    assert_eq!(
        gate_asking(&app),
        Some((p2, Ask::Offset, PassiveEvent::JailTrip)),
        "接住的那一位回到**第一问**：他的免罪照样被问"
    );
    // 窗口那一格也得跟着换人。测试答话走的是 `gate_asking`（门自己的读数），而面板与
    // 派发走的是那个槽 —— 只钉前者的话，「挂在已经脱身的那位身上」这一格照样全绿，
    // 而真人点下去什么都不会发生（10-27 的冒烟实拍到的正是它）
    assert_eq!(
        pending_of(&app),
        Some((p2, Pending::Passive)),
        "事转到谁，窗口就挂在谁身上"
    );

    answer_gate(&mut app, p2, PassiveMove::Decline);
    settle(&mut app, 2);

    assert_eq!(gate_asking(&app), None, "转过一次之后不再有第二跳那一问");
    assert_eq!(turns_left(&app, p2), Some(5), "两跳到头，事落在他头上");
    assert_eq!(
        shift_left(&app, p2),
        1,
        "他身上那张嫁祸从头到尾没被问过，也就不该被动过"
    );
    assert_eq!(
        forgive_in_hand(&app, p2),
        1,
        "说「不用」不消耗任何一张卡（验收⑤）"
    );
    let log = ledger(&app);
    assert_eq!(
        log.iter().filter(|l| l.starts_with("Blame(")).count(),
        1,
        "链只有一跳：{log:?}"
    );
}

#[test]
fn a_shift_card_in_hand_is_not_a_reason_to_ask_the_second_question() {
    // 「已转过一次」那一格的**牙齿**在哪：面板那一排候选按钮用的是 `blame_candidates`，
    // 它不区分「这人能不能接第二跳」—— 判据全在门里（`next_question` 读 `job.shifted`）。
    // 于是这里两件事都要拍到：他手里有嫁祸卡**也不会被问第二跳**，而一句认不出来的回答
    // （门问「免不免」而他答「转给谁」）不替他做决定 —— 那一问继续挂着，卡不落、事也不落。
    let (mut app, [p0, p1, p2]) = table(&[(1, &["item_shift"]), (2, &["item_insulate"])]);
    frame(&mut app, p0, p1);
    answer_gate(&mut app, p1, PassiveMove::Shift { to: p2 });
    settle(&mut app, 2);
    assert_eq!(
        gate_asking(&app),
        Some((p2, Ask::Offset, PassiveEvent::JailTrip)),
        "前置：第二跳回到第一问"
    );

    // 中途塞给他一张嫁祸卡：他手里现在两样都有，而那一问**不该**因此翻成「转给谁」
    let card = item_index(&app, "item_shift");
    deal(&mut app, p2, card.0);
    settle(&mut app, 2);
    assert_eq!(shift_left(&app, p2), 1, "前置：他手里现在真有一张嫁祸卡");
    assert_eq!(
        gate_asking(&app),
        Some((p2, Ask::Offset, PassiveEvent::JailTrip)),
        "转过一次之后，手里多一张嫁祸也不问第二跳"
    );

    // 硬答一句门没问的话：这一问继续挂着，什么都不落、什么都不花
    answer_gate(&mut app, p2, PassiveMove::Shift { to: p0 });
    settle(&mut app, 2);
    assert_eq!(
        gate_asking(&app),
        Some((p2, Ask::Offset, PassiveEvent::JailTrip)),
        "答非所问不该替他做决定"
    );
    assert_eq!(
        turns_left(&app, p2),
        None,
        "一句没认出来的回答不该让事落下来"
    );
    assert_eq!(shift_left(&app, p2), 1, "没问那一问就不该花掉那张嫁祸卡");
    assert_eq!(
        forgive_in_hand(&app, p2),
        1,
        "那一问还挂着，免罪那张也就还在"
    );

    // 而门没坏：答对它问的那一句，事情照常办完
    answer_gate(&mut app, p2, PassiveMove::Use);
    settle(&mut app, 2);
    assert_eq!(turns_left(&app, p2), None, "免罪那张把这一件挡掉了");
    assert_eq!(forgive_in_hand(&app, p2), 0, "花的就是那一问管的那张卡");
    assert_eq!(shift_left(&app, p2), 1, "嫁祸那张到最后也没被动过");
}

#[test]
fn two_shift_cards_are_asked_once_and_only_one_is_spent() {
    // 验收⑨（两张同类卡只问一次）在嫁祸那一侧的形状。免罪那一侧由 `tests/cards.rs` 的
    // `two_forgive_cards_are_asked_once_and_only_one_is_spent` 钉，两条各钉一半：
    // 合成一条的话「按张数循环」那种实现只在其中一格上红，而另一格照样绿。
    let (mut app, [p0, p1, p2]) =
        table(&[(1, &["item_shift", "item_shift"]), (2, &["item_shift"])]);
    assert_eq!(shift_left(&app, p1), 2, "前置：两张都在 P1 手里");
    frame(&mut app, p0, p1);

    assert_eq!(gate_depth(&app), 1, "两张卡不该问两遍");
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Shift, PassiveEvent::JailTrip)),
        "前置：问到的是 P1 的嫁祸那一问"
    );

    answer_gate(&mut app, p1, PassiveMove::Decline);
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), Some(5), "说不用，那一下照原样落");
    assert_eq!(shift_left(&app, p1), 2, "不用那一问，两张都留着");

    // 下一次照样被问（「按次」而不是「问过就闭嘴」），而这次真转出去：只少一张
    frame(&mut app, p0, p2);
    assert_eq!(
        gate_asking(&app),
        Some((p2, Ask::Shift, PassiveEvent::JailTrip)),
        "P2 手里一张，也被问一遍"
    );
    answer_gate(&mut app, p2, PassiveMove::Shift { to: p0 });
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p2), None, "转出去的那一位不留痕");
    assert_eq!(
        turns_left(&app, p0),
        Some(5),
        "他转给了出牌的那一位，而那一格只认名单不认谁开的场"
    );
    assert_eq!(shift_left(&app, p2), 0, "一次转嫁花一张");
    assert_eq!(shift_left(&app, p1), 2, "上面那一问没答成，两张还都在");
}

#[test]
fn a_shift_onto_a_gone_player_is_refused_and_costs_no_card() {
    // 名单的判据在门里（`shift_candidates` 借 `Books::solvent`），不在面板上：面板按同一句
    // 摆按钮，而这里把它**硬塞**一次 —— 办不成的话必须原样收回，而不是
    // 「卡没了而事还在甲头上」（P3d④a ⑤ 那条纪律的第二个用户）。
    let (mut app, [p0, p1, p2]) = table(&[(1, &["item_shift"])]);
    insert(&mut app, p2, Bankrupt);
    frame(&mut app, p0, p1);
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Shift, PassiveEvent::JailTrip)),
        "前置：场上还有 P0 一个可以点名的人，所以这一问挂得出来"
    );

    answer_gate(&mut app, p1, PassiveMove::Shift { to: p2 });
    settle(&mut app, 2);

    assert_eq!(turns_left(&app, p1), Some(5), "被拒的那一转不改承受者");
    assert_eq!(shift_left(&app, p1), 1, "被拒的那一转不花卡");
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Blame(")),
        "没转成就不该留一条换人的流水：{:?}",
        ledger(&app)
    );
}

#[test]
fn the_turn_stands_still_while_the_door_is_open_and_goes_back_when_answered() {
    // 验收⑥那句「窗口挂到我这儿、**回合停住**、答完再落」的中间那一半。
    // 它必须单独钉：门只挂窗口而不押回合的话，症状不是「报错」而是回合机替玩家往下走 ——
    // 于是那一下落在一个已经换了人的回合上。
    let (mut app, [p0, p1, _p2]) = table(&[(1, &["item_insulate"])]);
    let before = app.world().resource::<GameSession>().turn_count;
    frame(&mut app, p0, p1);

    assert_eq!(
        pending_of(&app),
        Some((p1, Pending::Passive)),
        "窗口挂在挨打的那位身上"
    );
    assert_eq!(phase(&app), TurnPhase::WaitDecision, "回合押在窗口那一格");
    assert!(
        app.world().resource::<Gate>().holds_turn(),
        "门自己说得出「回合是我押着的」"
    );

    // 没人答的那十几帧：世界该一动不动，而窗口每帧都还挂着
    settle(&mut app, 12);
    assert_eq!(
        app.world().resource::<GameSession>().turn_count,
        before,
        "窗口开着的那十几帧里一个回合都没走过去"
    );
    assert_eq!(
        pending_of(&app),
        Some((p1, Pending::Passive)),
        "押住的那十几帧里那一格还在（窗口只由 `offer` 挂上，也没有别人来清 —— RC④）"
    );
    assert_eq!(turns_left(&app, p1), None, "答完之前那一下不该落下");

    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "答完把回合交还给挂住它之前那一格（出牌人还没掷骰）"
    );
    assert_eq!(gate_asking(&app), None);
    assert_eq!(
        app.world().resource::<Gate>().resume,
        None,
        "交还之后不留「下一格该回哪」的余数"
    );

    // 回合机是活的，不是停在原地：换回自动应答，让 P0 真走完这一回合
    set_policy(&mut app, DecisionPolicy::DeclineAll);
    play_turns(&mut app, before + 1);
    assert_eq!(
        app.world().resource::<GameSession>().turn_count,
        before + 1,
        "门交还回合之后，掷骰那条路照样走得通"
    );
}

#[test]
fn a_fate_card_sending_him_abroad_can_be_shifted_to_somebody_else() {
    // 验收①的**另一列**：命运牌那一行（`AwayTrip`）既可免也可转，而这一条走真来路 ——
    // 踩命运格 → 抽到那张牌 → 门在落格那条链上把回合押住（可免那一半由
    // `tests/detention.rs` 的 `a_forgive_card_in_hand_shrugs_off_a_trip_abroad_too` 钉）。
    //
    // 「不可免的那一个出国」—— 踩到联合航空 —— 是表上的另一行 `AirlineTrip`，而它今天
    // 还没有门上的来路（公司到访那一处的主语是**一笔钱**，归 ③），所以那一行的答案目前
    // 只由 `passive::tests::the_same_trip_is_two_rows_from_one_place` 钉住。
    let mut app = boot(trio(), GameRules::instant(), seed_rolling_first(1));
    app.insert_resource(DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    let rows = roster(&mut app);
    let p2 = rows[2].0;
    place(&mut app, p0, FATE - 1);
    let card = item_index(&app, "item_shift");
    deal(&mut app, p0, card.0);
    let fate = fate_index(&app, "fate_008");
    stack_card(&mut app, CardKind::Fate, fate);
    settle(&mut app, 2);
    let p2_before = tile_of(&app, p2);

    // 掷骰、走完那一步，直到门开口（上限只为挡死循环，走不到就说明没问）
    roll(&mut app, p0);
    for _ in 0..30 {
        settle(&mut app, 1);
        if gate_asking(&app).is_some() {
            break;
        }
    }
    assert_eq!(
        gate_asking(&app),
        Some((p0, Ask::Shift, PassiveEvent::AwayTrip)),
        "命运牌的出国是可转嫁的那一行，而他手里只有嫁祸卡"
    );
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Passive)),
        "落格那条链上，窗口挂在抽到牌的人身上"
    );

    answer_gate(&mut app, p0, PassiveMove::Shift { to: p2 });
    settle(&mut app, 4);

    assert_eq!(detention_of(&app, p0), None, "转出去的那一位不该出国");
    assert_eq!(
        detention_of(&app, p2),
        Some(Detention {
            place: DetentionPlace::Away,
            term: Term::Turns(3)
        }),
        "被点名的那位替他挨这一趟"
    );
    assert_eq!(
        tile_of(&app, p2),
        p2_before,
        "而 `away` 那个地方不在棋盘上：他留在自己那一格，没人替他现编一个格子"
    );
    let log = ledger(&app);
    assert!(
        log.contains(&"Blame(P0→P2的出国旅游)".to_string()),
        "谁替谁出国要留一条流水：{log:?}"
    );
}

// ===== ③：钱那一侧的门 =====
//
// 上面那几条钉的是「事」，下面这三条钉的是「账」。它们同住这一份文件而不是搬去
// `tests/free_pass.rs`，因为这里问的是**门的性质**（链长、问序、谁答完就停），而那份文件
// 问的是那张卡自己的玩法（门槛、烧几张）。夹具也不同：那一份踩的是 P1 的地 —— 两人盘上
// 能接盘的不是业主本人没别人，「付钱的、收钱的、被点名的」是三个不同的人这一格读不出来。

/// 装配到「P0 的回合，下一步就踩到**丙**的 L0（名义租金 5000）」，并按座位发好手里的被动卡。
///
/// 走 P5 的遥控骰子而不是 `ready_on` 那颗种子骰：这份盘用的是默认种子，
/// 「下一颗正好是 1」押不住（`tests/news.rs` 的 `step_news_on` 同一条理由）。
fn rent_table(holds: &[(usize, &[&str])]) -> (App, [Entity; 3]) {
    let mut app = boot(trio(), GameRules::instant(), GameSeed::default());
    app.insert_resource(DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    let rows = roster(&mut app);
    let seats: [Entity; 3] = [p0, rows[1].0, rows[2].0];
    own(&mut app, 0, Some(2), 1, false);
    flat_rent(&mut app, 0, 5_000);
    let tile = land_tile(&app, 0);
    place(&mut app, p0, tile - 1);
    for (seat, ids) in holds {
        for id in ids.iter() {
            let card = item_index(&app, id);
            deal(&mut app, seats[*seat], card.0);
        }
    }
    make_current(&mut app, p0);
    (app, seats)
}

/// 让 P0 真走那一步，走到门把那笔过路费问出去为止。
fn step_onto_rent(app: &mut App, p0: Entity) {
    let want = app.world().resource::<GameSession>().turn_count + 1;
    grant_item(app, p0, ItemKind::RemoteDice, 1);
    use_remote_dice(app, p0, 1);
    settle(app, 2);
    play_turns(app, want);
}

/// 验收③（链最长两跳）在**钱**这一侧的形状：第二跳被 `shifted` 那一格挡掉，
/// 而不是被「他手里没有那张卡」挡掉 —— 所以接盘的那一位**也**得有一张嫁祸卡，
/// 否则这一条什么都没说（拿「没卡」冒充「链长到头」是这批用例最容易犯的错）。
#[test]
fn a_shifted_bill_lands_on_the_one_he_names_and_stops_there() {
    let (mut app, [p0, p1, p2]) = rent_table(&[(0, &["item_shift"]), (1, &["item_shift"])]);
    step_onto_rent(&mut app, p0);

    assert_eq!(
        gate_asking(&app),
        Some((p0, Ask::Shift, PassiveEvent::Rent)),
        "他手里没有免费卡，第一问落空 → 门自己跳到第二问"
    );

    answer_gate(&mut app, p0, PassiveMove::Shift { to: p1 });
    settle(&mut app, 2);

    assert_eq!(
        gate_asking(&app),
        None,
        "接住的那位不该再被问一遍 —— 挡他的是『已经转过一次』，不是『他没有卡』"
    );
    assert_eq!(cash_of(&mut app, p0), 20_000, "转出去的人一分不付");
    assert_eq!(cash_of(&mut app, p1), 15_000, "掏钱的是接盘的那位");
    assert_eq!(
        cash_of(&mut app, p2),
        25_000,
        "收款方不换：丙照样收到这笔租"
    );
    assert_eq!(
        shift_left(&app, p1),
        1,
        "他那张嫁祸卡原样留着，一句都没问过他"
    );
    let log = ledger(&app);
    assert!(
        log.contains(&"Rent(P1->P2@L0$5000)".to_string()),
        "收据说的是**实际付钱的人**付给谁：{log:?}"
    );
    assert!(
        log.iter().any(|l| l.starts_with("Blame(P0→P1")),
        "「谁把账推给谁」单独一条，与收据各说一件事：{log:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 验收④（先抵消后转嫁）在钱这一侧的形状：免掉之后第二问根本问不到，
/// 而那张嫁祸卡原样留在手里 —— 与上面事件那一侧的同名用例**同一句判据、两处形状**。
#[test]
fn a_waived_bill_never_reaches_the_shift_question() {
    let (mut app, [p0, _p1, p2]) = rent_table(&[(0, &["item_free", "item_shift"])]);
    step_onto_rent(&mut app, p0);

    assert_eq!(
        gate_asking(&app),
        Some((p0, Ask::Offset, PassiveEvent::Rent)),
        "两张卡都在手里时，第一问必须是抵消那一问"
    );

    answer_gate(&mut app, p0, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(gate_asking(&app), None, "免掉了就没有第二问");
    assert_eq!(cash_of(&mut app, p0), 20_000, "免掉：付款人不动");
    assert_eq!(cash_of(&mut app, p2), 20_000, "免掉：业主不收");
    assert_eq!(free_in_hand(&app, p0), 0, "免费那一张花掉了");
    assert_eq!(
        shift_left(&app, p0),
        1,
        "第二问没被问到，那张嫁祸就该原样留着"
    );
    let log = ledger(&app);
    assert!(
        !log.iter().any(|l| l.starts_with("Blame(")),
        "没有转嫁就不该有一条换人的流水：{log:?}"
    );
    assert!(
        log.contains(&"Rent(P0->P2@L0$0)".to_string()),
        "免单留一张 0 元的收据，而不是没有收据：{log:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 两问都答「不用」：什么都不消耗，账原样落下。
///
/// 这一条是上一条的反面，也是 PLAN §五 第 6 条那句「不可免者不可转」的反证形状 ——
/// 门问得出两句而两张卡都不花，说明「问到」与「花掉」是两件事；
/// 而它挡住的错误是「问一句就登记一次待用」。
#[test]
fn two_declines_cost_nothing_and_the_bill_lands_as_written() {
    let (mut app, [p0, _p1, p2]) = rent_table(&[(0, &["item_free", "item_shift"])]);
    step_onto_rent(&mut app, p0);

    let asked = ask_the_gate(&mut app, PassiveMove::Decline);
    assert_eq!(
        asked,
        vec![
            (p0, Ask::Offset, PassiveEvent::Rent),
            (p0, Ask::Shift, PassiveEvent::Rent)
        ],
        "两句都要问得出，而且顺序是先抵消后转嫁：{asked:?}"
    );
    assert_eq!(cash_of(&mut app, p0), 15_000, "原样落下：5000 照付");
    assert_eq!(cash_of(&mut app, p2), 25_000, "业主照收");
    assert_eq!(free_in_hand(&app, p0), 1, "两句都答不用，两张卡都该在");
    assert_eq!(shift_left(&app, p0), 1);
    assert_eq!(gate_spends(&app), 0, "门一张卡都没花");
    assert_cash_audit(&mut app, 20_000);
}
