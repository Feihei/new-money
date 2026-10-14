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
- 核对画面：`NM_SHOT=<路径>` 环境变量让 `game_app` 截图后自行退出（按墙钟触发，原因见 `.agents/MIGRATION.md` §六 的坑 5/10）。别用 `timeout N cargo run` 做冒烟测试。

## 硬约束

1. **Bevy 锁 `=0.20.0`，禁止 `cargo update`**。升级 Bevy 是独立任务，不与功能改动混在一起。
2. **分层红线**：`game_core` 只链接 bevy 的无渲染子集（`default_app`），不得引入窗口/渲染/UI 依赖；`render` 永不向 `core` 发"推进回合"的消息——core 拥有逻辑位置与步时，表现层只做视觉插值（§4.6）。
3. **旧棋盘格式刻意不移植**（`tiles[]` + `position:float` + Curve3D 烘焙）。以 ADR-001 的新格式为唯一路径，不要写兼容层。
4. **道路格不建实体**，只有土地建实体；拓扑查询走 `BoardDef`/`ActiveBoard`，别复制第二份真相来源（§4.2 的 B2a 修正）。
5. **移植时顺手修掉 §三 缺陷清单里的条目**，不要照搬 Godot 的错误行为；修完在 §三 的表下方标注归属阶段与覆盖它的测试。
6. Godot 端**不冻结**，双边节奏由用户控制。每移植完一个 Godot 文件，就在 §五 映射表里划掉并记录差异，防止两边行为悄悄分叉。
7. `assets/font/dev_*` 是开发授权字体（已 gitignore），**不要提交字体**；发行前必须换成可分发的子集（§7.3）。

## 提交与文档维护

- Commit message 用中文 conventional commits，标题带阶段号，沿用现有风格：`feat(B2c): 定步时行走……`。
- 阶段完成时把进度写回 `.agents/MIGRATION.md`（§十 的表加一行状态 + §三 缺陷清单的归属），**不另开计划文档**；一个阶段在途只有一份计划。
- 质量门结果、测试计数以实际运行为准，不沿用文档里的旧数字。
- 行尾由 `.gitattributes` 锁定 LF。

## 当前状态

B0、B1、B2a、B2b、B2c、B3 已完成（数据层、世界装配、回合状态机 + 种子化骰子 + 定步时行走、经济核心 property/bank/stock/card，缺陷 #1/#3/#4/#5/#13/#14 已修，规则层 97 项测试）。下一步是 B4 等轴测渲染切片。逐阶段验收标准以 `.agents/MIGRATION.md` §十 为准。
