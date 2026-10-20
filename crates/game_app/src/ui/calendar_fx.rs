//! 日历事件的表现层（P2b）：圣诞礼物一屏、春节红包的老虎机。
//!
//! core 只发凭据（`GiftsGiven` / `RedEnvelope`）。这里做两件**纯表现**的事：
//!
//! - 礼物：一屏列出谁收到哪张卡，5 秒后自动收走；任意点击也能提前收走。
//! - 红包：按玩家序一段一段播老虎机，四位数字逐位停，不做「点停」——
//!   AI 点不了，同一条规则不该只有人类有技巧。播完自动换下一位。
//!
//! **`NM_AUTOROLL` 下整块不装**：长跑验收不能被过节动画挡住。这不是「测试特例」，
//! 而是与「动画可跳过」同一条纪律 —— 自动玩家本来就是「跳过所有表现」的那个模式。

use std::collections::VecDeque;

use bevy::prelude::*;
use bevy::time::Timer;
use game_core::prelude::{GiftsGiven, PlayerName, RedEnvelope};

use super::theme::{col, size};
use super::{BoardUi, border_color, label};
use crate::render::DevFont;

#[derive(Component)]
struct GiftPanel;

#[derive(Component)]
struct GiftTimer(Timer);

#[derive(Component)]
struct EnvelopePanel;

/// 一段老虎机：一个人的红包。`digits` 是真金额的四位（含前导零）。
#[derive(Debug, Clone)]
struct EnvelopeShow {
    player: Entity,
    name: String,
    amount: i64,
    elapsed: f32,
}

#[derive(Resource, Default)]
struct EnvelopeQueue {
    waiting: VecDeque<EnvelopeShow>,
    active: Option<EnvelopeShow>,
}

/// 一位数字停下来的时刻（秒）。四位依次在 0.35 / 0.7 / 1.05 / 1.4 停，
/// 全部停稳后再留 1 秒给人看清，然后换下一位。
const SETTLE_AT: [f32; 4] = [0.35, 0.70, 1.05, 1.40];
const HOLD_UNTIL: f32 = 2.40;

/// 红包金额拆成四位（千位可以是 0）。0000 是合法的空红包。
pub(crate) fn four_digits(amount: i64) -> [u32; 4] {
    let mut left = amount.clamp(0, 9999);
    let mut out = [0u32; 4];
    for i in (0..4).rev() {
        out[i] = (left % 10) as u32;
        left /= 10;
    }
    out
}

/// 截至 `elapsed` 这一刻屏幕上该显示的四位：已停的位是真数字，没停的位在滚。
/// 返回 `(显示串, 是否全部停稳)`。
pub(crate) fn rolling_digits(amount: i64, elapsed: f32) -> (String, bool) {
    let real = four_digits(amount);
    let mut text = String::new();
    let mut settled_all = true;
    for (i, at) in SETTLE_AT.iter().enumerate() {
        if elapsed >= *at {
            text.push(char::from(b'0' + real[i] as u8));
        } else {
            settled_all = false;
            // 滚动值只服务画面：按 0.1 秒换一格，不取随机流（表现层不碰规则层的随机性）
            let spin = ((elapsed * 10.0) as u32 + i as u32) % 10;
            text.push(char::from(b'0' + spin as u8));
        }
    }
    (text, settled_all)
}

pub fn install(app: &mut App) {
    if std::env::var_os("NM_AUTOROLL").is_some() {
        return;
    }
    app.init_resource::<EnvelopeQueue>()
        .add_systems(
            Update,
            (show_gifts, dismiss_gifts_on_click, tick_gift_timer).chain(),
        )
        .add_systems(Update, (collect_envelopes, tick_envelopes).chain());
}

fn show_gifts(
    mut commands: Commands,
    font: Res<DevFont>,
    mut given: MessageReader<GiftsGiven>,
    names: Query<&PlayerName>,
    existing: Query<Entity, With<GiftPanel>>,
) {
    for gift in given.read() {
        for old in &existing {
            commands.entity(old).despawn();
        }
        let rows: Vec<(String, String)> = gift
            .gifts
            .iter()
            .map(|g| {
                let name = names
                    .get(g.player)
                    .map(|n| n.0.clone())
                    .unwrap_or_else(|_| "?".into());
                (name, g.card.title.clone())
            })
            .collect();
        let mut root = commands.spawn((
            BoardUi,
            GiftPanel,
            GiftTimer(Timer::from_seconds(5.0, TimerMode::Once)),
            Pickable::IGNORE,
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(360.0),
                top: Val::Px(150.0),
                width: Val::Px(560.0),
                flex_direction: FlexDirection::Column,
                row_gap: Val::Px(8.0),
                padding: UiRect::axes(Val::Px(20.0), Val::Px(16.0)),
                border: UiRect::all(Val::Px(2.0)),
                border_radius: BorderRadius::all(Val::Px(10.0)),
                ..default()
            },
            BackgroundColor(col::PANEL),
            border_color(col::TITLE_GOLD),
            ZIndex(9),
        ));
        root.with_children(|p| {
            p.spawn(label(
                &font.0,
                format!("圣诞礼物 · {} 年", gift.year),
                size::DIALOG_TITLE,
                col::TITLE_GOLD,
            ));
            if rows.is_empty() {
                p.spawn(label(&font.0, "今年没有人收到礼物", size::ROW, col::MUTED));
            }
            for (name, card) in &rows {
                p.spawn(label(
                    &font.0,
                    format!("{name} ← 「{card}」"),
                    size::ROW,
                    col::VALUE,
                ));
            }
            p.spawn(label(
                &font.0,
                "点击任意处继续（5 秒后自动收起）",
                size::SMALL,
                col::MUTED,
            ));
        });
    }
}

/// 任意一次点击都能提前收走礼物屏 —— 「必须可跳过」在鼠标上的实现。
fn dismiss_gifts_on_click(
    mut commands: Commands,
    mut clicks: MessageReader<PointerClick>,
    existing: Query<Entity, With<GiftPanel>>,
) {
    if clicks.read().next().is_none() {
        return;
    }
    for old in &existing {
        commands.entity(old).despawn();
    }
}

fn tick_gift_timer(
    mut commands: Commands,
    time: Res<Time>,
    mut panels: Query<(Entity, &mut GiftTimer)>,
) {
    for (entity, mut timer) in &mut panels {
        if timer.0.tick(time.delta()).is_finished() {
            commands.entity(entity).despawn();
        }
    }
}

fn collect_envelopes(mut shown: ResMut<EnvelopeQueue>, mut envelopes: MessageReader<RedEnvelope>) {
    for envelope in envelopes.read() {
        shown.waiting.push_back(EnvelopeShow {
            player: envelope.player,
            name: String::new(),
            amount: envelope.amount,
            elapsed: 0.0,
        });
    }
}

fn tick_envelopes(
    mut commands: Commands,
    time: Res<Time>,
    font: Res<DevFont>,
    names: Query<&PlayerName>,
    mut queue: ResMut<EnvelopeQueue>,
    existing: Query<(Entity, &EnvelopeStamp), With<EnvelopePanel>>,
) {
    if queue.active.is_none() {
        queue.active = queue.waiting.pop_front();
    }
    let Some(show) = queue.active.as_mut() else {
        for (old, _) in &existing {
            commands.entity(old).despawn();
        }
        return;
    };
    show.elapsed += time.delta_secs();
    if show.name.is_empty() {
        show.name = names
            .get(show.player)
            .map(|n| n.0.clone())
            .unwrap_or_else(|_| "?".into());
    }
    let (digits, settled) = rolling_digits(show.amount, show.elapsed);
    let done = settled && show.elapsed >= HOLD_UNTIL;

    // 画面只在数字变化时重建（与 ATM 面板同一条签名法），而不是每帧重 spawn
    let stamp = format!("{:?}|{digits}|{done}", show.player);
    if existing.iter().any(|(_, current)| current.0 == stamp) {
        return;
    }
    for (old, _) in &existing {
        commands.entity(old).despawn();
    }
    let total = if settled {
        format!("${}", show.amount)
    } else {
        "…".to_string()
    };
    let mut root = commands.spawn((
        BoardUi,
        EnvelopePanel,
        EnvelopeStamp(stamp),
        Pickable::IGNORE,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(420.0),
            top: Val::Px(210.0),
            width: Val::Px(440.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(8.0),
            padding: UiRect::axes(Val::Px(20.0), Val::Px(18.0)),
            border: UiRect::all(Val::Px(2.0)),
            border_radius: BorderRadius::all(Val::Px(10.0)),
            ..default()
        },
        BackgroundColor(col::PANEL),
        border_color(col::TITLE_GOLD),
        ZIndex(9),
    ));
    root.with_children(|p| {
        p.spawn(label(
            &font.0,
            format!("春节红包 · {}", show.name),
            size::NAME,
            col::TITLE_GOLD,
        ));
        p.spawn(label(&font.0, digits.clone(), 46.0, col::VALUE));
        p.spawn(label(&font.0, total, size::ROW, col::CASH_OK));
    });

    if done {
        queue.active = None;
    }
}

/// 当前红包屏的签名（玩家 + 已经显示出来的四位 + 是否停稳）。
#[derive(Component)]
struct EnvelopeStamp(String);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn four_digits_keep_leading_zeroes() {
        assert_eq!(four_digits(0), [0, 0, 0, 0]);
        assert_eq!(four_digits(7), [0, 0, 0, 7]);
        assert_eq!(four_digits(4271), [4, 2, 7, 1]);
        assert_eq!(four_digits(9999), [9, 9, 9, 9]);
        assert_eq!(four_digits(12345), [9, 9, 9, 9], "越界截到四位");
    }

    #[test]
    fn digits_settle_one_by_one() {
        let (text, done) = rolling_digits(4271, 0.0);
        assert!(!done);
        assert_eq!(text.len(), 4);
        let (text, done) = rolling_digits(4271, 0.4);
        assert_eq!(&text[..1], "4", "第一位先停");
        assert!(!done);
        let (text, done) = rolling_digits(4271, 1.5);
        assert_eq!(text, "4271");
        assert!(done, "四位都停稳了");
    }
}
