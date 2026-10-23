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
use crate::msg::{NewsFired, NewsHit, NewsNotice};

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
                    CardEffect::Stay { turns } | CardEffect::MoveCap { turns } => {
                        if *turns == 0 {
                            errs.push(format!("新闻 {} 的停走回合数是 0", n.id));
                        }
                    }
                    // 瞬移要靠格名，而新闻没有「出牌人指着哪块地」那一步 —— 它的锚点只能
                    // 由作用域算出来。留这一条在这里，是为了「哪天给新闻加地块类效果」
                    // 必须先在 `NewsScope` 那一头表态，而不是在运行时静默落空。
                    CardEffect::MoveTo { .. } | CardEffect::LandStatus { .. } => {
                        errs.push(format!(
                            "新闻 {} 用了 {:?}：它需要作用域之外的锚点，\
                             而新闻池里写不出这一格（要它就先扩 `NewsScope`）",
                            n.id, e
                        ));
                    }
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

/// 这一条新闻点名到的那几个人（**已经过滤掉破产者**）。
///
/// 返回的是「承受者」而不是「谁触发」：踩格的人由调用方带在凭据上（`NewsFired::broadcaster`），
/// 两处分开才是这条玩法的全部新意 —— 合成一个人的话，「他开广播又挨了打」与
/// 「他挨了打」在流水里读起来是同一句话。
///
/// `audience` 要 `&mut CardSink` 而不是「账本 + 随机流」两格：随机流挂在
/// [`crate::gods::BestowKit`] 里（那条流是 `Rng` 的第十五条，见 [`crate::rng`]），
/// 而账本与它在同一个参数包上，分两头借会在参数建立期撞。
fn audience(scope: NewsScope, actor: Entity, sink: &mut CardSink) -> Vec<Entity> {
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
            }
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
    let targets = audience(scope, actor, &mut sink.card);
    let mut hits = Vec::with_capacity(targets.len());
    for target in targets {
        hits.push(NewsHit {
            player: target,
            outcomes: apply_effects(&mut sink.card, actor, target, &effects, &news_id, None),
        });
    }
    info!(
        "新闻「{title}」（{news_id}）：由 {actor:?} 播出，点到 {} 个人",
        hits.len()
    );
    sink.fired.write(NewsFired {
        broadcaster: actor,
        news: NewsNotice {
            id: news_id,
            title,
            desc,
            scope,
            hits,
        },
    });
    true
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn a_land_anchored_effect_needs_a_scope_that_cannot_give_one() {
        // 「瞬移到某格」这类效果要的是**卡面上写死的锚点**，而新闻池的锚点只能由作用域算。
        // 这一条挡的是「先写了效果、忘了扩 NewsScope」那种半截数据
        let set = NewsSet(vec![NewsDef {
            id: "news_move".into(),
            title: "有人搬家".into(),
            desc: "把一位玩家送到南京路".into(),
            scope: NewsScope::Everyone,
            effects: vec![CardEffect::MoveTo {
                tile_name: "南京路".into(),
            }],
        }]);
        let errs = set.validate().unwrap_err();
        assert!(errs.iter().any(|e| e.contains("需要作用域之外的锚点")));
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
