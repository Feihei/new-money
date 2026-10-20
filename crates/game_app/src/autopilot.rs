//! 开发用自动玩家（`NM_AUTOROLL=1` 才装配）。
//!
//! 它是**输入侧**的替身，不是渲染层的一部分：只做两件 B5 的按钮同样会做的事 ——
//! 在 `WaitRoll` 发 `RequestRoll`、在 `WaitDecision` 回答 `DecisionChosen`。
//!
//! B5 的按钮上线后它并没有退役，因为两者用途不同：按钮是给人的，这个是用来跑长对局的。
//! 「打完一局」的验收要几百个回合，没有它就只能靠人手动点几百次；B6 的 AI 也会走同一条缝。

use bevy::prelude::*;
use game_core::prelude::{
    Decision, DecisionChosen, Pending, PendingDecision, RequestRoll, TurnOrder, TurnPhase,
};

pub struct AutopilotPlugin;

/// 两次动作之间留一拍，好让逐格动画看得清。`NM_AUTOGAP=<秒>` 改它：
/// 跑长对局验收时设成 0，一拍 0.9s 会把几百回合拖成十几分钟。
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
        info!("自动玩家已开启（NM_AUTOROLL）：一律接受报价");
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

fn auto_roll(
    time: Res<Time>,
    order: Res<TurnOrder>,
    gap: Res<AutoGap>,
    mut requests: MessageWriter<RequestRoll>,
    mut due: Local<f32>,
) {
    let Some(player) = order.current() else {
        return;
    };
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + gap.0;
    requests.write(RequestRoll { player, dice: 0 });
}

fn auto_answer(
    time: Res<Time>,
    pending: Res<PendingDecision>,
    gap: Res<AutoGap>,
    mut answers: MessageWriter<DecisionChosen>,
    mut due: Local<f32>,
) {
    let Some((player, about)) = pending.active() else {
        return;
    };
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + gap.0;
    // 报价类答「接受」，收据/卡面/两种柜台答「已阅」。这一分工不是讲究：柜台只认
    // `Acknowledge`（`landing.rs` 的 match），一律发 `Accept` 的写法会让长跑的自动玩家
    // 第一个踩到银行格（或百货格）的回合就永久停在 `WaitDecision` —— 而且不报错，只是不动了。
    let action = match about {
        Pending::Buy(_) | Pending::Upgrade(_) => Decision::Accept,
        Pending::RentNotice(_)
        | Pending::Card
        | Pending::BankTeller
        | Pending::Shop
        | Pending::Bail(_) => Decision::Acknowledge,
    };
    answers.write(DecisionChosen { player, action });
}
