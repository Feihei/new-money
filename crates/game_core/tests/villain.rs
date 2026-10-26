//! 四大恶人的行为（P12）。载体与轮转的地基在 `villain.rs` 的单元测试里；
//! 这里测的是**四个人的行为表** —— PLAN §四 P12 的验收原话是
//! 「每个恶人一条行为测试；雇主保释强盗后**自己的存款也被抢**（这是特性不是 bug，必须测住）」。
//!
//! 与其它阶段的测试同一条纪律：不走捷径。保释走真窗口（挂 `Pending::Bail` + 按那颗按钮），
//! 走路走真骰子（`RequestRoll` 消息），勒索与转付走真落格 —— 判据被绕过的测试
//! 保护不了判据。恶人被保释后站在监狱分支的接入点（tile 2，汉口路），
//! 演示盘的格子布局见 `assets/data/boards/prototype.json`。

use bevy::prelude::*;
use game_core::prelude::*;

mod common;
use common::*;
// prelude 里另有一个 `occupancy::place`（玩家放道具的那条）与本文件的夹具 `place`
// （搬位置）同名 —— 显式指认夹具那一个，其余保持 glob。
use common::place;

/// 四人阵容。行为表里要同时有「雇主、两位受害者、一位旁观者」，
/// 默认的两人局摆不出那个形状。
fn four_players() -> GameSetup {
    GameSetup {
        players: (0..4)
            .map(|i| PlayerSetup {
                name: format!("P{i}"),
                color: PLAYER_COLORS[i % PLAYER_COLORS.len()],
                gender: game_core::player::Gender::Male,
            })
            .collect(),
    }
}

fn boot4() -> App {
    boot(four_players(), GameRules::instant(), GameSeed::default())
}

/// 找到那一位恶人（开局四位都在笼里，实体由装配创建）。
fn villain_of(app: &mut App, kind: VillainKind) -> Entity {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &Villain)>();
    q.iter(w)
        .find(|(_, v)| v.kind == kind)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("四位恶人开局都在笼里（缺 {:?}）", kind))
}

/// 把一位恶人保释给 `employer`：挂真窗口（`Pending::Bail`）→ 按真按钮。
/// 出场行为（抢存款/偷点券）随 `VillainHired` 下一帧分派，所以这里留足帧数。
fn bail_villain(app: &mut App, employer: Entity, target: Entity) {
    app.world_mut()
        .resource_mut::<PendingDecision>()
        .offer(employer, Pending::Bail(DetentionPlace::Jail));
    settle(app, 1);
    bail_ask(app, employer, target);
    settle(app, 3);
}

/// 替 `who` **精确**走 `steps` 格：骰子流退回一颗「第一掷 = steps」的种子再掷
/// （`RequestRoll.dice = 0` 满载、恶人恒步行一颗）。
///
/// 为什么不用 `roll_with_dice(.., n)`：那第二个参数是**骰子颗数**，会被
/// `dice_count` 夹到载具上限 —— 恶人恒步行，永远只摇一颗。恶人也没有遥控骰子
/// （他压根没有道具栏），精确步数只能从种子里来。这与 [`rewind_dice`] 同一条手法：
/// 重置的是整条随机流，别的流（掉落、神明）随之从头来 —— 对这些用例无所谓。
fn walk_exact(app: &mut App, who: Entity, steps: u32) {
    let seed = nth_seed_rolling_first(steps, 0);
    app.insert_resource(Rng::new(seed.0));
    make_current(app, who);
    roll_with_dice(app, who, 0);
    // 走路、落格、窗口自动答、回合收尾 —— 消息各隔一帧，留足余量
    settle(app, 10);
}

/// 编号 → 实体。与夹具的 `roster` 用同一个顺序拿位。
fn player_no(app: &mut App, no: u32) -> Entity {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &PlayerId)>();
    q.iter(w)
        .find(|(_, id)| id.0 == no)
        .map(|(e, _)| e)
        .unwrap_or_else(|| panic!("场上没有 P{no}"))
}

/// 小偷保释、路上清干净、挪到 0 号格（tile 0）—— 「路上有什么」由各用例自己摆。
fn bailed_thief_at_start(app: &mut App, employer: Entity) -> Entity {
    let thief = villain_of(app, VillainKind::Thief);
    bail_villain(app, employer, thief);
    clear_occupancies(app);
    place(app, thief, 0);
    thief
}

// ===== 强盗（Robber）=====

/// PLAN §四 P12 的验收原话，一字不改：**雇主保释强盗后自己的存款也被抢**
/// （这是特性不是 bug，必须测住）。全场每人存款 10000 → 20% 一刀 → 8000。
#[test]
fn the_robber_sweeps_every_deposit_including_his_own_employers() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    let p2 = player_no(&mut app, 2);
    let p3 = player_no(&mut app, 3);
    for p in [p0, p1, p2, p3] {
        app.world_mut()
            .entity_mut(p)
            .get_mut::<BankBalance>()
            .expect("玩家应有账户")
            .deposit = 10_000;
    }
    set_points(&mut app, p0, 500);
    let robber = villain_of(&mut app, VillainKind::Robber);
    bail_villain(&mut app, p0, robber);

    // 含雇主自己 —— 四个人一人一刀
    for p in [p0, p1, p2, p3] {
        assert_eq!(
            bank_of(&mut app, p).0,
            8_000,
            "20% 的一刀不认雇主（那是拍板的特性，不是 bug）"
        );
    }
    let log = ledger(&app);
    assert!(
        log.iter()
            .any(|line| line.contains("Robbed(强盗抢$8000·4人)")),
        "抢走的总额与人数要能从流水对出来：{log:?}"
    );
    // 抢走的那笔进了强盗的口袋（世界外）：场上没有任何人变富，雇主也没得提成
    assert_eq!(cash_of(&mut app, p0), 20_000);
}

/// 擦肩抢卡：强盗走过**玩家所在的那一格**，那一格上的每一位玩家各被抢一张；
/// 雇主站在同一格也不被抢 —— 刀口不对着自己人。
#[test]
fn a_passing_robber_snatches_from_everyone_on_that_tile_but_not_his_employer() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    let p2 = player_no(&mut app, 2);

    let card = item_index(&app, "item_frame");
    deal(&mut app, p0, card.0);
    deal(&mut app, p1, card.0);
    deal(&mut app, p2, card.0);

    set_points(&mut app, p0, 500);
    let robber = villain_of(&mut app, VillainKind::Robber);
    bail_villain(&mut app, p0, robber);
    // 三个人的位置：两位受害者与雇主都站在 tile 3，强盗从接入点（tile 2）走出一步恰好擦过
    place(&mut app, p0, 3);
    place(&mut app, p1, 3);
    place(&mut app, p2, 3);
    walk_exact(&mut app, robber, 1);

    assert!(hand_of(&app, p1).is_empty(), "受害者 1 被抢走那张卡");
    assert!(hand_of(&app, p2).is_empty(), "受害者 2 被抢走那张卡");
    assert_eq!(
        hand_of(&app, p0).len(),
        3,
        "雇主站在同一格也不被抢（他那张还在），同时收下两件赃物：1 + 2 = 3"
    );
    assert_eq!(
        hand_of(&app, robber).len(),
        0,
        "恶人自己没有手牌 —— 抢来的卡给了雇主，不是揣进自己怀里"
    );
    let log = ledger(&app);
    assert_eq!(
        log.iter().filter(|line| line.contains("Snatch(")).count(),
        2,
        "擦过一圈是两条凭据：{log:?}"
    );
}

// ===== 小偷（Thief）=====

/// 出场偷走**其他玩家**一半点券给雇主；雇主自己的点券原封不动，
/// 0 点券的人不算受害 —— 「一半」对空口袋无从下手。
#[test]
fn the_thief_steals_half_of_every_opponents_points_but_not_the_employers() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    let p2 = player_no(&mut app, 2);

    set_points(&mut app, p0, 1_000);
    set_points(&mut app, p1, 501);
    // p2、p3 留在 0：空口袋不算受害
    let thief = villain_of(&mut app, VillainKind::Thief);
    bail_villain(&mut app, p0, thief);

    assert_eq!(
        points_of(&app, p1),
        251,
        "501 的一半向下取整是 250，被偷后剩 251"
    );
    assert_eq!(
        points_of(&app, p0),
        1_000 - 300 + 250,
        "雇主 1000 不在被偷之列：他付出 300 保释费、收下 250 赃物"
    );
    assert_eq!(points_of(&app, p2), 0);
    let log = ledger(&app);
    assert!(
        log.iter()
            .any(|line| line.contains("Stolen(小偷偷250点券·1人)")),
        "流水要说得出一共偷了几格、几个人：{log:?}"
    );
}

/// 「唯一能穿路障」落在两处：掷骰入队不因路障截断（走满全程），
/// 路障还在他被偷进雇主道具栏。对照组：同一条路换玩家走，人被截在路障格上、路障原地不动。
#[test]
fn only_the_thief_walks_through_a_roadblock_and_takes_it_home() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    set_points(&mut app, p0, 500);

    // 对照组：玩家从同一点走同一条路，被截断在路障格上，路障原地不动
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Roadblock, 2);
    place(&mut app, p1, 0);
    walk_exact(&mut app, p1, 4);
    assert_eq!(tile_of(&app, p1), 2, "路障把玩家截在它那一格");
    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::Roadblock),
        vec![2],
        "玩家走不过去，路障也还在原地"
    );

    // 小偷走同一条路：截断豁免（plan_walk 那个布尔）、路障进雇主道具栏
    let thief = villain_of(&mut app, VillainKind::Thief);
    bail_villain(&mut app, p0, thief);
    // 对照组那条路障没被消耗 —— 清掉重放一件，别让「一格一件」的前提被两件叠在一起
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Roadblock, 2);
    place(&mut app, thief, 0);
    walk_exact(&mut app, thief, 4);
    assert_eq!(tile_of(&app, thief), 4, "小偷走满了四步，路障没拦住他");
    assert!(
        tiles_holding(&mut app, OccupancyKind::Roadblock).is_empty(),
        "他顺手把路障搬走了"
    );
    // 开局配送本就有一个路障（`Inventory::starting`），偷来的这一件叠上去是 2
    assert_eq!(
        inventory_of(&app, p0),
        vec![(ItemKind::Roadblock, 2)],
        "赃物进了雇主道具栏（小偷自己没有 Inventory）"
    );
    let log = ledger(&app);
    assert!(
        log.iter()
            .any(|line| line.contains("Thieve(小偷:路障@格2)")),
        "偷走的每一件都要有凭据：{log:?}"
    );
}

/// 小偷踩到地雷不发火：他先下手把雷拆走，脚步不停 —— 而不是像玩家那样被送进医院
/// （玩家那一侧由 `tests/occupancy.rs` 钉着，这里不重复对照）。
#[test]
fn a_thief_disarms_a_mine_instead_of_being_sent_to_hospital() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    set_points(&mut app, p0, 500);
    let thief = bailed_thief_at_start(&mut app, p0);
    spawn_occupancy(&mut app, OccupancyKind::Mine, 2);

    walk_exact(&mut app, thief, 4);
    assert_eq!(tile_of(&app, thief), 4, "偷雷那一步照常走完");
    assert!(
        tiles_holding(&mut app, OccupancyKind::Mine).is_empty(),
        "雷被拆走了"
    );
    assert_eq!(
        detain_of(&app, thief),
        None,
        "没有进医院 —— 没有可中断的东西了"
    );
    // 开局配送的路障 + 偷来的雷
    assert_eq!(
        inventory_of(&app, p0),
        vec![(ItemKind::Roadblock, 1), (ItemKind::Mine, 1)],
        "拆下来的雷归雇主"
    );
}

/// 小偷落脚宝箱 = 连箱搬走：箱子里的东西进雇主道具栏，而不是走「开箱」那条流水。
/// 其余三种恶人踩到箱子什么都不发生 —— 偷窃是小偷一个人的职业行为。
#[test]
fn a_chest_under_the_thiefs_foot_goes_to_the_employer_and_others_leave_it_alone() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    set_points(&mut app, p0, 500);
    let thief = bailed_thief_at_start(&mut app, p0);
    // 宝箱在 tile 3：小偷从 0 走三步落脚，触发的是 Land 那张表
    spawn_occupancy(&mut app, OccupancyKind::Chest, 3);

    walk_exact(&mut app, thief, 3);
    assert!(
        tiles_holding(&mut app, OccupancyKind::Chest).is_empty(),
        "箱子被连锅端了"
    );
    assert_eq!(
        inventory_of(&app, p0).len(),
        2,
        "开局配送的一件 + 箱子里那件 —— 雇主收下了赃物"
    );
    let log = ledger(&app);
    assert!(
        log.iter().any(|line| line.contains("Thieve(小偷:")),
        "小偷的赃物走 Thieve 凭据：{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("Chest(")),
        "没有 Chest 流水 —— 开箱是玩家踩到宝箱那一条路，小偷走的是偷窃"
    );

    // 对照：流氓落脚同一格，箱子原封不动
    let mut app2 = boot4();
    let boss = boot_to_wait_roll(&mut app2);
    set_points(&mut app2, boss, 500);
    let hoodlum = villain_of(&mut app2, VillainKind::Hoodlum);
    bail_villain(&mut app2, boss, hoodlum);
    clear_occupancies(&mut app2);
    spawn_occupancy(&mut app2, OccupancyKind::Chest, 3);
    place(&mut app2, hoodlum, 0);
    walk_exact(&mut app2, hoodlum, 3);
    assert_eq!(
        tiles_holding(&mut app2, OccupancyKind::Chest),
        vec![3],
        "流氓不是贼：箱子留给它的主人"
    );
    // 雇主的道具栏只有开局配送的那一件 —— 流氓没替他偷到任何东西
    assert_eq!(inventory_of(&app2, boss), vec![(ItemKind::Roadblock, 1)]);
}

// ===== 流氓（Hoodlum）=====

/// 路过别人的地勒索地价等额（含物价、不含楼），钱归**流氓自己** —— 雇主一分拿不到，
/// 这是 PLAN §三 拍板的反制窗口。
#[test]
fn the_hoodlum_extorts_the_land_price_from_the_owner_and_keeps_it() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    own(&mut app, 0, Some(1), 0, false); // 南京路归 P1，地价 1500
    set_points(&mut app, p0, 500);
    let hoodlum = villain_of(&mut app, VillainKind::Hoodlum);
    bail_villain(&mut app, p0, hoodlum);
    // 流氓从 0 号格走出一步，踩进南京路
    place(&mut app, hoodlum, 0);
    walk_exact(&mut app, hoodlum, 1);

    assert_eq!(
        cash_of(&mut app, p1),
        20_000 - 1_500,
        "地主被勒走地价等额（物价 1.0 时就是棋盘价）"
    );
    assert_eq!(
        cash_of(&mut app, p0),
        20_000,
        "雇主一分不得 —— 勒索额归流氓自己，而流氓没有钱包"
    );
    let log = ledger(&app);
    assert!(
        log.iter()
            .any(|line| line.contains("Toll(流氓→P1@L0$1500)")),
        "勒索那一笔要有名有姓：{log:?}"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 被勒到付不起 = 破产，与租金同一条出口。流氓自己毫发无损。
#[test]
fn extortion_that_cannot_be_paid_bankrupts_the_owner() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    own(&mut app, 0, Some(1), 0, false);
    set_cash(&mut app, p1, 100);
    set_points(&mut app, p0, 500);
    let hoodlum = villain_of(&mut app, VillainKind::Hoodlum);
    bail_villain(&mut app, p0, hoodlum);
    place(&mut app, hoodlum, 0);
    walk_exact(&mut app, hoodlum, 1);

    let log = ledger(&app);
    assert!(
        log.iter().any(|line| line.contains("Bankrupt(P1")),
        "付不出勒索额走破产那条路：{log:?}"
    );
    assert!(detain_of(&app, hoodlum).is_none(), "勒索者不进笼");
}

/// 三条豁免：无主地没有可勒索的人（恶人也不弹买地窗）；查封中的地被法院冻着。
#[test]
fn the_hoodlum_leaves_unowned_and_sealed_land_alone() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    set_points(&mut app, p0, 500);
    let hoodlum = villain_of(&mut app, VillainKind::Hoodlum);
    bail_villain(&mut app, p0, hoodlum);

    // 无主：踩上去连一条流水都没有（也不弹买地窗 —— 那是玩家域的事）
    place(&mut app, hoodlum, 0);
    walk_exact(&mut app, hoodlum, 1);
    let log = ledger(&app);
    assert!(
        !log.iter().any(|line| line.contains("Toll(")),
        "无主地勒不到人：{log:?}"
    );
    assert!(
        !log.iter().any(|line| line.contains("BuyOffer(")),
        "恶人不买地：落格分派把他带进 Villain 域，买地窗口结构上就不会跑"
    );

    // 查封：地有主，但法院贴着条子 —— 与租金归零同一条理由
    own(&mut app, 0, Some(1), 0, false);
    let land_entity = app
        .world()
        .resource::<ActiveBoard>()
        .lands
        .get(&LandId(0))
        .copied()
        .expect("南京路在场");
    app.world_mut().entity_mut(land_entity).insert(Timed {
        kind: LandStatus::Sealed,
        days_left: 3,
    });
    place(&mut app, hoodlum, 0);
    walk_exact(&mut app, hoodlum, 1);
    let log = ledger(&app);
    assert!(
        !log.iter().any(|line| line.contains("Toll(")),
        "查封中的地勒不动：{log:?}"
    );
}

// ===== 间谍（Spy）=====

/// 「该地从未有人踩过交费则无功而返」的正反两面：有案底的街租进雇主口袋、
/// 新街照付地主；雇主自己踩到别人的地是正常交易，间谍不下手。
///
/// 四段各自独立构造（同一局里先后走完）：先替南京路攒一条「有人交过租」的案底，
/// 再请出间谍，然后一段一段走。每一段之前把物价摆回 1.0、把路上的神清干净 ——
/// 前者是因为指数在回合末重算、而这里的断言写的是棋盘价；后者是因为开局会刷两位神，
/// 落脚踩到会被附身送卡，把「租金该是谁的」这条断言搅浑。
#[test]
fn the_spy_reroutes_rent_only_for_roads_that_have_been_paid_before() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = player_no(&mut app, 1);
    let p2 = player_no(&mut app, 2);
    let p3 = player_no(&mut app, 3);
    own(&mut app, 0, Some(1), 0, false); // 南京路 L0 归 P1（rent[0] = 60）
    own(&mut app, 4, Some(2), 0, false); // 王府井 L4 归 P2（rent[0] = 160），全程没人交过费
    clear_gods(&mut app);

    // ① P3 先踩一次南京路（那时还没有间谍）：租金 60 照付地主，同时给 L0 留下案底
    set_price_index(&mut app, 1.0);
    place(&mut app, p3, 0);
    walk_exact(&mut app, p3, 1);
    let log = ledger(&app);
    assert!(
        log.iter().any(|line| line.contains("Rent(P3->P1@L0$60)")),
        "没有间谍时世界照旧：{log:?}"
    );
    assert!(!log.iter().any(|line| line.contains("Reroute(")));

    // ② 请出间谍（雇主 P0），再让 P2 踩南京路：这 60 块进 P0 的口袋
    set_points(&mut app, p0, 500);
    let spy = villain_of(&mut app, VillainKind::Spy);
    bail_villain(&mut app, p0, spy);
    set_price_index(&mut app, 1.0);
    let before = cash_of(&mut app, p0);
    place(&mut app, p2, 0);
    walk_exact(&mut app, p2, 1);
    let log = ledger(&app);
    assert!(
        log.iter().any(|line| line.contains("Rent(P2->P0@L0$60)")),
        "收据上写的就是实际收款人：{log:?}"
    );
    assert!(
        log.iter()
            .any(|line| line.contains("Reroute(间谍→P0@L0$60)")),
        "为什么 P1 没收到租，这条流水就是答案：{log:?}"
    );
    assert_eq!(cash_of(&mut app, p0), before + 60);
    assert_eq!(
        cash_of(&mut app, p1),
        20_060,
        "P1 只收到了 ① 那一笔：20_000 + 60，间谍截走的那笔不经过他"
    );

    // ③ 王府井从没人交过费：间谍对它无功而返，租金照付地主
    set_price_index(&mut app, 1.0);
    // 站到「掷 1 点正好踩到王府井（land 4）」那一格 —— 格号现读，别写死 10
    stand_to_land(&mut app, p3, 4, 1);
    walk_exact(&mut app, p3, 1);
    let log = ledger(&app);
    assert!(
        log.iter().any(|line| line.contains("Rent(P3->P2@L4$160)")),
        "没案底的街他探不到，世界与没有间谍时一模一样：{log:?}"
    );
    assert_eq!(
        log.iter()
            .filter(|line| line.contains("Reroute(") && line.contains("L4"))
            .count(),
        0
    );

    // ④ 雇主自己踩到别人的地：交租是正常交易，不截自己的钱
    set_price_index(&mut app, 1.0);
    place(&mut app, p0, 0);
    walk_exact(&mut app, p0, 1);
    let log = ledger(&app);
    assert!(
        log.iter().any(|line| line.contains("Rent(P0->P1@L0$60)")),
        "雇主的租照付 P1：{log:?}"
    );
    assert_eq!(
        log.iter().filter(|line| line.contains("Reroute(")).count(),
        1,
        "全场只有 ② 那一笔被转付"
    );
    assert_cash_audit(&mut app, 20_000);
}

// ===== 载体边界 =====

/// 神不上恶人的身（P12a 定的边界在这里的行为侧再钉一遍）：
/// 恶人落脚在神那一格，神还站在路上、谁也没被附身。
#[test]
fn a_god_on_the_road_is_not_attached_to_a_villain() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    set_points(&mut app, p0, 500);
    let hoodlum = villain_of(&mut app, VillainKind::Hoodlum);
    bail_villain(&mut app, p0, hoodlum);
    clear_occupancies(&mut app);
    spawn_god(&mut app, God::BigRich, 3);
    place(&mut app, hoodlum, 0);
    walk_exact(&mut app, hoodlum, 3);

    assert_eq!(
        tiles_holding(&mut app, OccupancyKind::God(God::BigRich)).len(),
        1,
        "神还站在场上（回合末他自己会挪一格，但绝没有被吃掉、也没有换人）"
    );
    assert!(possession_of(&app, hoodlum).is_none(), "没人被附身");
}

/// 恶人踩雷回笼（P12a 的 `custody_of` 在行为侧）：挨了雷他回医院那一处等保释
/// —— 没有钟、只有保释办得出来；回笼的那一刻也退出轮转。
/// 这一条就是 PLAN §四 P12 那句「踩地雷与恶犬回医院需再保释」。
#[test]
fn a_villain_hit_by_a_mine_goes_back_to_the_cage_until_bailed() {
    let mut app = boot4();
    let p0 = boot_to_wait_roll(&mut app);
    set_points(&mut app, p0, 500);
    let robber = villain_of(&mut app, VillainKind::Robber);
    bail_villain(&mut app, p0, robber);
    clear_occupancies(&mut app);
    spawn_occupancy(&mut app, OccupancyKind::Mine, 3);
    place(&mut app, robber, 0);
    walk_exact(&mut app, robber, 3);

    let cage = detain_of(&app, robber).expect("他回笼了");
    assert_eq!(
        cage.place,
        DetentionPlace::Hospital,
        "挨的是雷，去的是医院那一处"
    );
    assert_eq!(cage.term, Term::UntilBailed, "恶人没有钟：只有保释办得出来");
    // 回笼的人在轮转外：掷骰那一侧再没有人会替他按按钮
    let order = app.world().resource::<TurnOrder>();
    assert!(
        !order.order.contains(&robber),
        "「在笼里」与「不在轮转里」必须是同一件事的两面"
    );
}
