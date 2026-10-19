//! P3d① 的验收：命运那副牌按**数据顺序**循环发。
//!
//! PLAN §四 给这条的原话是「命运连踩 4 次能验出顺序」。这里真的连踩四次同一个命运格，
//! 断言的是**卡 id 的序列**而不是牌堆内部的下标 —— 顺序是玩法，它得在流水里读得出来。
//!
//! 机会那副照旧随机，所以同一份文件里两种发法并存。这正是要拍的形状：发法是**牌堆自己的
//! 一个位**（`DeckOrder`），不是抽卡处的一句 `if kind == Fate`。

mod common;

use bevy::app::App;
use bevy::prelude::*;
use common::*;
use game_core::prelude::{CardSet, DataHandles, Detention, GameSession, TileKind};

/// 1 号格被改造成命运格（`retile` 改的是 resource 里那份快照，不必动 JSON）。
const FATE: u32 = 1;

fn turn_count(app: &App) -> u32 {
    app.world().resource::<GameSession>().turn_count
}

/// 命运池的 id 序列。从资产里读而不是写死：加一张牌不该让两条断言一起失败。
fn fate_ids() -> Vec<String> {
    let (app, _) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    let sets = app.world().resource::<Assets<CardSet>>();
    let handle = &app.world().resource::<DataHandles>().fate;
    sets.get(handle)
        .expect("命运牌没加载")
        .0
        .iter()
        .map(|c| c.id.clone())
        .collect()
}

/// 让 P0 反复踩同一格命运 `times` 次，返回每次抽到的卡 id（按抽到先后）。
fn step_on_fate(times: usize) -> Vec<String> {
    let (mut app, p0) = ready_on(0, 1, DecisionPolicy::DeclineAll);
    retile(&mut app, FATE, TileKind::Fate);
    let mut drawn = Vec::new();
    for _ in 0..times {
        // 命运第一张就是「直接入狱」，而被拘留的人回合被跳过 —— 不清掉它就永远踩不到第二次。
        // 这里清它是为了把「发牌顺序」单独问出来；「跳过」那件事由 `tests/turn.rs` 管
        app.world_mut().entity_mut(p0).remove::<Detention>();
        rewind_dice(&mut app);
        make_current(&mut app, p0);
        place(&mut app, p0, FATE - 1);
        let next_turn = turn_count(&app) + 1;
        play_turns(&mut app, next_turn);
        // 流水是累计的，所以每轮重取整条 `Card(P0:…)` 序列，而不是只取最后一条：
        // 断言因此能看出「中间少发了一张」，而不只是「最后一次发错了」
        drawn = ledger(&app)
            .into_iter()
            .filter_map(|l| l.strip_prefix("Card(P0:").map(String::from))
            .map(|rest| rest.trim_end_matches(')').to_string())
            .collect();
    }
    drawn
}

#[test]
fn four_visits_draw_the_first_four_fate_cards_in_data_order() {
    let want: Vec<String> = fate_ids().into_iter().take(4).collect();
    assert_eq!(
        step_on_fate(4),
        want,
        "命运该按数据行序循环发，而不是每局洗一次"
    );
}

/// 抽穿之后回到第一张 —— 「循环」是这条规则的另一半，而它只在越过牌堆长度时才看得见。
#[test]
fn the_cycle_wraps_back_to_the_first_card() {
    let pool = fate_ids();
    let mut want = pool.clone();
    want.push(pool[0].clone());
    assert_eq!(
        step_on_fate(pool.len() + 1),
        want,
        "第一轮该按序发完，之后回到第一张"
    );
}
