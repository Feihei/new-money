//! P9 拍卖的验收：一台轮转式的英式拍卖，两处入口（拍卖卡与破产清算）共用它。
//!
//! PLAN §四 P9 那行的验收原话是三句，这个文件逐句对着钉：
//!
//! - **3 人竞价产生成交价与归属** → [`four_seats_produce_a_price_and_an_owner`]
//!   （四人一桌，开槌那位被排除，剩下三位一圈一圈喊到见胜负）；
//! - **流标时不转手** → [`a_lot_nobody_bids_on_stays_in_the_original_hand`] 与
//!   [`an_unsold_liquidation_lot_stays_on_the_market_with_its_house_standing`]（两条入口各一条 ——
//!   清算那头「不转手」的另一半是**楼照旧立着**：10-22 实报「破产后剩下的地变成无主，
//!   但上面建筑也是保留的」，拍不走的那块与其余归无主的那几块是同一种下场）；
//! - **使用者本人被排除仍能成交** → [`the_founder_is_never_asked_and_his_own_shout_is_refused`]。
//!
//! 破产变卖那一半钉四件事：抽几块（[`only_three_of_the_dead_plots_go_under_the_hammer`]）、
//! 建筑随成交过户（[`a_building_changes_hands_with_the_plot_it_stands_on]`）、拍不走就回到市场
//! 而楼不动（前面那条）、没地可拍的人不该惊动这台机器（[`a_broker_without_lands_holds_no_auction]`）。
//!
//! 出价怎么控制得住：这台机器的窗口挂在「此刻该出价的那一位」身上，所以每条用例先读
//! [`lot_of`] 里的 `bidder()` 断言一次轮转、再替**那一位**按按钮 —— 而不是按自己猜的人。
//! 「一圈一圈问」因此既是前置也是断言。回合策略一律 `Hold`：脚手架的自动玩家对拍卖的答案是
//! 「放弃」（`tests/common` 那一格），而这里要的正是自己喊出每一声。
//!
//! 钱一律掐**差值**，而起拍价那个数由 `value()` 现算（`economy::estate_value` 的同一条式子）：
//! 改地块数据时先在这里红，而不是让十几条用例各猜一个数。

mod common;

use bevy::app::App;
use bevy::ecs::entity::Entity;
use bevy::prelude::NextState;
use common::*;
use game_core::prelude::{
    ActiveBoard, BidMove, CardAim, Detention, DetentionPlace, GameRules, GameSetup, God, LandId,
    LandStatus, PLAYER_COLORS, Pending, PlayerSetup, Term, TurnPhase,
};

/// 徐家汇（地块 3，绑在 9 号格上）：名义地价 3800、加盖一层 1900。
const LOT: u32 = 3;
/// 王府井（地块 4，11 号格）：拿它当「收不到租就当场破产」那把刀 —— 空地租 160 元，
/// 而死者兜里只摆 50 元。
const LANDLORD_PLOT: u32 = 4;
/// 王府井踩到格（P10d② 顺移：9 → 11，机会格 8 → 10）。
const LANDLORD_TILE: u32 = 11;
/// 死者那点现金：低于这一趟的租，又高到不至于让别的用例误读成「他本来就一分没有」
const BROKE: i64 = 50;

/// 一块地 Lv`level` 此刻的身价（与 `economy::estate_value` 同一条式子：地价 + 已盖的每一层；
/// 级数重判后建筑数 = level，没有 `-1`）。
const fn value(price: i64, upgrade: i64, level: u8) -> i64 {
    price + upgrade * level as i64
}

/// 徐家汇 Lv2（两栋楼）= 3800 + 2×1900 = 7600；一档 10% = 760。
const OPENING: i64 = value(3_800, 1_900, 2);
const STEP: i64 = 760;
/// Lv3 那一栋（清算那两条用它）：3800 + 3×1900 = 9500。
const TALL: i64 = value(3_800, 1_900, 3);

/// 装配一桌 `n` 人（2–4），当前回合玩家是 P0，已经站在掷骰之前、策略 `Hold`。
fn table(n: usize) -> (App, Vec<Entity>) {
    // 第三人往后的名字与座位号成对写死：`PLAYER_COLORS[seat]` 与 `PlayerId(seat)` 必须
    // 是同一位（用例里的 `P1`/`P2`/`P3` 那些流水断言靠的就是这个对应关系）
    const EXTRA: [(&str, usize); 2] = [("王小二", 2), ("钱夫人", 3)];
    let mut setup = GameSetup::default();
    for (name, seat) in EXTRA {
        if seat < n {
            setup.players.push(PlayerSetup {
                name: name.into(),
                color: PLAYER_COLORS[seat],
            });
        }
    }
    let mut app = boot(setup, GameRules::instant(), seed_rolling_first(1));
    app.insert_resource(DecisionPolicy::Hold);
    let first = boot_to_wait_roll(&mut app);
    // 开局那两位神会站在路上某格：附身搬的钱与「买地必败」那条判据都会搅乱这里的每一条
    // 现金断言（清算那批尤其明显）
    clear_gods(&mut app);
    clear_board_gods(&mut app);
    let rows = roster(&mut app);
    let players = rows[..n].iter().map(|(e, ..)| *e).collect();
    assert_eq!(rows[0].0, first, "P0 该是第一个回合玩家");
    (app, players)
}

/// 把一张拍卖卡发到 `player` 手里，按下「→ 某块地」那颗按钮。
///
/// 之后世界是什么样由各用例自己读：这一槌可能开场，也可能被 `can_open` 挡回去而卡还留在
/// 手上 —— 那两类正是这批用例要分开钉的。
fn hammer(app: &mut App, player: Entity, land: u32) {
    let card = item_index(app, "item_auction");
    deal(app, player, card.0);
    play_card(app, player, card.0, Some(CardAim::Land(LandId(land))));
    settle(app, 3);
}

/// 此刻该出价的那一位（用例都先断言他是谁，再替他按按钮）。
fn bidder(app: &App) -> Entity {
    lot_of(app)
        .expect("前置：拍卖正在进行")
        .bidder()
        .expect("名单上没人可问的那一帧不该被读到 —— 那时这一场已经散了")
}

/// 替「此刻该出价的那一位」喊一声**加一档**，返回他这一刻认下的那个数。
fn bid(app: &mut App) -> i64 {
    let amount = lot_of(app).expect("前置：拍卖正在进行").next_amount();
    let player = bidder(app);
    auction_ask(app, player, BidMove::Raise);
    amount
}

/// 流水里那几行 `Auction(`（每开一场一行）。
fn opened(app: &App) -> Vec<String> {
    ledger(app)
        .into_iter()
        .filter(|line| line.starts_with("Auction("))
        .collect()
}

/// 让 P0 死在「踩上乙的王府井、付不出 160 元租」这一步上，`held` 是他名下的地与层数。
///
/// 走的是**真收租**那条路（`rent → debit 失败 → bankrupt`），不是在测试里直接调 `bankrupt`：
/// 待拍的那份料必须在扣不到钱的那一刻记下，绕开它就等于没测那一格。
///
/// `policy` 决定这台机器接下来怎么转：`DeclineAll` 让脚手架替每个被问到的人放弃（于是全部
/// 流标、三块地自己拍完），`Hold` 把按钮交回用例。
///
/// 现金用 `set_cash` 摆到 50 —— 那一条**不发** `CashChanged`，所以这几条用例不做
/// `assert_cash_audit`（起点数对不上是夹具的手，不是世界的谎）。要验钱的去向，断言的是
/// 没被摆过的那两位。
fn liquidate(held: &[(u32, u8)], policy: DecisionPolicy) -> (App, Vec<Entity>) {
    let (mut app, ps) = table(3);
    for (land, level) in held {
        own(&mut app, *land, Some(0), *level, false);
    }
    own(&mut app, LANDLORD_PLOT, Some(1), 1, false);
    set_cash(&mut app, ps[0], BROKE);
    make_current(&mut app, ps[0]);
    // `table` 把第一掷钉成 1 点，所以他得站在 9 号格**前一格**（8 号机会格 —— 只是路过，
    // 落脚的是王府井）
    place(&mut app, ps[0], LANDLORD_TILE - 1);
    set_policy(&mut app, policy);
    roll(&mut app, ps[0]);
    settle(&mut app, 6);
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line.starts_with("Bankrupt(P0")),
        "前置没走到破产，后面每条断言都是空的：{:?}",
        ledger(&app)
    );
    (app, ps)
}

/// 让这台机器自己转完（策略 `DeclineAll` 时每个被问到的人都放弃，于是每块地都流标）。
///
/// 一帧一帧喂而不是 `settle(app, 40)`：机器一空就停，那些多出来的帧会让**下一位玩家**开始
/// 走路、踩到别的东西，而用例要数的是「这台机器开了几场」，不是「这一回合又发生了什么」。
fn drain_machine(app: &mut App) {
    for _ in 0..60 {
        settle(app, 1);
        let auction = auction_of(app);
        if auction.lot.is_none() && auction.standby.is_empty() && auction.liquidation.is_empty() {
            return;
        }
    }
    panic!("这台机器 60 帧还没转完：{:?}", ledger(app));
}

// ===== 拍卖卡这一头 =====

/// 四人一桌的主线：开槌那位不参与，剩下三位一圈一圈喊到见胜负。
///
/// 喊价序列（徐家汇 Lv2 → 起拍 7600、一档 760）：
/// 乙认 7600 → 丙加到 8360 → 丁加到 9120 → 乙放弃 → 丙放弃 → 丁一人领先，当场落槌。
#[test]
fn four_seats_produce_a_price_and_an_owner() {
    let (mut app, ps) = table(4);
    let (opener, second, third, last) = (ps[0], ps[1], ps[2], ps[3]);
    own(&mut app, LOT, Some(1), 2, false); // 乙名下那块 Lv2 的楼

    hammer(&mut app, opener, LOT);

    assert_eq!(
        pending_of(&app),
        Some((second, Pending::Auction)),
        "开场该问座位序里第一位有资格的人：{:?}",
        ledger(&app)
    );
    let lot = lot_of(&app).expect("那一槌该开出场");
    assert_eq!(
        (lot.opening, lot.step, lot.pool.len()),
        (OPENING, STEP, 3),
        "起拍是这块地此刻的身价（连那两层楼），名单上不含开槌的人"
    );
    assert_eq!(lot.seller, Some(second), "「从谁手里拍走」念的是原主");

    assert_eq!(bid(&mut app), OPENING);
    assert_eq!(bidder(&app), third, "喊过价的人留在名单上，但轮到下一位");
    assert_eq!(bid(&mut app), OPENING + STEP);
    assert_eq!(bidder(&app), last);
    assert_eq!(bid(&mut app), OPENING + 2 * STEP);

    // 乙先撤：他被减出名单，而第一声是他认下的 —— 不影响丙丁还在争
    auction_ask(&mut app, second, BidMove::Withdraw);
    assert_eq!(bidder(&app), third);
    auction_ask(&mut app, third, BidMove::Withdraw);

    let final_price = OPENING + 2 * STEP;
    let led = ledger(&app);
    assert!(
        led.contains(&format!("Sold(P3:L{LOT},{final_price},to P0)")),
        "落槌要留下买家、地块、价钱与「钱归开槌的人」：{led:?}"
    );
    assert_eq!(
        plot_state(&mut app, LOT),
        (Some(3), 2, false),
        "地归丁、楼还是那两层（建筑随土地过户，拍卖卡这一头从没摘过它）"
    );
    assert_eq!(
        cash_of(&mut app, last),
        20_000 - final_price,
        "拍款从买家走"
    );
    assert_eq!(
        cash_of(&mut app, opener),
        20_000 + final_price,
        "所得归开槌的人（10-22 拍板）"
    );
    assert_eq!(cash_of(&mut app, second), 20_000, "原主一分也拿不着");
    assert_eq!(cash_of(&mut app, third), 20_000, "中途退出的人不动钱");
    assert_eq!(window_after(&app), None, "散场之后不该还挂着谁的窗口");
    assert_eq!(
        phase(&app),
        TurnPhase::WaitRoll,
        "拍卖卡是掷骰前打的，拍完他还要接着走"
    );
    assert!(hand_of(&app, opener).is_empty(), "卡办成了就该消耗");
    assert_cash_audit(&mut app, 20_000);
}

/// 窗口散场之后读数该是空的（`pending_of` 在拍卖结束后必须交还）。
///
/// 单独一个函数是因为这条断言在四条用例里都要问一遍，而 `pending_of` 的返回是
/// `Option<(Entity, Pending)>` —— 写 `None` 时编译器需要一个类型注解才知道是哪个 `None`。
fn window_after(app: &App) -> Option<(Entity, Pending)> {
    pending_of(app)
}

/// 开槌的人**不在名单上**，而他硬要喊也喊不动（10-22 拍板：使用者不参与拍卖）。
///
/// 两半都要：轮不到他（所以不会被问），以及他主动喊也被拒（否则「不参与」只剩面板上一句话，
/// 而 `AuctionMove` 是谁都能发的）。
#[test]
fn the_founder_is_never_asked_and_his_own_shout_is_refused() {
    let (mut app, ps) = table(3);
    let (opener, owner, other) = (ps[0], ps[1], ps[2]);
    own(&mut app, LOT, Some(1), 2, false);

    hammer(&mut app, opener, LOT);
    let lot = lot_of(&app).expect("开槌的是 P0，名单上就只有乙丙");
    assert_eq!(lot.pool, vec![owner, other], "开槌的人不进名单");

    let before = lot_of(&app);
    auction_ask(&mut app, opener, BidMove::Raise);
    assert_eq!(lot_of(&app), before, "被拒的那一声不该动机器");
    assert!(
        !ledger(&app).iter().any(|line| line.starts_with("Bid(P0")),
        "被拒的那一声连凭据都不该留：{:?}",
        ledger(&app)
    );

    // 乙丙各放弃一次 → 名单空、没人领先 → 流标
    auction_ask(&mut app, owner, BidMove::Withdraw);
    auction_ask(&mut app, other, BidMove::Withdraw);
    let led = ledger(&app);
    assert!(
        led.contains(&format!("Unsold(L{LOT},{OPENING})")),
        "没人喊过价也该有一句流标：{led:?}"
    );
    assert_eq!(plot_state(&mut app, LOT).0, Some(1), "流标不转手");
    assert_eq!(
        cash_of(&mut app, opener),
        20_000,
        "流标时开槌的人一分也拿不着"
    );
    assert_cash_audit(&mut app, 20_000);
}

/// 拍不走的那一块**什么都不留下**：不转手、不动钱、也不动归属。
#[test]
fn a_lot_nobody_bids_on_stays_in_the_original_hand() {
    let (mut app, ps) = table(4);
    own(&mut app, LOT, Some(1), 2, false);

    hammer(&mut app, ps[0], LOT);
    for player in &ps[1..4] {
        auction_ask(&mut app, *player, BidMove::Withdraw);
    }

    assert_eq!(
        plot_state(&mut app, LOT),
        (Some(1), 2, false),
        "三个人都放弃之后，这块地与它上面那两层都该原地不动"
    );
    for player in &ps {
        assert_eq!(cash_of(&mut app, *player), 20_000, "流标不该动任何人的钱包");
    }
    assert!(lot_of(&app).is_none(), "散场之后机器要空下来");
    assert_cash_audit(&mut app, 20_000);
}

/// 加不起这一档的人**喊不出来**，但他随时放得下手 —— 把「不想玩了」挡在门外，
/// 回合机就死停在这一格。这条钉的是拒绝表最后那一格与「放弃永远办得成」的不对称。
#[test]
fn a_broke_player_cannot_raise_but_can_always_walk_out() {
    let (mut app, ps) = table(3);
    let (opener, owner, broke) = (ps[0], ps[1], ps[2]);
    own(&mut app, LOT, Some(1), 2, false);
    set_cash(&mut app, broke, 100);

    hammer(&mut app, opener, LOT);
    bid(&mut app); // 乙认下起拍价，现在该丙
    assert_eq!(bidder(&app), broke, "前置：轮到丙");

    let before = lot_of(&app);
    auction_ask(&mut app, broke, BidMove::Raise);
    assert_eq!(lot_of(&app), before, "100 元喊不动 6270 这一档");
    assert!(
        !ledger(&app).iter().any(|line| line.starts_with("Bid(P2")),
        "被拒的那一声不留凭据：{:?}",
        ledger(&app)
    );

    auction_ask(&mut app, broke, BidMove::Withdraw); // 只剩乙，而他正是领先者 → 落槌
    assert_eq!(plot_state(&mut app, LOT).0, Some(1), "乙那一声就是最终价");
    assert_eq!(cash_of(&mut app, broke), 100, "放弃一分不花");
    assert_eq!(cash_of(&mut app, owner), 20_000 - OPENING);
    assert_eq!(cash_of(&mut app, opener), 20_000 + OPENING);
}

/// 被关着的人不参与拍卖（PLAN §三：坐牢／住院／被绑架者不可参与）。
///
/// 钉的是**开场那份名单**而不是「他被问到时拒掉」：把他留在名单上，拍卖就永远停在一个答不了
/// 的人身上，症状与回合机死锁一模一样。
#[test]
fn a_locked_up_player_is_not_on_the_list() {
    let (mut app, ps) = table(4);
    let (opener, jailed) = (ps[0], ps[2]);
    own(&mut app, LOT, Some(1), 2, false);
    insert(
        &mut app,
        jailed,
        Detention {
            place: DetentionPlace::Hospital,
            term: Term::Turns(3),
        },
    );

    hammer(&mut app, opener, LOT);
    let lot = lot_of(&app).expect("医院里那位该被摘出名单");
    assert_eq!(lot.pool, vec![ps[1], ps[3]], "丙住院，名单上只剩乙与丁");
    assert!(
        opened(&app).iter().any(|line| line.ends_with("2人)")),
        "开场那句也得把「几个人能喊」说对：{:?}",
        opened(&app)
    );
}

/// 那位让「置产必败」的神，在拍卖里读的**就是同一句判据**，而不是另起一条流标规则。
///
/// 这是 P6b② 留的那格「拍卖流标读同一句还是另起一句」的答案：读同一句，且只问在出价那一头
/// —— 于是他永远不会成为赢家，而不必为「已经定下的买卖」再开一次撤销。
/// 他**留在名单上**（面板上他还坐在牌桌上），被挡的只是那一声。
#[test]
fn the_god_that_dooms_acquisitions_never_wins_the_hammer() {
    let (mut app, ps) = table(3);
    let (opener, doomed, other) = (ps[0], ps[1], ps[2]);
    own(&mut app, LOT, Some(1), 2, false); // 衰神名下那块地
    attach_god(&mut app, doomed, God::BigDoom, Some(5));

    hammer(&mut app, opener, LOT);
    assert_eq!(bidder(&app), doomed, "衰神照样坐在名单上 —— 他被挡的是喊价");
    let before = lot_of(&app);
    auction_ask(&mut app, doomed, BidMove::Raise);
    assert_eq!(lot_of(&app), before, "他那一声不该被记下");

    auction_ask(&mut app, doomed, BidMove::Withdraw);
    // 丙被再问一次（只剩他一位而他还不是领先者）—— 他认下起拍价，槌才落
    assert_eq!(bidder(&app), other, "只剩一位时他还得被问一遍");
    assert_eq!(bid(&mut app), OPENING);

    assert_eq!(
        plot_state(&mut app, LOT).0,
        Some(2),
        "地被丙拍走，全程没让衰神置产"
    );
    assert_eq!(cash_of(&mut app, doomed), 20_000);
    assert_eq!(cash_of(&mut app, other), 20_000 - OPENING);
    assert_eq!(
        cash_of(&mut app, opener),
        20_000 + OPENING,
        "所得照样归开槌的人"
    );
}

// ===== 开不了场的那四条（卡不许白扣）=====

/// 押在银行的地进不了拍卖（`OpenDeny::Mortgaged`）。
///
/// 断言的重心不在「地没动」，而在**卡还留在手里**：判据若分叉成「面板说能拍、规则层不办」，
/// 症状就是钱花了事没成（P6d 那两张符立的纪律，在这里第二次被走到）。
#[test]
fn a_pawned_plot_never_makes_it_under_the_hammer() {
    let (mut app, ps) = table(3);
    own(&mut app, LOT, Some(1), 2, true);

    hammer(&mut app, ps[0], LOT);

    assert!(lot_of(&app).is_none(), "押着的地不该开出一场拍卖");
    assert!(opened(&app).is_empty(), "{:?}", ledger(&app));
    assert_eq!(hand_of(&app, ps[0]).len(), 1, "办不成时卡不消耗");
    assert_eq!(plot_state(&mut app, LOT).0, Some(1), "地还在乙名下、还押着");
    assert_cash_audit(&mut app, 20_000);
}

/// 查封中的地同样上不了拍 —— 法院贴着条子的地不许转移（与「不许施工」读同一个状态）。
#[test]
fn a_sealed_plot_stays_off_the_block() {
    let (mut app, ps) = table(3);
    own(&mut app, LOT, Some(1), 2, false);
    stamp(&mut app, LOT, LandStatus::Sealed, 4);

    hammer(&mut app, ps[0], LOT);

    assert!(lot_of(&app).is_none());
    assert_eq!(hand_of(&app, ps[0]).len(), 1, "办不成时卡不消耗");
    assert_eq!(plot_state(&mut app, LOT).0, Some(1));
    assert_cash_audit(&mut app, 20_000);
}

/// 拍自己的地被拒：钱从左口袋到右口袋，而所得还归自己 —— 那不是拍卖是记账。
#[test]
fn nobody_puts_his_own_plot_under_the_hammer() {
    let (mut app, ps) = table(3);
    own(&mut app, LOT, Some(0), 2, false);

    hammer(&mut app, ps[0], LOT);

    assert!(lot_of(&app).is_none());
    assert_eq!(hand_of(&app, ps[0]).len(), 1, "办不成时卡不消耗");
    assert_eq!(plot_state(&mut app, LOT).0, Some(0));
    assert_cash_audit(&mut app, 20_000);
}

/// 场上没有一个能喊价的人时，那一槌落不下去。
///
/// 两人桌 + 原主坐牢：唯一能出价的乙被关着，甲自己是开槌的人 —— 名单空。
/// 「开一场必流标的拍卖」不是拍卖，是一次点击，还会把卡白扣掉。
#[test]
fn no_other_bidders_means_no_auction() {
    let (mut app, ps) = table(2);
    let (opener, owner) = (ps[0], ps[1]);
    own(&mut app, LOT, Some(1), 2, false);
    insert(
        &mut app,
        owner,
        Detention {
            place: DetentionPlace::Jail,
            term: Term::Turns(3),
        },
    );

    hammer(&mut app, opener, LOT);

    assert!(lot_of(&app).is_none(), "{:?}", ledger(&app));
    assert_eq!(hand_of(&app, opener).len(), 1, "办不成时卡不消耗");
    assert_eq!(cash_of(&mut app, opener), 20_000);
    assert_cash_audit(&mut app, 20_000);
}

/// 起拍价是**这块地此刻**的身价：物价 2 倍时它跟着翻倍。
///
/// 这一条挡的是把 7600 写进拍卖机 —— 那等于「任何时候都能按开局价拍走一块地」，
/// 与购地卡那条「按管线报价付钱」是同一句话的两半。
#[test]
fn the_opening_price_follows_the_price_index() {
    let (mut app, ps) = table(3);
    own(&mut app, LOT, Some(1), 2, false);
    set_price_index(&mut app, 2.0);

    hammer(&mut app, ps[0], LOT);
    let lot = lot_of(&app).expect("翻倍也要开得了场");
    assert_eq!(lot.opening, 2 * OPENING, "地价与楼都按今天的价算");
    assert_eq!(lot.step, 2 * STEP, "一档也跟着身价走");
}

// ===== 破产变卖 =====

/// 死者名下**每一块**地都要还回市场，而挂拍的只有三块（10-22 拍板：随机选 3 处逐项拍卖，
/// 其余归无主）。四块地摆在他名下，`liquidation_lots` 是三 —— 这条钉的是那个数真被读了。
///
/// 「哪三块」由第十三条子流决定，所以这里只数场次、不猜顺序；而层数**逐块**断言（1、2、3、2
/// 各不相同）：归无主的含义是「主人没了」，不是「房子也没了」（10-22 实报），
/// 于是这块表同时钉住那一半 —— 四块地在 `DeclineAll` 下全部流标，读回来的层数只能是
/// 他们生前那一手。
#[test]
fn only_three_of_the_dead_plots_go_under_the_hammer() {
    let (mut app, _ps) = liquidate(
        &[(0, 1), (1, 2), (3, 3), (5, 2)],
        DecisionPolicy::DeclineAll,
    );
    drain_machine(&mut app);

    let opened = opened(&app);
    assert_eq!(
        opened.len(),
        3,
        "四处地里只该有三处挂拍（`liquidation_lots`）：{opened:?} / {:?}",
        ledger(&app)
    );
    assert!(
        liquidation_of(&app).is_empty(),
        "那位死者只有一处清算，做完就该空"
    );
    assert!(lot_of(&app).is_none(), "三场都拍完了，机器要空下来");
    for (land, level) in [(0u32, 1u8), (1, 2), (3, 3), (5, 2)] {
        assert_eq!(
            plot_state(&mut app, land),
            (None, level, false),
            "{land} 号地该回到市场：无主、没债，而楼还是他生前那一手（Lv{level}）"
        );
    }
}

/// 建筑随土地过户 —— 而这里没有任何「还回去」的动作要做（10-22 实报之后）。
///
/// 清算场那块地的楼从头到尾都立在世界上：破产只摘归属与抵押（`settle::Books::bankrupt`），
/// 起拍价读的是世界上那三层（3800 + 3×1900 = 9500），成交只是换个名字。
/// 第一场实现不是这样 —— 那里先清零、由队列把「被摘之前」那个数带进来、成交那一刻还回去，
/// 而那一整趟是在给自己造的毛病打补丁。
#[test]
fn a_building_changes_hands_with_the_plot_it_stands_on() {
    let (mut app, ps) = liquidate(&[(LOT, 3)], DecisionPolicy::Hold);

    let lot = lot_of(&app).expect("死者那块地该挂着拍");
    assert_eq!(lot.opening, TALL, "起拍算的是那三层楼的身价");
    assert_eq!(lot.seller, Some(ps[0]), "「从谁手里拍走」念的是死者");
    assert_eq!(bidder(&app), ps[1]);
    assert!(
        opened(&app).iter().any(|line| line.contains("by bank")),
        "开场那句要说清这是清算、不是某个玩家在卖地：{:?}",
        opened(&app)
    );

    assert_eq!(bid(&mut app), TALL); // 乙认下起拍价 → 该丙
    assert_eq!(bidder(&app), ps[2]);
    auction_ask(&mut app, ps[2], BidMove::Withdraw); // 丙放弃 → 乙一人领先，落槌

    assert_eq!(
        plot_state(&mut app, LOT),
        (Some(1), 3, false),
        "买家拿到的是一栋三层的楼"
    );
    assert_eq!(cash_of(&mut app, ps[1]), 20_000 - TALL);
    assert_eq!(cash_of(&mut app, ps[2]), 20_000, "丙没付钱");
    assert!(
        !paid_him(&app, ps[1], TALL),
        "也没人收到那笔钱 —— 变卖所得归银行，不另立债权账本"
    );
    assert_eq!(
        phase(&app),
        TurnPhase::TurnEnd,
        "清算发生在落格结算里，拍完这一回合就该收尾"
    );
}

/// 清算场流标：地回到市场，而**楼照样立着** —— 与其余归无主的那几块同一种下场（10-22 实报）。
///
/// 「不转手」这一半从没变过；变的是另一半。第一场实现断的是「连楼也不许留下」，
/// 因为那里在破产那一刻把层数清了，流标时地上只剩 Lv0 —— 那栋楼只能靠成交那一刻还回去。
/// 既然楼从来不必摘，流标那条出口就只是「归属没换」，而 `tests/mortgage.rs`
/// 那条「人走了地干净」也已改成「人走了、债清了、楼还在」。
#[test]
fn an_unsold_liquidation_lot_stays_on_the_market_with_its_house_standing() {
    let (mut app, ps) = liquidate(&[(LOT, 3)], DecisionPolicy::Hold);

    for seat in [ps[1], ps[2]] {
        auction_ask(&mut app, seat, BidMove::Withdraw);
    }

    assert_eq!(
        plot_state(&mut app, LOT),
        (None, 3, false),
        "人走了、没人认价：无主、没抵押，而那三层楼留在地上"
    );
    assert!(
        ledger(&app)
            .iter()
            .any(|line| line == &format!("Unsold(L{LOT},{TALL})")),
        "流标那句念得出的是那块地**连楼**的身价：{:?}",
        ledger(&app)
    );
    for player in &ps[1..] {
        assert_eq!(cash_of(&mut app, *player), 20_000, "没人付钱");
    }
    assert_eq!(phase(&app), TurnPhase::TurnEnd, "机器空了，回合该接着走");
}

/// 名下没有地的破产者**不该惊动这台机器**。
///
/// 挡的是「空转一圈再 `warn!` 一句开不成场」：那一圈里没有一块地要拍，而玩家听见的是假消息
/// —— 症状与真缺陷一模一样（`Books::bankrupt` 里那句 `if !held.is_empty()` 的全部意义）。
#[test]
fn a_broker_without_lands_holds_no_auction() {
    let (app, _ps) = liquidate(&[], DecisionPolicy::Hold);

    assert!(opened(&app).is_empty(), "{:?}", ledger(&app));
    assert!(liquidation_of(&app).is_empty());
    assert!(lot_of(&app).is_none());
    assert_ne!(phase(&app), TurnPhase::Over, "{:?}", ledger(&app));
}

/// 一台机器散场之后，下一局必须是空的（跨局残留那一族，B5 冒过、P8 的奖池也钉过）。
///
/// 清的是整台：`lot` 里那块地、`standby` 里排着的队、`liquidation` 里那位死者的料、
/// `requested` 里那一槌、`resume` 里那个阶段。留着 `resume` 尤其贵 —— 新局第一帧它就替
/// 上一局把回合机推走，而那一刻棋盘才刚刚铺好。
#[test]
fn a_rebooted_world_starts_with_an_idle_machine() {
    let (mut app, ps) = table(2);
    own(&mut app, LOT, Some(1), 2, false);
    hammer(&mut app, ps[0], LOT);
    assert!(lot_of(&app).is_some(), "前置：这一局确实开过一场");

    app.world_mut().remove_resource::<ActiveBoard>();
    app.world_mut()
        .resource_mut::<NextState<TurnPhase>>()
        .set(TurnPhase::Boot);
    assert!(run_until_world(&mut app), "世界没能重装");

    let fresh = auction_of(&app);
    assert!(fresh.lot.is_none(), "新局不该接着拍上一局那块地");
    assert!(fresh.standby.is_empty() && fresh.liquidation.is_empty());
    assert!(
        fresh.requested.is_none() && fresh.resume.is_none(),
        "跨局残留：{fresh:?}"
    );
}
