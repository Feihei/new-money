//! 回合状态机。对应 Godot 的 `turn_manager.gd`（492 行、三处 `await`、六处手写阶段守卫）。
//!
//! 源实现里「现在是哪个阶段」是 `var _phase` + 每个回调开头一句 `if _phase != X: return`；
//! 这里 `TurnPhase` 是 `States`，守卫由 `run_if(in_state(..))` 声明式给出，
//! 于是漏写守卫这类错误不复存在。`await token.arrived` 那种「逻辑等渲染」则被拆成
//! `Walking` 阶段（见 §4.6 的所有权红线）。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use crate::bank::AtmAccess;
use crate::economy;
use crate::items::{Inventory, ItemKind};
use crate::market::Market;
use crate::msg::{
    DetentionLifted, DetentionSkipped, DiceRolled, GameOver, MoveCapped, RemoteDiceUsed,
    RequestRemoteDice, RequestRoll, RequestStay, StayLifted, StaySkipped, TurnEnded, TurnStarted,
    WalkStarted,
};
use crate::occupancy::Occupancy;
use crate::player::{
    Bankrupt, Detention, Heading, Mount, MoveCap, PlayerId, StaySentence, Term, TokenPosition,
    WalkQueue, Walker,
};
use crate::rng::Rng;
use crate::settle::Books;
use crate::walk::{WalkClock, plan_walk};
use crate::world::ActiveBoard;

/// 回合阶段。`Boot` 是初始态：资产到位前什么都不做。
///
/// ⚠️ 初始态的 `OnEnter` **不会**自动触发（Bevy 的 `init_state` 不产生转移），
/// 所以 `begin_game_system` 挂在 `in_state(Boot)` 上而不是 `OnEnter(Boot)`。
#[derive(States, Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum TurnPhase {
    #[default]
    Boot,
    TurnStart,
    WaitRoll,
    /// 逐格行走：`WalkClock` 驱动 `WalkQueue`，每格一条 `TileStepped`
    /// （`walk_step_system`，见 §4.6 的所有权红线）。`walk_step_interval = 0`
    /// 时一帧走完，与 B2b 的瞬时落格行为等价。
    Walking,
    /// 走步被道路放置物中断（P5）。**不是 `Landed`**：中断的那一格不结算静态格型，
    /// 也没有 `PlayerArrived`。`occupancy::resolve_halt_system` 在这里应用后果
    /// （住院、炸弹的 3×3 降层），随后直接收尾到 `TurnEnd`。
    Halted,
    /// 落格结算的分派点。B3 在此接地产/卡牌/银行，目前所有格子都无结算。
    Landed,
    /// 等待玩家决策（买地/升级/确认租金/读卡）。生产者随 B3 的买地流程出现。
    WaitDecision,
    TurnEnd,
    Over,
}

/// 回合轮转顺序。破产玩家留在表里但被跳过（对应 `_advance_and_start` 的 while 循环）。
#[derive(Resource, Debug, Clone, PartialEq, Eq)]
pub struct TurnOrder {
    pub order: Vec<Entity>,
    pub cursor: usize,
}

impl TurnOrder {
    pub fn new(order: Vec<Entity>) -> Self {
        Self { order, cursor: 0 }
    }

    pub fn current(&self) -> Option<Entity> {
        self.order.get(self.cursor).copied()
    }

    /// 保证 `current()` 落在可行动玩家上（必要时前进），返回该玩家。
    /// 全员不可行动（或阵容为空）时返回 `None`。
    pub fn current_active(&mut self, is_active: impl Fn(Entity) -> bool) -> Option<Entity> {
        let n = self.order.len();
        for i in 0..n {
            let idx = (self.cursor + i) % n;
            if is_active(self.order[idx]) {
                self.cursor = idx;
                return Some(self.order[idx]);
            }
        }
        None
    }

    /// 前进到下一名可行动玩家（`i = 1..=n`，不含自己），返回该玩家。
    pub fn advance_to_active(&mut self, is_active: impl Fn(Entity) -> bool) -> Option<Entity> {
        let n = self.order.len();
        for i in 1..=n {
            let idx = (self.cursor + i) % n;
            if is_active(self.order[idx]) {
                self.cursor = idx;
                return Some(self.order[idx]);
            }
        }
        None
    }

    /// 把一位新行动者排到 `after` 那位的**后面**（P12：保释出来的恶人接着雇主的回合走）。
    /// 已在表里时不重复插，返回是否发生了插入。
    ///
    /// 游标跟着让位：插在游标**之前或正中**时 `cursor += 1`，否则它会指到刚插进来的那位身上，
    /// 症状是**别人的回合被这个新来的人抢走一格**（这一条没人会写第二遍，所以纪律在这里）。
    pub fn insert_after(&mut self, actor: Entity, after: Entity) -> bool {
        if self.order.contains(&actor) {
            return false;
        }
        let at = match self.order.iter().position(|e| *e == after) {
            Some(i) => i + 1,
            None => self.order.len(),
        };
        self.order.insert(at, actor);
        if at <= self.cursor {
            self.cursor += 1;
        }
        true
    }

    /// 把一位从轮转里摘掉（P12：恶人回笼）。返回是否发生了摘除。
    ///
    /// 游标的三条口径：摘在游标**之前**要退一格（不然会跳过一位）；摘的**正是游标那一位**时
    /// 游标不动，于是它指向紧跟在他后面的那位 —— 那正是「他的回合刚刚用完」那一帧要的读数，
    /// 所以调用方必须排在 [`end_turn_system`] 之后（见 `villain::villain_rotation_system` 的注）；
    /// 退到表尾或表空时归零（`advance_to_active` 自己会再找可行动的那位）。
    pub fn remove(&mut self, actor: Entity) -> bool {
        let Some(at) = self.order.iter().position(|e| *e == actor) else {
            return false;
        };
        self.order.remove(at);
        if at < self.cursor {
            self.cursor -= 1;
        }
        if self.cursor >= self.order.len() {
            self.cursor = 0;
        }
        true
    }
}

/// 对局元信息。取代 `game_state.gd` 的 `game_id`/`turn_count`。
///
/// `game_id` 不再保留：它只被用在一条错误的 `game_saved.emit`（缺陷 #11）里，
/// 而取时间戳本身就会让对局不可复现。存档需要标识时从 `seed` 推。
///
/// 只 `PartialEq` 不 `Eq`：`price_index` 是浮点，`Eq` 在 `NaN` 上不自反。
/// 存档比较走逐字段 `==`，不需要把整条资源塞进 `HashMap` 的键。
#[derive(Resource, Debug, Clone, PartialEq)]
pub struct GameSession {
    pub turn_count: u32,
    pub seed: u64,
    /// 物价指数（P2）。开局 1.0，每个回合末按「全员现资产总和 ÷ 全员初始总和」重算，
    /// 不设上限（PC 原版口径，PLAN §一）。哪些钱受它缩放是 `charge::scales_with_price`
    /// 那张穷尽表说了算 —— 股价与神明金额是点名豁免的。
    pub price_index: f32,
    /// 开局那天（P2b）。2048 年随机一个月的 1 日，由**历法专用子流**从会话种子抽出；
    /// `NM_EPOCH` 可钉死。它决定整局日历，所以必须进 `GameSnapshot`（P15）。
    pub epoch: crate::calendar::Date,
    /// 最后一次发圣诞礼物的年份（P2b）。没有这一位，读档回到 12/25 会再发一轮。
    pub last_gift_year: Option<i32>,
    /// 最后一次发春节红包的年份（P2b）。理由同上。
    pub last_envelope_year: Option<i32>,
    /// 上一条播出去的是哪一条新闻（P11）。✅ 10-20 拍板那条「全池随机 + **同场不连抽同一条**」
    /// 换来的状态 —— 它比命运那套定序省（不必存发到第几张），但**不是零状态**。
    ///
    /// 存的是**下标**而不是 id 字符串，与 P15「牌堆顺序必须存索引」同一条账：
    /// 一条新闻的寿命跟着数据文件走，而「上一条是哪条」是这一局的事实。
    /// 它也必须在**开局**归零（`world::begin_game_system` 造的就是新的一份）——
    /// 留着上一局那一条，新局第一次踩新闻中心会**避开一条它这局从没播过的新闻**。
    pub last_news: Option<crate::news::NewsId>,
    /// 机构停摆登记处（P11c②）：股市停牌 / 银行停止放款，各自还剩几天。
    ///
    /// 它是**全局规矩**而不是某个人的状态，所以进 `GameSession`（与 `last_news` 同一条理由：
    /// 必须进存档，且开局由 `begin_game_system` 造的新的一份归零 —— 留着上一局的停牌，
    /// 新局第一格股市就凭空不开盘）。
    pub suspensions: crate::institution::Suspensions,
    /// 银行储备（P10d②）：存款存进来多少、取款取走多少，这条流水是**融资三件套**
    /// 的地基 —— 4 代口径里银行不是印钞机，库里只有储户放进去的钱；取款时库不够，
    /// 差额由「银行保险」的董事长兜底（`bank.rs` 的融资规则 ②）。
    ///
    /// 借款不经过它（贷款的钱是银行创造的，风险由地产额度那条判据管着）——
    /// 「储备」只对**存款**这条流水记账，是刻意收窄的口径，理由写在 `bank.rs`。
    /// 进 `GameSession` 是因为它是全局账、且必须进存档（P15）。
    pub bank_vault: i64,
}

/// 暂停总闸（`pause_menu.gd` 的 `get_tree().paused`）。规则层只读，写的永远是输入侧。
///
/// 这不违反 §4.6 的红线：红线管的是「表现层替逻辑做决定」，而暂停与掷骰同属玩家意图 ——
/// 它说的是「现在不许推进」，不是「推进到下一步」。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Paused(pub bool);

/// `run_if` 条件：没按下暂停才让回合机的 system 跑。
pub fn not_paused(paused: Res<Paused>) -> bool {
    !paused.0
}

/// `begin_turn_system` 的出口消息打包（六个 `MessageWriter` 参数比挂 `#[allow]` 好读）。
///
/// ⚠️ 这里每加一条 `MessageWriter`，`lib.rs` 就要有一行对应的 `add_message::<..>()`。
/// 漏了不会编译失败、也不会报错 —— 整个 system 因为 `SystemParam` 建不出来而**静默不跑**，
/// 表现是「回合机停住」，看着像渲染层的锅（§6.1 的坑）。
#[derive(SystemParam)]
pub struct TurnWriters<'w> {
    turn_started: MessageWriter<'w, TurnStarted>,
    released: MessageWriter<'w, DetentionLifted>,
    skipped: MessageWriter<'w, DetentionSkipped>,
    over: MessageWriter<'w, GameOver>,
    /// 神老了一天/到期离场（P6）。挂在这里而不是单开一条 system：扣那一格必须发生在
    /// 拘留分支**之前**，而那正是这个函数最靠前的几行。
    god_left: MessageWriter<'w, crate::msg::GodLeft>,
    /// 衰神这一回合从他手里弄走的卡（P6b②）。与 `god_left` 同一个位置：那是「他的回合
    /// 开始」这一刻的事，而神到期那一回合不该再多弄走一张（判据见 `age_possession` 的注）。
    card_lost: MessageWriter<'w, crate::msg::CardLost>,
}

/// `roll_dice_system` 的输入打包：请求流水 + 当前轮转 + 棋盘拓扑
/// （§4.5 表格里「读」的那三样；reader 带两个生命周期，见坑 13）。
#[derive(SystemParam)]
pub struct RollInput<'w, 's> {
    requests: MessageReader<'w, 's, RequestRoll>,
    order: Res<'w, TurnOrder>,
    active: Res<'w, ActiveBoard>,
    /// 路障截断发生在入队时（ADR-002 §6）—— 掷骰这一侧只问「路径里第一个路障在哪」。
    occupancies: Query<'w, 's, (Entity, &'static Occupancy)>,
    /// 小偷穿路障（P12c）：判据读的是**走路的那一位**身上有没有那个标记，
    /// 所以输入包里带一条恶人查询。它不与 `walkers` 撞（`Villain` 不在那个元组里）。
    villains: Query<'w, 's, &'static crate::villain::Villain>,
}

/// `begin_turn_system` 要动的那一摞。P6 起了第九个参数，按房规打包而不是挂
/// `#[allow(clippy::too_many_arguments)]`（§六 坑 8）：这六样回答的是同一个问题 ——
/// 「轮到他了吗、他被什么按住了」。
#[derive(SystemParam)]
pub struct TurnDesk<'w, 's> {
    commands: Commands<'w, 's>,
    order: ResMut<'w, TurnOrder>,
    /// **`With<Walker>` 而不是 `With<PlayerId>`**（P12）：受雇的恶人要能轮到、要被跳过、
    /// 出得了笼。`alive` 读的是「他破产了吗」，而恶人压根没有破产这一说 ——
    /// 他不在 `Bankrupt` 那一族里，所以「没破产」对他恒真，正是想要的读数。
    players:
        Query<'w, 's, (Entity, Option<&'static Bankrupt>, &'static TokenPosition), With<Walker>>,
    held: Query<'w, 's, &'static Detention, With<Walker>>,
    possessions: Query<'w, 's, &'static mut crate::gods::Possession, With<PlayerId>>,
    /// 修正登记处。**为什么是这么小的一块而不是整个 `Books`**：`Books` 带着
    /// `&mut Detention`（P4② 搬进去的），而上面那格 `held` 读的是同一个组件 ——
    /// 两条并排会在参数建立期撞成借用冲突。回合机要的只有「神到期时把那几条撤掉」，
    /// 所以只把那一格拿进来。
    corrections: ResMut<'w, crate::charge::Corrections>,
    /// 衰神每回合弄走一张手牌（P6b②）要的三样。同样**不是整个 `Books`**（同上一条理由），
    /// 而这三样确实是那一件事的全部：从谁手里、按哪条随机流抽、抽到的那张叫什么名字。
    hands: Query<'w, 's, &'static mut crate::hand::Hand, With<PlayerId>>,
    shelf: crate::deck::Shelf<'w>,
    rng: ResMut<'w, Rng>,
    atm: ResMut<'w, AtmAccess>,
}

/// 回合开始的唯一入口：跳过破产者 → 处理拘留 → 否则等掷骰。
///
/// 拘留的递减规则与 `player_data.gd:72-80` 一致：`Term::Turns` 每次回合开始减一，
/// 扣到只剩 1 的那一回合**当场回来并可掷骰**（即判 3 回合 = 跳过 2 个回合）。
/// `Term::UntilBailed`（P12 的恶人）不扣也不放 —— 见下面那三条出口的注。
///
/// **停留（P4④ 起）不在这里扣**：身上有 `StaySentence` 的人照常进入 `WaitRoll`，只是
/// 掷骰按钮翻成「停留」，点一下才结束这一回合（`take_stay_system`）。两条计时因此
/// 各走各的入口，也不再需要「一回合只跳一次」那条合取 —— 被拘留的人根本进不了 `WaitRoll`。
/// 为什么不合成一个组件：见 [`StaySentence`] 那段 —— 被拘留的人被送进死胡同格，
/// 停留的人还在自己踩到的那一格上。
///
/// 神的那一格（P6）扣在**所有分支之前**：PLAN §三 说「坐牢 3 天期间神的 7 天照算 3 天」，
/// 而这句话在这台回合机里的意思就是 —— 他被拘留的那些回合，回合照样开始、扣数照样走。
/// 放到拘留分支之后会得到一条看起来合理、实际永远不响的分支。
pub fn begin_turn_system(
    mut desk: TurnDesk<'_, '_>,
    mut writers: TurnWriters,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let alive = |e: Entity| {
        desk.players
            .get(e)
            .map(|(_, bankrupt, _)| bankrupt.is_none())
            .unwrap_or(false)
    };

    let Some(player) = desk.order.current_active(alive) else {
        // 阵容为空，或全员已破产（正常流程下 end_turn_system 会先一步进 Over）
        writers.over.write(GameOver { winner: None });
        next.set(TurnPhase::Over);
        return;
    };
    // ATM 资格活到「持有人自己的下一回合开始」为止，不是任何人的回合开始都清。
    // 路过那一瞬间正是回合机最赶的时候（行走未完、落格已结算、下一位紧接着开始），
    // 只给那一帧的窗口等于按不到；放到下一次行动之前，玩家才有真正能点的一段 ——
    // 与证券面板「掷骰前可操作」是同一个窗口。
    if desk.atm.0 == Some(player) {
        desk.atm.0 = None;
    }
    let Ok((_, _, pos)) = desk.players.get(player) else {
        return;
    };

    // 身上那位神老一天。离场的那位由 `GodLeft` 说，恶犬接位在
    // `gods::land_deity_succession_system` 读那条消息去做，不在这里顺手放。
    let still_held = crate::gods::age_possession(
        &mut desk.commands,
        &mut desk.corrections,
        &mut desk.possessions,
        player,
        &mut writers.god_left,
    );
    // 衰神的「每回合丢一张」。读的是上面那个**返回值**而不是再查一次组件 ——
    // 摘组件走的是 `Commands`，同帧查得到旧值，那样会把「期满那一回合也丢一张」
    // 变成一条静默的规则（`age_possession` 的注里记了这条坑的出处）
    if let Some(god) = still_held {
        crate::gods::drain_cards(
            god,
            player,
            &mut desk.hands,
            &desk.shelf,
            &mut desk.rng,
            &mut writers.card_lost,
        );
    }

    let held = desk.held.get(player).ok().copied();

    // 回合开始即扣一格，所以「只剩 1」表示最后一格服完、本回合可行动。
    // **三条出口写成一张穷尽的 `match` 而不是两个 `filter`**（P12）：`UntilBailed` 那一支
    // 既不该被扣一格，更不该被当成「钟到 0 就放人」—— 漏写它的后果不是崩，是笼中的恶人
    // 自己在环上散步，而 `filter(|d| d.turns_left == 1)` 那种写法拦不住它。
    if let Some(d) = held {
        match d.term {
            // 没有钟：跳过本回合，放人只有保释办得出来。今天走到这一格的那位其实不在轮转里
            // （`villain::villain_rotation_system` 一进笼就把他摘出去了），所以这一支是那道
            // 同步失灵时的最后一道闸 —— 他站着不动，而不是自己走出来。
            Term::UntilBailed => {
                writers.turn_started.write(TurnStarted { player });
                writers.skipped.write(DetentionSkipped {
                    player,
                    place: d.place,
                    left: None,
                });
                next.set(TurnPhase::TurnEnd);
                return;
            }
            Term::Turns(n) if n > 1 => {
                // 先让 UI 显示「谁的回合被跳了」，再发那条凭据（`turn_manager.gd:188-189` 的顺序）
                writers.turn_started.write(TurnStarted { player });
                desk.commands.entity(player).insert(Detention {
                    place: d.place,
                    term: Term::Turns(n - 1),
                });
                writers.skipped.write(DetentionSkipped {
                    player,
                    place: d.place,
                    left: Some(n - 1),
                });
                next.set(TurnPhase::TurnEnd);
                return;
            }
            // 最后一格服完：当场放人，本回合照常行动
            Term::Turns(_) => {
                desk.commands.entity(player).remove::<Detention>();
                writers.released.write(DetentionLifted {
                    player,
                    place: d.place,
                    junction: pos.tile,
                });
            }
        }
    }
    writers.turn_started.write(TurnStarted { player });
    next.set(TurnPhase::WaitRoll);
}

/// 把这一回合用在「原地停留」上（P4④）。与 [`roll_dice_system`] 同一个窗口、同一条资格：
/// 只认当前回合玩家，且他身上必须真的挂着停留计时。
///
/// 计数是**字面**的：卡面写 1 就停 1 个回合，扣到 0 那一格把组件摘掉并发
/// [`StayLifted`]；还剩几格则发 [`StaySkipped`]。不再有「判 3 = 跳 2」那半条 ——
/// 那是拘留的口径（拘留数的是天），两者单位不同，不该共用一把尺。
pub fn take_stay_system(
    mut commands: Commands,
    order: Res<TurnOrder>,
    mut requests: MessageReader<RequestStay>,
    mut staying: Query<&mut StaySentence, With<Walker>>,
    mut skipped: MessageWriter<StaySkipped>,
    mut lifted: MessageWriter<StayLifted>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let Some(current) = order.current() else {
        return;
    };
    // 全部读完再判断：`any()` 会短路，没读完的请求下一帧又被读出来 → 一回合停两次
    let mut asked = false;
    for r in requests.read() {
        asked |= r.player == current;
    }
    if !asked {
        return;
    }
    let Ok(mut sentence) = staying.get_mut(current) else {
        return;
    };
    if sentence.turns_left > 1 {
        sentence.turns_left -= 1;
        skipped.write(StaySkipped {
            player: current,
            turns_left: sentence.turns_left,
        });
    } else {
        commands.entity(current).remove::<StaySentence>();
        lifted.write(StayLifted { player: current });
    }
    next.set(TurnPhase::TurnEnd);
}

/// 掷骰并定下本次移动。只在 `WaitRoll` 里跑，且只认「当前回合玩家」的请求 ——
/// 这是缺陷 #7 的修复点：Godot 的点数由表现层 `board.gd:704` 发出，逻辑因此离不开渲染、
/// headless 根本掷不了骰。
///
/// 逻辑位置（`TokenPosition`）在掷骰的一刻就落定到 `to`；此后 `Walking` 态只是把
/// 这次移动按 `GameRules.walk_step_interval` 展开成逐格时间线（`TileStepped`），
/// 表现层拿它做插值 —— 任何等待动画的 `await` 都不存在了。
///
/// 骰子的点数是**步数**，往哪边走是玩家身上的 [`Heading`]（P3d④b 的转向卡）。两者相乘
/// 喂给 `walk_path`，所以 `DiceRolled.steps` 与 `WalkStarted.steps` 里那个数永远是骰面本身 ——
/// 「掷出 3 点」与「倒着走 3 步」是两件事，流水里不该把它混成一个带符号的位移。
/// `roll_dice_system` 的出口。打包成参数包是房规：骰子、路径与乌龟夹步三件事
/// 各自要留一条凭据，而它们本来就是同一次入队的产物。
#[derive(SystemParam)]
pub struct RollWriters<'w, 's> {
    commands: Commands<'w, 's>,
    rolled: MessageWriter<'w, DiceRolled>,
    started: MessageWriter<'w, WalkStarted>,
    capped: MessageWriter<'w, MoveCapped>,
}

/// 掷骰这条入口要读的玩家状态。名字是为了躲 clippy 的 `type_complexity` ——
/// 六个分量一起回答「他从哪走、往哪走、能摇几颗、有没有被夹住」。
type RollWalker = (
    &'static mut TokenPosition,
    &'static mut WalkQueue,
    &'static Heading,
    &'static Mount,
    Option<&'static StaySentence>,
    Option<&'static mut MoveCap>,
);

/// 遥控骰子那条入口的玩家状态：载具在这里不参与（它决定的是颗数，而这条已经
/// 直接给出步数），所以少一个 `Mount`。
type RemoteWalker = (
    &'static mut TokenPosition,
    &'static mut WalkQueue,
    &'static Heading,
    Option<&'static StaySentence>,
    Option<&'static mut MoveCap>,
);

/// 掷骰并定下本次移动。只在 `WaitRoll` 里跑，且只认「当前回合玩家」的请求 ——
/// 这是缺陷 #7 的修复点：Godot 的点数由表现层 `board.gd:704` 发出，逻辑因此离不开渲染、
/// headless 根本掷不了骰。
///
/// 逻辑位置（`TokenPosition`）在掷骰的一刻就落定到 `to`；此后 `Walking` 态只是把
/// 这次移动按 `GameRules.walk_step_interval` 展开成逐格时间线（`TileStepped`），
/// 表现层拿它做插值 —— 任何等待动画的 `await` 都不存在了。
///
/// 骰子的点数是**步数**，往哪边走是玩家身上的 [`Heading`]（P3d④b 的转向卡）。两者相乘
/// 喂给 `walk_path`，所以 `DiceRolled.steps` 与 `WalkStarted.steps` 里那个数永远是骰面本身 ——
/// 「掷出 3 点」与「倒着走 3 步」是两件事，流水里不该把它混成一个带符号的位移。
///
/// P5 起这里还做三件事：按载具摇 1..=3 颗骰子、乌龟卡把**实际步数**夹到 1（骰面照记）、
/// 路障把路径截断到自身格（ADR-002 §6/§7.1）。
pub fn roll_dice_system(
    mut input: RollInput<'_, '_>,
    mut rng: ResMut<Rng>,
    mut clock: ResMut<WalkClock>,
    mut walkers: Query<RollWalker, With<Walker>>,
    mut writers: RollWriters<'_, '_>,
    mut next: ResMut<NextState<TurnPhase>>,
    mut last: ResMut<LastRoll>,
) {
    let Some(current) = input.order.current() else {
        return;
    };
    // 全部读完再判断：`any()` 会短路，没读完的请求下一帧又被读出来 → 多掷一次
    let mut asked = false;
    let mut wanted = 0;
    for r in input.requests.read() {
        if r.player == current {
            asked = true;
            // 同一帧两条请求只取最后一条的颗数（真人一次只按一颗，AI 发 0）
            wanted = r.dice;
        }
    }
    if !asked {
        return;
    }
    let Ok((mut pos, mut queue, heading, mount, stay, cap)) = walkers.get_mut(current) else {
        return;
    };
    // 被停留按住的人这一回合不能掷骰 —— 他唯一的出口是按「停留」（`RequestStay`）。
    // 判据放在规则层而不是只翻 UI 那颗按钮：AI 与 P12 的恶人没有那颗按钮。
    if stay.is_some() {
        return;
    }

    let from = pos.tile;
    let rolled: u32 = (0..dice_count(wanted, *mount))
        .map(|_| rng.roll_die())
        .sum();
    let cap_turns = cap.as_ref().map(|c| c.turns_left);
    let plan = plan_walk(
        &input.active.def,
        from,
        *heading,
        rolled,
        cap_turns,
        &input.occupancies,
        input
            .villains
            .get(current)
            .is_ok_and(|v| v.kind == crate::villain::VillainKind::Thief),
    );
    let mut target = WalkTarget {
        pos: &mut pos,
        queue: &mut queue,
        from,
    };
    apply_plan(
        &mut writers,
        &mut next,
        &mut clock,
        &mut target,
        current,
        &plan,
        &mut last,
    );
    if let Some(mut held) = cap {
        finish_cap(&mut writers, &mut held, current, &plan);
    }
}

/// 这一掷用几颗骰子。`0` = 满载（步行 1／机车 2／汽车 3）；真人选少了就按选的来。
pub fn dice_count(requested: u32, mount: Mount) -> u32 {
    match requested {
        0 => mount.dice(),
        n => n.clamp(1, mount.dice()),
    }
}

/// 本回合掷出的骰面（P10c）。C 型收费（加油站油费）要在**落格那一刻**读「这一掷是几点」，
/// 而 `DiceRolled` 是消息、只活两帧 —— 从掷骰到落格隔着整段 `Walking` 动画，读不到。
/// 资源在两次掷骰之间一直活着，落格时读它读到的就是本回合的骰面。
/// 写入点只有 [`apply_plan`] 一处：掷骰与遥控骰子两条入口共用它，「骰面照记」的口径
/// 不会在第二条入口上各修一半。遥控骰子记的是它给定的步数 —— 对 C 型来说那就是「走了几步」。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LastRoll(pub Option<(Entity, u32)>);

/// `apply_plan` 要落到的那个玩家：位置、队列、起点。三样一起变，打包成一个结构
/// 而不是八个散装参数（房规：参数多了先问「它们是不是同一件事」）。
struct WalkTarget<'a> {
    pos: &'a mut TokenPosition,
    queue: &'a mut WalkQueue,
    from: crate::board::TileIndex,
}

/// 把一份 [`crate::walk::WalkPlan`] 落到玩家身上，并留三条凭据。掷骰与遥控骰子共用，
/// 所以「骰面照记、实际步数照夹」这条口径不会在两条入口上各修一半。
fn apply_plan(
    writers: &mut RollWriters<'_, '_>,
    next: &mut NextState<TurnPhase>,
    clock: &mut WalkClock,
    target: &mut WalkTarget<'_>,
    player: Entity,
    plan: &crate::walk::WalkPlan,
    last: &mut LastRoll,
) {
    let from = target.from;
    target.pos.tile = plan.to;
    *target.queue = WalkQueue {
        path: plan.path.iter().copied().collect(),
        done: 0,
        from,
    };
    clock.elapsed = 0.0;
    last.0 = Some((player, plan.rolled));
    writers.rolled.write(DiceRolled {
        player,
        steps: plan.rolled,
        from,
        to: plan.rolled_to,
    });
    writers.started.write(WalkStarted {
        player,
        steps: plan.rolled,
        from,
        to: plan.to,
    });
    // 步数为 0（理论不可达：骰子 ≥1）也必须能出 Walking，否则状态机会卡死
    next.set(if target.queue.path.is_empty() {
        TurnPhase::Landed
    } else {
        TurnPhase::Walking
    });
}

/// 乌龟计时扣一格：写到 0 就摘组件，并留一条 [`MoveCapped`]。
fn finish_cap(
    writers: &mut RollWriters<'_, '_>,
    cap: &mut MoveCap,
    player: Entity,
    plan: &crate::walk::WalkPlan,
) {
    let left = plan.cap_left.unwrap_or(0);
    if left == 0 {
        writers.commands.entity(player).remove::<MoveCap>();
    } else {
        cap.turns_left = left;
    }
    writers.capped.write(MoveCapped {
        player,
        rolled: plan.rolled,
        moved: plan.moved,
        turns_left: left,
    });
}

/// 遥控骰子的输入包。与 [`RollInput`] 同形，只是请求换了一种。
#[derive(SystemParam)]
pub struct RemoteInput<'w, 's> {
    requests: MessageReader<'w, 's, RequestRemoteDice>,
    order: Res<'w, TurnOrder>,
    active: Res<'w, ActiveBoard>,
    occupancies: Query<'w, 's, (Entity, &'static Occupancy)>,
    inventories: Query<'w, 's, &'static mut Inventory, With<PlayerId>>,
    /// 与 [`RollInput`] 同一条豁免（P12c）：两个入口共用 [`plan_walk`]，
    /// 判据也共用同一份 —— 恶人今天用不上遥控骰子（他没有道具栏），
    /// 但「走路的人是谁」这一问不该只在一个入口问得出来。
    villains: Query<'w, 's, &'static crate::villain::Villain>,
}

#[derive(SystemParam)]
pub struct RemoteWriters<'w, 's> {
    used: MessageWriter<'w, RemoteDiceUsed>,
    roll: RollWriters<'w, 's>,
}

/// 用一颗遥控骰子定步数。与掷骰共享 [`plan_walk`] 与 [`apply_plan`] ——
/// 乌龟夹步与路障截断不会因为换了入口就漏掉。只在 `WaitRoll`、只认当前回合玩家。
pub fn remote_dice_system(
    mut input: RemoteInput<'_, '_>,
    mut clock: ResMut<WalkClock>,
    mut walkers: Query<RemoteWalker, With<Walker>>,
    mut writers: RemoteWriters<'_, '_>,
    mut next: ResMut<NextState<TurnPhase>>,
    mut last: ResMut<LastRoll>,
) {
    let Some(current) = input.order.current() else {
        for _ in input.requests.read() {}
        return;
    };
    // 全部读完：短路会让没读完的请求下一帧再受理一次
    let mut chosen = None;
    for r in input.requests.read() {
        if r.player == current {
            chosen = Some(r.steps);
        }
    }
    let Some(steps) = chosen else {
        return;
    };
    if !(1..=6).contains(&steps) {
        debug!("遥控骰子被拒：步数 {steps} 不在 1..=6");
        return;
    }
    let Ok((mut pos, mut queue, heading, stay, cap)) = walkers.get_mut(current) else {
        return;
    };
    if stay.is_some() {
        return;
    }
    let Ok(mut inventory) = input.inventories.get_mut(current) else {
        return;
    };
    if !inventory.spend(ItemKind::RemoteDice, 1) {
        debug!("遥控骰子被拒：身上没有这个道具");
        return;
    }
    let from = pos.tile;
    let cap_turns = cap.as_ref().map(|c| c.turns_left);
    let plan = plan_walk(
        &input.active.def,
        from,
        *heading,
        steps,
        cap_turns,
        &input.occupancies,
        input
            .villains
            .get(current)
            .is_ok_and(|v| v.kind == crate::villain::VillainKind::Thief),
    );
    writers.used.write(RemoteDiceUsed {
        player: current,
        steps,
    });
    let mut target = WalkTarget {
        pos: &mut pos,
        queue: &mut queue,
        from,
    };
    apply_plan(
        &mut writers.roll,
        &mut next,
        &mut clock,
        &mut target,
        current,
        &plan,
        &mut last,
    );
    if let Some(mut held) = cap {
        finish_cap(&mut writers.roll, &mut held, current, &plan);
    }
}

/// `end_turn_system` 的那一条「谁还在场上」查询。独立命名是因为它回答两个问题：
/// 终局只数 `Has<PlayerId>` 那一位，轮转数全部 —— 类型太长会被 clippy 拆散，
/// 而拆散后的两半正是函数那条注释要讲的两句话。
type Movers<'w, 's> = bevy::ecs::system::Query<
    'w,
    's,
    (Entity, Option<&'static Bankrupt>, Has<PlayerId>),
    With<Walker>,
>;

/// 回合收尾：发 `TurnEnded`、回合计数、判终局，否则轮转下一位。
///
/// 两个谓词分开写（P12），因为它们回答的是两个不同的问题：
/// - **终局**问的是「还剩几个**玩家**」，所以数的是 `Has<PlayerId>` 那一位 ——
///   场上有四个恶人不是有人赢了的反面证据，把恶人算进 `living` 会永远算不出终局；
/// - **轮转**问的是「下一个能行动的是谁」，所以走的是全部 `Walker` ——
///   用前一个集合去推游标，恶人就被跳过了一格，而「恶人不会动」这件事在画面上
///   看起来完全像是设计如此。读的就是上面那条 [`Movers`]。
pub fn end_turn_system(
    mut order: ResMut<TurnOrder>,
    mut session: ResMut<GameSession>,
    movers: Movers<'_, '_>,
    mut ended: MessageWriter<TurnEnded>,
    mut over: MessageWriter<GameOver>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    let Some(player) = order.current() else {
        return;
    };
    ended.write(TurnEnded { player });
    session.turn_count += 1;

    let alive = |e: Entity| {
        movers
            .get(e)
            .map(|(_, bankrupt, _)| bankrupt.is_none())
            .unwrap_or(false)
    };
    // 只剩一人（或零人）→ 终局。对应 `_check_game_over`
    let living: usize = movers
        .iter()
        .filter(|(_, bankrupt, is_player)| bankrupt.is_none() && *is_player)
        .count();

    if living <= 1 {
        let winner = movers
            .iter()
            .find(|(_, bankrupt, is_player)| bankrupt.is_none() && *is_player)
            .map(|(e, _, _)| e);
        over.write(GameOver { winner });
        next.set(TurnPhase::Over);
        return;
    }

    order.advance_to_active(alive);
    next.set(TurnPhase::TurnStart);
}

/// 每个回合结束重算物价指数（P2③）。
///
/// 排在 `TurnEnd` 的**最后**，因为分子里的股票市值要等行情 tick 完才是今天的数 ——
/// 跑在 `market_tick_on_turn_end_system` 之前会拿到昨天的价，那一帧的指数就说不清是哪天的。
///
/// 一天一个数、整场共用：它是世界的状态，不是某个玩家的状态，所以不进修正登记处
/// （那里每条都要有个 bearer），而是管线第一步直接读 [`GameSession::price_index`]。
pub fn update_price_index_system(mut books: Books, market: Market) {
    let previous = books.session.price_index;
    let total = books.total_assets(previous, |id| market.price(id).unwrap_or(0.0));
    let initial = books.initial_assets();
    let index = economy::price_index(total, initial);
    books.session.price_index = index;
    debug!("物价指数 {previous:.3} → {index:.3}（现资产 {total} / 初始 {initial}）");
}

/// 终局只负责记账与日志；胜负细节在 `GameOver` 消息里，UI 自己读。
pub fn game_over_system(session: Res<GameSession>) {
    info!(
        "对局结束：{} 回合（种子 {:#018x}）",
        session.turn_count, session.seed
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 造一批假 `Entity`：测试里只比较句柄，不碰世界。
    fn ents(n: usize) -> Vec<Entity> {
        (0..n as u32)
            .map(|i| Entity::from_raw_u32(i).expect("索引不该撞上 Entity 的保留值"))
            .collect()
    }

    #[test]
    fn current_active_falls_back_to_the_first_alive_one() {
        let e = ents(3);
        let mut order = TurnOrder::new(e.clone());
        // 0 号破产：应停在 1 号，且幂等
        assert_eq!(order.current_active(|x| x != e[0]), Some(e[1]));
        assert_eq!(order.current_active(|x| x != e[0]), Some(e[1]));
    }

    #[test]
    fn advance_skips_bankrupt_runs() {
        let e = ents(4);
        let mut order = TurnOrder::new(e.clone());
        let alive = |x: Entity| x != e[1] && x != e[2];
        assert_eq!(order.advance_to_active(alive), Some(e[3]));
        assert_eq!(order.advance_to_active(alive), Some(e[0]), "应绕回");
        assert_eq!(order.current(), Some(e[0]));
    }

    #[test]
    fn advance_reports_none_when_nobody_is_left() {
        let e = ents(3);
        let mut order = TurnOrder::new(e.clone());
        assert_eq!(order.advance_to_active(|_| false), None);
        assert_eq!(order.current_active(|_| false), None);
        assert_eq!(TurnOrder::new(vec![]).current_active(|_| true), None);
    }

    /// P12 的进出轮转：游标是一个**下标**而不是一个句柄，所以插一个人和摘一个人
    /// 都可能让它指错人。指错人的症状不是崩溃，是「有人的回合被跳过一格」——
    /// 那种 bug 只能靠把三条口径各钉一条测试钉住。
    #[test]
    fn inserting_before_the_cursor_moves_the_cursor_so_it_still_points_at_the_same_actor() {
        let e = ents(3);
        let mut order = TurnOrder::new(e.clone());
        order.cursor = 1; // 正在 B 的回合上
        let x = Entity::from_raw_u32(99).expect("索引不该撞上保留值");
        assert!(order.insert_after(x, e[0]), "插在 A 之后 = 插到游标正中");
        assert_eq!(order.order, vec![e[0], x, e[1], e[2]]);
        assert_eq!(
            order.current(),
            Some(e[1]),
            "游标要跟着让位，否则 B 的回合被 X 抢走"
        );
        // B 的回合照常结束，下一位还是原来那位 —— 插在他前面的人不许把他的后继吃掉
        assert_eq!(order.advance_to_active(|_| true), Some(e[2]));
        assert_eq!(order.advance_to_active(|_| true), Some(e[0]));
        // 绕回表头之后轮到 X：插队不等于漏掉，他慢一圈而不是消失
        assert_eq!(
            order.advance_to_active(|_| true),
            Some(x),
            "X 要绕一圈才轮得到"
        );
    }

    #[test]
    fn inserting_after_the_current_actor_leaves_him_alone_and_repeats_are_refused() {
        let e = ents(3);
        let mut order = TurnOrder::new(e.clone());
        order.cursor = 0; // 正在 A 的回合上 —— 保释就发生在这里
        let x = Entity::from_raw_u32(98).expect("索引不该撞上保留值");
        assert!(order.insert_after(x, e[0]));
        assert_eq!(order.order, vec![e[0], x, e[1], e[2]]);
        assert_eq!(order.cursor, 0, "插在游标之后不动游标");
        assert_eq!(order.current(), Some(e[0]));
        assert_eq!(
            order.advance_to_active(|_| true),
            Some(x),
            "雇主的回合一轮完就轮到他自己保释出来的那位"
        );
        // 第二次插同一个人：不新增，也不许把游标推歪
        assert!(!order.insert_after(x, e[2]));
        assert_eq!(order.order.len(), 4);
        assert_eq!(order.cursor, 1);
        // 插在一个不在表里的人后面 → 接到表尾（恶人的雇主被摘掉之后的退路，不能 panic）
        let ghost = Entity::from_raw_u32(97).expect("索引不该撞上保留值");
        let y = Entity::from_raw_u32(96).expect("索引不该撞上保留值");
        assert!(order.insert_after(y, ghost));
        assert_eq!(order.order.last(), Some(&y));
        assert_eq!(order.cursor, 1, "接到表尾也不许动游标");
    }

    #[test]
    fn removing_shifts_the_cursor_back_except_when_it_is_the_one_being_removed() {
        let e = ents(4);
        // ① 摘在游标之前：游标退一格，指的还是同一个人
        let mut order = TurnOrder::new(e.clone());
        order.cursor = 2;
        assert!(order.remove(e[0]));
        assert_eq!(order.cursor, 1);
        assert_eq!(order.current(), Some(e[2]));
        // ② 摘的正是游标那一位：游标不动，于是指向他的下一位（他的回合已经用完了）
        let mut order = TurnOrder::new(e.clone());
        order.cursor = 1;
        assert!(order.remove(e[1]));
        assert_eq!(order.cursor, 1);
        assert_eq!(order.current(), Some(e[2]));
        // ③ 摘掉游标上最后一格：归零而不是留下一个越下标
        let mut order = TurnOrder::new(e.clone());
        order.cursor = 3;
        assert!(order.remove(e[3]));
        assert_eq!(order.current(), Some(e[0]));
        // 摘一个不在表里的人：什么都不改
        let ghost = Entity::from_raw_u32(95).expect("索引不该撞上保留值");
        assert!(!order.remove(ghost));
        assert_eq!(order.cursor, 0);
        // 全摘光：游标归零、current 读出 None
        let all = order.order.clone();
        for a in all {
            order.remove(a);
        }
        assert_eq!(order.current(), None);
        assert_eq!(order.cursor, 0);
    }
}
