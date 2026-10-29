//! 地产结算流水线那一段：无主地报价、他人地收租／服务费、自有地升级／建设报价。
//!
//! 对应 Godot `turn_manager.gd:268-311`（`_handle_property_landing`）+ `property_system.gd`
//! 的四个动作。判定与算账都在 `economy` 的纯函数里，这里只做「查组件 → 调判定 →
//! 改组件 → 发消息」四步，所以 `test_property_system` 那 20 条 GUT 用例全部
//! 落在 `economy.rs` 的表驱动单测上，不依赖 ECS。
//!
//! P10c 起这一段多出一对分叉，判据都挂在**设施**（[`Facility`]）上：
//! - 踩到**别人**带设施的地 → 走 [`crate::service`] 原型库收一笔事件费（购物中心摇档、
//!   旅馆住宿、加油站油费），不再查租金表 —— 商业地的玩法是「设施」而不是「过路」；
//! - 踩到**自己的空商业地** → [`Pending::Build`] 开建设窗口，而不是升级报价。
//!   4 代里商业用地不盖平房、只建设施，所以「升级」这条分支对商业地是死路；
//!   已有设施的商业地踩回自己家则无事发生（设施不可再升级，也没有「逛自己的店」）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::auction::Gavel;
use crate::board::{BuildingKind, LandType};
use crate::charge::{Charge, ChargeKind};
use crate::economy;
use crate::land::{Facility, LandId};
use crate::msg::{BuildOffered, BuildRequested, BuyOffered, FacilityBuilt, UpgradeOffered};
use crate::passive::{GateDesk, Job, Step, Tail, submit};
use crate::player::Mount;
use crate::service;
use crate::settle::Books;
use crate::turn::LastRoll;

use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};

/// 落格结算要发出去的三种报价：买地、升级、建设。合成一个参数包是房规（§六 坑 8）——
/// 它们说的是同一件事的三个分支，散装就把 [`settle_property_landing`] 顶到 clippy 的
/// 参数上限，而那条警告在这里正是要吃掉「一条 system 要发四种消息」这个信号。
///
/// ⚠️ ③ 之前这里还有第四、第五格（`rents`／`rerouted`）。**收据的实付额只有落地那一刻
/// 才知道**，而落地发生在被动卡那道门里（问完才扣），所以那两条 writer 搬进了
/// [`GateDesk`](crate::passive::GateDesk) —— 留在这里就等于两处各写一次同一条收据。
#[derive(SystemParam)]
pub struct PropertySink<'w> {
    pub offers: MessageWriter<'w, BuyOffered>,
    pub upgrades: MessageWriter<'w, UpgradeOffered>,
    pub builds: MessageWriter<'w, BuildOffered>,
}

/// 踩到地产格：三种归属三种走法（无主 / 自己 / 别人），与 Godot 完全一致。
///
/// ⚠️ 这张包里没有 `Books`／`Commands`／`PendingDecision`，也没有那条骰面 ——
/// ③ 之后它们都从 [`GateDesk`] 里借（同一 system 里两份 `Books` 会在参数建立期直接撞，
/// §6.1 坑 33）。`last_roll` 是唯一的例外：它是只读的世界读数，与门不相交。
pub fn settle_property_landing(
    mut gate: GateDesk<'_, '_>,
    landing: Res<Landing>,
    last_roll: Res<'_, LastRoll>,
    mut sink: PropertySink<'_>,
) {
    // 只有地产格带土地；其余格（含 `land_id` 为 `None` 的坏数据）直接跳过
    let Some(Landed {
        player,
        land: Some(land),
        domain: LandingDomain::Property,
        ..
    }) = landing.0
    else {
        return;
    };
    let Some(view) = gate.sink.books.view(land) else {
        return;
    };
    let Some(self_id) = gate.sink.books.id_of(player) else {
        return;
    };
    match view.owner {
        // 无主 → 挂牌待决。Godot 也是「不管买不买得起都弹一张」（`confirm_buy` 里
        // 才做二次判定），差别在于这里买不起就是 `None`，不会静默扣成负数。
        //
        // 但**必败**那一格不给弹（P6b② 大衰神）：报价与成交读的是同一句
        // [`Books::buy_price`]，弹一张按下去必然失败的模态，比不弹更糟 ——
        // 玩家会以为是自己按错了，而不是身上那位神不让买。
        None if gate.sink.books.acquire_denied(player) => {
            let god = gate.sink.books.god_of(player).map(|held| held.god);
            debug!(
                "{} 想踩下这块无主地，但{}在场 —— 买地建屋必败",
                self_id.0,
                god.map_or(String::new(), |g| g.label().to_string())
            );
        }
        None => {
            // 土地公踩到无主地**不问价**（判据在 `Books::takes_landing`，过户由排在后面的
            // `settle_land_deity_landing_system` 做）。这一句必须问在弹窗口之前：先弹一张
            // 「是否购买此地」、答完才发现地已经归他，那是一个答完就失效的窗口
            if gate.sink.books.takes_landing(player) {
                debug!("无主地 L{} 不问价：他身上是土地公", land.0);
            } else {
                gate.pending.offer(player, Pending::Buy(land));
                sink.offers.write(BuyOffered { player, land });
            }
        }
        // 自己的地：住宅走升级报价，商业地走建设窗口（P10c）。
        // 两条判据互相独立 —— 升级看「楼还有几层可盖」，建设看「这块地上还什么都没有」
        // —— 所以各占一格，而不是一条 if 链。两个分支收进 `offer_own_land`，
        // 因为「停在自家地上」也要问同一句话（P14d② 的停留）
        Some(owner) if owner == self_id => {
            let books = &gate.sink.books;
            offer_own_land(
                books,
                player,
                land,
                &view,
                &mut gate.pending,
                &mut sink,
                true,
            );
        }
        // 别人的地：带设施 → 服务费（原型库当场结）；空商业地 → 一文不收
        // （设施还没立起来，不产生任何账 —— 连收据都不该有）；住宅 → 照旧收租。
        Some(owner) => {
            let Some(owner_entity) = gate.sink.books.holder(owner) else {
                return;
            };
            let facility = gate.sink.books.facility_of(land);
            if let Some(kind) = facility {
                settle_service(&mut gate, &last_roll, player, land, owner_entity, kind);
                return;
            }
            let Some(def_land) = gate.sink.books.active.def.land(land.0) else {
                return;
            };
            if def_land.land_type == LandType::Commercial {
                return;
            }
            let rent = gate.sink.books.rent_of(land);
            if rent <= 0 {
                return;
            }
            // 间谍转付（P12d）：判据在账本里（[`Books::rent_rerouted_to`]），
            // 收款人从地主换成雇主 —— 钱从结算那一刻就走对口袋，不做两段账。
            // 「无功而返」在这里没有形状：判据不过时 `None`，租金照付地主，
            // 世界看起来与没有间谍时一模一样 —— 那正是这句话的本意。
            //
            // ⚠️ 这一句读的是**踩格的那一位**（`player`），交进门之后不改：间谍探的是
            // 「谁在这块地上踩过交费」，与「此刻谁替谁付」是两件事（③ 落地时想过要不要
            // 按新付款人重算，答案是不要 —— 那等于让一张嫁祸卡改变第三个人（雇主）的账）
            let spy = gate.sink.books.on_duty_spy();
            let rerouted_to = gate.sink.books.rent_rerouted_to(player, land, owner_entity);
            let payee = rerouted_to.unwrap_or(owner_entity);
            let rerouted = match (spy, rerouted_to) {
                (Some((spy_entity, employer)), Some(_)) => Some((spy_entity, employer)),
                _ => None,
            };
            let charge = Charge::new(ChargeKind::Rent { land }, Some(player), Some(payee), rent);
            // ③：这一笔**交给门**而不是自己结掉。就地结掉与挂成窗口由门定 ——
            // 手里没有免费卡与嫁祸卡、或这笔不够「大额」那把尺时，门在同一帧里就地落，
            // 世界看起来与这一句还是 `books.settle` 时一模一样
            submit(
                &mut gate,
                Job::new(
                    player,
                    vec![Step::Bill(charge)],
                    format!("L{} 的过路费", land.0),
                    None,
                    Gavel::Public,
                    Tail::Rent {
                        land,
                        payee,
                        spy: rerouted,
                    },
                ),
            );
        }
    }
}

/// 站在自己的地上能做的那两件事：**商业空地 → 建设窗口、住宅 → 升级报价**。
///
/// 落脚（[`settle_property_landing`]）与「停留停在原地」（`turn::take_stay_system`，P14d②）
/// 共用这一份判据 —— 两处各写一遍就会出现「一边的按钮灰着、另一边的账本能办」。
/// 返回是否真的摆了窗口：停留那一侧拿它决定「把回合交给窗口」还是「照常结束回合」。
///
/// 商业地已有设施时是空支（设施不可升级，4 代也没有「逛自己的店」）；四个「必败」条件
/// （押着／查封／衰神在场／现金不够升级）里，前三条连窗口都不弹，现金不够**照样弹**
/// —— 与买地报价同一条口径，按钮灰着、理由写在按钮上。
pub fn offer_own_land(
    books: &Books,
    player: Entity,
    land: LandId,
    view: &economy::LandView,
    pending: &mut PendingDecision,
    sink: &mut PropertySink<'_>,
    from_landing: bool,
) -> bool {
    let Some(def_land) = books.active.def.land(land.0) else {
        return false;
    };
    if def_land.land_type == LandType::Commercial {
        if books.facility_of(land).is_some()
            || view.mortgaged
            || view.status == Some(crate::status::LandStatus::Sealed)
            || books.acquire_denied(player)
        {
            return false;
        }
        let base = economy::upgrade_cost(&books.active.def, view).unwrap_or(0);
        let cost = books.quoted(ChargeKind::Build { land }, player, base);
        offer(pending, player, Pending::Build(land), from_landing);
        sink.builds.write(BuildOffered { player, land, cost });
        return true;
    }
    let Some(owner) = books.id_of(player) else {
        return false;
    };
    let base = economy::upgrade_cost(&books.active.def, view).unwrap_or(0);
    let cost = books.quoted(ChargeKind::Upgrade { land }, player, base);
    if books.acquire_denied(player) {
        return false;
    }
    let views = books.views();
    if !economy::can_upgrade(
        view,
        owner,
        &views,
        cost,
        books.balance(player),
        &books.rules,
    ) {
        return false;
    }
    offer(pending, player, Pending::Upgrade(land), from_landing);
    sink.upgrades.write(UpgradeOffered { player, land, cost });
    true
}

/// 摆窗口的那一句：落脚来的走 `offer`（它带着保释后置问），停留来的走
/// `offer_within_turn`（那时 `Landing` 是上一回合的旧值）。
fn offer(pending: &mut PendingDecision, player: Entity, about: Pending, from_landing: bool) {
    if from_landing {
        pending.offer(player, about);
    } else {
        pending.offer_within_turn(player, about);
    }
}

/// 踩到**别人**的设施：按原型算一笔账，把账交给门（P10c，③ 换成门的形状）。
///
/// 「不产生账」与「免单」在这里分家（见 [`service::assess`] 的注）：步行踩加油站、
/// D 型（研究所/公园）连流水都没有；大财神赖账 / 免费卡吃掉的那笔**过管线**，
/// 收据照样弹、上面的金额是 0 —— 玩家得看得见「为什么没付钱」。
///
/// B 型（旅馆）那一句「钱结清了再送人去住」现在写在 [`Tail::Service`] 的收尾里，
/// 而「住店」本身是表上 [`PassiveEvent::Lodging`] 那一行（10-27 的例外：不可免也不可转，
/// 所以门读到它当场落，一次窗口都不弹）。顺序还是那一条：钱先扣
/// （付不起当场破产，人都没了就不必再住店），再动棋子。
fn settle_service(
    gate: &mut GateDesk<'_, '_>,
    last_roll: &Res<'_, LastRoll>,
    player: Entity,
    land: LandId,
    owner_entity: Entity,
    kind: BuildingKind,
) {
    // 设施类别在组件上，参数按「预建用自己的、现建用棋盘默认」那条规则查 ——
    // 数据校验（P10b）保证两者与类别配对；缺参数（这条盘没配这类）等于没建成，不收费
    let Some(proto) = gate
        .sink
        .books
        .active
        .def
        .facility_fee(land.0, kind)
        .map(|f| f.to_prototype())
    else {
        return;
    };
    // A/B 型摇一个数；C/D 型不摇（`sides()` 为 0），传什么都行
    let roll = if proto.sides() > 0 {
        gate.sink.kit.rng.pick_service(1..=proto.sides())
    } else {
        0
    };
    // C 型的步数 = 本回合骰面。`LastRoll` 缺席（理论不可达：落格必跟在一次掷骰后）
    // 就按 0 步算 —— 账会算出 0，而不是 panic 一局对局
    let steps = last_roll
        .0
        .filter(|(who, _)| *who == player)
        .map_or(0, |(_, steps)| steps);
    let mount = gate.sink.mounts.get(player).copied().unwrap_or(Mount::Foot);
    let Some(bill) = service::assess(&proto, roll, steps, mount) else {
        return;
    };
    let charge = Charge::new(
        ChargeKind::Service,
        Some(player),
        Some(owner_entity),
        bill.base,
    );
    submit(
        gate,
        Job::new(
            player,
            vec![Step::Bill(charge)],
            format!("L{} 的{}", land.0, kind.label()),
            None,
            Gavel::Public,
            Tail::Service {
                land,
                owner: owner_entity,
                kind,
                roll,
                stay_days: bill.stay_days,
            },
        ),
    );
}

/// 建造请求被拒的理由。与 [`crate::shop::ShopDeny`] 同一族：面板的灰按钮与规则层
/// 读同一张表，按钮上那句「为什么」从这里来。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuildDeny {
    /// 棋盘上没这块地（或窗口没对着它开）
    NoWindow,
    /// 出手的不是业主
    NotOwner,
    /// 这块地押在银行
    Mortgaged,
    /// 查封中：法院贴着条子，不能施工
    Sealed,
    /// 上面已经立着设施了 —— 一格一设施
    Occupied,
    /// 这不是商业地
    NotCommercial,
    /// 这条盘没配这一类的收费参数（研究所/公园除外 —— 它们不收费）
    NoFee,
    /// 衰神在场：买地建屋必败
    DeniedByGod,
    /// 现金不够这一笔
    ShortOfCash,
}

impl BuildDeny {
    pub const fn label(self) -> &'static str {
        match self {
            Self::NoWindow => "这块地不在盘上",
            Self::NotOwner => "这块地不是你的",
            Self::Mortgaged => "这块地押在银行",
            Self::Sealed => "查封中不能施工",
            Self::Occupied => "这里已经立着设施了",
            Self::NotCommercial => "住宅地不建设施",
            Self::NoFee => "这里建不了这一类",
            Self::DeniedByGod => "衰神在场，建了必塌",
            Self::ShortOfCash => "现金不够",
        }
    }
}

/// 建造窗口的一笔请求能不能办。判据的**唯一**读点：[`Books::build`]（规则层）
/// 与 UI 的灰按钮各把自己的世界读数喂进来，谁都不许自己另写一套 if。
///
/// 返回 `Err(why)` ＝ 办不成，卡在为什么；`Ok(())` ＝ 出手吧。
pub fn build_op_allowed(
    view: economy::LandView,
    is_commercial: bool,
    occupied: bool,
    has_fee: bool,
    denied_by_god: bool,
    cost: i64,
    cash: i64,
) -> Result<(), BuildDeny> {
    if !is_commercial {
        return Err(BuildDeny::NotCommercial);
    }
    if view.owner.is_none() {
        return Err(BuildDeny::NotOwner);
    }
    if occupied {
        return Err(BuildDeny::Occupied);
    }
    if view.mortgaged {
        return Err(BuildDeny::Mortgaged);
    }
    if view.status == Some(crate::status::LandStatus::Sealed) {
        return Err(BuildDeny::Sealed);
    }
    if denied_by_god {
        return Err(BuildDeny::DeniedByGod);
    }
    if !has_fee {
        return Err(BuildDeny::NoFee);
    }
    if cash < cost {
        return Err(BuildDeny::ShortOfCash);
    }
    Ok(())
}

/// 建造柜台（P10c）：`BuildRequested` 逐笔进来办，窗口照开 —— 交还回合是
/// `decision_system` 对「先不建」的回应，这里碰都不碰 `NextState`（域的纪律）。
///
/// 判据与扣款都在 [`Books::build`]：钱不够不是这颗按钮说了算，是账本说了算 ——
/// 同帧两条建造请求、第二条会看到第一条扣完的余额（`debit` 就地写），
/// 「一次窗口付两遍钱」在 P3d③ 已经修过一次，这条纪律不许在新柜台上破。
pub fn build_op_system(
    mut commands: Commands,
    mut books: Books,
    pending: ResMut<PendingDecision>,
    mut requests: MessageReader<BuildRequested>,
    mut built: MessageWriter<FacilityBuilt>,
) {
    let Some((asked, Pending::Build(window))) = pending.active() else {
        return;
    };
    for r in requests.read() {
        if r.player != asked || r.land != window {
            continue;
        }
        match books.build(r.player, r.land, r.kind) {
            Ok(cost) => {
                if let Some(entity) = books.land_entity(r.land) {
                    commands.entity(entity).insert(Facility(r.kind));
                }
                built.write(FacilityBuilt {
                    player: r.player,
                    land: r.land,
                    kind: r.kind,
                    cost,
                });
            }
            Err(why) => debug!(
                "建造被拒（{}）：{} 想在 L{} 建 {:?}",
                why.label(),
                books.id_of(r.player).map(|p| p.0).unwrap_or_default(),
                r.land.0,
                r.kind
            ),
        }
    }
}
