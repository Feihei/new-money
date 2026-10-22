//! 小游戏格（P7）：三种游戏共用一个格类型、一条接口、一张成绩→点券表。
//!
//! **规则层不实现玩法本体**（PLAN §五）：core 只管四件事 —— 这一格开哪一局、把回合挂住、
//! 收成绩、按表换点券。气球飞到哪、篮子接住没接住、挖宝格子翻开来是什么，全在表现层
//! （`game_app/src/ui/minigame.rs`）。
//!
//! 有三样东西仍然必须由规则层发，因为它们要进账：
//!
//! - **玩哪一款**。格子上写了 `data.game` 就是那一款；没写则由 [`Rng::pick_minigame`] 抽 ——
//!   演示盘那一格是「随机一款」，抽法必须走自己的子流，否则同一 `NM_SEED` 的两局会在这一格分岔。
//! - **这一局的布局种子**（[`ArcadeDeal`]）。挖宝的 16 格里哪 6 格埋着宝决定了得分，得分换
//!   点券、点券是账 —— 所以那份随机也从这里发。表现层拿它自己展开布局，但**不碰规则层的
//!   随机流**（与 P2b 老虎机同一条分工：表现层那些滚动的数字不算账，算账的那一份由 core 抽好）。
//! - **成绩换点券的那张表**（[`Payouts`]，挂在 [`crate::rules::GameRules`] 上）。三种各配
//!   「满分」与「每分几点券」：这三种游戏的平衡是这个功能唯一真正难的地方，参数必须在
//!   同一张表里能横向比较，而不是散在三个玩法文件里各写一个常数。
//!
//! 点券照旧是**另一套账**：这里只走 [`crate::settle::Books::award_points`]，一条
//! `CashChanged` 都不产生（PLAN §一「永不通兑」），所以 B3 的现金审计看不见它。

use bevy::prelude::*;
use serde::Deserialize;

use crate::landing::{Landed, Landing, LandingDomain, Pending, PendingDecision};
use crate::msg::{MinigameResult, MinigameScore};
use crate::rng::Rng;
use crate::settle::Books;
use crate::turn::TurnPhase;
use crate::world::ActiveBoard;

/// 三种小游戏。格子上写的是 `data.game`（`pop_balloons`／`catch_treasure`／`penguin_dig`），
/// 三种**共用 `TileKind::Arcade`**、靠它分流 —— 不各开一个 `TileKind`，否则格类型会被
/// 美术资产的数量污染（PLAN §五）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GameKind {
    /// 打气球：限时内点掉升起来的气球，一只一分。
    PopBalloons,
    /// 接财宝：限时内左右移动篮子接住掉下来的财宝，接到石头倒扣一分。
    CatchTreasure,
    /// 企鹅挖宝：4×4 的冰洞里有 6 格埋着宝，只有 8 次下铲的机会。
    PenguinDig,
}

impl GameKind {
    /// 穷尽清单。「随机一款」抽签与旋钮解析都读它，别处不再手抄一份三个名字。
    pub const ALL: [Self; 3] = [Self::PopBalloons, Self::CatchTreasure, Self::PenguinDig];

    pub const fn label(self) -> &'static str {
        match self {
            Self::PopBalloons => "打气球",
            Self::CatchTreasure => "接财宝",
            Self::PenguinDig => "企鹅挖宝",
        }
    }

    /// 认中文名也认英文名（`PopBalloons`／`pop_balloons` 都行）。
    ///
    /// 单独成一句是为了能被测到：一个拼错的 `NM_MINIGAME` 不会崩、也不会红，只会「面板没拉起来」，
    /// 而那张截图看起来只是「今天没有游乐场」—— 与 `NM_GOD`、`NM_HAND` 那两条同一种坑。
    pub fn parse(raw: &str) -> Option<Self> {
        let want = raw.trim();
        Self::ALL.into_iter().find(|game| {
            game.label() == want
                || format!("{game:?}").eq_ignore_ascii_case(want)
                || snake_of(game) == want
        })
    }
}

/// `PopBalloons` → `pop_balloons`：serde 的 `rename_all` 只在反序列化那一侧生效，
/// 旋钮要认的是数据里写的那个形状，所以这里自己转一次（三种都是两个词，规则只有一条）。
fn snake_of(game: &GameKind) -> String {
    let name = format!("{game:?}");
    let mut out = String::with_capacity(name.len() + 4);
    for (i, c) in name.chars().enumerate() {
        if c.is_uppercase() && i > 0 {
            out.push('_');
        }
        out.extend(c.to_lowercase());
    }
    out
}

/// 一种游戏的成绩→点券换算：`每分几点券 × min(成绩, 满分)`。
///
/// **满分是夹子而不是终点**：打出超过满分的成绩仍然只拿满分那笔。写成不夹的话，
/// 「一个手快的人能刷出多少点券」就成了一个没有上界的数，而点券要拿去保释恶人（300）、
/// 买乌龟卡（70）—— 那条经济线会被一个限时小游戏定价。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payout {
    /// 满分成绩。超过它的部分不计。
    pub perfect: u32,
    /// 每一点成绩换几点券（斜率）。
    pub per_score: u32,
}

impl Payout {
    /// 这个成绩值多少点券。整数乘，不涉浮点（点券是整数账，与 `shop::buyback` 同一条理由）。
    pub fn points(&self, score: u32) -> u32 {
        self.per_score.saturating_mul(score.min(self.perfect))
    }

    /// 打满能拿到的那一笔。面板上写的是它 —— 玩家在开局前就该知道这一局值多少。
    pub fn best(&self) -> u32 {
        self.per_score.saturating_mul(self.perfect)
    }
}

/// 三种游戏的那张表。一个结构体三个字段而不是 `[Payout; 3]`：
/// 字段名让人在读 `GameRules::default()` 时就知道哪一档是哪种游戏，
/// 而下标会要求读者去数 `GameKind::ALL` 的顺序 —— 那张顺序本来是给抽签用的。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Payouts {
    pub pop_balloons: Payout,
    pub catch_treasure: Payout,
    pub penguin_dig: Payout,
}

impl Default for Payouts {
    /// 三档的**上限都压在 100 点券左右、期望值都在 50 上下**，这是刻意的：
    /// 三种游戏要能横向比较（「今天踩到哪一种」不该是抽到一张大小不同的奖），
    /// 而 100 这个量级对上的是一个点券箱的 50 与一张乌龟卡的 70 —— 玩得漂亮够买一张贵卡，
    /// 玩得一般相当于路上捡两只箱子。
    ///
    /// ⚠️ 满分与玩法参数是**一对**：`perfect` 是 25 而 25 秒里根本点不掉 25 只气球的话，
    /// 那一档就成了永远拿不到的装饰。改这里的数时去 `ui/minigame.rs` 对一眼时长与密度。
    fn default() -> Self {
        Self {
            // 25 秒 / 屏上 8 只：满分 25 只等于「一只不漏」，per_score 4 → 上限 100
            pop_balloons: Payout {
                perfect: 25,
                per_score: 4,
            },
            // 25 秒 / 每 0.7 秒掉一件（八成是财宝）：满分 20 件，per_score 5 → 上限 100
            catch_treasure: Payout {
                perfect: 20,
                per_score: 5,
            },
            // 8 铲 / 16 格 6 宝：满分就是「六铲全中」，per_score 16 → 上限 96
            penguin_dig: Payout {
                perfect: 6,
                per_score: 16,
            },
        }
    }
}

impl Payouts {
    /// 取某一种游戏那一档。**穷尽 `match`、不许 `_`**：加第四种游戏时必须当场给它配一档，
    /// 漏了不会编译失败的话，症状是「新游戏白玩、一分不给」，而那正好是最难查的一类。
    pub fn of(&self, game: GameKind) -> Payout {
        match game {
            GameKind::PopBalloons => self.pop_balloons,
            GameKind::CatchTreasure => self.catch_treasure,
            GameKind::PenguinDig => self.penguin_dig,
        }
    }
}

/// 这一趟游乐场发下来的那一手：布局种子。
///
/// 「玩哪一款」不在这里 —— 那是 [`Pending::Minigame`] 带的，一份事实不写两遍。
/// 这里只有那份**给表现层展开布局用的随机**：它决定挖宝的宝埋在哪几格、气球从哪儿升起来，
/// 于是同一个 `NM_SEED` 走到同一格会摆出同一局。
///
/// 是资源而不是实体（AGENTS.md 硬约束第 4 条「道路格不建实体」）：它的全部状态就是一个整数，
/// 与点券箱的冷却账本同一个处理。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ArcadeDeal {
    pub seed: u64,
}

/// 踩到游乐场：定下这一趟玩哪一款，发一手布局，再把回合挂住。
///
/// 形状与 `shop::settle_shop_landing` 同一条（先备好这一趟的内容、再挂窗口），而**不**与
/// 点券箱同一条：捡箱子没有「要不要」这个决策，游乐场有 —— 玩家可以选择不玩（空手离开，
/// 见 `landing::decision_system` 里 `Pending::Minigame` 那一格），也可以提前收工。
pub fn settle_arcade_landing(
    landing: Res<Landing>,
    active: Res<ActiveBoard>,
    mut rng: ResMut<Rng>,
    mut deal: ResMut<ArcadeDeal>,
    mut pending: ResMut<PendingDecision>,
) {
    let Some(Landed {
        player,
        tile,
        domain: LandingDomain::Arcade,
        ..
    }) = landing.0
    else {
        return;
    };
    let Some(def) = active.def.tile(tile.0 as i64) else {
        return;
    };
    let game = match def.data.game {
        Some(pinned) => pinned,
        // 格子上没写就是「这一趟随机一款」。抽签走小游戏子流：踩几次游乐场
        // 不该改变下一张机会牌是谁（`rng.rs` 分流的全部理由）
        None => {
            let last = (GameKind::ALL.len() - 1) as u32;
            GameKind::ALL[rng.pick_minigame(0..=last) as usize]
        }
    };
    deal.seed = rng.arcade_seed();
    debug!("游乐场 {} 号格：这一趟玩{}", tile.0, game.label());
    pending.offer(player, Pending::Minigame(game));
}

/// 收一份成绩，按表换成点券，然后交还回合。
///
/// 它是 `Pending::Minigame` 这个窗口的**回答**，所以与 `decision_system` 同一条位置
/// （`WaitDecision` 里、链在它之后）：窗口关掉与阶段转移都由规则层做，表现层推不动回合
/// （§4.6 那条红线在这里的另一面）。
///
/// `awarded` 是那一格显式的奖励开关（PLAN §四 P7 的原话：主菜单练习不给点券，
/// **用 `awarded: bool` 表达，不靠「主菜单不发消息」隐式实现**）。为 `false` 时这里
/// 一分不发、也**不碰回合机**（练习局没有窗口可关），但仍然留一条 [`MinigameScore`] 凭据 ——
/// 「玩过、没给钱」与「什么都没发生」在流水上是两件事。
pub fn minigame_score_system(
    mut books: Books,
    mut pending: ResMut<PendingDecision>,
    mut results: MessageReader<MinigameResult>,
    mut scored: MessageWriter<MinigameScore>,
    mut next: ResMut<NextState<TurnPhase>>,
) {
    for r in results.read() {
        let at_window = pending.active() == Some((r.player, Pending::Minigame(r.game)));
        if !at_window && r.awarded {
            // 不在窗口前却要求发钱：与百货柜台同一条纪律（拒绝只 `debug!`，面板负责
            // 让这颗按钮根本按不到）。练习局走的是下面那条「不要钱」的路，不落这里
            debug!(
                "小游戏成绩被拒：{:?} 面前没有「{}」的窗口",
                r.player,
                r.game.label()
            );
            continue;
        }
        let points = if r.awarded {
            books.rules.minigames.of(r.game).points(r.score)
        } else {
            0
        };
        // 0 分不发 `PointsChanged`：与春节红包「0 元不发现金流水」同一条 ——
        // 一条 `+0` 的流水会让「点券审计」里多出一行什么都没说的事
        if points > 0 {
            books.award_points(r.player, points);
        }
        scored.write(MinigameScore {
            player: r.player,
            game: r.game,
            score: r.score,
            points,
            awarded: r.awarded,
        });
        if at_window {
            pending.clear();
            next.set(TurnPhase::TurnEnd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_game_has_a_name_in_both_forms() {
        for game in GameKind::ALL {
            assert!(!game.label().is_empty(), "{game:?} 没有中文名");
            assert_eq!(GameKind::parse(game.label()), Some(game), "中文名认不出");
            let english = format!("{game:?}");
            assert_eq!(GameKind::parse(&english), Some(game), "英文名认不出");
            assert_eq!(
                GameKind::parse(&english.to_lowercase()),
                Some(game),
                "小写英文名认不出"
            );
            assert_eq!(
                GameKind::parse(&snake_of(&game)),
                Some(game),
                "数据里那个 snake_case 形状认不出：{}",
                snake_of(&game)
            );
        }
        assert_eq!(GameKind::parse("打地鼠"), None);
        assert_eq!(GameKind::parse(""), None);
    }

    /// 三种游戏的数据名各不一样。撞车的话 serde 会挑前一个，而症状是「棋盘上写了 A、
    /// 玩起来是 B」—— 与神明那条 `no_two_gods_share_a_label` 同一种防线。
    #[test]
    fn no_two_games_share_a_data_name() {
        let names: Vec<String> = GameKind::ALL.iter().map(snake_of).collect();
        let mut uniq = names.clone();
        uniq.sort();
        uniq.dedup();
        assert_eq!(uniq.len(), GameKind::ALL.len(), "有两种游戏同名：{names:?}");
    }

    /// 斜率与满分：表里每一档都要能横向比较，而「打满」必须真能拿到那一档的上限。
    #[test]
    fn the_table_pays_the_slope_up_to_the_cap() {
        let table = Payouts::default();
        for game in GameKind::ALL {
            let p = table.of(game);
            assert_eq!(p.points(0), 0, "{}：0 分不该给钱", game.label());
            assert_eq!(
                p.points(1),
                p.per_score,
                "{}：一分就是一条斜率",
                game.label()
            );
            assert_eq!(
                p.points(p.perfect),
                p.best(),
                "{}：满分该拿满",
                game.label()
            );
            assert_eq!(
                p.points(p.perfect * 7 + 3),
                p.best(),
                "{}：超过满分的成绩不许把点券刷上去",
                game.label()
            );
        }
    }

    /// 三档的上限要压在同一个量级：这张表存在的理由是「三种游戏可以横向比较」，
    /// 而某一档高出三倍就等于「踩到哪一格」决定了对局平衡。
    #[test]
    fn all_three_games_pay_about_the_same_ceiling() {
        let table = Payouts::default();
        let bests: Vec<u32> = GameKind::ALL.iter().map(|g| table.of(*g).best()).collect();
        let (lo, hi) = (
            bests.iter().copied().min().unwrap(),
            bests.iter().copied().max().unwrap(),
        );
        assert!(
            hi <= lo * 2,
            "三种游戏的上限差了一倍以上：{bests:?}（哪一种被踩到就不该是抽奖）"
        );
        assert!(
            hi >= 50,
            "上限低于一只点券箱，这一格就不值得停下来：{bests:?}"
        );
    }

    /// 那一手布局种子要跟着种子变、也要能复现：同一个 `NM_SEED` 走到同一格摆出同一局，
    /// 换一个种子就不该是同一局（否则「随机一款 + 随机布局」其实一点也没随机）。
    #[test]
    fn the_deal_is_seeded_and_seed_sensitive() {
        let deal = |seed: u64| {
            let mut rng = Rng::new(seed);
            (rng.pick_minigame(0..=2), rng.arcade_seed())
        };
        assert_eq!(deal(11), deal(11), "同种子必须复现同一手");
        assert_ne!(deal(11), deal(12), "换种子还是同一手说明没读种子");
    }

    /// 布局种子不许越界成同一个数：三种游戏都要能被抽到（`ALL` 的宽度是 3，
    /// 抽法读的是 `ALL.len()`，这条钉的是「表加宽了抽法跟着宽」）。
    #[test]
    fn every_game_can_be_dealt() {
        let mut rng = Rng::new(5);
        let mut seen = Vec::new();
        for _ in 0..200 {
            let game =
                GameKind::ALL[rng.pick_minigame(0..=(GameKind::ALL.len() - 1) as u32) as usize];
            if !seen.contains(&game) {
                seen.push(game);
            }
        }
        assert_eq!(
            seen.len(),
            GameKind::ALL.len(),
            "200 次没抽全三种：{seen:?}"
        );
    }
}
