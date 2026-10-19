//! 卡牌定义：机会 / 命运牌池。效果用内部标签枚举，新增 action 时编译器会强制补分支。
//!
//! 对应 Godot 端 `card_data.gd` + `card_effects.gd`（后者是 `Dictionary` 弱类型 match，
//! 且 `deduct_cash` 失败时只回文案不真的判破产 —— 见 `.agents/ARCHITECTURE.md` §三 第 3 条）。

use bevy::asset::Asset;
use bevy::reflect::TypePath;
use serde::Deserialize;

/// 三副池。前两副是**踩到格子就抽、当场结算**的事件池（PLAN §三「命运 32 条」），
/// 第三副 `Item` 是**拿在手里、由玩家主动打出**的卡 —— 它没有对应的格子，
/// 来源是百货公司（P3d），所以 `domain_of` 里永远不会出现 `Card(Item)`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Chance,
    Fate,
    Item,
}

/// 这张卡要落在谁/什么头上。写在数据里而不是代码里，是因为出牌浮层的按钮要把目标
/// 编码进去（同 ATM 金额按钮的形状：一颗按钮 = 一个完整意图），而浮层不该去猜
/// 「这张卡是给自己用还是给对手用」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardTarget {
    /// 打在自己身上（免罪、停留、转向）
    #[default]
    SelfOnly,
    /// 选一名对手（陷害、嫁祸）
    Player,
    /// 选一个人，**含自己**（转向、停留，P3d④b/④c）。
    ///
    /// 为什么不并进 [`Self::Player`]：「不能陷害自己」是陷害卡的规则（自己送自己进监狱
    /// 没有意义，而那条判据写在 `aim_is_legal` 里），而掉头与停在原地这两件事
    /// **正是**要能对自己做 —— 4 代的这两张卡点谁都可以（`GLOSSARY-board.md` 那句
    /// 「反转绕行方向 / 给他人转向」）。并成一格的话，下一张「可以打给自己」的卡
    /// 就会去读 `effects` 才知道自己能不能被选。
    ///
    /// P13 魔法屋那 12 条法术按筛选条件点人（走路的人／存款最多／所有女生…），
    /// 挑的也是这一格：那件事的主语永远是「一个人」，而不是「一个对手」。
    Anybody,
    /// 选一块**有主**的地（涨价、查封、怪兽）
    Land,
    /// 选一块**无主**的地（购地卡，P3d④）。
    ///
    /// 为什么不并进 [`Self::Land`]：这两句话的承受者不是同一个人。`Land` 的承受者是那块地的
    /// **业主**（涨价是往别人钱袋上加钱），而无主地根本没有业主可承受 —— 购地卡的承受者就是
    /// 出牌人自己。并成一格的话，[`crate::hand::victim_of`] 与
    /// [`crate::hand::aim_is_legal`] 就得反过来读 `effects` 才知道该问谁，
    /// 「这张卡打给谁」这件事从此散在两张表里。
    UnownedLand,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum CardEffect {
    AddCash {
        amount: i64,
    },
    DeductCash {
        amount: i64,
    },
    /// 关进监狱 N 天。**承受者是这张卡的目标**（[`CardTarget`]），不是抽卡的人 ——
    /// 所以命运牌的「直接入狱」（target 缺省 = 自己）与陷害卡的「送对手入狱」
    /// 是同一条效果，不必为陷害单开一个变体。可被 [`crate::afflict::Insulation`] 抵消。
    GoToJail {
        #[serde(default = "default_jail_turns")]
        turns: u32,
    },
    /// 按格名瞬移。格名是数据与棋盘的隐式契约，`validate` 只能查非空，
    /// 真正「存不存在」要等 B3 拿到 BoardDef 才能校验。
    MoveTo {
        tile_name: String,
    },
    /// 挂上「抵消 N 次厄运」的护身（免罪卡）。见 [`crate::afflict`]。
    Insulate {
        charges: u32,
    },
    /// 给一片地块盖上限时状态（涨价卡 / 查封卡，P3c）。
    ///
    /// 三个字段缺一不可：`status` 是盖哪一种，`area` 是盖多大一片（4 代这两张都是「整街」，
    /// 但单块与整街是两个不同的数，写成一个变体就得为后者再开一个变体），`days` 是活几天。
    /// 承受者是**这块地的业主**（[`crate::hand::victim_of`]），不是选它的人 —— 所以
    /// 「查封对手整条街」与「涨价自己的街」共用同一条实现，差别只在数据。
    LandStatus {
        status: crate::status::LandStatus,
        days: u32,
        area: crate::plot::PlotArea,
    },
    /// 挂上一条**待用**的免付（免费卡，P3c②）。`floor` 是门槛的名义额：过完物价的实付
    /// 金额要**超过** `floor × 物价` 才免得掉，而且只免一次。
    ///
    /// 为什么不当场折现：这张卡的全部玩法在「留着等那笔大的」。打出去就结算的话，
    /// 它只是一张按市价换现金的券，玩家永远不必判断。
    ///
    /// 它抵的是**钱**，所以走 `Corrections` 那条管线；免罪卡抵的是**事**，走
    /// [`crate::afflict`]。两张表 P13 的免疫矩阵要各读一遍（PLAN §三「反制链」行）。
    FreePass {
        floor: i64,
    },
    /// 当场买下一块无主地（购地卡，P3d④）。**卡面上不带地块编号**：选哪块地是玩家按下去
    /// 那一刻定的，来自出牌请求的 `aim`（与 [`Self::LandStatus`] 同一条理由）。
    ///
    /// 价格不在卡面上，也不必在卡面上：走的是 `Books::buy` 那条正路，所以地价过一遍
    /// P2 管线 —— 物价指数照乘、小福神「买地免费」那一类 `Cancel` 照免。这张卡买的不是折扣，
    /// 买的是**不用走到那一格**这件事。
    BuyLand,
    /// 让承受者的绕行方向掉头（转向卡，P3d④b）。**没有字段**：翻的是玩家身上那一位
    /// （[`crate::player::Heading`]），而「翻成什么」不是卡面能说的 —— 翻两次就是原样，
    /// 所以这里既没有 `to` 也没有层数。
    ///
    /// 环还是那一条环，只是步进取 `+1` 还是 `-1`（ADR-001 行为规则 3 + `GLOSSARY-board.md`
    /// 「仍沿同一环，只是反向」）。它抵消得了（[`crate::afflict::AfflictDomain::Reversal`]
    /// 走免罪那条护身，因为这是一件**事**不是一笔钱），也因此与 `LandStatus`/`BuyLand` 不同：
    /// 没有 aim 它照样成立（打给自己），所以 `validate` 不给它绑 target。
    Reverse,
    /// 让承受者原地停留 N 回合（停留卡，P3d④c）。**人还在场上**：棋子不动、不改方向、
    /// 名下的地照样收租，只是那几回合的轮到他也被跳过（见 [`crate::player::StaySentence`]）。
    ///
    /// 与 [`Self::GoToJail`] 是两条效果而不是同一条带 `place` 参数的：入狱要把人送进
    /// 死胡同格并（P4 起）夺走收租权，这两件事停留都不做。它们共享的只是「回合开始扣一格」
    /// 那半条规则，而那半条写在 `begin_turn_system` 里，不该由卡面来合并。
    Stay {
        turns: u32,
    },
}

const fn default_jail_turns() -> u32 {
    3
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct CardDef {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: CardKind,
    pub title: String,
    pub desc: String,
    /// 百货公司的售价（点券）。`None` = 这张卡不出售（两副事件池全是 `None`，
    /// 它们是被抽到的，不是被买的）。点券与现金永不通兑（PLAN §一），所以这条
    /// 只可能由 `Points` 付。
    #[serde(default)]
    pub cost_points: Option<u32>,
    /// 打出去要选谁/选哪块地。缺省 `SelfOnly`。
    #[serde(default)]
    pub target: CardTarget,
    #[serde(default)]
    pub effects: Vec<CardEffect>,
}

#[derive(Debug, Clone, PartialEq, Asset, TypePath, Deserialize)]
#[serde(transparent)]
pub struct CardSet(pub Vec<CardDef>);

impl CardSet {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    /// `expect_kind` 让「把机会卡写进命运文件」这类数据错误在加载期就暴露。
    pub fn validate(&self, expect_kind: CardKind) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();

        if self.0.is_empty() {
            return Err(vec!["牌池为空".into()]);
        }

        let mut seen = std::collections::HashSet::new();
        for c in &self.0 {
            if c.id.is_empty() {
                errs.push("有卡牌 id 为空".into());
            } else if !seen.insert(c.id.as_str()) {
                errs.push(format!("卡牌 id 重复: {}", c.id));
            }
            if c.kind != expect_kind {
                errs.push(format!(
                    "卡牌 {} 的 type={:?} 与牌池 {:?} 不符",
                    c.id, c.kind, expect_kind
                ));
            }
            if c.title.is_empty() {
                errs.push(format!("卡牌 {} 缺 title", c.id));
            }
            if c.desc.is_empty() {
                errs.push(format!("卡牌 {} 缺 desc", c.id));
            }
            if c.effects.is_empty() {
                errs.push(format!("卡牌 {} 没有任何效果", c.id));
            }
            // 货架上的东西必须有价：没有价的 `Item` 卡要么白送要么永远买不到，
            // 两种都是把数据错误藏进「点数账对不上」里（P3d 的百货正靠这一列定价）
            if expect_kind == CardKind::Item && c.cost_points.unwrap_or(0) == 0 {
                errs.push(format!("卡牌 {} 是货架商品却没有 cost_points", c.id));
            }
            for e in &c.effects {
                match e {
                    CardEffect::AddCash { amount } | CardEffect::DeductCash { amount } => {
                        if *amount <= 0 {
                            errs.push(format!("卡牌 {} 的现金效果数值非法: {amount}", c.id));
                        }
                    }
                    CardEffect::GoToJail { turns } => {
                        if *turns == 0 {
                            errs.push(format!("卡牌 {} 的关押回合数为 0", c.id));
                        }
                    }
                    CardEffect::MoveTo { tile_name } => {
                        if tile_name.is_empty() {
                            errs.push(format!("卡牌 {} 的 move_to 缺 tile_name", c.id));
                        }
                    }
                    CardEffect::Insulate { charges } => {
                        if *charges == 0 {
                            errs.push(format!("卡牌 {} 的抵消次数是 0（打它做什么）", c.id));
                        }
                    }
                    CardEffect::LandStatus { days, .. } => {
                        // 0 天的状态会在面板上挂满一整天再消失（「剩 0 天」），而它的本意是
                        // 「什么也没发生」。数据里写 0 就是写错了，加载期拦下
                        if *days == 0 {
                            errs.push(format!("卡牌 {} 的地块状态天数是 0", c.id));
                        }
                        // 这种效果必须靠目标才知道盖哪儿：锚点来自出牌请求，不在卡面上。
                        // 写成 `target: self_only` 的机会牌会一路走到 `apply_card` 才落空，
                        // 那时候它已经是一张「买了没用的卡」了 —— 在加载期就拦
                        if c.target != CardTarget::Land {
                            errs.push(format!("卡牌 {} 要盖地块状态，但 target 不是 land", c.id));
                        }
                    }
                    CardEffect::FreePass { floor } => {
                        // 门槛写成 0（或负）的免费卡是「以后所有账一律免」，那不该是
                        // 一张卖几十点券的东西 —— 它的玩法形状全在这个数上
                        if *floor <= 0 {
                            errs.push(format!(
                                "卡牌 {} 的免付门槛是 {floor}，没有门槛的免付不是这张卡",
                                c.id
                            ));
                        }
                    }
                    CardEffect::BuyLand => {
                        // 与 `LandStatus` 同一条绑定：这块地来自出牌请求，不在卡面上。
                        // 目标写成 `land` 的话，购地卡会去问一块**有主**的地（那条判据要业主），
                        // 而它要买的恰恰是无主的 —— 两个判据互相矛盾，抽到/打出都只会静默失败
                        if c.target != CardTarget::UnownedLand {
                            errs.push(format!("卡牌 {} 要买地，但 target 不是 unowned_land", c.id));
                        }
                    }
                    CardEffect::Reverse => {
                        // 这一格**没有**要校验的数，而它必须写在这里：`Reverse` 翻的是玩家身上
                        // 那一位，没有字段、也没有「没有 aim 就静默失效」那条毛病（打给自己
                        // 照样成立），所以它不该被绑到任何 target 上。哪天它长出参数来
                        // （比如「转向 N 次」），编译会停在这一格
                    }
                    CardEffect::Stay { turns } => {
                        // 与 `GoToJail` 同一条：0 回合的停留是「什么都没发生」，而卡已经被打掉
                        if *turns == 0 {
                            errs.push(format!("卡牌 {} 的停留回合数为 0", c.id));
                        }
                    }
                }
            }
        }

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHANCE: &str = include_str!("../../../assets/data/cards/chance_cards.json");
    const FATE: &str = include_str!("../../../assets/data/cards/fate_cards.json");
    const ITEMS: &str = include_str!("../../../assets/data/cards/item_cards.json");

    #[test]
    fn both_pools_parse_and_validate() {
        let chance = CardSet::from_json(CHANCE).expect("机会牌应能解析");
        let fate = CardSet::from_json(FATE).expect("命运牌应能解析");
        assert!(chance.validate(CardKind::Chance).is_ok());
        assert!(fate.validate(CardKind::Fate).is_ok());
        assert!(chance.0.len() >= 5, "机会牌数偏少: {}", chance.0.len());
        assert!(fate.0.len() >= 5, "命运牌数偏少: {}", fate.0.len());
    }

    /// 第三副池：拿在手里的卡。它有自己的不变式（必须有价），所以不能只靠上面那条
    /// 顺带过一遍 —— 忘了给 `Item` 定价的卡会安静地变成「货架上买不到的商品」。
    #[test]
    fn the_item_pool_validates_and_carries_prices() {
        let items = CardSet::from_json(ITEMS).expect("道具牌应能解析");
        assert!(items.validate(CardKind::Item).is_ok());
        assert!(!items.0.is_empty(), "第一批可持有卡该有货");
        for c in &items.0 {
            assert!(c.cost_points.is_some(), "{} 没有点券售价", c.id);
        }
        // 混进事件池的 kind 由同一条 `validate` 拦住
        assert!(items.validate(CardKind::Fate).is_err());
        let mut free = items.clone();
        free.0[0].cost_points = None;
        assert!(
            free.validate(CardKind::Item).is_err(),
            "货架商品没有价应该报错"
        );
    }

    /// 陷害卡与命运入狱共用同一条效果，区别只在 `target`。这条钉住那个形状：
    /// 将来谁给「陷害」单开一个 `CardEffect` 变体，两边就会开始各修一半。
    #[test]
    fn framing_is_the_jail_effect_aimed_at_somebody_else() {
        let items = CardSet::from_json(ITEMS).unwrap();
        let frame = items
            .0
            .iter()
            .find(|c| c.target == CardTarget::Player)
            .expect("第一批卡里该有指定对手的卡");
        assert!(
            frame
                .effects
                .iter()
                .any(|e| matches!(e, CardEffect::GoToJail { turns } if *turns > 0)),
            "{} 的效果不是关人",
            frame.id
        );
        let insulate = items
            .0
            .iter()
            .find(|c| c.target == CardTarget::SelfOnly)
            .expect("免罪卡是打给自己的");
        assert!(matches!(
            insulate.effects[0],
            CardEffect::Insulate { charges: 1 }
        ));
    }

    #[test]
    fn every_effect_kind_is_exercised_by_the_data() {
        let all = CardSet::from_json(CHANCE)
            .unwrap()
            .0
            .into_iter()
            .chain(CardSet::from_json(FATE).unwrap().0)
            .chain(CardSet::from_json(ITEMS).unwrap().0)
            .flat_map(|c| c.effects)
            .collect::<Vec<_>>();

        let has = |pred: fn(&CardEffect) -> bool| all.iter().any(pred);
        assert!(has(|e| matches!(e, CardEffect::AddCash { .. })));
        assert!(has(|e| matches!(e, CardEffect::DeductCash { .. })));
        assert!(has(|e| matches!(e, CardEffect::GoToJail { turns: 3 })));
        assert!(has(
            |e| matches!(e, CardEffect::MoveTo { tile_name } if tile_name == "起点")
        ));
        assert!(has(|e| matches!(e, CardEffect::Insulate { .. })));
        assert!(has(|e| matches!(e, CardEffect::LandStatus { .. }),));
        assert!(has(|e| matches!(e, CardEffect::FreePass { .. })));
        assert!(has(|e| matches!(e, CardEffect::BuyLand)));
        assert!(has(|e| matches!(e, CardEffect::Reverse)));
        assert!(has(|e| matches!(e, CardEffect::Stay { .. })));
    }

    /// 「卡上的效果」与「卡要落在什么头上」必须成对，否则数据能过加载、卡却永远打不出去。
    /// 两条绑定各测一个反例：写歪的那一半会在 `validate` 里响，而不是等到玩家按下去。
    #[test]
    fn an_effect_that_needs_a_target_is_bound_to_that_target() {
        let items = CardSet::from_json(ITEMS).unwrap();

        let mut hike = items.clone();
        hike.0
            .iter_mut()
            .find(|c| {
                c.effects
                    .iter()
                    .any(|e| matches!(e, CardEffect::LandStatus { .. }))
            })
            .expect("货架上该有盖状态的卡")
            .target = CardTarget::SelfOnly;
        assert!(
            hike.validate(CardKind::Item).is_err(),
            "盖地块状态的卡没了 land 目标就该被拦下"
        );

        let mut grab = items;
        grab.0
            .iter_mut()
            .find(|c| c.effects.contains(&CardEffect::BuyLand))
            .expect("货架上该有购地卡")
            .target = CardTarget::Land;
        assert!(
            grab.validate(CardKind::Item).is_err(),
            "买无主地的卡指着有主的地，两条判据当场矛盾"
        );

        // 反例的反例：转向卡**不该**被绑住。它没有 aim 也成立（打给自己掉头），
        // 所以把它写成 `self_only` 照样是一张合法的卡 —— 哪天有人给 `Reverse` 加一条
        // target 检查，这条会先红，而那正是「为了整齐把一条真规则写窄了」的形状
        let mut turn = CardSet::from_json(ITEMS).unwrap();
        turn.0
            .iter_mut()
            .find(|c| c.effects.contains(&CardEffect::Reverse))
            .expect("货架上该有转向卡")
            .target = CardTarget::SelfOnly;
        assert!(
            turn.validate(CardKind::Item).is_ok(),
            "转向卡被 target 绑住了：{:?}",
            turn.validate(CardKind::Item).unwrap_err()
        );
    }

    #[test]
    fn card_ids_are_unique_within_pool() {
        let mut set = CardSet::from_json(CHANCE).unwrap();
        let first = set.0[0].clone();
        set.0.push(first);
        assert!(set.validate(CardKind::Chance).is_err());
    }

    #[test]
    fn wrong_kind_is_rejected() {
        let mut set = CardSet::from_json(CHANCE).unwrap();
        set.0[0].kind = CardKind::Fate;
        let errs = set.validate(CardKind::Chance).unwrap_err();
        assert!(errs.iter().any(|e| e.contains("不符")), "{errs:?}");
    }

    #[test]
    fn empty_effects_and_bad_amounts_are_rejected() {
        let mut set = CardSet::from_json(FATE).unwrap();
        set.0[0].effects.clear();
        assert!(set.validate(CardKind::Fate).is_err());

        let mut set = CardSet::from_json(FATE).unwrap();
        set.0[0].effects = vec![CardEffect::DeductCash { amount: 0 }];
        assert!(set.validate(CardKind::Fate).is_err());
    }
}
