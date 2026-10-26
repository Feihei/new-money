# 《暴富》架构与技术事实（原《Godot → Bevy 移植方案》）

> 源项目：`E:\play\Godot\new-money`（Godot 4.7 + GDScript，进度到 Phase 1.5 / Phase 2 之间）
> 目标：`E:\play\coding\new-money-bevy`（Bevy 0.20，**彻底 ECS 化**）
> 文档版本：v1.1 ｜ 2026-10-13 起，2026-10-16 改名并瘦身
>
> **改名原因**：移植那条线收口之后，这份文档装的东西已经换了一批。它现在是**唯一被源码注释引用的技术事实文档** ——
> 分层红线（§4.6）、道路不建实体（§4.2）、27 个 Bevy 0.20 坑（§6.1）、缺陷清单（§三）、表现层待做（§七）、存档方案（§九）。
> 叫「移植方案」会让人以为它随移植一起作废；恰恰相反，**动 Bevy API 之前必须查它**。
>
> **瘦身**：纯历史（§五 映射表、§十 阶段与验收记录）移入 [`docs/archive/migration-b0-b5b.md`](../docs/archive/migration-b0-b5b.md)，原位留指针、章节号不变。
>
> 本文所有 Godot 侧结论均带 `file:line` 出处；所有 Bevy 侧 API 名已在 0.20 官方文档/迁移指南核实，标 ⚠️ 的表示落地时需再确认。

> **参照范围已收窄（2026-10-15，B5 之后拍板）**：移植这条线在 B5 追平，**Bevy 侧自此为主线**，不再回头比对 Godot。
> 本文的 Godot 出处只保留两个用途：① 前期 ADR/规格与数值出处；② §三 缺陷清单的反例（哪些行为是错的）。
> **画面、UI 排版、交互路径一律不再对齐**——§七 与已归档的映射表里凡是「照 Godot 的样子摆」的约束，从此视为历史说明而非验收项；
> §五 / §5.1 映射表**停止逐文件维护**，10-16 整块搬入 [`docs/archive/migration-b0-b5b.md`](../docs/archive/migration-b0-b5b.md)。设计上新（如银行/证券面板）按本项目自己的判断走，不必找 Godot 对等物。

---

## 一、结论

**可行，且这个项目恰好是适合换引擎的少数形态。** 三个理由：

1. **逻辑层已经和渲染层解耦了**。`scripts/core/` 2041 行里 Godot 依赖只剩三类：`EventBus.emit()`、`Vector2/3`、`Resource`+`@export`。没有 `get_node`、没有场景树操作、没有物理（`project.godot` 里 2D/3D 物理引擎都是 `Dummy`）。这意味着核心规则可以近乎逐行翻译成 Rust，而不是"重写一遍设计"。
2. **零美术资产**。`assets/` 目录是空的，棋盘格、建筑、棋子全部是代码生成的 `BoxMesh/SphereMesh/CylinderMesh` + `unshaded` 材质（见 §七）。移植过程不包含任何资产管线迁移成本。
3. **换引擎的动因（测试）在本项目收益最大**。当前 2403 行测试必须靠 `godot --headless -s addons/gut/gut_cmdln.gd` 起一个引擎进程才能跑；迁移后是 `cargo test`，且回合状态机天然用 `App::update()` 步进，不需要渲染。

**代价要说清**：约 2450 行表现层（`scripts/board/` 704 + `scripts/ui/` 1208 + `scripts/entities/` 539）是**纯重写**，其中 UI 是最大成本项——Godot 的 `Control` 布局/`CanvasLayer`/`Tween`/`Label3D`/隐式 CJK 字体回退在 Bevy 都要显式重建，且 Bevy 没有场景编辑器可视化摆放，UI 排版靠代码迭代。预计表现层工时约等于逻辑层移植的 2 倍。

---

## 二、源项目盘点

| 模块 | 行数 | 内容 | 移植方式 |
|---|---|---|---|
| `scripts/core/` | 2041 | 16 个文件：`turn_manager`(492) `board_data`(336) `stock_market`(200) `property_system`(186) `property_registry`(128) `player_data`(105) `bank_system`(105) `card_deck`(88) `card_effects`(81) `board_tile`(72) `land_plot`(62) `tile_type`(49) `stock_data`(41) `card_data`(24) `dice`(26) `board_branch`(46) | **翻译**为 ECS system + component |
| `tests/` | 2403 | 16 个 GUT 文件，单元 + 集成 | **翻译**为 `cargo test`（模板见 §八） |
| `scripts/ui/` + `board/` + `entities/` | 2451 | 等轴测渲染、相机、行走动画、HUD、4 个决策对话框、主菜单/暂停菜单 | **重写** |
| `scripts/autoload/` | 341 | `event_bus`(61，24 个信号) `game_state`(89) `data_manager`(39) `save_manager`(40，**全是 TODO 占位**) `scene_manager`(41) `audio_manager`(占位) | **映射**为 Message / Resource / Asset |
| `data/*.json` | — | `boards/prototype.json`（12 主环格 + jail/hospital 分支）、`cards/{chance,fate}_cards.json`、`stocks/stocks.json` | **原样复用** |
| `docs/` | — | `DEV_PLAN.md`、`ADR-001-board-topology.md`（已 Accepted）、`board-data-schema.md`、`GLOSSARY-board.md`、`PHASE_1_5_PLAN.md` | **原样复用**，是移植的最有价值资产 |
| `tools/` | — | `validate_board.py`、`render_board_svg.py`、`selftest.gd` | Python 那两个与引擎无关，**直接搬过去继续用** |

一个必须提前指出的事实：**源项目自己已经偏离了文档**。`docs/PHASE_1_5_PLAN.md` 写的正交俯角 35.264°、`PathFollow3D.progress_ratio` 驱动移动、`CAM_SIZE=25`，实际 `board.gd:30-31` 是 `CAM_OFFSET=(8,16,8)`、`CAM_SIZE=8.0`，且 `player_token.gd:5` 明确注释"不再依赖 PathFollow3D/Curve3D"，移动走世界坐标直线补间。**移植时以代码为准，文档只作设计意图参考**——否则会照着过期的规格实现。

---

## 三、移植过程中顺手修掉的真实缺陷

这些是我读代码时确认的 bug/隐患，不是猜测。逐条在 Rust 侧从结构上消除，比在 Godot 里补更省事：

| # | 位置 | 问题 | ECS 后为何不会再发生 |
|---|---|---|---|
| 1 | `bank_system.gd:99-105` | `_calc_max_loan` 拿 `registry.get_owned_properties()` 返回的 **key** 去调 `board.get_tile(tile_idx)`。新格式下 key 是字符串 `"L"+land_id`（`property_registry.gd:19-24`），而 `get_tile(index:int)` 要的是主环整数下标 → 返回 null → **贷款上限恒为 0，贷款功能实际不可用** | `LandId` 是 newtype，`TileIndex` 另一个 newtype，`HashMap<LandId,Entity>` 与 `HashMap<TileIndex,Entity>` 不可能混用；这条错误在编译期即失败 |
| 2 | `turn_manager.gd:436` → `:169/184/189` | `turn_started` **每个回合发两次**：`_advance_and_start` 先 emit 一次，紧接着 `_start_turn()` 内部又 emit 一次。监听它的 UI 会重复刷新 | `TurnStarted` 是 system 副作用，状态用 `OnEnter(TurnPhase::TurnStart)` 唯一入口触发；不存在"两处都记得 emit"的问题 |
| 3 | `card_effects.gd:34-41` | `deduct_cash` 失败时只返回文案"现金不足，破产！"，**并没有调用 `go_bankrupt()`**（对比 `property_system.gd:92-95` 收租路径是真的破产）→ 玩家状态与 UI 文案不一致 | 扣款返回 `Result<(),InsufficientFunds>`，`?` 后必须由 `Insolvency` system 显式处理；文案由消息驱动，不由结算函数拼字符串 |
| 4 | `stock_market.gd:118,148` | 买入/卖出时 `emit stock_price_changed(stock_id, price, s.current_price)`，价格根本没变（old==new），语义错误；真正的交易事件缺失 | 拆成 `StockTraded` 与 `StockPriceChanged` 两个不同 Message 结构体 |
| 5 | `stock_market.gd:110` | `buy()` 现金不足时递归调用自身，无深度保护 | 改为一次算 `can_afford` 的循环/直接式 |
| 6 | `dice.gd:19-20` + `card_deck.gd:53` | 骰子用全局 `randi_range`，洗牌用 `_rng.randomize()` → **不可复现**。`roll_seeded()` 只是个 `assert` 后原样返回的假种子函数，仅 `test_dice.gd:18-20` 在用 | `Rng` 作为 Resource 持有 `SeedableRng`（`ChaCha8Rng`）+ `GameSeed` resource。顺带解锁：回放、失败复现、将来 lockstep 联机（`DEV_PLAN.md` §6.2.7 的诉求） |
| 7 | `board.gd:704` 发 `dice_rolled` | **表现层驱动游戏逻辑**：骰子点数由棋盘视图发出，`TurnManager` 监听（`turn_manager.gd:96`）。后果是逻辑无法脱离渲染运行，headless 测试根本掷不了骰 | `roll_dice_system` 属于 core（`in_state(WaitRoll)` + `RequestRoll` 消息触发），render 只订阅结果 |
| 8 | `player_data.gd:15,30,35,85,96` `turn_manager.gd:158,249-265` | 状态是裸字符串 `"active"/"jailed"/"bankrupt"`，格子类型是裸字符串 `"property"/"stock"/...`，比较散落各处（`board_tile.gd:72` 还有一份硬编码类型列表和 `tile_type.gd:31-35` 重复） | `JailSentence`/`Bankrupt` 是组件存在性（**已修**，见下方 B2b）；格子类型那半 **P1 已修**（见下面的收口记录） |
| 9 | `board_data.gd:258-304` `_compute_legacy_positions`（47 行 Curve3D 烘焙）+ `board_data.gd:6-9` 双格式分支 | 兼容"旧格式 `tiles[]`+`position:float`"，但 `data/boards/` 下**只剩 `prototype.json`**（新格式），旧格式棋盘文件不存在 | **不移植旧格式**，连带删除 `BoardData.has_coords`、`_legacy_positions`、`property_registry` 的 `set_owner/get_owner` 兼容层（`:73-106`，54 行）。这是移植带来的净删减 |
| 10 | `board_data.gd:63-67` `passes_start`、`game_state.gd:20` `inflation_index`、`scripts/entities/color_circle.gd`、`card_notice` 的 `HSeparator` 主题 | 死代码：`passes_start` 全项目无调用；`inflation_index` 只在 `test_game_state.gd:23-28` 被赋值和断言，不进任何公式；`color_circle.gd` 未被任何场景/脚本引用 | 不搬 |
| 11 | `game_state.gd:45` | 新开对局却 `EventBus.game_saved.emit(game_id)` | 消息语义对齐后自然消失 |
| 12 | `player_data.gd:98` + `game_state.gd:41` + `turn_manager.gd:39` | 起始资金 20000 硬编码三处（`START_CASH` 常量存在但 `reset()` 不走它） | 唯一来源 `GameRules` resource（可从 JSON 覆盖） |
| 13 | `player_data.gd:84-91` + `property_registry.gd` | 破产只清玩家自己的 `properties` 列表，registry 里的业主仍写着死者 → 租金永远打给破产者、同色组永远卡住别人 | 归属是 Land 实体上的 `Owner` 组件，`Books::bankrupt` 一次遍历把名下地块放回市场 |
| 14 | `property_system.gd:151-161` | `_owns_full_group` 用空串匹配得到「无格子能匹配 → 全都满足」的隐式 `true`，坏数据上白送垄断与升级权 | `group: Option<String>` 是类型而不是字符串，`None` 显式返回 `false` |

> 第 9 条值得单独强调：**移植不是 1:1 搬运，删掉的兼容代码约占 core 的 15%**。

> **截至 B2b（10-13）已修并有回归网的**：#2（`turn_flow.rs::assert_turns_balanced`）、#6（`same_seed_replays_the_same_match`）、#7（`requests_from_somebody_else_are_ignored` + 骰子在 core 的 `roll_dice_system` 里）、#8 的玩家状态部分（`JailSentence`/`Bankrupt` 是组件存在性，见 `jail_skips_two_turns_then_releases`）、#12（`world_boot.rs::setup_and_rules_are_the_only_sources`）。

> **#8 已收口（P1，10-16）**：那条「新增格类型漏写分支直接编译失败」的承诺以前不成立 —— 落格分派是 chain 里四个域各自 `filter` 自己关心的 kind，没有一处穷尽 `match`，`Lottery`/`GodDeity`/`Neutral`/`Transport`/`Hospital` 踩上去静默无事。现在 `landing::domain_of` 是**唯一一张** `TileKind → LandingDomain` 的表，`match` 里不许出现 `_`，五个域的 system 改读 `Landed::domain` 而不是各自的 `kind` 判据；没实现的格归 `LandingDomain::Unimplemented(kind)`，踩上去 `warn!` 一句，「什么都不发生」从此是可被告知的。顺手清掉的两处：`TileKind::is_card()` 删了（分派表是它唯一的用处，留着就是第二种问法）；`TileKind::is_detention()` 在 **P4④（10-20）** 也删了 —— 它断言的是「踩到某格会被关起来」，而这条规则在同一轮里被拍板否掉（天数只来自卡面），留着它就是一把指向错误规则的枪；`TileData.turns/fine` 那份 Godot 残留死数据随它一起删。P4③ 起 `Jail`/`Hospital` 两格归 `LandingDomain::Bail`（主环上的拘留场所 = 保释入口，演示盘把分支挂在地产接入点上、走 `decision_system` 的后置窗口）。覆盖它的测试：`landing.rs::every_kind_is_routed_to_exactly_one_domain` + `the_todo_list_is_explicit`（清单与表同宽，待办少一格就红）、`coupon.rs::an_unimplemented_tile_still_closes_the_turn`（待办格不发钱、不挂决策、但回合收得掉）。**「漏写分支编译不过」这一条用测试证不了，所以当场量过一次**：临时给 `TileKind` 加一个 `TempProbe` 变体 → `cargo check -p game_core` 报 `E0004: non-exhaustive patterns: TileKind::TempProbe not covered`，落点正是 `landing.rs` 的 `domain_of`，随即移除。表里没有 `_` 分支，这个事实就一直成立。

> **B3（10-14）修完的四条**：
> - **#1** 贷款额度 —— `economy::calc_max_loan` 的入参是 `LandView`（内部 `LandId`），`TileIndex` 喂进来编译不过；端到端回归 `settle.rs::loan_limit_comes_from_owned_lands`（顶格 6000 贷得到、再加 1 块拒）。
> - **#3** 扣款失败必须真破产 —— `Wallet` 只能经 `Books::credit`/`debit` 改动，付不起时 `charge` 直接走 `bankrupt`。回归：`a_fine_you_cannot_pay_bankrupts_you_for_real`（`CouldNotPay` 与 `PlayerBankrupt` 同帧成对、`Bankrupt` 组件真的挂上）。
> - **#4** 股价事件 —— 拆成 `StockPriceChanged{stock,old,new}`（tick 真的动了才发）与 `StockTraded{player,stock,shares,price,side}`；回归 `stock_visit_ticks_every_price_once`。
> - **#5** 买入递归 —— `economy::buy_plan` 一次算清可买股数；另有扫表性质 `buy_plan_never_asks_for_more_than_the_wallet_has`（39 个价格 × 6 档现金，花费绝不超过余额）。
>
> **#9/#10/#11 属后续**：旧格式本来就没搬；`passes_start`/`inflation_index`/`color_circle.gd` 未搬（`inflation_index` 那条 **10-17 由 PLAN P2③ 真正实现**：`GameSession::price_index` + `charge::stage`，Godot 那个死字段现在在 Rust 侧有公式、有豁免表、有回归 `tests/price_index.rs`）。#11 的 `game_saved` 误发随存档一起处理 —— 原 B7，**现 PLAN P15**，仍开着。

> **B3 过程里新发现、一并修掉的 Godot 错误（编号续在 12 之后）**：
> - **#13** `player_data.gd:84-91` 破产时只清玩家自己的 `properties` 列表，`PropertyRegistry` 里的业主仍写着死者 → 租金永远打给破产者、同色组永远卡住别人的垄断与升级。`Books::bankrupt` 把名下地块一并放回市场（回归 `cant_pay_rent_bankrupts_the_payer_and_frees_their_lands`）。
> - **#14** `property_system.gd:151-161` 的 `_owns_full_group` 用空串匹配会得到「没有任何格子能匹配 → 全都满足」的隐式 `true`，坏数据上等于白送垄断与升级权。Rust 侧无 `group` 的地块明确返回 `false`（`economy::owns_full_group`，回归 `groupless_land_gets_no_free_monopoly`）。
>
> **B3 记录的数据缺口 —— ✅ 已定（B5b，10-15）**：`prototype.json` 的 12 格里**没有 `stock` 格**，而 `stock_market.gd:68` 的 `tick()` 只在踩到证券格时调用 —— 照搬之下整局行情一动不动、分红永不发放。市场逻辑已完整实现并有测试（测试里临时把某格改成 `stock` 类型跑这条路径）。**结论是不加证券格**：行情改挂「每回合结束 tick」、分红改挂日历，证券做成掷骰前随时可下的单而非一格要踩的东西。两条分叉的理由与验收见 [归档的 §10.2](../docs/archive/migration-b0-b5b.md)。

> **B5 核对过、决定照搬的源游戏规则怪癖**（不是移植错误，改它等于改玩法，需要单独拍板）：
> - **Lv0 空地已经收 `rent[0]`，所以「升级到 Lv1」租金增幅为 0**。`property_system.gd:59-61` 先 `if level < 1: level = 1` 再取 `rent_array[level - 1]`，即空地与一级房收同一档租；升级面板如实显示「+$0」（截图核对过）。要改就得同时改 `calculate_rent` 与租金表语义。
>   **⚠️ 这条的「照搬」前提已经作废（10-16）**：当时比的是 Godot，而玩法基线后来换成大富翁 4 代 —— 4 代住宅是 空地→平房→店铺→商场→商业大楼→摩天大楼（**加盖 5 次，0→5 共六态**），我们现在是 Lv0 也收租 + `rent[6]` 表 + Lv0 视作 Lv1，等于两套级数混在一起，而且「空地＝有房子的 Lv1」这个隐式钳位本身就是玩家读不懂的行为。归属 [`PLAN.md`](./PLAN.md) 的 **P10**（建筑名与级数），改法要与 `building_kind` 一起定，别单独动 `calculate_rent`。**✅ 10-20 拍板确认：按 4 代重判**，所以 `calculate_rent` 里那条 Lv0 钳位与 `rent[6]` 表的语义在 P10 之前**不要再单独修** —— 一次改动要同时覆盖级数名、租金表长度与钳位三处。
> - **终局提示只有一行「游戏结束」是 Godot 的原样行为**：`board.gd` 的阶段旁白会盖掉胜者那句。Rust 侧反过来做了 —— `Over` 阶段不写旁白，让 `GameOver` 的「胜者：X！」留在屏幕上。这是刻意分叉，已记在 §5.1。**这条已收口，无需回头。**

---

## 四、目标架构（彻底 ECS 化）

### 4.1 crate 布局

```
new-money-bevy/
├─ Cargo.toml                  # workspace；[workspace.dependencies] 精确 =0.20.0
├─ assets/                     # 从 Godot 原样拷来，文件名不变
│  └─ data/{boards,cards,stocks}/*.json
├─ crates/
│  ├─ game_core/               # 全 ECS，但是「无渲染 App」
│  │  │                        #   bevy = { default-features = false } → 只有 ecs/app/asset/math/time
│  │  ├─ src/msg.rs            # 所有 Message 定义（EventBus 的对等物）
│  │  ├─ src/board/            # BoardDef(serde Asset) + 实体化 + 拓扑查询
│  │  ├─ src/turn/             # TurnPhase States + 回合 system
│  │  ├─ src/economy/          # land/property/bank/stock
│  │  ├─ src/card/             # deck Asset + effect 求解
│  │  ├─ src/player/           # Player 实体与组件
│  │  └─ src/rng.rs            # Rng + GameSeed
│  └─ game_app/                # bin：default plugins、渲染、UI、字体、音频、存档、场景流转
├─ crates/game_core/tests/     # headless 集成测试（App + app.update()）
└─ .agents/                    # agent 工作文档：本方案 + ADR-001 + schema + 术语表（Godot 原样副本）
```

**为什么选"彻底 ECS"还要拆 `game_core`**：这两件事不冲突。"彻底 ECS" 指的是**规则用 component + system 表达，而不是 OOP 的 `PropertySystem`/`StockMarket` 对象**；拆 crate 指的是**这一层只链接 `bevy` 的无渲染特性子集**。收益：`cargo test -p game_core` 编译量小、跑得快、不需要窗口；渲染/UI 侧的 0.20→0.21 API 破坏不会波及规则层。**已核实（B0）**：`game_core` 用 `bevy = { default-features = false, features = ["default_app"] }` 正好拿到 `async_executor + bevy_asset + bevy_log + bevy_state + reflect_auto_register`，不含 winit/窗口/渲染；`default_platform` 才是拉进 winit 的那一层。成员 crate 要覆盖 workspace 依赖的 `default-features`，**必须用 edition 2024**（否则 cargo 只是警告一句然后忽略你的设置）。

### 4.2 世界设计：实体 / 组件

| 实体 | 数量 | 组件 |
|---|---|---|
| 主环道路格 | N（prototype 12，正式 40） | **不建实体**（B2a 修正，见下）：`TileIndex`/`TileKind`/`GridPos` 全部经 `BoardDef` 查询取得 |
| 分支死胡同格 | 每分支若干 | `BranchTile{junction:TileIndex, order:u8, kind}` `GridPos` —— 对应 `board_branch.gd:11` "从靠近接入点 → 末端"有序链 |
| 路旁土地 | 若干 | `LandId(u32)` `GridPos` `LandSize(u8=1\|2)` `LandDef`(Arc：group/land_type/price/rent[6]/upgrade_cost) `OwnerAddr(Option<PlayerId>)` `BuildingLevel(u8)` `Mortgaged{proceeds:i64}`（**每块地都挂着，`proceeds>0` 即押着** —— P3d③ 之前它是「存在即抵押」的单位标记；改成带本金的值有两个理由：赎回要按「当初到手那笔 + 手续费」算，而标记存不下那个数；且 `Commands` 的延迟写会让同一帧内两条押同一块地的请求都读到「还没押」，钱付两遍） |
| 玩家 | 2–4 | `PlayerId(u32)` `PlayerName` `PlayerColor` `Wallet{cash:i64}` `TokenPosition{tile:TileIndex, off_board:Option<GridPos>}` `Detention{place:DetentionPlace,turns_left}`（P4① 拘留统一模型：**监狱／医院／出国三处去处共用一个组件**，`DetentionPlace` 那两条映射 `branch`（棋盘上那条死胡同）与 `label`/`back_verb`（人话）是它们的全部差别；`Away` 没有分支 —— 那个地方不在地图上，所以棋子留在接入点）`StaySentence{turns_left}`（停留卡，**P4④ 起是字面口径**：写 N 停 N 个回合，且不再自动跳回合 —— 轮到他的那颗「掷骰」翻成「停留」，点一下才消耗，`take_stay_system` 收这条意图。它与拘留是两条独立计时：被拘留的人进不了 `WaitRoll`，所以拘留那几回合不消耗停留的计数；不合成一个「处境」组件的理由见 `player.rs` 那段 —— 服刑的人被挪进死胡同格且 P4 起丧失收租权，停留的人还在自己踩到的那一格上、地照收租）`Heading`（`Forward \| Backward`，P3d④b 转向卡翻的就是这一位 —— **环只有一份**，方向是玩家的状态，见 ADR-001 行为规则 3）`StockHoldings(HashMap<StockId,u32>)` `BankBalance{deposit,loan:i64}` `Controller`（`Human \| Ai{difficulty}`）`Bankrupt`（marker，取代 `status=="bankrupt"` 判断） |
| 棋子（视觉） | 每玩家 1 | `TokenVisual{player:PlayerId}` `Transform` `WalkQueue{path,step_clock}` |
| 卡牌堆 | 2（机会/命运） | `Deck{kind, pile:Vec<CardId>, discard:Vec<CardId>}`，卡定义走 `Assets<CardDef>` |
| 股票 | 每只 1 | `StockId` `StockDef`(Arc：base/floor/ceiling/volatility/outstanding) `StockPrice{current:f64}` `PriceHistory(VecDeque<f64>，MAX 50)` |
| 相机/灯光/UI | — | 归 `game_app`，core 里没有 |

**关键取舍**：`PropertyRegistry`（`property_registry.gd`，128 行的弱类型字典）整个消失——owner/level/mortgaged 直接变成 Land 实体上的组件。同色组垄断判定（`property_system.gd:151-161`）、组内等级差不超过 1（`:164-178`）改为 `Query<&LandDef, With<OwnerAddr>>` 上的聚合，不再每次线性扫 `get_tiles_by_type`。

**B2a 落地修正（10-13）**：**道路格不建实体**，只有土地建。理由是 `BoardDef` 的拓扑查询（`tile()/destination()/land_for_tile()/branch_end()`）已经是道路格的完整接口，而道路格没有任何运行时可变状态 —— 再复制一份 component 就多出第二个真相来源，改一处忘一处。表现层为了渲染/拾取自己 spawn 的网格实体归 `game_app` 管，规则层不认识它。同理 `OwnerAddr` 的实际名字是 `Owner(pub Option<PlayerId>)`，`LandDef` 不重复挂在实体上（价格/租金查 `active.def.land(id)`）。

### 4.3 Resource（取代 autoload 单例）

| Godot 单例 | Bevy Resource |
|---|---|
| `GameState.game_id/turn_count/inflation_index` | `GameSession{id, turn_count, seed}` |
| `GameState.board` | `ActiveBoard{def:BoardDef, lands:HashMap<LandId,Entity>}` —— B2a 落地形态：`def` 是加载后的一次性快照，规则 system 直接 `Res<ActiveBoard>` 就能拿拓扑，headless 测试也只插这一个 resource |
| `GameState.players` | 不需要 —— 玩家就是实体，用 `Query<&Wallet, With<PlayerId>>` |
| `TurnManager._players/_current_player_index/_pending_*` | `TurnOrder{active:Vec<PlayerId>, cursor:usize}` + `PendingDecision{player, kind}` |
| `TurnManager.property_system/stock_market/bank_system/...`（`:59-69` 7 个实例字段） | 全部消失，变成 system + 组件数据 |
| `GameRules`（新）：`START_CASH/JAIL_TURNS/MAX_LEVEL=6/DIVIDEND_INTERVAL=5/DIVIDEND_RATE/BASE_DRIFT/DEPOSIT_RATE/LOAN_RATE/MAX_LOAN_RATIO` | 一个 `GameRules` resource，值从 `data/rules.json` 读（现散落在 `player_data.gd:13` `turn_manager.gd:38-39` `stock_market.gd:14-17` `bank_system.gd:8-10`）。**B2a 已落 `start_cash`**，缺陷 #12 就此消除（起始现金只剩这一处）；其余常量 **B3 已全落**（`max_building_level`/`monopoly_rent_multiplier`/`deposit_rate`/`loan_rate`/`max_loan_ratio`/`stock_base_drift`/`dividend_interval`/`dividend_rate`/`max_price_history`）。**`rules.json` 暂不做**（2026-10-14 决定）：单一来源由 resource 本身达成，外部化等真有策划调参需求时再加 |
| `DataManager`（`data_manager.gd`，手写 `_cache`） | `AssetServer` + `Assets<BoardDef>`/`Assets<CardDef>`/`Assets<StockDef>`，缓存与热重载免费得到 |
| `SaveManager`（**未实现**，`save_manager.gd:17-26` 是 TODO） | 见 §九 |
| `AudioManager`（未实现） | `bevy_audio`，`game_app` 侧 |

### 4.4 Message（取代 EventBus 的 24 个信号）

0.20 形态：`#[derive(Message)]` 结构体 + `app.add_message::<T>()` + **`MessageWriter<T>` 写**（⚠️ 不是 `Commands::send_message`，0.20 的 `Commands` 上没有这个方法）+ `MessageReader<T>` 读（存储是 `Messages<T>`；`MessageReader` 带 `'w`/`'s` 两个生命周期）。观察者推送是另一套 `#[derive(Event)]` + `trigger`。按 `event_bus.gd:14-61` 的九域分组翻译：

```
流程   TurnStarted{player} TurnEnded{player} RequestRoll{player} DiceRolled{player,steps}
移动   WalkStarted{player,from,to,steps} TileStepped{player,tile} PlayerArrived{player,from,to}
       PlayerTeleported{player,tile} PlayerJailed{player,end:GridPos} PlayerReleased{player,junction}
地产   BuyOffered{player,land} RentDue{payer,owner,land,amount} LandBought{player,land}
       LandUpgraded{player,land,new_level} PlayerBankrupt{player}
经济   CashChanged{player,delta,new_balance} BankVisited{player,deposit,loan}
       StockVisited{player} StockTraded{player,stock,shares,price,side} StockPriceChanged{stock,old,new}
卡牌   CardDrawn{player,card_id,title,desc,outcome}
决策   DecisionChosen{player,action:Decision}   ← 收拢 confirm_buy/cancel_buy/confirm_upgrade/
                                                   cancel_upgrade/dismiss_rent_notice/dismiss_card 六个方法
场景   SceneRequested{target} Paused Resumed
```

> **B3 落地（10-14）**：上表除「场景域」三条（随 B5）之外全部就位，注册了 25 条消息。
> 两处与表格的差别：① 多了 `UpgradeOffered{player,land,cost}` —— §4.4 的表里只有
> `LandUpgraded`（成交结果），但升级报价本身是个要 UI 回答的问题，Godot 用
> `upgrade_decision_requested` 表达，照搬语义比凑齐表格重要；② `CardDrawn` 带的是
> `CardNotice{id,kind,title,desc,outcomes}` 结构而不是拼好的字符串，`outcomes` 是
> `EffectOutcome` 枚举（`Gained`/`Lost`/`CouldNotPay`/`Jailed`/`Teleported`/`UnknownTile`），
> 文案由 UI 生成 —— 缺陷 #3 里「结算函数拼字符串」那一半就此消除。
> `DecisionChosen{player,action}` 的 `action` 是 `Decision{Accept,Decline,Acknowledge}` 三值，
> 语义由 `PendingDecision` 决定（六个 Godot 方法 → 三个动作 × 待决上下文）。

三条原则：
1. **`phase_changed`（`event_bus.gd:17`）和 `turn_phase_changed`（`turn_manager.gd:41`）不移植**。Godot 需要字符串化阶段是因为没有别的途径让 UI 感知状态机；Bevy 里 UI 直接读 `Res<State<TurnPhase>>` 或监听 `OnEnter`。少两个事件、少一处 `_phase_to_string` 映射（`turn_manager.gd:483-492`）。
2. **信号 → 消息的语义差别必须写进规范**：Godot `emit()` 是同步递归调用，`rent_paid` 里再触发 `cash_changed` 会嵌套执行（这正是 `DEV_PLAN.md:458` 提的"RPC 友好"隐患）；Bevy 消息是**帧末批量投递**，天然无重入，因果顺序靠 schedule 顺序显式表达。这是移植的实质收益之一。
3. **`player_moved` 的发送者从 core 移到 render**：源项目里 `board.gd:451` 在 `await token.arrived`（`board.gd:445`）之后才发 `player_moved`，`TurnManager` 再据此推进（`turn_manager.gd:219-228`）——即逻辑等渲染动画播完。见 §4.6。

### 4.5 回合状态机：把 `await` 拆成状态

源实现：`turn_manager.gd:28-36` 的 7 个 Phase + 手写信号连接簿记（`:54-55,94-109`，`_exit_tree` 里 disconnect）+ 三处 `await`。这是 GDScript 里最容易出错的部分，也是 ECS 收益最大的部分。

0.20 形态：`#[derive(States, PartialEq, Eq, Clone, Copy, Hash, Debug)] enum TurnPhase { Boot, TurnStart, WaitRoll, Walking, Landed, WaitDecision, TurnEnd, Over }`，注册用 **`app.init_state::<TurnPhase>()`**（`add_state` 在 0.20 已不存在），转移用 `ResMut<NextState<TurnPhase>>::set()` / `set_if_different()`。

⚠️ **重要**：0.20 已移除 `OnUpdate(States)`，按状态跑 system 一律用运行条件 `run_if(in_state(TurnPhase::X))`；`OnEnter/OnExit/OnTransition` 仍可用。下表 `条件` 列即照此写。

| system | 条件 | 读 | 写 |
|---|---|---|---|
| `begin_game_system` | `OnEnter(Boot)`（B2a 暂挂 `Update` + `Option<Res<ActiveBoard>>` 自守门 —— 0.20 没有 `resource_missing` 运行条件） | `Assets<BoardDef>` `GameSetup` `GameRules` | spawn land/player 实体（道路格不建实体，见 §4.2）；`TurnOrder`；→`TurnStart` |
| `deliver_calendar_events_system` | `OnEnter(TurnStart)`（排在 `begin_turn_system` 之前） | `MarketClock` `GameSession.epoch` `HolidaySet` `TurnOrder` `Books` `Shelf` `Rng` | 12/25 给在场玩家各一张道具卡（`GiftsGiven` + `CardGained`）；正月初一每人抽四位红包（`RedEnvelope`，0 元不动现金）；`last_gift_year` / `last_envelope_year` 防重发。**不挂 `Pending`**，不挡回合机 |
| `begin_turn_system` | `OnEnter(TurnStart)` | `TurnOrder` `Query<&Detention>` `&Bankrupt` | 破产→跳过；拘留 `turns_left>1` → 扣一格、发 `DetentionSkipped`、→`TurnEnd`；`==1` → 摘组件、发 `DetentionLifted`、照常行动；**停留不在这里扣**（P4④：有 `StaySentence` 的人照常进 `WaitRoll`，掷骰被拒，按「停留」才走 `take_stay_system`） |
| `take_stay_system` | `in_state(WaitRoll)` | `MessageReader<RequestStay>` `Query<&mut StaySentence>` | 当前回合玩家按「停留」：计数减一；还剩则发 `StaySkipped`，扣到 0 摘组件并发 `StayLifted`；→`TurnEnd`（人还在场上，地照收租） |
| `roll_dice_system` | `in_state(WaitRoll)` | `MessageReader<RequestRoll>` `Res<Rng>` `Option<&StaySentence>` | 被停留按住的人**不掷**（缺 `StaySentence` 才走）；`DiceRolled`；算 `destination`；`TokenPosition`；`WalkQueue`；→`Walking` |
| `walk_step_system` | `in_state(Walking)` | `Res<WalkClock>` `WalkQueue` `ActiveBoard` | `TileStepped`；步数耗尽→发 `PlayerArrived`，→`Landed` |
| `resolve_landing_system` | `OnEnter(Landed)` | `TileKind` `LinkedLand` `Query<(&LandDef,&OwnerAddr,&BuildingLevel)>` `GameRules` | 分派：property→`BuyOffered`/`RentDue`+`DecisionChosen` 等待；stock→`StockTick`；bank→`InterestApplied`；chance/fate→抽卡并结算效果；road/start→直接 `TurnEnd` |
| `decision_system` | `in_state(WaitDecision)` | `MessageReader<DecisionChosen>` `Res<PendingDecision>` `Res<Landing>` | 执行买/升级/确认，清 `PendingDecision`，→`TurnEnd`；**报价答完后**若这一格是拘留分支的接入点且那里关着人，改挂 `Pending::Bail`（P4③，窗口留着直到「离开」） |
| `end_turn_system` | `OnEnter(TurnEnd)` | `Wallet` `GameSession` | `TurnEnded`；`turn_count+=1`；活跃人数 ≤1 → `Over`，否则推进 cursor → `TurnStart` |
| `market_tick_on_turn_end_system` | `OnEnter(TurnEnd)` | `Market`（股票实体 + `Rng` + `MarketClock` + `HolidaySet`） `GameSession.epoch` | 一天 = 一回合结束：`tick+1`；**休市日（周末 ∪ 节假日）只推日历、不推行情**；每月 15 号分红（`calendar::is_dividend_day`）。银行不受日历影响 |

**B2b 落地进度（10-13）**：表内 `begin_game_system`、`begin_turn_system`、`roll_dice_system`、`resolve_landing_system`、`end_turn_system`（+ 一个只做日志的 `game_over_system`）已上线；`decision_system` 与 `stock_tick_system` 随 B3。`begin_game_system` 挂的是 `in_state(Boot)` 而不是 `OnEnter(Boot)` —— 初始态不产生转移，`OnEnter` 根本不会触发。

**B2c 落地进度（10-14）**：`walk_step_system` 上线，`Walking` 态有了生产者 —— 掷骰即定路径（`BoardDef::walk_path`，纯主环 `i → (i+1)%N` 展开），逻辑位置掷骰帧落定到 `to`；逐格推进只消费 `WalkClock`、每格发 `TileStepped`，队列走空发 `PlayerArrived` 交棒 `Landed`。消息补齐 §4.4 移动域的 `WalkStarted`/`TileStepped`/`PlayerArrived`（现共 10 条）。

**B3 落地进度（10-14）**：`decision_system` 与行情/银行/卡牌的结算全部上线，但落格那一段
的实际形状与上表不同 —— 不是一个 `resolve_landing_system` 里写 if 链，而是 `OnEnter(Landed)`
上的六段流水线（`landing.rs`/`property.rs`/`bank.rs`/`market.rs`/`deck.rs`）：

| 段 | system | 只做 |
|---|---|---|
| 1 | `classify_landing_system` | 把当前玩家的 `TokenPosition` 翻译成 `Landing{kind, land}`（本帧只解释一次） |
| 2-5 | `settle_property_landing` / `settle_bank_landing` / `settle_market_landing` / `settle_card_landing` | 各读 `Res<Landing>`，不是自己那类格就直接返回；只写组件与消息 |
| 6 | `close_landing_system` | 唯一有权决定下一步去哪的一段：挂着 `PendingDecision` → `WaitDecision`，否则 `TurnEnd` |

三条收益：域的结算 system **推不动回合**（想越权就没有 `NextState` 可拿）；`TileKind` 的
`match` 穷尽性由编译器守住（缺陷 #8 的格类型那一半）；每段的参数包只装自己那几样组件，
`Books` 一个 SystemParam 统一持有现金/地块/存款/持仓，避免了同一 system 里两份 `&mut Wallet`。

两处刻意偏离表格与 Godot：① `stock_tick_system` 不是「`Landed` 之前」跑，而是**只在踩到证券格时**
`tick()` + 分红（照 `turn_manager.gd:372-378` 的代码，非文档）；② 卡牌的 `move_to` 不连锁结算
新落到的格子（Godot 亦然，否则会引入「卡把人送到卡牌格」的环）。

对照可见的三个简化：**不再需要** `add_child(tm)` 才能收信号的约束（`turn_manager.gd:16`）、`_dice_connected/_moved_connected` 布尔簿记（`:54-55`）、以及 `get_phase() != X: return` 这类手写守卫（`:194-195,221-222,317-318,326-327,...`，共 6 处）——`in_state()` 就是这个守卫的声明式版本。

### 4.6 唯一需要小心的架构问题：谁拥有"棋子位置"

Godot 端把这件事做成了"逻辑等渲染"（`turn_manager.gd:219` 等 `board.gd:451`）。彻底 ECS 化必须给一个答案，否则每个回合流程都要反向依赖渲染层，headless 测试又跑不了。

**建议：core 拥有逻辑位置与步时，render 只做视觉插值。**

- core 侧 `WalkQueue{ path: Vec<TileIndex>, elapsed: f32 }`，由 `Res<WalkClock>`（每帧 `+= Time::delta()` × `GameRules.walk_step_interval`）驱动，逐步发 `TileStepped`；`walk_step_interval` 取 `player_token.gd:17` 的 `0.18s`。
- render 侧 `TokenVisual` 读 `TokenPosition` + `TileIndex→Entity` 的世界坐标，自己做 `Transform` lerp（0.18s 对齐），不参与任何逻辑判定。
- 测试里把 `GameRules.walk_step_interval` 设成 0（或直接改 `WalkClock`），一步到位，无需等待；UI 上"跳过动画"就是把同一个值调 0——顺带实现 `DEV_PLAN.md:503` 想要的"动画速度"设置项。

这是本方案里唯一一处需要你在实施时守住的红线：**render 永远不向 core 发"推进回合"的消息**。

> **B2c 落地（10-14）**：`WalkQueue` 最终形态是 `{path, done, from}` 而不是本节的 `{path, elapsed}` —— 耗时累计归全局 `WalkClock`（行走互斥，同一时刻至多一人在走），队列只记「已迈几步」的绝对进度，`walk_step_system` 每帧把 `floor(elapsed/interval)` 与 `done` 的差补齐。`advance_walk_clock_system` 挂 `First`、只在 `Walking` 态推进（帧间真实 Δt）；headless 测试没有真实时间流逝，一律 `GameRules::instant()`（interval=0 → 一帧走完，与 B2b 逐字等价），要测节奏就直接改写 `WalkClock.elapsed`。另核实：`bevy_time` 是 bevy_internal 的**非可选依赖**（0.20 的 `bevy` 门面没有同名 feature），`game_core` 不用加 feature 就有 `Res<Time>`。

---

## 五、概念映射速查表（已归档）

> 整块（含 §5.1「已移植的 Godot 文件与行为差异」）搬到 [`docs/archive/migration-b0-b5b.md`](../docs/archive/migration-b0-b5b.md)，**停止逐文件维护**。
> 要查「Godot 某文件 ↔ Rust 某模块」的对照，或想知道某个行为是刻意分叉还是照搬，去那里。
> 新模块**不再往那张表里登记**：解释「为什么这样分」直接写进代码注释，像 §4.6 与 §六 的坑那样。

---

## 六、Bevy 0.20 依赖矩阵（已核实）

Bevy **0.20.0 发布于 2026-10-08**（5 天前），生态还没跟上，这直接约束选型：

| crate | 支持 0.20 的版本 | 判定 |
|---|---|---|
| `bevy` | `=0.20.0` | **精确锁定**，`[workspace.dependencies]` 统一 |
| `bevy_egui` | 0.43.1（bevy `^0.20.0`） | ✅ 可用 —— 调试面板、棋盘可视化编辑辅助 |
| `bevy_common_assets` | 0.18.0（bevy `^0.20.0`） | **未采用**：自写 20 行 `JsonAssetLoader<A>` 就够了（`game_core/src/loader.rs`），既省一个第三方依赖，也避开了 `.crate` 文件走 static.crates.io 直连不稳的问题 |
| `bevy_inspector_egui` | 最新 0.37.0，只配 bevy `^0.19` | ❌ **0.20 版未发布**。缺口：不能可视化 inspect 实体。缓解：用 `bevy_egui` 自写一个只读 entity dump（`ReflectedEntity`/`World::inspect_entity`），或等适配 |
| `bevy_tweening` | 最新 0.16.0，只配 `^0.19` | ❌ 不用，手写 lerp system（本项目动画就三种：逐格行走、落格弹跳、相机跟随，成本低于引插件） |
| `bevy_mod_picking` | 0.20.1（2024-07）配 bevy 0.14，停更 | ❌ 改用内置 `bevy_picking` |
| `serde` / `serde_json` | 独立版本 | ✅ 棋盘/卡牌/股票/存档 |
| `rand` / `rand_chacha` | 独立版本 | ✅ `ChaCha8Rng` 做种子化 RNG |
| `bevy_settings` | 0.20 内置新 crate（`SettingsPlugin` + `#[derive(SettingsGroup)]`） | 🆕 可选，音量/动画速度等设置项。太新，先 spike |
| `bevy_world_serialization` | 0.20 内置新 crate（`DynamicWorld`/`WorldAsset`，带 serde 特性） | 🆕 存档可用，但**同为 0.20 首发 API，风险高**，先做手写快照（§九） |
| 中文字体 | 0.19 起 `bevy_text` 换 Parley；0.19 加 `bevy/system_font_discovery` feature 与 `FontSource::Family`；0.20 把 `resolve_font_source` 改为 `FontSource::resolve_font_family` | 桌面端可按系统字体族名加载中文字体；发行仍建议自带子集（§七） |

> ~~如果 `bevy_inspector_egui` 的缺口你认为不可接受，唯一退路是**降到 bevy 0.19**（生态已适配面更广）。这是 §十一 的待决问题之一。~~
> **✅ 已拍板（10-13）且不降版**：锁 0.20，缺口用 `bevy_egui` 自写只读 dump 面板补。10-16 核对时那条面板**还没落地**（`Cargo.toml` 里根本没有 `bevy_egui`），于是排进 [`PLAN.md`](./PLAN.md) 的 **T1**；**同日 T1 落地**（`game_app/src/dump.rs`，`NM_DUMP=1` 装载），踩过的两个坑是 §6.1 坑 28。

### 6.1 0.20 API 事实核对表（B0 已从本地 crate 源码 + 官方 examples 逐个核实）
| 事项 | 0.20 的确切形态 | 对方案的影响 |
|---|---|---|
| feature 模型 | `default = ["2d","3d","ui","audio"]`；`2d`/`3d`/`ui` 各自 = `default_app` + `default_platform` + `scene` + `picking` + 对应渲染层 | §4.1 的"`default-features=false` 即无渲染"成立，但要显式给 `features = ["default_app"]` |
| `default_app` | `async_executor, bevy_asset, bevy_log, bevy_state, reflect_auto_register` | **规则层需要的正是这个**：ECS + Asset + States + Log，不含 winit/渲染 |
| `default_platform` | `std, bevy_gilrs, bevy_winit, bevy_clipboard, default_font, multi_threaded, webgl2, x11, wayland, custom_cursor, sysinfo_plugin` | 只有表现层该链接它；注意 `default_font` 是 Fira Mono 子集，**无汉字** → 中文必现豆腐块 |
| 状态机 | `#[derive(States)]` + `app.init_state::<T>()`；`OnEnter/OnExit/OnTransition` 在；**`OnUpdate` 已删除**，改用 `run_if(in_state(T::X))`；转移 `NextState<T>::set()/set_if_different()` | §4.5 的表已按此写 |
| 消息 | `#[derive(Message)]` + `app.add_message::<T>()` + **`MessageWriter<T>::write()`** + `MessageReader<T>`/`Messages<T>`；`Commands` 上**没有** `send_message`（B2b 实测纠正）；观察者推送是另一套 `#[derive(Event)]` + `trigger`。⚠️ 新增消息漏了 `add_message`：带着这条 `MessageWriter` 的**参数包**（`Books` 被每个 system 用着）整局不跑，而报错里的 system 与参数名在非 debug 构建里被隐成 `<Enable the debug feature to see the name>` —— 症状是「什么都没发生」，先查注册再查逻辑（P3c② 实测） | §4.4 成立，写法见 `game_core/src/turn.rs` |
| 世界空间文本 | **`Text2d` 已从 `bevy_text` 移到 `bevy_sprite`**（需 `2d` 或 `bevy_sprite_render` feature）；UI 文本根是 `bevy_ui::Text::new(s)`，子 span 用 `TextSpan`；样式是并列组件 `TextFont{font, font_size, ..}` / `TextColor` / `TextLayout::justify(..)` | `tile_view.gd:146-152` 的 `Label3D` 无直接对等物（见下） |
| 字体来源 | `FontSource::{Handle(H), Family(SmolStr), Families(&str), List(Vec), Generic}`；`system_font_discovery` feature 开启后可按族名加载 | §7.3 的两条路都可行；开发期用文件路径最确定 |
| 正交等轴测 | `Camera3d::default()` + `Projection::from(OrthographicProjection{ scaling_mode: ScalingMode::FixedVertical{ viewport_height }, ..OrthographicProjection::default_3d() })`（官方 `examples/3d/orthographic.rs` 自称就是给 isometric 用的） | §7.1 直接照此实现，B0 已验证 |
| 几何 | `Mesh3d(meshes.add(Cuboid::new(x,y,z)))` + `MeshMaterial3d(materials.add(Color::srgb(..)))` | 图元走 `bevy_math::primitives`，映射表时代标的那条 ⚠️ 已解除 |
| 3D + UI 共存 | 单个 `Camera3d` 即可渲染 UI 层（官方 `examples/3d/3d_shapes.rs` 如此），不必额外 `Camera2d` | HUD 无需多相机编排 |
| 拾取 | 内置 `bevy_picking`（`picking` feature）：`DefaultPickingPlugins` + `Pickable` + `Pointer<*>`；`bevy_mod_picking` 停更 | 映射表时代的点击方案已落地 |
| UI 交互 | `Button`/`Interaction` 在 0.20 已弃用，改 `OnInteraction` 组件式；`TextScroll` → `EditableText::viewport`；cursor 模块并入 `bevy_picking` | B5 写按钮时按新形态，别抄旧教程 ⚠️ |
| 存档/设置 | 新增内置 `bevy_settings`、`bevy_world_serialization`（`scene` feature 会带后者） | 同为 0.20 首发，按 §九 走手写快照 |

**实跑发现的三十三条环境级坑（已踩过，记录避免重犯）**：
1. **`AssetPlugin::file_path` 在 0.20 是相对「可执行文件目录」的 `String`（不是 `Option`）**，而 `cargo run -p game_app` 的 CWD 是仓库根 → 默认会把资产解析成 `target/debug/assets/**`，所有资产**静默 404**（只有一行 `bevy_asset::server` ERROR，程序不崩）。落地做法：启动时把资产根解析成绝对路径再塞给 `AssetPlugin`，`std::fs` 与 `AssetServer` 共用同一个根，否则会出现"直读文件成功、Asset 加载失败"这种极难排查的组合。
2. **`default_font` feature 自带的是 Fira Mono 子集，无汉字**，所以不显式指定字体时中文不是"报错"而是**豆腐块/空白**。落地做法：把字体放 `assets/font/` 并用 `asset_server.load()` 显式指定（本仓库用开发授权的思源黑体，见 §7.3）。
3. **中文必须开 `bevy` 的 `complex_script_segmentation` feature**（转发到 `parley/complex-scripts`）。不开时 Parley/ICU4X 找不到中日韩分词模型，会**每帧刷** `ICU4X data error: No segmentation model for complex script: Chinese/Japanese`，且长句换行按非 CJK 规则处理。B0 实测：开启后错误归零。
4. **等轴测取景要自己算**：`Transform::from_rotation(...)` 只给旋转不给位移会让画面没有主体；正交相机的可视范围由 `ScalingMode::FixedVertical { viewport_height }` 决定，`default_3d()` 的 `near=0.0`。本项目按棋盘 AABB 推 `center`/`eye`/`viewport_height`（`main.rs::board_aabb`），视线方向沿用 Godot 的 `CAM_OFFSET=(8,16,8)`（即 (1,2,1) 对角、俯角约 54.7°）。
5. **`Screenshot` 的触发条件不能用帧数**（这条坑最贵，害我把「渲染没问题」误判成「3D 通路坏了」整整一轮）：`PresentMode::AutoNoVsync` 下前 40 帧只有一二十毫秒，3D 管线的着色器编译与网格上传还没热完，此时截出来的 PNG **只有 clear color 与 UI**，而窗口里其实是正常渲染的。判据：截图前后 md5 完全一致 → 说明截的是同一个未完成的中间态，而不是"画面没变"。正确做法是按**墙钟时间**触发（本项目 `time.elapsed_secs() >= 1.5`）。
6. **资产加载失败默认只打一条日志然后永远等下去**。`AssetServer::load` 失败不会 panic，依赖它的启动流程会静默卡住。本项目在 `boot_world` 里对每个 handle 调 `expect_ready`，命中 `LoadState::Failed(Arc<AssetLoadError>)`（注意是**元组变体**，不是单元变体）就带路径 panic。已用「临时把资产根指到不存在的位置」验证：退出码 101 + 明确中文报错，而不是白屏干等。
7. **同一扩展名注册多个 loader 是官方支持的**：`Loaders::find` 先按资产 `TypeId` 查 `type_id_to_loaders`，再按扩展名筛。所以 `BoardDef`/`CardSet`/`StockSet` 三个 `JsonAssetLoader` 都声明 `extensions = ["json"]` 并不冲突，typed handle 会路由到匹配的那个；只有**同类型 + 同扩展名**重复注册才会 warn。
8. **`#[derive(SystemParam)]` 不在 prelude 里**，要 `use bevy::ecs::system::SystemParam;`。用它打包多个 `Res<Assets<T>>` 比给系统挂 `#[allow(clippy::too_many_arguments)]` 好 —— 后者会把「参数确实太多了」这个真信号吃掉。
9. **headless 测试里用 `AssetServer` 读盘，必须自己初始化任务池**：`App::new()` + `CoreGamePlugin` 不够，`AssetPlugin` 的文件 IO 跑在 `IoTaskPool` 上，而它只由 `bevy::app::TaskPoolPlugin` 建立，否则第一次加载直接 panic `The IoTaskPool has not been initialized yet`。`game_app` 经 `DefaultPlugins` 自带，所以这个坑只在测试侧暴露（好在是响的，不是静默失败）。
10. **别用 `timeout N cargo run` 做冒烟测试**：debug 链接一个上百 MB 的 Bevy 二进制要吃掉大半预算，`timeout` 到点杀掉的是 cargo 而不是应用，日志会是**空文件**（看起来像"应用没输出"）。做法：先 `cargo build -p game_app` 再 `timeout N ./target/debug/game_app.exe`；并且 `NM_SHOT` 现在会在截图落盘后于 2.5s 自行 `exit(0)`，不依赖外部 kill，也不会留下没人关的窗口进程。
11. **`init_state::<T>()` 要求 `StatesPlugin` 已就位**（路径是 `bevy::state::app::StatesPlugin`）。真实客户端经 `DefaultPlugins` 自带，headless 测试的裸 `App::new()` 没有 → 直接 panic `The StateTransition schedule is missing`。而 `StatesPlugin::build` 用的是 `MainScheduleOrder::insert_after`，**不幂等**：加两遍会让状态转移每帧跑两次。所以 `CoreGamePlugin` 里补加之前先 `app.is_plugin_added::<StatesPlugin>()` 查一次。
12. **消息与状态转移都是「一步一帧」**，测试的等待条件不能写成「阶段等于 X」：本帧 `write` 的消息下一帧才 `read` 得到（缓冲在 `First` 交换），`StateTransition` 又排在 `PreUpdate` 之后。后果是「刚发出掷骰请求、阶段还没动」时 `phase == WaitRoll` 立刻为真 —— 本项目就因此把四个回合测试全判错过一次。判据换成 `GameSession.turn_count`，并在结尾固定多跑几帧把消息刷进流水（见 `tests/common/mod.rs::play_turns`）。
13. **`MessageReader<'w, 's, M>` 带两个生命周期**（`MessageWriter<'w, M>` 只有一个）。打包进 `#[derive(SystemParam)]` 时要把结构体写成 `<'w, 's>` 两参数形式。
14. **元组的 `.run_if(...)` / `.in_set(...)` 会在「元素里有双生命周期 `SystemParam`」时崩**：报错是 `cannot become an ObserverSystem`（重载解析掉进了 `ObserverSystemExt::run_if`）。单个 system 挂条件没事、纯函数元组挂条件也没事，混进 `MovementNews<'w,'s>` 这种参数包就不行。做法：条件挂到 `#[derive(SystemSet)]` 分组上，用 `configure_sets` 配；这也顺带把「一组 system 共用一个守门」说清楚了。
15. **`resource_added::<T>` / `resource_exists::<T>` 是函数项，不加括号**（`.run_if(resource_exists::<ActiveBoard>)`），与 `in_state(TurnPhase::Boot)`（要括号，它是个返回闭包的工厂）正好相反。加错括号的报错是「this function takes 1 argument but 0 arguments were supplied」。
16. **rig 父节点要自己带 `Visibility`**：`Camera3d` 需要 `InheritedVisibility`，而它是从父节点的 `Visibility` 继承来的。父节点只挂 `Transform` 就会刷 `bevy_app::hierarchy` 的 B0004 警告（可见性传播断在 rig 这一层），画面看着正常但每帧叫一次。
17. **`Commands` 造的实体在同一帧的后续 system 里看不到**（`.after()` 不保证插 `apply_deferred`，两个 system 无静态冲突时调度器不会加 flush 点）。这条在 HUD 上表现为「第一帧一个都没匹配到，却把文本记成最新，之后每帧提前返回」—— 缓存「已经写成什么样」必须等到**真的写到了**才更新。
18. **`MainSchedule::Startup` 的末尾会跑一次 `StateTransition`**：`ScenePlugin` 在 Startup 里 `set(Scene::Menu)`，而 `Menu` 正是默认值 —— 补出来的这次转移**与 Startup 同批**执行，于是 `OnEnter(Menu)` 的 spawn 代码看到的还是空 world。用 `Commands` 插开机资源要等帧末 flush，那时菜单已经试着建过了，报的是 `Res<DevFont> failed validation: Resource does not exist`。做法：开机资源（字体 handle、资产根、种子）直接 `world.insert_resource(...)`，不走 `Commands`。
19. **UI 也要有相机**：一个场景里没有任何 `Camera` 时，画面就是纯 `ClearColor`，UI 一个字都不画，而且**不报错**。菜单场景原本只打算放 UI，结果截图整片黑，补一台 `Camera2d`（带自己的 `ClearColorConfig`）才有字。
20. **0.20 的点击交互要自己搭三层**：① `bevy_ui::Button` 与 `Interaction` 已弃用（prelude 里带着 `expect(deprecated)`，`-D warnings` 直接吃错），点击只能读 `MessageReader<PointerClick>`；② `UiPickingSettings/MeshPickingSettings::require_markers` 判的是**相机**（`Has<UiPickingCamera>`），逐节点的开关是 `Pickable`；③ 命中测试报的是**最深的**节点，所以按钮文字要点得动就得沿 `ChildOf` 往上找动作，而**横跨整屏的顶栏必须挂 `Pickable::IGNORE`** —— 否则它背后那条带上的地板永远点不到（这条是静默失效：按钮一切正常，只是格子点不出来）。
21. **Bevy 的 `top` 不等于 Godot 的负 `offset_top`**：`player_hud.gd` 的 `PRESET_BOTTOM_WIDE + offset_top=-120` 直译成 `top: 120` 会把 HUD 摆到屏幕**顶上**，正确换算是 `bottom: 8 + height: 112`。编译、运行都不报错，只有截图看得出来 —— 所以 §7.2 的每一处绝对定位都要配一张图再收工。
22. **回主菜单时，跨局存活的 UI 记忆会静默毒化下一局**（B5 收尾时发现，三条都是实跑的账）：`OnEnter(Menu)` 把面板实体随 `BoardUi` 收走，但资源与 `Local` 留着 ——
    - `Paused(true)` 留着 → 下一局开局即冻结，而画面看着完全正常；
    - 「上次挂着哪个待决事项」存在 `Local`/资源里 → 新局第一次报价与上一局最后一次恰好相同（`Buy(LandId(0))` 很常见）时一比相等，**模态永远不再弹**，回合机停在 `WaitDecision` 等一个不存在的按钮；
    - `Local<Option<Entity>>` 存着面板句柄 → 面板早被 `despawn_board_view` 收走，下一次开合对着死实体 `despawn`。

    做法：**「现在显示成什么样」的键挂在实体上**（`ModalRoot((player, Pending))`、`TileCard(TileView)`、`PausePanel` 标记 + 查询），实体没了键自然没了，不需要谁记得去清；只有真正无法从实体推出的记忆（`RecentCard` 这种消息副产品、`PauseMenuOpen`、`Paused`、`SelectedTile`）留在资源里，由 `ui::reset_ui_state` 在 `OnEnter(Menu)` 链上统一清。判据：任何 `Local`/资源缓存都要能回答「换一局时它归谁清」。

23. **把「两个场景都要用」的系统挂进「一个场景才存在的门」里，另一个场景就是死的**（B5 验收后用户实报：主菜单「新游戏」「退出」点了没反应，只能点窗口关闭）。`dispatch_ui_actions`/`hover_feedback` 曾随大流挂进 `UiSet::Live`（`run_if(resource_exists::<ActiveBoard>)`），而 `ActiveBoard` 进对局才插入、回菜单被 `teardown_match` 移除 —— 主菜单上派发系统一帧都不跑，`PointerClick` 没人读，按钮全部哑掉。**B5 验收测不到它**：`NM_MENULOOP` 直改 `Scene` 状态、绕过了派发，等于用「不经过被测代码」的路径验证了被测代码；对局内按钮有 ActiveBoard 撑着，一切正常。修法：派发/悬停移出 Live 集，无条件跑、仍排链尾（依赖资源全部常驻：`Paused`/`PendingDecision` 是 `init_resource`、`TurnOrder` 是 `Option`）。回归测试 `the_menu_dispatches_without_an_active_board` 用**真实 UiPlugin+ScenePlugin 接线**（不是手搓 dispatch 的 Harness）钉住这条缝 —— 接线错误只有走真实接线才测得到。判据：`run_if` 门禁里的资源若只在某个 Scene 存在，门后的系统就只属于那个 Scene；跨场景系统必须不在任何场景门后。

24. **悬停高亮按事件增量涂色，会被同帧的 Out 覆盖成「一闪即逝」**（坑 23 修完后用户紧接着实报：悬停只亮一下就回原样）。picking 每帧只 hover **最深的一个**节点：指针在按钮**空白处**时 hover 集 = `{按钮}`，挪到**文字**上就变成 `{文字}`，于是「空白→文字」这一次最普通的移动会**同帧**发出 `Out(按钮)` + `Over(文字)`。`hover_feedback` 若按「先 Over 后 Out」处理，后到的 Out 把刚点亮的高亮立刻浇灭，而光标恰恰最爱停在文字上（按钮中央），稳态就是原色。修法：**先清算 Out、再应用 Over** —— 同帧「离开旧 + 进入新」指向同一按钮时最终停在 hover 色，彻底离开时只有 Out、自然复原。回归测试 `out_and_over_of_the_same_button_in_one_frame_stays_hovered`（真窗口冒烟：光标停在按钮文字上 8 秒，`NM_SHOT` 自截，按钮保持 hover 色）。判据：靠事件增量维护的视觉状态，同帧内的事件对要按「先失去、后获得」合成，最后写入的必须是语义上的最终态。

25. **`reset_ui_state` 把开发旋钮清成 `default()`，等于把「不依赖鼠标的入口」自己拆掉**（B5b 拍 ATM/证券两张截图时踩到）。开机那次 `OnEnter(Scene::Menu)` 在 `NM_SCENE=board` 下**照样跑**（`Menu` 是默认值，`scene.rs` 的 `startup_enters_menu` 补的就是这次转移），而 `reset_ui_state` 挂在它后面 —— 于是 `NM_ATM`/`NM_STOCK`/`NM_PAUSE` 三个开关刚被 `from_env()` 读进资源，就被同一批里的 `default()` 抹平，面板永远拍不到东西，看上去像「面板没渲染」。修法：这三个「开机时定」的开关一律复原到 `from_env()` 而不是 `default()`（真人没设环境变量，`from_env()` 就是 `default()`，行为不变）。判据：**清理函数只能清「上一局留下的」，不能清「这一局要用的」**；同一个资源既被跨局清理又被开发旋钮写入时，复原值必须是旋钮值。

26. **运行时才禁用的按钮只换底色、不换字色，玩家读不出「这颗按不动」**（B5b 真鼠标冒烟时误判出来的）。`button()` 在 spawn 那一刻按 `spec.disabled` 同时配好灰底 + 灰字，但顶栏的可用态是 `refresh_top_bar` **每帧算**的（ATM 卡到手/过期、暂停开合都不改阶段）；它只写 `BackgroundColor`，于是运行时禁用的按钮仍然顶着白字。而 `BTN_DISABLED(0.15)` 与 `BTN_SECONDARY(0.22)` 差得很有限 —— 结果「ATM 现在不能点」这件事在画面上等于没说，我第一次冒烟就是被它骗去怀疑「面板按钮拾取挂了」。修法：`refresh_top_bar` 换底色时一起改写那颗按钮的 label 颜色（`paint_label`）。判据：**灰不灰是一种信息，就必须每个来源都写它**；spawn 期与运行期两套着色路径不能只有一条记得改字色。

27. **合成鼠标点击要先 `SetForegroundWindow`，否则点击落到别人窗口上，冒烟会把「我的 UI 是死的」当成结论**（B5b 真鼠标冒烟连栽三次）。`SetCursorPos` + `mouse_event` 送的是**屏幕坐标**，落在哪一层窗口由 Z 序决定：游戏窗口被 IDE 压住时，点击全部进了 IDE，`NM_SHOT` 那张图却照常是游戏画面（GPU 侧截屏不看遮挡），于是「图是对的、数字没变」看起来像 UI bug。判据：自动化点击脚本第一步取 `MainWindowHandle` → `ShowWindow(SW_RESTORE)` + `BringWindowToTop` + `SetForegroundWindow`，再用 `GetClientRect`+`ClientToScreen` 换算客户区原点（窗口每次启动被系统级联，别把坐标写死）；并且**同一目标点多次、截图时刻推后**，让投递抖动不至于推翻结论。这条与坑 26 是同一场误诊的两半：先确认「按没按到」，再谈「亮不亮」。**第四半（P3d④a 踩到）**：级联不只是「别把坐标写死」那么轻 —— 它每次启动往下右挪二十几像素，于是**同一份命令、同一个客户区坐标**在不同次运行里落到屏幕上不同的地方，挪远了就落到别的窗口那儿；本机实测同一份点击连跑三次「一次成、两次空」，前两半的排查（前台、时机）都做完之后仍然复现，才被误记成「对局还没装配完所以点早了」。修法是把窗口 `MoveWindow` 钉到固定位置（保留宽高，客户区尺寸不变）、**每次按之前**重量一次 `ClientToScreen` 并把窗口重新提回前台，`tools/mouse_smoke.ps1` 已经这么做，挪了位会打在输出里（`moved: origin ...`）。判据也跟着升级：**一次空点不算结论，跨两次运行同一坐标一次成一次空也不算** —— 先看脚本有没有报出原点被挪动。**第五半（P6d 踩到）**：`SetForegroundWindow` 那条抢焦点保护**不是「撞上一次抢夺的窗口期」这种瞬态失败，而是可以持续整个脚本生命周期** —— 系统只允许「当前前台进程」把前台让给别人，而脚本是从 PowerShell 里拉起游戏的，它自己不是前台。本机实测：从 `ShowWindow(SW_RESTORE)` 到每次点击前共重试 1 秒 ×N 次（含 `SwitchToThisWindow` 那条 Alt-Tab 路径），`GetForegroundWindow` 仍返回别的窗口句柄，而 `NM_SHOT` 那张图照样是游戏画面 —— 症状与前四半完全相同。修法不是继续抢，而是**换投递路径**：`tools/mouse_smoke.ps1 -Route post` 用 `PostMessage` 把 `WM_MOUSEMOVE`/`WM_LBUTTONDOWN`/`WM_LBUTTONUP` 连**客户区坐标**（`lparam = y<<16 | x`）直接塞进游戏窗口的消息队列，不经过前台与 Z 序，winit 的 WndProc 照常发 `CursorMoved`/`MouseInput`，`bevy_picking` 的 `PointerClick` 于是照走。**这条路的边界要写在验收里**：它验的是「按钮的动作是通的」，不是「真人点得到」（真设备的命中测试被绕过了），所以 `-Route screen` 那条仍是首选、post 只在 `fg=NO` 时兜底，两者不许互相冒充。脚本现在两条都印 `raise before click: fg=…`，那一行就是「今天 screen 那条可不可信」的探针。

    **同一条坑的第二半（T1，10-16 又栽两次）：点击脚本的时间轴从「它找到窗口」起算，`NM_SHOT_AT` 从「游戏启动」起算 —— 脚本晚起 N 秒，点击就整体落到截图之后**。表现是「点了没反应」，而真实原因那次是点击根本还没发生。判据：**先确认点击时刻 < 截图时刻**（脚本前别再 `sleep`，或者干脆把 `NM_SHOT_AT` 推到脚本结束之后），再谈按没按到。

28. **`bevy_egui` 的主上下文钉在「第一台相机」上，而我们的相机是随场景生灭的 —— 换场景等于把上下文一起扔掉，且它不会自己重来**（T1 只读面板第一次上屏时踩到：插件加载了、渲染器起来了、`EguiPrimaryContextPass` 也注册了，面板一帧都不画）。0.43 的自动创建只在**第一台** `Camera` 的 `Added` 时刻插 `PrimaryEguiContext`，并且用一个内部 `Local<bool>` 记账 —— 菜单那台 `Camera2d` 先出生拿走上下文，进对局时随 `MenuPiece` 收走，上下文跟着实体一起没了，此后永远不再补。修法见 `dump.rs::attach_context`：`Update` 里查 `Query<(Entity, &mut EguiContext)>`，为空就把 `PrimaryEguiContext` 补到当时活着的相机上。**第二半必须一起做**：补出来的那个 `egui::Context` 是全新的，字体表回到默认 —— 只在第一次装字体会得到「菜单里是中文、进对局全变方框」，实测就是这么翻的车。所以字体跟着**宿主实体**装（`Local<Option<Entity>>` 换值即重装）。判据：**凡是「给某个上下文/资源做过一次性设置」的代码，先问它跟着哪个对象生灭**；重建的落点与设置的落点必须是同一个系统，否则设置会随旧对象一起蒸发。附带一条 API 事实：`EguiContexts` 内部已经握着 `&mut EguiContext` 的查询，同场再要一个 `Query<Entity, With<EguiContext>>` 会撞借用冲突 —— 要实体就直接查 `(Entity, &mut EguiContext)`。

29. **`Commands` 的删除当帧生效，`NextState` 的转移下一帧才生效 —— 把「收资源」和「叫停状态机」写在同一个 system 里，中间就有一整帧的空档，0.20 会把它 panic 出来**（T1 真鼠标冒烟点「返回主菜单」必崩，exit 101；`git show 37675e8` 核过与 P1 无关，是 B5 就带的）。`teardown_match` 原来同时做两件事：`remove_resource::<TurnOrder>()` + `phase.set(Boot)`。命令在 system 跑完就落地，而转移要等下一帧的 `StateTransition` —— 于是那一帧的后半程 `TurnPhase` 仍写着 `WaitRoll`，`roll_dice_system` 与 `trade_request_system` 照常跑，参数校验拿不到 `TurnOrder` / `ActiveBoard`，Bevy 0.20 默认把这种失败直接 panic。修法：拆成 `stop_turn_machine`（`OnEnter(Scene::Menu)`，只叫停）+ `clear_match`（`Update`，`run_if(in_state(Scene::Menu))` 且 `State<TurnPhase>` 真的是 `Boot` 才收尸）。判据：**"停机器"和"收机器"不能同帧**；凡是 `remove_resource` 与 `NextState::set` 出现在同一个 system 里，就要怀疑中间那一帧谁在跑。覆盖它的测试 `ui::tests::the_match_resources_outlive_the_turn_machine`，可复现的路径是 `NM_MENULOOP`（原来它只走「菜单→对局」，那半程从没踩过 —— 见该表最后一行）。

30. **面板的刷新签名少一位，面板就停在旧世界里，且不报错**（P2④ 补休市着色时踩到，是坑 22 那一族的又一实例）。`sync_modal` 的重建判据是「问题 + 签名」，柜台的签名原本只有四个金额：`现金|存款|负债|可贷`。周末那天这四个数一个都不动，于是「开门时烤出来的常态面板」在休市后继续挂着 —— 八颗按钮亮着、副标题写着「办完才继续」，而规则层一律不受理。修法：把 `banking_hours` 也拼进签名（`dialog.rs::modal_stamp`）。判据：**面板上每一个由判据决定的可见位，都必须是签名的输入**；写签名时逐个对着"这块屏上有什么会因世界变化而变"数一遍，漏一个就是一次静默过期。ATM/证券两块浮层的签名早就带了 `tick`。⚠️ **P2b 之后柜台那一半整条退役**：银行全年无休，`banking_hours` 这个输入不再存在；坑本身照样成立（证券面板的 `tick`/`tradable` 仍是签名的一部分）。

31. **逐帧着色 system 的查询范围是「全工程任何带 `UiAction` 的按钮」，于是顶栏替所有浮层决定了它们的灰不灰**（P2④ 拍休市截图时第一次注意到日志，P3b 真鼠标冒烟才看清后果）。成因一半是查询、一半是判据：`refresh_top_bar` 遍历全部按钮，而 `button_allowed` 对不认识的动作一律 `_ => true` —— 浮层里 spawn 期就该灰着的按钮，下一帧被顶栏 `remove::<Disabled>()` 剥掉组件、`paint_label` 刷成白字。两个后果：① 同一帧里面板因签名变化整块 `despawn`，排在帧末的 `remove` 打在空实体上，每次重建刷一叠 `Entity despawned` WARN（`NM_STOCK=1` + 日历跨进周末必现，一次 10 条 = 证券浮层的 10 颗买卖按钮；**与日历无关**，把 `Disabled` 的插入条件从"日"换成"阶段"同样触发）；② 底色仍是 `BTN_DISABLED` 而字色变白 —— 正是坑 26 说过的那个「灰了但读不出灰」，`hover_feedback` 也不再跳过它们，鼠标一划就点亮一颗按不动的按钮。P2 那句「只是日志噪音，无行为后果」是错的：噪音只是先响的那一半。
    修法按 B5b 定的分工收口：给顶栏根节点挂 `TopBar`，`refresh_top_bar` 只遍历它的子节点（旁白与阶段名两行没有按钮那套组件，`get_mut` 失败即跳过），浮层的灰由各自的 `ButtonSpec::disabled(...)` 与刷新签名负责。判据：**逐帧着色的 system 必须先说清它管哪几颗按钮**，"任何带某组件的实体"这种范围一旦写出来，全工程都是它的输入。覆盖测试 `ui::top_bar::tests::the_top_bar_refresh_only_touches_the_top_bar`（两头都断言：浮层那颗不许被碰，顶栏那颗该变还得变 —— 只断言前者会把「什么都没做」当成通过）。

32. **dev profile 不管调试信息 + 并行度放满 = 整机死机**（P3a 期间连续两次，都是 `cargo test` 编译到一半机器冻住，只能硬重启）。两条成因叠在一起：① 根 `Cargo.toml` 里 `[profile.dev.package."*"] opt-level = 3` 让**每个依赖 crate 以 O3 编译、又默认带完整调试信息**；② `cargo test -p game_core` 不是一个二进制，是 1 个库 + **10 条集成测试二进制**，每条都要把依赖那棵树的调试信息再消化一遍。实测（12 核 / 32 GB，测法是 [`tools/mem_probe.ps1`](../tools/mem_probe.ps1) 每 300 ms 采一次 `FreePhysicalMemory` 与 rustc/link 的 `PrivateMemorySize64`）：旧 profile 只开 **3 路**就吃掉 29.7 GB 物理内存、工具链私有内存峰值 14.6 GB（≈ 4.9 GB/job）—— 按默认 12 路外推就是 ~58 GB 提交内存，页面文件风暴，于是死机。旁证：`cargo clean -p game_core` 当时报 **11892 个文件 / 20.1 GiB**，全是本项目自己根本没在用的调试信息。
    修法两条，都在**根 `Cargo.toml` 的 `[profile.dev]`**：工作区 `debug = "line-tables-only"`、依赖 `debug = false`。改完 `cargo clean -p game_core` 报的产物量从 20.1 GiB 降到 **1.4 GiB / 2040 个文件**，同一件事的私有内存峰值从 14.6 GB（3 路）降到 7.0 GB（4 路）。断言失败打印的 panic 位置本来就不依赖调试信息，仍然精确到文件行；丢的是「用调试器单步进 bevy」——本项目从不这么查问题（查画面用 `NM_SHOT`、查状态用 `NM_DUMP`）。
    **但光换 profile 不够**：默认 `-j 12` 复测仍有 18.2 GB 工具链私有内存、24.6 GB 物理峰值，旁边开着 IDE 就悬。所以再加 `.cargo/config.toml` 的 `[build] jobs = 6`，复测降到 10.0 GB，而那一次重建的耗时没变（25 s → 26 s）：关键路径是「先编完库、再链 10 条二进制」，链接本身很便宜，多开并行本来就几乎没有收益。判据：**这台机器上别把 jobs 放满**，也别为了"快一点"回去调那两个 `debug` 键 —— 改 profile 会让**全部依赖重编一次**（实测 1285 s ≈ 21 分钟，一次性代价）。同理别顺手 `cargo clean`（全清），要清就 `cargo clean -p game_core`。改完之后 `./check.sh` 五项全绿用 242 s（那一次还含 clippy 按新 profile 把依赖重校一遍的代价）。



33. **同一个 system 里两条查询都读 `Transform` 会以 B0001 panic，即使它们碰的是天然不相交的两批实体**（P5 的道路图标第一次上屏时踩到）。`sync_occupancy_visuals` 一条查图标（`&mut Transform`）、一条查棋子（`&Transform`），实体集合一个来自 occupancy、一个来自 token，肉眼绝不会相交；但 Bevy 0.20 的冲突检查只看**组件类型与过滤器**，不看实体从哪来，参数建立期直接 panic：`error[B0001]: Query<(&TokenVisual, &Transform)> ... accesses component(s) Transform in a way that conflicts with a previous system parameter`。修法：给其中一条加 `Without<另一批的标记组件>`（本项目是 `shown: Query<..., Without<TokenVisual>>`），把「它们不是同一批实体」写成调度器看得见的条件，而不是写成注释。判据：**同一个 system 里出现两条读写同一组件的查询时，必须用 `With<T>`/`Without<T>` 证明互不相交**；`ParamSet` 是另一条路，但它会强迫两条查询顺序借用，能拆开时优先拆开。
34. **bevy_ui 默认不裁子节点：绝对定位的孩子画出父盒之外，一声不响**（P7 游乐场第一张冒烟图）。气球从底部进场时 `bottom` 是负的、财宝落到地面以下时也是，于是那几只画在玩法区外面、压在 HUD 与底栏上 —— 不报错、不警告，只是「图上多了两个球」。修法：容器节点写 `overflow: Overflow::clip()`。**判据：任何「东西从边界外进场/退场」的玩法区都要裁**；反过来，若哪天某个绝对定位的孩子「该露出来却不见了」，先查它祖先有没有这一位。
35. **`Node` 的 `flex_direction` 默认是 `Row`，而「一列一列往下排」是大多数面板想要的**（P7 挖宝第一张图：4×4 的 16 格排成**一行**横着溢出，被上一条的 clip 切成两半，症状像「格子数不对」而不是「方向不对」）。绝对定位的孩子不受这一位影响，所以同一块玩法区里只有 flex 排的那一种露馅 —— **混用绝对定位与 flex 的容器，方向错了只有一半孩子报错形状**。修法：建容器时显式写 `flex_direction: FlexDirection::Column`，别靠默认。
36. **并行元组内部没有顺序可言，所以「把 system 挪进元组换个位置」不是一次有效的破坏**（P8 red-check 当场撞出来的方法论坑）。`OnEnter(TurnEnd)` 那组是 `( (a, b, c), (d, e).chain(), f, g ).chain()`：外层 `chain` 保证「元组整体 → f → g」，而**内层元组的 a/b/c 之间顺序不保证**。我把 `draw_lottery_system` 从 `g` 那位移进内层元组、想验证「它必须排在行情 tick 之后」这条断言有没有牙，结果**整组全绿** —— 不是断言没牙，是这次改动压根没有改变任何可观察的顺序（它本来就可能跑在 tick 前，也可能在 tick 后，测试掐不住）。把它移到**外层链的第一格**（严格在元组之前）之后，四条开奖用例立刻红。**判据**：要用 red-check 验一条顺序断言，破坏动作必须把被测 system 放到**另一个阶段的严格前/后**（外层 `chain` 的一个独立位置），不能只在同一个并行元组里换位；换位后全绿先怀疑破坏动作，别急着宣布「这条断言是假的」。另一半是同一族：两条 system 只要都带 `Books`（或任何同一个 `#[derive(SystemParam)]` 参数包），并排放进元组会在**调度期**直接 panic 成借用冲突（§6.1 坑 33 的同一条形状，只是碰撞的是参数包而不是查询），所以「该串起来的」与「可以并排的」这两件事在源码里必须由注释说清 —— P8 那条「`lottery_op_system` 必须排在 `decision_system` 之前」就是靠 `chain` 而非元组实现的，元组里写这个顺序等于没写。

37. **同一帧里两处 `next.set(...)` 谁赢，取决于「谁排在后面」—— 而跨调度边界时这件事没人保证**（P9 踩出来的，与坑 36 是同族但更阴：坑 36 是「换位不等于破坏」，这一条是「换位根本不在同一条链上」）。破产发生在落格结算里（`OnEnter(TurnPhase::Landed)` 那条链的 `property::settle_property_landing` 一步），而那条链的**末尾**是 `landing::close_landing_system`：「没人等回答就把回合收尾」→ `next.set(TurnEnd)`。拍卖机（P9）要在同一件事上写 `next.set(WaitDecision)`，而它注册在 `Update` 里（不能 gate 在 `WaitDecision` 上 —— 那一槌是掷骰前打的、破产在落格里，两边都得转得起来）。于是两条写在**同一帧**里撞在一起，赢的是后执行的那条，而 `OnEnter(状态)` 链与 `Update` 系统的相对次序不是本项目定的 —— 症状是「破产之后拍卖停在半空」或者「回合已经换人而拍卖还在等人喊」，两种都可能在某台机器上不复现。**判据**：谁都不许赌顺序，让**收尾那一条先问一句**（`close_landing_system` 现在读 `Res<Auction>` 并问 `holds_turn()`：`lot`／`standby`／`liquidation`／`requested` 四样任一非空就不收尾）。一般形式：**一个跨阶段的机器要占回合时，必须由「原本负责交还回合」的那一处把它的状态读进判据**，而不是两处各写一次 `NextState` 再赌谁后。
    连带一条借用纪律（坑 33 的第三种形状）：`Books` 这个参数包里揣着 `ResMut<'w, Auction>`（破产那一刻要就地记下待拍的料，而 `Commands` 的延迟写会让同一帧两处破产各看到「还没记」）。于是任何同时碰账本与拍卖机的 system **必须借 `books.auction` 那一格**，再拿一条 `ResMut<Auction>` 就是同一个 system 参数里两条对同一资源的可变借用 —— 参数建立期直接撞。本仓库已有三个用户按这条写：`auction::AuctionDesk`、`ui::dialog::sync_modal`（`ModalView` 里没有 `Res<Auction>`，读的是 `books.auction`）、`dev::force_auction`（反过来：只要 `ResMut<Auction>` 而**不带** `Books`）。
38. **谁清窗口，谁就得负责把回合交出去 —— 而「交出去」在 Bevy 里是一次显式的 `next.set`**（P13 踩出来的两处，形状与坑 37 相邻但责任相反）。`PendingDecision` 是「有人挂着待决事项」，`TurnPhase::WaitDecision` 是「回合机在等这一格的答案」，**两者是两个事实，清掉一个不会自动清另一个**。于是任何一处 `pending.clear()` 都必须当场回答「那这一回合怎么往下走」：
    - 魔法屋**落空那一条**（条件筛不出人）原来只 `pending.clear()` → 窗口没了、相位还钉在 `WaitDecision`，`decision_system` 找不到可答的东西，**整局静默停住**：不 panic、不 warn、画面还在动，只是再没有人行动。这一条真人**点不到**（面板上那颗按钮是灰的），送得进来的是 P14 的 AI 与 `NM_MAGIC=<没人命中的条件>` 那个旋钮 —— 而规则层必须假设有人送得进来。施法那一条才是鼠标一点就复现的那一种。
    - 魔法屋**施法完成那一条**同样只 `clear` → 同上。修法是施法那一步自己交还 `TurnEnd`；**但拍卖那一道法术反过来不许交**（`auction_hold_system` 会在下一帧把窗口挂到该喊价的人身上并写 `WaitDecision`），于是那里要留一个 `handed_to_auction` 的分支 —— 这条守卫的牙齿由「回合还该停在施法那位这儿」那一句断言钉着（第一次写它时删掉守卫**一条用例都不红**，因为那条用例只验「地块上了拍」）。
    - **同族的第三种形状在开发侧**：`NM_AUTOROLL` 的自动玩家只发 `RequestRoll`，而被 `StaySentence` 按住的人那一发被规则层原样退回（`roll_dice_system` 的 `if stay.is_some() { return; }`，那条判据是对的）→ 长跑停在**第 32 回合不动**，前面 31 回合跑得好好的。人按的是同一颗按钮（`ui::send_roll` 在按住时改发 `RequestStay`），所以自动玩家必须跟着这个分岔，而不是只跟着「掷骰」那个名字。`RequestStay` 的注释从 P4④ 起就写着「AI 与恶人也要发这一条，否则那个 `StaySentence` 没人消耗，回合会死锁」—— 恶人那一份 P12 接了，自动玩家这一份漏了 6 天，直到 P13 的长跑验收撞上新闻「豪雨袭城」（走路的那几位原地停留 1 回合）命中当前玩家。
    **一般形式**：凡是「一条路能把回合机停住」的地方，收尾责任要写在**做收尾的那一处**，并且有一条断言它真的收尾了的用例（不是断言「窗口关了」—— 窗口关了而回合停着，正是这类缺陷唯一的形状）。判据在测试里的样子是 `assert_ne!(phase(&app), TurnPhase::WaitDecision, "交还回合由这一步做")`（魔法屋那三条各钉一头：落空、施法、离开），而不是 `pending.active().is_none()`。

**agent 自查画面的办法（已落地，比请人肉眼验收可靠）**：0.20 内置 `bevy::render::view::screenshot::{Screenshot, save_to_disk}`，
`commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path))` 即出一帧 PNG；
`game_app` 里做成 `NM_SHOT=<路径>` 的环境开关（**按墙钟 1.5s 触发、2.5s 自行 `exit(0)`**，见上面第 5/10 条），
后续每个渲染阶段都能用它自动生成对比图。
（10-15 补）**别用 GDI 抓屏（`BitBlt`/`CopyFromScreen`）核对画面**：flip-model 交换链 + 硬件叠加层（MPO）之下，
桌面 GDI 抓到游戏窗口那块是**纯白**，看着像渲染挂了，其实窗口显示正常、点击也照常命中 —— 核对画面一律走 `NM_SHOT`
（GPU 侧截屏）；模拟鼠标冒烟用 SendInput，再靠 `NM_SHOT_AT` 错开时刻让应用自证点击后的状态。

B5 之后这条判据升级为「**每个界面都必须有一条不依赖鼠标键盘的入口**」，否则那个界面等于没被验收过。
现有的环境变量清单（全部只在开发侧生效，不进游戏逻辑）：

| 变量 | 作用 | 为什么需要 |
|---|---|---|
| `NM_SHOT=<路径>` / `NM_SHOT_AT=<秒>` | 截图并自行退出 / 推迟截图时刻 | 见第 5、10 条 |
| `NM_CAM_SIZE=<秒>` | 相机取景边长（`0` = 整盘） | §7.1 要求两个值都可配 |
| `NM_SCENE=board` | 跳过主菜单直接开局 | 菜单→对局的流转要能分别拍 |
| `NM_SEED=<十进制>` | 钉死对局种子 | 几百回合后崩一次，换种子就换一种死法，查无可查 |
| `NM_AUTOROLL=1` | 自动玩家：自动掷骰 + 接受报价 | 「打完一局」的验收不能靠人点几百次 |
| `NM_AUTOGAP=<秒>` | 自动玩家两次动作的间隔（默认 0.9） | 长跑设 0，否则一局要十几分钟 |
| `NM_AUTODECISION=0` | 只掷骰不回答 | 回合停在第一个 `WaitDecision`，模态才拍得到 |
| `NM_DECISION=buy\|upgrade\|rent\|card\|bank` | 直接挂出某个模态 | 升级要凑齐同色组、收据要有人踩过别人的地，一局里未必碰得到。⚠️ **它只写 `PendingDecision`、不碰回合机**（阶段留在 `WaitRoll`），而 `decision_system` 只在 `WaitDecision` 里跑 —— 所以这四个报价／收据屏上**那颗按钮点了没反应是设计如此**，不是缺陷（10-22 撞过一次：坐标量准、`fg=ok`、现金分文不动）。要用真鼠标走通回答，得让状态从正经路径进来（`NM_PLOTS` + `NM_AUCTION` 那一套就是这样用的），或按 `NM_AUTODECISION=0` 停在真的 `WaitDecision` 上 |
| `NM_ATM=1` | 开局即把 ATM 卡发到当前玩家手里并浮出 ATM 面板 | 卡 normally 来自「路过银行格」，而路过哪一格由骰子说了算，截图脚本控制不了 |
| `NM_STOCK=1` | 开局即浮出证券面板 | 顶栏那颗按钮要鼠标点（见 §6.1 坑 27），面板否则进不了截图 |
| `NM_HAND=1` | 开局即浮出手牌面板，并给当前玩家发六种形状的样例卡（每种 `CardTarget` 一张） | 道具牌的正经来源是百货公司（P3d② 已落），但要拍齐**每种按钮形状**（自我／对手／一条街／一块无主地／灰着）仍得逐张发到手 —— 一次进店只抽得到三个货位，凑不齐。样例卡按 id 找、走 `deal_to` 发（上限与挤档规则本来就在它里面），有一条测试钉着「这些 id 必须还在货架上」。代价：六张卡 + 六颗购地按钮会把这块 bottom 锚定的面板顶出屏幕上沿，排版归 P17 |
| `NM_SHOP=1` | 挂出百货柜台模态，并备齐进得了店的两样前提：一次换货 + 100 点券 | 这块面板的正经入口是**踩到 12 号那一格**，而那一掷由骰子说了算；货架与点券两个前提缺一个，面板就只剩一句灰话。**代价说清楚**：换货由 `Local<bool>` 保证只补一次，所以同一进程里开第二局不重新摆货（`dialog::dev_open_shop`） |
| `NM_PLOTS=<n>` | 把棋盘上前 n 块地划给当前回合玩家（Lv1、未抵押） | 银行柜台那一节「你的地产」要有地块才拍得到，而「名下有地」在正经玩法里同样要靠骰子踩中无主地。配 `NM_DECISION=bank` 就是柜台那一屏。**不走 `Books::buy`**：那条要扣现金、要有报价在前，而这个旋钮只想改归属，所以它**不发 `LandBought`、不动现金**（`dialog::dev_grant_plots`） |
| `NM_HOUSE=<地块号>:<层数>` | 给那块地盖上指定层数（归属与钱都不动；`0` 层＝拆平，拍对照图用） | 10-22 实报之后「**无主而立着楼**」是真状态（破产剩下的那些地就是），而它的两个来源都押不住截图时刻：破产要输掉一局，天使那几层要连踩同一格几次。买地模态上那个「买价」（地价 + 已盖的每一层）是那一阶段唯一要肉眼核对的报价。**就地写而不是 `Commands`**：延迟到下一次同步点才落地，而 `sync_modal` 可能就在本帧把面板建好 —— 那会拍出一张写着旧价的图（`Pending::Buy` 的签名带层数是第二道防线）。`parse` 单开纯函数配单测，理由同 `NM_GOD`／`NM_LOTTERY`：**拼错的旋钮值不崩也不红，只会「注入没发生」**（`dev.rs`） |
| `NM_PAUSE=1` | 开局即暂停 | ESC 按不了，暂停面板否则进不了截图 |
| `NM_PICK_TILE=列:行` | 开局即选中那格 | 点格详情卡是新增交互，没有鼠标可点 |
| `NM_DUMP=1` | 挂出 egui 只读 dump 面板（回合/阶段/落格分派/待决/资格/修正登记处 + 每个玩家与每块地） | 结算顺序、分派表、跨局残留这类问题的真相都在世界里，不在画面上；看数字比看像素可靠，而 T1 之前只能靠 `warn!` 一条条猜 |
| `NM_DAY=<tick>` | 每帧把日历钉在第 n 天（一天 = 一个回合结束） | 「今天休不休市」是 ATM/证券/柜台共 22 颗按钮的着色判据。⚠️ **P2b 之后这天不再是"第几个周六"**：回合对应真实历法（PLAN §三 日期系统），星期由锚定日推出来、不由骰子决定，所以这条旋钮的用途从「撞一个不靠运气的休市日」变成「**指定日历上的某一天**」——它同时是**圣诞节礼物**（P2b ④）那条一次性事件的截图入口。**代价不变**：钉住之后行情/派息/结息都不再往前走，它是截图旋钮不是时光机（`ui::pin_day`） |
| `NM_EPOCH=<YYYY-MM-DD>` | 钉住这一局的起始日期（默认由会话种子在 2048 年的 12 个月里抽一个 1 日） | P2b 之后**日期不再由 tick 唯一决定**：`NM_DAY` 钉 tick，同一个 tick 在不同 epoch 下是不同日子，所以「圣诞发卡」「节假日休市」这类按日子发生的事，截图必须先钉 epoch 再钉 tick。它同时是 2/29 那条集成用例的入口（`NM_EPOCH=2048-02-01`） |
| `NM_MENULOOP=<秒>` | 到点回主菜单、开一局、再回菜单、再开一局（每步间隔同样的秒数） | 场景来回切换的错误（§6.1 坑 22、坑 29）只在**第二局**暴露，不自动走这条路径就验不到。前两步是「自转 + 开局」，真正的新覆盖是后两步 |
| `NM_MAGIC=<menu\|条件>` + `NM_SPELL=<法术>` | 把当前玩家摆进女巫的两步法事（P13）：前者答第一步的条件、后者答第二步的法术 | 魔法屋的正经入口是「踩到那一格」，一局里撞不上截图时刻。两条旋钮而不是 `NM_MAGIC=girls:jail` 一条：两步之间隔着一次「筛人」，名单由规则层算完才开得出第二步窗口（与真玩家的两次点击同一形状）。⚠️ **别配 `NM_AUTOROLL`**（坑 38 的形状）：注入与自动玩家在同一帧各写一次 `NextState`，谁赢由调度次序决定 —— 输的那一版拍到的是「他滚完路、买地模态把女巫的窗口顶掉了」。正经配方是 `NM_SCENE=board` 单独跑，回合机自己会停在 `WaitRoll` |

**已定案（B4，10-14）：`Label3D` 的等价物走「UI 层投影跟随」**。三条路的取舍：

| 路 | 结论 |
|---|---|
| ① `Text2d` + 反算 billboard | ❌ `Text2d` 在 0.20 属 `bevy_sprite` 的 2D 通路，与 3D 不同渲染层，还要为它多挂一台 `Camera2d`；遮挡关系必然错 |
| ② 名字烘进贴图 + billboard sprite | ❌ 要自己引字体光栅化与图集管理，为十几个静态标签不值 |
| ③ `Camera::world_to_viewport` + UI `Text` | ✅ **采用**。一台 `Camera3d` 同时渲染 3D 与 UI（§6.1 已核实），中文与 HUD 共用同一套排版，且永远正对相机 —— 正是 billboard 语义 |

落地在 `game_app/src/render/labels.rs`：spawn 时每格一个 UI `Text`（记住自己的世界锚点），
每帧用 `world_to_viewport` 换算到屏幕并居中（`ComputedNode::size()` 是上一帧的物理像素，
乘 `inverse_scale_factor` 换逻辑像素，故居中有最多一帧滞后）。投影失败（跑到近/远平面外）
就 `Display::None`，不排版。**代价说清**：标签不参与深度，永远浮在建筑之上 ——
等轴测棋盘上「每格名字都读得到」比「被前面那栋楼挡住」有用，所以这笔换算是赚的。
`ZIndex(-1)` 让它排在 HUD 之下（同属 UI 层，靠 z_index 分先后）。

---

## 七、表现层重写要点

### 7.1 等轴测棋盘（源 `board.gd:168-188,206-302`）
- 相机：`Camera3d` 正交，rig 跟随棋子 XZ。源常量 `CAM_OFFSET=(8,16,8)`、`CAM_SIZE=8.0`（**代码值**，非文档的 25）；实际俯角约 54.7°，与 `PHASE_1_5_PLAN.md` 要求的 35.264° 不符 —— 移植时把两个值都做成可配置，肉眼定标一次。
  > **B4 落地（10-14）**：`CAM_OFFSET`/`CAM_SIZE`/`CAM_DIST`/跟随时间常数都在 `render/mod.rs` 的 `dim` 里；
  > 取景高度额外留了 `NM_CAM_SIZE` 环境变量（默认 8.0 = Godot 值，`0` = 整盘取景）。
  > 跟随用指数平滑（`tau=0.12s` ≈ 0.35s 收敛到 95%），跟的是**棋子的视觉位置**而不是 `TokenPosition`
  > —— 后者在掷骰那一帧就跳到终点，跟着它会把镜头甩在棋子前面。
  > 待肉眼定标：默认紧取景下 12 格棋盘一屏装不满，B5 有 HUD 之后要再确认一次「晕不晕」。
- 几何：全部程序化。路面 = 按 `abs(dcol)==1 && abs(drow)==1` 区分正交段/对角段的旋转薄盒（`board.gd:242-249` `rotation.y=atan2(-dz,dx)`）；格地板 = 薄盒（`tile_view.gd:92-98`）；同色组色条（`:109-127`）；建筑 = 按 level 堆叠的扁盒 + 归属色圆盘（`land_view.gd:75-117`、`tile_view.gd:129-143`）；棋子 = 半径 0.5 球（`player_token.gd:29-40`）。
- 世界坐标：`world(col,row) = (col*cell_size, 0, row*cell_size)`，`cell_size` 来自 JSON（ADR-001 §"网格→世界"）。**这条在 Rust 里就是 `BoardDef::world_pos()` 一个纯函数**，与渲染无关，可在 core 里单测。
- 材质：全 unshaded，无光照依赖 → Bevy 侧用 `Unlit2d` 或 `StandardMaterial{unlit:true}`，`ClearColor` 对应源 `Environment background_color=Color(0.6,0.6,0.7)`（`board.gd:179-188`）。Z 遮挡由深度缓冲处理，和 Godot 一致，无额外工作。
- ADR-001 的渲染约束要照搬：道路必须**可见**、建筑画在**土地格**（环路外侧）而不是道路格、道路格只画路面+组色条（`board-data-schema.md` §行为规则 6）。

### 7.2 UI（源 `scripts/ui/` + `board.gd` 里 `_build_ui`）
需要重建的清单（按 Godot 结构逐条对应）：
1. 顶栏 `TopBar`：提示文本 / 阶段文本 / 掷骰按钮 / 返回按钮（`board.tscn:33-67`）
2. `player_hud`：底部 `PRESET_BOTTOM_WIDE`，每玩家一张 `HBox` 卡（色点 + 姓名 + 现金 + 地产数），当前回合高亮（`StyleBoxFlat` 边框）
3. 4 个模态：买地 / 升级 / 租金通知 / 卡牌展示（`card_notice.gd` + `board.gd` 里的 dialog 构造）——遮罩 `ColorRect` + `PanelContainer` + 圆角 10/边框 2 + `autowrap WORD_SMART` 长文本
4. `main_menu`（`ColorRect` 背景 + `VBox` + 48 号标题 + 按钮 + `HSeparator`）、`pause_menu`（`process_mode=3 WhenPaused` + 半透明 Dim + `Panel`）
5. 尚未实现但计划内的股票 K 线图（`DEV_PLAN.md` Phase 7，Godot 侧靠 `_draw`）→ Bevy 侧自写 `Mesh` 折线或 `bevy_egui` plot

工作量提示：Bevy UI 是代码式 flexbox，**没有编辑器可视化摆放**。每次调间距/字号都要重编译看效果。缓解手段：① 把尺寸/颜色常量集中到一个 `ui_theme.rs`；② 用 `bevy_inspector`（若可用）或 `bevy_egui` 做实时参数；③ 接受"UI 迭代比 Godot 慢"这一事实并把它计入排期。`DEV_PLAN.md` Phase 7 原估 5-7 天，Bevy 侧我按 1.5–2 倍估。

**内部编号与屏幕读数之间只留一个翻译点**（10-22 加，P9 那轮「Lv1 到底是空地还是一层楼」的争论逼出来的）。`ui::building_label(level)` 把规则层的 `BuildingLevel`（`Lv1`＝买下来的空地，`rent[0]` 就是它的租金）翻成人话：`Lv0`/`Lv1` 都写「空地」，`Lv2` 起写「建筑 Lv1」，满级 `Lv6` 写「建筑 Lv5」。买地模态、升级模态、点格详情卡（两处各有它的租金表行）、柜台「你的地产」那一行的地块名与 `NM_DUMP` 地块栏**共用这一个函数**（五块屏、八个调用点）——屏幕上的数与 `estate_value` 里那个 `Lv−1` 必须同源，否则「这块 Lv2 为什么只值地价加一层」就要靠人脑补那一格。只有两处**故意**留着内部编号：`modal_stamp` 里那一格（它不上屏，只用来判「内容变了没有」）与 `NM_HOUSE` 那两条注入日志（旋钮的字面参数就是内部层数，翻成人话反而对不上敲进去的那个数）。内部编号从此只在规则层、测试流水与注释里活着。同类翻译点以后要加就照这个形状加（一个纯函数 + 一条单测 + 所有屏引用它），别在每块屏自己 `- 1`。

### 7.3 中文文本
Godot 端**全项目没有任何字体资源**，也没设 `gui/theme/custom_font`，完全依赖 4.7 内建 CJK 回退。Bevy **不做这种回退**，这是最容易在第一天踩空的点。当前方案（2026-10-13 已实跑验证）：
- **开发期**：`assets/font/dev_SourceHanSansSC-Regular.otf`（思源黑体 Regular，用户自备的开发授权版本）。走 `asset_server.load(...)` + `TextFont{ font: handle.into(), .. }`。**OTF/CFF 在 Parley 下渲染中文正常**，已用截图核对过格名与 UI 文案。该文件按 `assets/font/dev_*` 忽略，不入库。
- **发行前**：用 `fonttools` 子集化后入库。本项目字符集有限（格名、卡面、UI 文案），全量字体 ~10MB 没必要，子集能压到几十 KB，同时省掉 glyph atlas 压力。
- 备选：开 `bevy/system_font_discovery` feature + `FontSource::Family("...")` 可按系统字体族名加载，但**依赖装机字体**，不适合发行，只适合临时救急。
- 仍需观察：长句换行与 glyph atlas 上限（`TextBounds` + `LineBreak` 的组合）要到 B5 做卡牌文案时才有真实压力 ⚠️。

### 7.4 动画（只有三处）
| 源 | 效果 | Bevy |
|---|---|---|
| `player_token.gd:99-112` | 逐格 0.18s，落格 y 0.8→1.1→0.8 弹跳（`TRANS_SINE`） | `walk_step_system`（core 计步）+ `token_visual_system`（render 插值 + 弹跳曲线） |
| `board.gd:462-473` | 相机 0.35s `EASE_IN_OUT` 跟随 | `camera_follow_system`，指数平滑即可，源注释里"移动晕眩"→ 加大 `CAM_SIZE` 或降低跟随速度 |
| `player_token.gd:52-67` | 入狱/出狱 0.4s 位移 | `TokenVisual` 的 `Teleport{from,to,t}` 一次性插值 |

### 7.5 角色模型（不入库，10-15 定）
`assets/model/` 下现有两颗 glb（`character-ATubo.glb` 16MB、`character-SunXiaomei.glb` 14MB，合计 29MB），**按 `assets/model/` 忽略，不入库**。

理由不是「占空间」，是**改版次数**：git 对二进制没法 diff，每重新导出一次就在历史里永久多存一份完整拷贝，而模型恰恰是最常被重导的资产（绑定、缩放、材质改一轮就是一次）。字体是同一条道理（§7.3）。

**现在能这么做的前提**：棋子仍是程序化几何（`render/token.rs` 的半径 0.5 球），**没有任何代码引用这两个文件**，忽略它们不会让「克隆下来跑不起来」。这条前提一旦消失（渲染真的换成 glb），就得从下面三条里挑一条：

| 路 | 取舍 |
|---|---|
| ① 定稿后一次性入库 | 先做 draco/meshopt 压缩（29MB 通常能掉到几 MB），代价是仓库体积永久上去；但只要**不再重导**，历史里就只有一份 |
| ② Git LFS | 二进制该走的路，但要装 `git-lfs`、托管端有配额，且 clone 方必须能取 LFS 对象 —— 对单人开发的本仓库是净增复杂度 |
| ③ 一直留在仓库外 | 最轻，但「新机器 clone 下来跑不起来」会回来，且必须写清原件在哪 |

**出处（10-15 补，详见 [`.agents/CHARACTERS.md`](./CHARACTERS.md) §十一）**：自制资产，无第三方素材、无授权问题。
原件是工作区 `.tmp/characters.blend`（31MB，同目录另有未压缩导出 `Character-ATB.glb` 49MB、`Character-SXM.glb` 40MB），
入库的两颗由 Blender 5.2 的 Khronos glTF Blender I/O v5.2.40 导出。
各约 48k 三角面 / 35k 顶点，单 mesh 无骨骼无动画，一个材质、三张 2048² 贴图（baseColor / normal / metallic-roughness），
模型高 1.14 单位、脚底在原点、原生朝 +Z。

**前提已出现一处例外**：`crates/game_app/examples/character_preview.rs`（一次性试装场景）引用了这两个路径，
文件缺失时它自己 panic。不影响 `build` 与 `test`，但"没有任何代码引用"这句从现在起只对运行代码成立。

**观感结论**（同一份文档 §十一 末）：`render/` 里一个光源都没有、材质全 `unlit`，
PBR 模型直接搬进对局会黑成剪影；要用这批模型，先定美术方向（加主光重调配色，或改成半 unlit 平涂）。
尺寸上 1.0 缩放在 `CAM_SIZE=8` 下只有约 66px 高，比现在那颗球还小，要 1.6 倍才读得出脸。

**美术方向已定，这条不用再回答**：角色走平涂剪纸 sprite，不是 3D 模型（见 `.agents/CHARACTERS.md` §十二）。
这两颗 glb 退回形体参考。`assets/sprite/` 的横条比照本节同样不入库。

---

## 八、测试策略（迁移的主要收益）

- `cargo test` 三个层次：
  1. **纯函数单测**：`BoardDef::world_pos` / `destination` / `calculate_rent`（垄断 ×2、抵押 0、业主被关 0、level 越界）/ 股价公式 clamp / `serve_jail_turn` —— 不需要 ECS。
  2. **App 级单测**：`App::new()` + `CoreGamePlugin` + `init_state::<TurnPhase>()`，`app.update()` 步进，断言 component/resource。等价于现在的 `test_board_index_sync.gd`、`test_jail_flow.gd`。
  3. **对局级集成**：2 玩家 × 12 格跑 N 回合，固定 `GameSeed`，断言"不 panic、现金流守恒、终局可达"。这是现在 GUT 做不到但最该做的事（`DEV_PLAN.md:504` 数值平衡需要它）。
- GUT → Rust 的语义转换模板：`EventBus.xxx.connect(spy)` 类断言改为"测试里挂一个 `collect_messages_system`，把 `MessageReader<T>` 转存进 `Vec` resource 后断言"。`test_card_teleport_signal.gd`、`test_event_bus.gd` 这类"断言信号发过"的测试都走这个模式。
- 不可直译的测试（要**删掉**而不是硬搬）：`test_scene_flow.gd`（Godot 场景切换）、`test_land_view.gd`/`test_card_notice.gd` 中的 Control 树断言、`test_dialog_runtime.gd`（依赖 `process_mode`）。这些在新架构里由"UI 只是消息的读者"这一性质保证，或用 `bevy_egui` snapshot 手测。
- 质量门：`cargo fmt --check && cargo clippy -- -D warnings && cargo test --workspace`，**每一步都要看真实退出码**（不要用管道尾接 `tail` 之类吞掉失败码）。

---

## 九、存档、确定性与回放

`SaveManager` 在源项目是纯 TODO（`save_manager.gd:17-26`），所以这里是**净新增**，不是移植。好消息：数据模型已经天生可序列化（所有 core 类都刻意"不持有 Node 引用"，见 `player_data.gd:2` `board_tile.gd:4` 的注释）。

建议两层：
1. **稳定层（先做）**：手写 `GameSnapshot`（`serde`），字段 = `GameSession` + 玩家 Vec + land 状态 Vec + deck 顺序 + 股票价格/历史 + `TurnPhase` 与 `PendingDecision`。版本号 `save_version` + 迁移函数。目录用 `directories`/`directories-next`（Bevy 无内建"用户数据目录"，⚠️ 若 `bevy_settings` 能覆盖则优先用它）。
2. **实验层（后做）**：0.20 内置 `bevy_world_serialization`（`DynamicWorld`/`WorldAsset`）直接序列化世界。好处是加字段不用改存档代码；风险是 0.20 首发 API，可能自己也变。

顺带把 §4.2 的**种子化 RNG + 帧末批量消息**变成回放能力：存档只要保存 `GameSeed + 决策消息序列 + 玩家/回合数`，理论可整局重放（`DEV_PLAN.md` 表格中"存档系统：全局状态序列化、**回放**"的野心从这里开始变得廉价）。

> **10-20 状态**：整节未开工，排进 [PLAN.md](./PLAN.md) 的 **P15**（放在全部玩法之后）。✅ **两层方案的选择已拍板：走手写 `GameSnapshot`、不赌 0.20 首发 API**（就是 §十一 原本的倾向；PLAN §七 那行随之关闭，「实验层不做」不再是可复议的暂定）。另：P 序列里每个阶段仍要跑一次「回主菜单再开一局」的冒烟，跨局残留那条纪律（坑 22/25）不因为存档推迟而可以松。

---

## 十、分阶段移植计划（已归档）

> B0–B5b 的逐阶段产出、验收标准与两节验收记录（§10.1 / §10.2）整块搬到
> [`docs/archive/migration-b0-b5b.md`](../docs/archive/migration-b0-b5b.md)。移植线到此结束，「粗估 N 天」那一列随归档一起停更。
>
> **排期此后只有一份**：[`PLAN.md`](./PLAN.md)。移植线留下的 B6/B7/B8 三行已并成 PLAN 的 P14–P17（见 PLAN §六）。
> 本文档里仍然活着的只有：§三 缺陷清单的未结项（#8 后半条 → PLAN P1、#11 → P15）、§六 的依赖矩阵与 32 个坑（唯一那条「待修」的 #31 已在 P3b 修掉，10-19）、
> §七 的表现层待做（字体子集化、glb 处置）、§九 的存档方案（→ P15）。

---

## 十一、风险与待决问题

| 风险 | 概率 | 影响 | 应对 |
|---|---|---|---|
| **Bevy 0.20 太新**（发布 5 天），插件与文档滞后；0.21 一升级 API 又变 | 高 | 中 | 锁 `=0.20.0`；`game_core` 只依赖无渲染子集隔离冲击；**禁止 `cargo update`**；升级 Bevy 作为独立任务单做 |
| `bevy_inspector_egui` 无 0.20 版，失去运行时实体可视化（对 agent 调试影响最大的一项） | 已发生 | 中 | ✅ **已拍板（10-13）选 ①**：用 `bevy_egui` 自写只读 dump 面板。10-16 核对时还没落地，同日 **T1 落地**（`game_app/src/dump.rs` + `NM_DUMP=1`，见 §6.1 坑 28）—— 缺口已补，但它是**手写 Debug 打印**不是通用 inspector：新组件要手动往面板加一行 |
| UI 无编辑器、代码排版迭代慢 | 高 | 高 | B4/B5 排在逻辑之后；主题常量集中；预留 1.5–2 倍工时 |
| 中文渲染第一天就踩空（无内建回退） | 高 | 低 | 开工第一件事就是"屏上显示 3 行中文"的 spike（并入 B0 验收） |
| `bevy_settings` / `bevy_world_serialization` 是 0.20 首发 API | 中 | 中 | 存档走手写 `GameSnapshot`；设置项自己存 JSON 也行，不赌新 API |
| 双份实现漂移：Godot 目录仍在开发中，边移植边加功能会失去参照 | ~~中~~ | ~~高~~ | **已消解（10-15）**：移植线在 B5 追平，Bevy 转为主线，Godot 只留 ADR/逻辑出处与缺陷反例两个用途。附带事实：Godot 目录**不是 git 仓库**，本来也 diff 不出漂移 |
| 测试语义不可直译（信号断言、场景树断言） | 高 | 低 | §八 模板；明确列出删除清单 |
| 数值平衡需重估 | 中 | 中 | B3 的"现金流守恒 + 100 局 property test"就是为它准备的 |
| Rust 借用检查与回合制结算的冲突（一处结算要同时改两个玩家） | 中 | 中 | 已规避：状态放组件、用 `Commands` 延迟；`charge_rent` 这类跨实体操作用 `Entity` 句柄 + `single`/`get` 而不是同时 `iter` |

已拍板（2026-10-13）：
1. **Bevy 锁 0.20**（`=0.20.0`）。`bevy_inspector_egui` 缺口用 `bevy_egui` 自写只读 entity dump 补。
2. ~~**Godot 目录不冻结**，双边改动节奏由用户控制。→ §五 映射表因此升级为**必须维护**的资产：每移植完一个 Godot 文件就划掉并记录差异，防止两边行为悄悄分叉。~~
   **10-15 推翻**：Bevy 转为主线，§5.1 停止逐文件维护（10-16 已整块归档到 [`docs/archive/migration-b0-b5b.md`](../docs/archive/migration-b0-b5b.md)），画面与交互不再对齐。
3. **场景流转用单 App + state**（不用 SubApp / 多窗口）。

---

## 十二、下一步

> **本节原文是 B0 前夜写的**（"建 workspace""拷 `data/`""按 §十 顺序做 B1"），那些早已完成，留原话只会误导。10-16 起本项目的「下一步」只有一份，在 [`PLAN.md`](./PLAN.md) §四。

**下一步：[`PLAN.md`](./PLAN.md) 的 P3** —— 卡片：持有与主动出牌 + 命运定序 + 百货公司。P1（点券与穷尽分派表）、T1（egui 只读 dump 面板）、P2（结算修正管线 + 建筑损害阶梯 + 物价指数 + 日历收口）已在 10-16/10-17 完成；P2 留下的两个"第一批用户"就是 P3：`Corrections` 登记处（免罪/免费/嫁祸卡）与 `LandDamage`（怪兽/恶魔卡，顺带补上 prototype 里踩不到的那个缺口）。其后 P4……到 P13（魔法屋），玩法之后是 P14 AI、P15 存档与设置、P16 音频、P17 发行。

> **长期资产只剩两份**：§三 缺陷清单（未结项：#8 后半条 → PLAN P1、#11 → P15）与 §6.1 的 Bevy 0.20 坑清单。
> ~~§五 映射表~~ 与 ~~§十 进度表~~ 已于 10-16 归档到 [`docs/archive/migration-b0-b5b.md`](../docs/archive/migration-b0-b5b.md)，不再维护。
> **移植线收口**：B0–B5b 全部完成，逐阶段验收记录在归档文件里；此后的排期与待办只在 [`PLAN.md`](./PLAN.md)。
