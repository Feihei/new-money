//! P13 魔法屋的验收：11 种筛选条件 × 12 种法术，外加 PLAN §四 点名那张**免疫矩阵**。
//!
//! 分四段，与计划里那一行的三半对齐：
//!
//! - **窗口**（第一段）验的是「两步」这件事本身：主语不对不算回答、空名单当场收摊、
//!   草稿与窗口同生同灭、施法之后回合交得回去。前三条各对应一种「不报错、只是不动了」
//!   的挂死，第四条（交回合）是 P13 写完真跑出来的那一个；
//! - **条件**验的是三个口径：「最多」并列取回合序第一个、0 也是合法答案、
//!   破产者与恶人**不在名单里**（而「名单里只有玩家」由等号钉住，不是靠查不到）；
//! - **法术**12 条各有**正面**断言（钱、等级、手牌、拍卖台真的动了），因为「法术落空」与
//!   「法术没实现」在流水上都长得像「什么都没发生」—— 那是本项目犯过五次的那类空跑；
//! - **矩阵**（最后一段）就是验收原话那一格。
//!
//! ## 免疫口径（10-25 拍板，覆盖 PLAN §三 那行原来的记法）
//!
//! **免罪卡只挡得住「把人关起来」那一族**：陷害、命运的坐牢／住院／出国，以及女巫的
//! 坐牢／住院两道。停留、乌龟、转向、拆除一类**不吃它** —— 否则一张免罪卡会被「向后转」
//! 白白耗掉，而玩家根本没有选择权（免疫规则该免的是事，不是卡）。神明那一半（大福神、
//! 天使的厄运豁免）照旧是 blanket，但**管不到女巫** —— 祂豁免的是厄运，女巫的法术不是厄运。
//! 于是矩阵在这里是三列：
//!
//! | 目标身上有 | 坐牢／住院 | 其余十道 |
//! |---|---|---|
//! | 免罪卡 | 挡下，吃掉一层 | 照常生效，一层不花 |
//! | 大福神 | 照常生效（豁免不到女巫） | 照常生效 |
//! | 大衰神 | 照常生效，**照 3 天**（`doom_days` 那把尺量不到女巫） | 照常生效 |
//!
//! 第四列「嫁祸卡」今天没有这张卡，也没有 4 代「移祸给谁」的记载 —— 已记进 PLAN §七 待拍板，
//! 不在这里替它编一个载体。
//!
//! 「抽命运 3 张」按计划算**间接效果**：三张牌走 [`game_core::deck::apply_card`] 那条正路，
//! 所以神明的修正吃得到 —— [`the_fate_spell_draws_three_and_the_god_taxes_them`] 用 A/B 两条
//! 把这句话验成数字，而不是验成一句注释。
//!
//! 夹具全部**现读格号**（[`common::kind_tile`]）：魔法屋插在主环 9 号，把 9 之后每一格都
//! 顺移了一位，而这批用例不该因为下次再插一格而集体变红。判据多用 [`common::hits`]
//! （规则层那同一个 `filter_targets`），因为一场法事只答得出一个条件，而「并排看五条判据」
//! 这类断言不该为此重启五局 —— 真窗口那条路由 `pick` 那几条覆盖。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::Assets;
use common::*;
use game_core::prelude::{
    BankBalance, Bankrupt, CardId, CardSet, DataHandles, Detention, DetentionPlace, GameRules,
    GameSeed, GameSetup, Gender, God, Heading, Insulation, ItemKind, LandId, MagicFilter,
    MagicRitual, MagicSpell, Mount, Pending, PlayerName, Rng, Term, TileKind, TurnPhase, Villain,
};

/// 0 号地（南京路）—— 三条「作用于脚下地」的法术共用它当道具。
const PLOT: u32 = 0;

/// 另一名玩家（默认阵容：P0 阿土伯＝男、P1 孙小美＝女）。
///
/// 法术与矩阵都以她为承受者：`AllGirls` 是默认盘上**唯一一条稳定只筛出一个人**的条件
/// （`Walker` 筛的是施法者自己，验「落到别人头上」就用不了它）。
fn other(app: &mut App, p0: Entity) -> Entity {
    *roster(app)
        .iter()
        .find(|(e, ..)| *e != p0)
        .map(|(e, ..)| e)
        .expect("默认阵容有两名玩家")
}

/// 装配到「P0 刚踩上魔法屋、第一步窗口挂着没人答」。
///
/// 与彩票那批同一手法：不替他 `offer` 窗口 —— 先按种子读出第一掷，再把他摆到「走那么多步
/// 正好落到魔法屋」那一格，于是踩到与开窗口走的都是真走路、真落格分派。
/// 开局那两位神与路上的宝箱在这里清掉（`clear_gods` 把 `Occupancy` 整族 despawn），否则
/// 「走过去踩到女巫家」这一路上撞见财神会当场搬一笔钱，下面那些现金断言就不再只等于法术。
fn at_the_witch() -> (App, Entity, Entity) {
    let seed = GameSeed::default();
    let mut app = boot(GameSetup::default(), GameRules::instant(), seed);
    app.insert_resource(DecisionPolicy::Hold);
    let p0 = boot_to_wait_roll(&mut app);
    let p1 = other(&mut app, p0);
    // 这份夹具的性别前提由夹具自己写死：**P0 男、P1 女**。下面三十来条用例里
    // `AllGirls` 是「选到 p1」的手段而不是 subject，它不该跟着演示盘的默认阵容走 ——
    // 阵容换人（10-26 从阿土伯／孙小美换成姬百年／范一文）当时就把这一批全部翻面，
    // 症状是一堆 `[p0] != [p1]`，而魔法屋的规则一个字没动
    insert(&mut app, p0, (Gender::Male,));
    insert(&mut app, p1, (Gender::Female,));
    clear_gods(&mut app);
    let face = Rng::new(seed.0).roll_die();
    let at = kind_tile(&app, TileKind::Magic);
    stand_to(&mut app, p0, at, face);

    roll(&mut app, p0);
    settle(&mut app, 6);

    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Magic)),
        "前置该把 P0 送到女巫家门口：骰面 {face}，该落在 {at} 号，流水 {:?}",
        events(&app)
    );
    (app, p0, p1)
}

// ===== 读数 =====

/// 草稿里那份名单 —— 第二步窗口显示的与法术真正落到的共用它，所以断言读它就等于读面板。
fn draft(app: &App) -> Option<(MagicFilter, Vec<Entity>)> {
    app.world().resource::<MagicRitual>().draft.clone()
}

/// 点第一步那颗条件，返回筛出的人（空名单时窗口当场收摊，这里回 `vec![]`）。
fn pick(app: &mut App, caster: Entity, filter: MagicFilter) -> Vec<Entity> {
    magic_filter_ask(app, caster, filter);
    settle(app, 2);
    draft(app).map(|(_, hits)| hits).unwrap_or_default()
}

/// 点第二步那颗法术，并跑两帧让规则层施法。
fn cast(app: &mut App, caster: Entity, spell: MagicSpell) {
    magic_spell_ask(app, caster, spell);
    settle(app, 2);
}

/// 流水里那两条魔法屋的记录（`Magic(…)` 与 `MagicBlock(…)`）。
fn magic_lines(app: &App) -> Vec<String> {
    events(app)
        .into_iter()
        .filter(|line| line.starts_with("Magic"))
        .collect()
}

/// 「P0 施了这一道，落到 `hit` 头上」那一行。`hit` 用流水里的名字，落空填 `""`。
/// 施法者恒为 P0 —— 这批用例的主语都是他，写成参数只会给每处断言加一个噪音。
fn cast_line(spell: MagicSpell, hit: &str) -> String {
    format!("Magic(P0:{}→{hit})", spell.label())
}

fn was_cast(app: &App, spell: MagicSpell, hit: &str) -> bool {
    magic_lines(app).contains(&cast_line(spell, hit))
}

fn was_blocked(app: &App, spell: MagicSpell, victim: &str) -> bool {
    magic_lines(app)
        .iter()
        .any(|l| *l == format!("MagicBlock({victim}挡P0的{})", spell.label()))
}

fn heading_of(app: &App, player: Entity) -> Heading {
    *app.world()
        .entity(player)
        .get::<Heading>()
        .expect("开局每个玩家都挂着方向（P3d④b）")
}

/// 场上有几位恶人（他们今天全在笼里，但**实体已经在世界上**了）。
fn villain_entities(app: &mut App) -> Vec<Entity> {
    let w = app.world_mut();
    let mut q = w.query::<(Entity, &Villain)>();
    q.iter(w).map(|(e, _)| e).collect()
}

/// 道具池有几张 —— 即「得一张卡」那颗骰子的面数。从**世界**里读，不在测试里重数一遍 JSON。
fn item_pool_len(app: &App) -> u32 {
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().items;
    sets.get(handle).expect("道具牌没加载").0.len() as u32
}

/// 直接摆一笔负债（夹具走 `BankBalance` 而不是真借款：这里要的是「他欠多少」这个读数，
/// 而真借款会连现金一起加进来，把财富对比搅成另一件事）。
fn set_loan(app: &mut App, player: Entity, loan: i64) {
    app.world_mut()
        .entity_mut(player)
        .get_mut::<BankBalance>()
        .expect("玩家应有银行账户")
        .loan = loan;
}

// ===== 一、两步窗口 =====

/// 踩到魔法屋开的是**第一步**（选条件），而第一步还没有名单 —— 名单是条件的产出。
#[test]
fn stepping_on_the_witch_house_offers_the_condition_step_only() {
    let (app, p0, _p1) = at_the_witch();
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "窗口挂着时回合不该往前走"
    );
    assert_eq!(pending_of(&app), Some((p0, Pending::Magic)));
    assert!(
        draft(&app).is_none(),
        "条件还没答，草稿就该是空的 —— 否则第二步的名单是猜出来的"
    );
}

/// 主语门禁：窗口挂在 P0 身上，**别人**答的条件不算回答。
///
/// 与保释／柜台那几条同一条形状。少了这一格，「两个人同时对着一场法事」就能把名单换成
/// 自己那一份，而窗口的主人毫不知情。
#[test]
fn a_condition_from_the_wrong_seat_is_not_an_answer() {
    let (mut app, p0, p1) = at_the_witch();
    magic_filter_ask(&mut app, p1, MagicFilter::Walker);
    settle(&mut app, 2);

    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::Magic)),
        "别人答的条件不该把窗口推进一步"
    );
    assert!(draft(&app).is_none(), "更不该留下一份名单");
}

/// 条件答完 → 开第二步，而**第二步显示的名单与法术落到的人必须是同一批**
/// （`filter_targets` 只有一个口径，两条 system 都调它，而不是一个筛一个猜）。
#[test]
fn picking_a_condition_opens_the_spell_step_with_that_same_list() {
    let (mut app, p0, p1) = at_the_witch();
    let hits = pick(&mut app, p0, MagicFilter::AllGirls);

    assert_eq!(hits, vec![p1], "条件筛出的名单不对");
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::MagicSpell(MagicFilter::AllGirls))),
        "第二步窗口该带上**哪一个条件**，面板才念得出这场法事在筛谁"
    );
}

/// 第二步按「离开」：窗口与草稿**同生同灭**。
///
/// 草稿是资源而不是 `Pending` 的字段（`Pending` 是 `Copy` 枚举，装不下 `Vec`），所以没有
/// 任何东西替它收账 —— 不跟着清就留下一份过期名单，dump 与面板都读得到它，而回主菜单
/// 那一帧也没有东西清（坑 22/25 同族）。
#[test]
fn leaving_the_spell_step_clears_the_draft_with_the_window() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);

    set_policy(&mut app, DecisionPolicy::DeclineAll);
    settle(&mut app, 2);

    assert_eq!(pending_of(&app), None, "离开之后不该还挂着窗口");
    assert!(draft(&app).is_none(), "窗口收掉了，草稿必须跟着走");
    assert_ne!(phase(&app), TurnPhase::WaitDecision, "回合该交还给状态机");
}

/// 没人命中的条件**当场收摊**，而不是把玩家摆到一排「点不着任何人的法术」前面。
///
/// 收摊也算「把回合交出去」：`decision_system` 只在有人答了一句时才动回合机，而这一步
/// 没有回答（面板上那颗按钮本来就是灰的，规则层却必须假设有人能把它送进来）。
/// 只 `pending.clear()` 的写法会把回合永久停在 `WaitDecision`，且不报错、只是不动了。
#[test]
fn a_condition_that_hits_nobody_ends_the_ritual_and_hands_the_turn_back() {
    let (mut app, p0, _p1) = at_the_witch();
    // 默认阵容两位都步行，所以「开机车的」是空的
    assert_eq!(
        pick(&mut app, p0, MagicFilter::OnBike),
        vec![],
        "前置：没人骑车，这一条该筛空"
    );

    assert_eq!(pending_of(&app), None, "空名单不该开第二步");
    assert!(draft(&app).is_none());
    assert_ne!(
        phase(&app),
        TurnPhase::WaitDecision,
        "落空也得把回合交还 —— 否则整局停在这一格"
    );
    assert!(
        magic_lines(&app).is_empty(),
        "落空不是一场施法，不该有 Magic 流水"
    );
}

/// 施法即收尾：一颗法术按钮既办完事又交还回合（与建设柜台那种「办完还要再按一次先不建」不同）。
#[test]
fn the_spell_step_hands_the_turn_back_after_casting() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::Stay);

    assert!(was_cast(&app, MagicSpell::Stay, "P1"), "法术该落到她头上");
    assert_eq!(pending_of(&app), None, "施完法窗口就收");
    assert!(draft(&app).is_none(), "草稿一次就消耗掉");
    assert_ne!(
        phase(&app),
        TurnPhase::WaitDecision,
        "交还回合由这一步做 —— 只 clear 窗口的写法会整局挂死"
    );
}

/// 两张穷尽表的形状：11 条条件、12 道法术，名字与说明都上得了屏。
///
/// 「玩家不能指定具体人」这条玩法（PLAN §三）由条件的**条数与形状**表达：表里若长出
/// 「指定某人」那一格，这一条就红在条数上，而那一格的代价是「害人变成一次查名操作」。
#[test]
fn the_two_lists_are_eleven_conditions_and_twelve_spells() {
    assert_eq!(MagicFilter::ALL.len(), 11);
    assert_eq!(MagicSpell::ALL.len(), 12);

    let names: Vec<&str> = MagicFilter::ALL.iter().map(|f| f.label()).collect();
    assert!(names.iter().all(|n| !n.is_empty()));
    assert_eq!(
        names.iter().collect::<std::collections::HashSet<_>>().len(),
        11,
        "两条条件同名 —— 面板上就点不出是哪一条"
    );

    let spells: Vec<&str> = MagicSpell::ALL.iter().map(|s| s.label()).collect();
    assert_eq!(
        spells
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        12,
        "两道法术同名 —— 顶栏那句「落到谁头上」就分不出来"
    );
    for spell in MagicSpell::ALL {
        assert!(
            spell.desc().chars().count() > 6,
            "{} 没有一句说明，玩家点不下去那颗按钮",
            spell.label()
        );
    }
    // 十二道里**恰好两道**把人关起来 —— 那两道就是免疫矩阵里唯一会被免罪卡挡下的两列
    let locking = MagicSpell::ALL
        .iter()
        .filter(|s| matches!(s, MagicSpell::Jail | MagicSpell::Hospital))
        .count();
    assert_eq!(
        locking, 2,
        "「挡得住」的那几道改了却没改矩阵 —— 口径就分叉了"
    );
}

// ===== 二、11 种筛选条件 =====

/// 「走路的人」是**此刻该行动的那一位**，也就是踩到女巫家的他自己 —— 与他的钱包、性别无关。
#[test]
fn walker_is_the_one_standing_at_the_witch_house() {
    let (mut app, p0, p1) = at_the_witch();
    // 把她的账做到「随便哪条『最多』都会选中她」，Walker 仍然只答走路的那位
    set_cash(&mut app, p1, 90_000);
    set_points(&mut app, p1, 500);
    own(&mut app, PLOT, Some(1), 3, false);

    assert_eq!(
        hits(&mut app, MagicFilter::TopCash, p0),
        vec![p1],
        "前置：她此刻确实处处压过 P0，否则这条 Walker 什么都没区分"
    );
    assert_eq!(pick(&mut app, p0, MagicFilter::Walker), vec![p0]);
    assert_eq!(
        draft(&app).map(|(f, _)| f),
        Some(MagicFilter::Walker),
        "草稿得记住**是哪个条件**筛出的这份名单，第二步的面板才念得出来"
    );
}

/// 性别读的是 `Gender` 组件而不是 `PlayerName`。
///
/// 这一条不是抬杠：诨名是角色设定（CHARACTERS.md），而「所有女生」是规则层的一句话。
/// 把名字当判据的话，改一句文案就改了一次玩法。
#[test]
fn the_two_kinds_are_read_from_the_component_not_the_name() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(hits(&mut app, MagicFilter::AllBoys, p0), vec![p0]);
    assert_eq!(hits(&mut app, MagicFilter::AllGirls, p0), vec![p1]);

    // 把男的名号改成「孙小美」，再走真窗口问一遍
    insert(&mut app, p0, (PlayerName("孙小美".into()),));
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
}

/// 载具两条筛的是此刻骑的东西，而且**都不命中时筛空**（不是「筛出所有人」）。
#[test]
fn mounts_split_the_table_and_miss_when_nobody_rides() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(mount_of(&app, p0), Mount::Foot, "前置：开局步行");

    set_mount(&mut app, p1, Mount::Car);
    assert_eq!(hits(&mut app, MagicFilter::OnCar, p0), vec![p1]);
    assert_eq!(
        hits(&mut app, MagicFilter::OnBike, p0),
        vec![],
        "没人骑车时该筛空，而不是把两人都算进来"
    );
    assert_eq!(pick(&mut app, p0, MagicFilter::OnBike), vec![]);
    assert_eq!(pending_of(&app), None, "没人命中 → 不落第二步");

    let (mut app, p0, _p1) = at_the_witch();
    set_mount(&mut app, p0, Mount::Bike);
    assert_eq!(hits(&mut app, MagicFilter::OnBike, p0), vec![p0]);
}

/// 「神灵附身的」筛的是**身上**那位，不是路上站着的那位。
#[test]
fn possessed_lists_the_bodies_with_a_god_on_them() {
    let (mut app, p0, p1) = at_the_witch();
    assert!(
        possession_of(&app, p1).is_none(),
        "前置：清场之后没人被附身"
    );
    assert_eq!(
        hits(&mut app, MagicFilter::Possessed, p0),
        vec![],
        "没人附身 → 筛空（这一格与「筛出所有人」是最容易写反的一对）"
    );

    attach_god(&mut app, p1, God::BigFortune, None);
    assert_eq!(pick(&mut app, p0, MagicFilter::Possessed), vec![p1]);
}

/// 「最多」那一族的三个口径一次问完：并列取回合序第一个、0 也是合法答案、各条读各的账。
///
/// 第三点是这一族最容易糊的地方：现金、存款、点券、地块、总财富是**五本不同的账**，
/// 「现金最多」在通胀局里未必是「财产最多」的那位 —— 这正是 PLAN §三 那句「条件把害人
/// 变成一次博弈」的意思。
#[test]
fn the_top_family_reads_its_own_ledger_and_ties_to_turn_order() {
    let (mut app, p0, p1) = at_the_witch();
    // 谁都没存过钱：0 也要有答案，而并列归回合序第一个（P0）
    assert_eq!(
        hits(&mut app, MagicFilter::TopDeposit, p0),
        vec![p0],
        "「最多」永远有答案 —— 女巫的窗口不该因为全场是 0 而拒绝开台"
    );

    set_cash(&mut app, p1, 30_000);
    set_points(&mut app, p1, 400);
    assert_eq!(hits(&mut app, MagicFilter::TopCash, p0), vec![p1]);
    assert_eq!(hits(&mut app, MagicFilter::TopPoints, p0), vec![p1]);
    assert_eq!(
        hits(&mut app, MagicFilter::TopDeposit, p0),
        vec![p0],
        "现金与点券都归她，存款这条仍该答 P0 —— 五本账不是同一本"
    );
    assert_eq!(pick(&mut app, p0, MagicFilter::TopCash), vec![p1]);
}

/// 「土地最多」数的是名下**块数**，不看那块地值多少钱。
///
/// 「大地主」那个词说的是地多（`Books::land_count_of` 的注同一条理由），而「财产最多」
/// 说的是钱 —— 两条判据各是一件事，合成一条就都错了。演示盘恰好摆得开这一幕：
/// 一块满级外滩贵过两块空地。
#[test]
fn top_lands_counts_plots_not_prices() {
    let (mut app, p0, p1) = at_the_witch();
    own(&mut app, 2, Some(0), 5, false);
    own(&mut app, PLOT, Some(1), 0, false);
    own(&mut app, 1, Some(1), 0, false);
    let one = market_value(&mut app, 2);
    let two = market_value(&mut app, PLOT) + market_value(&mut app, 1);
    assert!(
        one > two,
        "前置：要「一块贵过两块」（{one} 对 {two}），否则这两条判据没被分开"
    );

    assert_eq!(
        hits(&mut app, MagicFilter::TopLands, p0),
        vec![p1],
        "按块数是她"
    );
    assert_eq!(
        hits(&mut app, MagicFilter::TopWealth, p0),
        vec![p0],
        "按现值是他"
    );
}

/// 「财产最多」是那笔加总：现金 + 存款 − 负债 − 融资 + 持股现值 + 名下地的现值。
///
/// 「地的现值」用的是拍卖起拍那条式子（同一个数，两套账的说法在这里不能成立），
/// 而欠银行的钱要真的能把人从榜首拉下来。
#[test]
fn wealth_adds_the_estate_and_the_stocks_and_subtracts_the_debts() {
    let (mut app, p0, p1) = at_the_witch();
    own(&mut app, PLOT, Some(1), 3, false);
    assert_eq!(
        hits(&mut app, MagicFilter::TopWealth, p0),
        vec![p1],
        "两人现钱相同，她有一栋楼"
    );

    let worth = market_value(&mut app, PLOT);
    set_loan(&mut app, p1, worth + 10_000);
    assert_eq!(
        hits(&mut app, MagicFilter::TopWealth, p0),
        vec![p0],
        "她的楼值 {worth}，但她欠得比那更多 —— 榜首该翻回 P0"
    );

    let (mut app, p0, p1) = at_the_witch();
    set_shares(&mut app, p1, "stock_001", 20);
    assert!(
        price_of(&mut app, "stock_001") > 0.0,
        "前置：这只股有现价，否则『持股算进财产』是一条空话"
    );
    assert_eq!(
        hits(&mut app, MagicFilter::TopWealth, p0),
        vec![p1],
        "她只有股票也算：现钱相同而多一笔市值"
    );
}

/// 破产的人**不在名单里**：女巫的法术落不到一个已经离场的人头上。
///
/// 这一条的牙齿在「她此刻真的最有钱」那一格 —— 不筛掉破产者，答案就是她。
#[test]
fn a_bankrupt_player_is_off_the_witch_s_list() {
    let (mut app, p0, p1) = at_the_witch();
    set_cash(&mut app, p1, 99_999);
    assert_eq!(
        hits(&mut app, MagicFilter::TopCash, p0),
        vec![p1],
        "前置：没破产之前她确实压过 P0"
    );

    insert(&mut app, p1, (Bankrupt,));
    assert_eq!(hits(&mut app, MagicFilter::TopCash, p0), vec![p0]);
    assert_eq!(
        pick(&mut app, p0, MagicFilter::AllGirls),
        vec![],
        "「所有女生」只剩一个破产者 → 筛空、当场收摊"
    );
    assert_eq!(pending_of(&app), None);
}

/// 名单里**只有玩家**：四位恶人此刻也在世界上（在笼里），但他们没有 `PlayerId`，
/// 于是女巫点不到他们 —— 与 P12「恶人不进回合轮转」是同一件事的两半。
#[test]
fn the_list_never_contains_a_villain() {
    let (mut app, p0, _p1) = at_the_witch();
    let cages = villain_entities(&mut app);
    assert_eq!(
        cages.len(),
        4,
        "前置探针：四位恶人今天都在场上（在笼里），否则『名单里没有他』是句空话"
    );

    assert_eq!(
        hits(&mut app, MagicFilter::AllBoys, p0),
        vec![p0],
        "名单该恰好等于在场玩家，而不是『玩家加上别的行动者』"
    );
    assert_eq!(hits(&mut app, MagicFilter::TopCash, p0), vec![p0]);
    assert!(
        hits(&mut app, MagicFilter::AllGirls, p0)
            .iter()
            .all(|e| !cages.contains(e)),
        "『所有女生』也不该把恶人算进去"
    );
}

// ===== 三、12 道法术 =====

/// 得一张卡：发的是**货架那一副**的道具卡，下标走魔法屋专用子流（第 12 条）。
///
/// 「这一局进过几次店、开过几次槌」都不该改变女巫摸到的那张牌，所以这里断言的是
/// 独立算一遍 `pick_magic` 的结果 —— 与 `Rng` 那十一条子流各自的用例同一条手法。
#[test]
fn gain_card_deals_from_the_item_shelf_on_its_own_stream() {
    let (mut app, p0, _p1) = at_the_witch();
    let faces = item_pool_len(&app);
    assert!(faces > 0, "前置：道具池非空");
    let expect = CardId(Rng::new(GameSeed::default().0).pick_magic(0..=faces - 1));

    assert_eq!(pick(&mut app, p0, MagicFilter::Walker), vec![p0]);
    cast(&mut app, p0, MagicSpell::GainCard);

    assert_eq!(
        hand_of(&app, p0),
        vec![expect],
        "摸到的不是那颗骰子该说的那张"
    );
    assert!(was_cast(&app, MagicSpell::GainCard, "P0"));
    assert!(
        ledger(&app).iter().any(|l| l.starts_with("Gain(P0:")),
        "发卡凭据走的是 `deal_to` 那一条（手牌满了挤掉最前面那张的规则不用重写）"
    );
}

/// 就地加盖：他脚下那块地**免费**升一级，而且不管那块地是谁的。
#[test]
fn build_here_raises_his_storey_and_charges_nothing() {
    let (mut app, p0, p1) = at_the_witch();
    let at = land_tile(&app, PLOT);
    own(&mut app, PLOT, Some(1), 3, false);
    place(&mut app, p1, at);
    let before = cash_of(&mut app, p1);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::BuildHere);

    assert_eq!(plot_state(&mut app, PLOT).1, 4, "楼该升一层");
    assert_eq!(cash_of(&mut app, p1), before, "女巫不收费");
    assert!(was_cast(&app, MagicSpell::BuildHere, "P1"));
}

/// 加盖落空的两种物理前提：押着的地、已经满级的地。
///
/// 两条都断言**等级没变** —— 「落空」与「少升一级」在流水上看不出来，只有等号看得出来。
#[test]
fn build_here_misses_a_mortgaged_or_full_plot() {
    for (label, level, mortgaged) in [("押着的地上不去", 3, true), ("满级的地上不去", 5, false)]
    {
        let (mut app, p0, p1) = at_the_witch();
        let at = land_tile(&app, PLOT);
        own(&mut app, PLOT, Some(1), level, mortgaged);
        place(&mut app, p1, at);

        assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
        cast(&mut app, p0, MagicSpell::BuildHere);

        assert_eq!(plot_state(&mut app, PLOT).1, level, "{label}");
        assert!(
            was_cast(&app, MagicSpell::BuildHere, ""),
            "{label}：落空也要有凭据"
        );
        assert_eq!(pending_of(&app), None, "{label}：落空也得把回合交还");
    }
}

/// 存现金：全部现钱转进存款。是好事，而且女巫不管你要不要（防强盗那一刀）。
#[test]
fn bank_cash_moves_his_whole_wallet_into_the_deposit() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::BankCash);

    assert_eq!(cash_of(&mut app, p1), 0);
    assert_eq!(
        bank_of(&mut app, p1).0,
        20_000,
        "那笔钱该在存款里，而不是凭空消失"
    );
    assert!(was_cast(&app, MagicSpell::BankCash, "P1"));
    assert_cash_audit(&mut app, 20_000);
}

/// 存现金落空的那一格：他身上已经一分钱没有。
///
/// 它存在的理由是**给上面那条装牙齿** —— 若 `BankCash` 无条件返回 `true`，
/// 「名单里有没有他」就分辨不出「搬了钱」与「什么都没搬」。
#[test]
fn bank_cash_misses_the_one_who_is_already_liquid() {
    let (mut app, p0, p1) = at_the_witch();
    set_cash(&mut app, p1, 0);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::BankCash);

    assert!(was_cast(&app, MagicSpell::BankCash, ""));
    assert_eq!(bank_of(&mut app, p1).0, 0, "没钱可搬，存款也该一分没进");
}

/// 抽命运 3 张 = **间接效果**，所以神明的修正吃得到（PLAN §三 那一行的落地方式）。
///
/// A/B 两条同一份前置、只差身上那位神。命运是**定序**发的（P3d①），所以三张牌就是牌堆
/// 头三行：fate_001 入狱、fate_003 缴税 5000、fate_004 银行利息 4000。
/// （编号里缺的那位是 fate_002「回到起点」—— 10-26 起点整格删掉时一起退场，编号不复用。）
/// - 没神：现金 20000 − 5000 + 4000 = 19000，牢判 3 天；
/// - 大衰神：罚钱加倍（−10000）、得钱作废（+0）→ 10000，而那张「入狱」的天数被
///   `doom_days` 乘成 6。
///
/// 两条都不写「与神明无关」这种否定式：现金与天数都是**数字**，差别看得见。
#[test]
fn the_fate_spell_draws_three_and_the_god_taxes_them() {
    let plain = fate_run(None);
    let doomed = fate_run(Some(God::BigDoom));

    assert_eq!(
        plain.fates,
        vec![
            "Card(P1:fate_001)",
            "Card(P1:fate_003)",
            "Card(P1:fate_004)"
        ],
        "三张牌要按定序发，还要各自留下一条卡面凭据（顶栏念的就是这一行）"
    );
    assert_eq!(doomed.fates, plain.fates, "神不该改变抽到什么牌");

    assert_eq!(plain.cash, 19_000, "没神：−5000 与 +4000 照卡面走");
    assert_eq!(doomed.cash, 10_000, "大衰神：罚钱加倍、得钱作废");

    assert_eq!(
        plain.sentence,
        Some(Detention {
            place: DetentionPlace::Jail,
            term: Term::Turns(3)
        }),
        "fate_001 那张真把他关进去了（正面断言：这一趟真的动了世界）"
    );
    assert_eq!(
        doomed.sentence.map(|d| d.term),
        Some(Term::Turns(6)),
        "厄运天数加倍是命运那条路的规则，女巫借的就是这条路"
    );
}

/// [`the_fate_spell_draws_three_and_the_god_taxes_them`] 跑完一遍的读数。
struct FateRun {
    fates: Vec<String>,
    cash: i64,
    sentence: Option<Detention>,
}

fn fate_run(god: Option<God>) -> FateRun {
    let (mut app, p0, p1) = at_the_witch();
    if let Some(g) = god {
        attach_god(&mut app, p1, g, None);
    }
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::DrawFate);
    assert!(was_cast(&app, MagicSpell::DrawFate, "P1"));
    FateRun {
        fates: ledger(&app)
            .into_iter()
            .filter(|l| l.starts_with("Card("))
            .collect(),
        cash: cash_of(&mut app, p1),
        sentence: detention_of(&app, p1),
    }
}

/// 变卖所有卡：手牌清空、换成点券，价与「卖回百货」同一条式子。
///
/// 那条折扣的**算式**由 `shop` 自己的用例钉住（9 折是卖回那条路的数）；这里验的是女巫走的
/// 是同一条式子而不是另开一张价目表：手牌必须真的空了、点券必须真的到账。
#[test]
fn sell_cards_empties_the_hand_and_pays_in_points() {
    let (mut app, p0, p1) = at_the_witch();
    let frame = item_index(&app, "item_frame").0;
    let turn = item_index(&app, "item_turn").0;
    deal(&mut app, p1, frame);
    deal(&mut app, p1, turn);
    assert_eq!(hand_of(&app, p1).len(), 2, "前置：她手里两张");
    assert_eq!(points_of(&app, p1), 0);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::SellCards);

    assert!(hand_of(&app, p1).is_empty(), "一张不剩");
    assert!(
        points_of(&app, p1) > 0,
        "点券真的到账（不是发了个凭据就算）"
    );
    assert!(was_cast(&app, MagicSpell::SellCards, "P1"));
    assert_points_audit(&mut app, 0);
}

/// 手里没卡时这道落空 —— 而不是「成功了但换成 0 点券」。
#[test]
fn sell_cards_misses_the_empty_hand() {
    let (mut app, p0, _p1) = at_the_witch();
    assert!(hand_of(&app, p0).is_empty(), "前置：手里没卡");

    assert_eq!(pick(&mut app, p0, MagicFilter::Walker), vec![p0]);
    cast(&mut app, p0, MagicSpell::SellCards);

    assert!(was_cast(&app, MagicSpell::SellCards, ""));
    assert_eq!(points_of(&app, p0), 0, "没卡就不该有点券");
}

/// 变卖所有道具：按**研究所标价**全额换点券（变卖不是折扣柜台，女巫不抽成）。
///
/// 价读的是 `ItemKind::price_points` 那同一个函数而不是重抄一遍数 —— 抄了就测不出
/// 「两条路用的是不是一张价目表」。
#[test]
fn sell_items_empties_the_inventory_at_the_lab_price() {
    let (mut app, p0, p1) = at_the_witch();
    grant_item(&mut app, p1, ItemKind::Roadblock, 2);
    grant_item(&mut app, p1, ItemKind::Mine, 1);
    // 卖的是**全部**，连开局发给每个人的那一枚路障（`Inventory::starting`）一起，
    // 所以价按「变卖前那份清单」算，而不是按「我塞进去的那两件」算
    assert_eq!(
        inventory_of(&app, p1),
        vec![(ItemKind::Roadblock, 3), (ItemKind::Mine, 1)],
        "前置：三枚路障（塞的两枚 + 开局那枚）加一颗地雷"
    );
    let expect: i64 = inventory_of(&app, p1)
        .iter()
        .map(|(kind, count)| i64::from(kind.price_points() * count))
        .sum();

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::SellItems);

    assert!(inventory_of(&app, p1).is_empty(), "一件不剩");
    assert_eq!(
        i64::from(points_of(&app, p1)),
        expect,
        "点券按研究所标价全额"
    );
    assert!(was_cast(&app, MagicSpell::SellItems, "P1"));
}

/// 坐牢与住院：判的是**女巫自己的 3 天**（`MAGIC_DAYS` 那张表），而且人真的被搬进死胡同。
#[test]
fn the_two_confinement_spells_lock_him_up_for_the_witch_s_own_three_days() {
    for (spell, place) in [
        (MagicSpell::Jail, DetentionPlace::Jail),
        (MagicSpell::Hospital, DetentionPlace::Hospital),
    ] {
        let (mut app, p0, p1) = at_the_witch();
        let before = tile_of(&app, p1);

        assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
        cast(&mut app, p0, spell);

        assert_eq!(
            detention_of(&app, p1),
            Some(Detention {
                place,
                term: Term::Turns(3)
            }),
            "{} 该判 3 回合",
            spell.label()
        );
        assert!(
            was_cast(&app, spell, "P1"),
            "{} 的落点要有凭据",
            spell.label()
        );
        assert!(
            ledger(&app)
                .iter()
                .any(|l| l.starts_with(&format!("Detain(P1@{},", place.label()))),
            "{} 该留下「谁被关进哪一处」那一条",
            spell.label()
        );
        assert_ne!(tile_of(&app, p1), before, "他的棋子被挪进了那条死胡同");
    }
}

/// 原地停留：写 1 回合（P4④ 的口径），而他**还在牌桌上** —— 与坐牢的分别是这条断言的全部意义。
#[test]
fn stay_holds_one_turn_and_leaves_him_at_the_table() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::Stay);

    assert_eq!(stay_of(&app, p1), Some(1), "字面 1 回合，不是 3");
    assert!(detention_of(&app, p1).is_none(), "停留不是拘留：他没离场");
    assert!(was_cast(&app, MagicSpell::Stay, "P1"));
}

/// 向后转：翻的是他身上那一位，不是棋盘。
#[test]
fn turn_back_flips_him() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(heading_of(&app, p1), Heading::Forward, "前置：开局恒正向");

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::TurnBack);

    assert_eq!(heading_of(&app, p1), Heading::Backward);
    assert!(was_cast(&app, MagicSpell::TurnBack, "P1"));
    assert!(detention_of(&app, p1).is_none() && stay_of(&app, p1).is_none());
}

/// 就地拆除：降一层、**保所有权** —— 女巫的「拆除」不是征收。
#[test]
fn demolish_drops_one_storey_and_keeps_the_owner() {
    let (mut app, p0, p1) = at_the_witch();
    let at = land_tile(&app, PLOT);
    own(&mut app, PLOT, Some(1), 3, false);
    place(&mut app, p1, at);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::Demolish);

    let (owner, level, _) = plot_state(&mut app, PLOT);
    assert_eq!(level, 2, "降一级");
    assert_eq!(owner, Some(1), "地还是她的（征收那一档是新闻那条路的）");
    assert!(was_cast(&app, MagicSpell::Demolish, "P1"));
}

/// 拆除落空的那一格：他脚下是**空地**（10-23 级数重判之后空地就是 Lv0，没有「再降一层」）。
///
/// 这一格的形状值得单独钉：`damage` 报的 `hit` 是「范围盖到几块地」，空地也算盖到，
/// 所以拿它当「法术生效」的答案，顶栏会对一块空地念出「女巫拆掉了她的楼」。
#[test]
fn demolish_misses_an_empty_plot() {
    let (mut app, p0, p1) = at_the_witch();
    let at = land_tile(&app, PLOT);
    own(&mut app, PLOT, Some(1), 0, false);
    place(&mut app, p1, at);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::Demolish);

    assert_eq!(
        plot_state(&mut app, PLOT).1,
        0,
        "空地降不下去（`saturating_sub` 的地板）"
    );
    assert!(
        was_cast(&app, MagicSpell::Demolish, ""),
        "没拆掉东西就不该进生效名单"
    );
}

/// 拍卖他脚下那块地：开槌的是施法的人，而**这台机器自己说了算**的那几问（押着的、无主的、
/// 是他自己的、场上没别人的）由同一条 `can_open` 回答 —— 顶栏不能先说「挂上拍卖台」，
/// 再让机器悄悄丢掉这一槌。
#[test]
fn auction_here_puts_his_plot_on_the_block() {
    let (mut app, p0, p1) = at_the_witch();
    let at = land_tile(&app, PLOT);
    own(&mut app, PLOT, Some(1), 2, false);
    place(&mut app, p1, at);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::AuctionHere);
    settle(&mut app, 3);

    assert!(was_cast(&app, MagicSpell::AuctionHere, "P1"));
    let lot = lot_of(&app).expect("这一槌该真开出场，而不是停在『请求』那一格");
    assert_eq!(lot.land, LandId(PLOT));
    assert_eq!(lot.seller, Some(p1), "被拍的是她手里那块");
    assert_eq!(lot.by(), Some(p0), "钱归开槌的施法者");
    // 回合**没有**被施法那一步交还：拍卖机自己会写 `WaitDecision`，同一回合两处各写一个
    // `NextState` 就是坑 37（症状是回合指针已经给了下一个人，而台上还在喊价）
    assert_eq!(
        pending_of(&app).map(|(_, about)| about),
        Some(Pending::Auction),
        "窗口该挂到此刻该出价的人身上"
    );
    assert_eq!(
        current(&app),
        p0,
        "回合还该停在施法那位这儿 —— 拍卖机接手的是**这一回合**，不是替他新开一回合。         这条断言是 `handed_to_auction` 那道守卫的全部意义：施法那一步若也写 `TurnEnd`，         回合指针就往前走了一格，而台上还在喊价（坑 37 的形状）"
    );
}

/// 押着的地不上拍：法术落空，而**落空那次回合由这一步交还**（没把回合押给一台开不出场的机器）。
#[test]
fn auction_here_misses_a_plot_that_cannot_go_up() {
    let (mut app, p0, p1) = at_the_witch();
    let at = land_tile(&app, PLOT);
    own(&mut app, PLOT, Some(1), 2, true);
    place(&mut app, p1, at);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::AuctionHere);
    settle(&mut app, 3);

    assert!(was_cast(&app, MagicSpell::AuctionHere, ""), "押着的地落空");
    assert!(lot_of(&app).is_none(), "机器上不该多出一场");
    assert_ne!(phase(&app), TurnPhase::WaitDecision, "落空就得把回合交回去");
}

// ===== 四、免疫矩阵 =====

/// 把世界摆成「这道法术**一定落得到人**」。
///
/// 矩阵问的是「挡不挡」，所以每一条都得先让它**落得成** —— 否则「名单是空的」会让每一条
/// 断言都绿（本项目第五次登记的那类空跑）。
fn arm(app: &mut App, target: Entity, spell: MagicSpell) {
    let at = land_tile(app, PLOT);
    match spell {
        MagicSpell::BuildHere | MagicSpell::Demolish | MagicSpell::AuctionHere => {
            own(app, PLOT, Some(1), 3, false);
            place(app, target, at);
        }
        MagicSpell::SellCards => {
            let frame = item_index(app, "item_frame").0;
            deal(app, target, frame);
        }
        MagicSpell::SellItems => {
            grant_item(app, target, ItemKind::Roadblock, 2);
        }
        // 其余六道只靠开局那份就落得成：现钱、方向、停留计时、手牌空不影响它们各自那一格
        _ => {}
    }
}

/// 矩阵第一列：目标身上带着**一层免罪卡**。
///
/// 期望值就是 PLAN §三 那句话加上 10-25 的拍板：十二道里**只有**坐牢与住院挡得住，
/// 且挡住时吃掉一层；其余十道**一层不花** —— 免疫规则免的是事，不是卡。
#[test]
fn a_forgiveness_card_stops_only_the_two_that_lock_someone_up() {
    for spell in MagicSpell::ALL {
        let (mut app, p0, p1) = at_the_witch();
        arm(&mut app, p1, spell);
        insert(&mut app, p1, (Insulation { charges: 2 },));

        assert_eq!(
            pick(&mut app, p0, MagicFilter::AllGirls),
            vec![p1],
            "{}：前置名单",
            spell.label()
        );
        cast(&mut app, p0, spell);

        let forgivable = matches!(spell, MagicSpell::Jail | MagicSpell::Hospital);
        // 「抽命运 3 张」是**间接效果**：这一道本身挡不住（牌已经发到头上，三张真的抽了，
        // 所以名单里仍有他），但它抽来的那张 fate_001「直接入狱」走的正是命运那条路 ——
        // 而那条路认护身。PLAN §三 那句「受神明修正」说的是同一条通道：修正与抵消都落在
        // **牌**上，不落在女巫身上。
        let indirect = spell == MagicSpell::DrawFate;
        assert_eq!(
            shield_of(&app, p1),
            Some(if forgivable || indirect { 1 } else { 2 }),
            "{}：{} —— 护身层数",
            spell.label(),
            if forgivable {
                "挡下了，该吃掉一层"
            } else if indirect {
                "吃掉一层的是抽来的那张牌"
            } else {
                "不吃卡，一层都不该花"
            }
        );
        assert_eq!(
            was_blocked(&app, spell, "P1"),
            forgivable,
            "{}：挡住才有 MagicBlock 那一条",
            spell.label()
        );
        assert!(
            was_cast(&app, spell, if forgivable { "" } else { "P1" }),
            "{}：名单那一格该是{}",
            spell.label(),
            if forgivable { "空的" } else { "她" }
        );
        if forgivable {
            assert!(
                detention_of(&app, p1).is_none(),
                "{}：挡住就不该把人关起来",
                spell.label()
            );
        }
        if indirect {
            assert!(
                detention_of(&app, p1).is_none(),
                "抽来的那张「直接入狱」被她自己的护身挡了 —— 这就是『间接效果』四个字的形状"
            );
        }
    }
}

/// 矩阵第二列：目标是**大福神**附身（厄运豁免）。十二道**全部照常生效**。
///
/// 这一列才是这一阶段真正的难点，而它的答案在**构造方式**上：女巫那两条走
/// `Insulation::absorb`，刻意不接 `deck::resist` 里的神明分支 —— 祂豁免的是厄运，
/// 女巫的法术不是厄运。所以这里既没有一条查询去「逐条挡」，也没有一处特例。
#[test]
fn a_god_on_the_body_does_not_stop_the_witch() {
    for spell in MagicSpell::ALL {
        let (mut app, p0, p1) = at_the_witch();
        arm(&mut app, p1, spell);
        attach_god(&mut app, p1, God::BigFortune, None);
        assert!(possession_of(&app, p1).is_some(), "前置：祂真的在她身上");

        assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
        cast(&mut app, p0, spell);

        assert!(
            was_cast(&app, spell, "P1"),
            "{}：身上有大福神也该照常生效",
            spell.label()
        );
        assert!(
            !was_blocked(&app, spell, "P1"),
            "{}：不该出现『挡住』",
            spell.label()
        );
    }
}

/// 矩阵第三列：**大衰神**（厄运天数加倍）。那两道的 3 天**不加倍** —— 女巫的天数写在
/// `MAGIC_DAYS` 这张表上，而不是「落到某人头上的厄运」那条尺上。
///
/// 对照：同一张「入狱」在命运那条路上会被乘成 6（见
/// [`the_fate_spell_draws_three_and_the_god_taxes_them`]）。两格放在一起才说得出「量不到」。
#[test]
fn doom_does_not_lengthen_the_witch_s_sentence() {
    for (spell, place) in [
        (MagicSpell::Jail, DetentionPlace::Jail),
        (MagicSpell::Hospital, DetentionPlace::Hospital),
    ] {
        let (mut app, p0, p1) = at_the_witch();
        attach_god(&mut app, p1, God::BigDoom, None);

        assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
        cast(&mut app, p0, spell);

        assert!(was_cast(&app, spell, "P1"), "{}：照常生效", spell.label());
        assert_eq!(
            detention_of(&app, p1),
            Some(Detention {
                place,
                term: Term::Turns(3)
            }),
            "{}：女巫判的就是 3 天",
            spell.label()
        );
    }
}

/// 挡住那一下的**最后一层**要跟着摘掉 —— 与 P3a 那条同一格。
///
/// 「有免罪卡但什么都不挡」的玩家在面板上是一句假话，而这条路径（女巫的坐牢）与
/// 命运那条（`deck::resist`）共用同一个 `Insulation`，所以两处都得做同一件收尾。
#[test]
fn the_last_layer_is_taken_off_with_the_card() {
    let (mut app, p0, p1) = at_the_witch();
    insert(&mut app, p1, (Insulation { charges: 1 },));

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::Jail);

    assert_eq!(shield_of(&app, p1), None, "耗尽的护身要被摘掉");
    assert!(was_blocked(&app, MagicSpell::Jail, "P1"));
    assert!(detention_of(&app, p1).is_none());
}

/// 护身是**个人的**事：同一批里没挡住的那个人照样挨，包括施法者自己。
#[test]
fn a_shield_protects_its_own_holder_only() {
    let (mut app, p0, p1) = at_the_witch();
    insert(&mut app, p1, (Insulation { charges: 1 },));
    // 「所有女生」在她这儿只剩她一个，所以把施法者也改成女生才能一次点到两个人
    insert(&mut app, p0, (Gender::Female,));

    let hits = pick(&mut app, p0, MagicFilter::AllGirls);
    assert_eq!(hits, vec![p0, p1], "前置：两人都是女生，回合序 P0 在前");
    cast(&mut app, p0, MagicSpell::Jail);

    assert!(
        detention_of(&app, p0).is_some(),
        "他自己没有护身，就该被自己的法术关进去"
    );
    assert!(detention_of(&app, p1).is_none(), "她的护身挡住了她那一份");
    assert!(
        was_cast(&app, MagicSpell::Jail, "P0"),
        "生效名单里只留下没挡住的那一位"
    );
}

/// 施法**不消耗**任何卡与点券：女巫是踩到才办事的柜台，不是收费的服务。
///
/// 这一条钉的是「魔法屋不该偷偷学百货收一次钱」：两套账在整场法事里一分不动。
#[test]
fn the_ritual_costs_the_caster_nothing() {
    let (mut app, p0, p1) = at_the_witch();
    set_points(&mut app, p0, 300);
    let cash = cash_of(&mut app, p0);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::BankCash);

    assert_eq!(cash_of(&mut app, p0), cash, "施法者一分没花");
    assert_eq!(
        points_of(&app, p0),
        300,
        "点券也没动（两套账都不收女巫的过路费）"
    );
    assert!(
        !ledger(&app)
            .iter()
            .any(|l| l.starts_with("Shop(") || l.starts_with("BankOp(P0")),
        "P0 这一趟没在任何柜台上过账"
    );
}
