//! P6 的验收：神在路上的那一半（上身、顶走、对位、天数、漫游）与钱的那一半（P6b）。
//!
//! PLAN §四 P6 给了三条验收，这里逐条对上：
//! - **①「金额对得上 P2 测试」** → [`the_same_five_thousand_rent_under_the_poor_gods`]：
//!   P2 那四条状态里的「大穷神加倍」那一行，现在由一位**真神**复现，四个数逐字相同。
//!   它靠的是 `Landed` 链里的先后 —— 神站在脚下那块地产格上，先上身、后收租。
//! - **②「顶走旧神时旧神的对位新神才入场」** 分两处钉：可入场的判据是纯函数，
//!   由 `gods.rs` 的 `a_pair_keeps_its_counterpart_out_of_the_pool_until_it_leaves` 钉；
//!   「真跑起来时同一路不会挤着两位」是世界的性质，由下面
//!   [`counterparts_never_stand_in_play_together`] 在 30 回合的长跑里逐回合查。
//!   合成一条用例会两头都测不准：抽到哪一位由随机子流决定，而断言要的是「不该同时出现」。
//! - **③「坐牢 3 天期间神的 7 天照算 3 天」** → [`the_god_ages_while_he_is_locked_up`]。
//!
//! ADR-002 的两条结构义务也在这里：神是**落脚**触发（路过不给）、神换格走的是同一道
//! `place` 闸门（所以它永远不与别的放置物共占一格）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{
    CardAim, ChargeKind, Detention, DetentionPlace, GameRules, GameSeed, GameSetup, God, LandId,
    MAX_ON_BOARD, OccupancyKind, PlayerId, Possession, Rival, STARTING_GODS, StaySentence,
    TileKind,
};

/// prototype 的 1 号格挂着 L0 南京路（见 `settle.rs` 顶上那张格子对照表）。
/// 把一位神放上**同一格**是合法的：occupancy 与地块是两套实体（ADR-002），
/// 而这一格正好让「先上身、后收租」发生在同一条 `Landed` 链里。
const RENT_TILE: u32 = 1;
const RENT_LAND: u32 = 0;

// ===== 小工具 =====

/// 把一段路改成纯路面：用例要的是「踩到一位神」，不是「这一格该不该买」。
fn pave(app: &mut App, tiles: std::ops::RangeInclusive<u32>) {
    for tile in tiles {
        retile(app, tile, TileKind::Road);
    }
}

/// 默认阵容里的另一名玩家（把两个人都按住时要一起按）。
fn other_of(app: &mut App, me: Entity) -> Entity {
    roster(app)
        .iter()
        .find(|(e, ..)| *e != me)
        .map(|(e, ..)| *e)
        .expect("默认阵容有两名玩家")
}

/// 在场集合里查一位神在不在（路上或身上都算）。
fn in_play(app: &mut App, god: God) -> bool {
    gods_in_play(app).contains(&god)
}

/// 报一个价并**就地**断言。
///
/// 为什么断言写在闭包里而不是把数抄到外面比：`run_op` 的闭包是 `FnOnce`，编译器看不见
/// 它对捕获变量的赋值被外面读，于是 `unused_assignments` 一路 warning 到底；而这条读数
/// 唯一用途就是断言。就地比较还保得住那句「哪笔钱、对谁、名义多少、实付多少」+ 期望值。
/// 先例：`settle.rs` 里那批 `run_op(.., assert!(books.borrow(..)))`。
fn assert_quote(app: &mut App, kind: ChargeKind, player: PlayerId, base: i64, want: i64) {
    // 先把指数钉回 1.0。这些用例都要 `own()` 凭空塞一块地给某个人，全场资产因此多出
    // 一截、回合末的指数就不再是 1.0（P4② 记过同一条），而这里问的是**链在不在**、
    // 不是指数公式 —— 让物价那一乘留在公式自己的用例里。
    set_price_index(app, 1.0);
    run_op(app, move |books, _| {
        let me = books.holder(player).expect("这个编号在阵容里");
        let got = books.quoted(kind, me, base);
        assert_eq!(got, want, "{kind:?} 名义 {base} 对 P{} 实付不对", player.0);
    });
}

/// 过路费 L0 那一句的简称（P6b 的三条验收都问它）。
fn assert_rent_quote(app: &mut App, player: PlayerId, want: i64) {
    assert_quote(
        app,
        ChargeKind::Rent {
            land: LandId(RENT_LAND),
        },
        player,
        5_000,
        want,
    );
}

// ===== P6a：在路上、上身、顶走、天数、漫游 =====

#[test]
fn stepping_on_a_god_puts_it_on_him_and_off_the_road() {
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    spawn_god(&mut app, God::BigRich, 2);

    play_turns(&mut app, 1);

    assert_eq!(
        possession_of(&app, p0),
        Some(Possession {
            god: God::BigRich,
            days_left: 7
        }),
        "踩到财神就该上身，七天是 PLAN §三 写死的"
    );
    assert!(
        !gods_on_board(&mut app)
            .iter()
            .any(|(_, god)| *god == God::BigRich),
        "他已经变成他身上的那位，路上不该再站着一个"
    );
    let ledger = ledger(&app);
    assert!(
        ledger.contains(&"GodOn(P0:大财神,7天)".to_string()),
        "上身要留凭据：{ledger:?}"
    );
}

#[test]
fn walking_past_a_god_gives_nothing() {
    // ADR-002 §5：神是**落脚**触发。经过那一格不算踩到 —— 与宝箱同一条口径
    let (mut app, p0) = ready_on(0, 3, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    spawn_god(&mut app, God::BigRich, 2);

    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), 3, "这一趟应当正好走过它");
    assert!(
        possession_of(&app, p0).is_none(),
        "路过财神就给祝福，那「绕开它」就不是玩法了"
    );
    assert!(
        cash_of(&mut app, p0) == 20_000,
        "路过大财神一分不给：现金 {:?}",
        cash_of(&mut app, p0)
    );
    assert!(
        gods_on_board(&mut app)
            .iter()
            .any(|(tile, god)| *tile == 2 && *god == God::BigRich)
            || ledger(&app)
                .iter()
                .any(|line| line.starts_with("GodAppear")),
        "它没被路过吃掉：{:#?}",
        gods_on_board(&mut app)
    );
}

#[test]
fn a_second_god_tops_the_first_one_off_and_frees_his_slot() {
    // 验收②的那半条「顶走」：一人一神由组件保证，这里验的是凭据与槽位
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    attach_god(&mut app, p0, God::BigRich, None);
    spawn_god(&mut app, God::SmallPoor, 2);

    assert!(
        !god_pool(&mut app).contains(&God::SmallRich),
        "大财神还站在他身上，小财神这一路不能有人"
    );

    play_turns(&mut app, 1);

    assert_eq!(
        possession_of(&app, p0).map(|held| held.god),
        Some(God::SmallPoor),
        "新神顶旧神，不是叠第二层"
    );
    assert!(
        !in_play(&mut app, God::BigRich) || !in_play(&mut app, God::SmallRich),
        "同一路两位同时在场，对位规则白写"
    );
    assert!(
        !in_play(&mut app, God::BigRich),
        "被顶走的那位该彻底离场，而不是躲在谁身上"
    );
    let ledger = ledger(&app);
    assert!(
        ledger.contains(&"GodOn(P0:小穷神,7天顶走大财神)".to_string()),
        "顶走要说得出被顶掉的是谁：{ledger:?}"
    );
    assert!(
        ledger.contains(&"GodOff(P0:大财神,顶走)".to_string()),
        "旧神那条离场凭据不能省：{ledger:?}"
    );
    // 他那一路空出来了 → 小的这才可以入场（「才入场」的判据本身由纯函数那条钉）
    assert!(
        god_pool(&mut app).contains(&God::BigRich),
        "大财神的槽位已经空出来，池子里要能看见他"
    );
}

#[test]
fn counterparts_never_stand_in_play_together() {
    // 验收②的那半条「不同场」：30 回合长跑里逐回合查这个世界的性质
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(2),
    );
    let p0 = boot_to_wait_roll(&mut app);
    pave(&mut app, 1..=12);
    let mut seen: Vec<God> = Vec::new();
    for turn in 2..=30 {
        play_turns(&mut app, turn);
        let rows = gods_in_play(&mut app);
        for god in &rows {
            if !seen.contains(god) {
                seen.push(*god);
            }
            match god.rival() {
                Rival::God(other) => assert!(
                    !rows.contains(&other),
                    "{} 与 {} 同时在场（回合 {turn}，P0 身上 {:?}）",
                    god.label(),
                    other.label(),
                    possession_of(&app, p0)
                ),
                Rival::Dog => assert!(
                    !occupancies_on(&mut app)
                        .iter()
                        .any(|(_, kind)| *kind == OccupancyKind::Dog),
                    "土地公与恶犬是同一路，不能同时在场（回合 {turn}）"
                ),
                Rival::Alone => {}
            }
            assert_ne!(*god, God::Death, "死神不上街，它是被召来的（回合 {turn}）");
        }
        // 补刷只补到名额为止：路上不该越跑越多位
        assert!(
            gods_on_board(&mut app).len() <= MAX_ON_BOARD,
            "路上堆了超过名额的几位（回合 {turn}）"
        );
        // 一格一件对神同样成立
        let mut tiles: Vec<u32> = occupancies_on(&mut app)
            .iter()
            .map(|(tile, _)| *tile)
            .collect();
        let before = tiles.len();
        tiles.sort();
        tiles.dedup();
        assert_eq!(tiles.len(), before, "两件东西占了同一格（回合 {turn}）");
    }
    assert!(
        seen.len() >= 3,
        "30 回合只撞见过 {} 位神，刷出这条路等于没跑：{seen:?}",
        seen.len()
    );
}

#[test]
fn the_top_up_draw_never_picks_a_counterpart() {
    // 上面那条长跑**本来测不到这条规则**：两位神一路站在没人踩的格子上，补刷整个
    // 30 回合一次签都没抽 —— 第一次 red-check 短路掉对位判据时它照样绿。
    //
    // 这条每回合只清**路上**那几位（身上那位留着当「已占的那一路」），于是补刷每回合
    // 真的要去抽一次，而它每次都绕开小财神。两边一起清等于把对位规则要测的那半个世界
    // 也清掉了，所以不能那么写。
    let mut app = boot(GameSetup::default(), GameRules::instant(), GameSeed(9_931));
    let p0 = boot_to_wait_roll(&mut app);
    pave(&mut app, 1..=12);
    attach_god(&mut app, p0, God::BigRich, Some(60));

    let mut draws = 0;
    for turn in 2..=60 {
        clear_board_gods(&mut app);
        play_turns(&mut app, turn);
        // 他身上那位可能被路上的一位顶掉（那是另一条合法规则），每轮重新占住这一路
        if possession_of(&app, p0).map(|held| held.god) != Some(God::BigRich) {
            attach_god(&mut app, p0, God::BigRich, Some(60));
        }
        let rows = gods_in_play(&mut app);
        let fresh: Vec<God> = rows
            .iter()
            .copied()
            .filter(|g| *g != God::BigRich)
            .collect();
        if fresh.is_empty() {
            continue;
        }
        draws += fresh.len();
        for god in &fresh {
            if let Rival::God(other) = god.rival() {
                assert!(
                    !rows.contains(&other),
                    "第 {draws} 次抽签抽出了一对：{} 与 {}（回合 {turn}）",
                    god.label(),
                    other.label()
                );
            }
            assert_ne!(*god, God::Death, "抽签永远抽不到死神（回合 {turn}）");
        }
    }
    assert!(draws >= 20, "只跑到 {draws} 次抽签，这条用例就又变成空跑了");
}

#[test]
fn the_board_starts_with_the_complement_and_no_rival_pair() {
    let mut app = boot(
        GameSetup::default(),
        GameRules::instant(),
        seed_rolling_first(3),
    );
    assert!(run_until_world(&mut app), "装配应在时限内完成");
    settle(&mut app, 2);

    let rows = gods_on_board(&mut app);
    assert_eq!(rows.len(), STARTING_GODS, "开局该在路上放满这几位");
    for (tile, god) in &rows {
        if let Rival::God(other) = god.rival() {
            assert!(
                !rows.iter().any(|(_, other_god)| *other_god == other),
                "开局就同时放了 {} 与 {}（格{tile}）",
                god.label(),
                other.label()
            );
        }
        assert_ne!(*god, God::Death, "开局第一位就撞见死神，那不是祝福");
    }
}

#[test]
fn same_seed_gives_the_same_gods_on_the_same_tiles() {
    let once = || {
        let mut app = boot(GameSetup::default(), GameRules::instant(), GameSeed(4242));
        assert!(run_until_world(&mut app));
        settle(&mut app, 2);
        gods_on_board(&mut app)
    };
    assert_eq!(
        once(),
        once(),
        "同种子必须复现同一批神与同一批格子，否则 `NM_SEED` 与回放一起失效"
    );
}

#[test]
fn the_god_ages_while_he_is_locked_up() {
    // 验收③：他被关着的那些回合，回合照样开始、神的计数照样走。
    //
    // 断言写成「跳了几次回合 ⇒ 老了几格」的关系，而不是写死一个剩余天数：
    // `ready_on` 已经把他的第一个回合走过了（那一次扣数落在他挂上神之前），
    // 手算全局回合与个人回合的对应会把这条测成一条永远差一格的红线 ——
    // 而关系式对任何前置都成立，也照样能在「拘留期间不扣」的错误下变红。
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    attach_god(&mut app, p0, God::BigFortune, None);
    app.world_mut().entity_mut(p0).insert(Detention {
        place: DetentionPlace::Jail,
        turns_left: 6,
    });

    play_turns(&mut app, 9);

    // 跳过回合那条凭据在**回合流水**里（`events`），不在账本流水（`ledger`）里 ——
    // 两条流水分别由 `collect_flow` 与 `collect_books` 收，别在错的那一份里数
    let skipped = events(&app)
        .iter()
        .filter(|line| line.starts_with("DetainSkip(P0@"))
        .count();
    assert!(
        skipped >= 3,
        "他一个回合都没被跳过，那这条用例什么都没测：{skipped}"
    );
    let held = possession_of(&app, p0).expect("他被关着，但神还在他身上");
    assert_eq!(
        held.days_left,
        God::BigFortune.days() - skipped as u32,
        "跳了 {skipped} 个回合，神就该老 {skipped} 天 —— 拘留不许让它停住"
    );
    let sentence = detain_of(&app, p0).expect("这一批回合还关不住他，前置才有意义");
    assert_eq!(
        sentence.turns_left,
        6 - skipped as u32,
        "两条计时数的是同一批回合：拘留扣 {skipped}，神也扣 {skipped}"
    );
    assert!(
        !ledger(&app).iter().any(|line| line.starts_with("GodOff")),
        "还没到期，不该有离场凭据"
    );
}

#[test]
fn a_god_leaves_when_its_days_run_out() {
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    attach_god(&mut app, p0, God::BigPoor, Some(1));

    play_turns(&mut app, 3);

    let ledger = ledger(&app);
    assert!(
        ledger.contains(&"GodOff(P0:大穷神,期满)".to_string()),
        "最后一天那一回合开始时离场，并留一句期满：{ledger:?}"
    );
    assert_ne!(
        possession_of(&app, p0).map(|held| held.god),
        Some(God::BigPoor),
        "离场之后他身上不该还挂着那一位"
    );
}

#[test]
fn the_land_deity_hands_his_spot_to_a_dog() {
    // PLAN §三「土地公＝…期满变恶犬」：接位是那条对位规则的另一半
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    attach_god(&mut app, p0, God::LandDeity, Some(1));

    play_turns(&mut app, 3);

    let rows = occupancies_on(&mut app);
    assert!(
        rows.iter().any(|(_, kind)| *kind == OccupancyKind::Dog),
        "土地公期满要留下一条恶犬：{rows:?}"
    );
    assert!(
        !in_play(&mut app, God::LandDeity),
        "它接的是同一路，土地公本人不该还在场"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line == "GodOff(P0:土地公,期满)"),
        "接位之前先要有一条离场凭据"
    );
}

#[test]
fn a_god_that_walks_onto_a_player_does_not_possess_him() {
    // ADR-002 §9 把这半条交给 P6 拍板，这里拍的是「不算」：附身只由落脚触发。
    //
    // 前置要让**两个人都不动**（`play_turns` 会替身上有停留计时的人按那颗「停留」）：
    // 第一版让他俩照常掷骰，40 回合里那位神既没撞上人、还半路被对面踩走了，
    // 于是这条用例什么都没测。现在场上唯一会动的就是神自己，
    // 它每一步都写的是同一条 `at = OnTile(…)` —— 而那正是「走到人那一格」的全部形状。
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    spawn_god(&mut app, God::BigFortune, 3);
    place(&mut app, p0, 2);
    place(&mut app, p1, 8);
    for player in [p0, p1] {
        insert(&mut app, player, StaySentence { turns_left: 80 });
    }

    let mut overlapped = false;
    for turn in 2..=60 {
        play_turns(&mut app, turn);
        if gods_on_board(&mut app)
            .iter()
            .any(|(tile, god)| *god == God::BigFortune && *tile == tile_of(&app, p0))
        {
            overlapped = true;
            assert_ne!(
                possession_of(&app, p0).map(|held| held.god),
                Some(God::BigFortune),
                "神自己走到他那一格就附身了 —— 附身不该是一件世界事件（回合 {turn}）"
            );
            break;
        }
    }
    assert!(
        overlapped,
        "60 个回合里那位神没走到有人站着的那一格，用例白跑"
    );
    assert!(
        in_play(&mut app, God::BigFortune),
        "两个人都不动，那位神不该被谁吃掉"
    );
}

#[test]
fn an_attached_player_keeps_playing() {
    // 附身不是拘留：他被那位神跟着走，但回合机里他照样掷骰、照样挪格子
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    spawn_god(&mut app, God::SmallFortune, 2);

    play_turns(&mut app, 3);

    assert!(
        possession_of(&app, p0).is_some(),
        "他身上得挂着一位 —— 附身不会因为又走了一步就自己掉"
    );
    // 骰子最多 6 格而环有 13 格，所以他不可能一回合绕回踩到的那一格
    assert!(
        tile_of(&app, p0) != 2,
        "他被钉在踩到神的那一格上了 —— 附身不该挡住行走"
    );
}

// ===== P6b：钱的那一半 =====

/// 造一条「P0 踩到 L0（P1 的，名义租金 5000），脚下站着 `god`」的对局，
/// 回来 `(付款人现金, 业主现金)`。形状与 `price_index.rs::five_thousand` 一致 ——
/// 这样验收①才是「对得上」而不是「另算了一套」。
fn rent_with(god: Option<God>) -> (i64, i64) {
    let (mut app, payer) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let owner = other_of(&mut app, payer);
    clear_gods(&mut app);
    own(&mut app, RENT_LAND, Some(1), 1, false);
    flat_rent(&mut app, RENT_LAND, 5_000);
    if let Some(god) = god {
        spawn_god(&mut app, god, RENT_TILE);
    }

    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, payer), RENT_TILE, "前置：他要真的踩到那一格");
    (cash_of(&mut app, payer), cash_of(&mut app, owner))
}

#[test]
fn the_same_five_thousand_rent_under_the_poor_gods() {
    // 验收①：P2 那四条状态里的三行，现在由**真神**给出来。
    // 业主那一侧才是对照物（他不受那位神影响，这一笔到没到手看得见）；
    // 付款人那侧在大穷神这里叠了两件事 —— 上身那一刻先被抢走四分之一，再付双倍租金
    assert_eq!(
        rent_with(None),
        (15_000, 25_000),
        "没有神：5000 原样从付款人转到业主手上（P2 那一行的基线）"
    );
    assert_eq!(
        rent_with(Some(God::BigPoor)),
        (5_000, 30_000),
        "大穷神：业主收到 30000 ＝ 与 `price_index.rs` 那一行同一条数，而且加倍的钱真的到了手。 \
         付款人 5000 = 20000 − 当场被抢的 1/4（5000）− 双倍租金（10000）"
    );
    assert_eq!(
        rent_with(Some(God::SmallPoor)),
        (12_500, 27_500),
        "小穷神只抬五成，也不抢钱：一个 2.0、一个 1.5，写反了这里就红"
    );
}

#[test]
fn big_rich_wipes_the_rent_and_hands_over_a_four_digit_gift() {
    // 同一条链上的两半：`Rent → Cancel` 免掉这一笔，`bestow` 当场给一笔四位数。
    // 业主那侧单独断言，才能把「免掉了」与「他给了钱所以现金看着没变」分清楚
    let (mut app, payer) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let owner = other_of(&mut app, payer);
    clear_gods(&mut app);
    own(&mut app, RENT_LAND, Some(1), 1, false);
    flat_rent(&mut app, RENT_LAND, 5_000);
    spawn_god(&mut app, God::BigRich, RENT_TILE);

    play_turns(&mut app, 1);

    assert_eq!(
        cash_of(&mut app, owner),
        20_000,
        "整笔抵消：业主一分没收到，而不是「付了 0 元再退回来」"
    );
    let gift = cash_of(&mut app, payer) - 20_000;
    assert!(
        (1_000..=9_999).contains(&gift),
        "大财神那笔该是四位数（1000–9999），实际 {gift}"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("GodDeed(P0:大财神,+")),
        "上身那一刻做的事要单独留一条凭据：{:?}",
        ledger(&app)
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn small_rich_collects_a_three_digit_sum_from_everyone_else() {
    let (mut app, payer) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    let victim = other_of(&mut app, payer);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    spawn_god(&mut app, God::SmallRich, 2);

    play_turns(&mut app, 1);

    let gained = cash_of(&mut app, payer) - 20_000;
    let lost = 20_000 - cash_of(&mut app, victim);
    assert!(
        (100..=999).contains(&gained),
        "向每人收的该是三位数，实际 {gained}"
    );
    assert_eq!(
        gained, lost,
        "两人是同一笔钱的两半（不是凭空印、也不是蒸发）"
    );
    assert!(
        possession_of(&app, payer).is_some(),
        "小财神要留在他身上七天，收完这一笔就走的话那条减半就没主体了"
    );
    assert_cash_audit(&mut app, 20_000);
}

#[test]
fn a_god_s_collection_does_not_grow_with_the_price_index() {
    // PLAN §三「物价指数点名豁免神明金额」。这一笔过的是 `Books::settle`，
    // 所以豁免不是注释里的承诺：把那一笔写成 `ChargeKind::Penalty` 的话，
    // 物价 2.5 会让它涨到 2497 —— 这里用「不超过 999」把它挡在门外
    let (mut app, payer) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    let victim = other_of(&mut app, payer);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    spawn_god(&mut app, God::SmallRich, 2);
    set_price_index(&mut app, 2.5);

    play_turns(&mut app, 1);

    assert!(
        possession_of(&app, payer).is_some(),
        "前置：他该真的踩到那位神，否则这里什么都没测"
    );
    let gained = cash_of(&mut app, payer) - 20_000;
    assert!(
        (100..=999).contains(&gained),
        "物价 2.5 之下小财神那笔仍该是三位数：上限挡放大（写成 `Penalty` 就是 2497），\
         下限挡「什么都没收」—— red-check 时只写上限的那一版在 bestow 整块不装配时照样绿",
    );
    assert!(
        cash_of(&mut app, victim) >= 19_001,
        "被收的那位最多只该被收掉三位数：{:?}",
        cash_of(&mut app, victim)
    );
    assert_eq!(
        20_000 - cash_of(&mut app, victim),
        gained,
        "两个人读的是同一笔钱"
    );
}

#[test]
fn big_poor_reaches_into_his_pocket_by_a_quarter() {
    // 抢的是**比例**而不是固定数：物价 5 倍时一个固定数就成了白送（`rules.rs` 那格的注）。
    // 20_001 那一笔同时钉住取整方向 —— 向下，与抵押/回收那两条同一口径
    for cash in [20_000, 20_001, 999] {
        let (mut app, payer) = ready_on(0, 2, DecisionPolicy::DeclineAll);
        pave(&mut app, 1..=12);
        clear_gods(&mut app);
        set_cash(&mut app, payer, cash);
        spawn_god(&mut app, God::BigPoor, 2);

        play_turns(&mut app, 1);

        assert_eq!(
            possession_of(&app, payer).map(|held| held.god),
            Some(God::BigPoor),
            "前置：他要真的踩到大穷神（现金 {cash}）"
        );
        assert_eq!(
            cash_of(&mut app, payer),
            cash - cash / 4,
            "现金 {cash} 被大穷神抢过之后不对 —— `poor_cash_divisor` 是 4，向下取整"
        );
    }
}

#[test]
fn the_fortune_gods_hand_out_cards_and_no_cash() {
    for (god, want) in [(God::BigFortune, 2), (God::SmallFortune, 1)] {
        let (mut app, payer) = ready_on(0, 2, DecisionPolicy::DeclineAll);
        pave(&mut app, 1..=12);
        clear_gods(&mut app);
        let before = hand_of(&app, payer).len();
        spawn_god(&mut app, god, 2);

        play_turns(&mut app, 1);

        assert_eq!(
            possession_of(&app, payer).map(|held| held.god),
            Some(god),
            "前置：他要真的踩到{}",
            god.label()
        );
        assert_eq!(
            hand_of(&app, payer).len(),
            before + want,
            "{} 该送 {want} 张卡",
            god.label()
        );
        assert_eq!(
            cash_of(&mut app, payer),
            20_000,
            "{} 送的是卡，不是钱",
            god.label()
        );
        let deeds = ledger(&app)
            .iter()
            .filter(|line| line.starts_with("GodDeed"))
            .cloned()
            .collect::<Vec<_>>();
        assert_eq!(
            deeds.len(),
            1,
            "{} 的送卡要留一条凭据：{:?}",
            god.label(),
            ledger(&app)
        );
        assert!(
            deeds[0].contains("送卡"),
            "凭据里要说得出送的是卡：{deeds:?}"
        );
    }
}

#[test]
fn a_god_with_no_bestowal_leaves_the_wallet_alone() {
    // 小穷神与两位衰神**不当场动钱**：他们的钱在常驻那条链上。
    // 这一条挡的是「`bestow` 的穷尽表填错格」—— 比如把大穷神那笔抢钱复制给了小穷神
    for god in [God::SmallPoor, God::BigDoom, God::SmallDoom] {
        let (mut app, payer) = ready_on(0, 2, DecisionPolicy::DeclineAll);
        pave(&mut app, 1..=12);
        clear_gods(&mut app);
        spawn_god(&mut app, god, 2);

        play_turns(&mut app, 1);

        assert_eq!(
            possession_of(&app, payer).map(|held| held.god),
            Some(god),
            "前置：他该真的踩到{}（没踩到就等于什么都没测）",
            god.label()
        );
        assert_eq!(
            cash_of(&mut app, payer),
            20_000,
            "{} 上身那一刻不该动现金",
            god.label()
        );
        assert!(
            !ledger(&app).iter().any(|line| line.starts_with("GodDeed")),
            "{} 没有当场做的事，就不该有一条举动的凭据：{:?}",
            god.label(),
            ledger(&app)
        );
    }
}

#[test]
fn the_chain_goes_away_with_the_god() {
    // P6b 唯一那条「不崩也不会红」的缺陷：神走了而他给的几条还挂在登记处，
    // 于是「大穷神七天」变成「终身双倍租金」。判据用**报价**而不是「再让他走一遍」：
    // 撤销这件事的正确读数是链还在不在，而走路那一趟还要管骰点、别的放置物，
    // 以及路上随机刷出来的第二位神 —— 那些都不相干。
    //
    // 两个人都按住（`StaySentence`）也是同一个理由：这一条要跑到他的下一个回合开始，
    // 按住之后场上唯一会动的只有神自己（它挪格与补刷都不碰人）。
    let (mut app, p0) = ready_on(0, 2, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    pave(&mut app, 1..=12);
    clear_gods(&mut app);
    for player in [p0, p1] {
        insert(&mut app, player, StaySentence { turns_left: 40 });
    }
    attach_god(&mut app, p0, God::BigPoor, Some(1));
    assert_eq!(corrections_len(&app), 1, "大穷神给一条：过路费加倍");
    assert_rent_quote(&mut app, PlayerId(0), 10_000);

    // 跑到他的下一个回合开始：那一次 `begin_turn_system` 把他身上那位扣到期满并离场
    play_turns(&mut app, 3);

    assert!(
        ledger(&app)
            .iter()
            .any(|line| line == "GodOff(P0:大穷神,期满)"),
        "前置：那位神要真的到期离场，否则后面的断言是空的：{:?}",
        ledger(&app)
    );
    assert_eq!(possession_of(&app, p0), None, "组件摘了");
    assert_eq!(corrections_len(&app), 0, "他给的那几条要跟着一起走");
    assert_rent_quote(&mut app, PlayerId(0), 5_000);
}

#[test]
fn a_replaced_god_leaves_no_rows_behind() {
    // 上面那条测「期满」这一支；这一条测「顶走」那一支：同一条 detach 路径，
    // 而它是 P6b 里唯一会在**同一帧**里先撤销再登记的调用点 —— 顺序写反看不见差别，
    // 写漏就留下两条链，所以条数与金额各断一次
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    clear_gods(&mut app);
    own(&mut app, RENT_LAND, Some(1), 1, false);
    flat_rent(&mut app, RENT_LAND, 5_000);
    attach_god(&mut app, p0, God::BigPoor, None);
    spawn_god(&mut app, God::BigFortune, RENT_TILE);

    play_turns(&mut app, 1);

    assert_eq!(
        possession_of(&app, p0).map(|held| held.god),
        Some(God::BigFortune),
        "前置：新神要真的顶上来"
    );
    assert_eq!(
        corrections_len(&app),
        1,
        "登记处只该剩大福神那一条：双倍过路费那条必须跟着大穷神一起走"
    );
    assert_rent_quote(&mut app, PlayerId(0), 5_000);
    assert_quote(
        &mut app,
        ChargeKind::BuyLand {
            land: LandId(RENT_LAND),
        },
        PlayerId(0),
        1_500,
        0,
    );
}

#[test]
fn two_players_with_opposite_gods_do_not_read_each_other_s_chain() {
    // 「一人一神」之后是「一神一链」：P0 身上的大财神免的是**他自己**要付的租，
    // 而 P1 身上的大穷神只抬 P1 自己应付的那笔。`Corrections.bearer` 那半条判据
    // 在两个人都真挂着神的世界上走一遍（单测里那两个 `player(n)` 太像真的了）
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let p1 = other_of(&mut app, p0);
    clear_gods(&mut app);
    own(&mut app, RENT_LAND, Some(1), 1, false);
    flat_rent(&mut app, RENT_LAND, 5_000);
    attach_god(&mut app, p1, God::BigPoor, Some(40));
    spawn_god(&mut app, God::BigRich, RENT_TILE);

    play_turns(&mut app, 1);

    assert_eq!(
        possession_of(&app, p0).map(|held| held.god),
        Some(God::BigRich),
        "前置：他要真的踩到大财神"
    );
    assert_eq!(
        cash_of(&mut app, p1),
        20_000,
        "业主自己挂着大穷神，可他不是这笔账的付款人 —— 自己的加倍不该抬自己的收租"
    );
    let gift = cash_of(&mut app, p0) - 20_000;
    assert!(
        (1_000..=9_999).contains(&gift),
        "大财神给 P0 的那笔是四位数，而免掉的那笔租金不该混进它：{gift}"
    );
    assert_eq!(corrections_len(&app), 2, "两条链各归各的主语");
    assert!(
        possession_of(&app, p1).is_some(),
        "P1 身上那位还在（40 天）"
    );
    // P0 的租金整笔免掉，而 P1 的那条链只在他自己付钱时才抬价 —— 报价各问一遍
    assert_rent_quote(&mut app, PlayerId(0), 0);
    assert_rent_quote(&mut app, PlayerId(1), 10_000);
}

// ===== P6b②：不是钱的那几处 =====

/// 装配到「P0 的回合、掷骰之前」，回来两名玩家（路上随机刷的那两位已清掉）。
/// 出牌类前置与 `cards.rs` 同一条：打出去的卡走真入口（请求 → `play_card_system`
/// → `apply_card`），不是直接改组件。
fn two_players() -> (App, Entity, Entity) {
    let mut app = boot_default();
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = other_of(&mut app, p0);
    clear_gods(&mut app);
    (app, p0, p1)
}

/// 三处「天数」各用一把尺，所以要挨个读：拘留数他的回合、停留数他按那颗按钮的回合、
/// 乌龟数他掷骰的回合。合成一个「处境」就读不出「三处都过了同一个倍数」这件事。
#[derive(Clone, Copy)]
enum Doom {
    Jail,
    Stay,
    Cap,
}

/// 让 p1 挂上某位神（`None` = 谁都不挂），把 `card` 由 p0 打在他头上，读回那格计时。
/// 三处共用这一句，所以「哪几处被翻倍」只由 `doom_multiplier` 那张表决定。
fn days_played(card: &str, god: Option<God>, which: Doom) -> u32 {
    let (mut app, p0, p1) = two_players();
    if let Some(god) = god {
        attach_god(&mut app, p1, god, Some(40));
    }
    let card = item_index(&app, card);
    deal(&mut app, p0, card.0);
    play_card(&mut app, p0, card.0, Some(CardAim::Player(p1)));
    settle(&mut app, 3);
    let left = match which {
        Doom::Jail => detain_of(&app, p1).map(|held| held.turns_left),
        Doom::Stay => stay_of(&app, p1),
        Doom::Cap => move_cap_of(&app, p1),
    };
    left.expect("那张卡该在 p1 身上留下一格计时，否则这条用例什么都没读")
}

#[test]
fn every_doom_day_a_god_multiplies_covers_all_three_places() {
    // PLAN §三「衰神＝…厄运天数加倍」。只在 `detain` 那一条分支上乘一次的话，
    // 停留与乌龟会静默地不加倍 —— 所以三处各跑三遍（没神／大衰神／小衰神）
    for (card, which, plain) in [
        ("item_frame", Doom::Jail, 5u32),
        ("item_stay", Doom::Stay, 1),
        ("item_turtle", Doom::Cap, 3),
    ] {
        assert_eq!(
            days_played(card, None, which),
            plain,
            "{card} 没有神时该是卡面那个数（乘数的基准）"
        );
        assert_eq!(
            days_played(card, Some(God::BigDoom), which),
            plain * 2,
            "{card} 在大衰神在场时要翻倍"
        );
        assert_eq!(
            days_played(card, Some(God::SmallDoom), which),
            plain,
            "{card}：小衰神只动钱，不动天数（与 P6b① 那条「抢钱是大穷神独有」同一个分法）"
        );
    }
}

#[test]
fn the_fortune_god_shrugs_off_every_affliction_while_his_rival_does_not() {
    // 大福神「厄运豁免」＝期间每一次都不生效。判据现读 `Possession`，
    // 不往 `Insulation` 里塞一格（理由写在那个组件的注里）
    let (mut app, p0, p1) = two_players();
    let p1_tile = tile_of(&app, p1);
    attach_god(&mut app, p1, God::BigFortune, Some(40));
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 3);

    assert_eq!(
        detain_of(&app, p1),
        None,
        "大福神在场，陷害这一件根本没发生"
    );
    assert_eq!(
        tile_of(&app, p1),
        p1_tile,
        "被抵消的那次不该把他搬到任何接入点 —— 与 cards.rs 那条同一条口径"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("Play(P0->P1:item_frame") && line.contains('挡')),
        "挡下来了也要留一句为什么，否则流水只说得出「什么都没发生」：{:?}",
        ledger(&app)
    );

    // 反例的反例：小福神不免厄运。少了这一格，上面那条会把「整张陷害卡坏了」也测成绿
    let (mut app, p0, p1) = two_players();
    attach_god(&mut app, p1, God::SmallFortune, Some(40));
    let frame = item_index(&app, "item_frame");
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 3);
    assert_eq!(
        detain_of(&app, p1).map(|held| held.turns_left),
        Some(5),
        "小福神该照常被关进去（PLAN §三 只给大的那一半豁免）"
    );
}

#[test]
fn immunity_spends_no_card_layers() {
    // 两个来源、一个出口，而**顺序是有内容的**：反过来写，大福神在场时每一件事都会
    // 白白吃掉一层免罪卡，玩家那两层会在他还「免疫」的时候悄悄清零 —— 而那件事只有到
    // 他不再免疫的下一件上才看得见，也就是看不见。
    let (mut app, p0, p1) = two_players();
    let frame = item_index(&app, "item_frame");
    let insulate = item_index(&app, "item_insulate");
    // 两张免罪卡**由他自己打**（免罪卡没有目标那一格，卡面写的是「落到自己头上的一次」），
    // 而出牌的资格是「当前回合玩家」—— 所以这里把轮次交给他，别用夹具绕过那条判据
    make_current(&mut app, p1);
    for _ in 0..2 {
        deal(&mut app, p1, insulate.0);
        play_card(&mut app, p1, insulate.0, None);
        settle(&mut app, 2);
    }
    assert_eq!(shield_of(&app, p1), Some(2), "前置：两张卡该叠成两层");
    make_current(&mut app, p0);

    // 1) 没有神：陷害吃掉一层
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(detain_of(&app, p1), None, "第一件被卡挡住");
    assert_eq!(shield_of(&app, p1), Some(1), "卡挡的事要消耗卡");

    // 2) 大福神在场：陷害整件不生效，而**那一层卡分文不动**
    attach_god(&mut app, p1, God::BigFortune, Some(40));
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(detain_of(&app, p1), None, "那位神在场，这一件同样没发生");
    assert_eq!(
        shield_of(&app, p1),
        Some(1),
        "免疫期间不许吃卡 —— 两个来源的顺序写反了红在这里"
    );

    // 3) 神走了（这里直接摘，等价于期满/被顶走）：卡又变成唯一那道防线
    clear_gods(&mut app);
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(detain_of(&app, p1), None, "最后一层还挡得住一次");
    assert_eq!(shield_of(&app, p1), None, "用完那一层之后组件要被摘掉");

    // 4) 两层都用掉了：第三件照关，这条用例不能永远绿下去
    deal(&mut app, p0, frame.0);
    play_card(&mut app, p0, frame.0, Some(CardAim::Player(p1)));
    settle(&mut app, 2);
    assert_eq!(
        detain_of(&app, p1).map(|held| held.turns_left),
        Some(5),
        "没有卡也没有神的时候，陷害该照常生效"
    );
}

// ===== P6b②：买地／建屋必败 与 加盖多一层 =====

/// L0 那一组（brown）的第二块。注意它与 `RENT_TILE` 都是数字 1，但一个数是**地块**、
/// 另一个是**格子** —— prototype 里 L0 挂在 1 号格上，纯属撞车。
const SECOND_LAND: u32 = 1;

/// 造一条「P0 走 1 步踩到 1 号格（那块 L0 **无主**，脚下站着 `god`）」的对局，
/// 回来 `(app, L0 业主, P0 现金)`。
///
/// 策略是 `AcceptAll`：报价一发出来就接下。于是「地没到手」只剩一种解释 ——
/// 报价根本没发（`acquire_denied` 那条闸门），而不是「他自己点了拒绝」。
fn acquire_with(god: Option<God>) -> (App, Option<u32>, i64) {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::AcceptAll);
    clear_gods(&mut app);
    if let Some(god) = god {
        spawn_god(&mut app, god, RENT_TILE);
    }

    play_turns(&mut app, 1);

    assert_eq!(tile_of(&app, p0), RENT_TILE, "前置：他要真的踩到那一格");
    if let Some(god) = god {
        assert_eq!(
            possession_of(&app, p0).map(|held| held.god),
            Some(god),
            "前置：{} 要真的上了身，否则这句「买不成」测的是别的东西",
            god.label()
        );
    }
    let owner = plot_state(&mut app, RENT_LAND).0;
    let cash = cash_of(&mut app, p0);
    (app, owner, cash)
}

#[test]
fn the_doom_gods_and_death_refuse_the_purchase_entirely() {
    // PLAN §三「衰神＝…买地建屋必败」+「死神」同一条。这条**不能**做成 `Corrections`
    // 里的一条 `Cancel`：0 元成交仍然是成交，地会归他，而「必败」要的是这块地不属于任何人。
    // 所以判据在 `Books::acquire_denied`，报价与成交两处共读它（同一个理由见 P3d④a）
    let (app, owner, cash) = acquire_with(None);
    assert_eq!(
        (owner, cash),
        (Some(0), 18_500),
        "对照物：没有神时这一格买得下来（L0 地价 1500）"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("BuyOffer(P0,@0)")),
        "前置：报价真的发出去过 —— 下面那句「没有报价」才不是空跑：{:?}",
        ledger(&app)
    );
    drop(app);

    // 死神上不了街（`roams` 那条），但夹具能把他按在人身上 —— 这一格要靠它才测得到
    for god in [God::BigDoom, God::SmallDoom, God::Death] {
        let (app, owner, cash) = acquire_with(Some(god));
        assert_eq!(owner, None, "{} 在场：这块地不该归任何人", god.label());
        assert_eq!(
            cash,
            20_000,
            "{} 在场：必败＝这件事根本没做成，不是「付了钱再退回来」",
            god.label()
        );
        assert!(
            !ledger(&app)
                .iter()
                .any(|line| line.starts_with("BuyOffer(P0,@0)")),
            "连报价都不该弹出来（弹了就是一条玩家答得成的窗口）：{:?}",
            ledger(&app)
        );
    }
}

#[test]
fn the_grab_card_buys_nothing_while_a_doom_god_watches() {
    // 同一道闸门的第二个入口：购地卡不去那一格也能买，所以它必须**也**问一句。
    // 判据借 `Books::buy_price`（出牌那道 `aim_is_legal` 读的就是它），所以这里买不成的
    // 同时，面板上那颗按钮同样是灰的 —— 两个入口共用一份真相
    for god in [None, Some(God::BigDoom)] {
        let (mut app, p0, _p1) = two_players();
        if let Some(god) = god {
            attach_god(&mut app, p0, god, Some(40));
        }
        let grab = item_index(&app, "item_grab");
        deal(&mut app, p0, grab.0);
        assert_eq!(hand_of(&app, p0).len(), 1, "前置：手里只有这一张购地卡");

        play_card(&mut app, p0, grab.0, Some(CardAim::Land(LandId(RENT_LAND))));
        settle(&mut app, 3);

        let kept = hand_of(&app, p0).contains(&grab);
        let owner = plot_state(&mut app, RENT_LAND).0;
        match god {
            None => assert_eq!(
                (kept, owner),
                (false, Some(0)),
                "没有神：卡花掉、地到手（这一格是下面那条的对照物）"
            ),
            Some(god) => assert_eq!(
                (kept, owner),
                (true, None),
                "{} 在场：卡该留在手里、地不该归他 —— 红在「卡没了」那一半就是白扣一张",
                god.label()
            ),
        }
    }
}

/// 手握整组（L0 + L1，brown，都是 Lv1），付**一次**加盖费，回来 `(L0 等级, P0 现金)`。
///
/// 走 `Books::upgrade` 而不是真踩上去：这一格问的是「等级跳了几级、钱扣了几次」，
/// 而真踩还得先凑出「业主本人踩到自己那块地」的骰点，一路随机刷出来的神还会改这个数
fn build_with(god: Option<God>) -> (u8, i64) {
    let (mut app, p0, _p1) = two_players();
    own(&mut app, RENT_LAND, Some(0), 1, false);
    own(&mut app, SECOND_LAND, Some(0), 1, false);
    if let Some(god) = god {
        attach_god(&mut app, p0, god, Some(40));
    }
    // 指数直接摆平：`own()` 是凭空塞地，跑过回合末它就会不是 1.0（P4② 记过同一条），
    // 而这里要的读数是「一次加盖费到底扣了几回」
    set_price_index(&mut app, 1.0);
    run_op(&mut app, move |books, _| {
        let got = books.upgrade(p0, LandId(RENT_LAND));
        assert!(got.is_some(), "整组在手、又是组内最低层，这一格加盖该办成");
    });
    (plot_state(&mut app, RENT_LAND).1, cash_of(&mut app, p0))
}

#[test]
fn the_fortune_god_raises_two_levels_for_the_price_of_one() {
    // PLAN §三「大福神＝…加盖多盖一层」。多出来的那一层**不多收一次钱** ——
    // 卡面那句是「多盖一层」不是「买两层」。对照组先钉死「没有神时只跳一级、收一次费」
    assert_eq!(
        build_with(None),
        (2, 19_200),
        "没有大福神：一级 + 一次加盖费（L0 的 `upgrade_cost` 是 800）"
    );
    // 大福神那一次跳到 3 级，而现金分文不动：`Acquire` 域整条抵消，
    // 加盖费与地价同在域里，所以连它一起免 —— 这条已知口径写在 P6b① 的记录 ③
    assert_eq!(
        build_with(Some(God::BigFortune)),
        (3, 20_000),
        "大福神：跳两级，钱只该有一条抵消（0 元），不该收到第二次"
    );
    // 反例的反例：小福神只有半价，不多盖。少了这一格，上面那条会把
    // 「所有福神都多盖一层」也测成绿
    assert_eq!(
        build_with(Some(God::SmallFortune)),
        (2, 19_600),
        "小福神：还是两级，而那一笔收半价（800/2）"
    );
}

#[test]
fn the_extra_level_stops_at_the_roof() {
    // 封顶那一位上多出来的那一层少掉，不是缺陷（4 代也没有「盖穿顶」）
    let (mut app, p0, _p1) = two_players();
    let max = app.world().resource::<GameRules>().max_building_level;
    own(&mut app, RENT_LAND, Some(0), max - 1, false);
    own(&mut app, SECOND_LAND, Some(0), max - 1, false);
    attach_god(&mut app, p0, God::BigFortune, Some(40));
    run_op(&mut app, move |books, _| {
        assert_eq!(
            books.upgrade(p0, LandId(RENT_LAND)),
            Some(max),
            "只该到顶，不该越过顶"
        );
    });
    assert_eq!(plot_state(&mut app, RENT_LAND).1, max);
    run_op(&mut app, move |books, _| {
        assert_eq!(
            books.upgrade(p0, LandId(RENT_LAND)),
            None,
            "到了顶之后那位神在场也不能再走一步 —— 少一层不等于多一格"
        );
    });
}

#[test]
fn the_imbalance_a_god_creates_can_be_evened_up_again() {
    // 这条是 `economy::is_group_floor` 存在的全部理由。旧判据「组内等级差 ≤1」只在**事前**查，
    // 于是 (3,1) 这个形状里两块地都动不了：低的差 2 不合法、高的再升就差 3 —— 整组永久冻死，
    // 而大福神一次加两层**必然**造出这个形状。取最低层那一条把方向钉死：低的能追，高的等着
    let (mut app, p0, _p1) = two_players();
    own(&mut app, RENT_LAND, Some(0), 1, false);
    own(&mut app, SECOND_LAND, Some(0), 1, false);
    attach_god(&mut app, p0, God::BigFortune, Some(40));
    run_op(&mut app, move |books, _| {
        let me = books.holder(PlayerId(0)).unwrap();
        assert_eq!(
            books.upgrade(me, LandId(RENT_LAND)),
            Some(3),
            "前置：先造出落差 2"
        );
    });
    assert_eq!(plot_state(&mut app, SECOND_LAND).1, 1, "另一块还低着");
    // 神先请走：追平那一步要是一次**普通**加盖（+1），不然读到的还是「多盖一层」，
    // 这条用例想钉的那道闸门就被另一件事遮住了
    clear_gods(&mut app);
    run_op(&mut app, move |books, _| {
        assert_eq!(
            books.upgrade(p0, LandId(SECOND_LAND)),
            Some(2),
            "低的那一块要追得回来 —— 旧判据（组内等级差 ≤1）在这里返回 None，整组从此卡死"
        );
        assert_eq!(
            books.upgrade(p0, LandId(RENT_LAND)),
            None,
            "而高的那一块在追平之前不许再跑：拉开落差的那一步必须走不通"
        );
    });
    assert_eq!(
        (
            plot_state(&mut app, RENT_LAND).1,
            plot_state(&mut app, SECOND_LAND).1
        ),
        (3, 2),
        "两次加盖的落点要正好是这两级"
    );
}

// ===== P6b②：衰神每回合吃掉几张卡 =====

/// 让 `god` 附身在 p0 身上 `days` 天，两人都按住（不让走路卷进别的发牌与落格），
/// 发 8 张卡，跑到第 `turns` 个**全局**回合，回来「他被吃掉的张数」。
///
/// 按全局回合数是因为天数数的是**他自己的**回合：两人轮转，所以 `turns` 要给到
/// `days` 的两倍以上，才真跑过「到期那一回合」这一格。
fn cards_eaten(god: God, days: u32, turns: u32) -> usize {
    let (mut app, p0, p1) = two_players();
    for player in [p0, p1] {
        insert(&mut app, player, StaySentence { turns_left: 120 });
    }
    let frame = item_index(&app, "item_frame");
    for _ in 0..8 {
        deal(&mut app, p0, frame.0);
    }
    attach_god(&mut app, p0, god, Some(days));
    let start = hand_of(&app, p0).len();
    // 只数**这一趟跑出来的**那些流水：`deal` 那 8 张自己就写了 8 条 `Gain`，
    // 拿整张表去算「进账 − 丢卡」会把手牌数算成两倍（第一次跑就红在这里）
    let before = ledger(&app).len();

    play_turns(&mut app, turns);

    let all = ledger(&app);
    let lines = &all[before..];
    let lost = lines
        .iter()
        .filter(|line| line.starts_with("Lose(P0:") && line.contains(god.label()))
        .count();
    // 流水与手牌必须对上：每一条 `Lose` 真的少了一张卡，而少的每一张都留了流水。
    // 进账那几条（圣诞礼物之类，日期由锚点决定）也算进来，这条等式才对任何日历成立
    let gained = lines
        .iter()
        .filter(|line| line.starts_with("Gain(P0:"))
        .count();
    assert_eq!(
        hand_of(&app, p0).len(),
        start + gained - lost,
        "手牌与丢卡流水对不上（{lines:?}）"
    );
    lost
}

#[test]
fn the_doom_god_eats_two_cards_a_turn_while_the_small_one_takes_one() {
    // PLAN §三「衰神＝丢卡」。每回合吃的张数只在 `cards_lost_per_turn` 那张表上写一次
    // （大衰神 2、小衰神 1、其余 0），而这里三行各跑一遍
    assert_eq!(
        cards_eaten(God::BigDoom, 3, 10),
        4,
        "大衰神 2 张/回合，而他身上一共待成两回合（第三回合是期满那一回合，见下面那条）"
    );
    assert_eq!(
        cards_eaten(God::SmallDoom, 3, 10),
        2,
        "小衰神 1 张/回合 —— 与大的那个差一整倍，写反了就红"
    );
    assert_eq!(
        cards_eaten(God::BigRich, 3, 10),
        0,
        "财神不吃卡：这张表只有衰神那一族有值（否则「谁身上都掉卡」的世界也能全绿）"
    );
}

#[test]
fn the_turn_a_god_expires_he_takes_no_more_cards() {
    // `age_possession` 先把那位神扣到期满、再回答「还剩谁」，就是为这一格写的：
    // 组件走的是 `Commands`（延迟），而丢卡在同一帧就要读「他还有没有神」。
    // 拿 `Commands` 里那份过期的读数丢卡，这条会数出 2 而不是 0
    assert_eq!(
        cards_eaten(God::BigDoom, 1, 10),
        0,
        "只剩最后一天：那一回合他离场，卡一张不该少"
    );
    assert_eq!(
        cards_eaten(God::BigDoom, 2, 10),
        2,
        "两天：第一回合格照吃 2 张，第二回合格期满不吃 —— 这条把上面那条的下限也钉住了"
    );
}
