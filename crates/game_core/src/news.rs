//! 新闻中心（P11）。一块格 → 从整副新闻池里抽一条 → **当场自动播完**，不摆窗口、不要回答。
//!
//! 这个文件里只有两样新东西，其余全是借来的：
//!
//! - [`NewsScope`]：**作用域**这一个维度。命运与机会的作用域永远是「抽卡的那个人」，
//!   而新闻的全部新意在于「**挨打的不是踩格者**」——所以新造的是「这条打在谁头上」，
//!   而不是「这条做什么」。动词一律复用卡池那一套（[`crate::card::CardEffect`]）：
//!   发钱/扣钱/离场/拆房/盖状态/停留…新闻只多带一个「作用于谁」。
//!   这是 PLAN §四 排序原则第 3 条的兑现 —— 「三个来源只是包装，不各写一套效果」。
//! - [`draw`]：✅ 10-20 拍板的发法（**全池随机 + 同一场次内不连抽同一条**）。
//!   命运那副改成了定序（P3d①），新闻刻意不走同一条路：「可数」是命运的玩法资产，
//!   而新闻的乐趣正是不知道下一条是福是祸（4 代这里无任何记载，不欠还原义务）。
//!   代价是一条状态（`GameSession::last_news`，P15 存档要带上）。
//!
//! **为什么不挂 `Pending`**：这里没有「要不要」要等人回答。与 P2b 的圣诞礼物、春节红包
//! 同一类（core 只发凭据，动画归表现层且必须可跳过），理由与 P3a「出牌不挂 Pending」
//! 是同一条纪律的两个方向：`PendingDecision` 的语义是「规则层停下来等这个人」，
//! 而新闻是**世界** Changed，不是**某个人** 被问了一句话。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use serde::Deserialize;

use crate::card::CardEffect;
use crate::data::DataHandles;
use crate::deck::{CardSink, apply_effects};
use crate::landing::{Landed, Landing, LandingDomain};
use crate::msg::{NewsAudience, NewsFired, NewsHit, NewsNotice};

/// 一条新闻在池子里的位置。**存下标而不是 id 字符串**（与 [`crate::deck::CardId`] 同构）：
/// `GameSession::last_news` 要进存档，而下标在「策划改了 id 命名」时不会失效——
/// 失效的是那条记录指向哪一条，那正是 P15「牌堆顺序必须存索引」同一条账。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NewsId(pub u32);

/// 「这条新闻点名哪一位」。作用域是**人**的那一档里的第二层选择。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlayerPick {
    /// 随机一位**别人**。刻意不含开广播的那位 —— PLAN §三 那句「踩格者只负责开广播，
    /// 挨打的是别人」在**这一格**成立；[`NewsScope::Everyone`] 不含这层意思（全场缴税
    /// 当然包括他），而「点名」这一档要是能点到他自己，那句验收就写不出专项测试。
    RandomOther,
    /// 手上现金最多的一位。
    Richest,
    /// 名下地产最多的一位（「公开表扬第一大地主」，P11b②）。
    ///
    /// 先数**块数**再比现值：「大地主」那个词说的是地多，让屏幕上那句话读起来不必心算现值
    /// （两块空地赢一块三层楼，这在玩家眼里是对的）。并列按座位序靠前那位 ——
    /// 与 [`Self::Richest`] 同一条决胜纪律，否则同种子两次跑出两个人，回放当场对不齐。
    ///
    /// **一块地都没有时这一格是空的**（不点名任何人）：表扬一位零地主不是「这条新闻弱」，
    /// 是这条新闻在本局这一刻没有答案，而空答案要说得出为什么空。
    Landlord,
}

/// 「全场里的哪一类人」（P11b②）—— [`NewsScope::EveryoneIf`] 带的那一把筛子。
///
/// 为什么不并进 [`PlayerPick`]：那一格的答案**永远是一个人**，而这一格的答案是一个集合，
/// 而且**可以是空集**（今天没人开车、没人住院）。并成一格的话，「一个都没筛到」与
/// 「筛到一个」在类型上就分不开了，而前者是一句要说出口的人话（顶栏那句「场上没有步行的人」）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FieldFilter {
    /// 步行的人（豪雨：下雨天挨淋的是走路那几位，与 P4④ 之后的「按载具筛」同一条判据）
    OnFoot,
    /// 有载具的人（交通堵塞：机车与汽车都算，读 [`crate::player::Mount::is_vehicle`]）
    Riding,
    /// 关在某一处的人（「医院传出好消息」＝提前出院，P11b② 的 `Release` 配套那一格）
    Detained(crate::player::DetentionPlace),
}

impl FieldFilter {
    /// 人话。顶栏那句「一个都没筛到」与「全场 N 个 …」都要用它 —— 筛过之后再说「可点名的人」
    /// 是一句假话，而那句假话比少一句话更贵（玩家以为自己按错了按钮）。
    ///
    /// 它返回 `String` 而不是其余 `label()` 那种 `&'static str`：`Detained` 那一格的话要拼
    /// 地方的名字，而「地名 → 人话」那一条映射的归属在 [`crate::player::DetentionPlace`]，
    /// 不在这里抄一份。
    pub fn label(self) -> String {
        match self {
            Self::OnFoot => "步行的人".to_string(),
            Self::Riding => "有车的人".to_string(),
            Self::Detained(place) => format!("{}的人", place.held_phrase()),
        }
    }
}

/// 这条新闻打在谁头上。**不许出现 `_`**：新增一档时解析那条（[`audience`]）会编译失败，
/// 逼出一次「它到底找谁」的判断 —— 与 `landing::domain_of`、`charge::scales_with_price`
/// 同一条纪律。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NewsScope {
    /// 全场：还在场上的每一个人（破产者不算，与 P2b 圣诞/红包同一条口径）。
    Everyone,
    /// 点名一位。
    One(PlayerPick),
    /// 全场里满足一把筛子的那些人（P11b②：豪雨按住走路的、堵塞按住开车的、
    /// 好消息放出医院里的那位）。
    ///
    /// 为什么不写成 `Everyone` 外面挂一个「可选的筛子」字段：那样「筛子对 `One`／`Around`
    /// 算什么」要由人记得去问，而类型不拦。这一档与 `Everyone` 共用取数那一段，
    /// 区别只是末尾多一道 `filter`，且**答案可以是空集**（今天没人开车）。
    EveryoneIf(FieldFilter),
    /// 这一带：以**播报者脚下那块地**为锚的一片地（P11b① 的天灾与路段新闻）。
    ///
    /// 锚为什么是「他站的地方」而不是随机一块地、也不是「最富那位的地」：新闻池说的是
    /// 「打到谁」，数据里写不出「哪条街」，而让玩家点那一格就不是新闻了。把锚放在他身上，
    /// 荒诞感就留在规则里 —— 他刚走到自己家那条街旁边的新闻中心，灾正落在他最熟的那几块地。
    /// 脚下那一格通常**不是**地（新闻中心自己就是道路格），所以往环上找最近的一块，
    /// 同距时按他面朝的方向决胜，与请神符同一条键（[`crate::board::BoardDef::nearest_land`]）。
    ///
    /// **范围（整街 / 3×3 / 全图）不住在这里，住在动词里**：`CardEffect::LandStatus{area}`
    /// 与 `Damage{area}` 各自带那一格，这一档只给中心。两处各写一份迟早会分叉，
    /// 而「全场拆房」那种写法里的「全场」是地不是人 —— 它该写成 `Around` + `area: whole`。
    Around,
}

/// 一条效果落在**人**身上还是**地**上 —— [`mismatch`] 的那张配对表用的就是这一位。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// 承受者是一个人（钱、停走、离场、护身…）
    People,
    /// 锚点是一块地（盖状态、打损害）
    Ground,
    /// 今天两档都不属于它。写出来而不是并进前两格，是因为「它为什么不行」每条各有一句
    Neither,
}

/// 动词属于哪一族。**不许出现 `_`**：给 [`crate::card::CardEffect`] 加一条新动词时，
/// 这里必须当场回答「它打在谁身上」，否则它会以「谁都能带」的形状溜进池子，
/// 到运行时才在 [`crate::deck::apply_effects`] 里静默落空。
const fn family_of(effect: &CardEffect) -> Family {
    match effect {
        CardEffect::AddCash { .. }
        | CardEffect::DeductCash { .. }
        | CardEffect::Levy { .. }
        | CardEffect::Detain { .. }
        | CardEffect::Release { .. }
        | CardEffect::MoveTo { .. }
        | CardEffect::Insulate { .. }
        | CardEffect::FreePass { .. }
        | CardEffect::Reverse
        | CardEffect::Stay { .. }
        | CardEffect::MoveCap { .. }
        | CardEffect::SummonGod
        | CardEffect::BanishGod => Family::People,
        CardEffect::LandStatus { .. } | CardEffect::Damage { .. } => Family::Ground,
        // 拍卖那一槌打在**地**上（P11c①）。它当年被关在 `Neither` 里等的就是那一句答案：
        // 「无主的地谁挨这一槌」→ ✅ 10-24 拍板：政府挂牌、所得归银行，于是承受者不是某个
        // 业主而是一片公有地，与 `Damage`／`LandStatus` 同一族。
        // 而它挑客：只拍范围内**无主**的那几块（[`only_public`]），有主的地根本不在它范围内
        CardEffect::Auction { .. } => Family::Ground,
        // 购地卡：买哪块无主地是玩家按下去那一刻定的，而新闻不替人挑地（挑地是它的玩法本体）
        CardEffect::BuyLand => Family::Neither,
    }
}

/// 作用域与动词必须**同族**。`None` = 这一对成立；`Some(理由)` = 开局就拒。
///
/// 混档没有「谁挨打」的答案：地块类新闻没有承受者（无主的地也被拆了），而一条打在一个人
/// 身上的动词拿不到地块锚点 —— 它会在 `apply_effects` 里 `debug!` 一声然后什么都不做，
/// 于是玩家看见顶栏播出一条标题、世界却没变。那种「说了没做」的形状与 P1 那条
/// 「踩上去什么都不发生必须是数据错误」是同一条纪律。
fn mismatch(scope: NewsScope, effect: &CardEffect) -> Option<&'static str> {
    // 先问那一条**跨两档**的一致性：`Release` 自带的 `place` 必须与作用域筛的那一处对上。
    // 它在族上是「打人」的，光看族放得过去，而「筛的是医院、放的是监狱」这一条新闻
    // 是真写得出来的 —— 那种写法会真的把监狱里的人放出来，因为动词自己带着地点。
    if let CardEffect::Release { place } = effect {
        return match scope {
            NewsScope::EveryoneIf(FieldFilter::Detained(there)) if there == *place => None,
            NewsScope::EveryoneIf(FieldFilter::Detained(_)) => {
                Some("它放的那一处与作用域筛的那一处不是同一个地方")
            }
            _ => Some("它放的是关在某一处的人，而这一条的作用域没筛那一处"),
        };
    }
    match scope {
        NewsScope::Everyone | NewsScope::One(_) | NewsScope::EveryoneIf(_) => {
            match family_of(effect) {
                Family::People => None,
                Family::Ground => Some("它打在一块地上，而这一条的作用域是人"),
                Family::Neither => Some("它要的是玩家当场挑一块地，新闻不替人挑"),
            }
        }
        NewsScope::Around => match family_of(effect) {
            Family::Ground => None,
            Family::People => Some("它打在一个人的身上，而这一条的作用域是一片地"),
            Family::Neither => Some("它要的是玩家当场挑一块地，新闻不替人挑"),
        },
    }
}

/// 一条新闻。字段与 [`crate::card::CardDef`] 同构（`id`/`title`/`desc`/`effects`），
/// 只多一个 `scope`、少三样卡面才有的东西（`kind`/`target`/`cost_points`）：
/// 新闻没有副别、不由人出牌、也不上架。
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct NewsDef {
    pub id: String,
    pub title: String,
    pub desc: String,
    pub scope: NewsScope,
    #[serde(default)]
    pub effects: Vec<CardEffect>,
}

/// 新闻池。既是资产（开局按 `DataHandles::news` 加载、`Shelf` 那条路读），也在装配后
/// **克隆一份成资源**给 dump 面板用（与 `HolidaySet` 同一条处理）：`last_news` 存的是
/// 下标，而「上一条是哪一条」要说得出标题就得有这一份。
///
/// 刻意不 `init_resource`：装配之前那份资源如果是「空池」，面板就会把「还没开局」念成
/// 「池子是空的」—— 两句是不同的一句话，混了就再也分不清。
#[derive(Debug, Clone, PartialEq, Asset, TypePath, Deserialize, Resource)]
#[serde(transparent)]
pub struct NewsSet(pub Vec<NewsDef>);

impl NewsSet {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// 加载期的形状校验。与 `CardSet::validate` 同一条理由：**坏数据要在开局就响**，
    /// 而不是让玩家踩到新闻中心那一格之后看见「什么都没发生」。
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        if self.0.is_empty() {
            return Err(vec!["新闻池为空：踩到新闻中心不会有半条新闻".into()]);
        }
        let mut seen = std::collections::HashSet::new();
        for n in &self.0 {
            if n.id.is_empty() {
                errs.push("有新闻 id 为空".into());
            } else if !seen.insert(n.id.as_str()) {
                errs.push(format!("新闻 id 重复: {}", n.id));
            }
            if n.title.is_empty() {
                errs.push(format!("新闻 {} 缺 title", n.id));
            }
            if n.desc.is_empty() {
                errs.push(format!("新闻 {} 缺 desc", n.id));
            }
            // 一条什么都不做的新闻与一颗踩上去没反应的格子是同一件事，而且更隐蔽 ——
            // 顶栏会播出标题，玩家读完才发现世界没变
            if n.effects.is_empty() {
                errs.push(format!("新闻 {} 没有任何效果", n.id));
            }
            for e in &n.effects {
                // 先问同族（那条纪律的理由在 [`mismatch`]），再问这一条自己的数
                if let Some(why) = mismatch(n.scope, e) {
                    errs.push(format!(
                        "新闻 {} 的 {:?} 与 {:?} 配不上：{why}",
                        n.id, e, n.scope
                    ));
                }
                match e {
                    CardEffect::AddCash { amount } | CardEffect::DeductCash { amount } => {
                        if *amount <= 0 {
                            errs.push(format!("新闻 {} 的现金效果数值非法: {amount}", n.id));
                        }
                    }
                    CardEffect::Detain { turns, .. } => {
                        if *turns == 0 {
                            errs.push(format!("新闻 {} 的关押天数是 0", n.id));
                        }
                    }
                    // 与卡池那一条同一个判据（`CardSet::validate`）：0% 是一条不收钱的新闻，
                    // >100% 是「抽的比基数还多」，那该写成定额而不是百分比
                    CardEffect::Levy { percent, .. } => {
                        if *percent == 0 || *percent > 100 {
                            errs.push(format!("新闻 {} 的税率是 {percent}%，要写 1..=100", n.id));
                        }
                    }
                    // `Release` 的数只有 `place` 那一格，而它该与谁配 —— 那条一致性
                    // 是作用域层面的事，住在 [`mismatch`] 里，不在这里重复一遍
                    CardEffect::Release { .. } => {}
                    CardEffect::Stay { turns } | CardEffect::MoveCap { turns } => {
                        if *turns == 0 {
                            errs.push(format!("新闻 {} 的停走回合数是 0", n.id));
                        }
                    }
                    // 0 天的地块状态会在面板上挂满一整天再消失（「剩 0 天」），而它的本意是
                    // 「什么也没发生」—— 与卡池那一条同一个判据（`CardSet::validate`）
                    CardEffect::LandStatus { days, .. } => {
                        if *days == 0 {
                            errs.push(format!("新闻 {} 的地块状态天数是 0", n.id));
                        }
                    }
                    // 一记「谁都不打」的损害不是新闻，是空转：中心与范围都是数据给的，
                    // 算出零块只可能是盘上没地（那一条新闻本来就不该上池子）
                    CardEffect::Damage { .. } => {}
                    _ => {}
                }
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

/// 下标 → 新闻定义。
pub fn news_of(set: &NewsSet, id: NewsId) -> Option<&NewsDef> {
    set.0.get(id.0 as usize)
}

/// `NM_NEWS` 那句「播哪一条」→ 下标。两种写法都认：卡面的 **id 字符串**（`news_002`，
/// 改数据时它跟着行走）与池子里的**序号**（`2`，截图脚本里省事）。
///
/// 认不出来时返回 `None` 而不是退成「随机一条」：旋钮打错字的症状如果是「播了一条
/// 谁都没预期的新闻」，比「一句 warn 都不响」更难读 —— 与点券格那条「踩上去什么都不
/// 发生必须是数据错误」同一条纪律。
pub fn find_id(set: &NewsSet, wanted: &str) -> Option<NewsId> {
    if let Some(at) = set.0.iter().position(|n| n.id == wanted) {
        return Some(NewsId(at as u32));
    }
    let num: usize = wanted.parse().ok()?;
    (num < set.0.len()).then_some(NewsId(num as u32))
}

/// [`draw`] 的**取数值域**，与它放在一起而不是让调用方自己算：这两句是同一个算法的两半，
/// 分开写迟早分叉（值域给少了抽不到某几条，给多了那条「不连重」就失效）。
///
/// 有上一条时值域比池子**少一格** —— 少的那一格正是用来跳过的（见 [`draw`]）；
/// 没有上一条（这一局第一次播）时给满，否则池子里最后一条永远抽不到。
pub fn roll_span(pool: usize, last: Option<NewsId>) -> std::ops::RangeInclusive<u32> {
    match last {
        Some(_) if pool > 1 => 0..=pool.saturating_sub(2) as u32,
        _ => 0..=pool.saturating_sub(1) as u32,
    }
}

/// **「全池随机 + 同一场次内不连抽同一条」的唯一一处实现。**
///
/// `roll` 的取值域由 [`roll_span`] 给：**有上一条时是 `0..=pool-2`，没有时是 `0..=pool-1`**。
/// 少那一格就是「不连重」的实现本身 —— 把 `>= last` 的那一格往后顶一位，上一条因此
/// 永远抽不到，而其余每一条各命中一次（不偏）。
///
/// 为什么不用「抽到一样就重抽」：那是一条**取数次数不定**的路径 —— 同一个种子跑两遍
/// 结果虽然一样，但「重抽了几次」成了一条只有代码知道的隐藏状态，而它随池子大小变化。
/// 少抽一格的映射是**一次取数、一个确定的数**，池子改成几条都不影响别的流。
///
/// `pool == 1` 时无所谓连不连重（只有那一条），`prev` 那一格因此**只在池子真的大于 1 时**
/// 才参与。末尾的 `% pool` 不是设计的一部分，是「哪天调用方改了值域忘了这里」的兜底：
/// 越界只可能少播或多播一条，不该 panic，也不该指向一条不存在的新闻。
#[must_use]
pub fn draw(pool: usize, last: Option<NewsId>, roll: u32) -> Option<NewsId> {
    if pool == 0 {
        return None;
    }
    let prev = last.filter(|_| pool > 1).map(|l| l.0 as usize);
    let taken = roll as usize;
    let id = match prev {
        Some(p) if taken >= p => taken + 1,
        _ => taken,
    };
    Some(NewsId((id % pool) as u32))
}

/// 这一条新闻点名到的那几个人（**已经过滤掉破产者**）。人档专用 —— 地档走
/// [`ground_anchor`]，它给的是中心而不是承受者。
///
/// 返回的是「承受者」而不是「谁触发」：踩格的人由调用方带在凭据上（`NewsFired::broadcaster`），
/// 两处分开才是这条玩法的全部新意 —— 合成一个人的话，「他开广播又挨了打」与
/// 「他挨了打」在流水里读起来是同一句话。
///
/// `people` 要 `&mut CardSink` 而不是「账本 + 随机流」两格：随机流挂在
/// [`crate::gods::BestowKit`] 里（那条流是 `Rng` 的第十五条，见 [`crate::rng`]），
/// 而账本与它在同一个参数包上，分两头借会在参数建立期撞。
fn people(scope: NewsScope, actor: Entity, sink: &mut CardSink) -> Vec<Entity> {
    match scope {
        // 全场：破产的人已经不在牌桌上（与 P2b 的礼物/红包同一条判据）
        NewsScope::Everyone => sink.books.solvent().into_iter().map(|(p, _)| p).collect(),
        NewsScope::One(pick) => {
            let field = sink.books.solvent();
            match pick {
                // 「随机一位别人」：候选里先摘掉开广播的那位，再从剩下的里面抽。
                // 只剩他一个人时这一条抽不出别人 —— 那不是错误，是「这一局只有你」，
                // 于是这一格什么人都没点到，凭据照播（`hits` 为空）
                PlayerPick::RandomOther => {
                    let others: Vec<Entity> = field
                        .iter()
                        .map(|(p, _)| *p)
                        .filter(|p| *p != actor)
                        .collect();
                    if others.is_empty() {
                        return Vec::new();
                    }
                    let at = sink.kit.rng.pick_news(0..=(others.len() - 1) as u32) as usize;
                    vec![others[at]]
                }
                // 「现金最多的一位」：并列时取座位序里靠前的那一个 —— 必须有个确定的决胜，
                // 否则同一种子两次跑出两个人，回放当场对不齐
                PlayerPick::Richest => {
                    let mut best: Option<(i64, Entity)> = None;
                    for (player, _) in field {
                        let cash = sink.books.balance(player);
                        if best.is_none_or(|(top, _)| cash > top) {
                            best = Some((cash, player));
                        }
                    }
                    best.map(|(_, p)| vec![p]).unwrap_or_default()
                }
                // 「第一大地主」：先数块、再比现值，两块都是 0 的人**不算地主** ——
                // 于是「这一局还没人买到地」是一个空答案而不是一次勉强的点名
                // （空答案要说得出为什么空，见 `ui::top_bar` 那句「场上没有地主」）
                PlayerPick::Landlord => {
                    let mut best: Option<(u32, i64, Entity)> = None;
                    for (player, _) in field {
                        let count = sink.books.land_count_of(player);
                        if count == 0 {
                            continue;
                        }
                        let value = sink.books.land_value_of(player);
                        if best.is_none_or(|(c, v, _)| (count, value) > (c, v)) {
                            best = Some((count, value, player));
                        }
                    }
                    best.map(|(_, _, p)| vec![p]).unwrap_or_default()
                }
            }
        }
        // 全场里筛一类人（P11b②）。名单一次算完再筛，而不是三条路各查一遍：
        // 「谁还在牌桌上」这一问只有一个答案（破产者不算），它与筛子是两道独立的判据，
        // 合成一次查询的话，将来加一把新筛子就要重新回答一次「破产者算不算」
        NewsScope::EveryoneIf(filter) => {
            // 按处筛人读的是账本那份名单（`Books::held`）：它同时拿着实体与「他关在哪」，
            // 而这里要的正是这两样（P4 为保释面板建它的那条理由在这里一字不改）
            let held: Vec<Entity> = match filter {
                FieldFilter::Detained(place) => sink
                    .books
                    .held(place)
                    .into_iter()
                    .map(|(player, _)| player)
                    .collect(),
                FieldFilter::OnFoot | FieldFilter::Riding => Vec::new(),
            };
            sink.books
                .solvent()
                .into_iter()
                .filter(|(player, _)| match filter {
                    FieldFilter::OnFoot | FieldFilter::Riding => match sink.mounts.get(*player) {
                        // 筛「有车」= 是，筛「步行」= 否。缺组件不是「步行」也不是「有车」：
                        // `spawn_players` 漏挂是 bug，不是处境，所以两把筛子都把他挡在外面并响一声
                        Ok(mount) => mount.is_vehicle() == matches!(filter, FieldFilter::Riding),
                        Err(_) => {
                            warn!(
                                "{} 身上没有 Mount 组件，两把按载具的筛子都把他算成缺席",
                                player
                            );
                            false
                        }
                    },
                    FieldFilter::Detained(_) => held.contains(player),
                })
                .map(|(player, _)| player)
                .collect()
        }
        // 地档没有「哪几个人」这一问：它的锚是一块地，答案在 [`ground_anchor`] 那里。
        // 这一格今天走不到（`fire_news` 先按作用域分开了两条路），写出来是为了
        // **这张表不许有 `_`** —— 有人把 `Around` 接进人档时，症状得是一声 warn，
        // 而不是「播了一条谁都没点到的新闻」
        NewsScope::Around => {
            warn!("地档新闻走进了人档的取数：这一条该由 ground_anchor 回答锚点");
            Vec::new()
        }
    }
}

/// `settle_news_landing_system` 的输入包：卡池那一份参数包 + 新闻池资产 + 播新闻的凭据。
///
/// 直接**内含** [`CardSink`] 而不是重列那十几格：效果动词的执行者与出牌/抽卡是同一个
/// （[`apply_effects`]），而三样东西（账本、Commands、那一堆查询）少借一个都会在某条效果上
/// 缺一格。`Books` 与 `Commands` 在同一条 system 里出现两次会直接在参数建立期撞
/// （§6.1 坑 33），所以只能嵌套、不能并列。
#[derive(SystemParam)]
pub struct NewsSink<'w, 's> {
    pub card: CardSink<'w, 's>,
    sets: Res<'w, Assets<NewsSet>>,
    data: Res<'w, DataHandles>,
    pub fired: MessageWriter<'w, NewsFired>,
}

impl<'w> NewsSink<'w, '_> {
    /// 整副新闻池（`NM_NEWS` 那句「播哪一条」要在里面点名）。
    pub fn pool(&self) -> Option<&NewsSet> {
        self.sets.get(&self.data.news)
    }
}

/// 落格分派的那一支：踩到新闻中心 → 抽一条 → **当场播完并结算**，不挂窗口。
///
/// 抽与播分两步是刻意的：抽这一格只有这里会做（它读随机流与 `last_news`），
/// 而 [`fire_news`] 要能被 `NM_NEWS` 直接点名调用 —— 开发旋钮注入的是「播哪一条」，
/// 它不该顺手替随机流取一个数（那会让同一颗种子的截图与真实一局的新闻序对不上）。
pub fn settle_news_landing_system(landing: Res<Landing>, mut sink: NewsSink) {
    let Some(Landed {
        player: actor,
        domain: LandingDomain::News,
        ..
    }) = landing.0
    else {
        return;
    };
    let Some(set) = sink.sets.get(&sink.data.news) else {
        // 资产没就位：开局那一步（`world::begin_game_system`）等的是这七份数据，
        // 走到这里说明门禁漏了。宁可不播也不猜一条
        warn!("新闻池资产读不到，这一格没能播出任何新闻");
        return;
    };
    let pool = set.0.len();
    // 取数与映射成对读 [`roll_span`]：值域随「这一局播过没有」变，而两条都在同一个文件里
    let last = sink.card.books.session.last_news;
    let roll = sink.card.kit.rng.pick_news(roll_span(pool, last));
    let Some(id) = draw(pool, last, roll) else {
        return;
    };
    fire_news(&mut sink, actor, id);
}

/// 把指定那一条新闻打在 `actor` 播出来的广播里。返回「这一条真的在池子里吗」。
///
/// **它不动随机流**：取数是「抽」那一半的事（[`settle_news_landing_system`] 才读
/// `pick_news`）。但它**会写 `last_news`** —— 那个位的语义是「这一场刚播出去的是哪条」，
/// 而不是「刚抽到的是哪条」：`NM_NEWS` 点名播出去的那一条，玩家看得见，下一次抽就该躲开它。
/// 两处各写一半的话，注入过的那条就会在下一格被重播一遍。
pub fn fire_news(sink: &mut NewsSink, actor: Entity, id: NewsId) -> bool {
    let Some(set) = sink.sets.get(&sink.data.news) else {
        warn!("新闻池资产读不到，没能播 {id:?}");
        return false;
    };
    let Some(def) = news_of(set, id) else {
        // 下标越界：只有可能是调用方自己算的（落格那条永远在域内），所以这里响一声
        warn!("新闻池里没有 #{id:?} 这一条");
        return false;
    };
    // 「上一条是哪一条」由**播**这一处写，而不是由抽那一处：`NM_NEWS` 点名的那一条同样
    // 算「这一场刚播过的」，于是它之后第一次踩新闻中心不会又摸到同一条。写在两处会分叉
    sink.card.books.session.last_news = Some(id);
    let (title, desc, effects, news_id) = (
        def.title.clone(),
        def.desc.clone(),
        def.effects.clone(),
        def.id.clone(),
    );
    let scope = def.scope;
    let audience = match scope {
        NewsScope::Everyone | NewsScope::One(_) | NewsScope::EveryoneIf(_) => {
            let targets = people(scope, actor, &mut sink.card);
            let mut hits = Vec::with_capacity(targets.len());
            for target in targets {
                hits.push(NewsHit {
                    player: target,
                    outcomes: apply_effects(
                        &mut sink.card,
                        target,
                        &effects,
                        &news_id,
                        None,
                        // 人档今天拿不到拍卖那一槌（[`mismatch`] 把 `Auction` 留在地档），
                        // 所以这一格是一条走不到的路。它写 `Public` 而不是留空位：
                        // 「新闻中心这一播」开的槌就是公家那一把，与地档同一句
                        crate::auction::Gavel::Public,
                    ),
                });
            }
            NewsAudience::People(hits)
        }
        NewsScope::Around => {
            // 锚点是播报者脚下那块地。`None` 只可能是「这张盘上一块地都没有」——
            // 那时这一播扫不到任何东西，但仍然要播出去：顶栏那句「这一带没有地可打」与
            // 「场上没有可点名的人」是同一条纪律，空手要说得出为什么空
            let center = ground_anchor(actor, &mut sink.card);
            let lands = match center {
                Some(at) => touched(&mut sink.card, at, &effects),
                None => Vec::new(),
            };
            // `victim` 那一格对地块类动词是死的（[`mismatch`] 保证走到这里的只有地块类动词），
            // 传 `actor` 只为让签名成立 —— 地档没有「谁挨的」，只有「谁开的广播」。
            // `Gavel::Public` 是这条新闻的**身份**而不是一个占位：新闻不替某个玩家敲槌，
            // 它替公家挂牌 —— 所得因此归银行（✅ 10-24 拍板），而不是归刚踩到那一格的玩家
            let outcomes = apply_effects(
                &mut sink.card,
                actor,
                &effects,
                &news_id,
                center.map(crate::hand::CardAim::Land),
                crate::auction::Gavel::Public,
            );
            NewsAudience::Ground {
                center,
                lands,
                outcomes,
            }
        }
    };
    let readout = match &audience {
        NewsAudience::People(hits) => format!("点到 {} 个人", hits.len()),
        NewsAudience::Ground { lands, .. } => format!("扫到 {} 块地", lands.len()),
    };
    info!("新闻「{title}」（{news_id}）：由 {actor:?} 播出，{readout}");
    sink.fired.write(NewsFired {
        broadcaster: actor,
        news: NewsNotice {
            id: news_id,
            title,
            desc,
            scope,
            audience,
        },
    });
    true
}

/// 这一条新闻的锚点：播报者**脚下那块地**（他站的那一格不是地时，往环上找最近的一块）。
///
/// 位置与朝向都读他自己身上那两个组件，而不是「当前回合玩家」—— 注入路径（`NM_NEWS`）
/// 与踩格路径的播报者可以是两个人，而锚该跟着开广播的那一位走。
fn ground_anchor(actor: Entity, sink: &mut CardSink) -> Option<crate::land::LandId> {
    let at = sink.positions.get(actor).ok()?.tile;
    let heading = *sink.headings.get(actor).ok()?;
    let (land, _, _) = sink.books.active.def.nearest_land(at, heading)?;
    Some(crate::land::LandId(land))
}

/// 这一条动词的作用范围 —— 地块类动词各带一格，打在人身上的动词没有。
///
/// 这里**也**不带 `_`，因为两条表是对同一个枚举各表态一次：「打在地上」与
/// 「带一片范围」必须是同一条判断。写成 `Some(..) | _ => None` 的话，将来加一条地块类动词
/// 只需在那儿登记一次、这里漏一次，而漏掉的形状是「新闻播了、地也改了，凭据上那份清单却是
/// 空的」—— 顶栏念得出「拆了几块」，念不出**拆了哪几块**。两条表都要求新变体当场表态，
/// 那种半截登记就只剩编译错误这一种结局
/// （[`a_ground_verb_is_the_one_that_carries_a_land_area`] 数的是两条表已登记的那些对不对得上）。
fn verb_area(effect: &CardEffect) -> Option<crate::plot::PlotArea> {
    match effect {
        CardEffect::LandStatus { area, .. }
        | CardEffect::Damage { area, .. }
        | CardEffect::Auction { area } => Some(*area),
        CardEffect::AddCash { .. }
        | CardEffect::DeductCash { .. }
        | CardEffect::Levy { .. }
        | CardEffect::Detain { .. }
        | CardEffect::Release { .. }
        | CardEffect::MoveTo { .. }
        | CardEffect::Insulate { .. }
        | CardEffect::FreePass { .. }
        | CardEffect::BuyLand
        | CardEffect::Reverse
        | CardEffect::Stay { .. }
        | CardEffect::MoveCap { .. }
        | CardEffect::SummonGod
        | CardEffect::BanishGod => None,
    }
}

/// 范围内的这一块地，这条动词**要不要**（P11c① 加的第三问）。
///
/// 为什么范围要两张表而不是一张：`Auction` 带一个 `area`，却只拍其中**无主**的那几块 ——
/// 「这一带有几块地」与「这一带有几块公有地」在这块盘上是两个差很多的数（开局几乎全部无主，
/// 中盘只剩边角几块）。合写一张表的话，凭据那份清单去念一个没有发生的数：
/// 顶栏播「公有地拍卖：9 块」而拍卖机只开场 2 场。
///
/// 也不许写成 `Auction => true | _ => false` 之外再漏一条：新的地块类动词必须**当场**回答
/// 「你挑不挑客」。下面那些打在人身上的动词今天问不到（[`verb_area`] 对它们是 `None`），
/// 但这一格照样要表态 —— 有人把某条动词挪去地档而这里没改，症状就是它悄悄把有主的也扫进清单。
fn only_public(effect: &CardEffect) -> bool {
    match effect {
        CardEffect::Auction { .. } => true,
        CardEffect::LandStatus { .. } | CardEffect::Damage { .. } => false,
        CardEffect::AddCash { .. }
        | CardEffect::DeductCash { .. }
        | CardEffect::Levy { .. }
        | CardEffect::Detain { .. }
        | CardEffect::Release { .. }
        | CardEffect::MoveTo { .. }
        | CardEffect::Insulate { .. }
        | CardEffect::FreePass { .. }
        // 购地卡今天连范围都没有，而它**确实**只挑无主的地 —— 那条判据住在
        // `Books::buy_price`（无主 + 付得起），不在这里。这一格填 `false` 说的是
        // 「这张表管不到它」，不是「它不挑客」
        | CardEffect::BuyLand
        | CardEffect::Reverse
        | CardEffect::Stay { .. }
        | CardEffect::MoveCap { .. }
        | CardEffect::SummonGod
        | CardEffect::BanishGod => false,
    }
}

/// 这一播**真的扫到了哪几块地**（各动词的范围取并集、去重、排序）。
///
/// 它是凭据上那份清单，不是 `apply_effects` 的副产品：效果凭据带的是块数（`Stamped{lands}`
/// 与 `Razed{lands}`，与卡池共用同一套形状），而新闻要说得出「外星人拆了哪几块」——
/// 那句话里没有块数就没有「我家那块在不在里面」。清单由这里算而不是从 `damage` 里挤出来，
/// 是为了不把 `DamageTally` 的 `Copy` 破掉（它被飞弹、炸弹、恶魔与测试共用）。
///
/// 挑客的那一条读 [`crate::settle::Books::public_lands`] —— 与执行侧（`deck::apply_effects`
/// 的拍卖那一槌）同一个集合，所以「清单上几块」与「挂上台几块」在结构上是同一个数。
fn touched(
    sink: &mut CardSink,
    center: crate::land::LandId,
    effects: &[CardEffect],
) -> Vec<crate::land::LandId> {
    let mut lands: Vec<crate::land::LandId> = Vec::new();
    for effect in effects {
        let Some(area) = verb_area(effect) else {
            continue;
        };
        let found = if only_public(effect) {
            sink.books.public_lands(center, area)
        } else {
            sink.books.area_of(center, area)
        };
        lands.extend(found);
    }
    lands.sort();
    lands.dedup();
    lands
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::player::DetentionPlace;

    #[test]
    fn an_empty_pool_is_a_data_error() {
        let errs = NewsSet(Vec::new()).validate().unwrap_err();
        assert!(errs[0].contains("新闻池为空"));
    }

    #[test]
    fn a_news_that_does_nothing_is_rejected_at_load() {
        let set = NewsSet(vec![news("news_001", NewsScope::Everyone, vec![])]);
        let errs = set.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("没有任何效果")));
    }

    #[test]
    fn the_same_id_twice_is_a_data_error() {
        let set = NewsSet(vec![
            cash_news("dup", NewsScope::Everyone, 100),
            cash_news("dup", NewsScope::One(PlayerPick::Richest), 100),
        ]);
        let errs = set.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("id 重复")));
    }

    #[test]
    fn a_ground_verb_only_passes_under_the_ground_scope() {
        // 同一条动词，两档作用域两种命运：这条断言钉的是「混档会在开局就响」，
        // 而不是等到玩家踩上新闻中心、顶栏播出一条标题、街上什么都没变
        let wrong = NewsSet(vec![news(
            "news_alien",
            NewsScope::Everyone,
            vec![CardEffect::Damage {
                depth: crate::damage::LandDamage::Clear { seize: true },
                area: crate::plot::PlotArea::Street,
            }],
        )]);
        let errs = wrong.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("它打在一块地上，而这一条的作用域是人")),
            "{errs:?}"
        );

        let right = NewsSet(vec![news(
            "news_alien",
            NewsScope::Around,
            vec![CardEffect::Damage {
                depth: crate::damage::LandDamage::Clear { seize: true },
                area: crate::plot::PlotArea::Street,
            }],
        )]);
        assert!(
            right.validate().is_ok(),
            "地档带动词该过：{:?}",
            right.validate()
        );
    }

    #[test]
    fn a_people_verb_does_not_belong_to_a_ground_scope() {
        let set = NewsSet(vec![cash_news("news_subsidy", NewsScope::Around, 1_500)]);
        let errs = set.validate().unwrap_err();
        assert!(
            errs.iter()
                .any(|e| e.contains("它打在一个人的身上，而这一条的作用域是一片地")),
            "{errs:?}"
        );
    }

    #[test]
    fn a_ground_status_still_needs_a_number_of_days() {
        // 「天数是 0」那条判据从卡池搬过来：作用域换档不改变「0 天等于什么也没盖」
        let set = NewsSet(vec![news(
            "news_hike",
            NewsScope::Around,
            vec![CardEffect::LandStatus {
                status: crate::status::LandStatus::PriceHike,
                days: 0,
                area: crate::plot::PlotArea::Street,
            }],
        )]);
        let errs = set.validate().unwrap_err();
        assert!(
            errs.iter().any(|e| e.contains("地块状态天数是 0")),
            "{errs:?}"
        );
    }

    #[test]
    fn a_named_tile_now_belongs_to_the_people_side() {
        // P11a 那一步 `MoveTo` 是被**整条拒掉**的（那时新闻池只有人档，而拒掉的理由写的是
        // 「需要作用域之外的锚点」—— 那句对 `LandStatus` 成立，对 `MoveTo` 其实不成立：
        // 它的锚点就在条目上）。今天它按它本来的样子过：打在一个人身上、格名写在数据里
        let set = NewsSet(vec![news(
            "news_move",
            NewsScope::Everyone,
            vec![CardEffect::MoveTo {
                tile_name: "南京路".into(),
            }],
        )]);
        assert!(set.validate().is_ok(), "{:?}", set.validate());
    }

    #[test]
    fn the_land_choosing_card_verb_still_belongs_to_neither_side() {
        // 购地卡要有人**当场挑一块无主地**（那是它的玩法本体），而新闻不替人挑。
        // P11c① 之前这张表上有两位（购地 + 拍卖），拍卖那一位被搬去地档 —— 搬的理由写在
        // [`family_of`] 里那句「无主的地谁挨这一槌」已经答上。这条用例留在原地挡另一条：
        // 「新闻顺手替玩家挑一块地买下来」今天仍然写不出来
        for scope in [NewsScope::Everyone, NewsScope::Around] {
            let set = NewsSet(vec![news("news_grab", scope, vec![CardEffect::BuyLand])]);
            let errs = set.validate().unwrap_err();
            assert!(
                errs.iter().any(|e| e.contains("玩家当场挑一块地")),
                "news_grab 在 {scope:?} 下居然过了：{errs:?}"
            );
        }
    }

    #[test]
    fn a_public_auction_is_a_ground_news() {
        // P11c① 那条「公开拍卖公有土地」的形状：地档 + 拍卖那一槌。三格各问一次，
        // 缺任一格都会漏掉一种坏数据：
        // - 地档过：这条新闻进得了池子（进不去的话 PLAN 第二批那一条永远落不了地）
        // - 人档拒：点名一位玩家 + 拍卖 = 「新闻替他挑了一块公有地」，那种挑法没人按过按钮
        // - 筛过的人档也拒：`EveryoneIf` 的答案是「几个人」，不是「几块地」
        let auction = || CardEffect::Auction {
            area: crate::plot::PlotArea::Box { side: 3 },
        };
        let right = NewsSet(vec![news(
            "news_public",
            NewsScope::Around,
            vec![auction()],
        )]);
        assert!(
            right.validate().is_ok(),
            "地档 + 拍卖该过：{:?}",
            right.validate()
        );
        for scope in [
            NewsScope::Everyone,
            NewsScope::One(PlayerPick::Richest),
            NewsScope::EveryoneIf(FieldFilter::OnFoot),
        ] {
            let set = NewsSet(vec![news("news_public", scope, vec![auction()])]);
            let errs = set.validate().unwrap_err();
            assert!(
                errs.iter().any(|e| e.contains("作用域是人")),
                "{scope:?} + 拍卖该红在「作用域是人」上：{errs:?}"
            );
        }
    }

    #[test]
    fn a_ground_verb_is_the_one_that_carries_a_land_area() {
        // 「打在地上」「带一片范围」「范围内挑不挑客」是同一条判断的三半，各住在一张表里。
        // 这里把每一条动词都问一遍三半是否一致 —— 半截登记在画面上的形状有两种：
        // 「标题播了、地也拆了，而那份『哪几块』是空的」，以及「清单写 9 块而拍卖机只开场 2 场」。
        // 编译期那一半牙齿在三张都不带 `_` 的 match 上
        for effect in all_verbs() {
            let ground = matches!(family_of(&effect), Family::Ground);
            assert_eq!(
                ground,
                verb_area(&effect).is_some(),
                "{effect:?} 两条表读数不一致：地档={ground} 范围={:?}",
                verb_area(&effect)
            );
            assert!(
                !only_public(&effect) || ground,
                "{effect:?} 挑客却不打在地上：那句「只拍无主」没人读得到"
            );
        }
    }

    /// 每一条动词各取一个样本。列在这里而不是靠宏：要的是「这几张表对同一批东西说过话」，
    /// 而漏列一条的后果由 [`family_of`] / [`verb_area`] / [`only_public`] 的穷尽 match 挡
    /// （新变体每张都得表态）。
    fn all_verbs() -> Vec<CardEffect> {
        use crate::player::DetentionPlace;
        use crate::plot::PlotArea;
        use crate::status::LandStatus as Plot;
        vec![
            CardEffect::AddCash { amount: 100 },
            CardEffect::DeductCash { amount: 100 },
            CardEffect::Detain {
                place: DetentionPlace::Jail,
                turns: 3,
            },
            CardEffect::Release {
                place: DetentionPlace::Hospital,
            },
            CardEffect::Levy {
                basis: crate::card::LevyBasis::Cash,
                percent: 10,
            },
            CardEffect::MoveTo {
                tile_name: "南京路".into(),
            },
            CardEffect::Insulate { charges: 1 },
            CardEffect::LandStatus {
                status: Plot::PriceHike,
                days: 5,
                area: PlotArea::Street,
            },
            CardEffect::Damage {
                depth: crate::damage::LandDamage::Clear { seize: true },
                area: PlotArea::Box { side: 3 },
            },
            CardEffect::FreePass { floor: 2_000 },
            CardEffect::BuyLand,
            CardEffect::Reverse,
            CardEffect::Stay { turns: 1 },
            CardEffect::MoveCap { turns: 3 },
            CardEffect::SummonGod,
            CardEffect::BanishGod,
            CardEffect::Auction {
                area: PlotArea::Box { side: 3 },
            },
        ]
    }

    #[test]
    fn a_release_only_pairs_with_the_place_the_scope_filtered() {
        // 「筛的是医院、放的是监狱」这一条新闻是真写得出来的（两个字段各写一半就会分叉），
        // 而它的形状是「播了一条好消息、监狱里的人走出来了」—— 那既不是这条新闻说的话，
        // 也不是任何人按下去的决定。
        // 三格各问一次：同处过、异处拒、没筛拒 —— 「异处」那一格是这一条的存在理由，
        // 只写「没筛就拒」的话，把两处写成一处也能骗过校验
        let release = |scope, place| {
            NewsSet(vec![news(
                "news_release",
                scope,
                vec![CardEffect::Release { place }],
            )])
        };
        let hospital = NewsScope::EveryoneIf(FieldFilter::Detained(DetentionPlace::Hospital));
        let jail = NewsScope::EveryoneIf(FieldFilter::Detained(DetentionPlace::Jail));
        assert!(
            release(hospital, DetentionPlace::Hospital)
                .validate()
                .is_ok(),
            "筛医院放医院该过：{:?}",
            release(hospital, DetentionPlace::Hospital).validate()
        );
        for (scope, place, why) in [
            (hospital, DetentionPlace::Jail, "不是同一个地方"),
            (jail, DetentionPlace::Hospital, "不是同一个地方"),
            (NewsScope::Everyone, DetentionPlace::Hospital, "没筛那一处"),
            (
                NewsScope::One(PlayerPick::Richest),
                DetentionPlace::Hospital,
                "没筛那一处",
            ),
        ] {
            let errs = release(scope, place).validate().unwrap_err();
            assert!(
                errs.iter().any(|e| e.contains(why)),
                "{scope:?} + 放{place:?} 该红在「{why}」上：{errs:?}"
            );
        }
    }

    #[test]
    fn a_filtered_scope_is_the_people_side() {
        // 新档加在「打人」这一侧：地档动词配不上它，与 `Everyone` 同一条判据
        let set = NewsSet(vec![news(
            "news_rain",
            NewsScope::EveryoneIf(FieldFilter::OnFoot),
            vec![CardEffect::Damage {
                depth: crate::damage::LandDamage::Clear { seize: false },
                area: crate::plot::PlotArea::One,
            }],
        )]);
        let errs = set.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("作用域是人")), "{errs:?}");
    }

    #[test]
    fn a_tax_rate_must_be_between_one_and_a_hundred_percent() {
        // 0% 是一条不收钱的新闻（它照样会播标题、留一条 0 元流水），>100% 是「抽的比基数
        // 还多」—— 那该写成定额扣钱而不是百分比。100% 合法：那是「全额充公」，
        // 一条「把名下地价全部收走」的税（注意它**不是** PLAN §四 第二批那条「强制征收」：
        // 那条收的是**地本身**，走 `Damage{Clear{seize}}` 而不是税，见 `news_014`）
        for percent in [0, 101, 250] {
            let set = NewsSet(vec![news(
                "news_tax",
                NewsScope::Everyone,
                vec![CardEffect::Levy {
                    basis: crate::card::LevyBasis::Cash,
                    percent,
                }],
            )]);
            let errs = set.validate().unwrap_err();
            assert!(
                errs.iter().any(|e| e.contains("要写 1..=100")),
                "税率 {percent}% 居然过了校验：{errs:?}"
            );
        }
        let whole = NewsSet(vec![news(
            "news_seize",
            NewsScope::Everyone,
            vec![CardEffect::Levy {
                basis: crate::card::LevyBasis::LandValue,
                percent: 100,
            }],
        )]);
        assert!(
            whole.validate().is_ok(),
            "100% 是全额：{:?}",
            whole.validate()
        );
    }

    #[test]
    fn a_zero_turn_freeze_is_a_data_error() {
        let set = NewsSet(vec![NewsDef {
            id: "news_rain".into(),
            title: "豪雨".into(),
            desc: "全场停走".into(),
            scope: NewsScope::Everyone,
            effects: vec![CardEffect::Stay { turns: 0 }],
        }]);
        let errs = set.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("停走回合数是 0")));
    }

    fn news(id: &str, scope: NewsScope, effects: Vec<CardEffect>) -> NewsDef {
        NewsDef {
            id: id.into(),
            title: "标题".into(),
            desc: "正文".into(),
            scope,
            effects,
        }
    }

    fn cash_news(id: &str, scope: NewsScope, amount: i64) -> NewsDef {
        news(id, scope, vec![CardEffect::AddCash { amount }])
    }

    #[test]
    fn a_wanted_news_can_be_named_by_id_or_by_slot() {
        // `NM_NEWS` 的两种写法都在这里判，而不是在旋钮那一侧各判一次：
        // 「池子里有没有这一条」是数据说了算的事
        let set = NewsSet(vec![
            cash_news("news_001", NewsScope::Everyone, 100),
            cash_news("news_002", NewsScope::One(PlayerPick::Richest), 100),
        ]);
        assert_eq!(find_id(&set, "news_001"), Some(NewsId(0)));
        assert_eq!(find_id(&set, "news_002"), Some(NewsId(1)));
        assert_eq!(find_id(&set, "0"), Some(NewsId(0)));
        assert_eq!(find_id(&set, "1"), Some(NewsId(1)));
        // 认不出来就是认不出来：退成「随机一条」会让一句打错的旋钮变成一次意外事件
        assert_eq!(find_id(&set, "news_999"), None);
        assert_eq!(find_id(&set, "2"), None, "序号越界不许回绕到第一条");
        assert_eq!(find_id(&set, ""), None);
    }

    #[test]
    fn the_pool_never_repeats_itself_within_a_match() {
        // 「同场不连重」的全部内容：给定上一条、把 [`roll_span`] 给的值域打满，
        // 抽出来的**永远不是**上一条，而其余每一条各命中一次（既不偏心也不漏）
        const POOL: usize = 4;
        for last in 0..POOL as u32 {
            let span = roll_span(POOL, Some(NewsId(last)));
            let seen: Vec<u32> = span
                .map(|roll| draw(POOL, Some(NewsId(last)), roll).unwrap().0)
                .collect();
            assert!(
                !seen.contains(&last),
                "上一条是 {last} 时抽到了自己：{seen:?}"
            );
            let covers: Vec<u32> = (0..POOL as u32).filter(|i| *i != last).collect();
            assert_eq!(
                seen.len(),
                covers.len(),
                "上一条是 {last} 时值域该正好覆盖其余每一条：{seen:?}"
            );
            assert_eq!(
                seen.iter().collect::<std::collections::HashSet<_>>(),
                covers.iter().collect::<std::collections::HashSet<_>>(),
                "上一条是 {last} 时应覆盖其余每一条，而不是偏爱某几格：{seen:?}"
            );
        }
    }

    #[test]
    fn the_first_news_of_a_match_can_be_any_of_them() {
        // 这一局第一次播没有「上一条」要躲，值域因此给满。少给一格的代价是
        // **池子里最后一条永远抽不到** —— 那不是「不连重」，那是一条被删掉的新闻
        const POOL: usize = 4;
        let span = roll_span(POOL, None);
        let seen: Vec<u32> = span.map(|roll| draw(POOL, None, roll).unwrap().0).collect();
        assert_eq!(seen, vec![0, 1, 2, 3], "开局那一条该覆盖整副池");
    }

    #[test]
    fn a_single_item_pool_has_nothing_to_avoid() {
        // 池子只剩一条时「不连重」在物理上做不到 —— 那一格必须是「就播它」而不是越界
        assert_eq!(roll_span(1, Some(NewsId(0))), 0..=0);
        assert_eq!(draw(1, Some(NewsId(0)), 0), Some(NewsId(0)));
        assert_eq!(draw(1, None, 0), Some(NewsId(0)));
    }

    #[test]
    fn an_empty_pool_is_not_drawn_at() {
        assert_eq!(draw(0, None, 0), None);
    }

    #[test]
    fn two_item_pools_alternate() {
        // 只有两条时「不连重」等于「必须换一条」—— 这条用例是那条拍板最容易被写歪的形状
        assert_eq!(roll_span(2, Some(NewsId(0))), 0..=0);
        assert_eq!(draw(2, Some(NewsId(0)), 0), Some(NewsId(1)));
        assert_eq!(draw(2, Some(NewsId(1)), 0), Some(NewsId(0)));
    }

    #[test]
    fn a_rolled_index_past_the_domain_wraps_instead_of_panicking() {
        // 值域写错的那一种未来：越界不该 panic，也不该指向一条不存在的新闻
        assert_eq!(draw(3, None, 99).map(|n| n.0 < 3), Some(true));
        assert_eq!(draw(3, Some(NewsId(0)), 99).map(|n| n.0 < 3), Some(true));
    }
}
