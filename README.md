# 暴富 (new-money-bevy)

用 [Bevy](https://bevyengine.org) 0.20 重写的《暴富》—— 大富翁4 风格的模拟经营棋类游戏：在环形棋盘上掷骰移动，靠买地升级、股票买卖和随机事件积累财富，直到对手破产。

源项目是 Godot 版 `E:\play\Godot\new-money`（GDScript，做到 Phase 1.5/2）。换引擎的动因是测试：原来 2400 行测试要起一个 headless Godot 进程，现在是 `cargo test`。移植已完成（B0–B5b），分层红线、Bevy 0.20 的 API 事实与坑位记录、缺陷清单在 [`.agents/ARCHITECTURE.md`](./.agents/ARCHITECTURE.md)（原名《MIGRATION.md》），逐阶段的移植历史已归档；**下一步做什么排在 [`.agents/PLAN.md`](./.agents/PLAN.md)**（以大宇《大富翁4》为基线的玩法补齐计划）。

## 快速开始

环境：Rust stable（workspace 用 edition 2024）、Python 3（跑棋盘校验）。

```bash
cargo run -p game_app     # 打开窗口：主菜单 → 新游戏 → 用顶栏「掷骰」玩
cargo test -p game_core   # 规则层测试，不需要窗口
./check.sh                # 质量门：fmt + 测试 + check + clippy(-D warnings) + 棋盘校验
```

窗口里可以调的（全部只在开发侧生效）：`NM_CAM_SIZE`（取景高度，默认 8 = Godot 的 `CAM_SIZE`，`0` 整盘取景）、
`NM_SHOT=<路径>` + `NM_SHOT_AT=<秒>`（截一张 PNG 后自行退出，给 agent 自查画面用）、
`NM_AUTOROLL=1`（自动玩家替人点按钮，跑长对局用；配 `NM_AUTOGAP=0` 提速、`NM_SEED=<十进制>` 复现）、
`NM_ATM=1` / `NM_STOCK=1`（开局即浮出 ATM / 证券面板，`NM_ATM` 顺带把 ATM 卡发到当前玩家手里）。
完整清单与每条的存在理由见 [`.agents/ARCHITECTURE.md` §6.1](./.agents/ARCHITECTURE.md)。

Bevy 精确锁定 `=0.20.0`（见 `Cargo.toml`），**不要 `cargo update`**。

## 现在到哪一步了

**两个真人已经能打完一局**（B5 完成，10-14：固定种子 55 回合后一名玩家破产、屏幕留下「胜者：X！」；B5b 补上了银行与证券的交易入口）。已完成 B0–B5b：

- 数据层：棋盘 / 卡牌 / 股票的 JSON schema、加载与校验（`assets/data/**`）
- 世界装配：土地与玩家成为实体，起始现金等规则常量收拢到 `GameRules`
- 回合状态机 + 种子化骰子 + 定步时行走：可复现、可 headless 对局，规则层测试（P2 之后 174 项，以 `cargo test -p game_core` 为准）
- 经济核心：地产买卖与升级、银行存贷息、股票行情与分红、卡牌效果，落格走六段流水线；
  移植过程中顺手修掉 Godot 侧 14 处缺陷里的 10 处
- 表现层：等轴测棋盘（地面密铺 / 含对角的路面 / 组色条 / 按等级堆叠的建筑）、
  棋子逐格插值与弹跳、正交相机 rig 跟随、格名投影、中文字体、截图自查钩子
- UI 与交互：顶栏（掷骰 / ATM / 证券 / 暂停 / 返回主菜单）、底部玩家 HUD、四个模态（买地 / 升级 / 租金收据 / 卡面通知）、
  暂停菜单（ESC + 按钮，规则层 `Paused` 闸门）、主菜单↔对局流转、`bevy_picking` 点格，
  以及**源项目没有的新交互**：点一格看它的详情卡（类型 / 位置 / 地价 / 归属 / 等级 / 整张租金表）
- 银行与证券的交易入口（B5b，10-15）：**路过**银行格发一张 ATM 卡（浮层里存取，撑到自己下个回合开始）、
  **踩到**银行格开柜台模态（借 / 还 / 存取，办完才继续）、证券浮层常驻顶栏且只在掷骰前可下单。
  行情改成每回合结束动一次、分红按「几月几日」发（顶栏与证券面板都显示今天是几号、距派息几天）

待做按 `.agents/PLAN.md` §四 的 P 序列走（基线＝大宇《大富翁4》）：**P1 点券与点券格（10-16 ✅）→ T1 只读调试面板（10-16 ✅）→ P2 结算修正管线 + 建筑损害阶梯 + 物价指数 + 日历收口（10-17 ✅）** → **P3** 卡片持有与主动出牌 → P4 拘留 → P5 道具与道路放置物 → P6 神明 → P7 小游戏格 → P8 乐透 → P9 拍卖 → P10 收费原型与商业设施 → P11 新闻中心 → P12 四大恶人 → P13 魔法屋，玩法之后才是 P14 AI、P15 存档与设置、P16 音频、P17 发行（K 线图、飘字、结算画面、字体子集化、打包）。移植线的 B6/B7/B8 三行已并进这条序列。

## 目录结构

```
crates/game_core/   规则层：纯 ECS，bevy 只开无渲染子集（default_app）；集成测试在 crates/game_core/tests/
crates/game_app/    表现层 bin：窗口、等轴测渲染、HUD、字体、音频、存档
assets/data/        棋盘 / 卡牌 / 股票 JSON（从 Godot 原样搬来）
assets/font/        开发期 CJK 字体（gitignore，发行前换成子集字体）
tools/              与引擎无关的 Python 脚本：棋盘校验、SVG 可视化
.agents/            agent 开发文档（见下表）
docs/godot/         附件：Godot 源项目文档的原样副本
docs/archive/       冻结的历史（B0–B5b 移植记录），不再维护
```

## 文档索引

| 文档 | 内容 |
|---|---|
| [`.agents/ARCHITECTURE.md`](./.agents/ARCHITECTURE.md) | 架构与技术事实（原名《MIGRATION.md》）：分层红线与 crate 布局、Bevy 0.20 依赖矩阵与 §6.1 的 32 个坑、缺陷清单、表现层待做、存档方案。**唯一被源码注释引用的一份** |
| [`.agents/PLAN.md`](./.agents/PLAN.md) | **玩法补齐计划**（唯一的排期来源）：以大宇《大富翁4》为基线的缺口对照、T1 + P1–P17 实现顺序与依赖、点券格/小游戏格/新闻中心格设计、待拍板清单 |
| [`docs/archive/migration-b0-b5b.md`](./docs/archive/migration-b0-b5b.md) | 冻结的移植历史，不再维护：Godot→Bevy 概念映射表 + B0–B5b 逐阶段产出与验收记录 |
| [`.agents/ADR-001-board-topology.md`](./.agents/ADR-001-board-topology.md) | 棋盘拓扑决策（已确认）：网格路网 + 单向主环 + 事件死胡同分支 + 土地-道路绑定 |
| [`.agents/board-data-schema.md`](./.agents/board-data-schema.md) | 棋盘数据 JSON schema：`loop` / `lands` / `branches` 字段与实现约束 |
| [`.agents/GLOSSARY-board.md`](./.agents/GLOSSARY-board.md) | 棋盘术语表：主环、接入点、分支、对角路、土地绑定等用语统一定义 |
| [`docs/godot/DEV_PLAN.md`](./docs/godot/DEV_PLAN.md) | 附件：Godot 源项目的原始开发计划（游戏定位、系统拆解、Phase 0–9） |
| [`docs/godot/PHASE_1_5_PLAN.md`](./docs/godot/PHASE_1_5_PLAN.md) | 附件：Godot 侧 2.5D 等轴测改造方案。两份附件已知与实际代码偏离，只作设计意图参考 |

给 AI/协作者的开工约定（必读文档、硬约束、提交前检查）在 [`AGENTS.md`](./AGENTS.md)。

## 许可与资产

代码 MIT。`assets/font/dev_*` 是思源黑体的本地开发副本，不入库；发行前需替换为可再分发的子集化字体（做法记录在 `.agents/ARCHITECTURE.md` §7.3）。
