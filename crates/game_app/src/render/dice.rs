//! 世界空间骰子：每回合掷骰时，在当前棋子头顶甩出 1–3 颗程序化方骰。
//!
//! 素材是零：骰身是一个 `Cuboid`，六面的点数由同一个小球网格压扁贴在面上。
//! 这样做的理由与棋子那颗球同源 —— 全盘都是 `unlit` 平色 + 程序化几何
//! （`.agents/ARCHITECTURE.md` §七），为一次动画引入 glb 与光照会把整盘配色一起牵走。
//!
//! 时间线归 `render` 自己，**不推进逻辑**（§4.6）：core 在掷骰那一帧就已经把人送去终点，
//! 骰子只是跟着棋子的视觉位置滚一小会儿。落点读的是 `TokenVisual` 的插值结果而不是
//! `TokenPosition`，所以棋子走远时骰子也跟得住 —— 而镜头跟的是棋子，骰子因此始终在画面里。

use std::f32::consts::PI;

use bevy::prelude::*;
use game_core::prelude::DiceRolled;

use super::{Rendered, geom::BoardGeom, token::TokenVisual};

/// 骰身边长（世界单位）。一格 2.0，棋子球半径 0.5 —— 0.62 的骰子在跟随取景下
/// 一眼能数清点数，又不至于盖住整格。
const DIE_SIZE: f32 = 0.62;
/// 骰子停在空中时的中心高度。棋子 sprite 从脚底 0.2 长到 2.0，而正交等轴测把世界
/// +Y 只折算成屏幕的 0.577 —— 2.6–3.0 那颗还压在头发上，4.0 才真正抬到头顶之上。
const DICE_Y: f32 = 4.0;
/// 多颗骰子沿屏幕横向排开。正交等轴测相机在 (1,2,1) 方向看向原点，
/// 屏幕右方向在世界里就是 `(1, 0, -1)`。
const DICE_SPACING: f32 = 0.82;
/// 甩起的高度与滚动时长（秒）。龟速玩家走一格 0.18s，六步约 1.08s，骰子先落地。
const DICE_TOSS: f32 = 0.65;
const TUMBLE_SECS: f32 = 0.75;
/// 停住之后留多久再收（足够看清点数）。
const HOLD_SECS: f32 = 0.55;
/// 收尾缩小的时长。
const FADE_SECS: f32 = 0.24;
/// 滚动的总圈数（弧度）。前半段快、末段平滑收住。
const SPIN_ANGLE: f32 = 6.0 * PI;
/// 点数小球半径与贴面厚度。
const PIP_R: f32 = 0.045;
/// 点数在面上离中心的偏移（骰身边长的一半是 0.31）。
const PIP_SPREAD: f32 = 0.16;

/// 一颗正在滚的骰子。
#[derive(Component)]
pub(crate) struct DiceVisual {
    /// 对应的玩家实体：每帧读他的 `TokenVisual` 来锚定位置。
    player: Entity,
    /// 停住时的朝向（让结果面朝上）。
    rest: Quat,
    /// 已经过的秒数。
    age: f32,
    /// 多颗骰子时的横向错位。
    spread: Vec3,
    /// 找不到棋子时的备用落点（掷骰前的那一格 + 高度）。
    fallback: Vec3,
}

/// 共享的网格与材质。每次掷骰都重建 21 颗点数小球会制造大量临时资产，
/// 所以第一次用到时缓存下来（`Local` 活到本插件销毁为止）。
pub(crate) struct DiceAssets {
    cube: Handle<Mesh>,
    pip: Handle<Mesh>,
    ivory: Handle<StandardMaterial>,
    ink: Handle<StandardMaterial>,
}

impl DiceAssets {
    fn load(meshes: &mut Assets<Mesh>, mats: &mut Assets<StandardMaterial>) -> Self {
        Self {
            cube: meshes.add(Cuboid::new(DIE_SIZE, DIE_SIZE, DIE_SIZE)),
            pip: meshes.add(Sphere::new(PIP_R)),
            ivory: mats.add(StandardMaterial {
                base_color: Color::srgb(0.96, 0.93, 0.85),
                unlit: true,
                ..default()
            }),
            ink: mats.add(StandardMaterial {
                base_color: Color::srgb(0.12, 0.12, 0.15),
                unlit: true,
                ..default()
            }),
        }
    }
}

/// 哪一面在骰子的局部哪个方向。相对面之和为 7（1↔6、2↔5、3↔4）。
fn face_normal(face: u32) -> Vec3 {
    match face {
        1 => Vec3::Y,
        6 => -Vec3::Y,
        2 => Vec3::Z,
        5 => -Vec3::Z,
        3 => Vec3::X,
        4 => -Vec3::X,
        _ => Vec3::Y,
    }
}

/// 让结果面朝上（+Y）的朝向。相机从上方俯看，朝上的那面最易读。
fn rest_rotation(face: u32) -> Quat {
    let n = face_normal(face);
    if n == -Vec3::Y {
        // `from_rotation_arc` 在正相反的两个向量上有 180° 的任意轴，钉死一根。
        Quat::from_rotation_x(PI)
    } else {
        Quat::from_rotation_arc(n, Vec3::Y)
    }
}

/// 一个面上的点数在面内二维坐标里的位置（单位化到 ±1，`PIP_SPREAD` 再乘）。
fn pip_pattern(face: u32) -> &'static [(f32, f32)] {
    match face {
        1 => &[(0.0, 0.0)],
        2 => &[(-1.0, 1.0), (1.0, -1.0)],
        3 => &[(-1.0, 1.0), (0.0, 0.0), (1.0, -1.0)],
        4 => &[(-1.0, -1.0), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)],
        5 => &[
            (-1.0, -1.0),
            (-1.0, 1.0),
            (1.0, -1.0),
            (1.0, 1.0),
            (0.0, 0.0),
        ],
        6 => &[
            (-1.0, -1.0),
            (-1.0, 0.0),
            (-1.0, 1.0),
            (1.0, -1.0),
            (1.0, 0.0),
            (1.0, 1.0),
        ],
        _ => &[],
    }
}

/// 面内二维坐标 → 骰子局部坐标。`u`/`v` 两轴随面换基，法向轴就是 [`face_normal`]。
fn pip_offset(face: u32, u: f32, v: f32) -> Vec3 {
    let half = DIE_SIZE / 2.0;
    let s = PIP_SPREAD;
    match face {
        1 => Vec3::new(u * s, half, v * s),
        6 => Vec3::new(u * s, -half, v * s),
        2 => Vec3::new(u * s, v * s, half),
        5 => Vec3::new(u * s, v * s, -half),
        3 => Vec3::new(half, v * s, u * s),
        4 => Vec3::new(-half, v * s, u * s),
        _ => Vec3::ZERO,
    }
}

/// 把小球压扁成贴片：压扁的轴就是这一面的法向。
fn pip_flatten(face: u32) -> Vec3 {
    match face {
        1 | 6 => Vec3::new(1.0, 0.35, 1.0),
        2 | 5 => Vec3::new(1.0, 1.0, 0.35),
        3 | 4 => Vec3::new(0.35, 1.0, 1.0),
        _ => Vec3::ONE,
    }
}

/// 读 `DiceRolled` 摆骰子。一条消息摆一套：`faces` 前 `dice` 颗各一颗，
/// 每人每回合只会有一次掷骰，所以这里不做去重。
pub(crate) fn spawn_dice_visuals(
    mut commands: Commands,
    geom: Res<BoardGeom>,
    mut rolled: MessageReader<DiceRolled>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut cache: Local<Option<DiceAssets>>,
) {
    let right = Vec3::new(1.0, 0.0, -1.0).normalize();
    for m in rolled.read() {
        let assets = cache.get_or_insert_with(|| DiceAssets::load(&mut meshes, &mut mats));
        let count = m.dice.clamp(1, 3) as usize;
        let fallback = geom
            .tile(m.from.0 as usize)
            .map(|at| Vec3::new(at.at.x, DICE_Y, at.at.z))
            .unwrap_or(Vec3::new(0.0, DICE_Y, 0.0));
        for i in 0..count {
            let face = m.faces[i].clamp(1, 6);
            let spread = right * (DICE_SPACING * (i as f32 - (count - 1) as f32 / 2.0));
            let at = fallback + spread;
            let rest = rest_rotation(face);
            let die = commands
                .spawn((
                    DiceVisual {
                        player: m.player,
                        rest,
                        age: 0.0,
                        spread,
                        fallback,
                    },
                    Mesh3d(assets.cube.clone()),
                    MeshMaterial3d(assets.ivory.clone()),
                    Transform::from_translation(at).with_rotation(rest),
                    Rendered,
                ))
                .id();
            // 六面都贴点数：滚起来时每一面都得像骰子，而不是一圈白板
            commands.entity(die).with_children(|die| {
                for f in 1..=6 {
                    for (u, v) in pip_pattern(f) {
                        die.spawn((
                            Mesh3d(assets.pip.clone()),
                            MeshMaterial3d(assets.ink.clone()),
                            Transform::from_translation(pip_offset(f, *u, *v))
                                .with_scale(pip_flatten(f)),
                        ));
                    }
                }
            });
        }
    }
}

/// 逐帧滚动、收住、消失。锚点读棋子的视觉位置，所以镜头跟着棋子走时骰子也在画面里。
pub(crate) fn animate_dice_visuals(
    mut commands: Commands,
    time: Res<Time>,
    tokens: Query<(&TokenVisual, &GlobalTransform)>,
    mut dice: Query<(Entity, &mut DiceVisual, &mut Transform)>,
) {
    let dt = time.delta_secs();
    for (entity, mut visual, mut tr) in &mut dice {
        visual.age += dt;
        let u = (visual.age / TUMBLE_SECS).min(1.0);
        let anchor = tokens
            .iter()
            .find(|(token, _)| token.player == visual.player)
            .map_or(visual.fallback, |(_, at)| {
                let p = at.translation();
                Vec3::new(p.x, DICE_Y, p.z)
            });
        tr.translation = anchor + visual.spread + Vec3::Y * ((PI * u).sin() * DICE_TOSS);
        // 滚动从大角度收到 0，末段减速，于是停住时正好是 `rest` 那个面朝上
        tr.rotation = Quat::from_axis_angle(
            Vec3::new(0.35, 0.55, 0.75).normalize(),
            SPIN_ANGLE * (1.0 - u).powi(2),
        ) * visual.rest;
        let fade = ((TUMBLE_SECS + HOLD_SECS - visual.age) / FADE_SECS).clamp(0.0, 1.0);
        tr.scale = Vec3::splat(fade);
        if visual.age >= TUMBLE_SECS + HOLD_SECS {
            commands.entity(entity).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_face_lands_pointing_up() {
        for face in 1..=6 {
            let up = rest_rotation(face) * face_normal(face);
            assert!(
                (up - Vec3::Y).length() < 1e-4,
                "{face} 面停住时没有朝上：{up:?}"
            );
        }
    }

    #[test]
    fn opposite_faces_sum_to_seven_and_the_pattern_matches_the_number() {
        for face in 1..=6 {
            assert_eq!(
                pip_pattern(face).len(),
                face as usize,
                "{face} 面的点数画错了"
            );
            // 相对面之和为 7：这里验的是六面都没有分到重复的朝向
            let dirs: Vec<Vec3> = (1..=6).map(face_normal).collect();
            assert!(dirs.contains(&-face_normal(face)));
        }
    }

    #[test]
    fn pips_sit_on_the_face_they_belong_to() {
        for face in 1..=6 {
            let n = face_normal(face);
            for (u, v) in pip_pattern(face) {
                let at = pip_offset(face, *u, *v);
                // 每颗点数都贴在对应那一面上（法向分量等于半边长）
                assert!(
                    (at.dot(n) - DIE_SIZE / 2.0).abs() < 1e-4,
                    "{face} 面有一颗点数离开了自己的面：{at:?}"
                );
                // 并且在面内（二维偏移不超过半边长 + 一点余量）
                let along = at - n * (DIE_SIZE / 2.0);
                assert!(along.length() <= PIP_SPREAD * 2.0f32.sqrt() + 1e-4);
            }
        }
    }
}
