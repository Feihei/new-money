//! 开发用自动玩家（`NM_AUTOROLL=1` 才装配）。
//!
//! 它是**输入侧**的替身，不是渲染层的一部分：只做两件 B5 的按钮同样会做的事 ——
//! 在 `WaitRoll` 发 `RequestRoll`、在 `WaitDecision` 回答 `DecisionChosen`。
//!
//! B5 的按钮上线后它并没有退役，因为两者用途不同：按钮是给人的，这个是用来跑长对局的。
//! 「打完一局」的验收要几百个回合，没有它就只能靠人手动点几百次；B6 的 AI 也会走同一条缝。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use game_core::prelude::{
    Auction, AuctionMove, BidMove, BuyTicketRequested, Decision, DecisionChosen, GameKind,
    GameRules, Jackpot, MinigameResult, Pending, PendingDecision, PlayerId, RequestRoll, TurnOrder,
    TurnPhase, Wallet,
};

pub struct AutopilotPlugin;

/// 两次动作之间留一拍，好让逐格动画看得清。`NM_AUTOGAP=<秒>` 改它：
/// 跑长对局验收时设成 0，一拍 0.9s 会把几百回合拖成十几分钟。
#[derive(Resource)]
struct AutoGap(f32);

impl AutoGap {
    fn from_env() -> Self {
        Self(
            std::env::var("NM_AUTOGAP")
                .ok()
                .and_then(|s| s.trim().parse::<f32>().ok())
                .unwrap_or(0.9),
        )
    }
}

impl Plugin for AutopilotPlugin {
    fn build(&self, app: &mut App) {
        if std::env::var_os("NM_AUTOROLL").is_none() {
            return;
        }
        info!("自动玩家已开启（NM_AUTOROLL）：一律接受报价");
        app.insert_resource(AutoGap::from_env());
        let roll_only = auto_roll.run_if(in_state(TurnPhase::WaitRoll));
        // `NM_AUTODECISION=0` 只掷骰不回答：回合会停在第一个 `WaitDecision`，
        // 于是截图能稳定拍到模态本身（B5 的验收图要的就是这个）
        if std::env::var("NM_AUTODECISION").as_deref() == Ok("0") {
            app.add_systems(Update, roll_only);
        } else {
            app.add_systems(
                Update,
                (
                    roll_only,
                    auto_answer.run_if(in_state(TurnPhase::WaitDecision)),
                ),
            );
        }
    }
}

fn auto_roll(
    time: Res<Time>,
    order: Res<TurnOrder>,
    gap: Res<AutoGap>,
    mut requests: MessageWriter<RequestRoll>,
    mut due: Local<f32>,
) {
    let Some(player) = order.current() else {
        return;
    };
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + gap.0;
    requests.write(RequestRoll { player, dice: 0 });
}

/// 自动玩家对一个待决事项的回答。单开一个类型而不是直接返回 `Decision`：
/// 游乐场那一格（P7）要回的不是决策而是**一份成绩**，彩票柜台那一格（P8）要的也不是
/// 回答而是一笔**买券请求**，三种回答在类型上分开，于是「新增一种窗口」时必须当场决定
/// 自动玩家怎么答它（与 `landing::domain_of` 同一条纪律）。
enum AutoAnswer {
    Decide(Decision),
    Score {
        game: GameKind,
        score: u32,
    },
    /// 买一注。**买哪个号由派发那一侧决定**（见 [`auto_answer`]），因为号码池读的是奖池
    /// 而不是这张表 —— `answer_for` 是个纯函数，它拿不到世界状态。
    Ticket,
    /// 拍卖台上的一个动作（P9）。同 `Ticket` 一条理由：**喊到哪一档、放不放手要读世界**
    /// （当前最高价与这位的现金），而 `answer_for` 只有规则表。
    ///
    /// 这一格存在的最低代价是「让长跑转得动」：答成 `Acknowledge` 的话窗口不会被清掉
    /// （`decision_system` 对拍卖三种回答一律不认），于是**第一个破产就把整局停住**。
    Bid,
}

/// 报价类答「接受」，收据/卡面/三种柜台答「已阅」。这一分工不是讲究：柜台只认
/// `Acknowledge`（`landing.rs` 的 match），一律发 `Accept` 的写法会让长跑的自动玩家
/// 第一个踩到银行格（或百货格）的回合就永久停在 `WaitDecision` —— 而且不报错，只是不动了。
///
/// 游乐场报**满分的一半**而不是 0：报 0 的话长跑里「成绩换点券」这条路径一次都没被走过，
/// 而它正是 P7 的验收项之一。
///
/// 彩票柜台答「买一注」而不是「离开」：不买的话 `NM_AUTOROLL` 那种「跑完一局」的手段里
/// 奖池永远是 0，而 P8 验收那句「每笔进池的钱都能从流水追溯」就只在单测里成立过。
/// 买不起时规则层自己会拒（那张拒绝表与面板的灰按钮同源），这里不必先问现金。
///
/// 拍卖台答 `Bid` 而不是任何一种 `Decision`（P9）：那三种它一个都不认，答成任何一条都只是
/// 让长跑停在第一个破产那儿 —— 而那正是 P9 这一阶段要拍的东西。
fn answer_for(about: Pending, rules: &GameRules) -> AutoAnswer {
    match about {
        Pending::Buy(_) | Pending::Upgrade(_) => AutoAnswer::Decide(Decision::Accept),
        Pending::Minigame(game) => AutoAnswer::Score {
            game,
            score: rules.minigames.of(game).perfect / 2,
        },
        Pending::Lottery => AutoAnswer::Ticket,
        Pending::Auction => AutoAnswer::Bid,
        Pending::RentNotice(_)
        | Pending::Card
        | Pending::BankTeller
        | Pending::Shop
        | Pending::CompanyNotice
        | Pending::Bail(_) => AutoAnswer::Decide(Decision::Acknowledge),
        // 建设窗口（P10c）：自动玩家答「先不建」。**不是「还没实现」** —— 建哪一类是
        // 一个要读世界状态的意图（棋盘默认表配了哪几类、这块地的价、灰按钮的理由都在
        // `Books::build_probe`），而 `answer_for` 是纯函数，拿不到账本；要让它真建，
        // 得先给它一个 `AutoAnswer::Build` 那一格（P10d 公司企业落地时连董事长一起考虑）。
        // 今天这条的代价只有一条：长跑里商业地保持空地 —— 而演示盘的外滩是**预建**的
        // 购物中心，所以 A 型收费那条路在 `NM_AUTOROLL` 里照样走得到。
        Pending::Build(_) => AutoAnswer::Decide(Decision::Acknowledge),
        // 服务费收据（P10c）：钱已经扣完，与租金收据同一个回答
        Pending::ServiceNotice(_) => AutoAnswer::Decide(Decision::Acknowledge),
    }
}

/// 自动玩家的「一只手」：它读得着的世界状态与写得出去的那几种消息。打包成参数包是房规
/// （§六 坑 8）—— P8 加上买券、P9 加上拍卖之后这条 system 要写四种消息、读五样状态，
/// 散装就是十一个参数，而 `#[allow(clippy::too_many_arguments)]` 会把
/// 「一个 system 要动这么多」这个信号吃掉。
#[derive(SystemParam)]
struct AutoHand<'w, 's> {
    pending: Res<'w, PendingDecision>,
    rules: Res<'w, GameRules>,
    /// 上期开出的号。自动玩家就买**那个号** —— 「跟着上期买」是真玩家会做的事，
    /// 而这个选择不需要任何世界状态之外的东西
    jackpot: Res<'w, Jackpot>,
    /// 拍卖机（P9）。读的是**资源**而不是 `Books`：`Books` 里已经揣着一条 `ResMut<Auction>`，
    /// 而同一条 system 参数里两条对同一资源的可变借用会在参数建立期直接撞
    auction: Res<'w, Auction>,
    /// 各人的现金。自动玩家要问「这一档加不加得起」，而不必把整个账本借进来
    wallets: Query<'w, 's, &'static Wallet, With<PlayerId>>,
    answers: MessageWriter<'w, DecisionChosen>,
    scores: MessageWriter<'w, MinigameResult>,
    tickets: MessageWriter<'w, BuyTicketRequested>,
    bids: MessageWriter<'w, AuctionMove>,
}

fn auto_answer(
    time: Res<Time>,
    gap: Res<AutoGap>,
    mut hand: AutoHand<'_, '_>,
    mut due: Local<f32>,
) {
    let Some((player, about)) = hand.pending.active() else {
        return;
    };
    if time.elapsed_secs() < *due {
        return;
    }
    *due = time.elapsed_secs() + gap.0;
    // 游乐场那一格（P7）同样会卡死长跑：它只认 `MinigameResult`，`Acknowledge` 在那里的
    // 意思是「空手离开」。`awarded: true`：自动玩家跑的是真对局，不是练习。
    match answer_for(about, &hand.rules) {
        AutoAnswer::Decide(action) => {
            hand.answers.write(DecisionChosen { player, action });
        }
        AutoAnswer::Score { game, score } => {
            hand.scores.write(MinigameResult {
                player,
                game,
                score,
                awarded: true,
            });
        }
        AutoAnswer::Ticket => {
            let numbers = hand.rules.lottery_numbers;
            // 夹进号码池：上期那个号在「改过号码池的设置」之后可能已经不存在了，
            // 而发一条注定被拒的请求不如发一条合法的吗？不 —— 这里夹它只是因为
            // 自动玩家不该在每一期都撞一次拒绝表（那会把长跑的日志刷满）
            let number = hand
                .jackpot
                .last_draw
                .unwrap_or(1)
                .clamp(1, numbers.max(1) as u8);
            hand.tickets.write(BuyTicketRequested { player, number });
            // 买完还得有人关窗口：柜台不会因为一笔成交自己收摊（那是它与报价的区别）
            hand.answers.write(DecisionChosen {
                player,
                action: Decision::Acknowledge,
            });
        }
        AutoAnswer::Bid => {
            // **主语取机器而不是取窗口**：轮转指针与窗口在绝大多数帧上是同一个人，但它们由两条
            // system 写（`auction_move_system` 改机器、`auction_hold_system` 挂窗口），
            // 而这两条与本 system 之间没有排过序 —— 读到「新机器 + 旧窗口」那一帧是可能的。
            // 不一致时这一帧什么都不发，下一帧自然一致：发一条注定被「还没轮到你」拒掉的
            // 请求，除了把日志刷满什么也不换。
            let Some(lot) = hand.auction.lot.as_ref() else {
                return;
            };
            if lot.bidder() != Some(player) {
                return;
            }
            let needed = lot.next_amount();
            let cash = hand
                .wallets
                .get(player)
                .map(|wallet| wallet.cash)
                .unwrap_or_default();
            // 加得起就一直加：这样长跑里**一定**会有拍卖成交，而不是每一场都因为「没人愿意喊」
            // 流标。反过来一律放弃也能让机器转完，但那一局里 P9 的过户与所得两条路径
            // 一次都没被走过，而那些正是截图与流水要念的东西。
            let what = if cash >= needed {
                BidMove::Raise
            } else {
                BidMove::Withdraw
            };
            hand.bids.write(AuctionMove { player, what });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AutoAnswer, answer_for};
    use game_core::prelude::{Decision, GameKind, GameRules, LandId, Pending};

    /// 每一种窗口都要有自动玩家的答案，而游乐场那一份必须是**带分的**：
    /// 答成 `Acknowledge` 的话长跑会在第一个游乐场格永久停住，且不报错、只是不动了。
    #[test]
    fn every_window_has_an_answer_and_the_arcade_one_scores() {
        let rules = GameRules::default();
        for game in GameKind::ALL {
            match answer_for(Pending::Minigame(game), &rules) {
                AutoAnswer::Score { game: g, score } => {
                    assert_eq!(g, game);
                    assert_eq!(
                        score,
                        rules.minigames.of(game).perfect / 2,
                        "{} 的自动成绩不是满分的一半",
                        game.label()
                    );
                    assert!(score > 0, "报 0 分等于长跑里从没走过换点券那条路");
                }
                AutoAnswer::Decide(action) => {
                    panic!("游乐场被答成了 {action:?} —— 长跑会停在那一格")
                }
                AutoAnswer::Ticket => panic!("游乐场不该被当成买券"),
                AutoAnswer::Bid => panic!("游乐场不该被当成出价"),
            }
        }
        assert!(matches!(
            answer_for(Pending::Buy(LandId(0)), &rules),
            AutoAnswer::Decide(Decision::Accept)
        ));
        assert!(matches!(
            answer_for(Pending::Shop, &rules),
            AutoAnswer::Decide(Decision::Acknowledge)
        ));
        // 彩票柜台答成 `Acknowledge` 也能过关口，但那样一来 `NM_AUTOROLL` 那种长跑里
        // 奖池永远是 0 —— 「每笔进池的钱都能从流水追溯」那条验收就只在单测里成立过
        assert!(matches!(
            answer_for(Pending::Lottery, &rules),
            AutoAnswer::Ticket
        ));
        // 拍卖台答成任何一种 `Decision` 都是**整局停住**：那三种回答 `decision_system` 对拍卖
        // 一个都不认（窗口不清、回合不交还），于是第一个破产就把长跑钉死在那一格 ——
        // 而且不报错，只是不动了。这一条与上面游乐场那条是同一个坑的第二次登记。
        assert!(matches!(
            answer_for(Pending::Auction, &rules),
            AutoAnswer::Bid
        ));
    }
}
