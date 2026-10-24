//! 保释（P4③）：走到拘留场所的接入点，花点券把**别人**放出来。
//!
//! 10-20 拍板的形状与「交现金罚金自救」完全不同，这个文件里三件事都跟着它走：
//!
//! - **不动现金**：保释的价钱是点券（[`crate::rules::GameRules::bail_points`]，默认 30）。
//!   与 P12 保释四大恶人（300）是同一个界面、同一条判据 —— 两个价目只是同一张表里的两行，
//!   所以判据、窗口与流水都只有这一份实现。
//! - **主语是路人，不是被关的人**：被困者的棋子停在死胡同尽头格、不会沿环经过接入点，
//!   所以「不许自救」在结构上就成立，不需要再写一条判据。
//! - **它是「要不要」的决策**：挂 `Pending::Bail`，与 P3a「出牌不挂 Pending」相反 ——
//!   这里规则层确实要停下来等一个回答，而且回答之前那一趟的地产结算已经办完了。
//!
//! 演示盘的接入点是地产格（汉口路／徐家汇各挂一条分支），所以「走到监狱门口」在规则上
//! 表现为「落格结算完 → 若那里关着人，再把保释窗口摆出来」（见 `landing::decision_system`）。
//! 将来真棋盘上若有 `type: jail` 的**主环格**，`LandingDomain::Bail` 会直接从落格分派
//! 走到这里（[`settle_bail_landing_system`]），两条入口共用同一张窗口。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::board::{BoardDef, BranchKind, TileIndex};
use crate::landing::{Landing, LandingDomain, Pending, PendingDecision};
use crate::msg::{BailRequested, PlayerBailed, VillainHired};
use crate::player::{Detention, DetentionPlace, TokenPosition, Walker};
use crate::settle::Books;

/// 判一笔保释要读的四样东西。面板与规则层读同一份，才会「灰按钮按下去确实没声音」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BailFacts {
    /// 这个人此刻站在保释窗口前吗（`Pending::Bail` 挂着他）
    pub at_window: bool,
    /// 窗口管的是哪一处
    pub window: DetentionPlace,
    /// 要保的人被关在哪一处（`None` = 他根本没被关着）
    pub target_place: Option<DetentionPlace>,
    /// 保释者的点券
    pub points: u32,
    /// 这一笔要花多少点券
    pub cost: u32,
}

/// 办不成的原因。灰按钮文案与规则层拒绝共用这一张表（同 `ShopDeny`/`MortgageDeny`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BailDeny {
    /// 没站在保释窗口前
    NoWindow,
    /// 那个人不关在眼前这一处（监狱门口放不出医院里的人）
    NotHeldHere,
    /// 点券不够
    NoPoints,
}

impl BailDeny {
    pub fn label(self) -> &'static str {
        match self {
            Self::NoWindow => "要走到拘留所门口",
            Self::NotHeldHere => "这个人不关在这里",
            Self::NoPoints => "点券不够",
        }
    }
}

/// 这一笔办不办得成。顺序是「窗口 → 对象 → 钱」：窗口不对时不必读到别人被关在哪儿。
pub fn bail_op_allowed(f: &BailFacts) -> Result<(), BailDeny> {
    if !f.at_window {
        return Err(BailDeny::NoWindow);
    }
    if f.target_place != Some(f.window) {
        return Err(BailDeny::NotHeldHere);
    }
    if f.points < f.cost {
        return Err(BailDeny::NoPoints);
    }
    Ok(())
}

/// 这一格是不是某条拘留分支的接入点，是的话它管哪一处。
///
/// 演示盘把分支挂在地产格上（`branch` 字段），所以「接入点」与 `TileKind::Jail` 不是
/// 同一件事：前者是拓扑，后者是未来棋盘上那颗专用格。两种入口都要认得出来。
pub fn branch_place(def: &BoardDef, tile: TileIndex) -> Option<DetentionPlace> {
    let kind = def.tile(tile.0 as i64)?.branch.as_ref()?.kind;
    Some(match kind {
        BranchKind::Jail => DetentionPlace::Jail,
        BranchKind::Hospital => DetentionPlace::Hospital,
    })
}

/// 这一处现在关着人吗。给「要不要摆窗口」用：没人可保时不弹一个空窗口把回合停住。
pub fn someone_held(books: &Books<'_, '_>, place: DetentionPlace) -> bool {
    books.detained.iter().any(|d| d.place == place)
}

/// 落格分派的那一支：踩到 `type: jail|hospital` 的主环格时把保释窗口摆出来。
///
/// 没人可保就什么都不做 —— 那一格因此与路面一样直接收摊，而不是弹一个「这里没人」的窗口。
/// P12 起「有没有人」读的是 `Walker` 那一族：**笼中的恶人也算关着人**，
/// 而这正是「同一张窗口两个价目」的入口 —— 窗口不问他是什么，只问他在不在这一处。
pub fn settle_bail_landing_system(
    landing: Res<Landing>,
    held: Query<&Detention, With<Walker>>,
    mut pending: ResMut<PendingDecision>,
) {
    let Some(landed) = landing.0 else {
        return;
    };
    let LandingDomain::Bail(place) = landed.domain else {
        return;
    };
    if !held.iter().any(|d| d.place == place) {
        return;
    }
    pending.offer(landed.player, Pending::Bail(place));
}

/// `bail_op_system` 的输入包：窗口状态、请求、账本与位置。
///
/// `positions` 单独一条而不是塞进 [`Books`]：那条查询在账本里没有别的用户，
/// 而保释是唯一需要「被放出来的人站在哪一格」的结算。按 `Walker` 收（P12）——
/// 恶人也是被放出来的那一个。
///
/// `order` 只在**放出来的是恶人**那一格里动（把他插进轮转，见下面那条注）。
#[derive(SystemParam)]
pub struct BailSink<'w, 's> {
    pub books: Books<'w, 's>,
    pub positions: Query<'w, 's, &'static TokenPosition, With<Walker>>,
    pub order: ResMut<'w, crate::turn::TurnOrder>,
    /// 出场行为的那条入口（P12b/c）。窗口发完就走：强盗抢存款、小偷偷点券
    /// 由 [`crate::villain::on_hired_system`] 下一帧分派 —— 保释窗口不认识恶人。
    pub hired: MessageWriter<'w, VillainHired>,
}

/// 逐笔办保释。窗口留着：同一处关着好几个人时可以一个个放，直到玩家按「离开」。
///
/// **一笔的价钱问的是对象，不是窗口**（[`Books::bail_price`]）：监狱门口同时关着
/// 等保释的恶人（300）与服刑的玩家（30），而这两种人从窗口看出去是同一排候选。
/// 把价钱写成一个窗口级的常数，面板与规则层就都会「对谁都是同一个价」。
pub fn bail_op_system(
    mut commands: Commands,
    pending: Res<PendingDecision>,
    mut requests: MessageReader<BailRequested>,
    mut sink: BailSink,
    mut bailed: MessageWriter<PlayerBailed>,
) {
    let Some((asker, Pending::Bail(place))) = pending.active() else {
        return;
    };
    // `Commands` 的移除在帧末才生效，同一帧的第二条请求会读到「他还关着」——
    // 因此在这一次运行里自己记一份，与 P3d③「同一帧押两遍钱」同一条坑
    let mut freed: Vec<Entity> = Vec::new();

    for r in requests.read() {
        if r.player != asker || freed.contains(&r.target) {
            continue;
        }
        let target_place = sink.books.detained.get(r.target).ok().map(|d| d.place);
        let cost = sink.books.bail_price(r.target);
        let facts = BailFacts {
            at_window: true,
            window: place,
            target_place,
            points: sink.books.points_of(asker),
            cost,
        };
        if bail_op_allowed(&facts).is_err() {
            continue;
        }
        // `spend_points` 自己会再查一次余额；这里重复一次不是防呆，是因为上面那条
        // 判据要给出的「为什么灰着」与真正扣钱必须读同一个数
        if !sink.books.spend_points(asker, cost) {
            continue;
        }
        let villain = sink.books.is_villain(r.target);
        commands.entity(r.target).remove::<Detention>();
        if villain {
            // 恶人出来是**来干活的**：从此他跟着雇主走。两处写必须成对 ——
            // `Hired` 是他「替谁办事」的位，而轮转是他「什么时候动」的事实，
            // 缺一个就是「有个受雇的人从不行动」或「有个没雇主的人在环上抢钱」。
            // 插进轮转走的是资源（当场生效），挂组件走的是 `Commands`（帧末生效）——
            // 中间那一帧他在表里而身上还没有 `Hired`，而那一帧没有任何 system 读他
            commands
                .entity(r.target)
                .insert(crate::villain::Hired { employer: asker });
            if sink.order.insert_after(r.target, asker) {
                debug!("保释：{:?} 接在 {:?} 之后进入轮转", r.target, asker);
            }
            // 出场行为（P12b/c：强盗抢存款、小偷偷点券）走消息分派 —— 这个窗口
            // 刻意不认识任何一种恶人的名字，它只管「放人、收钱、发凭据」
            if let Some(villain) = sink.books.villain_kind(r.target) {
                sink.hired.write(VillainHired {
                    villain: r.target,
                    employer: asker,
                    kind: villain.kind,
                });
            }
        }
        freed.push(r.target);
        let junction = sink
            .positions
            .get(r.target)
            .map(|p| p.tile)
            .unwrap_or(TileIndex(0));
        bailed.write(PlayerBailed {
            player: r.target,
            by: asker,
            place,
            points: cost,
            junction,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn facts(
        at_window: bool,
        window: DetentionPlace,
        target: Option<DetentionPlace>,
        points: u32,
    ) -> BailFacts {
        BailFacts {
            at_window,
            window,
            target_place: target,
            points,
            cost: 30,
        }
    }

    #[test]
    fn the_three_refusals_each_say_something_different() {
        use DetentionPlace as P;
        let cases = [
            (
                facts(false, P::Jail, Some(P::Jail), 100),
                BailDeny::NoWindow,
            ),
            (
                facts(true, P::Jail, Some(P::Hospital), 100),
                BailDeny::NotHeldHere,
            ),
            (facts(true, P::Jail, None, 100), BailDeny::NotHeldHere),
            (facts(true, P::Jail, Some(P::Jail), 29), BailDeny::NoPoints),
        ];
        for (f, want) in cases {
            assert_eq!(bail_op_allowed(&f), Err(want));
        }
        assert_eq!(
            bail_op_allowed(&facts(true, P::Jail, Some(P::Jail), 30)),
            Ok(())
        );
        let labels: Vec<_> = [
            BailDeny::NoWindow,
            BailDeny::NotHeldHere,
            BailDeny::NoPoints,
        ]
        .into_iter()
        .map(BailDeny::label)
        .collect();
        assert_eq!(labels.len(), 3);
        assert!(
            labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                == 3,
            "三种拒绝要说三句不同的话：{labels:?}"
        );
    }
}
