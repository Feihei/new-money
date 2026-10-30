//! P15 的验收：**快照的形状**（P15a）与**读回来接着跑**（P15b）。
//!
//! 每一刀各管一半：
//! - [`a_capture_at_the_save_point_reads_the_same_world_twice`]：同一帧世界里拍两次逐字段相等 ——
//!   它是「捕获没有偷读时间/帧序」的判据（`AiClock` 与 `AiErrands` 不在里面，正是因为它必须是）；
//! - [`the_capture_matches_what_the_world_says`]：快照里那几个数**就是世界的数**（拿 `roster`
//!   那条真读数对，不拿快照自己的字段自证）；
//! - [`the_snapshot_round_trips_through_json_byte_for_byte`]：存→读→再存**字节一致**，
//!   这一条是「排序与 `HashMap` 别混进存档」的全部理由；
//! - [`every_busy_shape_refuses_the_capture_with_its_own_reason`]：正在办事时拒得干净，
//!   且理由就是灰按钮上那句话；
//! - [`every_state_block_reaches_the_snapshot`]：每一块状态都真的被读到（漏读一个字段时
//!   前面几条全绿）；
//! - [`a_loaded_match_continues_exactly_where_it_stopped`]：读档继续 == 没读档继续
//!   （两条时间线走到同一回合，快照逐字节相同）；
//! - [`the_save_point_reading_follows_the_world_every_frame`]：`judge_system` 的接线
//!   （灰按钮那句为什么是每帧算出来的，不是默认值）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::*;
use game_core::prelude::{GameSession, PendingDecision, Points, TurnPhase, WalkQueue};
use game_core::snapshot::{
    GameSnapshot, SAVE_VERSION, SaveDenied, SaveDeny, SaveRequest, SnapshotTaken,
};

/// 扮演一次「按下保存」：置愿望、跑一帧，捕获 system 在 `Last` 拍。
fn request_save(app: &mut App) {
    app.world_mut().resource_mut::<SaveRequest>().pending = true;
    app.update();
}

fn taken(app: &App) -> Option<GameSnapshot> {
    app.world().resource::<SnapshotTaken>().0.clone()
}

fn denied(app: &App) -> Option<SaveDeny> {
    app.world().resource::<SaveDenied>().0
}

#[test]
fn a_capture_at_the_save_point_reads_the_same_world_twice() {
    let mut app = boot_default();
    boot_to_wait_roll(&mut app);
    request_save(&mut app);
    let first = taken(&app).expect("可存点（轮到人类、掷骰之前）应存得下");
    assert_eq!(denied(&app), None);
    request_save(&mut app);
    let second = taken(&app).expect("第二次也要存得下");
    assert_eq!(first, second, "同一个世界拍两次，快照逐字段必须相等");
    assert_eq!(first.version, SAVE_VERSION);
    assert_eq!(first.players.len(), 2, "两人夹具");
    assert_eq!(first.decks.len(), 2, "机会与命运两副");
}

#[test]
fn the_capture_matches_what_the_world_says() {
    let mut app = boot_default();
    boot_to_wait_roll(&mut app);
    // 先让世界有点东西：跑两个回合，再回到「轮到人、还没掷骰」
    play_turns(&mut app, 2);
    assert!(
        run_until_phase(&mut app, TurnPhase::WaitRoll, 60),
        "两个回合之后应当回到 WaitRoll"
    );
    request_save(&mut app);
    let snapshot = taken(&app).expect("可存点应存得下");
    assert_eq!(denied(&app), None);
    // P2c：档里记着棋盘名，读档时对不上就拒绝（`LandId` 只在同一张盘上有意义）
    assert_eq!(
        snapshot.board,
        app.world().resource::<ActiveBoard>().def.board_name,
        "快照必须记下它属于哪张棋盘"
    );

    // 回合数与日历：读快照自己的字段没有意义，对着世界那两份资源比
    let session = app.world().resource::<GameSession>().clone();
    assert_eq!(snapshot.session.turn_count, session.turn_count);
    assert_eq!(snapshot.session.epoch, session.epoch);
    assert_eq!(snapshot.session.price_index, session.price_index);
    assert_eq!(
        snapshot.session.seed,
        app.world().resource::<game_core::rng::GameSeed>().0
    );

    // 每位玩家的两套账：对着 `roster` 那条真读数（现金、点券），而不是快照自证
    for (entity, id, cash, tile) in roster(&mut app) {
        let seat = snapshot
            .players
            .iter()
            .find(|state| state.id.0 == id)
            .unwrap_or_else(|| panic!("快照里少了 P{id}"));
        assert_eq!(seat.wallet.cash, cash, "P{id} 的现金");
        assert_eq!(seat.tile.tile.0, tile, "P{id} 的位置");
        let points = app.world().get::<Points>(entity).expect("玩家有点券").0;
        assert_eq!(seat.points.0, points, "P{id} 的点券");
    }
}

#[test]
fn the_snapshot_round_trips_through_json_byte_for_byte() {
    let mut app = boot_default();
    boot_to_wait_roll(&mut app);
    play_turns(&mut app, 3);
    assert!(run_until_phase(&mut app, TurnPhase::WaitRoll, 60));
    request_save(&mut app);
    let snapshot = taken(&app).expect("可存点应存得下");

    let text = serde_json::to_string(&snapshot).expect("快照必须能写成 JSON");
    let back: GameSnapshot = serde_json::from_str(&text).expect("写出去的 JSON 必须读得回来");
    assert_eq!(back, snapshot, "读回来的快照与原件不等");
    assert_eq!(
        serde_json::to_string(&back).expect("再写一次"),
        text,
        "存→读→再存必须字节一致（排序与 HashMap 混进存档时这一条会红）"
    );
    // 快照里那些「排序过的表」确实是排好的 —— 否则上面那条只在真遇上乱序时才红
    assert!(
        snapshot
            .players
            .windows(2)
            .all(|pair| pair[0].id.0 < pair[1].id.0)
    );
    assert!(
        snapshot
            .lands
            .windows(2)
            .all(|pair| pair[0].id.0 < pair[1].id.0)
    );
    assert!(
        snapshot
            .stocks
            .windows(2)
            .all(|pair| pair[0].id.0 < pair[1].id.0)
    );
}

#[test]
fn every_busy_shape_refuses_the_capture_with_its_own_reason() {
    // ① 还没开局（Boot）
    let mut app = boot_default();
    request_save(&mut app);
    assert_eq!(taken(&app), None);
    assert_eq!(denied(&app), Some(SaveDeny::NotInMatch));

    // ② 有人被挂着等回答
    let mut app = boot_default();
    let current = boot_to_wait_roll(&mut app);
    {
        let mut pending = app.world_mut().resource_mut::<PendingDecision>();
        pending.player = Some(current);
        pending.about = Some(game_core::landing::Pending::Card);
    }
    request_save(&mut app);
    assert_eq!(taken(&app), None);
    assert_eq!(denied(&app), Some(SaveDeny::Waiting));

    // ③ 棋子还在走（路径没走完）
    let mut app = boot_default();
    let current = boot_to_wait_roll(&mut app);
    app.world_mut()
        .entity_mut(current)
        .get_mut::<WalkQueue>()
        .expect("玩家应有行走队列")
        .path
        .push_back(game_core::board::TileIndex(1));
    request_save(&mut app);
    assert_eq!(taken(&app), None);
    assert_eq!(denied(&app), Some(SaveDeny::Walking));
}

/// 快照的**每一块状态**都必须真的被读到。
///
/// 这是「捕获漏读一个字段」唯一的防线：漏读时上面三条全绿 —— 字节一致只证明「写出去的和
/// 读回来的一致」，不证明写进去了什么。做法是给每一块塞一个**不可能由别处推出来**的数，
/// 再断言快照里那一格跟着变。**加状态区块时这一条要跟着加一行**，否则新组件会静默地
/// 不进存档（症状只在读档之后很久才出现）。
#[test]
fn every_state_block_reaches_the_snapshot() {
    let mut app = boot_default();
    let seat = boot_to_wait_roll(&mut app);
    request_save(&mut app);
    let before = taken(&app).expect("baseline");
    let id = app.world().get::<PlayerId>(seat).expect("有身份").0;
    // 破产标记塞给**另一位**：回合机正在问的是 current，别给它的判据添乱
    let other = app
        .world()
        .resource::<TurnOrder>()
        .order
        .iter()
        .copied()
        .find(|entity| *entity != seat)
        .expect("两人盘上的另一位");
    let other_id = app.world().get::<PlayerId>(other).expect("另一位有身份").0;
    let (land_entity, land_id) = {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &LandId)>();
        let mut rows: Vec<(Entity, LandId)> = query
            .iter(world)
            .map(|(entity, id)| (entity, *id))
            .collect();
        rows.sort_by_key(|(_, id)| id.0);
        rows[0]
    };

    {
        let world = app.world_mut();
        let mut seat_entity = world.entity_mut(seat);
        seat_entity.get_mut::<Wallet>().expect("钱包").cash = 4_321;
        seat_entity.get_mut::<Points>().expect("点券").0 = 99;
        seat_entity.get_mut::<BankBalance>().expect("银行").deposit = 1_000;
        seat_entity.get_mut::<BankBalance>().expect("银行").loan = 200;
        seat_entity.get_mut::<TokenPosition>().expect("位置").tile = TileIndex(3);
        *seat_entity.get_mut::<Heading>().expect("朝向") = Heading::Backward;
        *seat_entity.get_mut::<Mount>().expect("载具") = Mount::Car;
        seat_entity
            .get_mut::<Hand>()
            .expect("手牌")
            .cards
            .push_back(CardId(0));
        seat_entity
            .get_mut::<Inventory>()
            .expect("道具")
            .gain(ItemKind::Mine, 2);
        seat_entity
            .get_mut::<StockHoldings>()
            .expect("持仓")
            .0
            .insert(StockId("tech".into()), 7);
        seat_entity
            .get_mut::<LotteryTickets>()
            .expect("持号")
            .numbers
            .push(5);
        seat_entity.insert(Detention {
            place: DetentionPlace::Hospital,
            term: Term::Turns(2),
        });
        seat_entity.insert(StaySentence { turns_left: 2 });
        seat_entity.insert(MoveCap { turns_left: 3 });
        seat_entity.insert(Insured {
            days_left: 4,
            insurer: "tech".into(),
            indemnity_per_day: 10,
        });
        seat_entity.insert(Possession::new(God::BigRich));
    }
    app.world_mut().entity_mut(other).insert(Bankrupt);

    {
        let world = app.world_mut();
        let mut land = world.entity_mut(land_entity);
        *land.get_mut::<Owner>().expect("归属") = Owner(Some(PlayerId(id)));
        *land.get_mut::<BuildingLevel>().expect("等级") = BuildingLevel(3);
        *land.get_mut::<Mortgaged>().expect("抵押") = Mortgaged { proceeds: 700 };
        land.insert(Facility(BuildingKind::Mall));
        land.insert(Timed::new(LandStatus::PriceHike, 4));
    }

    {
        let world = app.world_mut();
        world
            .resource_mut::<Corrections>()
            .register(Correction::always(
                seat,
                Scope::Any,
                Adjustment::Cancel,
                Source::God,
            ));
        *world.resource_mut::<AtmAccess>() = AtmAccess(Some(seat));
        *world.resource_mut::<Jackpot>() = Jackpot {
            pot: 999,
            last_draw: Some(7),
        };
        world.resource_mut::<ShopStock>().slots = vec![CardId(0)];
        world.resource_mut::<CouponBoxes>().take(TileIndex(1), 0, 5);
        let mut session = world.resource_mut::<GameSession>();
        session.last_news = Some(NewsId(1));
        session.suspensions.suspend(Institution::StockMarket, 3);
        session.bank_vault = 123;
    }
    add_company_entry(&mut app, "tech", 500, "测试入账");
    app.world_mut().resource_mut::<Rng>().roll_die();

    request_save(&mut app);
    let after = taken(&app).expect("这些改动都没离开可存点");
    assert_eq!(denied(&app), None);

    let me = after
        .players
        .iter()
        .find(|state| state.id.0 == id)
        .expect("P{id} 在快照里");
    assert_eq!(me.wallet.cash, 4_321);
    assert_eq!(me.points.0, 99);
    assert_eq!(me.bank.deposit, 1_000);
    assert_eq!(me.bank.loan, 200);
    assert_eq!(me.tile.tile, TileIndex(3));
    assert_eq!(me.heading, Heading::Backward);
    assert_eq!(me.mount, Mount::Car);
    assert_eq!(me.hand.cards.len(), 1, "手里多了一张卡");
    assert_eq!(me.inventory.count(ItemKind::Mine), 2, "道具栏按 kind 堆叠");
    assert_eq!(me.holdings, vec![(StockId("tech".into()), 7)]);
    assert_eq!(me.tickets.numbers, vec![5]);
    assert_eq!(
        me.detention,
        Some(Detention {
            place: DetentionPlace::Hospital,
            term: Term::Turns(2)
        })
    );
    assert_eq!(me.stay, Some(StaySentence { turns_left: 2 }));
    assert_eq!(me.move_cap, Some(MoveCap { turns_left: 3 }));
    assert_eq!(
        me.insured,
        Some(Insured {
            days_left: 4,
            insurer: "tech".into(),
            indemnity_per_day: 10
        })
    );
    assert_eq!(me.possession.map(|held| held.god), Some(God::BigRich));
    assert!(
        after
            .players
            .iter()
            .find(|state| state.id.0 == other_id)
            .expect("另一位在快照里")
            .bankrupt
    );

    let land = after
        .lands
        .iter()
        .find(|state| state.id.0 == land_id.0)
        .expect("那块地在快照里");
    assert_eq!(land.owner, Some(PlayerId(id)));
    assert_eq!(land.level, BuildingLevel(3));
    assert_eq!(land.mortgaged.proceeds, 700);
    assert_eq!(land.facility, Some(BuildingKind::Mall));
    assert_eq!(land.status, Some((LandStatus::PriceHike, 4)));

    assert_eq!(after.corrections.len(), before.corrections.len() + 1);
    assert_eq!(after.atm, Some(PlayerId(id)));
    assert_eq!(after.jackpot.pot, 999);
    assert_eq!(after.jackpot.last_draw, Some(7));
    assert_eq!(after.shop.slots, vec![CardId(0)]);
    assert!(
        after.coupon_boxes.iter().any(|(tile, _)| tile.0 == 1),
        "点券箱冷却表"
    );
    assert_eq!(after.session.last_news, Some(NewsId(1)));
    assert_eq!(
        after
            .session
            .suspensions
            .days_left(Institution::StockMarket),
        3
    );
    assert_eq!(after.session.bank_vault, 123);
    assert!(!after.ledgers.is_empty(), "公司当月账页");
    assert_ne!(after.rng, before.rng, "随机流水位");
}

/// P2c：档里记着棋盘名。把一张原型盘的档改成「台湾盘」去读，`begin_game_system`
/// 必须**按新局开**而不是把 `LandId` 套到另一张盘的前六块地上 —— 后者不会崩，
/// 只会把业主与地名全装错，是最难在画面上认出来的那一种错。
#[test]
fn a_save_from_another_board_starts_a_fresh_match_instead_of_mixing_land_ids() {
    let mut living = boot_default();
    boot_to_wait_roll(&mut living);
    play_turns(&mut living, 5);
    assert!(run_until_phase(&mut living, TurnPhase::WaitRoll, 60));
    request_save(&mut living);
    let mut snapshot = taken(&living).expect("可存点应存得下");
    snapshot.board = "taiwan".into();
    snapshot.session.turn_count = 99;

    let mut other = boot_default();
    other
        .world_mut()
        .insert_resource(StartMode::Resume(Box::new(snapshot)));
    assert!(
        run_until_world(&mut other),
        "对不上盘之后仍应铺出一局新世界"
    );
    assert_eq!(
        other.world().resource::<ActiveBoard>().def.board_name,
        "prototype"
    );
    assert_eq!(
        other.world().resource::<GameSession>().turn_count,
        0,
        "盘对不上就不能按存档续跑，必须从新局开始"
    );
}

/// P15b 的核心验收：**读档继续 == 没读档继续**。
///
/// A 是原时间线：跑 6 回合、存档、再跑 4 回合；B 从那张档开一局、跑同样 4 回合。
/// 两边最后各拍一张快照，**逐字节相同**才算过 —— 差一个字段、一条随机流的水位没摆回、
/// 一个 `TurnStart` 被重跑两次，这条都会红，而画面上的差别要几十回合后才看得出来。
#[test]
fn a_loaded_match_continues_exactly_where_it_stopped() {
    let mut living = boot_default();
    boot_to_wait_roll(&mut living);
    play_turns(&mut living, 6);
    assert!(
        run_until_phase(&mut living, TurnPhase::WaitRoll, 60),
        "第 6 回合之后应当回到 WaitRoll"
    );
    request_save(&mut living);
    let snapshot = taken(&living).expect("可存点应存得下");

    let mut loaded = boot_default();
    loaded
        .world_mut()
        .insert_resource(StartMode::Resume(Box::new(snapshot.clone())));
    assert!(run_until_world(&mut loaded), "读档之后世界应当装配起来");
    // `NextState` 要等下一帧的 `StateTransition` 才落地（坑 12/29）
    assert!(
        run_until_phase(&mut loaded, TurnPhase::WaitRoll, 10),
        "读档之后应当落在 WaitRoll"
    );
    assert_eq!(
        phase(&loaded),
        TurnPhase::WaitRoll,
        "落点是「他还没开始这一回合」，不是 TurnStart"
    );
    assert_eq!(
        loaded.world().resource::<GameSession>().turn_count,
        snapshot.session.turn_count,
        "回合数照存档"
    );

    // `play_turns` 的第二个参数是**绝对**回合数（跑到 `turn_count >= want` 为止）
    play_turns(&mut living, 10);
    assert!(run_until_phase(&mut living, TurnPhase::WaitRoll, 60));
    play_turns(&mut loaded, 10);
    assert!(run_until_phase(&mut loaded, TurnPhase::WaitRoll, 60));

    let living_turn = living.world().resource::<GameSession>().turn_count;
    let loaded_turn = loaded.world().resource::<GameSession>().turn_count;
    assert!(
        living_turn == 10 && loaded_turn == 10,
        "两条时间线都真的走到了第 10 回合（否则下面那条字节比对是空跑）：         存档时 {} 回合、原时间线 {living_turn}、读档侧 {loaded_turn}",
        snapshot.session.turn_count
    );

    request_save(&mut living);
    let after_living = taken(&living).expect("原时间线还能存");
    request_save(&mut loaded);
    let after_loaded = taken(&loaded).expect("读档那一条也还能存");
    assert_eq!(
        serde_json::to_string(&after_loaded).expect("读档侧快照"),
        serde_json::to_string(&after_living).expect("原时间线快照"),
        "读档继续与原时间线分叉了（随机流水位、回合落点或某个状态块没摆回去）"
    );
}

/// P15b 接线：`judge_system` 每帧把「此刻能不能存」抄进 `SaveDenied`。
///
/// 这条钉的是**接线而不是判据**（判据在 `snapshot.rs` 的单测里逐条打过）：不挂这条
/// system，`SaveDenied` 会一直停在默认的 `None`，暂停菜单那颗按钮于是永远亮着 ——
/// 而别的用例一条都不会红，因为 `capture_match` 自己会拒。
#[test]
fn the_save_point_reading_follows_the_world_every_frame() {
    let mut app = boot_default();
    app.update();
    assert_eq!(
        denied(&app),
        Some(SaveDeny::NotInMatch),
        "还没开局就说过「能存」"
    );

    let current = boot_to_wait_roll(&mut app);
    assert_eq!(denied(&app), None, "轮到人类、还没掷骰 —— 这一帧就是可存点");

    // 挂一个窗口上去：读数必须当帧就变（这正是「按钮灰着并说出为什么」的那条数据）
    {
        let mut pending = app.world_mut().resource_mut::<PendingDecision>();
        pending.player = Some(current);
        pending.about = Some(game_core::landing::Pending::Card);
    }
    app.update();
    assert_eq!(
        denied(&app),
        Some(SaveDeny::Waiting),
        "窗口挂上去之后读数没跟上"
    );
}
