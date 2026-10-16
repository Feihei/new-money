//! 四个模态：买地、升级、租金收据、卡面通知。
//!
//! 对应 Godot `property_dialog.gd` / `upgrade_dialog.gd` / `rent_notice.gd` / `card_notice.gd`
//! （合计约 900 行，四个文件里 `_build_ui` 与 `_style_button` 几乎逐字重复）。
//!
//! 弹出时机不靠消息、靠 [`PendingDecision`]：规则层已经把「现在等谁回答什么」写成系统状态，
//! UI 只跟着它重建。用消息驱动会有一类漏弹 —— 弹窗 system 排在建报价的 system 之后时，
//! 本帧的消息要下一帧才读得到，而玩家在那一帧就已经能点东西了。
//!
//! 内容在弹出那一刻**烤死**，之后不随世界状态更新：待决事项被回答时 `PendingDecision`
//! 一定先清空再挂上下一个，所以每次报价都是一整块新面板，没有「忘了刷新」的余地。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, BankOp, Books, CardDrawn, CardKind, CardNotice, Decision, EffectOutcome, LandId,
    LandType, MarketClock, Pending, PendingDecision, PlayerName, TurnOrder,
};

use super::finance::{comma, money_row};
use super::theme::{col, layout, size};
use super::{
    BoardUi, ButtonSpec, SECONDARY, UiAction, button, info_row, is_open_day, label, overlay, panel,
    separator,
};
use crate::render::{DevFont, palette};

/// 模态根节点，兼作「现在挂着的是哪个问题」与「上次写成什么样」两份记录。
///
/// 键带 `Entity` 而不是只带 `Pending`：同一块地连续两次挂牌（甲拒买、乙又踩到）时
/// `about` 相等的，只有玩家不同才会重建 —— 少带这一半就会给乙看甲的现金。
///
/// `stamp` 只有柜台用得上（其余模态是空的）：柜台是一个**窗口**而不是一个报价，玩家能在
/// 里面连办四笔，每次余额都变。没有这份签名就会出现「已经存下 1,000，面板还显示存之前
/// 的现金」—— 而那正是让人按错第二次的形状。
///
/// 为什么不存成 resource（先前是 `OpenModal`）：回主菜单时面板随 `BoardUi` 一起被收走，
/// 而资源里的 `Entity` 句柄与「上次挂的是什么」会活下来。下一局第一次报价若与上一局
/// 最后一次恰好相同（`Buy(LandId(0))` 很常见），判据一比就相等 → 直接 return →
/// **模态永远不再出现**，回合机停在 `WaitDecision` 等一个不存在的按钮。挂在实体上，
/// 实体没了键就没了，不存在需要重置的残留。
#[derive(Component)]
pub(crate) struct ModalRoot {
    pub key: (Entity, Pending),
    pub stamp: String,
}

/// 卡面正文。`Pending::Card` 只说「有一张牌要读」，不说是哪张 —— 规则层刻意不把字符串
/// 放进系统状态（`msg.rs` 的 `CardDrawn` 带 `CardNotice`），所以 UI 自己记最近一张。
/// 这一份**必须**在回菜单时清掉：它是消息留下的记忆，实体上没有对应物。
#[derive(Resource, Default)]
pub(crate) struct RecentCard(pub(crate) Option<CardNotice>);

/// 开发用：`NM_DECISION=buy|upgrade|rent|card|bank` 直接把那个模态挂出来。
///
/// 五个模态各有各的触发条件（升级要凑齐同色组、收据要有人踩过别人的地、柜台要正好停在
/// 银行格上），一局里未必碰得到，而 agent 没有鼠标、截图按墙钟 —— 与 `NM_PAUSE`、
/// `NM_PICK_TILE` 同一条理由：进不了截图的界面等于没做完。只写 [`PendingDecision`]，
/// 不碰回合机：模态由状态驱动，挂上就显示。点掉它会再挂出来，这条开关只为核对界面存在。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalKind {
    Buy,
    Upgrade,
    Rent,
    Card,
    Bank,
}

#[derive(Resource)]
pub(crate) struct ForceModal(Option<ModalKind>);

impl ForceModal {
    pub(crate) fn from_env() -> Self {
        let name = match std::env::var("NM_DECISION") {
            Ok(name) => name,
            Err(_) => return Self(None),
        };
        let kind = match name.as_str() {
            "buy" => ModalKind::Buy,
            "upgrade" => ModalKind::Upgrade,
            "rent" => ModalKind::Rent,
            "card" => ModalKind::Card,
            "bank" => ModalKind::Bank,
            _ => {
                warn!("NM_DECISION={name} 不认识，应为 buy|upgrade|rent|card|bank");
                return Self(None);
            }
        };
        Self(Some(kind))
    }
}

/// 把 `NM_DECISION` 要的那个待决事项挂上。排在 [`sync_modal`] 之前，同帧就显示。
pub(crate) fn force_decision(
    force: Res<ForceModal>,
    active: Res<ActiveBoard>,
    order: Res<TurnOrder>,
    mut pending: ResMut<PendingDecision>,
) {
    let Some(kind) = force.0 else { return };
    if pending.active().is_some() {
        return;
    }
    let Some(player) = order.current() else {
        return;
    };
    // 柜台不关于任何对象，直接挂
    if kind == ModalKind::Bank {
        pending.offer(player, Pending::BankTeller);
        return;
    }
    // 其余四个都要一块地：挑一块租金表最长、带同色组的，模态内容才不会是空壳
    let Some(land) = active
        .def
        .lands
        .iter()
        .filter(|l| l.group.is_some() && l.rent.len() > 1)
        .max_by_key(|l| l.rent.len())
        .map(|l| LandId(l.id))
    else {
        return;
    };
    let about = match kind {
        ModalKind::Buy => Pending::Buy(land),
        ModalKind::Upgrade => Pending::Upgrade(land),
        ModalKind::Rent => Pending::RentNotice(land),
        ModalKind::Card => Pending::Card,
        ModalKind::Bank => unreachable!("柜台在上面就已经挂好了"),
    };
    pending.offer(player, about);
}

pub(crate) fn remember_drawn_card(
    mut drawn: MessageReader<CardDrawn>,
    mut memory: ResMut<RecentCard>,
) {
    for card in drawn.read() {
        memory.0 = Some(card.card.clone());
    }
}

/// 模态这一屏要读的世界状态。打包成参数包是这仓库的房规：`sync_modal` 本来就要同时看
/// 「等谁回答」「上一张卡」「今天几号」「现在挂着的是哪块面板」，
/// 而 `#[allow(clippy::too_many_arguments)]` 会把「一个 system 要读八样东西」这个信号吃掉
/// （`.agents/ARCHITECTURE.md` §六 坑 8）。
#[derive(SystemParam)]
pub(crate) struct ModalView<'w, 's> {
    pending: Res<'w, PendingDecision>,
    memory: Res<'w, RecentCard>,
    clock: Res<'w, MarketClock>,
    open: Query<'w, 's, (Entity, &'static ModalRoot)>,
}

/// 跟着 `PendingDecision` 建/换/收模态。排在 [`remember_drawn_card`] 之后，
/// 于是卡面消息与面板内容在同一帧到位，不会出现「先弹一张空白卡」。
///
/// 重建判据是「问题 + 签名」两半：问题变了当然重建，而柜台那种**窗口**（一个待决事项里
/// 能办四笔）问的是同一个问题、数字却一直在动 —— 只看问题就会出现「钱已经存进去了，
/// 面板还停在存之前」。签名只对柜台有意义，其余四个模态的内容弹出即定稿。
pub(crate) fn sync_modal(
    mut commands: Commands,
    font: Res<DevFont>,
    books: Books,
    names: Query<&PlayerName>,
    view: ModalView<'_, '_>,
) {
    let want = view.pending.active();
    let banking_hours = is_open_day(&view.clock, &books.rules);
    let stamp = want.map_or(String::new(), |(player, about)| {
        modal_stamp(about, &books, player, banking_hours)
    });
    // 两边都是 `None` 时相等：没有待决事项、也没有模态，正是常态
    if view
        .open
        .iter()
        .next()
        .is_some_and(|(_, root)| Some(root.key) == want && root.stamp == stamp)
    {
        return;
    }
    // 一次只挂一个：换问题时先把旧的整块收走（`despawn` 递归，遮罩是子树根）
    for (stale, _) in view.open.iter() {
        commands.entity(stale).despawn();
    }
    if let Some((player, about)) = want {
        let stamp = stamp.clone();
        match about {
            Pending::Buy(land) => buy_modal(&mut commands, &font, &books, player, land, stamp),
            Pending::Upgrade(land) => {
                upgrade_modal(&mut commands, &font, &books, player, land, stamp)
            }
            Pending::RentNotice(land) => {
                rent_modal(&mut commands, &font, &books, &names, player, land, stamp)
            }
            Pending::Card => card_modal(&mut commands, &font, &view.memory, player, stamp),
            Pending::BankTeller => {
                bank_modal(&mut commands, &font, &books, player, stamp, banking_hours)
            }
        };
    }
}

/// 这一屏「该写成什么样」的签名。只有柜台需要，别的模态返回空串。
///
/// `banking_hours` 也在签名里：正常对局里回合停在 `WaitDecision` 上，日历动不了，
/// 柜台开着的那几帧不可能跨到周末 —— 但 `NM_DAY` 与「读档回到某一天」都能让日子在
/// 面板背后跳。签名少了它，面板就会停在按开门那天烤死的样子（灰按钮配「办完才继续」），
/// 而这正是 §6.1 坑 22 那一族：**刷新判据漏了一个输入，且没有任何报错**。
fn modal_stamp(about: Pending, books: &Books, player: Entity, banking_hours: bool) -> String {
    let Pending::BankTeller = about else {
        return String::new();
    };
    let bank = books.bank_of(player);
    format!(
        "{}|{}|{}|{}|{banking_hours}",
        books.balance(player),
        bank.deposit,
        bank.loan,
        loan_room(books, player)
    )
}

/// 名下未抵押地块的可贷上限（缺陷 #1 修的就是这个数在 Godot 里恒为 0）。
fn loan_room(books: &Books, player: Entity) -> i64 {
    let Some(pid) = books.id_of(player) else {
        return 0;
    };
    game_core::economy::calc_max_loan(&books.active.def, &books.views(), pid, &books.rules)
}

/// 遮罩 + 居中面板的外壳。遮罩节点是整棵子树的根，`despawn` 它即收走全部内容；
/// `key` 是这一屏在回答哪个待决事项，[`sync_modal`] 靠它判断要不要重建。
fn frame(
    commands: &mut Commands,
    key: (Entity, Pending),
    stamp: String,
    spec: [f32; 3],
    accent: Color,
    body: impl FnOnce(&mut ChildSpawnerCommands),
) -> Entity {
    let (overlay_node, overlay_bg, overlay_z) = overlay(3);
    let (panel_node, panel_transform, panel_bg, panel_border) = panel(spec[0], spec, accent);
    commands
        .spawn((
            BoardUi,
            ModalRoot { key, stamp },
            overlay_node,
            overlay_bg,
            overlay_z,
        ))
        .with_children(|mask| {
            mask.spawn((panel_node, panel_transform, panel_bg, panel_border))
                .with_children(|p| body(p));
        })
        .id()
}

/// 面板里居中的那一行（标题、副标题、现金）。
///
/// 节点仍然拉满面板宽度，靠 `Justify::Center` 让文字在行内居中 —— 与 Godot 的
/// `Label` + `HORIZONTAL_ALIGNMENT_CENTER` 同形。改成父级 `align_items: Center` 会
/// 连带把信息行也居中，而 Godot 的信息行是左对齐的。
fn center_label(
    font: &Handle<Font>,
    text: impl Into<String>,
    px: f32,
    color: Color,
) -> (Node, TextLayout, Text, TextFont, TextColor) {
    let (t, f, c) = label(font, text, px, color);
    (
        Node {
            width: Val::Percent(100.0),
            ..default()
        },
        TextLayout::justify(Justify::Center),
        t,
        f,
        c,
    )
}

/// 一行按钮（Godot 的 `btn_box`：居中、间距 20）。
fn button_row(parent: &mut ChildSpawnerCommands, f: impl FnOnce(&mut ChildSpawnerCommands)) {
    parent
        .spawn(Node {
            flex_direction: FlexDirection::Row,
            justify_content: JustifyContent::Center,
            column_gap: Val::Px(layout::BTN_ROW_GAP),
            ..default()
        })
        .with_children(|line| f(line));
}

fn buy_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let def = books.active.def.land(land.0);
    let name = def.map(|l| l.name.clone()).unwrap_or_default();
    let group = def.and_then(|l| l.group.clone()).unwrap_or_default();
    let accent = if group.is_empty() {
        col::TITLE_GOLD
    } else {
        palette::group(&group)
    };
    let price = def.map(|l| l.price).unwrap_or(0);
    let rent: Vec<i64> = def.map(|l| l.rent.clone()).unwrap_or_default();
    let type_text = match def.map(|l| l.land_type) {
        Some(LandType::Commercial) => "商业用地",
        Some(LandType::Insurance) => "保险用地",
        _ => "住宅用地",
    };
    let cash = books.balance(player);
    let affordable = cash >= price;
    let buy_text = if affordable {
        format!("购买 (${price})")
    } else {
        "现金不足".to_string()
    };
    let cash_text = format!("你的现金: ${cash}");

    frame(
        commands,
        (player, Pending::Buy(land)),
        stamp,
        layout::BUY_PANEL,
        col::BORDER_BUY,
        |p| {
            p.spawn(center_label(&font.0, name, size::BUY_TITLE, accent));
            p.spawn(center_label(
                &font.0,
                "是否购买此地？",
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            // 信息行组套一个竖排容器（Godot 的 `InfoPanel` VBox，行距 4）
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::INFO_ROW_GAP),
                ..default()
            })
            .with_children(|rows| {
                let key_w = layout::INFO_KEY_W;
                if !group.is_empty() {
                    info_row(rows, &font.0, "颜色组", &group, accent, key_w);
                }
                info_row(
                    rows,
                    &font.0,
                    "地价",
                    &format!("${price}"),
                    col::COST,
                    key_w,
                );
                info_row(rows, &font.0, "类型", type_text, col::VALUE, key_w);
                if !rent.is_empty() {
                    rows.spawn(label(&font.0, "租金表", size::SMALL, col::KEY));
                    for (i, amount) in rent.iter().enumerate() {
                        info_row(
                            rows,
                            &font.0,
                            &format!("Lv{}", i + 1),
                            &format!("${amount}"),
                            col::VALUE,
                            key_w,
                        );
                    }
                }
            });
            p.spawn(separator());
            p.spawn(center_label(
                &font.0,
                cash_text,
                size::NAME,
                if affordable {
                    col::CASH_OK
                } else {
                    col::CASH_BAD
                },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        &buy_text,
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Accept),
                    )
                    .disabled(!affordable),
                );
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "取消",
                        layout::MODAL_BUTTON,
                        SECONDARY,
                        UiAction::Decide(Decision::Decline),
                    ),
                );
            });
        },
    )
}

fn upgrade_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let Some(view) = books.view(land) else {
        return fallback_note(commands, font, (player, Pending::Upgrade(land)), "升级报价");
    };
    let def = books.active.def.land(land.0);
    let name = def.map(|l| l.name.clone()).unwrap_or_default();
    let accent = def
        .and_then(|l| l.group.as_deref())
        .map(palette::group)
        .unwrap_or(col::TITLE_GOLD);
    let cost = game_core::economy::upgrade_cost(&books.active.def, &view).unwrap_or(0);
    let rent: Vec<i64> = def.map(|l| l.rent.clone()).unwrap_or_default();
    let level = view.level as usize;
    let current_rent = rent.get(level.saturating_sub(1)).copied().unwrap_or(0);
    let new_rent = rent.get(level).copied().unwrap_or(0);
    let cash = books.balance(player);
    let affordable = cash >= cost;
    let title = format!("升级 {name}");
    let buy_text = if affordable {
        format!("升级 (${cost})")
    } else {
        "现金不足".to_string()
    };
    let cash_text = format!("你的现金: ${cash}");

    frame(
        commands,
        (player, Pending::Upgrade(land)),
        stamp,
        layout::UPGRADE_PANEL,
        col::BORDER_UPGRADE,
        |p| {
            p.spawn(center_label(&font.0, title, size::DIALOG_TITLE, accent));
            p.spawn(center_label(
                &font.0,
                "是否升级此地？",
                size::ROW,
                col::MUTED,
            ));
            p.spawn(separator());
            // 信息行组套一个竖排容器（Godot 的 `InfoPanel` VBox，行距 4）
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::INFO_ROW_GAP),
                ..default()
            })
            .with_children(|rows| {
                let key_w = layout::INFO_KEY_W_WIDE;
                info_row(
                    rows,
                    &font.0,
                    "当前等级",
                    &format!("Lv{level}"),
                    col::VALUE,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "升级后",
                    &format!("Lv{}", level + 1),
                    col::GAIN,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "升级费用",
                    &format!("${cost}"),
                    col::COST,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "当前租金",
                    &format!("${current_rent}"),
                    col::VALUE,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "升级后租金",
                    &format!("${new_rent}"),
                    col::GAIN,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "租金增幅",
                    &format!("+${}", new_rent - current_rent),
                    col::GAIN,
                    key_w,
                );
            });
            p.spawn(separator());
            p.spawn(center_label(
                &font.0,
                cash_text,
                size::NAME,
                if affordable {
                    col::CASH_OK
                } else {
                    col::CASH_BAD
                },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        &buy_text,
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Accept),
                    )
                    .disabled(!affordable),
                );
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "取消",
                        layout::MODAL_BUTTON,
                        SECONDARY,
                        UiAction::Decide(Decision::Decline),
                    ),
                );
            });
        },
    )
}

fn rent_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    names: &Query<&PlayerName>,
    payer: Entity,
    land: LandId,
    stamp: String,
) -> Entity {
    let def = books.active.def.land(land.0);
    let land_name = def.map(|l| l.name.clone()).unwrap_or_default();
    let amount = books.rent_of(land);
    let owner = books
        .view(land)
        .and_then(|v| v.owner)
        .and_then(|id| books.holder(id))
        .and_then(|e| names.get(e).ok())
        .map(|n| n.0.clone())
        .unwrap_or_else(|| "无主".to_string());
    let payer_name = names.get(payer).map(|n| n.0.clone()).unwrap_or_default();
    // 「先扣钱后通知」是规则层的既有顺序（`property.rs:62-82`），所以破产在这里
    // 已经是既成事实，收据只是把它说出来 —— 与 Godot 的 `went_bankrupt` 同一个语义
    let bankrupt = books.is_bankrupt(payer);
    let title = if bankrupt {
        "破产！"
    } else {
        "支付租金"
    };
    let title_color = if bankrupt {
        col::BANKRUPT
    } else {
        col::TITLE_GOLD
    };
    let desc = if bankrupt {
        format!(
            "{payer_name} 踩到 {owner} 的地（{land_name}）\n租金 ${amount} 无法支付，宣告破产！"
        )
    } else {
        format!("{payer_name} 踩到 {owner} 的地（{land_name}）")
    };
    let ok_text = if bankrupt { "确认破产" } else { "确认" };

    frame(
        commands,
        (payer, Pending::RentNotice(land)),
        stamp,
        layout::RENT_PANEL,
        col::BORDER_RENT,
        |p| {
            p.spawn(center_label(
                &font.0,
                title,
                size::DIALOG_TITLE,
                title_color,
            ));
            p.spawn(center_label(&font.0, desc, size::NAME, col::BODY));
            p.spawn(center_label(
                &font.0,
                format!("- ${amount}"),
                size::RENT_AMOUNT,
                if bankrupt { col::BANKRUPT } else { col::COST },
            ));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        ok_text,
                        layout::OK_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

fn card_modal(
    commands: &mut Commands,
    font: &DevFont,
    memory: &RecentCard,
    player: Entity,
    stamp: String,
) -> Entity {
    let Some(card) = memory.0.as_ref() else {
        return fallback_note(commands, font, (player, Pending::Card), "卡牌效果已结算");
    };
    let (type_text, accent) = match card.kind {
        CardKind::Chance => ("机会", col::CHANCE),
        CardKind::Fate => ("命运", col::FATE),
    };
    let effect = outcome_line(&card.outcomes);

    frame(
        commands,
        (player, Pending::Card),
        stamp,
        layout::CARD_PANEL,
        col::BORDER_CARD,
        |p| {
            p.spawn(center_label(
                &font.0,
                format!("【{type_text}】"),
                size::BODY,
                accent,
            ));
            p.spawn(center_label(
                &font.0,
                card.title.clone(),
                size::CARD_TITLE,
                accent,
            ));
            p.spawn(separator());
            if !card.desc.is_empty() {
                p.spawn(label(&font.0, card.desc.clone(), size::NAME, col::BODY));
            }
            if !effect.is_empty() {
                p.spawn(label(&font.0, effect, size::BODY, col::RESULT));
            }
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec {
                        text: "确定",
                        size: layout::CARD_OK_BUTTON,
                        font: size::CARD_OK,
                        palette: super::PRIMARY,
                        action: UiAction::Decide(Decision::Acknowledge),
                        disabled: false,
                    },
                );
            });
        },
    )
}

/// 一张兜底说明。`Pending` 与消息对不上时（测试里没抽过牌却被挂着、土地数据缺行）
/// 也要有个能点掉的东西 —— 模态挡着掷骰，弹不出内容就会把这一局卡死。
fn fallback_note(
    commands: &mut Commands,
    font: &DevFont,
    key: (Entity, Pending),
    text: &str,
) -> Entity {
    let text = text.to_string();
    frame(
        commands,
        key,
        String::new(),
        layout::RENT_PANEL,
        col::BORDER_CARD,
        |p| {
            p.spawn(label(&font.0, text, size::DIALOG_TITLE, col::BODY));
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "确定",
                        layout::OK_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 银行柜台：**踩到**银行格才开（路过只给 ATM 卡，见 [`super::finance`]）。
///
/// 与其余四个模态最大的分别是它不是一次报价，而是一个窗口：玩家可以连办四笔，
/// 每笔之后数字都要重算 —— 所以 [`sync_modal`] 的重建判据里带了金额签名这一半。
/// 「办完」才把回合交还给结束流程；中途按别的（`Accept`/`Decline`）规则层一律不认。
///
/// 窗口判据在这里只剩 `banking_hours` 一条：柜台既然开着，四种操作的主语与地点都已经
/// 成立，剩下「这笔金额办不办得动」。周末柜台照样开（人踩上去了就是事实），但八颗按钮
/// 全灰 + 一句「周末休市」—— 规则层的 `bank_op_system` 那两天一律不受理，
/// 按钮亮着按下去没声音是这里最坏的一种失败。
fn bank_modal(
    commands: &mut Commands,
    font: &DevFont,
    books: &Books,
    player: Entity,
    stamp: String,
    banking_hours: bool,
) -> Entity {
    let cash = books.balance(player);
    let bank = books.bank_of(player);
    let room = loan_room(books, player);
    // 还能借多少 = 上限 - 现有负债（`economy::loan_plan` 判的就是这个累计口径）
    let headroom = (room - bank.loan).max(0);
    // 还清要一次掏够：现金不够就灰着，别让「还 3000」在只有 2000 时静默失败
    let clear_debt = bank.loan.min(cash);
    let rules = *books.rules;

    frame(
        commands,
        (player, Pending::BankTeller),
        stamp,
        layout::BANK_PANEL,
        col::BORDER_BANK,
        |p| {
            p.spawn(center_label(
                &font.0,
                "银行柜台",
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            p.spawn(center_label(
                &font.0,
                if banking_hours {
                    "存取款、借贷都在这里办，办完才继续"
                } else {
                    "周末休市，今天八笔都办不了 —— 先办完离开"
                },
                size::ROW,
                if banking_hours {
                    col::MUTED
                } else {
                    col::BANKRUPT
                },
            ));
            p.spawn(separator());
            p.spawn(Node {
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(layout::INFO_ROW_GAP),
                ..default()
            })
            .with_children(|rows| {
                let key_w = layout::INFO_KEY_W_WIDE;
                info_row(rows, &font.0, "现金", &comma(cash), col::CASH_OK, key_w);
                info_row(
                    rows,
                    &font.0,
                    "存款余额",
                    &comma(bank.deposit),
                    col::VALUE,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "现有负债",
                    &comma(bank.loan),
                    col::COST,
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "还可借",
                    &comma(headroom),
                    if headroom > 0 {
                        col::GAIN
                    } else {
                        col::BANKRUPT
                    },
                    key_w,
                );
                info_row(
                    rows,
                    &font.0,
                    "利率",
                    &format!(
                        "存 {:.1}% / 贷 {:.1}%，每月末各结一次",
                        rules.deposit_rate * 100.0,
                        rules.loan_rate * 100.0
                    ),
                    col::KEY,
                    key_w,
                );
            });
            p.spawn(separator());
            money_row(
                p,
                font,
                BankOp::Deposit,
                banking_hours,
                ("存", 1_000, cash >= 1_000),
                ("存全部", cash, cash > 0),
            );
            money_row(
                p,
                font,
                BankOp::Withdraw,
                banking_hours,
                ("取", 1_000, bank.deposit >= 1_000),
                ("取全部", bank.deposit, bank.deposit > 0),
            );
            money_row(
                p,
                font,
                BankOp::Borrow,
                banking_hours,
                ("借", 5_000, headroom >= 5_000),
                ("借到上限", headroom, headroom > 0),
            );
            money_row(
                p,
                font,
                BankOp::Repay,
                banking_hours,
                ("还", 5_000, bank.loan >= 5_000 && cash >= 5_000),
                ("还清", clear_debt, bank.loan > 0 && clear_debt >= bank.loan),
            );
            button_row(p, |line| {
                button(
                    line,
                    &font.0,
                    ButtonSpec::new(
                        "办完",
                        layout::MODAL_BUTTON,
                        super::PRIMARY,
                        UiAction::Decide(Decision::Acknowledge),
                    ),
                );
            });
        },
    )
}

/// 卡面效果行。规则层只发凭据、不拼文案（`msg.rs` 的注释），字符串在这里生成。
fn outcome_line(outcomes: &[EffectOutcome]) -> String {
    let parts: Vec<String> = outcomes.iter().map(describe).collect();
    if parts.is_empty() {
        return String::new();
    }
    format!("效果：{}", parts.join("；"))
}

fn describe(outcome: &EffectOutcome) -> String {
    match outcome {
        EffectOutcome::Gained(amount) => format!("获得 ${amount}"),
        EffectOutcome::Lost(amount) => format!("支付 ${amount}"),
        EffectOutcome::CouldNotPay { needed } => format!("现金不足，欠 ${needed}"),
        EffectOutcome::Jailed { turns, .. } => format!("被关进监狱（{turns} 回合）"),
        EffectOutcome::Teleported { name, .. } => format!("移动到「{name}」"),
        // 数据写了个棋盘上不存在的格名：如实报出来，别退成一句漂亮话把数据错误藏起来
        EffectOutcome::UnknownTile { name } => format!("数据错误：找不到「{name}」"),
    }
}
