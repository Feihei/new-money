# 暴富 (new-money-bevy)

用 [Bevy](https://bevyengine.org) 0.20 重写的《暴富》—— 大富翁4 风格的模拟经营棋类游戏：在环形棋盘上掷骰移动，靠买地升级、股票买卖和随机事件积累财富，直到对手破产。

源项目是 Godot 版 `E:\play\Godot\new-money`（GDScript，做到 Phase 1.5/2）。换引擎的动因是测试：原来 2400 行测试要起一个 headless Godot 进程，现在是 `cargo test`。移植方案、阶段计划与进度全部在 [`.agents/MIGRATION.md`](./.agents/MIGRATION.md)。

## 快速开始

环境：Rust stable（workspace 用 edition 2024）、Python 3（跑棋盘校验）。

```bash
cargo run -p game_app     # 打开窗口：等轴测棋盘 + 中文 HUD
cargo test -p game_core   # 规则层测试，不需要窗口
./check.sh                # 质量门：fmt + 测试 + check + clippy(-D warnings) + 棋盘校验
```

Bevy 精确锁定 `=0.20.0`（见 `Cargo.toml`），**不要 `cargo update`**。

## 现在到哪一步了

开发中，**还不能完整玩一局**。已完成 B0–B2b：

- 数据层：棋盘 / 卡牌 / 股票的 JSON schema、加载与校验（`assets/data/**`）
- 世界装配：土地与玩家成为实体，起始现金等规则常量收拢到 `GameRules`
- 回合状态机 + 种子化骰子：可复现、可 headless 对局，规则层 33 项测试
- 表现层：正交等轴测棋盘渲染、中文字体、截图自查钩子

进行中/待做：B2c 逐格行走 → B3 经济核心（地产/银行/股票/卡牌结算）→ B4 渲染切片 → B5 UI 与交互。逐阶段的验收标准见 [`.agents/MIGRATION.md` §十](./.agents/MIGRATION.md)。

## 目录结构

```
crates/game_core/   规则层：纯 ECS，bevy 只开无渲染子集（default_app）；集成测试在 crates/game_core/tests/
crates/game_app/    表现层 bin：窗口、等轴测渲染、HUD、字体、音频、存档
assets/data/        棋盘 / 卡牌 / 股票 JSON（从 Godot 原样搬来）
assets/font/        开发期 CJK 字体（gitignore，发行前换成子集字体）
tools/              与引擎无关的 Python 脚本：棋盘校验、SVG 可视化
.agents/            agent 开发文档（见下表）
docs/godot/         附件：Godot 源项目文档的原样副本
```

## 文档索引

| 文档 | 内容 |
|---|---|
| [`.agents/MIGRATION.md`](./.agents/MIGRATION.md) | 移植方案与活文档：Godot→Bevy 概念映射表、待修缺陷清单、Bevy 0.20 依赖矩阵与坑位记录、B0–B8 阶段计划 |
| [`.agents/ADR-001-board-topology.md`](./.agents/ADR-001-board-topology.md) | 棋盘拓扑决策（已确认）：网格路网 + 单向主环 + 事件死胡同分支 + 土地-道路绑定 |
| [`.agents/board-data-schema.md`](./.agents/board-data-schema.md) | 棋盘数据 JSON schema：`loop` / `lands` / `branches` 字段与实现约束 |
| [`.agents/GLOSSARY-board.md`](./.agents/GLOSSARY-board.md) | 棋盘术语表：主环、接入点、分支、对角路、土地绑定等用语统一定义 |
| [`docs/godot/DEV_PLAN.md`](./docs/godot/DEV_PLAN.md) | 附件：Godot 源项目的原始开发计划（游戏定位、系统拆解、Phase 0–9） |
| [`docs/godot/PHASE_1_5_PLAN.md`](./docs/godot/PHASE_1_5_PLAN.md) | 附件：Godot 侧 2.5D 等轴测改造方案。两份附件已知与实际代码偏离，只作设计意图参考 |

给 AI/协作者的开工约定（必读文档、硬约束、提交前检查）在 [`AGENTS.md`](./AGENTS.md)。

## 许可与资产

代码 MIT。`assets/font/dev_*` 是思源黑体的本地开发副本，不入库；发行前需替换为可再分发的子集化字体（做法记录在 `.agents/MIGRATION.md` §7.3）。
