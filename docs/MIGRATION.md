# 《暴富》Godot → Bevy 移植方案

> 源项目：`E:\play\Godot\new-money`（Godot 4.7 + GDScript，进度到 Phase 1.5 / Phase 2 之间）
> 目标：`E:\play\coding\new-money-bevy`（Bevy 0.20，**彻底 ECS 化**）
> 文档版本：v1.0 ｜ 2026-10-13
> 本文所有 Godot 侧结论均带 `file:line` 出处；所有 Bevy 侧 API 名已在 0.20 官方文档/迁移指南核实，标 ⚠️ 的表示落地时需再确认。

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
| 8 | `player_data.gd:15,30,35,85,96` `turn_manager.gd:158,249-265` | 状态是裸字符串 `"active"/"jailed"/"bankrupt"`，格子类型是裸字符串 `"property"/"stock"/...`，比较散落各处（`board_tile.gd:72` 还有一份硬编码类型列表和 `tile_type.gd:31-35` 重复） | `#[repr(u8)] enum PlayerStatus` / `enum TileKind`，`match` 穷尽性由编译器保证，新增类型时漏分支直接编译失败 |
| 9 | `board_data.gd:258-304` `_compute_legacy_positions`（47 行 Curve3D 烘焙）+ `board_data.gd:6-9` 双格式分支 | 兼容"旧格式 `tiles[]`+`position:float`"，但 `data/boards/` 下**只剩 `prototype.json`**（新格式），旧格式棋盘文件不存在 | **不移植旧格式**，连带删除 `BoardData.has_coords`、`_legacy_positions`、`property_registry` 的 `set_owner/get_owner` 兼容层（`:73-106`，54 行）。这是移植带来的净删减 |
| 10 | `board_data.gd:63-67` `passes_start`、`game_state.gd:20` `inflation_index`、`scripts/entities/color_circle.gd`、`card_notice` 的 `HSeparator` 主题 | 死代码：`passes_start` 全项目无调用；`inflation_index` 只在 `test_game_state.gd:23-28` 被赋值和断言，不进任何公式；`color_circle.gd` 未被任何场景/脚本引用 | 不搬 |
| 11 | `game_state.gd:45` | 新开对局却 `EventBus.game_saved.emit(game_id)` | 消息语义对齐后自然消失 |
| 12 | `player_data.gd:98` + `game_state.gd:41` + `turn_manager.gd:39` | 起始资金 20000 硬编码三处（`START_CASH` 常量存在但 `reset()` 不走它） | 唯一来源 `GameRules` resource（可从 JSON 覆盖） |

> 第 9 条值得单独强调：**移植不是 1:1 搬运，删掉的兼容代码约占 core 的 15%**。

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
├─ tests/                      # headless 集成测试（App + app.update()）
└─ docs/                       # 从 Godot 拷来 ADR/schema/GLOSSARY
```

**为什么选"彻底 ECS"还要拆 `game_core`**：这两件事不冲突。"彻底 ECS" 指的是**规则用 component + system 表达，而不是 OOP 的 `PropertySystem`/`StockMarket` 对象**；拆 crate 指的是**这一层只链接 `bevy` 的无渲染特性子集**。收益：`cargo test -p game_core` 编译量小、跑得快、不需要窗口；渲染/UI 侧的 0.20→0.21 API 破坏不会波及规则层。**注意**：`default-features = false` 下 `bevy_ecs` 是否仍由 `bevy` 主 crate 无条件导出、以及需要显式开启哪些 feature（`bevy_state`、`bevy_time`），落地时以 `cargo doc -p bevy` 为准 ⚠️。退路是直接依赖 `bevy_ecs`/`bevy_app`/`bevy_state` 独立 crate，代价是要人工保证它们与 `bevy` 版本同步。

### 4.2 世界设计：实体 / 组件

| 实体 | 数量 | 组件 |
|---|---|---|
| 主环道路格 | N（prototype 12，正式 40） | `TileIndex(u32)` `TileKind`（enum，对齐 `tile_type.gd:11-27` 13 类）`GridPos{col,row}` `TileName`（`String`，仅显示用）`LinkedLand(LandId)?`（property 格才有）`BranchJunction{kind}`? |
| 分支死胡同格 | 每分支若干 | `BranchTile{junction:TileIndex, order:u8, kind}` `GridPos` —— 对应 `board_branch.gd:11` "从靠近接入点 → 末端"有序链 |
| 路旁土地 | 若干 | `LandId(u32)` `GridPos` `LandSize(u8=1\|2)` `LandDef`(Arc：group/land_type/price/rent[6]/upgrade_cost) `OwnerAddr(Option<PlayerId>)` `BuildingLevel(u8)` `Mortgaged`（marker） |
| 玩家 | 2–4 | `PlayerId(u32)` `PlayerName` `PlayerColor` `Wallet{cash:i64}` `TokenPosition{tile:TileIndex, off_board:Option<GridPos>}` `JailSentence{turns_left}` `StockHoldings(HashMap<StockId,u32>)` `BankBalance{deposit,loan:i64}` `Controller`（`Human \| Ai{difficulty}`）`Bankrupt`（marker，取代 `status=="bankrupt"` 判断） |
| 棋子（视觉） | 每玩家 1 | `TokenVisual{player:PlayerId}` `Transform` `WalkQueue{path,step_clock}` |
| 卡牌堆 | 2（机会/命运） | `Deck{kind, pile:Vec<CardId>, discard:Vec<CardId>}`，卡定义走 `Assets<CardDef>` |
| 股票 | 每只 1 | `StockId` `StockDef`(Arc：base/floor/ceiling/volatility/outstanding) `StockPrice{current:f64}` `PriceHistory(VecDeque<f64>，MAX 50)` |
| 相机/灯光/UI | — | 归 `game_app`，core 里没有 |

**关键取舍**：`PropertyRegistry`（`property_registry.gd`，128 行的弱类型字典）整个消失——owner/level/mortgaged 直接变成 Land 实体上的组件。同色组垄断判定（`property_system.gd:151-161`）、组内等级差不超过 1（`:164-178`）改为 `Query<&LandDef, With<OwnerAddr>>` 上的聚合，不再每次线性扫 `get_tiles_by_type`。

### 4.3 Resource（取代 autoload 单例）

| Godot 单例 | Bevy Resource |
|---|---|
| `GameState.game_id/turn_count/inflation_index` | `GameSession{id, turn_count, seed}` |
| `GameState.board` | `ActiveBoard{handle:Handle<BoardDef>, tiles:HashMap<TileIndex,Entity>, lands:HashMap<LandId,Entity>}` |
| `GameState.players` | 不需要 —— 玩家就是实体，用 `Query<&Wallet, With<PlayerId>>` |
| `TurnManager._players/_current_player_index/_pending_*` | `TurnOrder{active:Vec<PlayerId>, cursor:usize}` + `PendingDecision{player, kind}` |
| `TurnManager.property_system/stock_market/bank_system/...`（`:59-69` 7 个实例字段） | 全部消失，变成 system + 组件数据 |
| `GameRules`（新）：`START_CASH/JAIL_TURNS/MAX_LEVEL=6/DIVIDEND_INTERVAL=5/DIVIDEND_RATE/BASE_DRIFT/DEPOSIT_RATE/LOAN_RATE/MAX_LOAN_RATIO` | 一个 `GameRules` resource，值从 `data/rules.json` 读（现散落在 `player_data.gd:13` `turn_manager.gd:38-39` `stock_market.gd:14-17` `bank_system.gd:8-10`） |
| `DataManager`（`data_manager.gd`，手写 `_cache`） | `AssetServer` + `Assets<BoardDef>`/`Assets<CardDef>`/`Assets<StockDef>`，缓存与热重载免费得到 |
| `SaveManager`（**未实现**，`save_manager.gd:17-26` 是 TODO） | 见 §九 |
| `AudioManager`（未实现） | `bevy_audio`，`game_app` 侧 |

### 4.4 Message（取代 EventBus 的 24 个信号）

0.20 形态：`#[derive(Message)]` 结构体 + `app.add_message::<T>()` + `Commands::send_message()` + `MessageReader<T>`/`MessageWriter<T>`（存储为 `Messages<T>`）。按 `event_bus.gd:14-61` 的九域分组翻译：

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
| `begin_game_system` | `OnEnter(Boot)` | `Assets<BoardDef>` | spawn tile/land/player/deck/stock 实体；`TurnOrder`；→`TurnStart` |
| `begin_turn_system` | `OnEnter(TurnStart)` | `TurnOrder` `Query<(&JailSentence,&Bankrupt)>` | 破产→直接 `TurnEnd`；服刑→`JailProcessed`；正常→发 `TurnStarted`，→`WaitRoll` |
| `roll_dice_system` | `in_state(WaitRoll)` | `MessageReader<RequestRoll>` `Res<Rng>` | `DiceRolled`；算 `destination`；`TokenPosition`；`WalkQueue`；→`Walking` |
| `walk_step_system` | `in_state(Walking)` | `Res<WalkClock>` `WalkQueue` `ActiveBoard` | `TileStepped`；步数耗尽→发 `PlayerArrived`，→`Landed` |
| `resolve_landing_system` | `OnEnter(Landed)` | `TileKind` `LinkedLand` `Query<(&LandDef,&OwnerAddr,&BuildingLevel)>` `GameRules` | 分派：property→`BuyOffered`/`RentDue`+`DecisionChosen` 等待；stock→`StockTick`；bank→`InterestApplied`；chance/fate→抽卡并结算效果；road/start→直接 `TurnEnd` |
| `decision_system` | `in_state(WaitDecision)` | `MessageReader<DecisionChosen>` `Res<PendingDecision>` | 执行买/升级/确认，清 `PendingDecision`，→`TurnEnd` |
| `end_turn_system` | `OnEnter(TurnEnd)` | `Wallet` `GameSession` | `TurnEnded`；`turn_count+=1`；活跃人数 ≤1 → `Over`，否则推进 cursor → `TurnStart` |
| `stock_tick_system` | `in_state(Landed)` 之前 | `Query<(&StockDef,&mut StockPrice,&mut PriceHistory)>` `Res<Rng>` | `StockPriceChanged`；`tick%5==0` 分红 |

对照可见的三个简化：**不再需要** `add_child(tm)` 才能收信号的约束（`turn_manager.gd:16`）、`_dice_connected/_moved_connected` 布尔簿记（`:54-55`）、以及 `get_phase() != X: return` 这类手写守卫（`:194-195,221-222,317-318,326-327,...`，共 6 处）——`in_state()` 就是这个守卫的声明式版本。

### 4.6 唯一需要小心的架构问题：谁拥有"棋子位置"

Godot 端把这件事做成了"逻辑等渲染"（`turn_manager.gd:219` 等 `board.gd:451`）。彻底 ECS 化必须给一个答案，否则每个回合流程都要反向依赖渲染层，headless 测试又跑不了。

**建议：core 拥有逻辑位置与步时，render 只做视觉插值。**

- core 侧 `WalkQueue{ path: Vec<TileIndex>, elapsed: f32 }`，由 `Res<WalkClock>`（每帧 `+= Time::delta()` × `GameRules.walk_step_interval`）驱动，逐步发 `TileStepped`；`walk_step_interval` 取 `player_token.gd:17` 的 `0.18s`。
- render 侧 `TokenVisual` 读 `TokenPosition` + `TileIndex→Entity` 的世界坐标，自己做 `Transform` lerp（0.18s 对齐），不参与任何逻辑判定。
- 测试里把 `GameRules.walk_step_interval` 设成 0（或直接改 `WalkClock`），一步到位，无需等待；UI 上"跳过动画"就是把同一个值调 0——顺带实现 `DEV_PLAN.md:503` 想要的"动画速度"设置项。

这是本方案里唯一一处需要你在实施时守住的红线：**render 永远不向 core 发"推进回合"的消息**。

---

## 五、概念映射速查表

| Godot | Bevy 0.20 | 备注 |
|---|---|---|
| `extends Node` autoload 单例 | `Resource` / `Message` | |
| `signal` / `emit()` / `connect()` | `#[derive(Message)]` / `Commands::send_message()` / `MessageReader<T>` | 同步→帧末批量 |
| `enum Phase` + 手写守卫 | `#[derive(States)]` + `app.init_state::<T>()` + `run_if(in_state(..))` | `OnUpdate` 已移除 ⚠️ |
| `await token.arrived` | `TurnPhase::Walking` + `TileStepped`/`PlayerArrived` | 无协程 |
| `Resource` / `@export` 数据类 | `serde` struct → `Assets<T>`（配置）或 Component（运行时状态） | 二选一，不混 |
| `Dictionary` 弱类型注册表 | `HashMap<NewtypeId, Entity>` 或组件 | 修 bug #1 |
| `String status` 比较 | enum + marker component | 修 bug #8 |
| `randi_range` / `randomize()` | `Res<Rng>`（`SeedableRng` + `GameSeed`） | 修 bug #6 |
| `.tscn` 场景文件 + 编辑器摆放 | `bevy::scene` 或 spawn system | 无编辑器，代码摆放 |
| `get_tree().change_scene_to_file()` | 多 `SubApp` 或 state 驱动实体增删 | 主菜单/对局切换，需决策（§十一） |
| `Control`/`VBoxContainer`/`PanelContainer` | `Node` + `Ui` + flexbox（`DisplayFlex`/`Direction`/`Justify`/`Align`） | |
| `StyleBoxFlat`（圆角/边框/margin） | `Node::border_radius` + `border` + `BackgroundColor` + `Padding` | |
| `add_theme_font_size_override` | `TextFont`/`TextColor`/`FontSize` | |
| `Label3D`（`tile_view.gd:146-152`） | `Text2d`（世界空间文本）+ billboard | |
| `MeshInstance3D` + `BoxMesh`/`StandardMaterial3D` unshaded | `Mesh3d` + `Mesh::from(Primitive)` + `Unlit2d`/`StandardMaterial.unlit` | 0.16+ 图元走 `bevy_math::primitives` ⚠️ |
| `Tween` / `create_tween()` | 手写 lerp system（`bevy_tweening` 只有 ^0.19 版，未适配 0.20） | |
| `Camera3D` 正交 + `look_at` | `Camera3d` + `Projection::orthographic_*` + 自写 rig system | |
| `WorldEnvironment` background color | `ClearColor` + `AmbientLight` | |
| `DirectionalLight3D` | `DirectionalLight` | 材质 unshaded 时其实可有可无 |
| 点击 3D 格子选中 | 内置 `bevy_picking`：`DefaultPickingPlugins` + `Pickable` + `Pointer<Click>`（mesh_picking 后端） | `bevy_mod_picking` 已停更（配 bevy 0.14）❌ |
| GUT `assert_eq/assert_true` | `#[test]` + `assert_eq!` | |
| `print()` 调试（40+ 处） | `bevy::log` / `tracing` + `GameLog` resource | 顺带做回合日志与回放 |
| `gutconfig.json` + headless godot | `cargo test` | 迁移的主要动因 |

---

## 六、Bevy 0.20 依赖矩阵（已核实）

Bevy **0.20.0 发布于 2026-10-08**（5 天前），生态还没跟上，这直接约束选型：

| crate | 支持 0.20 的版本 | 判定 |
|---|---|---|
| `bevy` | `=0.20.0` | **精确锁定**，`[workspace.dependencies]` 统一 |
| `bevy_egui` | 0.43.1（bevy `^0.20.0`） | ✅ 可用 —— 调试面板、棋盘可视化编辑辅助 |
| `bevy_common_assets` | 0.18.0（bevy `^0.20.0`） | ✅ 可用 —— serde_json 资产加载；也可自写 `AssetLoader` 省掉 |
| `bevy_inspector_egui` | 最新 0.37.0，只配 bevy `^0.19` | ❌ **0.20 版未发布**。缺口：不能可视化 inspect 实体。缓解：用 `bevy_egui` 自写一个只读 entity dump（`ReflectedEntity`/`World::inspect_entity`），或等适配 |
| `bevy_tweening` | 最新 0.16.0，只配 `^0.19` | ❌ 不用，手写 lerp system（本项目动画就三种：逐格行走、落格弹跳、相机跟随，成本低于引插件） |
| `bevy_mod_picking` | 0.20.1（2024-07）配 bevy 0.14，停更 | ❌ 改用内置 `bevy_picking` |
| `serde` / `serde_json` | 独立版本 | ✅ 棋盘/卡牌/股票/存档 |
| `rand` / `rand_chacha` | 独立版本 | ✅ `ChaCha8Rng` 做种子化 RNG |
| `bevy_settings` | 0.20 内置新 crate（`SettingsPlugin` + `#[derive(SettingsGroup)]`） | 🆕 可选，音量/动画速度等设置项。太新，先 spike |
| `bevy_world_serialization` | 0.20 内置新 crate（`DynamicWorld`/`WorldAsset`，带 serde 特性） | 🆕 存档可用，但**同为 0.20 首发 API，风险高**，先做手写快照（§九） |
| 中文字体 | 0.19 起 `bevy_text` 换 Parley；0.19 加 `bevy/system_font_discovery` feature 与 `FontSource::Family`；0.20 把 `resolve_font_source` 改为 `FontSource::resolve_font_family` | 桌面端可按系统字体族名加载中文字体；发行仍建议自带子集（§七） |

> 如果 `bevy_inspector_egui` 的缺口你认为不可接受，唯一退路是**降到 bevy 0.19**（生态已适配面更广）。这是 §十一 的待决问题之一。

### 6.1 0.20 API 事实核对表（B0 已从本地 crate 源码 + 官方 examples 逐个核实）

| 事项 | 0.20 的确切形态 | 对方案的影响 |
|---|---|---|
| feature 模型 | `default = ["2d","3d","ui","audio"]`；`2d`/`3d`/`ui` 各自 = `default_app` + `default_platform` + `scene` + `picking` + 对应渲染层 | §4.1 的"`default-features=false` 即无渲染"成立，但要显式给 `features = ["default_app"]` |
| `default_app` | `async_executor, bevy_asset, bevy_log, bevy_state, reflect_auto_register` | **规则层需要的正是这个**：ECS + Asset + States + Log，不含 winit/渲染 |
| `default_platform` | `std, bevy_gilrs, bevy_winit, bevy_clipboard, default_font, multi_threaded, webgl2, x11, wayland, custom_cursor, sysinfo_plugin` | 只有表现层该链接它；注意 `default_font` 是 Fira Mono 子集，**无汉字** → 中文必现豆腐块 |
| 状态机 | `#[derive(States)]` + `app.init_state::<T>()`；`OnEnter/OnExit/OnTransition` 在；**`OnUpdate` 已删除**，改用 `run_if(in_state(T::X))`；转移 `NextState<T>::set()/set_if_different()` | §4.5 的表已按此写 |
| 消息 | `#[derive(Message)]` + `app.add_message::<T>()` + `Commands::send_message()` + `MessageReader/Messages`；观察者推送是另一套 `#[derive(Event)]` + `trigger` | §4.4 成立 |
| 世界空间文本 | **`Text2d` 已从 `bevy_text` 移到 `bevy_sprite`**（需 `2d` 或 `bevy_sprite_render` feature）；UI 文本根是 `bevy_ui::Text::new(s)`，子 span 用 `TextSpan`；样式是并列组件 `TextFont{font, font_size, ..}` / `TextColor` / `TextLayout::justify(..)` | `tile_view.gd:146-152` 的 `Label3D` 无直接对等物（见下） |
| 字体来源 | `FontSource::{Handle(H), Family(SmolStr), Families(&str), List(Vec), Generic}`；`system_font_discovery` feature 开启后可按族名加载 | §7.3 的两条路都可行；开发期用文件路径最确定 |
| 正交等轴测 | `Camera3d::default()` + `Projection::from(OrthographicProjection{ scaling_mode: ScalingMode::FixedVertical{ viewport_height }, ..OrthographicProjection::default_3d() })`（官方 `examples/3d/orthographic.rs` 自称就是给 isometric 用的） | §7.1 直接照此实现，B0 已验证 |
| 几何 | `Mesh3d(meshes.add(Cuboid::new(x,y,z)))` + `MeshMaterial3d(materials.add(Color::srgb(..)))` | 图元走 `bevy_math::primitives`，§五 的 ⚠️ 解除 |
| 3D + UI 共存 | 单个 `Camera3d` 即可渲染 UI 层（官方 `examples/3d/3d_shapes.rs` 如此），不必额外 `Camera2d` | HUD 无需多相机编排 |
| 拾取 | 内置 `bevy_picking`（`picking` feature）：`DefaultPickingPlugins` + `Pickable` + `Pointer<*>`；`bevy_mod_picking` 停更 | §五 的点击方案落地 |
| UI 交互 | `Button`/`Interaction` 在 0.20 已弃用，改 `OnInteraction` 组件式；`TextScroll` → `EditableText::viewport`；cursor 模块并入 `bevy_picking` | B5 写按钮时按新形态，别抄旧教程 ⚠️ |
| 存档/设置 | 新增内置 `bevy_settings`、`bevy_world_serialization`（`scene` feature 会带后者） | 同为 0.20 首发，按 §九 走手写快照 |

**B0 实跑发现的四条环境级坑（已踩过，记录避免重犯）**：
1. **`AssetPlugin::file_path` 在 0.20 是相对「可执行文件目录」的 `String`（不是 `Option`）**，而 `cargo run -p game_app` 的 CWD 是仓库根 → 默认会把资产解析成 `target/debug/assets/**`，所有资产**静默 404**（只有一行 `bevy_asset::server` ERROR，程序不崩）。落地做法：启动时把资产根解析成绝对路径再塞给 `AssetPlugin`，`std::fs` 与 `AssetServer` 共用同一个根，否则会出现"直读文件成功、Asset 加载失败"这种极难排查的组合。
2. **`default_font` feature 自带的是 Fira Mono 子集，无汉字**，所以不显式指定字体时中文不是"报错"而是**豆腐块/空白**。落地做法：把字体放 `assets/font/` 并用 `asset_server.load()` 显式指定（本仓库用开发授权的思源黑体，见 §7.3）。
3. **中文必须开 `bevy` 的 `complex_script_segmentation` feature**（转发到 `parley/complex-scripts`）。不开时 Parley/ICU4X 找不到中日韩分词模型，会**每帧刷** `ICU4X data error: No segmentation model for complex script: Chinese/Japanese`，且长句换行按非 CJK 规则处理。B0 实测：开启后错误归零。
4. **等轴测取景要自己算**：`Transform::from_rotation(...)` 只给旋转不给位移会让画面没有主体；正交相机的可视范围由 `ScalingMode::FixedVertical { viewport_height }` 决定，`default_3d()` 的 `near=0.0`。本项目按棋盘 AABB 推 `center`/`eye`/`viewport_height`（`main.rs::board_aabb`），视线方向沿用 Godot 的 `CAM_OFFSET=(8,16,8)`（即 (1,2,1) 对角、俯角约 54.7°）。
5. **`Screenshot` 的触发条件不能用帧数**（这条坑最贵，害我把「渲染没问题」误判成「3D 通路坏了」整整一轮）：`PresentMode::AutoNoVsync` 下前 40 帧只有一二十毫秒，3D 管线的着色器编译与网格上传还没热完，此时截出来的 PNG **只有 clear color 与 UI**，而窗口里其实是正常渲染的。判据：截图前后 md5 完全一致 → 说明截的是同一个未完成的中间态，而不是"画面没变"。正确做法是按**墙钟时间**触发（本项目 `time.elapsed_secs() >= 1.5`）。

**agent 自查画面的办法（已落地，比请人肉眼验收可靠）**：0.20 内置 `bevy::render::view::screenshot::{Screenshot, save_to_disk}`，
`commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path))` 即出一帧 PNG；
`game_app` 里做成 `NM_SHOT=<路径> cargo run -p game_app` 的环境开关（**按墙钟 1.5s 触发**，见上面第 5 条），
后续每个渲染阶段都能用它自动生成对比图。

**遗留待解（B4 处理）**：`Label3D`（格名跟着 3D 物体朝向相机）在 Bevy 没有等价物。三条路：① `Text2d` + 由相机朝向反算 billboard（但 `Text2d` 属 2D 渲染通路，与 3D 深度不同层，遮挡关系会不对）；② 把名字烘进贴图用 billboard sprite；③ UI 层跟随投影（点分世界→屏幕坐标）。B4 需要先做一个 billboard spike 再定。

---

## 七、表现层重写要点

### 7.1 等轴测棋盘（源 `board.gd:168-188,206-302`）
- 相机：`Camera3d` 正交，rig 跟随棋子 XZ。源常量 `CAM_OFFSET=(8,16,8)`、`CAM_SIZE=8.0`（**代码值**，非文档的 25）；实际俯角约 54.7°，与 `PHASE_1_5_PLAN.md` 要求的 35.264° 不符 —— 移植时把两个值都做成可配置，肉眼定标一次。
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

---

## 八、测试策略（迁移的主要收益）

- `cargo test` 三个层次：
  1. **纯函数单测**：`BoardDef::world_pos` / `destination` / `calculate_rent`（垄断 ×2、抵押 0、level 越界）/ 股价公式 clamp / `serve_jail_turn` —— 不需要 ECS。
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

---

## 十、分阶段移植计划

排期原则：**先让 `cargo test` 绿，再让窗口出现**。这与 Godot 端的顺序（Phase 1 就开渲染）相反，目的是把最大风险（表现层）留到架构确认之后。

| 阶段 | 产出 | 验收标准 | 粗估 |
|---|---|---|---|
| **B0** 骨架 ✅ 完成 (10-13) | workspace、`game_core`(`default_app` 无渲染)、`game_app` 等轴测棋盘、`.gitignore`、`check.sh` 质量门、`NM_SHOT` 截图钩子；拷 `data/` 与 `docs/` | `check.sh` 五项全过退出码 0；窗口渲染出棋盘 + 中文（`target/b0.png`） | 0.5 天 |
| **B1** 数据与棋盘 🚧 半 | `BoardDef/LandDef/BranchDef/LandType/TileKind` serde + `validate()`（含 `tools/validate_board.py` 的 7 条不变式）+ 9 组单测**已完成**；`CardDef/StockDef` serde、`AssetServer` 加载器**未做** | 移植 `test_board_data_prototype.gd`、`test_board_tile.gd`、`test_board_index_sync.gd`；**不移植旧格式路径** | 1 天 |
| **B2** 玩家 + 骰 + 回合状态机 | `PlayerId/Wallet/TokenPosition/JailSentence`；`Rng`+`GameSeed`；`TurnPhase` 全部 8 态与表内 8 个 system；`WalkClock` 定步时 | 移植 `test_dice`、`test_game_state`、`test_jail_flow`、`test_player_data`；**新增**：固定种子 100 回合不 panic 的 property test | 1.5 天 |
| **B3** 经济核心（无 UI） | property（买/租/升级/抵押/同色组/等级差 ≤1）、bank（存贷息，**修 bug #1**）、stock（tick/买/卖/分红/历史，**修 #4 #5**）、card 效果 enum 化（**修 #3**） | 移植 `test_bank_system`、`test_card_system`；`calculate_rent` 全分支表驱动测试；现金流守恒断言 | 1.5 天 |
| **B4** 等轴测渲染切片 | tile/land/token spawn system、路面（含对角）、建筑高度、`Text2d` 格名、正交相机 + rig 跟随、`ClearColor` | **肉眼**验收：与 Godot 版截图对比；此时才第一次开窗 | 1.5–2 天 |
| **B5** UI + 交互 | 中文字体接入；顶栏/HUD/4 个模态；`bevy_picking` 点格与按钮；`DecisionChosen` 闭环；主菜单→对局流转；暂停 | 能完整打完一局（2 人类玩家）；长中文文案换行不溢出 | 2–3 天 |
| **B6** AI | `Controller::Ai` + utility 决策 system（规则树版），发同样的 `RequestRoll`/`DecisionChosen`；伪延迟 `500–1500ms` | 3 AI + 1 人类可完局；AI 不阻塞（分帧） | 2 天 |
| **B7** 音频/设置/存档 | `bevy_audio` BGM+SFX、`GameRules.walk_step_interval` 挂到设置、`GameSnapshot` 存读、自动存档 | 存读回归测试：存档→读档→再存档字节一致 | 1.5 天 |
| **B8** 打磨/发布 | K 线图、飘字、结算画面、i18n、Windows 打包 | `DEV_PLAN.md` M3 里程碑 | 2 天+ |

逻辑层（B0–B3）约 **4.5 天**就能追平现有 Godot 进度并顺手修掉 12 处缺陷；真正的成本集中在 B4–B5（约 4–5 天）。总计约 **14 人日**（不含美术）。

---

## 十一、风险与待决问题

| 风险 | 概率 | 影响 | 应对 |
|---|---|---|---|
| **Bevy 0.20 太新**（发布 5 天），插件与文档滞后；0.21 一升级 API 又变 | 高 | 中 | 锁 `=0.20.0`；`game_core` 只依赖无渲染子集隔离冲击；**禁止 `cargo update`**；升级 Bevy 作为独立任务单做 |
| `bevy_inspector_egui` 无 0.20 版，失去运行时实体可视化（对 agent 调试影响最大的一项） | 已发生 | 中 | ① 用 `bevy_egui` 自写只读 dump 面板；② 或整体降到 0.19 换回生态 —— **需你决定** |
| UI 无编辑器、代码排版迭代慢 | 高 | 高 | B4/B5 排在逻辑之后；主题常量集中；预留 1.5–2 倍工时 |
| 中文渲染第一天就踩空（无内建回退） | 高 | 低 | 开工第一件事就是"屏上显示 3 行中文"的 spike（并入 B0 验收） |
| `bevy_settings` / `bevy_world_serialization` 是 0.20 首发 API | 中 | 中 | 存档走手写 `GameSnapshot`；设置项自己存 JSON 也行，不赌新 API |
| 双份实现漂移：Godot 目录仍在开发中，边移植边加功能会失去参照 | 中 | 高 | **冻结 Godot 端**为只读参照（新功能一律先 Bevy），或在本文 §五 表格里维护映射 —— **需你决定** |
| 测试语义不可直译（信号断言、场景树断言） | 高 | 低 | §八 模板；明确列出删除清单 |
| 数值平衡需重估 | 中 | 中 | B3 的"现金流守恒 + 100 局 property test"就是为它准备的 |
| Rust 借用检查与回合制结算的冲突（一处结算要同时改两个玩家） | 中 | 中 | 已规避：状态放组件、用 `Commands` 延迟；`charge_rent` 这类跨实体操作用 `Entity` 句柄 + `single`/`get` 而不是同时 `iter` |

已拍板（2026-10-13）：
1. **Bevy 锁 0.20**（`=0.20.0`）。`bevy_inspector_egui` 缺口用 `bevy_egui` 自写只读 entity dump 补。
2. **Godot 目录不冻结**，双边改动节奏由用户控制。→ §五 映射表因此升级为**必须维护**的资产：每移植完一个 Godot 文件就划掉并记录差异，防止两边行为悄悄分叉。
3. **场景流转用单 App + state**（不用 SubApp / 多窗口）。

---

## 十二、下一步

我建议的第一批动作（不需要你做更多决定，B0 的中文 spike 除外）：
1. 建 workspace + `game_core`（`bevy/default-features=false`）+ `game_app` 空窗口，把"屏幕上渲染出一行中文 + 一个等轴测方块"作为 B0 验收（**这是全项目最可能翻车的两个点，值得第一天就证伪**）。
2. 拷 `data/` 与 `docs/`（ADR-001、board-data-schema、GLOSSARY）进新目录 —— 它们是移植的规格来源。
3. 按 §十 顺序做 B1。

> 本文档的 §三（缺陷清单）和 §五（映射表）建议当作长期资产维护：每移植完一个 Godot 文件，就在表里划掉并注明差异，避免两边行为悄悄分叉。
