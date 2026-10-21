//! 场景流转：主菜单 ↔ 对局。
//!
//! 用单 App + state（`.agents/ARCHITECTURE.md` §十一 决策 3），不是 Godot 的
//! `get_tree().change_scene_to_file()`。两条纪律：
//! - **进对局才装配世界**：`CoreSet::Boot` 被 `in_state(Scene::Board)` 挡住，
//!   于是 `TurnPhase` 停在 `Boot`，整台回合机在菜单下都是死的。
//! - **回菜单必须清场**：`begin_game_system` 重开一局会再 spawn 一批玩家实体，
//!   旧的没删就会被 `With<PlayerId>` 一起查到 —— 多出两个人，利息付给死人。

use bevy::prelude::*;
use game_core::prelude::{
    ActiveBoard, CoreSet, Corrections, CouponBoxes, GameSession, Gameplay, Landing, MarketClock,
    PendingDecision, Rng, TurnOrder, TurnPhase, WalkClock,
};

use crate::ui::BoardUi;
use crate::ui::menu;

/// 当前在跑哪一屏。`Menu` 是默认值，所以它**不会**触发 `OnEnter` —— 见 `startup_enters_menu`。
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Scene {
    #[default]
    Menu,
    Board,
}

pub struct ScenePlugin;

impl Plugin for ScenePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<Scene>()
            // 装配世界的唯一门：菜单下 `begin_game_system` 不会跑
            .configure_sets(Update, CoreSet::Boot.run_if(in_state(Scene::Board)))
            .add_systems(Startup, request_initial_scene)
            .add_systems(
                OnEnter(Scene::Menu),
                (
                    stop_turn_machine,
                    crate::ui::reset_ui_state,
                    despawn_board_view,
                    menu::spawn_menu_camera,
                    menu::spawn_menu,
                )
                    .chain(),
            )
            .add_systems(OnEnter(Scene::Board), despawn_menu)
            .add_systems(Update, clear_match.run_if(in_state(Scene::Menu)))
            .add_systems(Update, auto_scene_loop);
    }
}

/// `NM_MENULOOP=<秒>`：到点开一局，再隔同样的秒数回主菜单、再开一局。
///
/// 场景来回切换是 B5 唯一「错了不会立刻看得见」的路径 —— 上一局留下的 UI 记忆会让新局
/// 开局即冻结、或第一次报价永远弹不出模态，而画面看着一切正常。没有鼠标可点的 agent
/// 要能跑到这条路径，才有资格说它做完了（与 `NM_PAUSE`、`NM_PICK_TILE` 同一条理由）。
///
/// 后两步（回菜单、再开一局）是补的：从菜单出发时第一步 `set(Menu)` 只是一次自转，
/// 于是「对局→菜单」那半程从来没被踩到过 —— 清场的顺序缺陷就是这么漏出去的（`clear_match`）。
fn auto_scene_loop(time: Res<Time>, mut next: ResMut<NextState<Scene>>, mut step: Local<u8>) {
    let Ok(spec) = std::env::var("NM_MENULOOP") else {
        return;
    };
    let Ok(secs) = spec.trim().parse::<f32>() else {
        return;
    };
    let t = time.elapsed_secs();
    if *step == 0 && t >= secs {
        *step = 1;
        next.set(Scene::Menu);
    } else if *step == 1 && t >= secs * 2.0 {
        *step = 2;
        next.set(Scene::Board);
    } else if *step == 2 && t >= secs * 3.0 {
        *step = 3;
        next.set(Scene::Menu);
    } else if *step == 3 && t >= secs * 4.0 {
        *step = 4;
        next.set(Scene::Board);
    }
}

/// 启动时显式「进入」菜单。
///
/// `Scene::Menu` 已经是默认值，但初始态不产生转移，挂在 `OnEnter(Menu)` 的装配 system
/// 一辈子都不会跑；`NextState::set` 到同一个值**会**补出这一次转移（`startup_enters_menu`
/// 钉住这个语义，别改成 `set_if_different`）。
///
/// `NM_SCENE=board` 直接进对局：agent 没有鼠标可点，而「棋盘装配 + HUD 刷新」这条路径
/// 必须能被截图自检覆盖到（与 `NM_AUTOROLL` 同类的开发开关）。
fn request_initial_scene(mut next: ResMut<NextState<Scene>>) {
    next.set(match std::env::var("NM_SCENE").as_deref() {
        Ok("board") => Scene::Board,
        _ => Scene::Menu,
    });
}

/// 回菜单时收走表现层的东西：棋盘几何、棋子、格名、相机，以及对局内的全部 UI。
///
/// 不清的后果是「新游戏」会再 spawn 一套棋盘压在第一套上，两套格名互相透叠。
fn despawn_board_view(
    mut commands: Commands,
    rendered: Query<Entity, With<crate::render::Rendered>>,
    board_ui: Query<Entity, With<BoardUi>>,
) {
    let count = rendered.iter().count() + board_ui.iter().count();
    for entity in rendered.iter().chain(board_ui.iter()) {
        commands.entity(entity).despawn();
    }
    if count > 0 {
        info!("清场：回收 {count} 个表现层实体");
    }
}

/// 进对局时收走菜单。它压在 `GlobalZIndex(10)` 上，留着会挡住整个棋盘。
fn despawn_menu(mut commands: Commands, menu: Query<Entity, With<crate::ui::MenuPiece>>) {
    for entity in &menu {
        commands.entity(entity).despawn();
    }
}

/// 回主菜单第一半：叫停回合机。只叫停、不收尸，收尸是 [`clear_match`] 的活。
fn stop_turn_machine(mut phase: ResMut<NextState<TurnPhase>>) {
    phase.set(TurnPhase::Boot);
}

/// 回菜单第二半：等回合机真的停稳，再收走上一局的实体与对局资源。
///
/// 为什么不能和 [`stop_turn_machine`] 同帧做完：`Commands` 的删除在系统跑完就生效，而
/// `NextState` 的转移要到**下一帧**的 `StateTransition` 才应用。于是「回菜单」那一帧的
/// 后半程里 `TurnPhase` 还写着 `WaitRoll`，`roll_dice_system` 与 `trade_request_system`
/// 照常跑，可它们要的 `TurnOrder` / `ActiveBoard` 已经没了 —— Bevy 0.20 把这种参数校验
/// 失败直接 panic，真鼠标点「返回主菜单」必崩（exit 101）。停在 `Boot` 之后回合机上
/// 没有任何 system 会跑，这时候收才是安全的。
///
/// 两道门各挡一半：`in_state(Scene::Menu)` 保证不会在重新开局的那一帧动手（开局走
/// `CoreSet::Boot`，那一帧 Scene 已经是 `Board`）；`TurnOrder` 还在则保证整个菜单期间
/// 只收一次。
///
/// 表现层的实体由 `crate::render` 自己的标记负责清理，这里只动规则层的东西。
/// `WalkClock`/`Landing`/`PendingDecision`/`MarketClock`/`CouponBoxes`/`Corrections`
/// 是 `init_resource` 常驻的（结算 system 用 `Res<T>` 拿它们），所以重置成默认值而不是移除。
fn clear_match(
    mut commands: Commands,
    phase: Res<State<TurnPhase>>,
    order: Option<Res<TurnOrder>>,
    game: Query<Entity, With<Gameplay>>,
) {
    if phase.get() != &TurnPhase::Boot || order.is_none() {
        return;
    }
    let count = game.iter().count();
    for entity in &game {
        commands.entity(entity).despawn();
    }
    commands.remove_resource::<ActiveBoard>();
    commands.remove_resource::<TurnOrder>();
    commands.remove_resource::<GameSession>();
    commands.remove_resource::<Rng>();
    commands.insert_resource(WalkClock::default());
    commands.insert_resource(MarketClock::default());
    commands.insert_resource(Landing::default());
    commands.insert_resource(PendingDecision::default());
    // 上一局捡空的点券箱必须在新局重新长出来，否则第二局一整盘都是冷却中
    commands.insert_resource(CouponBoxes::default());
    // 修正登记处里那几条的主语是**上一局那些人**（`Corrections` 按 bearer 实体键）。
    // 槽位带世代，所以死人不会被活人复用成同一个人 —— 行为上无害，但 `NM_DUMP` 会在
    // 主菜单上写出「修正登记处：1 条」而那条指向一个已经 despawn 的实体。面板是查账
    // 工具，说这种话就等于把下一次排查带偏（本文件头那句「回菜单必须清场」的完整含义）
    commands.insert_resource(Corrections::default());
    info!("清场：回收 {count} 个规则层实体，回合机已停在 Boot");
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::state::app::StatesPlugin;

    #[derive(Resource, Default)]
    struct Entered(bool);

    /// 整个菜单装配都挂在 `OnEnter(Scene::Menu)` 上，而 `Menu` 是 `Scene` 的默认值。
    /// 这条测试钉住「`set` 到当前值会补出一次转移」—— 换成 `set_if_different` 或
    /// Bevy 改了这条语义，菜单就会是纯背景色，而程序一个字都不报错。
    #[test]
    fn startup_enters_menu() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StatesPlugin)
            .init_state::<Scene>()
            .insert_resource(Entered(false))
            .add_systems(OnEnter(Scene::Menu), |mut e: ResMut<Entered>| e.0 = true)
            .add_systems(Startup, request_initial_scene);
        app.update();
        assert_eq!(app.world().resource::<State<Scene>>().get(), &Scene::Menu);
        assert!(app.world().resource::<Entered>().0, "OnEnter(Menu) 没触发");
    }

    /// 「回菜单必须清场」那条纪律在 P6b② 之后多了一格：修正登记处里那几条的主语是
    /// **上一局那些人**。这一条不跑整局，只把 `clear_match` 单独走一遍 —— 它要的两道门
    /// （回合机停在 `Boot`、`TurnOrder` 还在）都是能直接摆出来的事实。
    ///
    /// 为什么值得为一句「面板说错话」写测试：这张面板存在的理由就是「查钱为什么是这个数」，
    /// 而主菜单上一句指向死人的「修正登记处：1 条」正好是它最该防的那类假象
    #[test]
    fn clear_match_releases_the_previous_match_s_corrections() {
        use game_core::prelude::{Adjustment, Correction, Scope, Source};
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(StatesPlugin)
            .init_state::<TurnPhase>()
            .insert_resource(TurnOrder::new(vec![]))
            .init_resource::<Corrections>()
            .add_systems(Update, clear_match);
        let bearer = app.world_mut().spawn_empty().id();
        app.world_mut()
            .resource_mut::<Corrections>()
            .register(Correction::always(
                bearer,
                Scope::Any,
                Adjustment::Cancel,
                Source::God,
            ));
        assert_eq!(
            app.world().resource::<Corrections>().len(),
            1,
            "前置：登记处得真挂着一条，否则下面那句「清零」是空跑"
        );

        app.update();

        assert_eq!(
            app.world().resource::<Corrections>().len(),
            0,
            "回菜单那一次清场要把上一局那几条一起收走"
        );
    }
}
