# 移植记录 B0–B5b（已冻结）

> 这份文件是 [`.agents/ARCHITECTURE.md`](../../.agents/ARCHITECTURE.md) 在 2026-10-16 瘦身时移出的**纯历史部分**：
> 概念映射表（原 §五 / §5.1）与逐阶段移植计划及验收记录（原 §十 / §10.1 / §10.2）。
>
> **不再维护**（AGENTS.md 硬约束第 6 条：移植线在 B5 追平，此后不回头比对 Godot）。留在这里只回答两个问题：
> 「Godot 那个文件对应 Rust 侧哪个模块、当初为什么这样分」，以及「B0–B5b 每一阶段验收时到底看到了什么」。
>
> 文中出现的 §三 / §六 / §7.x / §九 指 [`ARCHITECTURE.md`](../../.agents/ARCHITECTURE.md)；
> **排期与待办只看** [`PLAN.md`](../../.agents/PLAN.md)（移植线留下的 B6/B7/B8 已并成 PLAN 的 P14–P17）。
> ARCHITECTURE.md 的原位置留了指针，章节号（§五、§十）不变，所以旧引用不会断。

---
## 五、概念映射速查表

| Godot | Bevy 0.20 | 备注 |
|---|---|---|
| `extends Node` autoload 单例 | `Resource` / `Message` | |
| `signal` / `emit()` / `connect()` | `#[derive(Message)]` / `MessageWriter<T>` / `MessageReader<T>` | 同步→帧末批量 |
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

### 5.1 已移植的 Godot 文件与行为差异

> **表格已于 2026-10-15 冻结**（原本是「每移植完一个就加一行，防两边悄悄分叉」）。移植线追平后 Bevy 转为主线，
> 这张表现在是 B0–B5 的历史存档：**读它来理解某个 Rust 设计为什么长这样**，不要再往里加行，也不要因为它缺一个
> Godot 文件就认为那功能没做。最后一列刻意的差异也不再是"偏离源行为"，只是本项目自己的决定。

| Godot 文件 | 落到哪里 | 与源行为的差异（都是刻意的） |
|---|---|---|
| `dice.gd`(26) | `rng.rs`（B2b） | `roll_seeded()` 假种子删掉；骰子走 `Rng` 的骰子子流 |
| `board_data.gd`(336) `board_tile.gd`(72) `tile_type.gd`(49) `board_branch.gd`(46) `land_plot.gd`(62) | `board.rs`（B1） | 旧格式 `tiles[]`+`position:float` 分支不搬（#9）；类型字符串换成 `TileKind`/`LandType` 枚举 |
| `player_data.gd`(105) | `player.rs` 组件 + `settle.rs` 的 `Books`（B2a/B3） | `status` 字符串换成 `JailSentence`/`Bankrupt` 组件存在性（#8）；起始现金单一来源（#12）；`add_cash`/`deduct_cash` 变成必发 `CashChanged` 的 `credit`/`debit` |
| `property_system.gd`(186) | `economy.rs` 判定 + `property.rs` 分派 + `Books::buy/upgrade/rent_of` | 垄断倍数走 `GameRules`；无 `group` 的地块不再白送垄断与升级（#14）；`can_upgrade` 多带一个「谁在升」参数 |
| `property_registry.gd`(128) | **整个消失** → `Owner`/`BuildingLevel`/`Mortgaged` 组件 + `ActiveBoard.lands` 索引 | 54 行旧 key 兼容层不搬（#9）；`"L"+land_id` 字符串 key 不复存在（#1 的根因） |
| `bank_system.gd`(105) | `economy.rs` 的 plan 函数 + `Books::deposit/withdraw/borrow/repay/apply_interest_all` + `bank.rs` | 可贷额度按地块算（#1）；四个动作返回「计划额」而不是直接改对象，改钱包仍走 `Books` |
| `stock_market.gd`(200) | `market.rs`（实体 + tick + 交易 + 分红）+ `economy.rs` 公式 | 两个 `Dictionary` 换成股票实体；`buy()` 不再递归（#5）；价格波动与成交拆成两条消息（#4）；`load_from_file` 由 `AssetServer` 取代 |
| `card_data.gd`(24) | `card.rs`（B1） | `effects` 从 `Dictionary` 换成 `CardEffect` 枚举 |
| `card_deck.gd`(88) | `deck.rs` | 洗牌走 `Rng` 的牌堆子流（#6）；牌堆存 `CardId`（下标）而不是持有卡面对象 |
| `card_effects.gd`(81) | `deck.rs::settle_card_landing` | 扣不起钱真破产（#3）；返回值是 `EffectOutcome` 结构而不是拼好的中文文案；找不到格名不再回退到起点 |
| `turn_manager.gd`(492) | `turn.rs` + `walk.rs` + `landing.rs`/`property.rs`/`bank.rs`/`market.rs`/`deck.rs`（B2b–B3） | 三处 `await` 变成 `Walking` 态；六个决策方法变成 `Decision{Accept,Decline,Acknowledge}` × `PendingDecision`；`turn_started` 每回合恰好一条（#2）；`_phase_to_string` 与 `phase_changed` 不搬 |

| `board.gd`(704) 的几何与相机部分（`:168-188,206-302`） | `render/board.rs` + `render/camera.rs`（B4） | 路面旋转用 `atan2(-dz,dx)`，**B0 那份写成 `atan2(dx,-dz)` 的 90° 错位已修**；分支也铺路（B0 漏了）；相机改成 rig 父子结构 + 指数平滑跟随，取景高度可配（见 §7.1）。`board.gd` 的 UI/对话框/信号连接簿记（`:339-704`）仍未移植（B5） |
| `tile_view.gd`(152) | `render/board.rs`（地板与组色条）+ `render/labels.rs`（格名） | 同色组标记条按「道路格 → 绑定土地」的真实朝向摆，不再固定 -Z；颜色取所绑 land 的 group，不再写死 `"blue"`（`:157-160` 那份是 Godot 自己的简化）。格名从 `Label3D` 换成 UI 投影（§六 定案），且带上主环下标 |
| `land_view.gd`(117) | `render/land.rs` | 等级方体不再随 `property_bought`/`property_upgraded` 信号重建 mesh，而是**一次铺满 `max_building_level` 个、按 `BuildingLevel` 显隐**；驱动源是 `Ref<Owner>`/`Ref<BuildingLevel>` 变化检测。归属色取玩家实体的 `PlayerColor`，不按 id 取模 |
| `player_token.gd`(117) | `render/token.rs` | 不再自己数步（core 的 `TileStepped` 才是节奏来源）；弹跳从「走完再弹两拍」合成成一步内的正弦弧；瞬移从「逐格走最短环」改成 0.4s 直线滑移；同格四人按 `PlayerId` 分角偏移（Godot 开局两颗球完全重合） |
| `scene_manager.gd`(67) | `scene.rs` 的 `Scene` 状态 + `OnEnter/OnExit` 链（B5） | 字符串场景路径换成 `Scene` 枚举，`change_scene` 的 `err != OK` 分支不复存在（转移不会失败）；`scene_changing`/`scene_changed` 两条信号不搬（转场动画在 B8 之前没有需求）；「切换前 `get_tree().paused = false`」变成显式 `reset_ui_state`：闸门与四份 UI 记忆一起清，漏一条就是**新局开局即冻结而画面正常**（§6.1 坑 22） |
| `board.gd`(704) 的 UI 与提示部分（`:339-704`、`:548-583`、`:537-538,668-679`） | `ui/top_bar.rs` + `ui/mod.rs` 的派发（B5） | 阶段名不再直接打内部枚举 `WAIT_ROLL`，改中文；提示行拆成「阶段旁白 + 事件流水」两条来源、事件覆盖阶段（Godot 里买地提示会被紧接着的「X 的回合」冲掉，本来就读不到）；`Over` 阶段**不写**旁白，否则晚一帧的 `GameOver` 胜者句会被盖掉；按钮可用态从「信号里记得刷新」改成「判据幂等、逐帧对齐」，`can_roll` 与派发同源 |
| `property_dialog.gd`(282) `upgrade_dialog.gd`(257) `rent_notice.gd`(171) `card_notice.gd`(187) | `ui/dialog.rs` 一个文件（B5） | 四份几乎逐字重复的 `_build_ui`/`_style_button` 合成 `frame`/`panel`/`info_row`/`button_row` 四个构件；**弹出时机由 `PendingDecision` 状态驱动，不再由信号驱动**（消息驱动会漏弹：弹窗 system 排在建报价之后时本帧消息读不到，而玩家那一帧已能点东西）；卡面文案仍来自 `CardDrawn`，因为 `Pending::Card` 刻意不携带字符串 |
| `pause_menu.gd`(57) + `pause_menu.tscn` | `ui/pause.rs`（B5） | `get_tree().paused` 换成 core 的 `Paused` 资源（规则层只读，写的永远是输入侧），四个推进型 system 挂 `run_if(not_paused)`；「谁是真相」只有一个 `PauseMenuOpen`，面板与闸门由它同时驱动，不会出现「菜单还开着但回合已跑」；ESC 之外补了顶栏「暂停」按钮（Godot 只用键盘，鼠标玩家找不到这条路径）；「设置」在 Godot 端也只是 `print("未实现")`，这里照摆但灰掉 |
| `player_hud.gd`(220) | `ui/player_hud.rs`（B5） | 每帧重建子节点改成「spawn 一次 + 按组件缓存改写」；地产数不再读玩家自己的 `properties` 数组，而是遍历 `Owner` 组件（与 #13 同源）；当前回合高亮走 `TurnOrder.current()`；`PRESET_BOTTOM_WIDE + offset_top=-120` 翻成 `bottom:8 + height:112`（§6.1 坑 21） |
| `main_menu.gd`(34) + `main_menu.tscn` | `ui/menu.rs`（B5） | 「读档」「设置」灰掉（B7 之前没有后端）；`quit` 走 `AppExit`；菜单需要一台 `Camera2d` 才画得出来（§6.1 坑 19） |
| —— | `ui/tile_info.rs`（B5，**源项目没有这条交互**） | 新增：点一格 → 右上角详情卡（名字/类型/位置/地价/归属/等级/整张租金表）。只认带 `TilePick` 的道路格地板，点建筑、棋子、空白都不开卡；内容全部回 `BoardDef` + 组件现算，实体上只留 `(col,row)` |

**尚未移植**：`autoload/audio_manager.gd`（空壳 → [PLAN.md](../../.agents/PLAN.md) **P16**）、`save_manager.gd`（空壳 → **P15**）、`game_state.gd` 的存档语义（→ **P15**，见 §九）、`entities/color_circle.gd`（死代码，不搬 #10）。`board.gd` 到 B5 已全部拆完（几何/相机 B4，UI/对话框 B5）。

---

## 十、分阶段移植计划

排期原则：**先让 `cargo test` 绿，再让窗口出现**。这与 Godot 端的顺序（Phase 1 就开渲染）相反，目的是把最大风险（表现层）留到架构确认之后。

| 阶段 | 产出 | 验收标准 | 粗估 |
|---|---|---|---|
| **B0** 骨架 ✅ 完成 (10-13) | workspace、`game_core`(`default_app` 无渲染)、`game_app` 等轴测棋盘、`.gitignore`、`check.sh` 质量门、`NM_SHOT` 截图钩子；拷 `data/` 与 `docs/` | `check.sh` 五项全过退出码 0；窗口渲染出棋盘 + 中文（`target/b0.png`） | 0.5 天 |
| **B1** 数据与棋盘 ✅ 完成 (10-13) | `BoardDef/LandDef/BranchDef/LandType/TileKind` + `CardSet/CardEffect`（内部标签枚举）+ `StockSet/StockDef` serde；`validate()` 覆盖 `tools/validate_board.py` 的 7 条不变式与卡牌/股票的字段约束；`JsonAssetLoader<A>` 自写（无第三方），`CoreGamePlugin` 注册三类资产与 loader，`game_app` 全走 `AssetServer` | `cargo test -p game_core` **15 项**全过；启动日志确认「棋盘 12 格 / 机会 7 张 / 命运 6 张 / 股票 5 只」；资产缺失时 panic 而非静默等待 | 1 天 |
| **B2a** 世界装配 ✅ 完成 (10-13) | `GameRules`/`GameSetup`/`DataHandles` 移进 core；`TileIndex`/`LandId`/`PlayerId` newtype；`Wallet`/`TokenPosition`/`JailSentence`/`Bankrupt`/`Owner` 组件；`ActiveBoard` resource；`begin_game_system` 铺土地与玩家实体 | `cargo test -p game_core` **19 项**（其中 4 项是 App 级：真资产装配世界、只装配一次、阵容与现金只来自 resource、棋盘缺失时不装配）；窗口日志「世界装配完成：12 主环格 / 6 土地实体 / 2 玩家」 | 0.5 天 |
| **B2b** 骰 + 回合状态机 ✅ 完成 (10-13) | `Rng`+`GameSeed`（测试固定、客户端取时钟）；`TurnPhase` 8 态（`Walking` 尚无生产者）；`begin_turn`/`roll_dice`/`resolve_landing`/`end_turn`/`game_over` 五个 system；`TurnOrder`/`GameSession`；7 条流程消息 | `cargo test -p game_core` **33 项**（21 单元 + 12 App 级）：整条回合流水逐字比对、同种子复现 / 换种子改变、非当前玩家的请求被忽略、破产者整轮跳过、服刑跳两格后出狱且组件被摘掉、只剩一人即终局、空阵容立即终局不挂死、20 回合不 panic 且回合计数与掷骰次数一致 | 1 天 |
| **B2c** 定步时行走 ✅ 完成 (10-14) | `WalkClock` + `WalkQueue`：把「掷出 N 点」展开成逐格推进，`WalkStarted`/`TileStepped`/`PlayerArrived` 取代瞬时落格；`GameRules.walk_step_interval`（0 即跳过动画，测试走 `GameRules::instant()`）；`BoardDef::walk_path` 纯主环展开 | `cargo test -p game_core` **41 项**（25 单元 + 16 App 级）全过：绕回起点经过、分支死路不进主环（两个方向都挡住）、interval=0 一帧走完与 B2b 等价（既有 12 个 App 用例不改语义全绿）、0.18s 定步节奏逐格推进；`NM_SHOT` 冒烟截图无退化 | 0.5 天 |
| **B3** 经济核心（无 UI） ✅ 完成 (10-14) | property（买/租/升级/抵押标记/同色组/等级差 ≤1）、bank（存贷息，**修 #1**）、stock（tick/买/卖/分红/历史，**修 #4 #5**）、card 效果枚举化（**修 #3**）、落格六段流水线 + `DecisionChosen` 闭环、`Rng` 分三条子流 | `cargo test -p game_core` **97 项**（56 单元 + 41 App 级）全过：`calculate_rent` 全分支表驱动、贷款额度端到端、破产释放地块、分红只在第 5 tick、部分成交不超付、四效果卡牌（含 #3 回归）、40 回合长跑现金流水可审计 + 无负现金 + 同种子复现整条经济流水；`NM_SHOT` 冒烟无退化（`target/b3.png`） | 1.5 天 |
| **B4** 等轴测渲染切片 ✅ 完成 (10-14) | `render/{board,land,token,camera,labels}.rs` + `RenderPlugin`：地面密铺、主环与分支路面（含对角，**顺带修掉 B0 的 90° 旋转错位**）、道路格地板按 `TileKind` 配色、组色条朝向绑定的土地、建筑按 `BuildingLevel` 显隐 + 按 `PlayerColor` 换色、棋子按 `TileStepped` 逐格插值带正弦弹跳、入狱/瞬移 0.4s 滑移、正交相机 rig 指数平滑跟随、`ClearColor`、格名走 UI 投影（§六 定案）；`NM_AUTOROLL` 开发用自动玩家（B5 没删，改成跑长对局的验收工具，见 §10.1） | `check.sh` 五项退出码 0；`NM_SHOT` 出图三张（`target/b4_static.png` 整盘、`target/b4_play.png` 跟随 + 30 回合后的建筑与现金流、`target/b4_wide.png`）；运行日志零 WARN。~~与 Godot 版并排肉眼对比~~ —— **10-15 取消**，画面不再对齐（见文首参照范围说明） | 1.5–2 天 |
| **B5** UI + 交互 ✅ 完成 (10-14) | 顶栏（提示行/阶段/掷骰/暂停/返回主菜单）、底部玩家 HUD、四个模态（买地/升级/租金收据/卡面）由 `PendingDecision` **状态驱动**、暂停菜单 + core 的 `Paused` 闸门、主菜单↔对局流转（`Scene` 状态 + 清场 + `reset_ui_state`）、`bevy_picking` 按钮与点格、**新增交互：点格显示详情卡**；开发侧旋钮 `NM_SCENE`/`NM_SEED`/`NM_AUTOGAP`/`NM_AUTODECISION`/`NM_DECISION`/`NM_PAUSE`/`NM_PICK_TILE`/`NM_MENULOOP` | 见 §10.1 的 B5 验收记录 | 2–3 天 |
| **B5b** 银行与证券的交易入口 ✅ 完成 (10-15) | **规则层**：`AtmAccess`（路过银行格发的那张卡，撑到持有人下一次 `TurnStart`）、`bank_op_allowed` 四种操作各自的窗口、`BankOpRequested`/`BankOpDone`/`StockTradeRequested` 三条请求消息、`bank_op_system` 认人不认阶段、行情改成**每回合结束 tick**、分红改挂**日历**（`days_per_month`/`dividend_day` + `date_of`/`is_dividend_day`/`days_until_dividend`）；`prototype.json` **不加证券格**（证券是随时可下的单，不是一格）。**表现层**：顶栏「ATM」「证券」两颗入口、两块常驻浮层（ATM 左上、证券左下，随世界状态刷新，键挂 `FinanceStamp`）、第五个模态「银行柜台」（踩到才开，八颗金额按钮 + 办完）、`UiAction` 新增四种动作、旁白补「存/取/借/还/买入/卖出」流水与「第 N 回合 · 几月几日」。开发侧新增 `NM_ATM`/`NM_STOCK`/`NM_DECISION=bank` | 见 §10.2 的 B5b 验收记录 | 1.5–2 天 |
| ~~**B6** AI~~ → **PLAN P14** | 移植线没有 AI 可参照（Godot 侧根本没有 AI），原描述（`Controller::Ai` + utility 决策 + 伪延迟）仍然有效，但**动作集未定型前写它必返工** | 移交 [PLAN.md](../../.agents/PLAN.md) | — |
| ~~**B7** 音频/设置/存档~~ → **PLAN P15/P16** | 拆两份：存档+设置＝P15（§九 的手写 `GameSnapshot` 方案不变）、音频＝P16（`audio_manager.gd` 是空壳，纯净新增） | 移交 [PLAN.md](../../.agents/PLAN.md) | — |
| ~~**B8** 打磨/发布~~ → **PLAN P17** | K 线图（§7.2 那条）、飘字、结算画面、i18n、**字体子集化**（§7.3，发行前必做）、Windows 打包 | 移交 [PLAN.md](../../.agents/PLAN.md) | — |

> **10-16：B6/B7/B8 整条尾巴移交 [`PLAN.md`](../../.agents/PLAN.md)**。移植线到 B5b 就没有 Godot 参照物了，这三行原本按「Godot 还差什么」排，而实际缺口已经变成「4 代有而我们没有」——那是另一条线、另一套依赖，继续写在这里会让两份计划互相假装对方在做。本文此后只保留 §三 与 §6.1 两份长期资产。

逻辑层（B0–B3）**已完成**，追平现有 Godot 进度并顺手修掉 §三 的 14 处缺陷（#1–#8、#12–#14；#9/#10 是「刻意不搬」，#11 随 PLAN P15 存档，**#8 的格子类型那半没成立**，见 §三 的 P1 归属说明）。表现层的几何与动画（B4）**已完成**。**B5 也已完成** —— 「能打完一局」这条线在 10-14 打通，验收记录见下面 §10.1。

### 10.1 B5 验收记录（10-14）

**「能完整打完一局（2 人类玩家）」**：`NM_AUTOROLL=1 NM_AUTOGAP=0 NM_SEED=20261014` 跑真窗口，
**55 回合后 `玩家 0 破产（付不出 18000）` → `对局结束：55 回合`**，屏幕上留下「胜者：孙小美！」、
败者卡片显示 `[破产]`、`掷骰` 自动变灰（`can_roll` 在 `Over` 为假）。同种子重跑两次结果逐字一致（55 回合、同一笔 18000），
说明 B5 新加的输入侧派发没有引入不确定性。自动玩家与按钮走的是**同一对消息**（`RequestRoll`/`DecisionChosen`），
所以这条长跑同时证明了按钮那条缝是通的。

**「长中文文案换行不溢出」**：逐屏截图核对过 9 张 —— 主菜单、对局空盘、等待掷骰（按钮亮）、
买地模态（南京路 / 外滩商业区两种配色）、升级模态、租金收据、卡面通知、暂停面板、点格详情卡（地产格 + 死胡同格两种）。
换行靠 `info_row` 的键列定宽 + 值列 `flex_grow`，父节点被宽度约束时 Parley 按 `WordBoundary` 切开 CJK（§6.1 坑 3）。

**B5 过程中发现并修掉的三个静默缺陷**（都是「回主菜单再开一局」这条路径上的，全部记进 §6.1 坑 22）：
`Paused` 跨局残留导致新局开局即冻结；`OpenModal`/`Local` 残留导致下一局第一次相同的报价永远不弹模态；
`Local<Option<Entity>>` 存着已被 `BoardUi` 收走的面板句柄。修法统一是**把「现在显示成什么样」的键挂到实体上**，
只有消息副产品（`RecentCard`）留在资源里由 `reset_ui_state` 清。另外 `Over` 阶段的旁白会盖掉胜者那句，改成该阶段不写旁白。

**测试**：`game_app` 新增 7 项 App 级测试（用 `MinimalPlugins` + 手搓 `PointerClick`
覆盖点击派发这一层：按到按钮**文字**要能沿父链找到按钮、待决事项/暂停/`Disabled` 三种情况必须拦住掷骰、
无动作实体（遮罩）不吃事件、`Decide` 的主语是待决玩家而非当前玩家、`CloseInfo`/`TogglePause` 只写 UI 资源）。
这条缝是 B5 唯一「错了不会立刻看得见」的地方，而它不需要窗口。

**B5 验收后用户实报并修掉的缺陷（10-15，§6.1 坑 23）**：主菜单「新游戏」「退出」点击无响应 ——
派发/悬停系统被误挂进 `UiSet::Live`（门是 `resource_exists::<ActiveBoard>`），主菜单上没有 ActiveBoard，
点击消息永远无人读；`NM_MENULOOP` 绕过派发，所以验收测不到。修法：两系统移出门禁、无条件跑；
新增**接线级**回归测试 `the_menu_dispatches_without_an_active_board`（真实 UiPlugin+ScenePlugin 接线 +
手拼 `PointerClick`，菜单态点「新游戏」必须切进对局 —— Harness 那批测不到接线错误）。
真人鼠标冒烟（SendInput）：点「退出」→ 进程 2s 内自退；点「新游戏」→ `NM_SHOT_AT=8` 自截 GPU 画面为对局棋盘。
`game_app` 测试现为 9 项全过（规则层 98 项不变）。

### 10.2 B5b 验收记录（10-15）

**做了什么**：B5 收尾时留下的两块欠账 —— 经济与证券的**逻辑与测试早就齐了，玩家却点不到**。这一阶段补的是入口，
顺带把两条与 Godot 不同的规则定下来（此后按 §一 的收窄声明，不再回头比对 Godot）。

**两条刻意的规则分叉**（都写在 `market.rs` 的文件头，读那个文件就能看到）：

| 分叉 | Godot 原来 | 现在 | 为什么 |
|---|---|---|---|
| 行情什么时候动 | 踩到证券格才 tick | **每回合结束动一次**（`OnEnter(TurnPhase::TurnEnd)`） | 没有证券格（见下），挂在那儿就是整局不动；而且「回合结束动一次」才是玩家能预期的节拍 |
| 分红什么时候发 | 每 `dividend_interval` 个 tick | **每月固定日**（`days_per_month` / `dividend_day`，默认 12 天一月、每月 1 日） | 「每月 1 号派息」是能说清的一句话；顶栏与证券面板因此都带上「第 X 月 Y 日（距派息 N 天）」 |

`prototype.json` **不加证券格**：证券是「掷骰前随时可下的单」，不是一格要踩的东西；把它做成格子，
就变成「不踩到就买不了」，与常驻顶栏的入口互相矛盾。骰子流、牌堆流、行情流本来就是 `Rng` 的三条独立子流，
所以行情改到每回合结束动一次**没有挪动任何一个骰子**（下面的同种子复现就是这条的证据）。

**三个设计决定**（都不是从 Godot 抄来的，是这一阶段现拍的）：
- **ATM 与柜台是两件事**：路过银行格发一张 `AtmAccess`（只能存取），踩到才开柜台模态（能借还）。
  那张卡**撑到持有人自己的下一次 `TurnStart`** —— 按「本回合有效」写的话，乙丙丁各走一轮就把甲的卡耗光了；
  所以 `begin_turn_system` 只在「即将开工的人正是持卡人」时回收它。回归测试在 `tests/finance.rs`。
- **柜台是模态，但内容是活的**：四个既有模态的文案在弹出那一刻烤死（钱不会中途变），柜台的余额却每笔操作都在变。
  做法是给 `ModalRoot` 加一条 `stamp`（只有 BankTeller 参与计算：`现金|存款|负债|还可借`），签名变了才重建面板。
- **浮层的刷新键挂在实体上**（`FinanceStamp`），不是 `Local` —— 与坑 22 同一条纪律，回主菜单随 `BoardUi` 一起消失。
- **ATM 面板在卡过期时不跟着清 `AtmPanelOpen`**：下次路过银行格会自己弹回来。试过「一起清掉」，
  代价更大 —— 装配前那几帧还没有持卡人，标志被抹成 `false`，`NM_ATM=1` 这张截图就永远拍不到东西（正是坑 25 那一族）。

**测试**：`cargo test --workspace` **133 项全过**（`game_core` 116 = 63 单元 + 12 finance + 25 settle + 8 turn_flow + 4 walk + 4 world_boot；
`game_app` 17）。新增的重点是 `game_core/tests/finance.rs` 那 12 项：路过发卡 / 踩到开柜台 / 四种操作各动对那一列 /
借超上限整笔作废且不留流水 / 还款封顶在负债 / **乙替甲办被拒** / 交易只在 `WaitRoll` / 行情每回合恰好动一次 /
分红只落在日历那天。`game_app` 侧新增 8 项接线级测试（浮层开关、金额按钮的主语、买卖下单、回菜单关浮层）。

**画面**：`target/` 下 6 张 —— `b5b_panels`（两块浮层同时开着）、`b5b_counter`（柜台模态，八颗金额按钮 + 办完）、
`b5b_btn_off` / `b5b_btn_on`（没卡 / 有卡时的顶栏 ATM 按钮）、`b5b_dbg`（真鼠标买 10 股后：现金 $19200、持股 10、卖按钮转绿）、
`b5b_dbg4`（真鼠标连点五笔存款后：现金 $15000 / 存款 $5000、取款两颗转绿、旁白「阿土伯 存了$1,000」）。

**长跑**：`NM_SCENE=board NM_AUTOROLL=1 NM_AUTOGAP=0 NM_SEED=20261014` → **55 回合、`玩家 0 破产（付不出 18000）`**，
与 B5 的记录逐字一致（同种子、同一步破产、同一笔金额）。这条同时钉住两件事：行情改到回合结束 tick 没动骰子流；
自动玩家对 `Pending::BankTeller` 回 `Acknowledge`（否则第一块银行格就把长跑永久挂住，而且是静默挂住）。

**真鼠标冒烟**（第一次走这条路，脚本 `target/nm_click.ps1`，一次性工具不入库）：
`SetForegroundWindow` + `GetClientRect`/`ClientToScreen` 换算客户区坐标，点顶栏「证券」→ 面板浮出，点「买 10」→ 成交。
这一趟把两个「看起来像 UI 挂了、其实不是」的误诊翻了出来，记成 §6.1 坑 26（运行时禁用不改字色）与
坑 27（合成点击要先激活窗口）—— 两者都不是拾取路径的问题，`dispatch_ui_actions` 一次都没漏派发。
