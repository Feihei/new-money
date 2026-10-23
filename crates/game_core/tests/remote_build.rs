//! 日产建设（P10d②）的验收：踩到不收费、开远程升级窗口，窗口里**只升自己的地**。
//!
//! D 型与 A/B/C 三型的分家写在了数据上（`fee` 缺省 = D 型），所以「踩到收费格」
//! 那一整条管线（摇数、记账、出国）对它不存在 —— 这一条要守的是另一头：
//! 窗口开了之后，升级请求走 [`game_core::settle::Books::upgrade`] 正路
//! （垄断、组内最低层、未满级、付得起全在那里判），别人名下的地静默办不成。
//!
//! 演示盘格子对照（P10d② 重排后）：**4 日产建设**（3 保险公司掷 1 点即踩到，
//! 绑 `stock_002`、无 fee）。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use common::*;
use game_core::prelude::{LandId, Pending};

/// prototype 上的日产建设格。
const BUILDER: u32 = 4;

/// 走到「建设窗口开着」的那一帧：p0 从 3 号掷 1 点停在 4 号。
/// 用 `Hold` 策略 —— 自动应答会答「先不建」把窗口关掉，就什么都观察不到了。
fn open_window() -> (App, Entity) {
    let (mut app, p0) = ready_on(BUILDER - 1, 1, DecisionPolicy::Hold);
    clear_gods(&mut app); // 神明按自己的钟刷出：小财神折半造价、deed 改现金，这里都要干净
    roll(&mut app, p0);
    play_turns(&mut app, 1);
    assert_eq!(
        tile_of(&app, p0),
        BUILDER,
        "前置：从 3 号掷 1 点踩到日产建设"
    );
    assert_eq!(
        pending_of(&app),
        Some((p0, Pending::RemoteBuild)),
        "前置：远程建设窗口该开着"
    );
    (app, p0)
}

/// 踩到不收费、只开窗 —— D 型与 A/B/C 的分家要在账面上看得见。
#[test]
fn stepping_on_the_builder_opens_a_free_window() {
    let (mut app, p0) = open_window();

    assert_eq!(cash_of(&mut app, p0), 20_000, "没有 fee 就没有账单");
    let led = ledger(&app);
    assert!(
        led.contains(&"RemoteOffered(P0)".to_string()),
        "窗口开了要报到（面板与 autopilot 读它弹 UI）：{led:?}"
    );
    assert!(
        !led.iter().any(|l| l.starts_with("Company(")),
        "D 型没有收据 —— 与 A/B/C「踩格必有一张 Company 收据」的形状分家：{led:?}"
    );
}

/// 窗口里只升自己的地：别人名下的、无主的，静默办不成。
///
/// 「静默」不是「没实现」：面板该把办不动的按钮亮成灰的（读同一份归属事实），
/// 规则层丢掉这笔请求与 `build_op_system` 对「建不动的请求」同一条纪律。
///
/// 判据里「整组同色地都在手」这一条（`can_upgrade` → `owns_full_group`）在这里
/// 顺带被钉住：brown 组 = 南京路 + 汉口路，只给他摆一块，自己的地也升不动。
#[test]
fn the_window_builds_only_what_he_owns() {
    let (mut app, p0) = open_window();
    own(&mut app, 0, Some(0), 1, false); // 南京路是他的 —— 但 brown 组还有一块不在手

    // 自己的（半组的）地：同样丢掉 —— 远程建设走的就是加盖正路
    remote_build_ask(&mut app, p0, LandId(0));
    settle(&mut app, 2);
    assert!(
        !ledger(&app).iter().any(|l| l.starts_with("Upgraded(")),
        "半组升不动：{:?}",
        ledger(&app)
    );

    // 补齐整组：自己的地走 `Books::upgrade` 正路，造价 800（物价 1.0）当场扣
    own(&mut app, 1, Some(0), 1, false); // 汉口路也是他的，brown 组齐了
    remote_build_ask(&mut app, p0, LandId(0));
    settle(&mut app, 2);
    assert!(
        ledger(&app).contains(&"Upgraded(P0@0→2)".to_string()),
        "整组在手，自己的地当场升上去：{:?}",
        ledger(&app)
    );
    assert_eq!(cash_of(&mut app, p0), 20_000 - 800, "普通玩家照价付钱");

    let upgraded = |app: &App, who: &str| -> usize {
        ledger(app)
            .iter()
            .filter(|l| l.starts_with(&format!("Upgraded({who}")))
            .count()
    };
    // 别人的地（red 组只有徐家汇一块，归乙）：丢掉
    own(&mut app, 3, Some(1), 1, false);
    remote_build_ask(&mut app, p0, LandId(3));
    settle(&mut app, 2);
    assert_eq!(upgraded(&app, "P0"), 1, "乙名下的地办不成");
    assert_eq!(upgraded(&app, "P1"), 0, "更不该记到乙头上");
    assert_eq!(cash_of(&mut app, p0), 20_000 - 800, "办不成的请求分文不动");

    // 无主的地（yellow 组两块都没人要）：同样丢掉
    remote_build_ask(&mut app, p0, LandId(4));
    settle(&mut app, 2);
    assert_eq!(upgraded(&app, "P0"), 1, "无主地没有「业主升级」这回事");
    assert_eq!(cash_of(&mut app, p0), 20_000 - 800);
}
