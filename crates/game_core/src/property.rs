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

use crate::board::{BuildingKind, LandType};
use crate::charge::{Charge, ChargeKind};
use crate::economy;
use crate::land::{Facility, LandId};
use crate::msg::{
    BuildOffered, BuildRequested, BuyOffered, FacilityBuilt, PlayerDetained, RentDue,
    ServiceCharged, UpgradeOffered,
};
use crate::player::{DetentionPlace, Mount, PlayerId, TokenPosition};
use crate::rng::Rng;
use crate::service;
use crate::settle::Books;
use crate::turn::LastRoll;

use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};

/// 服务结算那张台子（P10c）：[`crate::service`] 原型库接上世界的那几样。
/// 打包成参数包是房规（§六 坑 8）—— 摇数、步数、载具、棋子与两条凭据是同一件事的
/// 六个侧面，散装就是一条十五个参数的 system。
#[derive(SystemParam)]
pub struct ServiceDesk<'w, 's> {
    /// 第十四条子流：商业设施摇的数。
    pub rng: ResMut<'w, Rng>,
    /// 本回合掷出的骰面（C 型按它计费）。写入在 `turn::apply_plan` 一处。
    pub last_roll: Res<'w, LastRoll>,
    /// C 型的载体判据：油费只对载具收。
    pub mounts: Query<'w, 's, &'static Mount, With<PlayerId>>,
    /// 这块地上立着什么。`None` ＝ 空地（商业空地或住宅）。
    pub facilities: Query<'w, 's, Option<&'static Facility>>,
    /// B 型住宿要把人挪走 —— 拘留的写就在 [`crate::landing::detain`]，这里只递工具。
    pub positions: Query<'w, 's, &'static mut TokenPosition, With<PlayerId>>,
    pub services: MessageWriter<'w, ServiceCharged>,
    /// 住宿那一下的凭据。与扣款凭据分开写：一笔记的是钱、一笔记的是人。
    pub detained_note: MessageWriter<'w, PlayerDetained>,
}

/// 落格结算要发出去的四种报价/凭据：买地、升级、建设、租金。合成一个参数包是房规
/// （§六 坑 8）—— 它们说的是同一件事的四个分支，散装就把
/// [`settle_property_landing`] 顶到 clippy 的参数上限，而那条警告在这里正是要吃掉
/// 「一条 system 要发四种消息」这个信号。
#[derive(SystemParam)]
pub struct PropertySink<'w> {
    pub offers: MessageWriter<'w, BuyOffered>,
    pub upgrades: MessageWriter<'w, UpgradeOffered>,
    pub builds: MessageWriter<'w, BuildOffered>,
    pub rents: MessageWriter<'w, RentDue>,
}

/// 踩到地产格：三种归属三种走法（无主 / 自己 / 别人），与 Godot 完全一致。
pub fn settle_property_landing(
    mut commands: Commands,
    mut books: Books,
    landing: Res<Landing>,
    mut pending: ResMut<PendingDecision>,
    mut desk: ServiceDesk<'_, '_>,
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
    let Some(view) = books.view(land) else {
        return;
    };
    let Some(self_id) = books.id_of(player) else {
        return;
    };
    let def = &books.active.def;

    match view.owner {
        // 无主 → 挂牌待决。Godot 也是「不管买不买得起都弹一张」（`confirm_buy` 里
        // 才做二次判定），差别在于这里买不起就是 `None`，不会静默扣成负数。
        //
        // 但**必败**那一格不给弹（P6b② 大衰神）：报价与成交读的是同一句
        // [`Books::buy_price`]，弹一张按下去必然失败的模态，比不弹更糟 ——
        // 玩家会以为是自己按错了，而不是身上那位神不让买。
        None if books.acquire_denied(player) => {
            let god = books.god_of(player).map(|held| held.god);
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
            if books.takes_landing(player) {
                debug!("无主地 L{} 不问价：他身上是土地公", land.0);
            } else {
                pending.offer(player, Pending::Buy(land));
                sink.offers.write(BuyOffered { player, land });
            }
        }
        // 自己的地：住宅走升级报价，商业地走建设窗口（P10c）。
        // 两条判据互相独立 —— 升级看「楼还有几层可盖」，建设看「这块地上还什么都没有」
        // —— 所以各占一格，而不是一条 if 链。
        Some(owner) if owner == self_id => {
            let Some(def_land) = def.land(land.0) else {
                return;
            };
            if def_land.land_type == LandType::Commercial {
                // 已有设施的商业地踩回自己家：无事。设施不可升级，4 代也没有
                // 「逛自己的店」这一出 —— 这一条刻意是空支，不是漏了升级报价。
                let occupied = books.facility_of(land).is_some();
                if occupied {
                    return;
                }
                // 必败不给弹（与无主地那一条同一句理由）：押着／查封／衰神在场时
                // 弹一张每颗按钮都按不动的模态，比不弹更糟。现金不够**照样弹** ——
                // 与买地报价同一条口径，按钮灰着、理由写在按钮上
                if !view.mortgaged
                    && view.status != Some(crate::status::LandStatus::Sealed)
                    && !books.acquire_denied(player)
                {
                    let base = economy::upgrade_cost(def, &view).unwrap_or(0);
                    let cost = books.quoted(ChargeKind::Build { land }, player, base);
                    pending.offer(player, Pending::Build(land));
                    sink.builds.write(BuildOffered { player, land, cost });
                }
                return;
            }
            let base = economy::upgrade_cost(def, &view).unwrap_or(0);
            let cost = books.quoted(ChargeKind::Upgrade { land }, player, base);
            let views = books.views();
            if !books.acquire_denied(player)
                && economy::can_upgrade(
                    &view,
                    owner,
                    &views,
                    cost,
                    books.balance(player),
                    &books.rules,
                )
            {
                pending.offer(player, Pending::Upgrade(land));
                sink.upgrades.write(UpgradeOffered { player, land, cost });
            }
        }
        // 别人的地：带设施 → 服务费（原型库当场结）；空商业地 → 一文不收
        // （设施还没立起来，不产生任何账 —— 连收据都不该有）；住宅 → 照旧收租。
        Some(owner) => {
            let Some(owner_entity) = books.holder(owner) else {
                return;
            };
            let facility = books.facility_of(land);
            if let Some(kind) = facility {
                settle_service(
                    &mut commands,
                    &mut books,
                    &mut pending,
                    &mut desk,
                    player,
                    land,
                    owner_entity,
                    kind,
                );
                return;
            }
            let Some(def_land) = def.land(land.0) else {
                return;
            };
            if def_land.land_type == LandType::Commercial {
                return;
            }
            let rent = books.rent_of(land);
            if rent <= 0 {
                return;
            }
            let charge = Charge::new(
                ChargeKind::Rent { land },
                Some(player),
                Some(owner_entity),
                rent,
            );
            // `payer` 取成交后那个：被嫁祸卡转手时，收据要说的是**实际付钱的人**
            if let Ok(done) = books.settle(&mut commands, charge) {
                sink.rents.write(RentDue {
                    payer: done.payer.unwrap_or(player),
                    owner: owner_entity,
                    land,
                    amount: done.amount,
                });
            }
            pending.offer(player, Pending::RentNotice(land));
        }
    }
}

/// 踩到**别人**的设施：按原型算一笔账并当场结掉（P10c）。
///
/// 「不产生账」与「免单」在这里分家（见 [`service::assess`] 的注）：步行踩加油站、
/// D 型（研究所/公园）连流水都没有；大财神赖账 / 免费卡吃掉的那笔**过管线**，
/// 收据照样弹、上面的金额是 0 —— 玩家得看得见「为什么没付钱」。
///
/// B 型（旅馆）在钱结清之后把人送去住宿：钱先扣（付不起当场破产，人都没了就不
/// 必再住店），再动棋子。`detain` 是四条去处的唯一入口 —— 棋子停在旅馆所在的
/// 那一格（`Hotel` 没有死胡同分支，`DetentionPlace::branch()` 是 `None`），回来那天
/// 从这格继续走。
#[allow(clippy::too_many_arguments)]
fn settle_service(
    commands: &mut Commands,
    books: &mut Books,
    pending: &mut PendingDecision,
    desk: &mut ServiceDesk<'_, '_>,
    player: Entity,
    land: LandId,
    owner_entity: Entity,
    kind: BuildingKind,
) {
    // 设施类别在组件上，参数按「预建用自己的、现建用棋盘默认」那条规则查 ——
    // 数据校验（P10b）保证两者与类别配对；缺参数（这条盘没配这类）等于没建成，不收费
    let Some(proto) = books
        .active
        .def
        .facility_fee(land.0, kind)
        .map(|f| f.to_prototype())
    else {
        return;
    };
    // A/B 型摇一个数；C/D 型不摇（`sides()` 为 0），传什么都行
    let roll = if proto.sides() > 0 {
        desk.rng.pick_service(1..=proto.sides())
    } else {
        0
    };
    // C 型的步数 = 本回合骰面。`LastRoll` 缺席（理论不可达：落格必跟在一次掷骰后）
    // 就按 0 步算 —— 账会算出 0，而不是 panic 一局对局
    let steps = desk
        .last_roll
        .0
        .filter(|(who, _)| *who == player)
        .map_or(0, |(_, steps)| steps);
    let mount = desk.mounts.get(player).copied().unwrap_or(Mount::Foot);
    let Some(bill) = service::assess(&proto, roll, steps, mount) else {
        return;
    };
    let charge = Charge::new(
        ChargeKind::Service,
        Some(player),
        Some(owner_entity),
        bill.base,
    );
    let Ok(done) = books.settle(commands, charge) else {
        return; // 付不起 → 破产已由 `settle` 宣告；人都没了，住宿无从谈起
    };
    if bill.stay_days > 0 {
        // B 型：离场 N 天。`detain` 更久的那张赢 —— 他本就关着时不换地方、只续到更久
        if let Some((sentence, junction, end)) = crate::landing::detain(
            commands,
            &mut desk.positions,
            &mut books.detained,
            &books.active.def,
            player,
            DetentionPlace::Hotel,
            bill.stay_days,
        ) {
            desk.detained_note.write(PlayerDetained {
                player,
                place: sentence.place,
                junction,
                end,
            });
        }
    }
    desk.services.write(ServiceCharged {
        payer: done.payer.unwrap_or(player),
        owner: owner_entity,
        land,
        kind,
        amount: done.amount,
        roll,
        stay_days: bill.stay_days,
    });
    // 钱已扣、人已送走 —— 收据仍然要弹：B 型那张上写着「住了几天」，
    // A 型那张上写着「摇出几点走了哪一档」。免单（金额 0）也一样弹。
    pending.offer(player, Pending::ServiceNotice(land));
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
