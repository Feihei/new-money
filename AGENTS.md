# AGENTS.md — 本仓库的工作约定

《暴富》Godot → Bevy 0.20 重写。**动手前先读 [`.agents/ARCHITECTURE.md`](./.agents/ARCHITECTURE.md)**：分层红线、Bevy 0.20 的 API 事实与 32 个坑位、待修缺陷清单全在那里（它原名《MIGRATION.md》，移植线收口后按实际内容改名）。**排期只看 [`.agents/PLAN.md`](./.agents/PLAN.md)**。本文只写约定与指路，不重复那些内容。

## 文档地图

| 位置 | 是什么 | 什么时候读 |
|---|---|---|
| [`.agents/ARCHITECTURE.md`](./.agents/ARCHITECTURE.md) | 架构与技术事实（活文档，**唯一被源码注释引用的一份**）。§三=缺陷清单与未结项，§四=目标架构与分层红线（§4.2 道路不建实体、§4.6 core 拥有位置），§六=0.20 依赖矩阵 + §6.1 坑 1–32（含 32：dev profile 的调试信息与 jobs 上限，本机死机的根因），§七=表现层要点与待做，§九=存档方案。§五 与 §十 已归档（原位留指针） | 任何碰 Bevy API、分层、结算顺序的改动之前 |
| [`.agents/PLAN.md`](./.agents/PLAN.md) | **玩法补齐计划**（活文档，唯一的排期来源）。基线是大宇《大富翁4》，不是 Godot 版。§一=与 4 代的口径偏差（哪些东西其实不存在），§三=缺口对照表，§四=**T1 + P1–P17 实现顺序**与依赖，§五=点券格/小游戏格/新闻中心格设计，§六=AI/存档/音频/发行的安插，§七=待拍板（含已拍板项） | 碰「要不要加某个玩法/格子」；动手做任何阶段之前 |
| [`docs/archive/migration-b0-b5b.md`](./docs/archive/migration-b0-b5b.md) | **冻结的移植历史**，不再维护：Godot→Bevy 概念映射表（原 §五/§5.1）+ B0–B5b 逐阶段产出与两节验收记录（原 §十/§10.1/§10.2） | 只在这两类情况下去查：「Godot 那个文件对应 Rust 哪个模块」、「当初验收时看到了什么」 |
| [`.agents/ADR-001-board-topology.md`](./.agents/ADR-001-board-topology.md) | 棋盘拓扑决策，状态 **Accepted**（权威，不要重开讨论） | 碰 board / land / 移动逻辑 |
| [`.agents/board-data-schema.md`](./.agents/board-data-schema.md) | 棋盘 JSON schema 与不变式（权威） | 改 `assets/data/**` 或对应 serde 结构 |
| [`.agents/GLOSSARY-board.md`](./.agents/GLOSSARY-board.md) | 棋盘术语表 | 命名与讨论时用统一用语 |
| [`.agents/CHARACTERS.md`](./.agents/CHARACTERS.md) | 角色设定（活文档）：三层结构与配比、入境诨名命名法、首发十二人档案、**开局参数表 + AI 四维**（角色不带规则分支，见其 §八）、**神明系谱**（四部下发钱、赵公明收钱＝巡回赛骨架，提案未拍板）、沈万三＝隐藏位、**IP 边界与红线清单**、**美术方向＝会动的剪纸 sprite（§十二，含 I2V 管线与踩坑）**、两颗 glb 退回形体参考 | 碰角色/文案/UI 文本/`assets/sprite`/`assets/model`/P14 的 AI 先验；任何要往 UI 或宣传里写角色名的时候 |
| [`docs/godot/DEV_PLAN.md`](./docs/godot/DEV_PLAN.md) / [`docs/godot/PHASE_1_5_PLAN.md`](./docs/godot/PHASE_1_5_PLAN.md) | 附件：Godot 源项目文档的**原样副本**。已知与代码偏离 → 以 Godot 代码为准，文档只作设计意图参考。副本内的相对路径指向 Godot 仓库，本仓库不对齐 | 需要原始需求、里程碑背景时 |
| [`README.md`](./README.md) | 面向使用者的项目说明与快速开始 | 需要给外部解释这个项目时 |

## 命令

常用命令见 [`README.md` 的快速开始](./README.md)；agent 侧额外需要的是：

```bash
./check.sh                  # 五项质量门，逐步取退出码；提交前必须退出码 0
cargo test -p game_core      # 只跑规则层（无窗口，编译量小）
cargo clippy --workspace --all-targets -- -D warnings
```

- **不要用 `... | tail` 之类管道判断成败**，`check.sh` 已经逐步取退出码，照它做。
- **别给 cargo 加 `-j`，也别把根 `Cargo.toml` 里 `[profile.dev]` 的两个 `debug` 键调回去**：这台机器 12 核 / 32 GB，`.cargo/config.toml` 把并行度钉在 6，而 `cargo test -p game_core` 是「1 个库 + 10 条集成测试二进制」。实测（数字见 §6.1 坑 32）：默认 12 路时工具链私有内存 18.2 GB，把依赖调试信息加回去之后 3 路就 29.7 GB 物理内存 —— 连续两次整机死机就是这么来的。清缓存也别用裸 `cargo clean`（那会把全部依赖重编 ~21 分钟），要清就 `cargo clean -p game_core`。
- 核对画面：`NM_SHOT=<路径>` 让 `game_app` 截图后自行退出，`NM_SHOT_AT=<秒>` 推迟到对局跑起来之后再截（按墙钟触发，原因见 `.agents/ARCHITECTURE.md` §六 的坑 5/10）。`NM_CAM_SIZE=0` 整盘取景、默认 8 跟随紧取景。别用 `timeout N cargo run` 做冒烟测试。
- B5 之后 `NM_AUTOROLL=1` 不再是「没有按钮的替代品」，而是**跑完一局的手段**（人点几百次不现实）：配 `NM_AUTOGAP=0` 提速、`NM_SEED` 复现。浮层与模态的开发入口是 `NM_ATM=1`/`NM_STOCK=1`/`NM_DECISION=bank`，日历入口是 `NM_DAY=<tick>`（P2④ 加的：哪天休市由骰子决定，截图控制不了就得给一条不靠运气的路）。要看世界状态本身（回合/阶段/落格分派/待决/每人两套账/物价指数与修正登记处），用 `NM_DUMP=1` 开只读面板 —— 查「钱为什么是这个数」比猜 `warn!` 快。
- **真人鼠标冒烟要先激活游戏窗口**（`SetForegroundWindow` + `GetClientRect`/`ClientToScreen` 换算客户区坐标）：窗口被 IDE 压住时点击全落到别人那儿，而 `NM_SHOT` 照常截出游戏画面，看着就像「UI 是死的」（§6.1 坑 27）。**同一条坑的另一半：点击脚本的计时从「它找到窗口」起算，`NM_SHOT_AT` 从「游戏启动」起算** —— 启动前多 `sleep` 几秒，点击就整体落到截图之后，看着同样是「按钮是死的」。先确认点击时刻早于截图时刻。

## 硬约束

> **章节号约定**：本文里裸写的 `§三`/`§4.x`/`§6.1`/`§7.x`/`§九`/`§十` **一律指 `.agents/ARCHITECTURE.md`**；提到玩法计划的章节会写成 `PLAN §一`…`PLAN §七` 这种带名字的形式。两份文档都用中文数字编号，不带名字就会撞车。

1. **Bevy 锁 `=0.20.0`，禁止 `cargo update`**。升级 Bevy 是独立任务，不与功能改动混在一起。
2. **分层红线**：`game_core` 只链接 bevy 的无渲染子集（`default_app`），不得引入窗口/渲染/UI 依赖；`render` 永不向 `core` 发"推进回合"的消息——core 拥有逻辑位置与步时，表现层只做视觉插值（§4.6）。
3. **旧棋盘格式刻意不移植**（`tiles[]` + `position:float` + Curve3D 烘焙）。以 ADR-001 的新格式为唯一路径，不要写兼容层。
4. **道路格不建实体**，只有土地建实体；拓扑查询走 `BoardDef`/`ActiveBoard`，别复制第二份真相来源（§4.2 的 B2a 修正）。
5. **移植时顺手修掉 §三 缺陷清单里的条目**，不要照搬 Godot 的错误行为；修完在 §三 的表下方标注归属阶段与覆盖它的测试。
6. **Godot 端从 2026-10-15 起不再是参照主线**（B5 之后拍的）。它只剩两个用处：前期 ADR/规格出处，和 §三 缺陷清单的反例。**画面、UI 排版、交互路径一律不对齐**，Bevy 侧按自己的判断设计；经济/规则要改动也不必回头比对 Godot。§五 映射表因此**停止逐文件维护**，作为 B0–B5 的历史记录保留，不要再往里加行。
7. `assets/font/dev_*` 是开发授权字体（已 gitignore），**不要提交字体**；发行前必须换成可分发的子集（§7.3）。
8. `assets/model/` 的 glb 与 `assets/sprite/` 的横条 **都不入库**（已 gitignore）：二进制没法 diff，每重导一次就在历史里多存一份完整拷贝；sprite 那条还额外在改（风格、帧数、白边宽度都没定稿）。现在忽略它们不会让克隆跑不起来，因为**没有任何运行代码引用**它们、棋子仍是程序化几何（唯一的例外是 `examples/character_preview.rs` 这个一次性试装场景，缺文件时它自己 panic，不影响 build 与测试）；等渲染真的换成 sprite 或 glb，再按 §7.5 的三条路重新评估。别顺手 `git add -f` 它们。glb 的出处见 [`.agents/CHARACTERS.md`](./.agents/CHARACTERS.md) §十一，sprite 的生成管线与重跑方法见同文件 §十二。

## 提交与文档维护

- Commit message 用中文 conventional commits，标题带阶段号，沿用现有风格：`feat(B2c): 定步时行走……`。
- 进度写回**一份**文档就够了：玩法阶段做完写 `.agents/PLAN.md`（§四 的 P 表加状态 + §七 拍板项落定）；过程中顺手修掉缺陷时，在 `ARCHITECTURE.md` §三 标归属与覆盖它的测试。**移植线不再回写进度** —— 那张阶段表已冻结归档，需要查验收细节去 `docs/archive/migration-b0-b5b.md`。**一个阶段在途只有一份计划**，别为单个阶段再开第三份。
- 质量门结果、测试计数以实际运行为准，不沿用文档里的旧数字。
- 行尾由 `.gitattributes` 锁定 LF。

## 当前状态

B0、B1、B2a、B2b、B2c、B3、B4、B5、**B5b** 已完成（数据层、世界装配、回合状态机 + 种子化骰子 + 定步时行走、经济核心 property/bank/stock/card、等轴测渲染切片：几何/建筑/棋子动画/相机跟随/格名投影、UI 与交互：顶栏/HUD/四个模态/暂停/点格详情/主菜单↔对局流转、B5b：ATM 与证券两块常驻浮层 + 银行柜台模态 + 行情按回合 tick + 分红按日历发，缺陷 #1/#3/#4/#5/#13/#14 已修，#8 的格子类型那半随 P1 收口；B5 的验收记录（55 回合打完一局、9 张界面截图、三个跨局残留缺陷的修法）见 [`docs/archive/migration-b0-b5b.md`](./docs/archive/migration-b0-b5b.md) §10.1；验收后实报的「主菜单按钮点击无响应」已修（`ARCHITECTURE.md` §6.1 坑 23，真鼠标冒烟通过）。B5b 的验收记录（两条规则分叉的取舍、6 张截图、真鼠标冒烟翻出的两个误诊）见同一份归档 §10.2。

**移植这条线已追平**：Godot 侧非 addon 源码在 B5 全部有了归属，剩下的 `audio_manager`/`save_manager` 是空壳，B6–B8（AI、存档、K 线）是 Godot 没有的东西。按上面第 6 条，此后不再回头比对 Godot。

当前在做的两块欠账（都是「两边都没接上」的，不是移植遗漏）：
- ~~**银行/证券没有交易入口**~~ —— **B5b 已补**：路过银行格发 ATM 卡（存取），踩到开柜台模态（借还），证券常驻顶栏、掷骰前可下单。
- ~~**`prototype.json` 没有证券格**~~ —— **拍板不补**：证券不是一格要踩的东西。行情改成每回合结束 tick，分红改挂日历（`days_per_month`/`dividend_day`），两条分叉的理由见 [`docs/archive/migration-b0-b5b.md`](./docs/archive/migration-b0-b5b.md) §10.2。

下一步换成**玩法补齐**这条线，排期只在 `.agents/PLAN.md`：**P1 已完成（10-16）** —— 点券（`Points` 组件 + `PointsChanged`，与现金两套账、永不通兑）+ 点券格（`TileData.coupon`，一次性 + 按回合冷却）+ 落格分派改成一张穷尽表 `landing::domain_of`（新增格类型漏写分支＝编译失败，ARCHITECTURE §三 的 #8 后半条同时收口）。**T1 也已完成（10-16）** —— `NM_DUMP=1` 的 egui 只读面板（`game_app/src/dump.rs`，core 一条依赖没加），它第一次跑真鼠标冒烟就翻出一个 B5 就带着的必崩缺陷：「对局→主菜单」从没被走过（§6.1 坑 28/29）。**P2 已完成（10-17）** —— 结算修正管线（`game_core/src/charge.rs`：`Charge` + 一张 `Corrections` 登记表 + 纯函数 `stage`，每条账带「可否抵消／可否转嫁」两个位）、建筑损害阶梯（`damage.rs`：`LandDamage::{Degrade,Clear{seize}} × DamageArea::{One,Box,Street,Whole}`，「降层＋夺地」那一格在类型上写不出来）、物价指数（`GameSession::price_index`，白名单豁免股价与神明金额）、日历收口（利息挂 `is_month_end`、周末同时挡行情与银行，利率改成月息 10%/15%）。测试计数以 `./check.sh` 实际运行为准，P2 之后是**规则层 174 项 + 表现层 20 项**。**P3 已开工，P3a 完成（10-19）** —— 手牌地基：`hand.rs`（`Hand` 有序 + 上限 15、满了挤掉最前面那张并写进 `CardGained.pushed_out`）+ 出牌入口 `PlayCardRequested{player, card, aim}` → `play_card_system`（窗口与证券同一条：`WaitRoll` 且无待决事项；**刻意不挂 `Pending::PlayCard`**，理由见 PLAN §四 的 P3a 落地记录）+ `afflict.rs`（`Affliction`/`Insulation`：免罪抵消的是「事」不是钱，所以不挂 `Corrections`）+ 第三副池 `item_cards.json`（陷害 / 免罪两张，`CardTarget` 让同一句 `GoToJail` 既能给自己也能给对手）。P3a 之后是**规则层 196 项 + 表现层 20 项**；余下 P3b 手牌浮层、P3c 地块限时状态容器＋涨价·查封·免费、P3d 命运定序＋百货公司＋抵押赎回。再往后是 P4 拘留统一模型……到 P13 魔法屋，玩法之后才是 P14 AI、P15 存档设置、P16 音频、P17 发行。原 B6/B7/B8 三行已从移植计划表（已归档那份）并进来。
