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
    GameRules, Hand, Insulation, LandId, Paused, PendingDecision, PlayerId, PlayerName, Shelf,
    TurnOrder, TurnPhase, deal_to,
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
/// （涨价 / 查封）、给一块无主地（购地）。少一种，那个形状的按钮排就永远拍不到 ——
/// 而「拍不到」在验收里读起来和「没做」一样。
const SAMPLE_CARDS: [&str; 6] = [
    "item_frame",
    "item_insulate",
    "item_hike",
    "item_seal",
    "item_free",
    "item_grab",
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
/// `lands` 是「此刻真有人持有的地块」（无主地不在里面 —— 规则层也不认，同一句判据），
/// `parcels` 是「此刻**买得下来**的无主地」（买不起的不在里面，同一句判据的另一半）。
///
/// 写成对 `CardTarget` 的穷尽匹配、不留 `_`：新增一种目标时必须当场决定它长什么样，
/// 而不是等截图时才发现有颗哑弹。
pub(crate) fn play_buttons(
    def: &CardDef,
    opponents: &[(Entity, String)],
    lands: &[LandPick],
    parcels: &[LandPick],
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
        CardTarget::Land => "对一块地",
        CardTarget::UnownedLand => "对一块无主地",
    }
}

/// 护身那一行。没挂护身就不写这一句；`Some(0)` 也不写 —— 规则层在用完那一层就把组件
/// 摘了，这里再兜一次，免得面板说出「免罪护身 0 层」这种自相矛盾的话。
fn shield_text(shield: Option<&Insulation>) -> Option<String> {
    match shield?.remaining() {
        None => Some("护身：免疫".into()),
        Some(0) => None,
        Some(n) => Some(format!("护身 {n} 层")),
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
    let shield = shield_text(player.and_then(|p| view.shields.get(p).ok()));
    // 签名要覆盖面板上每一个由判据决定的可见位（§6.1 坑 30：少一位就是一次静默过期）。
    // `lands` 那一位是 P3c 加上的：地块卡的按钮排跟着「谁买了哪块地」变，
    // 少了它，玩家刚买下的那块地不会出现在按钮排里，而下一次别的原因重建面板时才冒出来。
    // `parcels` 是同一件事的购地卡那一半，而且它比 `lands` 更脆：那一排连**现金**都读
    // （花过一笔钱、物价跳一格，候选排就变了）
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
            let buttons = play_buttons(def, &opponents, &lands, &parcels);
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

    #[test]
    fn a_self_only_card_grows_exactly_one_button_and_no_target() {
        let buttons = play_buttons(
            &def(CardTarget::SelfOnly),
            &[foe(2, "孙小美"), foe(3, "阿土伯")],
            &[pick("我的南京路", 0)],
            &[pick("外滩北 $1500", 2)],
        );
        assert_eq!(buttons.len(), 1, "给自己的卡不该长出「对谁」这一排按钮");
        assert_eq!(buttons[0].aim, None);
        assert_eq!(buttons[0].blocked, None, "没有理由让它灰着");
    }

    #[test]
    fn a_framing_card_grows_one_button_per_opponent_carrying_that_target() {
        let foes = [foe(2, "孙小美"), foe(3, "阿土伯")];
        let buttons = play_buttons(&def(CardTarget::Player), &foes, NO_PLOTS, NO_PARCELS);
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
        let buttons = play_buttons(&def(CardTarget::Player), &[], NO_PLOTS, NO_PARCELS);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("场上没有别的人了"));
        assert_eq!(buttons[0].aim, None, "灰按钮不该带目标");
    }

    #[test]
    fn a_land_card_grows_one_button_per_owned_plot_and_ignores_the_other_two_lists() {
        // 地块卡数的是地，不是人：候选排里混进「→ 孙小美」会让人以为能直接封人。
        // 无主地那一排同样不归它 —— 查封一块无主地没有业主可办，那条判据在规则层
        let picks = [pick("我的南京路", 0), pick("孙小美的徐家汇", 3)];
        let buttons = play_buttons(
            &def(CardTarget::Land),
            &[foe(2, "孙小美")],
            &picks,
            &[pick("外滩北 $1500", 2)],
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
        let buttons = play_buttons(&def(CardTarget::Land), &[], NO_PLOTS, NO_PARCELS);
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
            &[pick("我的南京路", 0)],
            &parcels,
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
        let buttons = play_buttons(&def(CardTarget::UnownedLand), &[], NO_PLOTS, NO_PARCELS);
        assert_eq!(buttons.len(), 1);
        assert_eq!(buttons[0].blocked, Some("没有买得起的无主地"));
        assert_eq!(buttons[0].aim, None, "灰按钮不该带目标");
    }

    #[test]
    fn the_shield_line_names_layers_and_never_shows_zero() {
        assert_eq!(shield_text(None), None, "没挂护身就不写这一句");
        assert_eq!(
            shield_text(Some(&Insulation { charges: Some(2) })),
            Some("护身 2 层".into())
        );
        assert_eq!(
            shield_text(Some(&Insulation { charges: Some(0) })),
            None,
            "用完的那一层规则层已经摘了组件，面板不许说「护身 0 层」"
        );
        assert_eq!(
            shield_text(Some(&Insulation { charges: None })),
            Some("护身：免疫".into()),
            "不限次那一类（P6 神明）要写成「免疫」，不是「∞」也不是一个大数"
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
