//! 拍卖（P9）：轮转式英式增价。破产变卖与拍卖卡共用这一台机器。
//!
//! 三条决定 shaping 这个文件：
//!
//! - **轮转而不是同桌并发**（10-22 拍板）。PLAN §四 那句「要同时挂多个 bidder」在这里落成的
//!   形状是：拍卖是一个**资源**（[`Auction`]），而窗口每次只挂「此刻该出价的那一位」。
//!   为什么不把 `PendingDecision` 改成一张多人表：真并发会让「谁在什么时候喊的价」变成
//!   帧相关，同种子复现要连帧数一起钉住，而 P15 存档要存的是「这一局怎么走到这里的」。
//!   轮转把同一件事变成**座位序上的一次遍历**：可复现、可测（「三人各加一次」是写得出的一串），
//!   而人类与 AI 发的仍是同一条意图（[`AuctionMove`]，与 P4④ 那颗「停留」按钮同形）。
//! - **起拍＝这块地此刻值多少**（10-22 拍板「起拍=买下它本来要付的钱」）。基数是
//!   [`crate::economy::estate_value`]（地价 + 已盖的每一层），不是 `land_price` —— 那句考据是
//!   「建筑随土地一起过户」，按空地价起拍等于让天使白给的那几层楼被人花几块钱拍走。
//!   物价指数照乘，而**任何人**的修正都不参与（`Charge` 的 payer 填 `None`）：小福神那句
//!   「买地免费」说的是买地那笔账单，而这里把地卖出去的是对手，不是系统。
//!   同一条式子也是 [`Books::buy_price`] 的基数 —— 「起拍=买下它本来要付的钱」在代码里
//!   就是两处读同一个函数。破产那一头因此不需要任何额外照顾：
//!   [`Books::bankrupt`] 只摘归属与抵押、**楼留在地上**（10-22 实报），所以挂拍要读的那几层
//!   一直好好站在世界上，起拍价读的就是它，成交时也不必「还」什么。
//!   第一场实现不是这样：那里先清零、再由队列把「被摘之前」那个数带进来、成交那一刻还回去，
//!   为的是堵「流标的地上凭空多一层楼」—— 而那个毛病真正的答案是**根本别摘**
//!   （拍不走的地与其余归无主的地同一种下场：无主、楼还在，下一位走到那一格连楼一起付）。
//! - **成交不过结算管线**：价由竞价产生，不是账单。它一旦进管线，物价会把别人喊出的数
//!   再乘一遍，而「我出 12000」这句话的意思就是 12000。
//!
//! 谁能出价：座位序里那些**没破产、不在拘留处、不是开槌的人**（考据：拍卖卡使用者本人不能
//! 参与）。大衰神那位走 [`Books::acquire_denied`] 这**同一句**判据、问在出价那一头 ——
//! 于是他永远不会成为赢家。P6b② 留下的那格「拍卖流标读同一句还是另起一句」的答案是：
//! 读同一句，且只问一次。反过来若让他喊价、再在他落槌时宣布成交无效，就要为一笔已经定下的
//! 买卖再开一次撤销，那是第二条生命周期而不是一条判据。

use bevy::ecs::system::SystemParam;
use bevy::prelude::*;

use std::collections::VecDeque;

use crate::land::LandId;
use crate::landing::{Pending, PendingDecision};
use crate::msg::{
    AuctionFailed, AuctionMove, AuctionStarted, AuctionWon, BidMove, BidPlaced, BidWithdrawn,
};
use crate::rng::Rng;
use crate::rules::GameRules;
use crate::settle::Books;
use crate::status::LandStatus;
use crate::turn::{TurnOrder, TurnPhase};

/// 拍卖机现在在做什么。
///
/// 五格各管一件事，拆开写是因为它们的**寿命**不同：`lot` 是正在进行的这一场，
/// `standby` 是排好了队还没开拍的下一块，`liquidation` 是死者留下的整处清算，
/// `requested` 是拍卖卡那一刻许下的愿（它要等 [`auction_hold_system`] 拿到座位序才建得出场），
/// `resume` 是散场之后回合该回哪一阶段。
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct Auction {
    pub lot: Option<Lot>,
    /// 排队待拍的下一块。只有破产变卖会填 —— 一个人名下常不止一块，而一次只能拍一块。
    pub standby: VecDeque<Listing>,
    /// 等着开拍的破产清算：带的是**死者名下每一块地**。归属与抵押那一刻已经清了，
    /// 而楼还在地上（[`Books::bankrupt`]，10-22 实报）—— 这里记的是「哪几块本来是他的」，
    /// 不是「它们值多少」，后者随时能从世界上读回来。
    pub liquidation: Vec<LiquidationPlan>,
    /// 拍卖卡刚许下的那一槌（新闻那条走 [`Auction::standby`]，因为它的块集合当场就定得下来）。
    pub requested: Option<Listing>,
    pub resume: Option<TurnPhase>,
}

/// 这一槌是**谁**开的 —— 「所得归谁」与「无主的地凭什么上拍」两句都由它一次答完。
///
/// 为什么不是两个 `Option<Entity>`（开槌人 + 原主）：那两个位今天能摆出四种组合，而其中
/// 「无原主 + 无开槌人」那一格在旧口径里是**上不了拍**的（[`OpenDeny::Unowned`] 正是为它建的）。
/// 新闻「公开拍卖公有土地」要的恰恰是那一格 —— 所以把三条来路写成三个变体，让「这一槌凭什么敲」
/// 在类型上有名字，而不是由调用方传两个 `None` 碰运气（那两个 `None` 与「忘了填」读起来一模一样）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gavel {
    /// 拍卖卡：出牌人当场挑一块**别人手里**的地，所得归他（10-22 拍板）
    Card(Entity),
    /// 破产清算：死者名下的地。原主由这一格带进来（此刻世界上它已无主），所得归银行
    Liquidation(Entity),
    /// 新闻「公开拍卖公有土地」：地本来就没有主人，政府挂牌，所得归银行。
    ///
    /// ✅ 10-24 拍板那两句：钱归银行，而且**不进任何公司的当月损益账**（`LedgerEntry` 只有
    /// 保费／服务费／保险赔付三个写入点），所以它与命运罚款是同一个对手方 —— 离开牌桌。
    Public,
}

impl Gavel {
    /// 谁开的槌。只有拍卖卡那一位拿钱，其余两条归银行（`None` 就是「桌上没人收这笔」）。
    pub const fn by(self) -> Option<Entity> {
        match self {
            Self::Card(founder) => Some(founder),
            Self::Liquidation(_) | Self::Public => None,
        }
    }

    /// 这块地本来在谁手里 —— 顶栏那句「从谁手里拍走」念的是它。`owner` 是世界上**此刻**的业主。
    pub fn seller_of(self, owner: Option<Entity>) -> Option<Entity> {
        match self {
            // 清算那一条：地刚从死者名下摘掉，世界上的 `Owner` 已经是 `None`，
            // 所以「原主」只能由这一格带进来（[`Books::bankrupt`] 摘归属不摘楼）
            Self::Liquidation(dead) => Some(dead),
            // 公有地没有原主。那一格留 `None` 而不是塞一个假业主：面板那句要念「政府挂牌」，
            // 「从无主手里拍走」是一句会让人觉得漏了谁的话
            Self::Public => None,
            Self::Card(_) => owner,
        }
    }
}

/// 一块要挂拍的地。三条来路（拍卖卡那一槌、新闻拍的那几块公有地、清算现抽的那几块）
/// 都写成形，而**它们各自带的就是「这一槌凭什么敲」**。
///
/// 这块表上**没有层数**那一格：楼跟着地一起站在街上，起拍价由 [`Books::market_value`]
/// 现场读世界上那几层（见模块头第二条）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listing {
    pub land: LandId,
    pub gavel: Gavel,
}

/// 一处破产清算留下的料。
///
/// `dead` 带的是 `Entity` 而不是 `PlayerId`（与 `msg.rs` 文件头那句「玩家一律用 `Entity`
/// 指代」同一条），而 `plots` 只带**地块号**：那几块地的归属与抵押已经在清算那一刻清了，
/// 层数没有（[`Books::bankrupt`]）—— 挂拍要定价的那栋楼此刻还在世界上，读得到就不必另记一份。
/// 旧版在这里记的是 `(LandId, u8)`，那个 `u8` 唯一的用途是把「被自己清零的层数」带回来，
/// 属于替一个不存在的毛病打补丁。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LiquidationPlan {
    pub dead: Entity,
    pub plots: Vec<LandId>,
}

/// 正在进行的这一场。
#[derive(Debug, Clone, PartialEq)]
pub struct Lot {
    pub land: LandId,
    /// 这一槌谁开的 —— 见 [`Gavel`]。「所得归谁」由它答，所以这里不再另存一个 `by`：
    /// 那两个位会各自漂（一块公有地既能写成 `by: None` 也能写成 `Gavel::Public` 的时候，
    /// 就轮到读它们的那两处各猜一次）。
    pub gavel: Gavel,
    /// 这块地原来是谁的（`Entity` 而不是 `PlayerId`，与 `msg.rs` 文件头那条同一条纪律）。
    /// 顶栏那句「从谁手里拍走」与 dump 念的是它；`None` = 公有地，那里念的是「政府挂牌」。
    pub seller: Option<Entity>,
    /// 起拍价（已过物价）。还没人认价时 [`Lot::next_amount`] 就是它
    pub opening: i64,
    /// 一档加多少
    pub step: i64,
    /// 当前最高喊价
    pub high: i64,
    pub leader: Option<Entity>,
    /// 还没放弃的人，按座位序。放弃就从这里减一位 —— 「谁已出局」因此只有一份真相
    pub pool: Vec<Entity>,
    /// `pool` 里此刻该谁
    pub turn: usize,
}

impl Lot {
    /// 下一个合法喊价。没人认价时它就是起拍价 —— 写成函数而不是散在判据里，是因为
    /// 「按钮上那个数」与「规则层扣的那个数」必须是同一个表达式。
    pub fn next_amount(&self) -> i64 {
        if self.leader.is_none() {
            self.opening
        } else {
            self.high + self.step
        }
    }

    /// 此刻该出价的人。`None` = 这一场已经没人可问了。
    pub fn bidder(&self) -> Option<Entity> {
        self.pool.get(self.turn).copied()
    }

    /// 谁开的槌（`None` = 公家或死者，所得归银行）。转给 [`Gavel::by`] —— 面板与流水要问的
    /// 是这一句，而「三种来路各有哪一格」那一层不该漏到读它的人面前。
    pub fn by(&self) -> Option<Entity> {
        self.gavel.by()
    }
}

impl Auction {
    /// 这台机器**还占着回合吗**。四样里有任何一样就不该交出去：正在进行的这一场、
    /// 排着队的下一块、等着抽块的清算、拍卖卡刚许下的那一槌。
    ///
    /// 这个问句是给 [`crate::landing::close_landing_system`] 用的，而它必须被问出来：
    /// 落格那条链的末尾「没人等回答就把回合收尾」与本机的「把窗口挂到下一个人」写的可能是
    /// **同一个** `NextState`，而破产恰好发生在落格那一帧 —— 两处各写一次时赢的是同一帧里
    /// 后写的那一条，而那取决于 `OnEnter(Landed)` 与 `Update` 的相对次序。让它先问一句，
    /// 这条竞态就不存在了：机器还占着，「收尾」那一条根本不会被写出来。
    ///
    /// 只写 `lot.is_some()` 是不够的：一处清算在两场之间确实有那么一帧 `lot == None`，
    /// 而那一帧里回合机照样不该走 —— 下一块地就排在队列里，它要的只是「再问一圈人」。
    pub fn holds_turn(&self) -> bool {
        self.lot.is_some()
            || !self.standby.is_empty()
            || !self.liquidation.is_empty()
            || self.requested.is_some()
    }
}

/// 开不成场的原因。灰按钮的文案与规则层的拒绝共用这一张表（同 `ShopDeny`/`LotteryDeny`）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenDeny {
    /// 这块地没有业主 —— 无主地本来就能直接买（踩到那一格，或购地卡），不必绕一台拍卖机。
    /// 这是**拍卖卡与破产清算**那两条的判据；新闻「公开拍卖公有土地」拍的就是无主地，
    /// 所以它挨的是下面那条反过来的问句（见 [`Gavel::Public`]）
    Unowned,
    /// 这块地**有**主人 —— [`Gavel::Public`] 那一槌唯一的上不了拍的理由：政府只挂牌卖自己的地。
    /// 与 `Unowned` 是同一条问句的两个答案，所以两条分开写而不是合成一条「归属不对」：
    /// 面板与 `warn!` 要念得出的是「这块已经卖过了」还是「这块根本没人挂」
    Private,
    /// 开槌的人自己的地。拍自己的地等于把左口袋卖给自己，而所得还归自己
    OwnLand,
    /// 押在银行里的地。与恶魔夺地同一口径（`Books::take_land` 也拒这一格）：带着债的地过户，
    /// 要么债随地产走（替陌生人还债），要么留下一份无主的抵押（那会永久挡租）
    Mortgaged,
    /// 查封中的地。法院贴着条子的地不许转移 —— 与「不许施工」读的是同一个状态
    Sealed,
    /// 场上没有一个可出价的人（只剩死者自己，或全被关着）。开一场必流标的拍卖不如当场说明
    /// 「没有别人」
    NoBidder,
}

impl OpenDeny {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unowned => "这块地还没有主人",
            Self::Private => "这块地已有主人，公有地拍卖只拍无主的",
            Self::OwnLand => "不能拍卖自己的地",
            Self::Mortgaged => "这块地押在银行",
            Self::Sealed => "这块地被查封",
            Self::NoBidder => "场上没有别人可以出价",
        }
    }
}

/// 出价不成的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BidDeny {
    /// 此刻没有进行中的拍卖
    NoAuction,
    /// 这一轮不该他喊（轮转式的全部含义在这一格）
    NotYourTurn,
    /// 他身上那位让置产必败（大衰神／小衰神）
    GodRefuses,
    /// 加不起下一档
    TooBroke,
}

impl BidDeny {
    /// 每颗灰按钮都得带一句为什么，否则「按不动」与「坏了」在玩家眼里是同一件事
    /// （§6.1 坑 27 的 UI 版）。
    pub const fn label(self) -> &'static str {
        match self {
            Self::NoAuction => "现在没有拍卖",
            Self::NotYourTurn => "还没轮到你",
            Self::GodRefuses => "衰神在，拍地必败",
            Self::TooBroke => "现金不够这一档",
        }
    }
}

/// 判一次出价要读的那几样。面板与规则层读同一份，才会「灰按钮按下去确实没声音」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BidFacts {
    pub at_lot: bool,
    /// 现在问的就是他
    pub yours: bool,
    /// 他身上那位让置产必败
    pub denied_by_god: bool,
    pub cash: i64,
    /// 下一档要多少
    pub needed: i64,
}

/// 顺序是刻意的：先问「有没有这一场」，再问「是不是你」，再问「你办不办得动」，
/// 最后才问钱 —— 前三条不对时，钱那条会说出一句误导的话
/// （一个轮不到他的人得到「现金不够」，而真正的原因是还没轮到他）。
pub fn bid_allowed(f: &BidFacts) -> Result<(), BidDeny> {
    if !f.at_lot {
        return Err(BidDeny::NoAuction);
    }
    if !f.yours {
        return Err(BidDeny::NotYourTurn);
    }
    if f.denied_by_god {
        return Err(BidDeny::GodRefuses);
    }
    if f.cash < f.needed {
        return Err(BidDeny::TooBroke);
    }
    Ok(())
}

/// 谁**有资格**出价（判据，不含次序）：没破产、不在拘留处、不是开槌的人。
///
/// 大衰神那位**留在名单上** —— 他被挡的是出价那一刻，不是从名单上被抹掉。抹掉的话面板就
/// 说不清「他去哪了」，而他明明坐在牌桌上。
///
/// 判据与座位序**分成两个函数**，是因为出牌前那一道「这张卡会不会落空」的问（`hand.rs` 的
/// `unmet_precondition`）只借得到账本、借不到 `TurnOrder` —— 它要回答的是「有没有别人」，
/// 而「先问谁」不是它要管的。把两件事写在一个函数里，那道门就得为一次排序去拿一个它
/// 本来不需要的位置参数。
pub fn bidders(books: &Books<'_, '_>, by: Option<Entity>) -> Vec<Entity> {
    books
        .solvent()
        .into_iter()
        .map(|(player, _)| player)
        .filter(|player| Some(*player) != by)
        .filter(|player| books.detained.get(*player).is_err())
        .collect()
}

/// 把有资格的人按**座位序**排一遍。拍卖一圈一圈问的就是这个次序，
/// 而它必须稳定 —— 同种子复现同一串喊价，靠的是「谁先喊」不是随机。
pub fn seat_order(order: &TurnOrder, eligible: &[Entity]) -> Vec<Entity> {
    let at = |player: Entity| {
        order
            .order
            .iter()
            .position(|p| *p == player)
            .unwrap_or(usize::MAX)
    };
    let mut pool: Vec<Entity> = eligible.to_vec();
    pool.sort_by_key(|p| at(*p));
    pool
}

/// 这块地此刻能不能上拍。**唯一的闸门**，三条入口（拍卖卡、破产清算、新闻那条公有地）
/// 都问它 —— 「面板那颗按钮亮不亮」与「规则层肯开场」因此不可能分叉。
///
/// 只读、且不依赖座位序（见 [`bidders`] 上面那段），于是它既能在出牌前当「会不会落空」
/// 那一问（卡不白扣，与 P6d 那两张符同一条纪律），也能在建场前用。
///
/// 「有没有主人」这一问的答案**按来路相反**，而那句相反就写在 [`Gavel`] 上：拍卖卡拍的是
/// 别人手里的地（无主地本来就能直接买，绕一台拍卖机只是多一步），清算场里那块地刚从死者名下
/// 摘掉、此刻本就无主，而新闻那条「公开拍卖公有土地」**要的就是无主地**。
/// 旧版用 `seller.is_none() && view.owner.is_none()` 表达同一件事，它在两条来路下都对，
/// 第三条一加就露出形状：那一句把「没有原主」同时当成了「不许拍」与「正常状态」。
pub fn can_open(books: &Books<'_, '_>, land: LandId, gavel: Gavel) -> Result<(), OpenDeny> {
    let view = books.view(land).ok_or(OpenDeny::Unowned)?;
    match gavel {
        Gavel::Public => {
            if view.owner.is_some() {
                return Err(OpenDeny::Private);
            }
        }
        Gavel::Card(_) if view.owner.is_none() => return Err(OpenDeny::Unowned),
        Gavel::Card(_) | Gavel::Liquidation(_) => {}
    }
    if gavel
        .by()
        .is_some_and(|founder| books.owner_of(land) == Some(founder))
    {
        return Err(OpenDeny::OwnLand);
    }
    if view.mortgaged {
        return Err(OpenDeny::Mortgaged);
    }
    if view.status == Some(LandStatus::Sealed) {
        return Err(OpenDeny::Sealed);
    }
    if bidders(books, gavel.by()).is_empty() {
        return Err(OpenDeny::NoBidder);
    }
    Ok(())
}

/// 建一场。起拍价读的是这块地**此刻**在世界上的样子（地价 + 地上那一栋楼，再过物价）——
/// [`Books::market_value`] 与 [`Books::buy_price`] 用的是同一条式子，所以「起拍=买下它本来
/// 要付的钱」不是注释而是代码事实。破产的那几层不用带进来：它们从没被摘掉（模块头第二条）。
///
/// 「谁开的槌」与「原来在谁手里」两句都从 `listing.gavel` 推，不再由调用方各传一个
/// `Option<Entity>`：那样三条来路要各自记住「我该传什么」，而传错的两种形状（多传一个假业主、
/// 少传一个真开槌人）在画面上分别是「顶栏凭空多出一个卖家」与「钱凭空进了某人的钱包」。
pub fn open(
    books: &Books<'_, '_>,
    order: &TurnOrder,
    rules: &GameRules,
    listing: &Listing,
) -> Result<Lot, OpenDeny> {
    let land = listing.land;
    let gavel = listing.gavel;
    can_open(books, land, gavel)?;
    let opening = books.market_value(land).ok_or(OpenDeny::Unowned)?;
    Ok(Lot {
        land,
        gavel,
        seller: gavel.seller_of(books.owner_of(land)),
        opening,
        step: step_of(opening, rules.auction_step_percent),
        high: opening,
        leader: None,
        pool: seat_order(order, &bidders(books, gavel.by())),
        turn: 0,
    })
}

/// 一档加多少：起拍价的百分之几，夹一个下限 —— 一块 300 元的地按 10% 是 30 元一档，
/// 一场拍卖要点二十几次才见得出胜负，那不是竞价而是点击练习。
/// 整数乘除向下取整（与 `shop::buyback` 同一条纪律）。
pub fn step_of(opening: i64, percent: u32) -> i64 {
    (opening * percent as i64 / 100).max(50)
}

/// 一次加价之后：他还留在名单上（下一轮还可能被问），只是轮到座位序里下一位。
pub fn record_raise(lot: &mut Lot, player: Entity, amount: i64) {
    lot.high = amount;
    lot.leader = Some(player);
    if !lot.pool.is_empty() {
        lot.turn = (lot.turn + 1) % lot.pool.len();
    }
}

/// 一次放弃之后：他从名单上减掉。减完之后「该谁」落在**同一个下标**上 ——
/// 那正是被减掉那位后面的下一位；越界时取模绕回队首。
pub fn record_withdraw(lot: &mut Lot, player: Entity) {
    let Some(at) = lot.pool.iter().position(|p| *p == player) else {
        return;
    };
    lot.pool.remove(at);
    lot.turn = if lot.pool.is_empty() {
        0
    } else {
        at % lot.pool.len()
    };
}

/// 这一场结了没有。两条出口，都在「剩下的人喊不动了」那一刻成立：
///
/// - 名单空了 → 有领先者就卖给他（他喊过的价不因为他随后放弃而收回 —— 落槌前收回喊价没有
///   意义，而价格已经被他推到那儿）；没有领先者就是流标。
/// - 只剩一位**而且他就是领先者** → 当场落槌，不再问第二遍（问他只会让他给自己加价）。
///   剩下最后一位但不是领先者的那位必须被再问一次：他可能愿意出更高的价。
pub fn close_if_done(lot: &Lot) -> Option<Close> {
    if lot.pool.is_empty() {
        return Some(Close::of(lot));
    }
    let only_leader = lot.pool.len() == 1 && lot.leader == Some(lot.pool[0]);
    if only_leader {
        return Some(Close::of(lot));
    }
    None
}

/// 散场那一刻的读数。`sold_to == None` 是流标，`price` 那格带着起拍价供播报。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Close {
    pub sold_to: Option<Entity>,
    pub price: i64,
}

impl Close {
    fn of(lot: &Lot) -> Self {
        Self {
            sold_to: lot.leader,
            price: lot.leader.map(|_| lot.high).unwrap_or(lot.opening),
        }
    }
}

/// 拍卖机要动的东西。打包是房规（§六 坑 8），而这里另有一条硬理由：
/// [`Books`] 自己就带着 `ResMut<Auction>`（破产那一刻要就地记下待拍的料、出牌那一刻要许下
/// 开槌的愿），所以任何想同时碰账本与拍卖机的 system **必须借账本里那一格**，不能另拿一条
/// `ResMut<Auction>` —— 同一个 system 参数里两条对同一资源的可变借用，会在参数建立期直接撞。
#[derive(SystemParam)]
pub struct AuctionDesk<'w, 's> {
    pub books: Books<'w, 's>,
    pub order: Res<'w, TurnOrder>,
    pub pending: ResMut<'w, PendingDecision>,
    pub phase: Res<'w, State<TurnPhase>>,
    pub next: ResMut<'w, NextState<TurnPhase>>,
    pub started: MessageWriter<'w, AuctionStarted>,
    pub placed: MessageWriter<'w, BidPlaced>,
    pub withdrawn: MessageWriter<'w, BidWithdrawn>,
    pub won: MessageWriter<'w, AuctionWon>,
    pub failed: MessageWriter<'w, AuctionFailed>,
}

impl<'w, 's> AuctionDesk<'w, 's> {
    fn rules(&self) -> GameRules {
        *self.books.rules
    }

    /// 此刻的拍卖机（从账本里那一格读，见 [`AuctionDesk`] 上面那段）。
    pub fn auction(&self) -> &Auction {
        &self.books.auction
    }
}

/// 办「加价」与「放弃」。
///
/// 它**不动回合状态**（散场时也只把队列与 `resume` 落进资源）：窗口与阶段都由
/// [`auction_hold_system`] 维持，于是「散场同帧接着开下一块」只有一处实现。
pub fn auction_move_system(mut desk: AuctionDesk<'_, '_>, mut moves: MessageReader<AuctionMove>) {
    // 先收齐再逐条办：每条都按**当时**的窗口判，所以同帧两条「加价」只有第一条成立
    // （第二条会撞上「还没轮到你」—— 轮转已经在第一条里前进过一位了）
    let requests: Vec<AuctionMove> = moves.read().copied().collect();
    for request in requests {
        let player = request.player;
        let Some(lot) = desk.books.auction.lot.clone() else {
            debug!("拍卖被拒：{:?} 在没有拍卖的时候出价", player);
            continue;
        };
        let yours = lot.bidder() == Some(player);
        let facts = BidFacts {
            at_lot: true,
            yours,
            denied_by_god: desk.books.acquire_denied(player),
            cash: desk.books.balance(player),
            needed: lot.next_amount(),
        };
        // 放弃永远办得成（只要轮到他）：把「不想玩了」挡在门外，回合机就死停在这一格。
        // 而「加价」要过四道判据 —— 与 `buy_allowed` 同一条纪律：灰按钮与这里读同一张表
        match request.what {
            BidMove::Raise => {
                if let Err(why) = bid_allowed(&facts) {
                    debug!("拍卖被拒：{:?} 想加价 —— {}", player, why.label());
                    continue;
                }
            }
            BidMove::Withdraw => {
                if !yours {
                    debug!("拍卖被拒：{:?} 在不该他喊的时候放弃", player);
                    continue;
                }
            }
        }
        let mut lot = lot;
        let land = lot.land;
        match request.what {
            BidMove::Raise => {
                let amount = lot.next_amount();
                record_raise(&mut lot, player, amount);
                desk.placed.write(BidPlaced {
                    player,
                    land,
                    amount,
                });
            }
            BidMove::Withdraw => {
                record_withdraw(&mut lot, player);
                desk.withdrawn.write(BidWithdrawn { player, land });
            }
        }
        let done = close_if_done(&lot);
        // 还没散场就写回这一场的读数；散场了则由 `settle` 负责清空与下一步
        desk.books.auction.lot = done.is_none().then(|| lot.clone());
        if let Some(close) = done {
            // `lot` 是**这一场**的读数（资源里那位已经是 `None`），落槌那段要用它
            settle(&mut desk, &lot, close);
        }
    }
}

/// 落槌那一段：过户、收钱、写凭据，再决定这台机器是空着还是接着排下一块。
fn settle(desk: &mut AuctionDesk<'_, '_>, lot: &Lot, close: Close) {
    let land = lot.land;
    let by = lot.by();
    match close.sold_to {
        Some(buyer) => {
            // 钱可能真的不够：拍卖期间他仍能动 ATM（那块浮层不阻断回合），把钱借出去或存进去
            // 之后，落槌那一步就付不出了。所以这不是「为不可能留的分支」，而它选的出口是
            // **流标**（地不转手）—— 与 `decision_system` 里「买地付不出就什么也不发生」
            // 同一条既有口径，而不是「先扣钱再退钱」那种两段式
            if desk.books.auction_sale(buyer, land, close.price).is_some() {
                if let Some(founder) = by {
                    desk.books.credit(founder, close.price);
                }
                desk.won.write(AuctionWon {
                    player: buyer,
                    land,
                    price: close.price,
                    by,
                    seller: lot.seller,
                });
                let who = desk
                    .books
                    .id_of(buyer)
                    .map(|p| p.0.to_string())
                    .unwrap_or_else(|| "?".to_string());
                info!(
                    "拍卖落槌：{who} 以 {} 元拍得 {land:?}（{}）",
                    close.price,
                    match lot.gavel {
                        Gavel::Card(_) => "所得归开槌的人",
                        // 两句各自说清是**谁**的钱离开了牌桌：清算那句的主体是死者，
                        // 公有地那句的主体是政府 —— 两者都不进公司账，但玩家听到的不是同一件事
                        Gavel::Liquidation(_) => "所得归银行（破产变卖）",
                        Gavel::Public => "所得归银行（公有地挂牌）",
                    }
                );
            } else {
                desk.failed.write(AuctionFailed {
                    land,
                    by,
                    seller: lot.seller,
                    price: close.price,
                });
                warn!(
                    "拍卖落槌却收不到钱：{land:?} 流标（领先者付不出 {}）",
                    close.price
                );
            }
        }
        None => {
            desk.failed.write(AuctionFailed {
                land,
                by,
                seller: lot.seller,
                price: close.price,
            });
            info!("拍卖流标：{land:?} 没人出价（起拍 {}）", close.price);
        }
    }
    // 散场之后该回哪一阶段：只有这台机器**彻底空了**才写。还排着下一块时不写，
    // 因为 [`auction_hold_system`] 同一帧就会把下一场挂上，中间插一次「回合收尾」
    // 会让那块地没人来拍。
    if desk.books.auction.standby.is_empty() && desk.books.auction.liquidation.is_empty() {
        desk.books.auction.resume = Some(match lot.gavel {
            // 拍卖卡是掷骰前打的，拍完他还要接着走
            Gavel::Card(_) => TurnPhase::WaitRoll,
            // 清算与新闻都发生在落格结算里，拍完这一回合就该收尾
            Gavel::Liquidation(_) | Gavel::Public => TurnPhase::TurnEnd,
        });
    }
    desk.books.auction.lot = None;
}

/// 维持这台机器：建场、把窗口挂到该出价的人身上、散场时把回合交回去。
///
/// 它排在 [`auction_move_system`] **之后**，而且要写成一条 `chain` 而不是并行元组里换个位置
/// （元组内顺序不保证，§6.1 坑 36）。
pub fn auction_hold_system(mut desk: AuctionDesk<'_, '_>, mut rng: ResMut<Rng>) {
    if matches!(desk.phase.get(), TurnPhase::Boot | TurnPhase::Over) {
        return;
    }
    // 机器空着才去找下一块 —— 一场没散就开第二场等于同时挂着两个 bidder，
    // 而 PLAN §四 那句「同时挂多个 bidder」在本阶段的形状是**队列**（模块头第一条）。
    // 用 `let` 绑定而不是嵌套 `if`：内层要 `&mut desk`，clippy 会要求把两层并成一条
    // `&&`，而并起来的那一行读起来像是「先建场再判断」这种奇怪的顺序。
    let fresh = if desk.books.auction.lot.is_none() {
        next_lot(&mut desk, &mut rng)
    } else {
        None
    };
    if let Some(lot) = fresh {
        desk.started.write(AuctionStarted {
            land: lot.land,
            by: lot.by(),
            seller: lot.seller,
            opening: lot.opening,
            step: lot.step,
            bidders: lot.pool.len() as u32,
        });
        desk.books.auction.lot = Some(lot);
    }
    match desk.books.auction.lot.clone() {
        Some(lot) => {
            // 拍卖挤掉同一格里别的窗口（典型那一幕：他付不出租金 → 当场破产 → 名下的地上拍，
            // 而那一趟的租金收据还挂着）。被挤掉的那些都在「他已经付不出」之后，
            // 再摆出来只是让人读一张不会有下一步的收据。
            if let Some(bidder) = lot.bidder() {
                desk.pending.offer(bidder, Pending::Auction);
            }
            desk.next.set(TurnPhase::WaitDecision);
        }
        None => {
            if desk
                .pending
                .active()
                .is_some_and(|(_, about)| about == Pending::Auction)
            {
                desk.pending.clear();
            }
            if let Some(resume) = desk.books.auction.resume.take() {
                desk.next.set(resume);
            }
        }
    }
}

/// 下一场该开哪一块。三条来源按优先级：**拍卖卡许下的那一槌** → **排着队的下一块** →
/// **死者留下的一处新清算**（现选、现排队、再取第一块）。
///
/// 队列里混得下两种来路（清算抽出来的那几块与新闻挂上来的那几块公有地），因为「这一块凭什么
/// 上拍」写在 [`Listing::gavel`] 里而不是在字段位置里 —— 用位置表达的话，两条队列就得并成一条
/// 带标签的，而那条带标签的迟早与 `can_open` 那三格分叉。
///
/// 开不成就跳过这一块而不是卡住整台机器 —— 「一块押着的地把一场清算永久停住」比它要防的
/// 那个错误难查得多，而 `warn!` 会替玩家把这件事说出口。
fn next_lot(desk: &mut AuctionDesk<'_, '_>, rng: &mut Rng) -> Option<Lot> {
    let rules = desk.rules();
    if let Some(listing) = desk.books.auction.requested.take() {
        let land = listing.land;
        return match open(&desk.books, &desk.order, &rules, &listing) {
            Ok(lot) => Some(lot),
            Err(why) => {
                warn!("拍卖开不成（{land:?}）：{}", why.label());
                None
            }
        };
    }
    // 死者留下的整处清算：现选几块排进队（一次一处，两处破产同帧就先拍完这一处）
    if desk.books.auction.standby.is_empty() && !desk.books.auction.liquidation.is_empty() {
        let plan = desk.books.auction.liquidation.remove(0);
        for land in pick_lots(rng, &plan.plots, rules.liquidation_lots) {
            desk.books.auction.standby.push_back(Listing {
                land,
                gavel: Gavel::Liquidation(plan.dead),
            });
        }
    }
    while let Some(listing) = desk.books.auction.standby.pop_front() {
        match open(&desk.books, &desk.order, &rules, &listing) {
            Ok(lot) => return Some(lot),
            Err(why) => {
                warn!("拍卖开不成（{:?}）：{}", listing.land, why.label());
            }
        }
    }
    None
}

/// 从死者的地产里抽最多 `count` 块，**不放回**，并按原序交回去。
///
/// 抽法走第十三条子流：「这一局进过几次店」不该改变谁的地被拍（与 `pick_shop`／`pick_lottery`
/// 同一条理由）。按原序交回去是为了让「哪一块排第一」只由种子决定，不随 `Vec` 的插入顺序抖。
fn pick_lots(rng: &mut Rng, plots: &[LandId], count: u32) -> Vec<LandId> {
    let want = (count as usize).min(plots.len());
    let mut free: Vec<usize> = (0..plots.len()).collect();
    let mut taken: Vec<usize> = Vec::with_capacity(want);
    for _ in 0..want {
        if free.is_empty() {
            break;
        }
        let at = rng.pick_auction(0..=free.len() as u32 - 1) as usize;
        taken.push(free.swap_remove(at));
    }
    taken.sort_unstable();
    taken.into_iter().map(|i| plots[i]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    // 测试里的玩家不来自世界，而 `Entity` 的裸构造在 0.20 只认 `from_raw_u32`
    // （索引撞上保留值会返回 `None`，所以那一句 `expect` 是必需的而不是装饰）
    fn one() -> Entity {
        Entity::from_raw_u32(1).expect("索引不该撞上保留值")
    }
    fn two() -> Entity {
        Entity::from_raw_u32(2).expect("索引不该撞上保留值")
    }
    fn three() -> Entity {
        Entity::from_raw_u32(3).expect("索引不该撞上保留值")
    }
    fn seller() -> Entity {
        Entity::from_raw_u32(9).expect("索引不该撞上保留值")
    }

    /// 三条来路各答两句：钱进谁的钱包（[`Gavel::by`]）与顶栏那句「从谁手里拍走」
    /// （[`Gavel::seller_of`]）。公有地那一条**两格都是 `None`** —— 那不是缺了两个数，
    /// 那正是 ✅ 10-24 拍板的全部内容：没人敲槌（所以没人收钱）、没人卖地
    /// （所以那句「从谁手里」不该出现在屏幕上）。
    ///
    /// 最后那一格是这条用例的牙齿：`seller_of` 拿到「此刻的业主」时不许把他说成卖家 ——
    /// 公有地一旦有了主人就根本不该站在这个台上（`OpenDeny::Private` 是同一句话的另一半），
    /// 而「把现任业主写成原主」的下一步就是「顶栏念了一句凭空多出的话」。
    #[test]
    fn each_gavel_answers_who_gets_the_money_and_who_sold_it() {
        let (owner, dead, founder) = (one(), two(), three());
        assert_eq!(Gavel::Card(founder).by(), Some(founder));
        assert_eq!(Gavel::Card(founder).seller_of(Some(owner)), Some(owner));
        assert_eq!(Gavel::Liquidation(dead).by(), None, "清算没有开槌的人");
        assert_eq!(
            Gavel::Liquidation(dead).seller_of(None),
            Some(dead),
            "清算场里世界上已经没有业主，原主只能由槌带进来"
        );
        assert_eq!(Gavel::Public.by(), None, "公有地的所得不进任何玩家的钱包");
        assert_eq!(Gavel::Public.seller_of(None), None);
        assert_eq!(
            Gavel::Public.seller_of(Some(owner)),
            None,
            "政府挂牌不认「此刻的业主」当卖家"
        );
    }

    fn lot() -> Lot {
        Lot {
            land: LandId(1),
            gavel: Gavel::Liquidation(seller()),
            seller: Some(seller()),
            opening: 2_000,
            step: 200,
            high: 2_000,
            leader: None,
            pool: vec![one(), two(), three()],
            turn: 0,
        }
    }

    /// 起拍价那一格：**还没人认价时，下一个合法喊价就是起拍价本身**，而不是起拍 + 一档。
    /// 反过来写的话，一块没人理的地也会被第一声喊价抬高一档，而那句「起拍 2000」成了假话。
    #[test]
    fn the_first_bid_is_the_opening_price() {
        let mut lot = lot();
        assert_eq!(lot.next_amount(), 2_000);
        let amount = lot.next_amount();
        record_raise(&mut lot, one(), amount);
        assert_eq!((lot.high, lot.leader), (2_000, Some(one())));
        assert_eq!(lot.next_amount(), 2_200, "有人认价之后才是增价拍卖");
    }

    /// 轮转与「放弃即出局」的全部算术。这台机器的骨架钉在纯函数上，世界侧的用例才不必
    /// 为了「该谁」再摆一遍随机。
    #[test]
    fn rotation_walks_the_pool_and_drops_whoever_passes() {
        let mut lot = lot();
        assert_eq!(lot.bidder(), Some(one()));
        let amount = lot.next_amount();
        record_raise(&mut lot, one(), amount);
        assert_eq!(
            lot.bidder(),
            Some(two()),
            "喊过价的人留在名单上，但轮到下一位"
        );

        // 第二位放弃：他出局，「该谁」落在同一个下标上 = 第三位
        record_withdraw(&mut lot, two());
        assert_eq!(lot.pool, vec![one(), three()]);
        assert_eq!(lot.bidder(), Some(three()));

        // 第三位是队尾，放弃之后下标越界 → 绕回队首
        record_withdraw(&mut lot, three());
        assert_eq!(lot.pool, vec![one()]);
        assert_eq!(lot.bidder(), Some(one()));

        // 只剩他一个，而他就是领先者 → 当场落槌，不再问第二遍
        let close = close_if_done(&lot).expect("该散场了");
        assert_eq!((close.sold_to, close.price), (Some(one()), 2_000));
    }

    /// 被减掉的是队尾时，`turn` 必须取模而不是照抄 —— 照抄会让「该谁」指到不存在的那一格，
    /// 症状是拍卖停在没人能回答的地方。
    #[test]
    fn dropping_the_last_index_wraps_instead_of_pointing_off_the_end() {
        let mut lot = lot();
        lot.turn = 2;
        record_withdraw(&mut lot, three());
        assert_eq!(lot.turn, 0, "队尾被减掉之后该回到队首");
        assert_eq!(lot.bidder(), Some(one()));
    }

    /// **只剩的那一位不是领先者**时不许散场：他还得被问一次（他可能愿意盖过那个价）。
    /// 这条与上面那条只差「领先者是谁」，而它的反面（一律散场）会让拍卖在
    /// 「两人互不相让」那一幕里少收一笔钱，且全程静默。
    #[test]
    fn the_last_man_still_gets_asked_when_he_is_not_the_leader() {
        // 直接摆出那个形状：两位还在场，其中一位已经喊到 2200，现在该另一位喊
        let mut lot = lot();
        lot.pool = vec![one(), two()];
        lot.leader = Some(one());
        lot.high = 2_200;
        lot.turn = 1;
        assert!(
            close_if_done(&lot).is_none(),
            "还剩两位，其中一位还没被问过 —— 不该落槌"
        );

        // 领先者放弃：他被减掉，而他喊过的那个价还挂在他名下
        record_withdraw(&mut lot, one());
        assert_eq!(lot.pool, vec![two()]);
        assert_eq!(lot.leader, Some(one()), "领先者已出局，但价格是他推上去的");
        assert_eq!(
            close_if_done(&lot),
            None,
            "只剩一位而他不是领先者 —— 必须再问他一次"
        );

        // 他认了这个价，槌才落（而落槌价是他刚喊的那个，不是原来的 2200）
        let raised = lot.next_amount();
        record_raise(&mut lot, two(), raised);
        let close = close_if_done(&lot).expect("现在他就是领先者且只剩他一位");
        assert_eq!((close.sold_to, close.price), (Some(two()), 2_400));
    }

    /// 没人出价 = 流标，而流标不许被悄悄当成成交。
    #[test]
    fn a_lot_nobody_touches_fails() {
        let mut lot = lot();
        for player in [one(), two(), three()] {
            record_withdraw(&mut lot, player);
        }
        assert!(lot.pool.is_empty());
        let close = close_if_done(&lot).expect("没人出价也该散场");
        assert_eq!(close.sold_to, None);
        assert_eq!(close.price, 2_000, "播报要念得出起拍价");
    }

    /// 领先者**没有**被再问一遍就落槌：别人都放弃之后，他那一声就是最终价。
    /// 这一条钉的是「不必问第二遍」那半句 —— 反过来（一直问到他自己放弃）会让一场
    /// 已经见出胜负的拍卖多走一圈，而那一圈上他唯一能做的事是给自己加价。
    #[test]
    fn the_leading_price_stands_when_everyone_else_passes() {
        let mut lot = lot();
        let opening = lot.next_amount();
        record_raise(&mut lot, one(), opening); // 2000，该 two
        let raised = lot.next_amount();
        record_raise(&mut lot, two(), raised); // 2200，该 three
        record_withdraw(&mut lot, three()); // pool [one,two]，该 one
        assert_eq!(lot.bidder(), Some(one()));
        record_withdraw(&mut lot, one()); // pool [two]，而 two 正是领先者
        let close = close_if_done(&lot).expect("只剩领先者，该落槌");
        assert_eq!((close.sold_to, close.price), (Some(two()), 2_200));
    }

    /// 拒绝表的问话顺序：轮不到你时不该说出「现金不够」—— 那是两句不同的谎话。
    #[test]
    fn the_gate_asks_the_turn_then_the_god_then_the_money() {
        let ok = BidFacts {
            at_lot: true,
            yours: true,
            denied_by_god: false,
            cash: 9_000,
            needed: 2_000,
        };
        assert_eq!(bid_allowed(&ok), Ok(()));
        assert_eq!(
            bid_allowed(&BidFacts {
                at_lot: false,
                ..ok
            }),
            Err(BidDeny::NoAuction)
        );
        assert_eq!(
            bid_allowed(&BidFacts {
                yours: false,
                cash: 0,
                ..ok
            }),
            Err(BidDeny::NotYourTurn),
            "不是他的回合时，钱那条不许抢先说话"
        );
        assert_eq!(
            bid_allowed(&BidFacts {
                denied_by_god: true,
                ..ok
            }),
            Err(BidDeny::GodRefuses)
        );
        assert_eq!(
            bid_allowed(&BidFacts { cash: 1_999, ..ok }),
            Err(BidDeny::TooBroke)
        );
    }

    /// 台阶那一格：百分之十，但夹下限。
    #[test]
    fn the_step_is_a_percent_with_a_floor() {
        assert_eq!(step_of(2_000, 10), 200);
        assert_eq!(step_of(300, 10), 50, "30 会被下限夹到 50");
        assert_eq!(step_of(0, 10), 50);
        // 整数除向下取整：不该因为四舍五入而凭空多出一档
        assert_eq!(step_of(1_499, 10), 149);
    }

    /// 抽几块地：**不放回**，且按原序交回去。两条都有反面 —— 放回会拍出两块同一天
    /// （第二场是把空地再拍一遍），而按抽中的顺序交回去会让「哪一块排第一」随插入顺序抖。
    #[test]
    fn the_liquidation_draws_without_replacement_and_keeps_the_original_order() {
        let plots: Vec<LandId> = (0..6).map(LandId).collect();
        let mut rng = Rng::new(7);
        let taken = pick_lots(&mut rng, &plots, 3);
        assert_eq!(taken.len(), 3);
        let ids: Vec<u32> = taken.iter().map(|l| l.0).collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(ids.len(), sorted.len(), "同一块地被抽了两次");
        assert_eq!(ids, sorted, "抽完要按原序交回去");
        // 抽中的就是那份表里的那一块，不是按位置凑的（`swap_remove` 会打乱顺序，
        // 而「这块地被拍了」这句话要说的是这块地）
        for land in &taken {
            assert!(plots.contains(land), "{land:?} 不在死者的名下");
        }
        // 不足三块时抽全部，而不是报错或重复
        assert_eq!(pick_lots(&mut rng, &plots[..2], 3).len(), 2);
        assert!(pick_lots(&mut rng, &[], 3).is_empty());
    }

    /// 同一颗种子抽出同一串 —— 拍卖的次序是玩家会回头对的事（「我那块地为什么是第二场」）。
    #[test]
    fn the_draw_is_a_pure_function_of_the_seed() {
        let plots: Vec<LandId> = (0..6).map(LandId).collect();
        assert_eq!(
            pick_lots(&mut Rng::new(99), &plots, 3),
            pick_lots(&mut Rng::new(99), &plots, 3)
        );
    }

    /// 每一句拒绝都有话说 —— 一句空文案与「按不动但没理由」是同一个缺陷。
    #[test]
    fn every_denial_reads_as_a_sentence() {
        for why in [
            BidDeny::NoAuction,
            BidDeny::NotYourTurn,
            BidDeny::GodRefuses,
            BidDeny::TooBroke,
        ] {
            assert!(!why.label().is_empty(), "{why:?} 没有文案");
        }
        for why in [
            OpenDeny::Unowned,
            OpenDeny::OwnLand,
            OpenDeny::Mortgaged,
            OpenDeny::Sealed,
            OpenDeny::NoBidder,
        ] {
            assert!(!why.label().is_empty(), "{why:?} 没有文案");
        }
    }
}
