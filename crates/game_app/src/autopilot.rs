//! 开发用自动玩家（`NM_AUTOROLL=1` 才装配）。
//!
//! 它是**输入侧**的替身，不是渲染层的一部分：只做两件 B5 的按钮、B6 的 AI 同样会做的事 ——
//! 在 `WaitRoll` 发 `RequestRoll`、在 `WaitDecision` 回答 `DecisionChosen`。
//! 留着它是因为 B4 的验收要看「棋子在走、楼在长」，而那需要真的在打对局；
//! 掷骰按钮上线（B5）后就该删掉它。

use bevy::prelude::*;
use game_core::prelude::{
    Decision, DecisionChosen, PendingDecision, RequestRoll, TurnOrder, TurnPhase,
};

pub struct AutopilotPlugin;

impl Plugin for AutopilotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("NM_AUTOROLL").is_none() {
            return;
        }
        info!("自动玩家已开启（NM_AUTOROLL）：一律接受报价，B5 的按钮会取代它");
        app.add_systems(
            Update,
            (
                auto_roll.run_if(in_state(TurnPhase::WaitRoll)),
                auto_answer.run_if(in_state(TurnPhase::WaitDecision)),
            ),
        );
    }
}

/// 两次动作之间留一拍，好让逐格动画看得清。
const GAP_SECS: f32 = 0.9;

fn auto_roll(
    time: Res<Time>,
    order: Res<TurnOrder>,
    mut requests: MessageWriter<RequestRoll>,
    mut due: Local<f32>,
) {
    let Some(player) = order.current() else {
        return;
    };
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + GAP_SECS;
    requests.write(RequestRoll { player });
}

fn auto_answer(
    time: Res<Time>,
    pending: Res<PendingDecision>,
    mut answers: MessageWriter<DecisionChosen>,
    mut due: Local<f32>,
) {
    let Some((player, _)) = pending.active() else {
        return;
    };
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + GAP_SECS;
    answers.write(DecisionChosen {
        player,
        action: Decision::Accept,
    });
}
