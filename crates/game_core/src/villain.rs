//! 四大恶人（P12）：环上的**第二种行动者**。
//!
//! 一句话的形状：恶人不是「会走路的道具」，也不是「没有钱包的玩家」，而是
//! **与玩家共用同一台回合机的另一种东西** —— 他挂 `TokenPosition`/`Heading`/`Mount`/
//! `WalkQueue`/`StaySentence`/`MoveCap`/`Detention`，所以他走、他停、他受伤；
//! 他不挂 `PlayerId`/`Wallet`/`Points`/`Hand`/`Inventory`/`BankBalance`/`Possession`，
//! 所以他没钱可付、没租可收、不进物价指数的分子、不算进「还剩几个人」。
//!
//! 三条在 P12 一开始就定下来、后面四个行为全要踩着的判断：
//!
//! - **保释只有一份实现，两个价目**（10-20 拍板）。[`crate::bail`] 的窗口与判据不认识
//!   「恶人」这个词，它只问「这一位关在这一处吗、这笔价钱是几」——价钱由
//!   [`crate::settle::Books::bail_price`] 一处答，而它的判据是「身上有没有 [`Villain`]」。
//!   30 与 300 是同一张表里的两行，将来要分叉就是加一行。
//! - **「在笼里」与「不在轮转里」必须是同一件事的两面**。笼中的恶人**不进** `TurnOrder`：
//!   这台回合机里「一个回合」等于日历上的一天（`end_turn_system` 加的是 `session.turn_count`），
//!   让四位没人保释的恶人各占一格，等于把整局的日历按三倍速走完 —— 分红、结息、开奖
//!   全跟着错位。所以进出轮转由本模块那一条同步 system 守住（[`villain_rotation_system]），
//!   而判据用的是 `Detention` 本身，不是第二个「他在不在笼里」的位。
//! - **「停留」从此不是按钮专属**。被按住的恶人由 [`villain_turn_system`] 发
//!   [`crate::msg::RequestStay`] —— 与 P4④ 给人类翻字那颗按钮、P14 要给 AI 的那条意图
//!   是同一条消息。少了这一支，`StaySentence` 在恶人身上就是死锁：规则层拒他的掷骰，
//!   而场上没有任何东西能按那颗按钮。
//!
//! 本文件带**载体与四个行为**：出场的抢夺（[`on_hired_system`]：强盗的存款、小偷的点券）、
//! 环上的两件事（[`robber_snatch_system`] 的擦肩抢卡、[`hoodlum_toll_system`] 的勒索）、
//! 小偷搬放置物的收货端（[`thief_deliver_system`]）；间谍是纯被动的，判据在
//! [`crate::settle::Books::rent_rerouted_to`]（那里也答「这块地有没有案底」），
//! 发送端在 `property.rs` 的租金结算里。集成行为测试在 `tests/villain.rs`。

use bevy::prelude::*;

use crate::board::{BoardDef, TileIndex, TileKind};
use crate::landing::PendingDecision;
use crate::msg::{
    DepositsRobbed, PointsStolen, RequestRoll, RequestStay, TileStepped, TollExtorted, VillainHired,
};
use crate::player::{
    Detention, DetentionPlace, Heading, Mount, Term, TokenPosition, WalkQueue, Walker,
};
use crate::turn::TurnOrder;
use crate::world::Gameplay;

/// 四种恶人。**新增一种必须同时回答三格**：他抢谁的什么（P12b–d 的行为表）、
/// 他归哪一处笼（[`VillainKind::cell`]）、他上屏叫什么名字。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum VillainKind {
    /// 强盗：抢全场存款 20%（**含雇主自己的**），擦肩抢一张卡（P12b）
    Robber,
    /// 小偷：偷对手一半点券、把路上的放置物搬来给雇主、**唯一能穿路障**（P12c）
    Thief,
    /// 流氓：路过别人地勒索地价等额，**钱归他自己、不归雇主**（P12b）
    Hoodlum,
    /// 间谍：把别人地应收的租金转付雇主，**该地从未有人交过费则无功而返**（P12d）
    Spy,
}

impl VillainKind {
    pub const ALL: [VillainKind; 4] = [Self::Robber, Self::Thief, Self::Hoodlum, Self::Spy];

    pub const fn label(self) -> &'static str {
        match self {
            Self::Robber => "强盗",
            Self::Thief => "小偷",
            Self::Hoodlum => "流氓",
            Self::Spy => "间谍",
        }
    }

    /// 开局关在哪一处。四个全在**监狱**：他们是恶人，不是病人 ——
    /// 医院那一处从此只在他的**受雇期间**挨了地雷或恶犬之后才用得上（PLAN §四 P12
    /// 那句「踩地雷与恶犬回医院需再保释」说的就是这一格从谁手里来）。
    pub const fn cell(self) -> DetentionPlace {
        DetentionPlace::Jail
    }

    /// 从数据里的字符串认一种恶人（棋盘/道具将来要按名字点人时读这一句）。
    /// 认不出返回 `None` 而不是默认某一位：默认值会让「写错一个字母」变成「场上多了一个贼」。
    pub fn from_key(key: &str) -> Option<Self> {
        Some(match key {
            "robber" => Self::Robber,
            "thief" => Self::Thief,
            "hoodlum" => Self::Hoodlum,
            "spy" => Self::Spy,
            _ => return None,
        })
    }
}

/// 他是恶人（而不是玩家）。带 `kind` 而不是让四种各建一个组件：四种的身份差别全在
/// 「他的行为读哪一格」，而那一格由 [`VillainKind`] 在各自的行为表里回答。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Villain {
    pub kind: VillainKind,
}

impl Villain {
    pub const fn label(self) -> &'static str {
        self.kind.label()
    }
}

/// 受谁雇用。**存在即在环上走**（他的回合、他的骰子、他的停留都从这一位来）。
///
/// 为什么不把「受雇」写成 `Option<VillainKind>` 或者塞进 `Villain` 的一个字段：
/// `Villain` 说的是他**是**谁（一辈子不变），`Hired` 说的是他此刻替谁办事（今天你、明天他），
/// 两次保释可以换雇主。合成一个组件就等于让「换雇主」变成「换一个人」，
/// 而他的位置和身上的伤都得跟着搬。
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hired {
    pub employer: Entity,
}

/// 同一颗伤口在两种行动者身上不是同一个句子。**这条判据只写在这一处**：
/// 地雷、恶犬、炸弹……每一处伤害出口都调它，将来加第五处不用再问一遍「他是谁」。
///
/// 玩家挨一下是「住院 N 天」，恶人挨一下是「回笼等保释」——PLAN §四 P12 的原文是
/// 「踩地雷与恶犬回医院需再保释」，而这句话在类型上只有 [`Term::UntilBailed`] 写得出来
/// （一个很大的 `u32` 也能跑，但它说的是「数得完」，那是假话）。
pub fn custody_of(villain: Option<&Villain>, days: u32) -> Term {
    if villain.is_some() {
        Term::UntilBailed
    } else {
        Term::Turns(days)
    }
}

/// 这张棋盘上有没有通向某一处的门（主环那一格，或某块地产上挂的那条分支）。
///
/// 没有门 = 那一处关着的人**永远保不出来**。这不是「少一个入口」而是「数据错」：
/// 与点券格那条「踩上去什么都不发生必须是数据错误」同一种纪律，所以装配时要 `warn!` 一句
/// 而不是让四位恶人在棋盘上无声地站到终局。
pub fn has_door(def: &BoardDef, place: DetentionPlace) -> bool {
    def.tiles.iter().any(|t| {
        let same_kind = match (place, t.kind) {
            (DetentionPlace::Jail, TileKind::Jail) => true,
            (DetentionPlace::Hospital, TileKind::Hospital) => true,
            // 国外与旅馆压根不在地图上，所以也没有「门口」这一格
            _ => false,
        };
        let same_branch = place
            .branch()
            .is_some_and(|kind| t.branch.as_ref().is_some_and(|b| b.kind == kind));
        same_kind || same_branch
    })
}

/// 开局把四位恶人铺进各自的笼子（**不进回合序**，见模块头第二条）。
///
/// 逻辑位置放**接入点**而不是死胡同格 —— 与 [`crate::landing::detain`] 给玩家的口径同一条
/// （core 拥有逻辑位置，死胡同格只是表现层把棋子挪过去的视觉停车位）。这张棋盘没有那条
/// 分支时退到环上第一格（0 号，10-26 起它是一条普通路而不是起点），位置仍然可读，
/// 而那件事由 [`has_door`] 在装配那一句 `warn!` 出来。
pub fn spawn_caged_villains(commands: &mut Commands, def: &BoardDef) -> Vec<Entity> {
    let junction = |place: DetentionPlace| {
        place
            .branch()
            .and_then(|kind| def.branch_junction(kind))
            .map(|t| TileIndex(t.id))
            .unwrap_or(TileIndex(0))
    };
    VillainKind::ALL
        .iter()
        .map(|kind| {
            let cell = kind.cell();
            commands
                .spawn((
                    Walker,
                    Villain { kind: *kind },
                    // 诨名就是他上屏的名字（`PlayerName` 的注记解释了为什么恶人也挂这一位）
                    crate::player::PlayerName(kind.label().to_string()),
                    TokenPosition {
                        tile: junction(cell),
                    },
                    WalkQueue::default(),
                    Heading::default(),
                    // 恶人恒步行：载具是「谁花钱买的」那件事，而他不花钱
                    Mount::Foot,
                    Detention {
                        place: cell,
                        term: Term::UntilBailed,
                    },
                    Gameplay,
                ))
                .id()
        })
        .collect()
}

/// 恶人的回合：他与玩家、AI 共用**同一套意图**（PLAN §四 P12 那句「人点按钮、AI 与 NPC
/// 发同一条意图」）。身上被停留按住时发 [`RequestStay`]，否则发 [`RequestRoll`]。
///
/// 为什么这里必须走消息而不是直接调 `roll_dice_system` 里那段算术：那一段只认
/// 「当前回合玩家 + 他身上的那一摞计时 + 路障截断 + 乌龟夹步」，抄一遍就是第二份实现，
/// 而 P5 之后每加一种「这一回合走几格」的东西都要记得改两处。
///
/// 只发意图、**不碰 `NextState`**：推进回合的永远是回合机自己（§4.6 的红线在 NPC 身上
/// 同样是红线 —— 否则「恶人替表现层推回合」就是那条红线的反面教材）。
pub fn villain_turn_system(
    order: Res<TurnOrder>,
    pending: Res<PendingDecision>,
    villains: Query<&Villain>,
    staying: Query<&crate::player::StaySentence>,
    mut rolls: MessageWriter<RequestRoll>,
    mut stays: MessageWriter<RequestStay>,
) {
    let Some(current) = order.current() else {
        return;
    };
    // 窗口开着的时候不许他插嘴：拍卖那一种会**换人问**，而此刻被问的未必是他
    if pending.active().is_some() {
        return;
    }
    let Ok(villain) = villains.get(current) else {
        return;
    };
    if let Ok(sentence) = staying.get(current) {
        debug!(
            "{} 被停留按住（剩 {} 回合），发「停留」意图",
            villain.label(),
            sentence.turns_left
        );
        stays.write(RequestStay { player: current });
        return;
    }
    // `dice: 0` = 满载（按他身上的载具摇）。他没车，所以恒一颗骰子
    rolls.write(RequestRoll {
        player: current,
        dice: 0,
    });
}

/// 「在笼里」与「不在轮转里」的那道同步。挂在 `OnEnter(TurnEnd)`、排在
/// [`crate::turn::end_turn_system`] **之后**：那一条要把游标推到下一位，而这一条要摘的
/// 正是刚用完回合的那一位 —— 反过来写，游标会跟着被摘掉的那格一起往前跳，
/// 症状是**有人的回合凭空被跳过一次**（与 §6.1 坑 36/37 同一族：顺序只能由 `chain` 说）。
///
/// 它干两件事，两件都是「恶人身上的两个位必须自洽」：
/// 1. 笼中（身上有 `Detention`）的恶人**不该在轮转里** —— 受伤那一刻他还站在自己的回合上，
///    所以摘除只能等到他的回合真的收尾。
/// 2. 雇主破产/不在了的受雇恶人**关回笼子** —— 「替谁办事」这个位指着一个不能行动的人，
///    读起来就像他还在替一个死人抢钱。不修的话他会一直走、一直抢，却把钱交给查不到的实体。
// 参数是「两位自洽都要对」的那一套（轮转、位置、拘留、雇主），拆散只会让两条写各借一遍
#[allow(clippy::too_many_arguments)]
pub fn villain_rotation_system(
    mut commands: Commands,
    mut order: ResMut<TurnOrder>,
    hired: Query<(Entity, &Villain, &Hired)>,
    employers: Query<Option<&crate::player::Bankrupt>>,
    mut positions: Query<&mut TokenPosition, With<Walker>>,
    // 只读那一条「他在不在笼里」用的是**这同一条查询**而不是再开一条 `Query<&Detention, With<Villain>>`：
    // 两条对同一组件的一读一写会在调度期直接撞成借用冲突（§6.1 坑 33 的同族）
    mut detained: Query<&mut Detention, With<Walker>>,
    active: Res<crate::world::ActiveBoard>,
    mut notices: MessageWriter<crate::msg::PlayerDetained>,
) {
    for (villain, held_by, hire) in hired.iter() {
        if detained.get(villain).is_ok() {
            // ① 受伤那一回合还没收尾，所以此刻「在笼里」与「在轮转里」同时成立 —— 摘除只能等这里
            if order.remove(villain) {
                debug!("{} 在笼里，退出轮转", held_by.label());
            }
            continue;
        }
        // ② 雇主还在场上吗。`get` 失败（实体已不在）与 `Some(Bankrupt)` 是同一句话：没人可替了
        let gone = employers
            .get(hire.employer)
            .map(|bankrupt| bankrupt.is_some())
            .unwrap_or(true);
        if !gone {
            continue;
        }
        commands.entity(villain).remove::<Hired>();
        let place = held_by.kind.cell();
        if let Some((sentence, junction, end)) = crate::landing::detain(
            &mut commands,
            &mut positions,
            &mut detained,
            &active.def,
            villain,
            place,
            Term::UntilBailed,
        ) {
            notices.write(crate::msg::PlayerDetained {
                player: villain,
                place: sentence.place,
                junction,
                end,
            });
            if order.remove(villain) {
                debug!(
                    "{} 的雇主不在了，遣回{}并退出轮转",
                    held_by.label(),
                    place.label()
                );
            }
        }
    }
}

// ===== 出场行为（P12b/c）=====

/// 保释成交那一帧的**出场行为**（P12b/c）：强盗抢存款、小偷偷点券。
///
/// 为什么走消息而不是在 `bail_op_system` 里就地调：保释窗口不该认识任何一种恶人的名字
/// （「同一张窗口两个价目」的全部意义就是它只问价钱不问身份），而「出场时做什么」是
/// 恶人模块自己的家务 —— 分派只有一张 `match`，加第五种恶人时改的是这张表，
/// 不是保释窗口。消息下一帧投递（坑 12），对「出场那一刻」的语义没有影响：
/// 玩家在窗口里看到的候选、付出的点券都已经是过去式。
///
/// 只有两处在场行为放这里，是刻意的：强盗的擦肩抢卡与流氓的勒索都发生在**环上走路**时
/// （`TileStepped` 那两条），不随出场发生；间谍更是纯被动（租金结算里判）。
/// 「出场发一笔、走路发一路」是四种人各自的形状，别混。
pub fn on_hired_system(
    mut books: crate::settle::Books<'_, '_>,
    mut hired: MessageReader<VillainHired>,
    mut robbed: MessageWriter<DepositsRobbed>,
    mut stolen: MessageWriter<PointsStolen>,
) {
    for done in hired.read() {
        match done.kind {
            VillainKind::Robber => {
                let (total, victims) = books.sweep_deposits(books.rules.robber_deposit_percent);
                robbed.write(DepositsRobbed {
                    villain: done.villain,
                    employer: done.employer,
                    total,
                    victims,
                });
            }
            VillainKind::Thief => {
                let (total, victims) =
                    books.steal_points(done.employer, books.rules.thief_points_percent);
                stolen.write(PointsStolen {
                    villain: done.villain,
                    employer: done.employer,
                    total,
                    victims,
                });
            }
            // 出场不发一笔的那种人：走到这一格只是把「他是这种恶人」再确认一遍。
            // 显式列出而不是 `_`，第四种人的行为表少一行就编译不过
            VillainKind::Hoodlum | VillainKind::Spy => {}
        }
    }
}

/// 强盗擦肩抢卡（P12b）：他走路的每一步，踩到**玩家所在格**就抢一张。
///
/// 「擦肩」的判据与炸弹转让同一条（路径经过对方当前所在格），读者却不必是同一条
/// system：炸弹的转让必须与「爆炸」同帧裁定（`walk.rs` 里做），抢卡没有那个紧迫性 ——
/// 走路不会被它打断，晚一帧的顶栏播报玩家看不出差别。所以这里读 `TileStepped` 消息，
/// `walk.rs` 保持不认识「强盗」这个词。
///
/// 目标的排他判据全在这一处：受害者必须是**玩家**（恶人没有手牌可抢）、不是雇主
/// （刀口不对着自己人）、没破产（离场的人身上没有卡）、手牌非空。同格站了几个人
/// 就各被抢一张 —— 擦过一圈，不是擦过第一个。
// 擦肩要问的七零八落（他、雇主、手牌、随机流、卡面）都是同一条 reader 的输入，
// 与 `settle_service` 同一条处理
#[allow(clippy::too_many_arguments)]
pub fn robber_snatch_system(
    mut rng: ResMut<crate::rng::Rng>,
    rules: Res<crate::rules::GameRules>,
    mut steps: MessageReader<TileStepped>,
    villains: Query<&Villain>,
    jobs: Query<&Hired>,
    standings: Query<(Entity, Option<&crate::player::Bankrupt>), With<crate::player::PlayerId>>,
    positions: Query<&TokenPosition>,
    mut hands: Query<&mut crate::hand::Hand, With<crate::player::PlayerId>>,
    shelf: crate::deck::Shelf<'_>,
    mut snatched: MessageWriter<crate::msg::CardSnatched>,
    mut gained: MessageWriter<crate::msg::CardGained>,
) {
    for step in steps.read() {
        let Ok(robber) = villains.get(step.player) else {
            continue;
        };
        if robber.kind != VillainKind::Robber {
            continue;
        }
        let Ok(job) = jobs.get(step.player) else {
            continue;
        };
        // 同格的受害者按实体序拿稳（archetype 迭代序不保证，§6.1 同族）
        let mut victims: Vec<Entity> = standings
            .iter()
            .filter(|(e, bankrupt)| {
                bankrupt.is_none()
                    && *e != job.employer
                    && positions.get(*e).is_ok_and(|p| p.tile == step.tile)
            })
            .map(|(e, _)| e)
            .collect();
        victims.sort_by_key(|e| e.index());
        for victim in victims {
            // 抽哪一张在借用手牌之前定：`rng` 的可变借用不能与 `hands` 的同时活着
            let count = hands.get(victim).map(|h| h.len()).unwrap_or(0);
            if count == 0 {
                continue;
            }
            let pick = rng.pick_villain(0..=count as u32 - 1) as usize;
            let card = {
                let Ok(mut hand) = hands.get_mut(victim) else {
                    continue;
                };
                let card = hand.cards[pick];
                if !hand.spend(card) {
                    continue;
                }
                card
            };
            // 雇主收卡走与抽卡同一条入口 `deal_to`：上限规则与「被挤掉要发流水」是一体的，
            // 绕过它就会有一处忘了处理手牌满 15 张的那一格
            if let Ok(mut hand) = hands.get_mut(job.employer) {
                crate::hand::deal_to(
                    &mut hand,
                    rules.hand_limit,
                    card,
                    job.employer,
                    &shelf,
                    &mut gained,
                );
            }
            snatched.write(crate::msg::CardSnatched {
                robber: step.player,
                victim,
                employer: job.employer,
                card: crate::hand::card_notice(&shelf, card),
            });
        }
    }
}

/// 流氓勒索（P12b）：路过别人的地，向地主勒索地价等额的现金。
///
/// 「路过」走 `TileStepped`：走步的**每一步**（不只是落脚）都算路过 —— 4 代的那句
/// 「经过」在逐格行走的引擎里就是这个意思，而 2×2 地共享的两条路格各算一次，
/// 与路障、地雷的逐格口径一致。钱归流氓自己（`Books::charge` 只扣不加，
/// 强盗连雇主都不分）—— PLAN §三 拍板的反制窗口。
///
/// 三条豁免与租金同一族判据：无主地没有可勒索的人；查封中的地法院贴着条子
/// （与租金归零同一条理由：那块地此刻被冻结）；雇主的地不勒（自己人）。
/// 抵押与业主被关**不豁免** —— 勒索的是「地主这个人」，不是这块地的经营权。
pub fn hoodlum_toll_system(
    mut commands: Commands,
    mut books: crate::settle::Books<'_, '_>,
    active: Res<crate::world::ActiveBoard>,
    mut steps: MessageReader<TileStepped>,
    villains: Query<&Villain>,
    jobs: Query<&Hired>,
    mut extorted: MessageWriter<TollExtorted>,
) {
    for step in steps.read() {
        let Ok(hoodlum) = villains.get(step.player) else {
            continue;
        };
        if hoodlum.kind != VillainKind::Hoodlum {
            continue;
        }
        let Some(land) = active
            .def
            .tile(step.tile.0 as i64)
            .and_then(|t| t.land_id)
            .map(crate::land::LandId)
        else {
            continue;
        };
        let Some(owner) = books.owner_of(land) else {
            continue;
        };
        if jobs.get(step.player).is_ok_and(|h| h.employer == owner) {
            continue;
        }
        let Some(view) = books.view(land) else {
            continue;
        };
        if view.status == Some(crate::status::LandStatus::Sealed) {
            continue;
        }
        let Some(amount) = books.land_market_price(land) else {
            continue;
        };
        if amount <= 0 {
            continue;
        }
        // 付不起当场破产 —— 与租金同一条出口。被勒到破产是这条规则字面允许的事
        if books.charge(&mut commands, owner, amount).is_err() {
            continue;
        }
        extorted.write(TollExtorted {
            hoodlum: step.player,
            land,
            owner,
            amount,
        });
    }
}

/// 小偷把偷来的东西送进雇主道具栏（P12c）。
///
/// 发送侧（走步循环的雷/路障、落脚结算的宝箱）只说「这一格少了一件东西」，
/// 「该给谁」在这条 reader 里答 —— `walk.rs` 保持不认识 `Hired` 这个组件。
/// 雇主查不到（同帧破产的边角）时东西就地消失：装不下的赃物不能凭空还给街上，
/// 而播报里「小偷偷走了路障」那半句仍然是真的。
pub fn thief_deliver_system(
    mut thieved: MessageReader<crate::msg::ItemThieved>,
    jobs: Query<&Hired>,
    mut inventories: Query<&mut crate::items::Inventory, With<crate::player::PlayerId>>,
) {
    for done in thieved.read() {
        let Ok(job) = jobs.get(done.thief) else {
            warn!(
                "小偷偷到 {:?} 却查不到雇主（`Hired` 与「在环上走」分叉了），赃物就地消失",
                done.item
            );
            continue;
        };
        if let Ok(mut inventory) = inventories.get_mut(job.employer) {
            inventory.gain(done.item, 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::DetentionPlace as P;

    const PROTOTYPE: &str = include_str!("../../../assets/data/boards/prototype.json");

    fn board() -> BoardDef {
        BoardDef::from_json(PROTOTYPE).expect("prototype.json 应能解析")
    }

    #[test]
    fn the_four_each_have_a_name_and_the_keys_map_one_to_one() {
        let labels: Vec<_> = VillainKind::ALL.iter().map(|k| k.label()).collect();
        assert_eq!(
            labels,
            vec!["强盗", "小偷", "流氓", "间谍"],
            "四个人的名字是这张表的对外口径：改了要连同 PLAN §三 那一行一起改"
        );
        // 每个 key 认得出、且认到的是**那一个**人；四条全对不上就说明 key 与 ALL 脱钩了
        for (kind, key) in [
            (VillainKind::Robber, "robber"),
            (VillainKind::Thief, "thief"),
            (VillainKind::Hoodlum, "hoodlum"),
            (VillainKind::Spy, "spy"),
        ] {
            assert_eq!(VillainKind::from_key(key), Some(kind));
        }
        // 认不出必须是 `None` 而不是某一位：默认值会把「写错一个字母」变成「场上多了一个贼」
        assert_eq!(VillainKind::from_key("boss"), None);
        assert_eq!(VillainKind::from_key("Robber"), None, "大小写不放过");
    }

    #[test]
    fn all_four_start_locked_in_the_prison() {
        for kind in VillainKind::ALL {
            assert_eq!(kind.cell(), P::Jail, "{} 的开局笼子变了", kind.label());
        }
    }

    #[test]
    fn the_same_mine_is_a_sentence_for_a_player_and_a_cage_for_a_villain() {
        assert_eq!(custody_of(None, 3), Term::Turns(3));
        let robber = Some(&Villain {
            kind: VillainKind::Robber,
        });
        // 「需再保释」落在这一个函数上：3 天那一句在恶人身上说不通
        assert_eq!(custody_of(robber, 3), Term::UntilBailed);
        assert_eq!(
            custody_of(robber, 5),
            Term::UntilBailed,
            "几天都不该改变答案"
        );
    }

    #[test]
    fn the_demo_board_has_a_door_for_each_cell_and_none_for_the_places_off_the_map() {
        let b = board();
        // 演示盘：汉口路挂 jail 分支、徐家汇挂 hospital 分支（`bail.rs` 的模块头记着这件事）
        assert!(
            has_door(&b, P::Jail),
            "没有监狱门的话四位恶人站到终局也保不出来"
        );
        assert!(has_door(&b, P::Hospital));
        // 国外与旅馆压根不在地图上 —— 这两格必须回答「没有门」，否则「门」这个词就
        // 变成了「任何一处去过的地方」
        assert!(!has_door(&b, P::Away));
        assert!(!has_door(&b, P::Hotel));
    }
}
