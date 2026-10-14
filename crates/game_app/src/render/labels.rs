//! 格名：世界坐标 → 屏幕投影 → UI 层文本。
//!
//! 这是 §六「遗留待解：`Label3D` 在 Bevy 没有等价物」的 spike 结论，三条路里选了投影那条：
//! - `Text2d` 属 2D 渲染通路，与 3D 不同层，遮挡关系必然错，还得为它多挂一台 `Camera2d`；
//! - 把名字烘进贴图要自己引字体光栅化与图集管理，为十几个静态标签不值；
//! - `Camera::world_to_viewport` + UI `Text`：一台 3D 相机就能同时渲染 3D 与 UI（§6.1 已核实），
//!   中文与 HUD 共用同一套排版，而且永远正对相机 —— 正是 `Label3D` 的 billboard 语义。
//!
//! 代价说清楚：投影出来的标签不参与深度，永远浮在建筑之上。等轴测棋盘上「每格名字都读得到」
//! 比「被前面那栋楼挡住」有用，所以这笔换算是赚的。

use bevy::prelude::*;
use game_core::prelude::ActiveBoard;

use super::{DevFont, Rendered, dim};

/// 一个格名标签：它在世界里的锚点（投影每帧重算，rig 一动标签就跟动）。
#[derive(Component)]
pub(crate) struct TileLabel {
    anchor: Vec3,
}

pub(crate) fn spawn_tile_labels(
    mut commands: Commands,
    active: Res<ActiveBoard>,
    font: Res<DevFont>,
) {
    let b = &active.def;
    for t in &b.tiles {
        // 主环格带下标：调试时「棋子在第几格」与「看到的是哪一格」要能一眼对上
        spawn_label(
            &mut commands,
            &font.0,
            b.world_pos(t.col, t.row),
            format!("{} {}", t.id, t.name),
        );
        if let Some(branch) = &t.branch {
            for bt in &branch.tiles {
                // 死胡同格按 schema 没有 `id`，只写名字
                spawn_label(
                    &mut commands,
                    &font.0,
                    b.world_pos(bt.col, bt.row),
                    bt.name.clone(),
                );
            }
        }
    }
}

fn spawn_label(commands: &mut Commands, font: &Handle<Font>, at: Vec3, text: String) {
    commands.spawn((
        TileLabel {
            anchor: Vec3::new(at.x, dim::LABEL_Y, at.z),
        },
        Node {
            position_type: PositionType::Absolute,
            ..default()
        },
        // 排在 HUD 之下：两者同属 UI 层，靠 `ZIndex` 分先后
        ZIndex(-1),
        // 完全退出命中测试：标签正好压在格子正上方，不忽略它就会吃掉点格详情卡的点击
        Pickable::IGNORE,
        Text::new(text),
        TextFont {
            font: font.clone().into(),
            font_size: FontSize::Px(15.0),
            ..default()
        },
        TextColor(Color::WHITE),
        Rendered,
    ));
}

/// 把每个标签摆到它的锚点在屏幕上的位置；投影不出来的（跑到远/近平面外）直接不排版。
pub(crate) fn project_tile_labels(
    cams: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    mut labels: Query<(&TileLabel, &mut Node, Option<&ComputedNode>)>,
) {
    let Ok((camera, ct)) = cams.single() else {
        return;
    };
    for (label, mut node, sized) in &mut labels {
        let placed = camera.world_to_viewport(ct, label.anchor).ok().map(|at| {
            // 标签中心对齐锚点。`ComputedNode` 是上一帧布局的结果（物理像素），
            // 所以居中有最多一帧的滞后 —— 标签不动，只有相机动，看不出来
            let half = sized.map_or(Vec2::ZERO, |c| c.size() * c.inverse_scale_factor) * 0.5;
            at - half
        });
        let (left, top, display) = match placed {
            Some(at) => (Val::Px(at.x), Val::Px(at.y), Display::Flex),
            None => (node.left, node.top, Display::None),
        };
        // 只在真的变了才写：每帧无谓地改 `Node` 会让整个 UI 布局重算一遍
        if node.left != left || node.top != top || node.display != display {
            node.left = left;
            node.top = top;
            node.display = display;
        }
    }
}
