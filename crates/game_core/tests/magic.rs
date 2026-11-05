//! P13/P13c 魔法屋的验收：11 种筛选条件 × 12 种法术，外加 PLAN §四 点名那张**免疫矩阵**。
//!
//! 分五段：
//!
//! - **窗口**（第一段）验的是那一屏的形状：条件与名单是同一次结算的产物、主语不对不算回答、
//!   窗口与草稿同生同灭、施法之后回合交得回去、**钉住的空名单也必须把回合交出去**
//!   （坑 38 唯一可达的那条路）；
//! - **摇条件**（第二段）是 P13c 新长出来的那一半：筛不出人就重摇（且这一格真被走到）、
//!   同一种子同一条、而「走路的人」保留但降权；
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
//! （规则层那同一个 `filter_targets`），因为**一局窗口只挂一条条件**（P13c 之后那条是摇/钉
//! 出来的，不是玩家挑的），而「并排看五条判据」这类断言不该为此重启五局。真窗口那一条路由
//! [`pick`]（钉一条 → 再踩一次女巫家）覆盖，`pin_witch` 走的就是 `NM_MAGIC=<条件>` 那格接缝。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::{Assets, ResMut, Resource, Update};
use common::*;
use game_core::prelude::{
    Ask, BankBalance, Bankrupt, CardId, CardSet, DataHandles, Detention, DetentionPlace, GameRules,
    GameSeed, GameSetup, Gender, God, Heading, ItemKind, LandId, MagicFilter, MagicRitual,
    MagicSpell, Mount, PassiveEvent, PassiveMove, Pending, PlayerName, Rng, Term, TileKind,
    TurnPhase, Villain,
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

/// 装配到「P0 刚踩上魔法屋、那一屏挂着」。
///
/// 与彩票那批同一手法：不替他 `offer` 窗口 —— 先按种子读出第一掷，再把他摆到「走那么多步
/// 正好落到魔法屋」那一格，于是踩到与开窗口走的都是真走路、真落格分派。
/// 开局那两位神与路上的宝箱在这里清掉（`clear_gods` 把 `Occupancy` 整族 despawn），否则
/// 「走过去踩到女巫家」这一路上撞见财神会当场搬一笔钱，下面那些现金断言就不再只等于法术。
///
/// ⚠️ 条件不再是玩家挑的，所以这里必须**钉一条**（走 [`pin_witch`] —— 与 `NM_MAGIC=girls`
/// 同一格开发接缝）。传 `None` 是让规则层按权重摇，那一条的形状由
/// [`stepping_on_the_witch_house_opens_the_drawn_screen`] 单独验；钉住的话这批用例才敢写
/// `[p1]` 这种具体名单，而不会被某一局的骰子翻面。默认钉「所有女生」。
fn at_the_witch_pinned(pinned: Option<MagicFilter>) -> (App, Entity, Entity) {
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
    pin_witch(&mut app, pinned);
    let face = Rng::new(seed.0).roll_die();
    let at = kind_tile(&app, TileKind::Magic);
    stand_to(&mut app, p0, at, face);

    roll(&mut app, p0);
    settle(&mut app, 6);

    if let Some(filter) = pinned {
        assert_eq!(
            pending_of(&app),
            Some((p0, Pending::MagicSpell(filter))),
            "前置该把 P0 送到女巫家门口并钉好这一条：骰面 {face}，该落在 {at} 号，流水 {:?}",
            events(&app)
        );
    } else {
        // 让规则层自己摇，那就只能验**形状**而不是那一条：窗口是法术那一屏、
        // 窗口上的条件与草稿是同一条、且筛得出人（这三条就是 P13c 的口径）
        let Some((_, Pending::MagicSpell(on_window))) = pending_of(&app) else {
            panic!(
                "前置该把 P0 送到女巫家门口并直接开法术那一屏：骰面 {face}，\
                 该落在 {at} 号，实际挂着 {:?}，流水 {:?}",
                pending_of(&app),
                events(&app)
            );
        };
        let (on_draft, hits) = draft(&app).expect("窗口挂着，草稿就该已经筛好");
        assert_eq!(on_window, on_draft, "窗口与草稿该是同一条条件");
        assert!(!hits.is_empty(), "摇出来的条件筛不出人就该重摇");
    }
    (app, p0, p1)
}

/// 钉好「所有女生」那一条的常用夹具 —— 名单就是 `[p1]`，一个人，最好断言。
fn at_the_witch() -> (App, Entity, Entity) {
    at_the_witch_pinned(Some(MagicFilter::AllGirls))
}

// ===== 读数 =====

/// 草稿里那份名单 —— 法术真正落到的就是它，而它**不上屏**（P13c），所以读它等于读规则层。
fn draft(app: &App) -> Option<(MagicFilter, Vec<Entity>)> {
    app.world().resource::<MagicRitual>().draft.clone()
}

/// 让 P0 **再踩一次**女巫家，钉住 `filter` 那一条，返回这一次筛出的人。
///
/// P13c 之前这一格是「点第一步那颗条件按钮」；现在条件由规则层摇，测试要控制它只能钉
/// （[`pin_witch`]，与 `NM_MAGIC=girls` 同一条路径）。走的仍是真走路 + 真落格分派：
/// 先把上一场按「离开」收干净（窗口与草稿同生同灭那条纪律），再把轮转指针挪回他、
/// 随机流退回开局第一条（于是骰面还是 `face`，[`common::rewind_dice`] 存在的理由就是这一格），
/// 摆到门口走那一趟。
///
/// 钉住的条件筛空时**照旧开窗**（名单是空的），落空由施法那一步收尾 —— 与生产路径
/// （摇到空就重摇）相反的那一格，只有这条接缝进得去。
fn pick(app: &mut App, caster: Entity, filter: MagicFilter) -> Vec<Entity> {
    set_policy(app, DecisionPolicy::DeclineAll);
    settle(app, 2);
    assert!(
        run_until_phase(app, TurnPhase::WaitRoll, 30),
        "离开之后回合机该回到掷骰前，实际停在 {:?}",
        phase(app)
    );
    set_policy(app, DecisionPolicy::Hold);
    make_current(app, caster);
    rewind_dice(app);
    pin_witch(app, Some(filter));
    let at = kind_tile(app, TileKind::Magic);
    let face = Rng::new(app.world().resource::<GameSeed>().0).roll_die();
    stand_to(app, caster, at, face);
    roll(app, caster);
    settle(app, 6);
    draft(app).map(|(_, hits)| hits).unwrap_or_default()
}

/// 点那一屏的法术按钮，并跑两帧让规则层施法。
fn cast(app: &mut App, caster: Entity, spell: MagicSpell) {
    magic_spell_ask(app, caster, spell);
    settle(app, 2);
}

/// 一次「按权重摇条件」的请求（[`drawn_with_seed`] / [`drawn_series`] 的载体）。
#[derive(Resource)]
struct DrawReq(Option<(u64, Entity, u32)>);

/// 摇出来的那一条，**与第一试摇到的那一条**分开记 —— 后者是用来证明重摇真的被走到了，
/// 否则「每次摇出来都筛得出人」可能只是因为第一试本来就筛得出人（空跑的那一类）。
#[derive(Resource, Default)]
struct DrawOut {
    first: Option<MagicFilter>,
    drawn: Option<(MagicFilter, Vec<Entity>)>,
    /// `drawn_series` 的那一条流：连抽 N 次定下来的条件，按序
    series: Vec<MagicFilter>,
}

/// 摇条件要 `FilterDesk`（里面是 `Books`），与 [`common::hits`] 同一条理由单开一次 system，
/// 走的仍是规则层那两个公开函数（`weighted_filter` / `draw_filter`）。
fn draw_system(
    mut req: ResMut<DrawReq>,
    mut out: ResMut<DrawOut>,
    desk: game_core::magic::FilterDesk<'_, '_>,
) {
    let Some((seed, current, times)) = req.0.take() else {
        return;
    };
    // 每次现装一条流：分布类断言要的是「同一种子同一条序列」，而局内那条已经被开局
    // 那几掷与前面几百次抽样走过了水位
    let mut first_rng = Rng::new(seed);
    let first = game_core::magic::weighted_filter(&mut first_rng);
    let mut rng = Rng::new(seed);
    out.drawn = Some(game_core::magic::draw_filter(
        &mut rng, current, &desk, None,
    ));
    out.first = Some(first);
    // 连抽那一段读的是**同一条已经走过一次的流**：分布要看的是「一条流连着施法几百场」，
    // 而不是「几百个连续整数种子各自的第一抽」—— 后者是相邻状态，第一抽会互相相关，
    // 比例统计出来是假的（第一版就栽在这里：200 个连续种子把 1:2 抽成了 21:27）
    out.series = (0..times)
        .map(|_| game_core::magic::draw_filter(&mut rng, current, &desk, None).0)
        .collect();
}

/// 按种子 `seed` 现摇一条条件，返回「第一试摇到谁」与「重摇之后定下谁」。
fn drawn_with_seed(
    app: &mut App,
    seed: u64,
    current: Entity,
) -> (MagicFilter, (MagicFilter, Vec<Entity>)) {
    app.insert_resource(DrawReq(Some((seed, current, 0))));
    app.insert_resource(DrawOut::default());
    app.add_systems(Update, draw_system);
    app.update();
    let out = app.world().resource::<DrawOut>();
    (
        out.first.expect("第一试该摇出一条"),
        out.drawn.clone().expect("摇完该有一条定下来"),
    )
}

/// 同一条流上连抽 `times` 场法事，返回每次定下来的那一条条件。
fn drawn_series(app: &mut App, seed: u64, current: Entity, times: u32) -> Vec<MagicFilter> {
    app.insert_resource(DrawReq(Some((seed, current, times))));
    app.insert_resource(DrawOut::default());
    app.add_systems(Update, draw_system);
    app.update();
    app.world().resource::<DrawOut>().series.clone()
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

// ===== 一、那一屏（P13c 之后只有一屏）=====

/// 踩到魔法屋开的是**那一屏**：窗口带着一条条件，而名单已经摇完、填进草稿。
///
/// P13c 之前这一格断的是「开的是第一步、还没有名单」；现在条件与名单是**同一次结算**的
/// 产物（[`draw_filter`] 摇一条、当场筛人），所以「窗口挂着而草稿是空的」在结构上就是分叉 ——
/// 这一条把它钉住，而钉的方式是让规则层**自己摇**（`pin_witch(None)`，与玩家实际那一屏同一条路）。
#[test]
fn stepping_on_the_witch_house_opens_the_drawn_screen() {
    let (app, p0, _p1) = at_the_witch_pinned(None);
    assert_eq!(
        phase(&app),
        TurnPhase::WaitDecision,
        "窗口挂着时回合不该往前走"
    );
    let Some((who, Pending::MagicSpell(on_window))) = pending_of(&app) else {
        panic!(
            "踩到女巫家该直接开法术那一屏，实际挂着 {:?}",
            pending_of(&app)
        );
    };
    assert_eq!(who, p0, "窗口挂在踩格那位身上");
    let (on_draft, hits) = draft(&app).expect("窗口挂着，草稿就该已经筛好");
    assert_eq!(
        on_window, on_draft,
        "窗口上的条件与草稿里的必须是同一条 —— 面板念一条、法术落另一条就是分叉"
    );
    assert!(!hits.is_empty(), "摇出来的条件必须筛得出人（筛空就重摇）");
}

/// 主语门禁：窗口挂在 P0 身上，**别人**答的法术不算回答。
///
/// 与保释／柜台那几条同一条形状。少了这一格，「两个人同时对着一场法事」就能替女巫把
/// 那道法术落到 P0 选好的名单上，而窗口的主人毫不知情。
#[test]
fn a_spell_from_the_wrong_seat_is_not_an_answer() {
    let (mut app, p0, p1) = at_the_witch();
    magic_spell_ask(&mut app, p1, MagicSpell::Jail);
    settle(&mut app, 2);

    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::MagicSpell(MagicFilter::AllGirls))),
        "别人答的法术不该把这一屏收走"
    );
    assert!(
        magic_lines(&app).is_empty(),
        "别人那一句该一个字都不办，流水 {:?}",
        magic_lines(&app)
    );
    assert_eq!(
        draft(&app).map(|(_, hits)| hits),
        Some(vec![p1]),
        "草稿也不该被别人的那一句消耗掉"
    );
}

/// 窗口上的条件**就是**草稿里那一条，而法术落到的是同一次筛出的名单。
///
/// 「名单不上屏」是这一格的全部玩法（看得见名单的玩家一定能选到对自己有利的那一条），
/// 但规则层内部必须只有一份口径：`settle_magic_landing` 筛一次、`magic_spell_system` 读那一份，
/// 而不是施法时再筛一次 —— 中间世界动了（有人破产、被新闻搬走）就会给出一份不同的名单。
#[test]
fn the_window_and_the_draft_are_the_same_ritual() {
    let (mut app, p0, p1) = at_the_witch_pinned(None);
    let on_window = match pending_of(&app).expect("前置：窗口挂着") {
        (_, Pending::MagicSpell(filter)) => filter,
        (_, other) => panic!("踩到女巫家该开法术那一屏，实际挂着 {other:?}"),
    };
    let on_draft = draft(&app).expect("草稿该与窗口同时出现");
    assert_eq!(on_window, on_draft.0);

    // 钉一条「所有女生」再踩一次：名单换成一个人，窗口跟着换
    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::MagicSpell(MagicFilter::AllGirls))),
        "窗口该带上**摇出来那一条**，面板才念得出女巫说的是什么"
    );
}

/// 按「离开」：窗口与草稿**同生同灭**。
///
/// 草稿是资源而不是 `Pending` 的字段（`Pending` 是 `Copy` 枚举，装不下 `Vec`），所以没有
/// 任何东西替它收账 —— 不跟着清就留下一份过期名单，dump 读得到它，而回主菜单
/// 那一帧也没有东西清（坑 22/25 同族）。
#[test]
fn leaving_the_screen_clears_the_draft_with_the_window() {
    let (mut app, _p0, p1) = at_the_witch();
    assert_eq!(draft(&app).map(|(_, hits)| hits), Some(vec![p1]));

    set_policy(&mut app, DecisionPolicy::DeclineAll);
    settle(&mut app, 2);

    assert_eq!(pending_of(&app), None, "离开之后不该还挂着窗口");
    assert!(draft(&app).is_none(), "窗口收掉了，草稿必须跟着走");
    assert_ne!(phase(&app), TurnPhase::WaitDecision, "回合该交还给状态机");
}

/// 钉住一条**没人命中**的条件：窗口照样开，点一道下去照样落空、照样把回合交出去。
///
/// 生产路径摇不出空名单（筛空就重摇），所以这一格只有 `NM_MAGIC=<没人命中的条件>` 这条
/// 接缝进得来 —— 而它是**坑 38 唯一可达的路**：「谁清窗口谁就得交还回合」这句话在
/// 空名单那一条分支上最容易漏（只 `pending.clear()` 的写法把整局停在 `WaitDecision`，
/// 不报错、只是不动了）。
#[test]
fn a_pinned_empty_list_still_hands_the_turn_back() {
    let (mut app, p0, _p1) = at_the_witch();
    // 默认阵容两位都步行，所以「开机车的」是空的
    assert_eq!(
        pick(&mut app, p0, MagicFilter::OnBike),
        vec![],
        "前置：没人骑车，钉住这一条就该筛空"
    );
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::MagicSpell(MagicFilter::OnBike))),
        "钉住的条件筛空时窗口照开 —— 落空是玩家那一屏的合法结果"
    );

    cast(&mut app, p0, MagicSpell::Jail);

    assert_eq!(pending_of(&app), None, "施完法窗口就收");
    assert!(draft(&app).is_none(), "草稿一次就消耗掉");
    assert_ne!(
        phase(&app),
        TurnPhase::WaitDecision,
        "落空也得把回合交还 —— 否则整局停在这一格"
    );
    // 正面断言：落空也要留下一条「施了、没人挨着」的凭据（`targets` 是空的），
    // 而不是静默什么都不写 —— 只断「没人被关起来」的那一类否定式很容易空跑
    assert!(
        was_cast(&app, MagicSpell::Jail, ""),
        "空名单也得有一条落空的凭据，流水 {:?}",
        magic_lines(&app)
    );
}

/// 施法即收尾：一颗法术按钮既办完事又交还回合（与建设柜台那种「办完还要再按一次先不建」不同）。
#[test]
fn the_screen_hands_the_turn_back_after_casting() {
    let (mut app, p0, p1) = at_the_witch();
    assert_eq!(draft(&app).map(|(_, hits)| hits), Some(vec![p1]));
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

/// 三张表的形状：11 条条件、11 格权重、12 道法术，名字与说明都上得了屏。
///
/// 「玩家不能指定具体人」这条玩法（PLAN §三）由条件的**条数与形状**表达：表里若长出
/// 「指定某人」那一格，这一条就红在条数上，而那一格的代价是「害人变成一次查名操作」。
/// 权重表（P13c）与条件表**同源**这一条只能钉在这里：漏一条不会编译失败（`FILTER_WEIGHTS`
/// 是手写的表而不是 `match`），症状只会是「那条条件永远摇不出来」。
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

    // ===== 权重表与条件表同源（P13c）=====
    let weighted: Vec<MagicFilter> = game_core::magic::FILTER_WEIGHTS
        .iter()
        .map(|(f, _)| *f)
        .collect();
    assert_eq!(
        weighted.len(),
        MagicFilter::ALL.len(),
        "权重表与条件表不同宽 —— 多出来的那一格摇的是空气"
    );
    for filter in MagicFilter::ALL {
        assert!(
            weighted.contains(&filter),
            "「{}」不在权重表里 —— 这条条件永远摇不出来",
            filter.label()
        );
    }
    assert_eq!(
        weighted
            .iter()
            .map(|f| f.label())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        MagicFilter::ALL.len(),
        "权重表里有重复的条件 —— 抽数区间会盖住后面那一格"
    );
    let sum: u32 = game_core::magic::FILTER_WEIGHTS
        .iter()
        .map(|(_, weight)| *weight)
        .sum();
    assert_eq!(
        sum,
        game_core::magic::TOTAL_WEIGHT,
        "权重总和与抽数上限漂了 —— `weighted_filter` 会喊「回落第一条」，那就是替玩家改结果"
    );
    // 「走路的人」降权那一格：它恒等于施法者自己，等权的话自我惩罚约 1/11 而不是 1/21
    let walker = game_core::magic::FILTER_WEIGHTS
        .iter()
        .find(|(f, _)| *f == MagicFilter::Walker)
        .map(|(_, weight)| *weight)
        .expect("「走路的人」该在权重表里");
    assert_eq!(walker, 1, "「走路的人」的降权被抹平了");
    assert!(
        game_core::magic::FILTER_WEIGHTS
            .iter()
            .filter(|(f, _)| *f != MagicFilter::Walker)
            .all(|(_, weight)| *weight > walker),
        "其余条件都不比「走路的人」重 —— 那就不是降权而是只留它一条"
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

// ===== 一b、摇条件（P13c 新增的那一半）=====

/// 摇出来的条件**筛得出人** —— 而重摇这一格不是理论上的：演示盘此刻就有三条筛不出人。
///
/// 这条用例的形状是三条断言一起才成立：
/// ① 第一试确实会摇到筛不出人的那几条（`OnBike`／`OnCar`／`Possessed` —— 开局没人骑车、
///    `clear_gods` 之后没人附身），否则「筛空就重摇」这句话永远不被走到，断言就是空跑；
/// ② 定下来的那一条筛得出人；
/// ③ 摇到筛不出人时**换了一条**（`first != drawn`），而不是原地把空名单交回去。
#[test]
fn a_drawn_condition_always_hits_somebody() {
    let (mut app, p0, _p1) = at_the_witch_pinned(None);
    let empty_at_the_start = [
        MagicFilter::OnBike,
        MagicFilter::OnCar,
        MagicFilter::Possessed,
    ];
    let mut redraw_fired = 0;
    for seed in 0..80 {
        let (first, (drawn, hits)) = drawn_with_seed(&mut app, seed, p0);
        assert!(
            !hits.is_empty(),
            "种子 {seed} 定下「{}」却筛不出人 —— 重摇没收住",
            drawn.label()
        );
        if empty_at_the_start.contains(&first) {
            redraw_fired += 1;
            assert_ne!(
                first,
                drawn,
                "种子 {seed}：第一试「{}」筛不出人，定下来的却还是同一条",
                first.label()
            );
        }
    }
    assert!(
        redraw_fired > 0,
        "六十次抽样里没有一次摇到筛不出人的条件 —— 那这一条什么都没验，权重表怕是空了"
    );
}

/// 同一个种子摇出同一条条件。
///
/// 这条不是形式：截图核对与 `NM_SEED` 复现靠的就是它，而「重摇」很容易写成读全局时钟或
/// 读 `frame` 那种不可复现的东西。抽数走的是 `pick_magic` 那一条子流（第 11 条）。
#[test]
fn the_same_seed_draws_the_same_condition() {
    let (mut app, p0, _p1) = at_the_witch_pinned(None);
    for seed in [3, 17, 41] {
        let (_, first) = drawn_with_seed(&mut app, seed, p0);
        let (_, again) = drawn_with_seed(&mut app, seed, p0);
        assert_eq!(
            first, again,
            "种子 {seed} 两次摇出不一样的条件 —— 复现拍图就拍不出同一次法事"
        );
    }
}

/// 「走路的人」确实降权，但**不是从表里删掉**。
///
/// 拍板的是「保留但降权」：自我惩罚那一格荒诞感要留着，而它恒等于施法者自己，等权会让
/// 大约每九场法事就有一次「女巫把玩家自己关起来」。200 个种子里它该出现（否则等价于删掉），
/// 又该**明显少于一条正常条件的两倍之一**（否则等价于等权）。
///
/// 断言是**上下界 + 一个比值**而不是一个具体分布：演示盘此刻筛不出人的那三条会被重摇替掉，
/// 所以实际比例是 1/15 而不是 1/21 —— 写死一个数就成了一条会随棋盘漂红的用例。
/// 比值那一头是这条用例的牙：真权重下「所有女生」是它的两倍（2 vs 1），等权时两者相等，
/// 而 `girls * 2 >= walker * 3` 恰好卡在中间（red-check 换回等权就是靠这一条红的）。
///
/// ⚠️ 抽样走 [`drawn_series`]（一条流连抽 600 场）而不是「600 个连续种子各抽一场」：
/// 相邻种子的第一抽是相关的，那样量出来的比例是假的 —— 第一版就是这样，1:2 抽成 21:27。
#[test]
fn the_walker_keeps_its_weight_but_the_lightest() {
    let (mut app, p0, _p1) = at_the_witch_pinned(None);
    let total = 600;
    let drawn = drawn_series(&mut app, 7, p0, total);
    assert_eq!(
        drawn.len(),
        total as usize,
        "抽样没跑够次数 —— 那下面两条比值都是空谈"
    );
    let walker = drawn.iter().filter(|f| **f == MagicFilter::Walker).count();
    let girls = drawn
        .iter()
        .filter(|f| **f == MagicFilter::AllGirls)
        .count();
    assert!(walker > 0, "「走路的人」从没被摇出来 —— 降权降成了删除");
    assert!(
        walker * 4 <= total as usize,
        "{total} 场里自我惩罚 {walker} 次，比拍板的量级高出一截 —— 权重表怕是又等权了"
    );
    assert!(
        girls * 2 >= walker * 3,
        "「所有女生」{girls} 次对「走路的人」{walker} 次 —— 比值不到 1.5，那条降权怕是已经抹平了（拍的是 1 vs 2）"
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
        "草稿得记住**是哪个条件**筛出的这份名单，那一屏才念得出女巫说的是什么"
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
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::MagicSpell(MagicFilter::OnBike))),
        "钉住的条件筛空时那一屏照开 —— 落空由施法那一步收尾（见上面坑 38 那条）"
    );

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
/// P13c 之后这里不再走真窗口：全场只剩一位不破产的人时，「所有女生」筛空 → 摇到它就重摇，
/// 而两位玩家只剩一位能行动，回合机当场判终局 —— 那一格由
/// [`a_pinned_empty_list_still_hands_the_turn_back`] 用没破产的阵容验。
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
    assert_eq!(
        hits(&mut app, MagicFilter::TopCash, p0),
        vec![p0],
        "破产之后她整个从名单上消失：「最多」的答案改到 P0 头上"
    );
    assert_eq!(
        hits(&mut app, MagicFilter::AllGirls, p0),
        vec![],
        "场上唯一的女生破产 → 「所有女生」筛空（生产路径摇到它就重摇）"
    );
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

/// 矩阵第一列：目标手里**带着免罪卡**。
///
/// 期望值就是 PLAN §三 那句话加上 10-25 的拍板：十二道里**只有**坐牢与住院挡得住，
/// 且挡住时吃掉一张；其余十道**一张不花** —— 免疫规则免的是事，不是卡。
///
/// P13b② 之后这一列多读一样东西：**门问过的那几行**。它是「哪些法术构造了 Affliction」
/// 这句话在测试里的正身 —— 以前那条证据藏在「护身层数变没变」里，而层数说不出**为什么**没变
/// （没问、问了又落、挡下来，三者在层数上都是「少一层或没少」）。
#[test]
fn a_forgive_card_in_hand_stops_only_the_two_that_lock_someone_up() {
    for spell in MagicSpell::ALL {
        let (mut app, p0, p1) = at_the_witch();
        arm(&mut app, p1, spell);
        let insulate = item_index(&app, "item_insulate").0;
        deal(&mut app, p1, insulate);
        deal(&mut app, p1, insulate);
        assert_eq!(
            forgive_in_hand(&app, p1),
            2,
            "前置：两张都该在她手里（免罪卡从此不用先打出去）"
        );

        assert_eq!(
            pick(&mut app, p0, MagicFilter::AllGirls),
            vec![p1],
            "{}：前置名单",
            spell.label()
        );
        cast(&mut app, p0, spell);

        let forgivable = matches!(spell, MagicSpell::Jail | MagicSpell::Hospital);
        // 「抽命运 3 张」是**间接效果**：这一道本身挡不住（三张真的抽了，名单里仍有她），
        // 而她抽来的那张 fate_001「直接入狱」走的正是命运那条路 —— 而那条路认免罪卡。
        // PLAN §三 那句「受神明修正」说的是同一条通道：修正与抵消都落在**牌**上，
        // 不落在女巫身上。
        let indirect = spell == MagicSpell::DrawFate;
        // 每一问都答「用」。被问过几行、问的是哪一行，就是这一列的真正读数
        let asked = ask_the_gate(&mut app, PassiveMove::Use);
        assert_eq!(
            asked.len(),
            usize::from(forgivable) + usize::from(indirect),
            "{}：问了 {} 句，而该问的是{}句 —— 门多问一句就是替女巫发明了一条免疫，\
             少问一句就是那张卡白躺在手里",
            spell.label(),
            asked.len(),
            usize::from(forgivable) + usize::from(indirect)
        );
        // 门花的卡数才是这一列的读数，而不是「她手里还剩几张」。理由：十二道里有一道
        // （变卖所有卡）自己就把整只手清空了，按张数断言等于把法术自己的账算到门头上 ——
        // 症状会是「门没花卡而那条断言红了」。花的每一张都发一条 `PassiveUsed`，按它数。
        assert_eq!(
            gate_spends(&app),
            usize::from(forgivable) + usize::from(indirect),
            "{}：{} —— 门花的卡数",
            spell.label(),
            if forgivable {
                "挡下了，该吃掉一张"
            } else if indirect {
                "吃掉一张的是抽来的那张牌"
            } else {
                "不吃卡，一张都不该花"
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
                "抽来的那张「直接入狱」被她手里那张卡挡了 —— 这就是『间接效果』四个字的形状"
            );
        }
    }
}

/// 矩阵第二列：目标是**大福神**附身（厄运豁免）。十二道**全部照常生效**。
///
/// 这一列才是这一阶段真正的难点，而它的答案在**构造方式**上：女巫那几条挨得人事的法术
/// 今天照旧进门，而门把神明那一格钉成「不吃」—— 判据挂在**事**上而不是来路上
/// （[`is_ritual`](game_core::afflict::Affliction::is_ritual) 为真的那四条既不挨大福神的免疫，
/// 也不挨大衰神的加倍），刻意不接 `deck::resist` 里的那位 —— 祂豁免的是厄运，女巫的法术不是厄运。
/// 所以这里既没有一条查询去「逐条挡」，也没有一处特例。
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

/// 门问完之后**没有第二问**：一张卡答一句，答完那一格就空了。
///
/// ① 之前这条用例数的是「耗尽的护身组件要被摘掉」—— 那个组件从此不存在，而它留下的
/// 那句话仍然要说得出：面板上不许有一个「手里有免罪卡却谁也不挡」的状态，
/// 而现在那个状态的替身就是**门里挂着一句没人能答的第二问**。
#[test]
fn the_gate_asks_once_and_then_is_empty() {
    let (mut app, p0, p1) = at_the_witch();
    let insulate = item_index(&app, "item_insulate").0;
    deal(&mut app, p1, insulate);

    assert_eq!(pick(&mut app, p0, MagicFilter::AllGirls), vec![p1]);
    cast(&mut app, p0, MagicSpell::Jail);
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::JailTrip)),
        "前置：门真的在问她"
    );

    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert_eq!(gate_asking(&app), None, "答完不该留下第二问");
    assert_eq!(gate_depth(&app), 0, "信封也要一起走空");
    assert_eq!(forgive_in_hand(&app, p1), 0, "那一张花掉了");
    assert!(was_blocked(&app, MagicSpell::Jail, "P1"));
    assert!(detention_of(&app, p1).is_none());
}

/// 卡是**个人的**事：同一批里没挡住的那个人照样挨，包括施法者自己。
///
/// 这一条在门上的形状比旧写法更硬：一次法事筛出两个人，门问的是**两个人各一次**，
/// 而每次的主语都不同 —— 这正是「决策主体能在回合内跳动」（PLAN §五 第 2 条）的最小样本。
#[test]
fn a_forgive_card_protects_its_own_holder_only() {
    let (mut app, p0, p1) = at_the_witch();
    let insulate = item_index(&app, "item_insulate").0;
    deal(&mut app, p1, insulate);
    // 「所有女生」在她这儿只剩她一个，所以把施法者也改成女生才能一次点到两个人
    insert(&mut app, p0, (Gender::Female,));

    let hits = pick(&mut app, p0, MagicFilter::AllGirls);
    assert_eq!(hits, vec![p0, p1], "前置：两人都是女生，回合序 P0 在前");
    cast(&mut app, p0, MagicSpell::Jail);

    // 第一问落在 P0 身上 —— 他自己没有卡，所以门**根本不该问他**
    assert_eq!(
        gate_asking(&app),
        Some((p1, Ask::Offset, PassiveEvent::JailTrip)),
        "名单里只有拿着卡的那一位该被问"
    );
    answer_gate(&mut app, p1, PassiveMove::Use);
    settle(&mut app, 2);

    assert!(
        detention_of(&app, p0).is_some(),
        "他自己没有免罪卡，就该被自己的法术关进去"
    );
    assert!(
        detention_of(&app, p1).is_none(),
        "她手里那一张挡住了她那一份"
    );
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
