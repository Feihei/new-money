//! 厄运（P3a）：会落到某个玩家头上的**事**，与「钱」是两类账。
//!
//! [`crate::charge`] 管的是「一笔钱在离开钱包之前要过的那条链」，它的输入输出都是金额。
//! 而「被关进监狱 5 天」「原地停留」「向后转」不落金额 —— 但它们同样需要一个承受者、
//! 同样可能被抵消或转嫁，所以**形状必须与 `Charge` 一致**：
//!
//! ```text
//! Affliction { event, victim, what }   ↔   Charge { kind, payer/payee, cancellable, transferable }
//! 门问「免不免、转给谁」                ↔   charge::stage 里的 Cancel / TransferTo
//! ```
//!
//! ## 两个位今天在一张表上
//!
//! 这一族以前自带一张 `AfflictDomain::forgivable` 穷尽表，而钱那一侧另有一张
//! `charge::transferable`。**P13b① 把两张收成一张**：读数全部搬到 [`crate::passive`]
//! 那张**按具体事件逐行**答的表上，这里只剩「哪一行 + 落在谁头上 + 做什么」。
//!
//! 收成一张的理由是最尖的那一格：命运牌的出国旅游可以免、可以转，而踩到联合航空那一下
//! 两样都不可以 —— 它们是同一个 [`DetentionPlace::Away`](crate::player::DetentionPlace)、
//! 两个行。按「一族事」建键答不出这种差别，所以 [`Affliction`] 带的是**行**而不是族
//! （行怎么来的见 [`Affliction::detain`] 那条注）。
//!
//! 顺带一笔旧债：旧枚举里那一格 `Damage`（怪兽卡、飞弹、新闻天灾）**表上有、路上没有** ——
//! 全工程没有任何一处构造它，损害走的是 [`Books::damage`](crate::settle::Books::damage)。
//! ① 把它从枚举里删掉，答案留在表上那一行 [`PassiveEvent::Hazard`](crate::passive::PassiveEvent::Hazard)
//! （「不许留一格假答案」，PLAN §五 第 1 条）。
//!
//! ## ② 收口：这里只剩「落地」这一个动作
//!
//! P3a～P13 的形状是「就地构造 → 就地挡 → 就地写状态」，五处各自重复那一遍
//! （`deck.rs` 四处、`magic.rs` 一处）。10-27 拍的那条「被动卡没有出牌这个动作」要求
//! **事件落地之前过一个咽喉**，于是那五处的三段式收成两段：
//!
//! ```text
//! 构造 Affliction ──→ crate::passive::affair(...)   ← 门：神挡？免罪？嫁祸？
//!                          │ 答完了才落
//!                          └──→ land(sink, aff)     ← 本模块唯一的写状态出口
//! ```
//!
//! 旧的 `Insulation` 组件因此**删除**：它本来是「玩家挑一笔厄运用掉一张免罪卡」的容器
//! （打出去挂上位），而现在免罪卡**留在手里就是资格**，门在花掉那一刻直接从
//! [`Hand`](crate::hand::Hand) 里摘一张。抄一份层数到组件上就是同一副牌两处数 ——
//! P6b② 那次「神明的免疫覆盖掉玩家两层护身」正是这个冗余长出来的坑。
//!
//! ⚠️ 神明那一半**一直**不在这个模块里：它现读那位神是谁（[`crate::gods::resists`]），
//! 与 `Presence` 同一条纪律 —— 会过期的账不留副本。今天这一句仍然成立，只是问它的地方
//! 从五处收成了一处（[`crate::passive`] 的门）。所以「魔法屋的十二条不受任何卡与神影响」
//! 那半条不是这张表答的，是 `magic.rs` 里那十条**根本不构造** [`Affliction`]。

use bevy::prelude::*;

use crate::deck::CardSink;
use crate::landing::detain;
use crate::msg::{EffectOutcome, PlayerDetained};
use crate::passive::{PassiveEvent, trip_of};
use crate::player::{DetentionPlace, MoveCap, StaySentence, Term};

/// 这一件事落下去要做的那个动作。**与「哪一行」分开**是必须的：
/// [`PassiveEvent::Hold`] 那一行管着三件事（停留／少动／掉头），而行只答两列（可免／可转）。
/// 合成一个枚举位的话，「停留 3 回合」与「掉头」就只能靠调用方记住自己传了哪个 ——
/// 而 [`Affliction`] 要活得比那一帧久（它在门的队列里等人回答），它必须自己说得出要做什么。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Afflict {
    /// 送进某一处 N 天（四个去处共用这一支，差别全在 [`DetentionPlace`] 那两条映射上）
    Detain { place: DetentionPlace, turns: u32 },
    /// 原地停留 N 回合（人还在场上，地照收租）
    Stay { turns: u32 },
    /// 接下来 N 个掷骰回合只走 1 格
    Slow { turns: u32 },
    /// 绕行方向掉头
    Reverse,
}

/// 一件要落到某人头上的事。
///
/// 带的不是「后果的种类」而是**表上的那一行**：同一句「离场三天」，从命运牌来与从
/// 联合航空来是两行，而这两行的两列答案相反。行由造这件事的那一处点名，
/// 不许从 payload 反推（见 [`trip_of`] 的注）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Affliction {
    /// 表上的哪一行（决定「可免」「可转嫁」两列）
    pub event: PassiveEvent,
    /// 此刻该挨它的人。嫁祸改的就是这一格，所以它是**门答出来的**而不是数据写的
    pub victim: Entity,
    /// 这一件是**法事**（女巫的那四条）而不是一件厄运。神明吃不吃这一件全看这一格：
    /// 大福神的免疫与衰神的天数加倍是同一句判据的两句话（P13 定稿：祂豁免的是厄运，
    /// 女巫的法术不是厄运；她的 3 天写在 [`MAGIC_DAYS`](crate::magic::MAGIC_DAYS) 这张表上，
    /// 不在「落到某人头上的厄运」那条尺上）。
    ///
    /// 为什么挂在事上而不是由 [`crate::passive::Tail`] 现场问：那条来路同时管着免疫与
    /// 加倍两处，把它抄成这里的一格，是为了让**落地那一句**自己说得出倍不乘 ——
    /// `land` 拿不到来路（五处来路共用它），而两处各问一次就是两处可修一半。
    pub is_ritual: bool,
    pub what: Afflict,
}

impl Affliction {
    /// 把一个人送去某一处 N 天。**行按去处数**，而那只对得上「卡／女巫」这一族来源：
    /// 命运牌、陷害卡、女巫的坐牢住院都从这里进，所以它们拿到的是可以免的那几行。
    /// 踩到联合航空那一条**不走这一句**，它自己点名
    /// [`PassiveEvent::AirlineTrip`](crate::passive::PassiveEvent::AirlineTrip)。
    pub fn detain(victim: Entity, place: DetentionPlace, turns: u32) -> Self {
        Self::at(trip_of(place), victim, Afflict::Detain { place, turns })
    }

    /// 原地停留 N 回合（停留卡、新闻「豪雨」、女巫「原地停留」）。
    pub fn stay(victim: Entity, turns: u32) -> Self {
        Self::at(PassiveEvent::Hold, victim, Afflict::Stay { turns })
    }

    /// 接下来 N 个掷骰回合只走 1 格（乌龟卡，P5）。与 [`Self::stay`] 同一行而两件事：
    /// 一个让人不动，一个让人走不远。
    pub fn slow(victim: Entity, turns: u32) -> Self {
        Self::at(PassiveEvent::Hold, victim, Afflict::Slow { turns })
    }

    /// 行进方向被反转（向后转）。
    pub fn reversal(victim: Entity) -> Self {
        Self::at(PassiveEvent::Hold, victim, Afflict::Reverse)
    }

    /// 点名一行并落在 `victim` 头上。给**来源自己点名行**的那两处用（联合航空、住店）：
    /// 它们不是 trip 那一族，从去处数会数出「可以免」的那一行，而那句是假的 ——
    /// 踩到航空公司那一格的人没有「这一次可以被免」这回事（10-27 的例外）。
    pub fn named(event: PassiveEvent, victim: Entity, what: Afflict) -> Self {
        Self::at(event, victim, what)
    }

    /// 把一件已经点好行的事改成**法事**。只有
    /// [`crate::passive::affair_of`] 调用它 —— 那一处才知道这一步是卡面动词还是女巫的法术。
    pub const fn ritual(self) -> Self {
        Self {
            is_ritual: true,
            ..self
        }
    }

    /// 上面几个构造函数之外造不出 [`Affliction`]：这一句是私有的。
    /// 「这一行是哪件事」有两种来源 —— 从去处数出来（[`trip_of`]）与点名，
    /// 而把这一句开放出去，调用方就能挑一个跟手上那件事对不上号的行，
    /// 那种错的形状是「命运牌的出国忽然免疫了」或反过来。
    const fn at(event: PassiveEvent, victim: Entity, what: Afflict) -> Self {
        Self {
            event,
            victim,
            what,
            is_ritual: false,
        }
    }
}

/// 落在 `victim` 头上的厄运天数，过一遍「那位神说延长几倍」。
///
/// 三个调用点（拘留／停留／乌龟）共用这一句，所以倍数只写在
/// [`crate::gods::doom_multiplier`] 一处 —— 与钱那一侧「乘数只在 `money_corrections`
/// 里写一次」是同一条纪律。
fn doom_days_for(sink: &CardSink<'_, '_>, victim: Entity, days: u32) -> u32 {
    let god = sink.possessions.get(victim).ok().map(|held| held.god);
    crate::gods::doom_days(god, days)
}

/// 这一件落下去要写几天。**法事不在这条尺上**（P13 定的那半条，与大福神挡不挡女巫同一条
/// 判据）：她的 3 天是 `MAGIC_DAYS` 那张表给的数，而神明量的是「落到某人头上的厄运」。
/// 把两句话合成一处而不是让调用方各自记：漏掉哪一句都不会报错，只会多两天或少两天。
fn days_of(sink: &CardSink<'_, '_>, aff: &Affliction, written: u32) -> u32 {
    if aff.is_ritual {
        written
    } else {
        doom_days_for(sink, aff.victim, written)
    }
}

/// **把一件事落到世界上** —— 全工程写这些状态的唯一出口。
///
/// 门问完之后交回这里，所以「挡住」与「没挡住」两条路都只在这一处落地（那正是 ② 收口的
/// 全部内容：以前五处各写一遍，于是「命运里的住院三天和新闻里的不一样长」这类错有五个
/// 可以各修一半的地方）。返回的是凭据（[`EffectOutcome`]），世界状态已经改完；
/// 谁调用谁决定那份凭据交给哪条消息（卡面收据、法事名单、新闻 hits）。
///
/// 就地写而不是 `Commands` 沿用 P3d③／P3d④c 那两条：同一帧可能来第二件事
/// （第二张乌龟卡要看得见第一张的剩余值，而 `Commands` 的延迟写读不到第一条），
/// 只有组件的**插入**（他身上本来没有）走 `Commands`。
pub fn land(sink: &mut CardSink, aff: &Affliction, source: &str) -> Vec<EffectOutcome> {
    match aff.what {
        Afflict::Detain { place, turns } => {
            let days = days_of(sink, aff, turns);
            // 接入点与死胡同格由 `detain` 一处算（四个去处共用）。它查不到那条分支时
            // 什么都不改并返回 `None` —— 与「数据写了个棋盘上没有的格名」同一条纪律：
            // 不猜一个格子，也不回退到起点。返回的那条拘留**可能是原来更久的那一张**，
            // 所以凭据一律用它给的值，而不是算完的那个倍数
            let Some((held, junction, end)) = detain(
                &mut sink.commands,
                &mut sink.positions,
                &mut sink.books.detained,
                &sink.books.active.def,
                aff.victim,
                place,
                // 卡面写几天就是几天（10-20 拍板「天数只来自卡面」）。恶人专用的那一支
                // `UntilBailed` 从这里出不去：卡打不到恶人，而「踩到某格就被关」也没有
                Term::Turns(days),
            ) else {
                debug!(
                    "这张棋盘没有「{}」那条分支，把 {} 送去那里这条路落空",
                    place.label(),
                    source
                );
                return Vec::new();
            };
            sink.detained.write(PlayerDetained {
                player: aff.victim,
                place: held.place,
                junction,
                end,
            });
            vec![EffectOutcome::Detained {
                place: held.place,
                // 读的是 `detain` 返回的那一张而不是 `days`：更久的旧账赢。
                // 「赢的会不会是笼中那一张（读不出数）」这条路今天走不到 ——
                // 卡与新闻都打不到恶人（`CardTarget` 与 `NewsAudience` 的承受者都是玩家），
                // 所以兜底取的正是卡上写的那个数
                turns: held.term.turns().unwrap_or(days),
                junction,
            }]
        }
        Afflict::Stay { turns } => {
            let days = days_of(sink, aff, turns);
            // 字面口径（P4④）：写 1 就停 1 回合，不再有「判 3 = 跳 2」那半条 ——
            // 计数由玩家按「停留」时扣（`turn::take_stay_system`），这里只写状态。
            // **取较大**而不是相加或覆盖：给一个已经停留 2 回合的人再落一个 1，
            // 覆盖等于替他减刑 —— 惩罚卡不该有让人早走的副作用
            let left = match sink.stays.get_mut(aff.victim) {
                Ok(mut held) => {
                    held.turns_left = held.turns_left.max(days);
                    held.turns_left
                }
                Err(_) => {
                    sink.commands
                        .entity(aff.victim)
                        .insert(StaySentence { turns_left: days });
                    days
                }
            };
            vec![EffectOutcome::Stayed { turns: left }]
        }
        Afflict::Slow { turns } => {
            let days = days_of(sink, aff, turns);
            // 与停留同一条形状：就地写 + 取较大，两张乌龟卡不叠加、短的不覆盖长的
            // （惩罚卡不该替人加速）。计数在每次掷骰时扣一格（`turn::roll_dice_system`）
            let left = match sink.caps.get_mut(aff.victim) {
                Ok(mut held) => {
                    held.turns_left = held.turns_left.max(days);
                    held.turns_left
                }
                Err(_) => {
                    sink.commands
                        .entity(aff.victim)
                        .insert(MoveCap { turns_left: days });
                    days
                }
            };
            vec![EffectOutcome::Slowed { turns: left }]
        }
        Afflict::Reverse => {
            // 就地翻，不走 `Commands`：同一帧两条转向**该**翻两次（翻两次=原样），
            // 而延迟写会让第二条读不到第一条 —— 与 P3d③ 那个「同一帧押两遍钱」同一条坑
            let Ok(mut heading) = sink.headings.get_mut(aff.victim) else {
                warn!(
                    "{source} 要替 {:?} 掉头，可他身上没有 Heading 组件（spawn_players 漏挂了？）",
                    aff.victim
                );
                return Vec::new();
            };
            *heading = heading.flipped();
            vec![EffectOutcome::Reversed { heading: *heading }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn victim() -> Entity {
        Entity::from_raw_u32(3).expect("索引不该撞上保留值")
    }

    /// 构造函数把「行」与「动作」配对各钉一格。配错的那一格（比如停留配成 `JailTrip`）
    /// 在门里会问出一句「用免罪卡吗？」—— 而 P13 立的口径正是停留不吃它。
    #[test]
    fn each_constructor_pairs_one_row_with_one_action() {
        let jail = Affliction::detain(victim(), DetentionPlace::Jail, 5);
        assert_eq!(jail.event, PassiveEvent::JailTrip);
        assert_eq!(
            jail.what,
            Afflict::Detain {
                place: DetentionPlace::Jail,
                turns: 5
            }
        );
        // 三个去处三行，而「出国」那一行与联合航空那一行不是同一行（见下面的用例）
        assert_eq!(
            Affliction::detain(victim(), DetentionPlace::Away, 3).event,
            PassiveEvent::AwayTrip
        );
        for aff in [
            Affliction::stay(victim(), 1),
            Affliction::slow(victim(), 3),
            Affliction::reversal(victim()),
        ] {
            assert_eq!(aff.event, PassiveEvent::Hold, "{:?} 归错行", aff.what);
        }
    }

    /// P13 定稿的免疫口径（10-25 拍板）+ P13b① 的那次并表：**只有「被关起来」那一族吃免罪卡**，
    /// 而「被关起来」在表上是三行（坐牢／住院／出国），不是旧枚举里那**一族**。
    ///
    /// 这一条从旧表 `only_a_trip_away_is_forgivable` 改过来的地方正是形状：过去逐**族**问
    /// 「护身吃不吃」，现在逐**行**问 —— 因为同一族里已经有答案相反的两行，而族这个键装不下它。
    #[test]
    fn only_a_trip_away_is_forgivable() {
        for place in [
            DetentionPlace::Jail,
            DetentionPlace::Hospital,
            DetentionPlace::Away,
        ] {
            assert!(
                Affliction::detain(victim(), place, 3).event.forgivable(),
                "{place:?} 该吃免罪卡 —— 免罪卡那句『抵消一次厄运』说的就是这一族"
            );
        }
        let airline = Affliction::named(
            PassiveEvent::AirlineTrip,
            victim(),
            Afflict::Detain {
                place: DetentionPlace::Away,
                turns: 3,
            },
        );
        let lodging = Affliction::named(
            PassiveEvent::Lodging,
            victim(),
            Afflict::Detain {
                place: DetentionPlace::Hotel,
                turns: 2,
            },
        );
        for aff in [
            Affliction::stay(victim(), 1),
            Affliction::slow(victim(), 3),
            Affliction::reversal(victim()),
            airline,
            lodging,
        ] {
            assert!(
                !aff.event.forgivable(),
                "{:?} 不该被免罪卡挡掉 —— 它是处境变了或压根不吃卡，不是一件灾落下来",
                aff.event
            );
        }
        // 最尖的那一格：同去处、不同来源，两行答案相反
        assert_eq!(trip_of(DetentionPlace::Away), PassiveEvent::AwayTrip);
        assert!(PassiveEvent::AwayTrip.forgivable());
        assert!(!PassiveEvent::AirlineTrip.forgivable());
        // 而点名那一句不是装饰：它造出来的东西与从去处数出来的那个**不相等**，
        // 所以门翻表时读到的是相反的答案。这一格若相等，② 的所有用例都会假装通过
        assert_ne!(
            airline,
            Affliction::detain(victim(), DetentionPlace::Away, 3)
        );
    }

    /// 落地那一句读的是 `what` 而不是行 —— 行只答两列，它说不出「做什么」。
    /// [`PassiveEvent::Hold`] 那一行管三件事就是这个形状的唯一理由，
    /// 所以三件事必须各自说得出（塌成一件，乌龟卡与停留卡在 `land` 里就分不开）。
    #[test]
    fn one_row_can_still_carry_three_different_actions() {
        let actions = [
            Affliction::stay(victim(), 1).what,
            Affliction::slow(victim(), 3).what,
            Affliction::reversal(victim()).what,
        ];
        assert_eq!(actions[0], Afflict::Stay { turns: 1 });
        assert_eq!(actions[1], Afflict::Slow { turns: 3 });
        assert_eq!(actions[2], Afflict::Reverse);
        assert_ne!(actions[0], actions[1], "停留与少动塌成一件，乌龟卡就白买了");
        assert_ne!(actions[1], actions[2]);
        // 拘留那一族的四格去处也各自成立（`land` 把 place 原样交给 `detain`）
        for place in [
            DetentionPlace::Jail,
            DetentionPlace::Hospital,
            DetentionPlace::Away,
            DetentionPlace::Hotel,
        ] {
            assert_eq!(
                Affliction::detain(victim(), place, 3).what,
                Afflict::Detain { place, turns: 3 }
            );
        }
    }
}
