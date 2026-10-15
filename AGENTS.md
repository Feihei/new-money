# AGENTS.md — 本仓库的工作约定

《暴富》Godot → Bevy 0.20 重写。**动手前先读 [`.agents/MIGRATION.md`](./.agents/MIGRATION.md)**：移植方案、阶段计划、Godot→Bevy 映射表、待修缺陷清单、Bevy 0.20 的坑位全在那里。本文只写约定与指路，不重复那些内容。

## 文档地图

| 位置 | 是什么 | 什么时候读 |
|---|---|---|
| [`.agents/MIGRATION.md`](./.agents/MIGRATION.md) | 移植方案（活文档）。§三=待修缺陷清单，§五=概念映射速查，§六=0.20 依赖矩阵/API 核对表+环境级坑，§十=B0–B8 阶段与验收 | 任何非单点修复的改动之前；每完成一个阶段回来更新 |
| [`.agents/ADR-001-board-topology.md`](./.agents/ADR-001-board-topology.md) | 棋盘拓扑决策，状态 **Accepted**（权威，不要重开讨论） | 碰 board / land / 移动逻辑 |
| [`.agents/board-data-schema.md`](./.agents/board-data-schema.md) | 棋盘 JSON schema 与不变式（权威） | 改 `assets/data/**` 或对应 serde 结构 |
| [`.agents/GLOSSARY-board.md`](./.agents/GLOSSARY-board.md) | 棋盘术语表 | 命名与讨论时用统一用语 |
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
- 核对画面：`NM_SHOT=<路径>` 让 `game_app` 截图后自行退出，`NM_SHOT_AT=<秒>` 推迟到对局跑起来之后再截（按墙钟触发，原因见 `.agents/MIGRATION.md` §六 的坑 5/10）。`NM_CAM_SIZE=0` 整盘取景、默认 8 跟随紧取景。别用 `timeout N cargo run` 做冒烟测试。
- B5 之后 `NM_AUTOROLL=1` 不再是「没有按钮的替代品」，而是**跑完一局的手段**（人点几百次不现实）：配 `NM_AUTOGAP=0` 提速、`NM_SEED` 复现。浮层与模态的开发入口是 `NM_ATM=1`/`NM_STOCK=1`/`NM_DECISION=bank`。
- **真人鼠标冒烟要先激活游戏窗口**（`SetForegroundWindow` + `GetClientRect`/`ClientToScreen` 换算客户区坐标）：窗口被 IDE 压住时点击全落到别人那儿，而 `NM_SHOT` 照常截出游戏画面，看着就像「UI 是死的」（§6.1 坑 27）。

## 硬约束

1. **Bevy 锁 `=0.20.0`，禁止 `cargo update`**。升级 Bevy 是独立任务，不与功能改动混在一起。
2. **分层红线**：`game_core` 只链接 bevy 的无渲染子集（`default_app`），不得引入窗口/渲染/UI 依赖；`render` 永不向 `core` 发"推进回合"的消息——core 拥有逻辑位置与步时，表现层只做视觉插值（§4.6）。
3. **旧棋盘格式刻意不移植**（`tiles[]` + `position:float` + Curve3D 烘焙）。以 ADR-001 的新格式为唯一路径，不要写兼容层。
4. **道路格不建实体**，只有土地建实体；拓扑查询走 `BoardDef`/`ActiveBoard`，别复制第二份真相来源（§4.2 的 B2a 修正）。
5. **移植时顺手修掉 §三 缺陷清单里的条目**，不要照搬 Godot 的错误行为；修完在 §三 的表下方标注归属阶段与覆盖它的测试。
6. **Godot 端从 2026-10-15 起不再是参照主线**（B5 之后拍的）。它只剩两个用处：前期 ADR/规格出处，和 §三 缺陷清单的反例。**画面、UI 排版、交互路径一律不对齐**，Bevy 侧按自己的判断设计；经济/规则要改动也不必回头比对 Godot。§五 映射表因此**停止逐文件维护**，作为 B0–B5 的历史记录保留，不要再往里加行。
7. `assets/font/dev_*` 是开发授权字体（已 gitignore），**不要提交字体**；发行前必须换成可分发的子集（§7.3）。
8. `assets/model/` 下的 glb **不入库**（已 gitignore）：二进制没法 diff，每重导一次就在历史里多存一份完整拷贝。现在忽略它们不会让克隆跑不起来，因为**没有任何代码引用**这两个文件、棋子仍是程序化几何；等渲染真的换成 glb，按 §7.5 的三条路重新评估。别顺手 `git add -f` 它们。

## 提交与文档维护

- Commit message 用中文 conventional commits，标题带阶段号，沿用现有风格：`feat(B2c): 定步时行走……`。
- 阶段完成时把进度写回 `.agents/MIGRATION.md`（§十 的表加一行状态 + §三 缺陷清单的归属），**不另开计划文档**；一个阶段在途只有一份计划。
- 质量门结果、测试计数以实际运行为准，不沿用文档里的旧数字。
- 行尾由 `.gitattributes` 锁定 LF。

## 当前状态

B0、B1、B2a、B2b、B2c、B3、B4、B5、**B5b** 已完成（数据层、世界装配、回合状态机 + 种子化骰子 + 定步时行走、经济核心 property/bank/stock/card、等轴测渲染切片：几何/建筑/棋子动画/相机跟随/格名投影、UI 与交互：顶栏/HUD/四个模态/暂停/点格详情/主菜单↔对局流转、B5b：ATM 与证券两块常驻浮层 + 银行柜台模态 + 行情按回合 tick + 分红按日历发，缺陷 #1/#3/#4/#5/#13/#14 已修，规则层 116 项 + 表现层 17 项测试）。B5 的验收记录（55 回合打完一局、9 张界面截图、三个跨局残留缺陷的修法）见 `.agents/MIGRATION.md` §10.1；验收后实报的「主菜单按钮点击无响应」已修（§6.1 坑 23，真鼠标冒烟通过）。B5b 的验收记录（两条规则分叉的取舍、6 张截图、真鼠标冒烟翻出的两个误诊）见 §10.2。

**移植这条线已追平**：Godot 侧非 addon 源码在 B5 全部有了归属，剩下的 `audio_manager`/`save_manager` 是空壳，B6–B8（AI、存档、K 线）是 Godot 没有的东西。按上面第 6 条，此后不再回头比对 Godot。

当前在做的两块欠账（都是「两边都没接上」的，不是移植遗漏）：
- ~~**银行/证券没有交易入口**~~ —— **B5b 已补**：路过银行格发 ATM 卡（存取），踩到开柜台模态（借还），证券常驻顶栏、掷骰前可下单。
- ~~**`prototype.json` 没有证券格**~~ —— **拍板不补**：证券不是一格要踩的东西。行情改成每回合结束 tick，分红改挂日历（`days_per_month`/`dividend_day`），两条分叉的理由见 §10.2。

下一步是 B6（AI 玩家）。
