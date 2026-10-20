//! 日历事件（P2b）：圣诞礼物与春节红包。
//!
//! 两者都挂在**玩家回合开始之前**（`OnEnter(TurnStart)`，排在 `begin_turn_system` 前面）：
//! 一天恰好是一个回合，所以「12/25 那天第一个回合之前」在这个游戏里就是「那一天的那个回合
//! 开始之前」，不需要另找「一天的起点」。
//!
//! 三条设计纪律：
//!
//! - **发过没有要留位**（[`GameSession::last_gift_year`] / [`GameSession::last_envelope_year`]）。
//!   没有它，读档回到同一天会再发一轮 —— 与 P15「牌堆顺序必须存索引」同一类账。
//! - **随机性各走专用子流**（`pick_gift` / `pick_envelope`）：收礼物、抽红包都不许挪动
//!   牌序与骰点，否则同一个 `NM_SEED` 在节日之后就复现不出同一局。
//! - **core 不挂 `Pending`**。没人要回答「要不要」，挂一个待决事项只会把回合机停下来；
//!   动画是表现层的事，而且必须可以被跳过（`NM_AUTOROLL` 长跑不能被它挡住）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::calendar::{HolidaySet, date_of};
use crate::deck::{CardId, Shelf};
use crate::hand::{Hand, card_notice, deal_to};
use crate::market::MarketClock;
use crate::msg::{CardGained, GiftGiven, GiftsGiven, RedEnvelope};
use crate::player::PlayerId;
use crate::rng::{Rng, four_digits};
use crate::settle::Books;
use crate::turn::TurnOrder;

/// 发礼物与红包要动的全部东西。收成参数包是房规（§六 坑 8），
/// 而这几样确实是同一件事的两半：谁在场、今天几号、抽什么、往哪记。
#[derive(SystemParam)]
pub struct CalendarFx<'w, 's> {
    books: Books<'w, 's>,
    clock: Res<'w, MarketClock>,
    holidays: Res<'w, HolidaySet>,
    order: Res<'w, TurnOrder>,
    shelf: Shelf<'w>,
    rng: ResMut<'w, Rng>,
    hands: Query<'w, 's, &'static mut Hand, With<PlayerId>>,
    gained: MessageWriter<'w, CardGained>,
    gifts: MessageWriter<'w, GiftsGiven>,
    envelopes: MessageWriter<'w, RedEnvelope>,
}

/// 回合开始前发这一天的礼物与红包。两个事件同一天不可能撞（春节在 1–2 月、圣诞在 12 月）。
pub fn deliver_calendar_events_system(mut fx: CalendarFx<'_, '_>) {
    let today = date_of(fx.clock.tick, fx.books.session.epoch);
    deliver_gifts(&mut fx, today);
    deliver_envelopes(&mut fx, today);
}

fn deliver_gifts(fx: &mut CalendarFx<'_, '_>, today: crate::calendar::Date) {
    if (today.month, today.day) != (12, 25) {
        return;
    }
    if fx.books.session.last_gift_year == Some(today.year) {
        return;
    }
    fx.books.session.last_gift_year = Some(today.year);

    let pool = fx.shelf.items().len() as u32;
    if pool == 0 {
        // 没有道具池就没人收得到礼物。数据错误要在流水里看得见，不走静默成功
        warn!("{} 的圣诞礼物没有可发的卡：道具池为空", today);
        return;
    }
    let limit = fx.books.rules.hand_limit;
    let mut handed = Vec::new();
    for player in present(fx) {
        // 池是整副道具卡（不读货架）：礼物不该被「这一趟没上这款」限制住
        let card = CardId(fx.rng.pick_gift(pool));
        let Ok(mut hand) = fx.hands.get_mut(player) else {
            continue;
        };
        deal_to(&mut hand, limit, card, player, &fx.shelf, &mut fx.gained);
        handed.push(GiftGiven {
            player,
            card: card_notice(&fx.shelf, card),
        });
    }
    info!("圣诞礼物：{} 人各一张", handed.len());
    fx.gifts.write(GiftsGiven {
        year: today.year,
        gifts: handed,
    });
}

fn deliver_envelopes(fx: &mut CalendarFx<'_, '_>, today: crate::calendar::Date) {
    let Some(start) = fx.holidays.spring_festival(today.year) else {
        // 表外年份：农历节不放假，红包也不发。P2b 的显式降级路径
        return;
    };
    if today != start || fx.books.session.last_envelope_year == Some(today.year) {
        return;
    }
    fx.books.session.last_envelope_year = Some(today.year);

    for player in present(fx) {
        // 四位各 0–9，千位也可以是 0 → 0000–9999；0000 是空红包。
        // 那一句折叠与大财神共用（[`four_digits`]），PLAN §三 神明行点名的「别各写一套」
        let leading = fx.rng.pick_envelope_digit();
        let rest = [0u32; 3].map(|_| fx.rng.pick_envelope_digit());
        let amount = four_digits(leading, rest);
        // `Books::credit` 对 `<= 0` 本来就是空操作（连 `CashChanged` 都不发），
        // 所以空红包在这里不必再判一次 —— 但凭据照发，老虎机照播
        fx.books.credit(player, amount);
        fx.envelopes.write(RedEnvelope { player, amount });
    }
    info!("春节红包：{} 已发", today);
}

/// 还在场上的玩家，按**回合序**排。顺序定了，同种子的随机流才对齐 ——
/// 用实体查询序也行（spawn 顺序就是回合序），但那是实现细节，写出来就得一直成立。
fn present(fx: &CalendarFx<'_, '_>) -> Vec<Entity> {
    fx.order
        .order
        .iter()
        .copied()
        .filter(|p| !fx.books.is_bankrupt(*p))
        .collect()
}
