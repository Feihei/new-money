//! 台词气泡（P16d）：棋子头顶的一朵对话框，正文与语音同时起、同时灭。
//!
//! 位置每帧由棋子的世界坐标投影出来 —— 与 `render/labels.rs` 同一条路（一台 3D 相机
//! 同时渲染 3D 与 UI）。**不参与命中测试**：单击跳过台词那条判据读的是全屏任意左键，
//! 气泡若吃掉点击就成了「看得见、点不到」的假 UI。
//!
//! 生命周期跟着 [`VoiceCaption`]，这里不自己计时：声音有多长由素材说了算，字幕比它
//! 早走或晚走都是错位。唯一的例外是静音模式（`NM_AUDIO=0`），那里没有 sink 可等，
//! 停留钟由 `audio` 自己按字数估（`CaptionClock`）。

use bevy::prelude::*;

use super::theme::{col, layout, size};
use super::{BoardUi, border_color};
use crate::audio::VoiceCaption;
use crate::render::DevFont;
use crate::render::token::TokenVisual;

#[derive(Component)]
pub(crate) struct VoiceBubble;

/// 气泡上正写着哪一句（`VoiceCaption::seq`）。只在这一格变了才重写文字：
/// 每帧无谓地改 `Text` 会让整块 UI 重新排版。
#[derive(Component, Default)]
pub(crate) struct BubbleStamp(u64);

pub(crate) fn spawn_voice_bubble(mut commands: Commands, font: Res<DevFont>) {
    commands.spawn((
        BoardUi,
        VoiceBubble,
        BubbleStamp::default(),
        Node {
            position_type: PositionType::Absolute,
            display: Display::None,
            max_width: Val::Px(layout::CAPTION_MAX_W),
            padding: UiRect::axes(
                Val::Px(layout::CAPTION_PAD[0]),
                Val::Px(layout::CAPTION_PAD[1]),
            ),
            border: UiRect::all(Val::Px(layout::CAPTION_BORDER)),
            border_radius: BorderRadius::all(Val::Px(layout::CAPTION_RADIUS)),
            ..default()
        },
        BackgroundColor(col::BUBBLE),
        border_color(col::BUBBLE_BORDER),
        // 压在模态遮罩（ZIndex 3）之上：收据与报价屏开着时，台词仍然读得到；
        // 又让位于游乐场（5）、暂停（6）与菜单（GlobalZIndex 10）—— 那三屏是另一件事
        ZIndex(4),
        Pickable::IGNORE,
        Text::new(""),
        TextFont {
            font: font.0.clone().into(),
            font_size: FontSize::Px(size::BUBBLE),
            ..default()
        },
        TextColor(col::HINT),
        TextLayout::justify(Justify::Center),
    ));
}

/// 有台词 → 投影到说话者的棋子头顶；没有台词、或棋子不在场上 → 收起来。
pub(crate) fn sync_voice_bubble(
    caption: Res<VoiceCaption>,
    tokens: Query<(&TokenVisual, &GlobalTransform)>,
    cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut bubbles: Query<
        (
            &mut Node,
            &mut Text,
            &mut BubbleStamp,
            Option<&ComputedNode>,
        ),
        With<VoiceBubble>,
    >,
    mut last: Local<Vec2>,
) {
    let Ok((mut node, mut text, mut stamp, computed)) = bubbles.single_mut() else {
        return;
    };
    let Some(player) = caption.player else {
        hide(&mut node);
        return;
    };
    let Some((_, at)) = tokens.iter().find(|(visual, _)| visual.player == player) else {
        hide(&mut node);
        return;
    };
    let Ok((camera, cam_at)) = cams.single() else {
        hide(&mut node);
        return;
    };
    // 说话的人可能不在镜头里（镜头跟着当前回合的人，开局那两秒还在滑过去）。
    // `world_to_viewport` 对屏外的点照样给坐标，所以「投不出来」不够当判据 ——
    // 必须自己问一句落点是否在视口内，否则气泡会悬在画面中间、底下空无一人。
    let bounds = camera
        .logical_viewport_size()
        .unwrap_or(Vec2::new(1280.0, 720.0));
    let anchor = at.translation() + Vec3::Y * layout::CAPTION_LIFT;
    let on_screen = camera
        .world_to_viewport(cam_at, anchor)
        .ok()
        .filter(|p| (0.0..=bounds.x).contains(&p.x) && (0.0..=bounds.y).contains(&p.y));
    let screen = on_screen.unwrap_or(Vec2::new(
        bounds.x / 2.0,
        bounds.y - layout::CAPTION_FALLBACK_BOTTOM,
    ));
    if stamp.0 != caption.seq {
        stamp.0 = caption.seq;
        text.0.clone_from(&caption.text);
    }
    // `ComputedNode` 是上一帧布局的结果（物理像素），而 `Node` 的 left/top 是逻辑像素 ——
    // 与 `render::labels` 的换算同一条。刚显示的那一帧还没量过，沿用上一次的宽高，
    // 否则气泡会在锚点右下角闪一帧。
    let size = computed.map_or(Vec2::ZERO, |c| c.size() * c.inverse_scale_factor);
    if size != Vec2::ZERO {
        *last = size;
    }
    let margin = layout::CAPTION_SCREEN_MARGIN;
    let left = (screen.x - last.x / 2.0).clamp(margin, (bounds.x - last.x - margin).max(margin));
    let top = (screen.y - last.y - layout::CAPTION_GAP)
        .clamp(margin, (bounds.y - last.y - margin).max(margin));
    node.left = Val::Px(left);
    node.top = Val::Px(top);
    node.display = Display::Flex;
}

fn hide(node: &mut Node) {
    if node.display != Display::None {
        node.display = Display::None;
    }
}
