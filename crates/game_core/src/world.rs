//! 对局世界：棋盘静态定义 + 由它派生的实体索引，以及开局装配流程。

use std::collections::HashMap;

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::BoardDef;
use crate::calendar::{Date, EPOCH_YEAR, EpochOverride, HolidaySet};
use crate::card::{CardKind, CardSet};
use crate::charge::Corrections;
use crate::coupon::CouponBoxes;
use crate::data::DataHandles;
use crate::deck::Deck;
use crate::land::{self, LandId};
use crate::market::MarketClock;
use crate::player::{self, GameSetup};
use crate::rng::{GameSeed, Rng};
use crate::rules::GameRules;
use crate::shop::ShopStock;
use crate::stock::{PriceHistory, StockId, StockPrice, StockSet};
use crate::turn::{GameSession, TurnOrder, TurnPhase};

/// 规则层为「这一局」spawn 的实体都带这个标记，返回主菜单时按它一次性清场。
///
/// 不清行的后果不是内存泄漏而是**逻辑错误**：`begin_game_system` 重开一局会 spawn 新的
/// 玩家/土地实体，而 `Query<_, With<PlayerId>>` 会同时查到新旧两批 —— 一局里凭空多出两个人，
/// 利息与分红付给死人，回合序也乱。标记由 spawn 方负责，清场方因此不需要知道有任何实体类型。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gameplay;

/// 规则层里需要被表现层门控的 system 组。
///
/// 只把 `begin_game_system` 圈出来：装配一旦被 `Scene::Menu` 挡住，`TurnPhase` 就停在 `Boot`，
/// 后面所有 `OnEnter(TurnStart)`/`in_state(WaitRoll)` 的 system 自然都不会跑 ——
/// 一个门就够，不必给整台状态机套条件。
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CoreSet {
    Boot,
}

/// 取代 Godot 的 `GameState.board`（`game_state.gd:24`）。
///
/// `def` 是加载后的一次性快照而不是 `Handle`：规则 system 直接 `Res<ActiveBoard>` 就能拿拓扑，
/// 不必每处再解一遍 `Assets<BoardDef>`，headless 测试也只插这一个 resource 就能跑。
///
/// 只有**土地**建实体（要挂归属/等级/抵押这些运行时状态）；**道路格不建实体** —— 它没有任何
/// 可变状态，`BoardDef` 的拓扑查询就是它的接口，再复制一份 component 只会多出第二个真相来源。
/// 表现层为了拾取/渲染自己 spawn 的网格实体归 `game_app` 管，与规则层无关。
#[derive(Resource, Debug)]
pub struct ActiveBoard {
    pub def: BoardDef,
    pub lands: HashMap<LandId, Entity>,
}

/// 开局要读的四份资产（棋盘 + 两副牌 + 股票）。
///
/// 打包成系统参数而不是给 `begin_game_system` 挂 `#[allow(clippy::too_many_arguments)]`：
/// 后者会把「参数确实太多了」这个真信号吃掉（`.agents/ARCHITECTURE.md` §六 坑 8）。
#[derive(SystemParam)]
pub struct BootAssets<'w> {
    data: Res<'w, DataHandles>,
    boards: Res<'w, Assets<BoardDef>>,
    card_sets: Res<'w, Assets<CardSet>>,
    stock_sets: Res<'w, Assets<StockSet>>,
    holidays: Res<'w, Assets<HolidaySet>>,
}

/// 开局要读的配置打包成一个系统参数（阵容、数值、种子、历法锚点）。
/// `pub` 是必须的：`begin_game_system` 是公开 system，参数类型比它更私有条目会报错。
#[derive(SystemParam)]
pub struct BeginCfg<'w> {
    rules: Res<'w, GameRules>,
    setup: Res<'w, GameSetup>,
    seed: Res<'w, GameSeed>,
    epoch: Res<'w, EpochOverride>,
}

/// 四类数据资产全部就位后一次性装配世界：校验 → spawn 土地/玩家/股票/牌堆实体 →
/// 插入 `ActiveBoard`/`TurnOrder`/`Rng`/`GameSession`，然后把状态机交给 `TurnStart`。
///
/// 挂在 `in_state(TurnPhase::Boot)` 上；`Option<Res<ActiveBoard>>` 的守门仍然保留 ——
/// 状态转移要到下一帧才生效，而本帧内不该重复铺实体（0.20 没有 `resource_missing` 运行条件，
/// `Option<Res<T>>` 的写法语义更直白）。
///
/// B3 起卡牌与股票也是运行时的依赖（牌堆要洗牌、市场要建实体），所以这里等的是**全部四份**
/// 资产，不再只等棋盘。校验也收到这里：数据错了就 panic，无头测试与真实客户端走同一条路，
/// 而不是只有开窗那一侧会响。
pub fn begin_game_system(
    mut commands: Commands,
    active: Option<Res<ActiveBoard>>,
    assets: BootAssets<'_>,
    cfg: BeginCfg,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    if active.is_some() {
        return;
    }
    let (Some(def), Some(chance), Some(fate), Some(items), Some(stocks), Some(holidays)) = (
        assets.boards.get(&assets.data.board).cloned(),
        assets.card_sets.get(&assets.data.chance),
        assets.card_sets.get(&assets.data.fate),
        assets.card_sets.get(&assets.data.items),
        assets.stock_sets.get(&assets.data.stocks).cloned(),
        assets.holidays.get(&assets.data.holidays).cloned(),
    ) else {
        return;
    };
    for (what, errs) in [
        ("棋盘", def.validate()),
        ("机会牌", chance.validate(CardKind::Chance)),
        ("命运牌", fate.validate(CardKind::Fate)),
        ("道具牌", items.validate(CardKind::Item)),
        ("股票", stocks.validate()),
        ("节假日表", holidays.validate()),
    ] {
        if let Err(errs) = errs {
            panic!("{what}校验失败:\n  {}", errs.join("\n  "));
        }
    }

    // 洗牌消耗的是**牌堆子流**，骰子流一位没动 —— 所以「对局第一手点数 ==
    // `Rng::new(seed).roll_die()`」这条 B2 的断言在 B3 之后依然成立（见 rng.rs）。
    // 历法锚点走自己的子流：同一种子随机到同一个月，且不影响牌序与骰点。
    let mut rng = Rng::new(cfg.seed.0);
    let epoch = cfg
        .epoch
        .0
        .unwrap_or_else(|| Date::new(EPOCH_YEAR, rng.pick_epoch_month(), 1));
    let lands = land::spawn_lands(&mut commands, &def);
    let players = player::spawn_players(&mut commands, &cfg.setup, &cfg.rules);
    spawn_stocks(&mut commands, &stocks);
    for deck in [
        Deck::dealt(CardKind::Chance, chance, &mut rng),
        Deck::dealt(CardKind::Fate, fate, &mut rng),
    ] {
        commands.spawn((deck, Gameplay));
    }
    // 路上的宝箱（P5）。位置走专用子流，所以「宝箱在哪」不改变骰点、牌序与行情。
    let chests = crate::occupancy::spawn_starting_chests(&mut commands, &def, &mut rng);

    info!(
        "世界装配完成：棋盘「{}」{} 主环格 / {} 土地实体 / {} 玩家 / {} 只股票 / 两副牌（{}+{}）+ 货架 {} 张 + 宝箱 {} 个（起始现金 {}）· 开局 {epoch}",
        def.board_name,
        def.loop_len(),
        lands.len(),
        players.len(),
        stocks.0.len(),
        chance.0.len(),
        fate.0.len(),
        items.0.len(),
        chests.len(),
        cfg.rules.start_cash
    );
    match holidays.coverage() {
        Some((first, last)) => {
            warn!("节假日表覆盖 {first}–{last}：表外年份农历三节与清明不放假，公历四条照常")
        }
        None => warn!("节假日表为空：所有年份都只按公历四条放假"),
    }

    commands.insert_resource(ActiveBoard { def, lands });
    commands.insert_resource(TurnOrder::new(players));
    commands.insert_resource(rng);
    // 节假日是这一局的静态日历事实：克隆成资源，规则层与 UI 都直接读，
    // 免得每处再解一遍 `Assets`。装配失败前它不存在，于是所有读者都躲在状态门禁后面。
    commands.insert_resource(holidays);
    commands.insert_resource(GameSession {
        turn_count: 0,
        seed: cfg.seed.0,
        price_index: 1.0,
        epoch,
        last_gift_year: None,
        last_envelope_year: None,
    });
    commands.insert_resource(MarketClock::default());
    // 修正登记处是**这一局**的账：上一局挂着的免罪卡若不清，新局第一笔租金就会静默归零
    // （坑 22/25 同一族，所以装配世界时重置而不是等谁去撤销）
    commands.insert_resource(Corrections::default());
    // 点券箱的「谁被取走过」是**这一局**的事实：上一局捡空的格子必须在新局重新长出来
    // （B5 验收那批跨局残留缺陷的同一族，见 ARCHITECTURE §6.1 坑 22/25）
    commands.insert_resource(CouponBoxes::default());
    // 货架同理：上一局被买空的货位不属于这一局。它本来每次进店都重抽，所以漏掉这一条
    // **不会每次都错** —— 只在「新局第一次踩百货之前」那几帧里，面板能读到上一局剩下的货
    commands.insert_resource(ShopStock::default());
    next.set(TurnPhase::TurnStart);
}

/// 每只股一个实体：现价从 `base_price` 起步，历史先记这一条
/// （`stock_market.gd:26-28` 的 `add_stock` 就是这么初始化 `price_history` 的）。
fn spawn_stocks(commands: &mut Commands, stocks: &StockSet) {
    for s in &stocks.0 {
        let mut history = std::collections::VecDeque::new();
        history.push_back(s.base_price);
        commands.spawn((
            StockId(s.id.clone()),
            StockPrice(s.base_price),
            PriceHistory(history),
            Gameplay,
        ));
    }
}
