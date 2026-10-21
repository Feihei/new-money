//! 手牌浮层（P3b）：拿在手里的卡，与「打出去」那几颗按钮。
//!
//! 与 ATM/证券同一类（见 [`super::finance`] 的模块头）：**不阻断回合**、内容跟着世界状态
//! 刷新、窗口由判据给。出牌与下单共用同一条窗口（`WaitRoll` 且没被模态占住），UI 这半边
//! 是 [`super::can_play`]，规则层那半边是 `game_core::hand::PlayWindow::opener`。
//!
//! 每张卡都带效果文案与目标种类 —— 只有卡名读不出规则。打出去之后的结果走顶栏那条事件
//! 流水（`top_bar::hint_from_events` 读 `CardPlayed`），面板自己不留痕：面板随手牌重建，
//! 把一次性结果烤进去，下一次重建就没了。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, Bankrupt, Books, CardAim, CardDef, CardGained, CardId, CardKind, CardTarget,
    GameRules, God, Hand, Insulation, LandId, Paused, PendingDecision, PlayerId, PlayerName,
    Possession, Shelf, TurnOrder, TurnPhase, deal_to, resists,
};

use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, PRIMARY, SECONDARY, UiAction, border_color, button, can_play, label,
};
use crate::render::DevFont;

/// 手牌浮层开着吗。顶栏那颗按钮的唯一记忆。
#[derive(Resource, Default, Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct HandPanelOpen(pub bool);

impl HandPanelOpen {
    /// `NM_HAND=1`：开面板，并由 [`dev_deal_sample`] 发五种形状的样例卡。
    /// 道具卡的正经来源是百货公司柜台（P3d②），而「踩到哪一格」「攒够点券没有」都不由
    /// 截图脚本控制 —— 没有这个旋钮，这块面板在截图里永远是空的，
    /// 「每个界面都得有一条不靠鼠标的入口」那条判据落不了地。
    pub(crate) fn from_env() -> Self {
        Self(std::env::var_os("NM_HAND").is_some())
    }
}

#[derive(Component)]
pub(crate) struct HandPanel;

/// 面板根上的「上次写成什么样」。键挂实体（同 `finance::FinanceStamp`，§6.1 坑 22），
/// 回主菜单时随实体一起消失，不会让下一局的第一次刷新误判成「没变化」。
#[derive(Component, Debug, Clone, PartialEq, Eq)]
pub(crate) struct HandStamp(pub String);

/// 样例卡。按 id 找而不是按数组位置：改数据顺序不该让这个旋钮发出别的卡。
///
/// 每种出牌形状各一张：给自己（免罪）、存着待用（免费）、给对手（陷害）、给一条街
/// （涨价 / 查封）、给一块无主地（购地）、给一个人含自己（转向、停留）。
/// 少一种，那个形状的按钮排就永远拍不到 —— 而「拍不到」在验收里读起来和「没做」一样。
const SAMPLE_CARDS: [&str; 8] = [
    "item_frame",
    "item_insulate",
    "item_hike",
    "item_seal",
    "item_free",
    "item_grab",
    "item_turn",
    "item_stay",
];

/// 开发用：给当前回合玩家发 [`SAMPLE_CARDS`] —— 一张要选对手、一张给自己，两种形状都要拍到。
///
/// 走 [`deal_to`] 而不是直改 `Hand`：上限与「被挤掉要发流水」两条规则本来就在它里面，
/// 绕过它发牌就是把 P3d 的百货账目测了个寂寞。`Local` 保证只发一次，否则每帧发一轮牌。
pub(crate) fn dev_deal_sample(
    order: Option<Res<TurnOrder>>,
    mut hands: Query<&mut Hand, With<PlayerId>>,
    shelf: Shelf<'_>,
    rules: Res<GameRules>,
    mut gained: MessageWriter<CardGained>,
    mut dealt: Local<bool>,
) {
    if *dealt {
        return;
    }
    let Some(player) = order.as_ref().and_then(|o| o.current()) else {
        return;
    };
    let Ok(mut hand) = hands.get_mut(player) else {
        return;
    };
    for id in SAMPLE_CARDS {
        let Some(index) = shelf
            .items()
            .iter()
            .position(|c| c.id == id)
            .map(|at| CardId(at as u32))
        else {
            warn!("NM_HAND 要发的 {id} 不在货架上");
            continue;
        };
        deal_to(
            &mut hand,
            rules.hand_limit,
            index,
            player,
            &shelf,
            &mut gained,
        );
    }
    *dealt = true;
}

/// 「一张卡在该长什么样子」要读的那几样：货架、手牌、护身、别人还在不在、地块表（走账本）。
///
/// 地块那一读**刻意不查组件**：`Books` 里的地块查询带着 `&mut Owner`，与面板上另开一条
/// `Query<&Owner>` 在同一条 system 里就是访问冲突（Bevy 会在运行时撞车）。少一条查询的
/// 另一个好处是归属只有一份真相 —— 涨价卡的候选排与购地卡的候选排都从 `books.views()` 数出来。
#[derive(SystemParam)]
pub(crate) struct HandView<'w, 's> {
    open: Res<'w, HandPanelOpen>,
    paused: Res<'w, Paused>,
    pending: Res<'w, PendingDecision>,
    phase: Res<'w, State<TurnPhase>>,
    order: Option<Res<'w, TurnOrder>>,
    shelf: Shelf<'w>,
    /// 地块类卡的候选要按棋盘定义出名字，而棋盘是**可选**资源：菜单态没有它，
    /// 这块面板那时也不该在（`UiSet::Live` 的门挡着），拿不到时按「没有地」画
    board: Option<Res<'w, ActiveBoard>>,
    hands: Query<'w, 's, &'static Hand, With<PlayerId>>,
    shields: Query<'w, 's, &'static Insulation>,
    /// 身上挂着哪位神（P6b②）。护身那一行有**两个来源**，少读一个就会在玩家其实免疫的时候
    /// 只念出卡上的层数（§6.1 坑 30 同一条：可见位要有它的全部判据）。
    possessions: Query<'w, 's, &'static Possession>,
    /// 地块表 + 「这块无主地买得下来吗」的那句判据（[`Books::buy_price`]）。
    /// 按钮上的价不在这里另算：修正登记处（小福神买地免费那一类）与物价指数只住在账本
    /// 那一侧，抄一份就迟早分叉成「按钮写着 1,200、实际扣 2,400」。
    /// 带 `Books` 的 system 都躲在 `UiSet::Live` 那道 `resource_exists::<ActiveBoard>`
    /// 门后面（同 `dialog::sync_modal`）。
    books: Books<'w, 's>,
    faces: Query<
        'w,
        's,
        (
            Entity,
            &'static PlayerId,
            &'static PlayerName,
            Option<&'static Bankrupt>,
        ),
    >,
}

/// 一颗「用这张卡」的按钮。`aim` 是烤进按钮的目标 —— 那颗按钮本身就是一整个意图，
/// 规则层因此不必挂一个「选目标」的待决事项（同 ATM 那颗「存 1,000」）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PlayButton {
    pub text: String,
    pub aim: Option<CardAim>,
    /// 按不动时要说的话。**灰按钮必须自带一句为什么**，否则「按不动」与「坏了」在玩家
    /// 眼里是同一件事（§6.1 坑 27 的 UI 版）。
    pub blocked: Option<&'static str>,
}

/// 地块类卡的一颗候选按钮。`label` 由 [`sync_hand_panel`] 拼好（「谁的 + 地名」与「地名 +
/// 实付价」要读名字、归属与账本，那几样只在那里有），`play_buttons` 只管形状不管世界。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LandPick {
    pub label: String,
    pub land: LandId,
}

/// 这张卡该长出哪几颗按钮。`opponents` 是「还活着的别人」（不含出牌人自己），
/// `anyone` 是「还活着的人，含自己」（转向 / 停留这两张点的是人，不挑敌我），
/// `lands` 是「此刻真有人持有的地块」（无主地不在里面 —— 规则层也不认，同一句判据），
/// `parcels` 是「此刻**买得下来**的无主地」（买不起的不在里面，同一句判据的另一半）。
///
/// 写成对 `CardTarget` 的穷尽匹配、不留 `_`：新增一种目标时必须当场决定它长什么样，
/// 而不是等截图时才发现有颗哑弹。
/// `parcels` 是「此刻**买得下来**的无主地」（买不起的不在里面，同一句判据的另一半），
/// `denied` 是「他身上有衰神／死神，置产这一族的事一件也办不成」（`Books::acquire_denied`）。
/// 两者都能让那一排空掉，而玩家要听到的是一句不同的话：「没有买得起的无主地」在
/// 必败那一位身上是一句**假话**——地是无主的、钱也是够的。
pub(crate) fn play_buttons(
    def: &CardDef,
    opponents: &[(Entity, String)],
    anyone: &[(Entity, String)],
    lands: &[LandPick],
    parcels: &[LandPick],
    denied: bool,
) -> Vec<PlayButton> {
    match def.target {
        CardTarget::SelfOnly => vec![PlayButton {
            text: "用".into(),
            aim: None,
            blocked: None,
        }],
        CardTarget::Player if opponents.is_empty() => {
            // 其他人都离场了。这时候按下去规则层会静默拒绝（`aim_is_legal`），
            // 而玩家看得见的应该是一句话，不是一颗死按钮
            vec![PlayButton {
                text: "用".into(),
                aim: None,
                blocked: Some("场上没有别的人了"),
            }]
        }
        CardTarget::Player => opponents
            .iter()
            .map(|(target, name)| PlayButton {
                text: format!("→ {name}"),
                aim: Some(CardAim::Player(*target)),
                blocked: None,
            })
            .collect(),
        // 「一个人，含自己」。这一排正常情况下至少有「自己」那颗，所以它没有灰按钮那一格；
        // 但空排照样要写出一句话 —— 一排什么都不长的卡行，读起来跟坏了没区别
        CardTarget::Anybody if anyone.is_empty() => vec![PlayButton {
            text: "用".into(),
            aim: None,
            blocked: Some("现在没有可行动的人"),
        }],
        CardTarget::Anybody => anyone
            .iter()
            .map(|(target, name)| PlayButton {
                text: format!("→ {name}"),
                aim: Some(CardAim::Player(*target)),
                blocked: None,
            })
            .collect(),
        CardTarget::Land if lands.is_empty() => vec![PlayButton {
            text: "用".into(),
            aim: None,
            blocked: Some("还没有人有地"),
        }],
        CardTarget::Land => lands
            .iter()
            .map(|pick| PlayButton {
                text: format!("→ {}", pick.label),
                aim: Some(CardAim::Land(pick.land)),
                blocked: None,
            })
            .collect(),
        // 这句说的是「买得起的无主地」而不是「没有无主地」：候选排是账本筛出来的，
        // 面板只读得到「筛完是空」，读不到空在哪一头。反过来写就是一句假话
        // （地全有主时同样「没有买得起的无主地」，那句话倒是永远成立）
        //
        // 而「空」现在有**两个**来源：钱不够，与身上那位衰神／死神让整族事办不成
        // （P6b② `Books::acquire_denied`）。两者在候选排上长得一模一样，所以那句
        // 为什么必须分开说 —— 对附身的人念「没有买得起的无主地」是句假话：
        // 地是无主的，钱也是够的，缺的是「这件事今天不给做」
        CardTarget::UnownedLand if denied => vec![PlayButton {
            text: "用".into(),
            aim: None,
            blocked: Some("那位神让买地必败"),
        }],
        CardTarget::UnownedLand if parcels.is_empty() => vec![PlayButton {
            text: "用".into(),
            aim: None,
            blocked: Some("没有买得起的无主地"),
        }],
        CardTarget::UnownedLand => parcels
            .iter()
            .map(|pick| PlayButton {
                text: format!("→ {}", pick.label),
                aim: Some(CardAim::Land(pick.land)),
                blocked: None,
            })
            .collect(),
    }
}

/// 目标种类的人话。数据里写的是 `target: "player"`，屏幕上要读得出「这是对谁用的」。
fn aim_text(def: &CardDef) -> &'static str {
    match def.target {
        CardTarget::SelfOnly => "对自己",
        CardTarget::Player => "对一名对手",
        CardTarget::Anybody => "对一个人",
        CardTarget::Land => "对一块地",
        CardTarget::UnownedLand => "对一块无主地",
    }
}

/// 护身那一行。没挂护身就不写这一句；0 层也不写 —— 规则层在用完那一层就把组件
/// 摘了，这里再兜一次，免得面板说出「免罪护身 0 层」这种自相矛盾的话。
///
/// `god` 是那位神给的那半（P6b② 的大福神／天使＝厄运豁免）。它**不在这个组件里**
/// （见 `game_core::afflict::Insulation` 的注），所以这里要把两半一起念，否则面板会在
/// 玩家其实免疫的时候只写「护身 2 层」—— 那句话说的是卡，而玩家问的是「为什么这次没关进去」。
/// 两半都在时那句必须说清是两层**加**免疫：免疫期间卡上的层数不会被吃掉，这是玩法信息。
fn shield_text(shield: Option<&Insulation>, god: Option<God>) -> Option<String> {
    let immune = god.is_some_and(resists);
    let charges = shield.map(|s| s.charges).filter(|n| *n > 0);
    match (immune, charges) {
        (true, Some(n)) => Some(format!("护身：免疫（另有卡 {n} 层）")),
        (true, None) => Some("护身：免疫".into()),
        (false, Some(n)) => Some(format!("护身 {n} 层")),
        (false, None) => None,
    }
}

/// 手牌浮层：每张卡三行（卡名+种类 / 效果 / 按钮），末尾一个关闭。
pub(crate) fn sync_hand_panel(
    mut commands: Commands,
    font: Res<DevFont>,
    view: HandView<'_, '_>,
    shown: Query<(Entity, &HandPanel, &HandStamp)>,
) {
    if !view.open.0 {
        for (entity, _, _) in &shown {
            commands.entity(entity).despawn();
        }
        return;
    }
    let player = view.order.as_ref().and_then(|o| o.current());
    let playable = player.is_some_and(|p| {
        can_play(*view.phase.get(), &view.paused, &view.pending) && same_actor(p, &view.pending)
    });
    // 迭代顺序是生成顺序，而断言与截图不该依赖它：按名字排（同证券按 id 排）
    let mut opponents: Vec<(Entity, String)> = view
        .faces
        .iter()
        .filter(|(e, _, _, bankrupt)| Some(*e) != player && bankrupt.is_none())
        .map(|(e, _, name, _)| (e, name.0.clone()))
        .collect();
    opponents.sort_by(|a, b| a.1.cmp(&b.1));

    // 「一个人，含自己」（转向 / 停留）。自己排最前面而不是混进按名字排的那一串：
    // 这两张卡最常见的用法就是对自己用，而「→ 自己」读起来比「→ 阿土伯」少一次迟疑。
    // 破产的人照样挡（`aim_is_legal` 同一句判据），但**自己**即便已破产也还在这一排里 ——
    // 那种情况下根本没有出牌的窗口，这里不值得为它编一个假名字
    let mut anyone: Vec<(Entity, String)> = Vec::new();
    if let Some(me) = player {
        anyone.push((me, "自己".into()));
    }
    anyone.extend(opponents.iter().cloned());

    // 地块类卡的候选 = 地块表里两类各一排：涨价/查封数「此刻真有人持有的地」，购地数
    // 「此刻买得下来的无主地」。两排都从账本数出来（`Books::views()` / `Books::buy_price`），
    // 而不是面板自己查组件或自己乘物价 —— 与规则层同一句判据，所以「按钮亮着却被拒」
    // 与「按钮上的价不是扣掉的价」都不会发生。
    // label 里带上业主，而且自己的地写「我的」：涨价是往自己街上加钱、查封是往别人
    // 街上贴条子，这两件事按错一个方向就是完全相反的意图，玩家一眼就得看出来。
    let mut lands: Vec<LandPick> = Vec::new();
    let mut parcels: Vec<LandPick> = Vec::new();
    let mine = player
        .and_then(|p| view.faces.get(p).ok())
        .map(|(_, pid, _, _)| *pid);
    // 「买不下来」有两个来源，而候选排只把两者都收成空排，所以这里单独问一句、
    // 让灰按钮那句为什么说得准。读的仍是账本那一句判据（`Books::acquire_denied`），
    // 面板不查 `Possession` —— 与 `buy_price` 那条「报价与成交共读一句」同一条纪律
    let denied = player.is_some_and(|p| view.books.acquire_denied(p));
    if let Some(board) = view.board.as_deref() {
        for plot in view.books.views() {
            let Some(def) = board.def.land(plot.id.0) else {
                continue;
            };
            match plot.owner {
                Some(owner) => {
                    let who = view
                        .faces
                        .iter()
                        .find(|(_, pid, _, _)| **pid == owner)
                        .map(|(_, _, name, _)| name.0.clone());
                    // 业主查不到 = 数据里的归属指向一个不在场的人，宁可不列也不编一个名字
                    let Some(who) = who else { continue };
                    let label = if mine == Some(owner) {
                        format!("我的{}", def.name)
                    } else {
                        format!("{who}的{}", def.name)
                    };
                    lands.push(LandPick {
                        label,
                        land: plot.id,
                    });
                }
                // 无主地那颗按钮要写实付价：玩家按下去之前唯一需要判断的就是这个数，
                // 而物价 2 倍时「南京路」三个字底下藏着两个不同的价
                None => {
                    let Some(price) = player.and_then(|p| view.books.buy_price(p, plot.id)) else {
                        continue;
                    };
                    parcels.push(LandPick {
                        label: format!("{} ${price}", def.name),
                        land: plot.id,
                    });
                }
            }
        }
    }
    lands.sort_by(|a, b| a.label.cmp(&b.label));
    parcels.sort_by(|a, b| a.label.cmp(&b.label));

    let hand: Vec<CardId> = player
        .and_then(|p| view.hands.get(p).ok())
        .map(|h| h.cards.iter().copied().collect())
        .unwrap_or_default();
    let shield = shield_text(
        player.and_then(|p| view.shields.get(p).ok()),
        player
            .and_then(|p| view.possessions.get(p).ok())
            .map(|held| held.god),
    );
    // 签名要覆盖面板上每一个由判据决定的可见位（§6.1 坑 30：少一位就是一次静默过期）。
    // `lands` 那一位是 P3c 加上的：地块卡的按钮排跟着「谁买了哪块地」变，
    // 少了它，玩家刚买下的那块地不会出现在按钮排里，而下一次别的原因重建面板时才冒出来。
    // `parcels` 是同一件事的购地卡那一半，而且它比 `lands` 更脆：那一排连**现金**都读
    // （花过一笔钱、物价跳一格，候选排就变了）。
    // `anyone`（转向/停留那一排）不另开一位：它就是「自己 + opponents」，
    // 那两样已经分别是签名的第一段与第二段 —— 多写一位只会让签名有个没人读的字段
    let stamp = format!(
        "{}|{playable}|{}|{}|{}|{}|{}",
        player.map_or_else(|| "-".into(), |p| format!("{p:?}")),
        opponents
            .iter()
            .map(|(_, name)| name.as_str())
            .collect::<Vec<_>>()
            .join(","),
        hand.iter()
            .map(|c| c.0.to_string())
            .collect::<Vec<_>>()
            .join(","),
        shield.clone().unwrap_or_default(),
        lands
            .iter()
            .map(|p| p.label.as_str())
            .collect::<Vec<_>>()
            .join(","),
        parcels
            .iter()
            .map(|p| p.label.as_str())
            .collect::<Vec<_>>()
            .join(","),
    );
    if shown.iter().next().is_some_and(|(_, _, s)| s.0 == stamp) {
        return;
    }
    for (entity, _, _) in shown.iter() {
        commands.entity(entity).despawn();
    }
    // 下标与卡定义配成对再建面板：`None` 的那些是「数据里没有这张卡」，
    // 它要在屏幕上留名，而不是悄悄少一行 —— 少一行是唯一线索，而那读不出为什么
    let rows: Vec<(CardId, Option<&CardDef>)> = hand
        .iter()
        .map(|id| (*id, view.shelf.get(CardKind::Item, *id)))
        .collect();

    let mut root = commands.spawn((
        BoardUi,
        HandPanel,
        HandStamp(stamp),
        // 根节点参与命中测试的话，整块面板会把背后的点格点击吃掉（同顶栏那一坑）
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            right: Val::Px(layout::HAND_PANEL[1]),
            bottom: Val::Px(layout::HAND_PANEL[2]),
            width: Val::Px(layout::HAND_PANEL[0]),
            flex_direction: FlexDirection::Column,
            row_gap: Val::Px(layout::HAND_ROW_GAP),
            padding: UiRect::axes(Val::Px(16.0), Val::Px(14.0)),
            border: UiRect::all(Val::Px(layout::PANEL_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::PANEL_RADIUS)),
            ..default()
        },
        BackgroundColor(col::PANEL),
        border_color(col::BORDER_HAND),
        ZIndex(4),
    ));
    root.with_children(|p| {
        p.spawn(label(
            &font.0,
            format!("手牌 {}", rows.len()),
            size::NAME,
            col::TITLE_GOLD,
        ));
        // 窗口判据写在标题下面那一行：牌按不动时，玩家得先看见「为什么现在不能出」
        let mut header = if playable {
            "掷骰前可出一张".to_string()
        } else {
            "只在掷骰前可出牌".to_string()
        };
        if let Some(line) = &shield {
            header.push_str(" · ");
            header.push_str(line);
        }
        p.spawn(label(
            &font.0,
            header,
            size::ROW,
            if playable { col::MUTED } else { col::BANKRUPT },
        ));
        if rows.is_empty() {
            p.spawn(label(&font.0, "手里没有卡", size::BODY, col::KEY));
        }
        for (card, def) in &rows {
            let Some(def) = def else {
                p.spawn(label(
                    &font.0,
                    format!("数据错误：货架上没有第 {} 张", card.0),
                    size::ROW,
                    col::BANKRUPT,
                ));
                continue;
            };
            let buttons = play_buttons(def, &opponents, &anyone, &lands, &parcels, denied);
            // 三行分开：合成一行的话，两张卡名加两颗按钮在 1024 宽的屏上就顶到边了
            let mut title = p.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(8.0),
                ..default()
            });
            title.with_children(|line| {
                line.spawn((
                    Node {
                        flex_grow: 1.0,
                        ..default()
                    },
                    label(&font.0, def.title.clone(), size::NAME, col::HINT),
                ));
                line.spawn(label(&font.0, aim_text(def), size::ROW, col::KEY));
            });
            p.spawn(label(&font.0, def.desc.clone(), size::ROW, col::MUTED));
            let mut actions = p.spawn(Node {
                flex_direction: FlexDirection::Row,
                // 允许换行：地块卡在一盘后期能长出七八颗按钮（每块有主的地一颗），
                // 不换行就会顶穿面板右边界，而那颗被切掉的按钮读起来像「没有这个功能」
                flex_wrap: FlexWrap::Wrap,
                column_gap: Val::Px(8.0),
                row_gap: Val::Px(6.0),
                align_items: AlignItems::Center,
                ..default()
            });
            // 地块那颗按钮上要写「谁的 + 地名」或「地名 + 价」，比「用」与「→ 名字」长一截：
            // 单独给它一个宽度与字号，而不是把三种形状挤进同一个 96 像素
            let (box_size, box_font) =
                if matches!(def.target, CardTarget::Land | CardTarget::UnownedLand) {
                    (layout::LAND_BUTTON, size::ROW)
                } else {
                    (layout::CARD_BUTTON, size::BODY)
                };
            actions.with_children(|line| {
                for pb in &buttons {
                    button(
                        line,
                        &font.0,
                        ButtonSpec {
                            font: box_font,
                            ..ButtonSpec::new(
                                &pb.text,
                                box_size,
                                PRIMARY,
                                UiAction::PlayCard {
                                    card: *card,
                                    aim: pb.aim,
                                },
                            )
                        }
                        .disabled(!playable || pb.blocked.is_some()),
                    );
                }
                // 灰掉的话，那句话就写在按钮旁边 —— 面板上只有一处解释会读漏
                if let Some(reason) = buttons.iter().find_map(|pb| pb.blocked) {
                    line.spawn(label(&font.0, reason, size::ROW, col::JAILED));
                }
            });
        }
        // 分隔线不是装饰：最后一张卡的「用」与「关闭」都是 96×32 的绿/灰按钮，
        // 挨在一起时「关闭」会被读成那张卡的一部分
        p.spawn(super::separator());
        button(
            p,
            &font.0,
            ButtonSpec::new(
                "关闭",
                layout::PANEL_CLOSE_BUTTON,
                SECONDARY,
                UiAction::ToggleHand,
            ),
        );
    });
}

/// 挂着模态时，只有那位被占住的人能出牌。与 `finance::same_actor` 同一条理由：
/// `can_play` 已经要求没有待决事项，这一句只是把那个前提说清楚。
fn same_actor(current: Entity, pending: &PendingDecision) -> bool {
    pending.active().is_none_or(|(player, _)| player == current)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(target: CardTarget) -> CardDef {
        CardDef {
            id: "item_test".into(),
            kind: CardKind::Item,
            title: "测试卡".into(),
            desc: "测试".into(),
            cost_points: Some(20),
            target,
            effects: vec![],
        }
    }

    fn foe(n: u32, name: &str) -> (Entity, String) {
        (
            Entity::from_raw_u32(n).expect("索引不该撞上保留值"),
            name.to_string(),
        )
    }

    fn pick(label: &str, land: u32) -> LandPick {
        LandPick {
            label: label.to_string(),
            land: LandId(land),
        }
    }

    /// 空的地块候选：开局那一回合还没有人买地。
    const NO_PLOTS: &[LandPick] = &[];
    /// 空的无主地候选：一块现在买得下来的地都没有。
    const NO_PARCELS: &[LandPick] = &[];
    /// 空的「人」候选。只有被测那张卡用不到它时才是空排 —— 面板上那一排永远至少有「自己」。
    const NO_PEOPLE: &[(Entity, String)] = &[];

    #[test]
    fn a_self_only_card_grows_exactly_one_button_and_no_target() {
        let buttons = play_buttons(
            &def(CardTarget::SelfOnly),
            &[foe(2, "孙小美"), foe(3, "阿土伯")],
            &[foe(5, "自己"), foe(2, "孙小美")],
            &[pick("我的南京路", 0)],
            &[pick("外滩北 $1500", 2)],
            false,
        );
        assert_eq!(buttons.len(), 1, "给自己的卡不该长出「对谁」这一排按钮");
        assert_eq!(buttons[0].aim, None);
        assert_eq!(buttons[0].blocked, None, "没有理由让它灰着");
    }

    #[test]
    fn a_framing_card_grows_one_button_per_opponent_carrying_that_target() {
        let foes = [foe(2, "孙小美"), foe(3, "阿土伯")];
        let buttons = play_buttons(
            &def(CardTarget::Player),
            &foes,
            &[foe(5, "自己"), foe(2, "孙小美"), foe(3, "阿土伯")],
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(buttons.len(), 2, "两个对手就该有两颗可选的按钮");
        // 目标烤在按钮里：按下去就是一个完整意图，规则层不必再挂「选目标」的待决
        assert_eq!(buttons[0].aim, Some(CardAim::Player(foes[0].0)));
        assert_eq!(buttons[1].aim, Some(CardAim::Player(foes[1].0)));
        assert_eq!(buttons[0].text, "→ 孙小美", "按钮要说得出打的是谁");
        assert!(buttons.iter().all(|b| b.blocked.is_none()));
    }

    #[test]
    fn a_player_card_with_nobody_left_is_greyed_with_a_reason() {
        // 单人不残局：其他人都破产离场了。按下去规则层会静默拒绝，
        // 而玩家看见的必须是一句话，不是一颗死按钮（坑 27 的 UI 版）
        let buttons = play_buttons(
            &def(CardTarget::Player),
            &[],
            NO_PEOPLE,
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("场上没有别的人了"));
        assert_eq!(buttons[0].aim, None, "灰按钮不该带目标");
    }

    /// 转向卡点的是「一个人」，那一排里必须有「自己」那颗 —— 这一格存在的全部理由：
    /// 陷害卡不许打给自己，而掉头与停下正是要能对自己用的两件事
    #[test]
    fn an_anybody_card_offers_the_caster_too_and_puts_self_first() {
        let buttons = play_buttons(
            &def(CardTarget::Anybody),
            &[foe(2, "孙小美")],
            &[foe(5, "自己"), foe(2, "孙小美")],
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(buttons.len(), 2, "自己 + 一名对手 = 两颗按钮");
        assert_eq!(
            buttons[0].text, "→ 自己",
            "自己排最前面：那是这两张卡最常见的用法"
        );
        assert_eq!(buttons[0].aim, Some(CardAim::Player(foe(5, "自己").0)));
        assert_eq!(buttons[1].aim, Some(CardAim::Player(foe(2, "孙小美").0)));
        assert!(buttons.iter().all(|b| b.blocked.is_none()));
    }

    #[test]
    fn an_anybody_card_with_nobody_at_all_still_says_something() {
        // 空排只在「没有当前回合玩家」那种退化状态下出现。这时给一排零按钮的卡行
        // 读起来就是坏了，所以它得有一句话 —— 与 `Player` 那一格同一条纪律
        let buttons = play_buttons(
            &def(CardTarget::Anybody),
            &[],
            NO_PEOPLE,
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("现在没有可行动的人"));
        assert_eq!(buttons[0].aim, None);
    }

    #[test]
    fn a_land_card_grows_one_button_per_owned_plot_and_ignores_the_other_two_lists() {
        // 地块卡数的是地，不是人：候选排里混进「→ 孙小美」会让人以为能直接封人。
        // 无主地那一排同样不归它 —— 查封一块无主地没有业主可办，那条判据在规则层
        let picks = [pick("我的南京路", 0), pick("孙小美的徐家汇", 3)];
        let buttons = play_buttons(
            &def(CardTarget::Land),
            &[foe(2, "孙小美")],
            NO_PEOPLE,
            &picks,
            &[pick("外滩北 $1500", 2)],
            false,
        );
        assert_eq!(buttons.len(), 2, "两块有主的地就该有两颗按钮");
        assert_eq!(
            buttons[0].text, "→ 我的南京路",
            "label 要原样上按钮，玩家靠它分辨自己与别人"
        );
        assert_eq!(buttons[0].aim, Some(CardAim::Land(LandId(0))));
        assert_eq!(buttons[1].aim, Some(CardAim::Land(LandId(3))));
        assert!(buttons.iter().all(|b| b.blocked.is_none()));
    }

    #[test]
    fn a_land_card_with_nothing_owned_is_greyed_with_a_reason() {
        // 开局第一回合谁都没买地。这时打地块卡是空转，而空转该读成一句话
        let buttons = play_buttons(
            &def(CardTarget::Land),
            &[],
            NO_PEOPLE,
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("还没有人有地"));
        assert_eq!(buttons[0].aim, None, "灰按钮不该带目标");
    }

    #[test]
    fn a_grab_card_grows_one_button_per_buyable_parcel_and_ignores_owned_plots() {
        // 购地卡与涨价卡指着同一种东西（一块地），候选排却互斥：有主的地对它没意义，
        // 而无主的地要按现金筛过才敢亮 —— 所以这两排不能合成一排
        let parcels = [pick("汉口路 $1000", 1), pick("外滩北 $1500", 2)];
        let buttons = play_buttons(
            &def(CardTarget::UnownedLand),
            &[foe(2, "孙小美")],
            NO_PEOPLE,
            &[pick("我的南京路", 0)],
            &parcels,
            false,
        );
        assert_eq!(buttons.len(), 2, "两块买得下来的无主地就该有两颗按钮");
        assert_eq!(
            buttons[1].text, "→ 外滩北 $1500",
            "价要写在按钮上：按下去之前唯一要判断的就是这个数"
        );
        assert_eq!(buttons[0].aim, Some(CardAim::Land(LandId(1))));
        assert!(buttons.iter().all(|b| b.blocked.is_none()));
    }

    #[test]
    fn a_grab_card_with_nothing_buyable_says_the_one_sentence_that_is_always_true() {
        // 那句是「买得起的无主地」而不是「没有无主地」：候选排是账本按现金筛过的，
        // 面板读得到「筛完是空」，读不到空在哪一头。地全有主时那句话照样成立，
        // 而「没有无主地」在钱不够的那一格就是句假话（同百货「这一趟没有货」那次改法）
        let buttons = play_buttons(
            &def(CardTarget::UnownedLand),
            &[],
            NO_PEOPLE,
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("没有买得起的无主地"));
        assert_eq!(buttons[0].aim, None, "灰按钮不该带目标");
    }

    #[test]
    fn a_doom_bearer_hears_why_he_cannot_buy_rather_than_that_nothing_is_cheap_enough() {
        // 空排有两个来源，而那句「没有买得起的无主地」对附身的人是假话：地是无主的、
        // 钱也够，缺的是「这件事今天不给做」。两条各占一格，红在该换的那一句上
        let buttons = play_buttons(
            &def(CardTarget::UnownedLand),
            &[],
            NO_PEOPLE,
            NO_PLOTS,
            NO_PARCELS,
            true,
        );
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("那位神让买地必败"));
        let again = play_buttons(
            &def(CardTarget::UnownedLand),
            &[],
            NO_PEOPLE,
            NO_PLOTS,
            NO_PARCELS,
            false,
        );
        assert_eq!(
            again[0].blocked,
            Some("没有买得起的无主地"),
            "没有神时仍要说钱那一头 —— 两句换成同一句就测不出判据分了叉"
        );
    }

    #[test]
    fn the_shield_line_names_both_sources_and_never_shows_zero() {
        assert_eq!(shield_text(None, None), None, "两样都没有就不写这一句");
        assert_eq!(
            shield_text(Some(&Insulation { charges: 2 }), None),
            Some("护身 2 层".into())
        );
        assert_eq!(
            shield_text(Some(&Insulation { charges: 0 }), None),
            None,
            "用完的那一层规则层已经摘了组件，面板不许说「护身 0 层」"
        );
        // 神明那一半不在组件里（P6b②），所以它由第二个参数带进来：大福神＝免疫
        assert_eq!(
            shield_text(None, Some(God::BigFortune)),
            Some("护身：免疫".into()),
            "免疫要写成「免疫」，不是「∞」也不是一个大数"
        );
        assert_eq!(
            shield_text(None, Some(God::SmallFortune)),
            None,
            "小福神不免厄运（PLAN §三 只给大的那一半），面板不该替他吹"
        );
        assert_eq!(
            shield_text(Some(&Insulation { charges: 2 }), Some(God::Angel)),
            Some("护身：免疫（另有卡 2 层）".into()),
            "两半都要念出来：免疫期间卡上的层数不消耗，这是玩家要知道的玩法"
        );
    }

    #[test]
    fn every_sample_card_actually_exists_on_the_shelf() {
        // `NM_HAND` 是按 id 找卡的。改数据时把某张删掉，这个旋钮就会静默拍不到东西 ——
        // 而「截图里面板是空的」读起来像面板坏了，不像数据少了张卡
        let text = include_str!("../../../../assets/data/cards/item_cards.json");
        let set = game_core::prelude::CardSet::from_json(text).expect("道具牌应能解析");
        for id in SAMPLE_CARDS {
            assert!(
                set.0.iter().any(|c| c.id == id),
                "item_cards.json 里少了样例卡 {id}，NM_HAND 会拍不到东西"
            );
        }
    }
}
