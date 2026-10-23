//! 商业用地的落格装配（P10c）。
//!
//! 演示盘的外滩（L2，格 4/5 两格共享）是**预建的购物中心**，所以 A 型收费、
//! 空商业地、建设窗口全都能在这条真棋盘上验收；旅馆（B 型）与加油站（C 型）
//! 演示盘没有现成地块，用例把西单/王府井就地改造成对应设施 —— 与 `flat_rent`
//! 同一条手法：棋盘是资源里的一份快照，改它不必动 JSON。
//!
//! 断言的重心：
//! - **钱走哪条账**（`Service(...)` 流水 + `assert_cash_audit`），而不是「现金变了」
//!   —— 「该收没收」只有正面断言抓得住（P8 red-check ⑥ 的教训）；
//! - **人被送到哪**（B 型的 `Detention{place: Hotel}`），棋子停在旅馆那格；
//! - **判据被谁读**（建设窗口的四道门：不是业主、不是商业地、一格一设施、现钱不够），
//!   与灰按钮同源，拒得干净时现金分文不动。

mod common;

use bevy::prelude::App;
use common::*;
use game_core::board::{BuildingKind, FeeDef, LandType};
use game_core::prelude::*;

const START: i64 = 20_000;

/// 外滩购物中心的档位表（prototype.json）。断言「摇出几走第几档」要用它换算，
/// 写在这里而不是去翻棋盘 —— 表变了这条常量会当场红。
const MALL_TIERS: [i64; 6] = [200, 400, 600, 800, 1200, 1600];

/// 把某块地改造成指定的商业设施（演示盘没有旅馆/加油站，就地改）。
///
/// **def 与组件要一起改**：`Facility` 是装配时从 `LandDef.building_kind` 铺出来的
/// 组件（`spawn_lands`），只改 def 的话，读组件的那条查询永远看不到它 ——
/// 第一版测试就栽在这里：七条用例集体无声失败，探针显示落格分派在跑、
/// 设施分支没进。棋盘上写的是「这块地立着什么」，实体上的组件是它的运行时影子。
fn retune(app: &mut App, land: u32, kind: BuildingKind, fee: Option<FeeDef>) {
    let mut board = app.world_mut().resource_mut::<ActiveBoard>();
    let l = board
        .def
        .lands
        .iter_mut()
        .find(|l| l.id == land)
        .expect("prototype 里有这块地");
    l.land_type = LandType::Commercial;
    l.building_kind = Some(kind);
    l.fee = fee;
    l.group = None;
    let entity = board.lands[&LandId(land)];
    app.world_mut().entity_mut(entity).insert(Facility(kind));
}

/// 摘掉某块地上的预建设施（「空商业地」那条路）。
fn strip_facility(app: &mut App, land: u32) {
    let w = app.world_mut();
    let mut q = w.query::<(bevy::prelude::Entity, &LandId)>();
    let target = q
        .iter(w)
        .find(|(_, id)| id.0 == land)
        .map(|(e, _)| e)
        .expect("prototype 里有这块地");
    app.world_mut().entity_mut(target).remove::<Facility>();
}

fn service_lines(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|l| l.starts_with("Service("))
        .collect()
}

/// 摇出几走第几档。`roll` 出界（数据坏）直接 panic —— 用例里不该发生。
fn tier_of(roll: u32) -> i64 {
    MALL_TIERS[(roll - 1) as usize]
}

/// 走完当前回合（coupon.rs 同一条手法：跑够 turn_count+1 个回合数）。
fn land(app: &mut App) {
    play_turns(app, turn_count(app) + 1);
}

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

// ===== A 型：购物中心（预建，真棋盘） =====

/// 踩到别人的购物中心：摇出一个数、按档收钱，**不查租金表** ——
/// 外滩那条租金表（280..9000）对商业地是死数据，这条用例钉的就是这个分流。
/// 策略用 `Hold`：收据窗口要留着验 —— `DeclineAll` 的自动玩家会把它当场看完关掉。
#[test]
fn the_mall_bills_the_tier_the_service_roll_picked() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::Hold);
    let p1 = *roster(&mut app)
        .iter()
        .find(|(_, id, _, _)| *id == 1)
        .map(|(e, ..)| e)
        .unwrap();
    own(&mut app, 2, Some(1), 0, false);
    // 前置期的租金钱也进了流水，审计基线取世界此刻的真值
    let base = (cash_of(&mut app, p0), cash_of(&mut app, p1));

    play_turns(&mut app, 1);

    let lines = service_lines(&app);
    assert_eq!(lines.len(), 1, "恰好一笔服务费，实际: {lines:?}");
    // `Service(P0->P1@L2$X,摇r,住0)`
    let line = &lines[0];
    let roll: u32 = line
        .split("摇")
        .nth(1)
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse().ok())
        .expect("流水里带摇出的数");
    assert!((1..=6).contains(&roll));
    let amount: i64 = line
        .split('$')
        .nth(1)
        .and_then(|s| s.split(',').next())
        .and_then(|s| s.parse().ok())
        .expect("流水里带金额");
    assert_eq!(amount, tier_of(roll), "摇出 {roll} 就走第 {roll} 档");
    assert_eq!(cash_of(&mut app, p0), base.0 - amount, "付款方按档位少钱");
    assert_eq!(cash_of(&mut app, p1), base.1 + amount, "业主按同一笔进账");
    assert_cash_audit(&mut app, START);
    // 收据窗口挂着，等玩家看完再走
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::ServiceNotice(LandId(2))))
    );
    // 不是收租：租金那条流水不该出现
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Rent(")),
        "商业地不该走租金表"
    );
}

/// 免费卡（Cancel 修正）吃掉服务费：钱一分不动，**收据照样弹、金额是 0** ——
/// 「免单」与「没这回事」的区别就在这条流水上。
#[test]
fn a_waived_service_still_leaves_a_receipt() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::DeclineAll);
    let p1 = *roster(&mut app)
        .iter()
        .find(|(_, id, _, _)| *id == 1)
        .map(|(e, ..)| e)
        .unwrap();
    own(&mut app, 2, Some(1), 0, false);
    cancel_for(&mut app, p0);
    let base = (cash_of(&mut app, p0), cash_of(&mut app, p1));

    play_turns(&mut app, 1);

    let lines = service_lines(&app);
    assert_eq!(lines.len(), 1, "免单也要有一条收据，实际: {lines:?}");
    assert!(lines[0].contains("$0,"), "金额是 0：{}", lines[0]);
    assert_eq!(cash_of(&mut app, p0), base.0, "现金分文不动");
    assert_eq!(cash_of(&mut app, p1), base.1, "业主也没收到钱");
    // 服务费**过修正管线**：Cancel 挂上，档位钱（摇1 → 第 1 档 200）被吃成 0，
    // 两边现金都不动。always 型修正不注销、也不发一次性那种 Waive 凭据
    // （那条路归免费卡，free_pass.rs 钉过了）—— 这里要验的是「服务费免得掉」。
    assert!(corrections_len(&app) == 1, "always 型修正不消耗");
    assert_cash_audit(&mut app, START);
}

/// 服务摇数走**自己的子流**：同一种子重放，摇出的数逐位复现 ——
/// 「这一局进过几次店/收过几次费」不改变下一次摇数（P3d② 百货同一条纪律）。
#[test]
fn the_service_roll_replays_from_its_own_stream() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::DeclineAll);
    let _p1 = *roster(&mut app)
        .iter()
        .find(|(_, id, _, _)| *id == 1)
        .map(|(e, ..)| e)
        .unwrap();
    own(&mut app, 2, Some(1), 0, false);

    play_turns(&mut app, 1);
    let first = service_lines(&app);
    assert_eq!(first.len(), 1);
    let roll_1: u32 = first[0]
        .split("摇")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();

    // 同一个人再来一次（骰子流退回开局、轮转挪回 p0、摆回购物中心前一格）：
    // 子流重放应摇出同一个数
    rewind_dice(&mut app);
    make_current(&mut app, p0);
    common::place(&mut app, p0, 3);
    land(&mut app);
    let lines = service_lines(&app);
    assert_eq!(lines.len(), 2);
    let roll_2: u32 = lines[1]
        .split("摇")
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(roll_1, roll_2, "子流重放应摇出同一个数");
    assert_cash_audit(&mut app, START);
}

// ===== B 型：旅馆（就地改造西单） =====

/// 小财神上身那一下从别人身上抽走的总数（`GodDeed(P?:小财神,+N)`）。
/// 这一族 fixture 的种子下神明真的会附身（rent_rights.rs 的注记过同一件事），
/// 现金断言要么掐差值、要么把这笔账从期望里减掉 —— 它与住宿费无关，不该让它红。
fn smallrich_take(app: &App) -> i64 {
    ledger(app)
        .iter()
        .filter(|l| l.starts_with("GodDeed(") && l.contains("小财神"))
        .filter_map(|l| l.split('+').nth(1)?.split(')').next()?.parse::<i64>().ok())
        .sum()
}

/// 踩到别人的旅馆：按天付费 + 强制住宿 —— 「既扣钱又扣人」一条流水说清两件事，
/// 棋子停在旅馆那格（`Hotel` 没有死胡同分支），下回合开始时被跳过。
#[test]
fn the_hotel_bills_by_the_day_and_detains_the_guest() {
    let (mut app, p0) = ready_on(10, 1, DecisionPolicy::DeclineAll);
    let p1 = *roster(&mut app)
        .iter()
        .find(|(_, id, _, _)| *id == 1)
        .map(|(e, ..)| e)
        .unwrap();
    retune(
        &mut app,
        5,
        BuildingKind::Hotel,
        Some(FeeDef::RandomStay {
            sides: 4,
            per_day: 500,
        }),
    );
    own(&mut app, 5, Some(1), 0, false);
    let base = (cash_of(&mut app, p0), cash_of(&mut app, p1));

    play_turns(&mut app, 1);

    let lines = service_lines(&app);
    assert_eq!(lines.len(), 1, "实际: {lines:?}");
    let stay: u32 = lines[0]
        .split("住")
        .nth(1)
        .and_then(|s| s.strip_suffix(')'))
        .and_then(|s| s.parse().ok())
        .expect("流水里带住宿天数");
    assert!((1..=4).contains(&stay), "旅馆摇 1–4，实际住 {stay}");
    let amount: i64 = lines[0]
        .split('$')
        .nth(1)
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(amount, 500 * stay as i64, "住几天掏几天的钱");
    assert_eq!(
        cash_of(&mut app, p0),
        base.0 - amount - smallrich_take(&app),
        "付款方按天数少钱（小财神那一笔与住宿无关）：{:?}",
        ledger(&app)
    );

    // 人被送进旅馆：place 是 Hotel、剩的正是摇出的天数、棋子停在原格。
    // 读数必须赶在他**自己的下一个回合开始**之前 —— begin_turn 见 turns_left 就放人/扣格，
    // 而 play_turns 的收尾帧足够走进他的下一回合（P3d④c 踩过的同一口井）。
    let detention = detain_of(&app, p0).expect("住店的人该被扣下");
    assert_eq!(detention.place, DetentionPlace::Hotel);
    assert_eq!(detention.turns_left, stay);
    assert_eq!(tile_of(&app, p0), 11, "旅馆不在地图上，棋子停在那一格");
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l.starts_with("Detain(P0@旅馆,格11)")),
        "凭据也要说他在旅馆：{:?}",
        ledger(&app)
    );

    // 跳过口径与监狱同一条：**判 N = 跳 N-1**（P4① 的统一容器，旅馆只是第四个名字）。
    // 这一夹具的种子摇出 1 —— 住 1 天的读数就是「他的下一回合开始即放行」，没有跳过格。
    // 放行凭据写在 begin_turn 里（`Lifted`），补几帧让流水 reader 把它捞进来。
    play_turns(&mut app, 2);
    for _ in 0..3 {
        app.update();
    }
    eprintln!(
        "[probe-t2] phase={:?} turn={} cur={:?} events={:?}",
        phase(&app),
        turn_count(&app),
        app.world()
            .resource::<game_core::prelude::TurnOrder>()
            .cursor,
        events(&app)
    );
    assert!(
        events(&app).iter().any(|l| l.starts_with("Lifted(P0@旅馆")),
        "服完那 1 天要放人（跳过口径由 tests/detention.rs 钉，旅馆共用同一处实现）：{:?}",
        events(&app)
    );
    assert_cash_audit(&mut app, START);
    let _ = p1; // 业主的进账已由审计对平
}

/// 住不起的客人当场破产，而不是欠着房钱住进去 —— 钱先结、人后送，
/// `settle` 失败的那条路连 `Detention` 都不该挂。
#[test]
fn a_broke_guest_goes_bankrupt_instead_of_checking_in() {
    let (mut app, p0) = ready_on(10, 1, DecisionPolicy::DeclineAll);
    retune(
        &mut app,
        5,
        BuildingKind::Hotel,
        Some(FeeDef::RandomStay {
            sides: 4,
            per_day: 500,
        }),
    );
    own(&mut app, 5, Some(1), 0, false);
    set_cash(&mut app, p0, 300); // 住 1 天也要 500

    play_turns(&mut app, 1);

    assert!(
        ledger(&app).iter().any(|l| l.starts_with("Bankrupt(")),
        "付不起房钱该破产：{:?}",
        ledger(&app)
    );
    assert!(service_lines(&app).is_empty(), "没收成的账不留收据");
    assert!(detain_of(&app, p0).is_none(), "破产的人不再被扣在旅馆");
}

// ===== C 型：加油站（就地改造王府井） =====

/// 步行踩加油站：**根本不产生账** —— 没车就没油耗，连 0 元收据都不该有。
#[test]
fn a_gas_station_lets_walkers_pass_for_free() {
    let (mut app, p0) = ready_on(8, 1, DecisionPolicy::DeclineAll);
    retune(
        &mut app,
        4,
        BuildingKind::GasStation,
        Some(FeeDef::PerStep {
            rate: 100,
            requires_mount: true,
        }),
    );
    own(&mut app, 4, Some(1), 0, false);
    let base = cash_of(&mut app, p0);

    play_turns(&mut app, 1);

    assert!(service_lines(&app).is_empty(), "步行者没有油费这回事");
    assert_eq!(cash_of(&mut app, p0), base, "现金分文不动");
    // 连收据窗口都没有：这一格对他等于什么都没发生
    assert_eq!(pending_of(&app), None);
    assert_cash_audit(&mut app, START);
}

/// 开车踩加油站：单价 × **骰面原值**。骰面 5 就按 5 步收，哪怕乌龟只让他走 1 格 ——
/// 「按骰数收费」读的是 `DiceRolled.steps`（P3d④b 起它永不乘方向、不夹步）。
#[test]
fn a_driver_pays_the_dice_face_not_the_steps_actually_taken() {
    let (mut app, p0) = ready_on(8, 5, DecisionPolicy::DeclineAll);
    retune(
        &mut app,
        4,
        BuildingKind::GasStation,
        Some(FeeDef::PerStep {
            rate: 100,
            requires_mount: true,
        }),
    );
    own(&mut app, 4, Some(1), 0, false);
    set_mount(&mut app, p0, Mount::Car);
    // 乌龟只让走 1 格：实走 1 步、骰面 5 —— 收的必须是 500
    insert(&mut app, p0, MoveCap { turns_left: 3 });
    let base = cash_of(&mut app, p0);

    play_turns(&mut app, 1);

    let lines = service_lines(&app);
    assert_eq!(lines.len(), 1, "实际: {lines:?}");
    // 汽车是三颗骰：骰面不再是 ready_on 塞的那颗 5 —— 而这**正是**要验的规则：
    // 收的是「骰面 × 单价」，不是「实走的格数」。骰面从 `LastRoll` 读（它就是
    // `assess` 吃的那个数），实走几格由乌龟夹步决定，两件事必须分开。
    let face = app
        .world()
        .resource::<LastRoll>()
        .0
        .expect("掷过骰才有落格")
        .1;
    assert!(face >= 3, "三颗骰的最小面是 3，实际 {face}");
    assert_eq!(tile_of(&app, p0), 9, "乌龟把他按在只走一格的位置上");
    assert_ne!(face, 1, "骰面与实走格数必须是两个数，这条用例才有牙");
    assert!(
        lines[0].contains(&format!("${},", 100 * face as i64)),
        "100 × 骰面 {face}：{}",
        lines[0]
    );
    assert_eq!(cash_of(&mut app, p0), base - 100 * face as i64);
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l.starts_with(&format!("Cap(P0,{}→1", face))),
        "乌龟夹步该生效（Cap 流水在账本不在事件栏）：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, START);
}

// ===== 空商业地与建设窗口 =====

/// 踩到别人的**空**商业地：一文不收 —— 设施没立起来，不产生任何账，连流水都不该有。
#[test]
fn an_empty_commercial_lot_owes_nothing_to_a_passerby() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::DeclineAll);
    strip_facility(&mut app, 2);
    own(&mut app, 2, Some(1), 0, false);
    let base = cash_of(&mut app, p0);

    play_turns(&mut app, 1);

    assert!(service_lines(&app).is_empty());
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Rent(")),
        "空商业地也不该走租金表"
    );
    assert_eq!(cash_of(&mut app, p0), base);
    assert_eq!(pending_of(&app), None);
    assert_cash_audit(&mut app, START);
}

/// 自己的空商业地：开**建设**窗口、不开升级报价 —— 4 代商业用地不盖平房；
/// 按下去建成购物中心，同一格再按第二下被「一格一设施」拒掉、现金分文不动。
/// 策略 `Hold`：窗口要留着亲手按 —— `DeclineAll` 会替他「先不建」把窗口关掉。
#[test]
fn your_own_empty_lot_opens_the_build_window_and_refuses_a_second_build() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::Hold);
    strip_facility(&mut app, 2);
    own(&mut app, 2, Some(0), 0, false);

    play_turns(&mut app, 1);

    assert_eq!(pending_of(&app), Some((p0, Pending::Build(LandId(2)))));
    let offers: Vec<_> = ledger(&app)
        .into_iter()
        .filter(|l| l.starts_with("BuildOffer("))
        .collect();
    assert_eq!(offers.len(), 1, "实际: {offers:?}");
    assert!(
        offers[0].contains("$2200"),
        "造价取 upgrade_cost：{}",
        offers[0]
    );
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("UpgradeOffer(")),
        "商业地不开升级报价"
    );

    // 建一座购物中心（类别以请求给的为准 —— 棋盘默认表里有它的价）
    build_ask(&mut app, p0, LandId(2), BuildingKind::Mall);
    assert_eq!(facility_of(&mut app, 2), Some(BuildingKind::Mall));
    assert_eq!(cash_of(&mut app, p0), START - 2200);
    assert!(
        ledger(&app)
            .iter()
            .any(|l| l.starts_with("Built(P0@2:Mall$2200)")),
        "建成凭据：{:?}",
        ledger(&app)
    );

    // 窗口还开着（办完照开，交还回合是「先不建」的回应），但第二下被拒
    build_ask(&mut app, p0, LandId(2), BuildingKind::Mall);
    assert_eq!(
        facility_of(&mut app, 2),
        Some(BuildingKind::Mall),
        "一格一设施"
    );
    assert_eq!(
        cash_of(&mut app, p0),
        START - 2200,
        "拒得干净：现金分文不动"
    );
    assert_eq!(
        ledger(&app)
            .iter()
            .filter(|l| l.starts_with("Built("))
            .count(),
        1,
        "建成凭据只有一条"
    );
    assert_cash_audit(&mut app, START);
}

/// 建成之后踩回自己家：无事发生 —— 设施不可升级、没有「逛自己的店」。
#[test]
fn your_own_occupied_lot_says_nothing_at_all() {
    let (mut app, _p0) = ready_on(3, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 2, Some(0), 0, false); // 预建购物中心就在身上

    play_turns(&mut app, 1);

    assert_eq!(pending_of(&app), None);
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("BuildOffer(")),
        "有设施的地不开建设窗口"
    );
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("UpgradeOffer(")),
        "设施也不接受升级报价"
    );
}

/// 押着的空商业地不开建设窗口 —— 必败不给弹，比一张按不动的模态更糟
/// （与无主地那条衰神特例同一句理由）。
#[test]
fn a_mortgaged_lot_wont_open_the_build_window() {
    let (mut app, _p0) = ready_on(3, 1, DecisionPolicy::DeclineAll);
    strip_facility(&mut app, 2);
    own(&mut app, 2, Some(0), 0, true);

    play_turns(&mut app, 1);

    assert_eq!(pending_of(&app), None);
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("BuildOffer(")),
        "押着的地不该弹建设报价"
    );
}

/// 报价不筛现金、柜台上拒 —— 窗口照样弹（按钮灰着、理由写在按钮上），
/// 按下去办不成、现金分文不动；补够钱之后同一扇窗口办得成。
#[test]
fn the_build_quote_opens_even_when_cash_is_short_but_the_counter_refuses() {
    let (mut app, p0) = ready_on(3, 1, DecisionPolicy::Hold);
    strip_facility(&mut app, 2);
    own(&mut app, 2, Some(0), 0, false);
    set_cash(&mut app, p0, 500);

    play_turns(&mut app, 1);

    // 报价照弹
    assert_eq!(pending_of(&app), Some((p0, Pending::Build(LandId(2)))));
    // 按下去：拒（ShortOfCash），什么都没发生
    build_ask(&mut app, p0, LandId(2), BuildingKind::Mall);
    assert_eq!(facility_of(&mut app, 2), None);
    assert_eq!(cash_of(&mut app, p0), 500);
    assert!(
        ledger(&app)
            .iter()
            .filter(|l| l.starts_with("Built("))
            .count()
            == 0
    );

    // 补够钱，同一扇窗口办得成
    set_cash(&mut app, p0, 2200);
    build_ask(&mut app, p0, LandId(2), BuildingKind::Mall);
    assert_eq!(facility_of(&mut app, 2), Some(BuildingKind::Mall));
    assert_eq!(cash_of(&mut app, p0), 0);
    assert_cash_audit(&mut app, START);
}

/// 住宅地不开建设窗口：自己的南京路照旧弹**升级**报价 —— 商业/住宅的分流
/// 在「踩到自己地」这一支上也有牙。升级要**垄断同色组**，所以两条 brown 一起给他。
#[test]
fn a_residential_lot_still_offers_an_upgrade_not_a_build() {
    let (mut app, _p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    own(&mut app, 0, Some(0), 1, false);
    own(&mut app, 1, Some(0), 1, false); // 汉口路：凑满 brown 组

    play_turns(&mut app, 1);

    assert!(
        ledger(&app).iter().any(|l| l.starts_with("UpgradeOffer(")),
        "住宅照旧走升级：{:?}",
        ledger(&app)
    );
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("BuildOffer(")),
        "住宅地不开建设窗口"
    );
}

/// 住旅馆的人连监狱门口的保释窗口都撑不开 —— 开窗判据是「**这一处**真关着人」
/// （`someone_held`，bail.rs：没人可保就什么都不做），`Hotel` 不是 `Jail`，
/// 四个去处各自只认自己的门。两人盘里没有第三个可关的人，「开了窗再拒捞人」
/// 那条路要等多人局（P14）；这条钉的是它前面那一格：窗口压根不来。
#[test]
fn a_hotel_guest_cannot_be_bailed_out_of_the_jail_gate() {
    let (mut app, p0) = ready_on(10, 1, DecisionPolicy::DeclineAll);
    let p1 = *roster(&mut app)
        .iter()
        .find(|(_, id, _, _)| *id == 1)
        .map(|(e, ..)| e)
        .unwrap();
    retune(
        &mut app,
        5,
        BuildingKind::Hotel,
        Some(FeeDef::RandomStay {
            sides: 4,
            per_day: 500,
        }),
    );
    own(&mut app, 5, Some(1), 0, false);

    play_turns(&mut app, 1);
    assert_eq!(
        detain_of(&app, p0).map(|d| d.place),
        Some(DetentionPlace::Hotel),
        "他住着店（读数赶在他下一回合开始前 —— 判 1 = 跳 0，收尾帧就会放人）"
    );

    // P1 走到监狱接入点（格 2，汉口路）：先照常弹购买报价，亲手答「不买」。
    // 换成 Hold —— 自动玩家对 Bail 一律「离开」，会抢在我们看清之前把窗口关掉。
    rewind_dice(&mut app);
    make_current(&mut app, p1);
    common::place(&mut app, p1, 1);
    app.insert_resource(DecisionPolicy::Hold);
    roll(&mut app, p1);
    let _ = run_until_phase(&mut app, TurnPhase::WaitDecision, 60);
    assert!(
        matches!(pending_of(&app), Some((_, Pending::Buy(_)))),
        "前置：格 2 先弹的是购买报价"
    );
    // 亲手答「不买」（bail.rs 那条 answer 的内联版：写真消息，走同一扇门）。
    // 不能用 run_until_phase 等下一个 WaitDecision —— 此刻相位已经是它，一帧都不会跑，
    // 消息就烂在队列里；settle 三帧既消化消息又把后续窗口走稳。
    app.world_mut()
        .resource_mut::<bevy::ecs::message::Messages<DecisionChosen>>()
        .write(DecisionChosen {
            player: p1,
            action: Decision::Decline,
        });
    settle(&mut app, 3);

    // 报价答完了，保释窗口没来：监狱里没有人 —— 那位客人在旅馆
    assert_eq!(pending_of(&app), None, "旅馆里的人撑不开监狱门口的保释窗口");
}
