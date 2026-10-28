//! 开发用自动玩家（`NM_AUTOROLL=1` 才装配）——**只替不在键盘前的那位人类按按钮**。
//!
//! 它是**输入侧**的替身，不是渲染层的一部分：只做三件 B5 的按钮同样会做的事 ——
//! 在 `WaitRoll` 发 [`RequestRoll`]（被停留按住时发 [`RequestStay`]，见 [`auto_roll`]）、
//! 在 `WaitDecision` 回答一句话。
//!
//! ## P14a 之后它只剩一半，而且是次要的那一半
//!
//! 它原来揣着那张「每种窗口答什么」的表（`answer_for`）。那张表在 P14a **搬进规则层**
//! （[`game_core::ai::greedy`] / [`game_core::ai::AiHand::emit`]），理由不是整理房间：
//! 那一份是纯函数、只有规则表，**拿不到世界**。装进 core 还换来第二条更硬的：验收那句
//! 「3 AI + 1 人类可完局」从此在 `cargo test -p game_core` 里成立，不必开窗口。
//!
//! | 谁 | 答哪些座位 | 答案出自 |
//! |---|---|---|
//! | core 的 `ai_answer_system` | [`Controller::Ai`] | `AiHand::decide`（读账本打分，P14b 起）→ 没学会的那些格落到 `greedy` |
//! | 这一份脚手架 | 其余（＝人类座位） | `ai::greedy` —— 替的是一个**不在键盘前的人**，不是一份先验 |
//!
//! 两边共用同一个 [`game_core::ai::AiHand::emit`]，所以「答一句话到底要写几条消息」这份知识
//! 世界上只有一份；两边的**判据**从 P14b 起故意不共用，因为那不是一个东西。
//!
//! 两边靠 [`Controller`] 分开，**永远不会同一个窗口收到两份回答**（那的症状不是崩，
//! 是「买地买了两次」）。所以 `NM_AUTOROLL` 从此的含义也换了一句更有用的：
//! **长跑测的是真 AI**，这一份只替那个不在键盘前的人按按钮。
//!
//! B5 的按钮上线后它并没有退役，因为两者用途不同：按钮是给人的，这个是用来跑长对局的。
//! 「打完一局」的验收要几百个回合，没有它就只能靠人手动点几百次。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::ai::AiHand;
use game_core::prelude::{
    Controller, RequestRoll, RequestStay, StaySentence, TurnOrder, TurnPhase, greedy,
};

pub struct AutopilotPlugin;

/// 两次按按钮之间留一拍，好让逐格动画看得清。`NM_AUTOGAP=<秒>` 改它：
/// 跑长对局验收时设成 0，一拍 0.9s 会把几百回合拖成十几分钟。
///
/// 它只管**人类座位**那一侧；AI 座位的间隔是 core 的 `AiCadence`（同一个旋钮驱动，见
/// `main.rs` 的 `boot_cadence`）。分成两个旋钮会被拍出「AI 秒答而人类那一格还在想」那种怪画面。
#[derive(Resource)]
struct AutoGap(f32);

impl AutoGap {
    fn from_env() -> Self {
        Self(
            std::env::var("NM_AUTOGAP")
                .ok()
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.9),
        )
    }
}

impl Plugin for AutopilotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("NM_AUTOROLL").is_none() {
            return;
        }
        info!("自动玩家已开启（NM_AUTOROLL）：只替人类座位按按钮，AI 座位自己答");
        app.insert_resource(AutoGap::from_env());
        let roll_only = auto_roll.run_if(in_state(TurnPhase::WaitRoll));
        // `NM_AUTODECISION=0` 只掷骰不回答：回合会停在第一个 `WaitDecision`，
        // 于是截图能稳定拍到模态本身（B5 的验收图要的就是这个）
        if std::env::var("NM_AUTODECISION").as_deref() == Ok("0") {
            app.add_systems(Update, roll_only);
        } else {
            app.add_systems(
                Update,
                (
                    roll_only,
                    auto_answer.run_if(in_state(TurnPhase::WaitDecision)),
                ),
            );
        }
    }
}

/// 脚手架在 `WaitRoll` 那一格要读的要写的。打包成参数包：加上 P14a 的 `Controller`
/// 之后这条签名就有八个参数，而 clippy 给的「抽出来」正是这仓库的房规（§6.1 坑 8）。
#[derive(SystemParam)]
struct StandinRoll<'w, 's> {
    order: Res<'w, TurnOrder>,
    stays: Query<'w, 's, &'static StaySentence>,
    controllers: Query<'w, 's, &'static Controller>,
    requests: MessageWriter<'w, RequestRoll>,
    stay_requests: MessageWriter<'w, RequestStay>,
}

/// 脚手架在 `WaitRoll` 里的那一下。顶栏那颗按钮怎么答，它就这么答（`ui::send_roll`）：
/// 身上挂着 [`StaySentence`] 的人**发「停留」而不是掷骰**。
///
/// 少这一支是**死锁**而不是「少走一步」：`roll_dice_system` 对被按住的人直接 `return`
/// （那条判据在规则层，不认 UI），于是 `RequestRoll` 每帧被写、每帧被吞，回合机停在
/// `WaitRoll` 不动也不报错。`RequestStay` 的文档注释从 P4④ 起就写着「AI 与恶人也要发
/// 这一条」，恶人那一份 P12 接了，AI 那一份 P14a 接在 core，这一份接的是人类座位。
fn auto_roll(
    time: Res<Time>,
    gap: Res<AutoGap>,
    mut turn: StandinRoll<'_, '_>,
    mut due: Local<f32>,
) {
    let Some(player) = turn.order.current() else {
        return;
    };
    // 电脑开的座位有自己的那条 system（core 的 `ai_roll_system`），这里不许替它按
    if turn.controllers.get(player).is_ok_and(Controller::is_ai) {
        return;
    }
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + gap.0;
    if turn.stays.get(player).is_ok() {
        turn.stay_requests.write(RequestStay { player });
        return;
    }
    turn.requests.write(RequestRoll { player, dice: 0 });
}

/// 脚手架在 `WaitDecision` 里的那一下：**读的是 core 那张表，写的是 core 那只手**。
///
/// 它自己不认任何一种窗口 —— 新增一种窗口时这里不必改（穷尽匹配在 `ai::greedy` 里响），
/// 这一条正是 P14a 把那张表搬进规则层想换来的东西。
fn auto_answer(time: Res<Time>, gap: Res<AutoGap>, mut hand: AiHand<'_, '_>, mut due: Local<f32>) {
    let Some((player, about)) = hand.pending.active() else {
        return;
    };
    if hand.is_ai(player) {
        return;
    }
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + gap.0;
    hand.emit(player, greedy(about, &hand.books.rules));
}

// 这里原来那条「每种窗口都有答案」的守序用例跟着 `answer_for` 一起搬进了规则层
// （`game_core/src/ai.rs` 的 `every_window_has_an_answer_and_the_sticky_ones_are_not_decisions`，
// 外加 `tests/ai.rs` 那条「AI 座位与脚手架不双答」的世界级用例）。
// 搬的理由不只是那张表要走账本：**守「AI 不会把回合机挂死」这条的牙齿不该长在表现层**，
// 而这条 system 现在一个字都不认任何一种窗口，它在这里也没有可断言的东西了。
