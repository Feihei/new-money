# Phase 1.5：渲染层 2.5D 等轴测改造方案

> 插入位置：阶段 1（棋盘与移动）之后、阶段 2（回合制引擎）之前
> 方案版本：v1.0 ｜ 对应决策：2.5D 采用 45° 等轴测、无透视（正交投影）、正确处理 Z 深度遮挡（Route B）
> 关联文档：`docs/DEV_PLAN.md` 阶段 1

---

## 一、目标（Goal）

把**渲染层**从 2D 切换为 **2.5D 等轴测**（45° 轴测视角、正交投影无透视、深度缓冲自动正确遮挡），同时：

- **数据层**（`BoardData` / `BoardTile` / `TileType`）—— 100% 不动
- **逻辑/通信层**（`EventBus` / `GameState` / `Dice` / `RollResult`）—— 100% 不动
- **UI 层**（`CanvasLayer` 上的主菜单/HUD/暂停）—— 100% 不动
- **GUT 测试**—— 仍全绿（核心逻辑未触碰）

**Phase 1.5 最小验收切片**：等轴测棋盘正确显示，棋子沿 `Path3D` 按骰子点数逐格移动，楼体增高时能正确遮挡身后棋子。

---

## 二、范围（Scope）

### 2.1 保留（不改动）
| 文件 | 说明 |
|------|------|
| `scripts/core/board_tile.gd`、`board_data.gd`、`tile_type.gd` | 纯数据 `Resource`，`position` 是标量 `[0,1]`，与维度无关 |
| `scripts/core/dice.gd`、`roll_result.gd` | 纯逻辑 |
| `scripts/autoload/*`（EventBus / GameState / DataManager / SaveManager / AudioManager / SceneManager） | 信号驱动，无渲染依赖 |
| `scenes/ui/*`、`board.tscn` 中的 `CanvasLayer`（BGLayer / UILayer） | 2D UI 原样保留 |
| `data/boards/mini_board.json` | 棋盘数据无需改动（路径映射在渲染层内部完成） |
| `tests/*` | 全部保留并重跑 |

### 2.2 改造（渲染层）
| 文件 | 从 | 到 |
|------|----|----|
| `scenes/board/board.tscn` | `BoardRoot`(Control) 内含 `Camera2D`/`Path2D`/`TilesContainer`/`TokensContainer` | `BoardRoot` 内嵌 `SubViewport` + `SubViewportContainer`，其中放 `Node3D` 棋盘 + `OrthographicCamera3D` |
| `scripts/board/board.gd` | `_build_path_from_tiles` 构建 `Curve2D`；`_setup_camera` 用 `Camera2D`；`_follow_token` 平移 2D 相机 | 构建 `Curve3D`；配置正交等轴测相机；跟随逻辑改为正交相机（或 Phase 1.5 先整体显示不跟随） |
| `scenes/entities/tile_view.tscn` + `scripts/entities/tile_view.gd` | `extends Node2D`，`ColorRect` + `Label` | `extends Node3D`，`MeshInstance3D`（菱形地板/楼体）+ `Label3D` |
| `scenes/entities/player_token.tscn` + `scripts/entities/player_token.gd` | `extends PathFollow2D`，`ColorCircle` | `extends PathFollow3D`，棋子 `Mesh`（球体/圆柱） |
| （新增）光照与环境 | — | `DirectionalLight3D` + `AmbientLight3D`，或棋盘材质设 `unshaded` 跳过光照 |

### 2.3 明确不做（本阶段）
- 不引入美术资源（仍用程序化基础网格占位，风格化靠 `unshaded` 材质）
- 不做相机环绕/缩放交互（Phase 1.5 先固定等轴测机位，整体显示棋盘）
- 不做格子点击拾取（当前由「掷骰按钮」驱动移动，无需射线拾取）
- 不动任何玩法系统（阶段 2+ 才做）

---

## 三、技术方案（Technical Approach）

### 3.1 场景结构：用 SubViewport 隔离 3D
保留 `Board`(Control) 作为 2D 根，新增 3D 内容放进 `SubViewport`，通过 `SubViewportContainer` 显示。这样 UI（`CanvasLayer`）天然绘制在 3D 之上，且 3D 与 2D 渲染顺序干净、互不干扰。

```
Board (Control)
├── BGLayer (CanvasLayer, -1)          # 背景色（保留）
│   └── BG (ColorRect)
├── BoardRoot (Control)
│   └── ViewportContainer (SubViewportContainer)   # 新增
│       └── Board3D (SubViewport, transparent_bg=true)
│           ├── World3D / Environment
│           ├── BoardRig (Node3D)                  # 棋盘根，置于世界原点
│           │   ├── Path3D (BoardPath)            # Curve3D 等轴测环
│           │   ├── TilesContainer (Node3D)       # 实例化 TileView
│           │   └── TokensContainer (Node3D)      # 棋子容器
│           ├── OrthographicCamera3D (BoardCamera)
│           └── DirectionalLight3D / AmbientLight3D（可选）
└── UILayer (CanvasLayer, 1)            # HUD/按钮（保留，原样叠加）
    └── TopBar / PauseMenu ...
```

> 备选（更简单但有风险）：直接把 `Node3D` 棋盘作为 `Board`(Control) 的子节点，靠主视口自己的 3D 渲染通道。Godot 主视口能同时渲 2D/3D，但 3D 默认在 2D 画布之下，且混用节点类型在编辑器里不直观。**Phase 1.5 推荐 SubViewport 方案**，结构更清晰、调试更稳。

### 3.2 等轴测正交相机
- `OrthographicCamera3D.projection_mode = Camera3D.PROJECTION_ORTHOGONAL`（无透视）
- 旋转（欧拉角，编辑器填度数）：经典等轴测 `rotation_degrees = Vector3(35.264, 45, 0)`；若想更"陡"的 45° 观感，可设 `Vector3(45, 45, 0)`（二测投影）。以视觉满意为准。
- `size`：控制正交视锥高度（世界单位），调它与棋盘尺寸匹配，使整盘刚好入镜。
- 位置：放在棋盘中心斜上方，看向 `(0,0,0)`（`look_at` 或手动摆）。

### 3.3 路径：Curve2D → Curve3D（几乎 1:1）
- 新建 `Path3D`，`curve = Curve3D.new()`。
- `_build_path_from_tiles` 改为在 **XZ 平面**（Y 向上）构建环形：用与原 `Curve2D` 相同的圆角矩形逻辑，只是把每个点映射到 `Vector3(x, 0, z)`（Y 设 0 或略抬高做"地面"）。
- 棋子 `progress_ratio`（`[0,1]`）语义不变 —— `BoardTile.position` 仍是标量，直接喂给 `PathFollow3D.progress_ratio`。**数据层零改动**。

### 3.4 格子：TileView → Node3D + Mesh
- 地板：一个压扁的 `BoxMesh`（或 `QuadMesh` 贴地）做菱形地块；用 `StandardMaterial3D` 设 `shading_mode = unshaded`，颜色取原 `TYPE_COLORS` 表。
- 地产楼体：在 `PROPERTY` 类型上叠一个 `BoxMesh`，高度按等级（Phase 3 才接等级数据，本阶段先固定矮盒占位，验证"增高遮挡"只需临时拉高即可）。
- 文字：`Label3D`（billboard 或固定朝上）显示格子序号/名称；后续可换 `SubViewport` 贴图获得更清晰的地面文字。
- `setup(data: BoardTile)` 接口保持不变，仅内部用 3D 节点重建视觉。

### 3.5 棋子：PlayerToken → PathFollow3D
- `extends PathFollow3D`，`rotates = false`。
- 子节点放一个 `Mesh`（球体/圆柱/胶囊）做棋子外观，`Color` 取自玩家色。
- 移动逻辑平移：原 `move_to_tile` 用 `Tween` 改 `progress_ratio` 逐格推进、每格 `STEP_DURATION`、落格 `bounce()` —— 这套代码**几乎原样可用**（`progress_ratio` 在 3D 下同名）。
- 信号 `arrived` / `step_done` 保留，与 `board.gd` 的 `await token.arrived` 衔接不变。

### 3.6 光照（可选，推荐 unshaded）
- 想要"扁平等轴测"质感：棋盘与楼体材质一律 `unshaded`，**不发光、不吃影**，靠颜色区分 —— 最省事、最贴 2.5D 折中风格，也免去调试灯光。
- 若想要一点体积感：加一盏 `DirectionalLight3D` + `AmbientLight3D`，材质改回正常着色（仍正交无透视）。Phase 1.5 建议先 unshaded 跑通，后续再决定是否加光。

### 3.7 Z 深度遮挡（核心收益）
- 由 3D 深度缓冲自动处理：楼体 `Mesh` 比棋子 `Mesh` 更靠近相机时，棋子被正确遮挡；棋子绕到楼后时自然被挡住。
- **无需任何 `z_index` / `y_sort` 手调**，这正是选 Route B 的原因。
- 验收时故意把某 `PROPERTY` 临时楼体拉高，确认身后棋子被遮住即可。

### 3.8 性能与配置
- 棋盘格子少（~40），正交 3D 开销极低。
- 可在 `project.godot` 关掉 3D 物理省内存：`physics/3d/physics_engine="none"`（与 DEV_PLAN 6.2 建议一致）。

---

## 四、执行步骤（Step-by-Step）

- [ ] **1. 场景结构调整**：编辑 `board.tscn`，在 `BoardRoot` 下加 `SubViewportContainer` → `SubViewport`（设 `transparent_bg = true`），内部建 `BoardRig`(Node3D) + `OrthographicCamera3D`。保留 `BGLayer`/`UILayer`。
- [ ] **2. 相机配置**：选 `OrthographicCamera3D`，设 `projection_mode = ORTHOGONAL`，`rotation_degrees = Vector3(35.264, 45, 0)`，调 `size` 使整盘入镜；`make_current()`。
- [ ] **3. 路径改造**：`board.gd` 的 `_build_path_from_tiles` 改为构建 `Curve3D` 环（XZ 平面，圆角矩形），挂到 `Path3D`。保留用 `path.curve == null` 判空后 `Curve3D.new()` 的防御（沿用阶段 1 踩坑经验）。
- [ ] **4. TileView 改造**：`tile_view.tscn`/`tile_view.gd` 改为 `Node3D` + `MeshInstance3D`（地板 + 可选楼体）+ `Label3D`；颜色走 `unshaded` 材质；`setup(data)` 接口不变。
- [ ] **5. PlayerToken 改造**：`player_token.tscn`/`player_token.gd` 改为 `PathFollow3D` + 棋子 `Mesh`；`move_to_tile` / `set_tile_index` 平移 `progress_ratio` 逻辑，保留 `arrived`/`step_done` 信号与 `bounce()`。
- [ ] **6. board.gd 适配**：
  - `_render_tiles`：实例化新 `TileView`（Node3D），位置由 `_position_along_path(ratio)` 采样 `Curve3D` 的 `get_baked_points()`（注意 3D 点）。
  - `_spawn_demo_token`：`PathFollow3D` 作为 `Path3D` 子节点（而非 `TokensContainer`），`set_tile_index(0, board_data)` 照常。
  - `_setup_camera`：改为配置正交相机（或用 Step 2 场景内已设好的相机，`enabled` + `make_current`）。
  - `_follow_token`：Phase 1.5 先**整体显示、不跟随**（最简单）；若保留跟随，则平移正交相机位置或移动 `BoardRig` 使棋子居中。
- [ ] **7. 光照（可选）**：按 3.6 决定 unshaded 或加 `DirectionalLight3D` + `AmbientLight3D`。
- [ ] **8. 验证**：见第五节。

> 注：`board.gd` 里 `extends Control` 可保持（它仍是 2D 根 + SubViewport 容器）；只有其内部的棋盘/相机/路径/棋子走 3D。若后续想更"纯 3D 主场景"，可另立项重构根节点，本阶段不动。

---

## 五、验证与验收（Verification）

- [ ] 视角为 45° 等轴测，**无透视畸变**（正交相机确认）
- [ ] 棋盘等轴测环正确显示，格子按 `mini_board.json` 顺序排布
- [ ] 点「掷骰」，棋子沿 `Path3D` 逐格移动、落格弹跳，与阶段 1 行为一致
- [ ] **Z 遮挡正确**：临时拉高某 `PROPERTY` 楼体，身后棋子被遮住；棋子绕到楼后自然隐藏
- [ ] UI（掷骰/返回/HUD）正常叠加，且**不受 3D 相机变换影响**（SubViewport 隔离生效）
- [ ] 现有 GUT 测试全绿（`tests/` 未改动）：`gutconfig.json` 或编辑器面板 `Run All`
- [ ] 小场景帧率无压力

**GUT 跑测命令**（参考 DEV_PLAN / 记忆）：
```
E:/apps/Godot/Godot_v4.7-stable_win64_console.exe --headless -s addons/gut/gut_cmdln.gd -gconfig=gutconfig.json -gexit
```
（或编辑器底栏 GUT 面板 → Run All）

---

## 六、风险与注意（Risks）

| 风险 | 概率 | 影响 | 应对 |
|------|------|------|------|
| SubViewport 尺寸/缩放适配 | 中 | 中 | `SubViewportContainer` 设 `stretch=true` + 锚定满屏；`SubViewport` 分辨率匹配 |
| 3D/2D 渲染顺序错乱 | 低 | 高 | 用 SubViewport 隔离，UI 走 CanvasLayer 顶层，规避混用 |
| `Label3D` 文字分辨率/朝向糊 | 中 | 低 | 先 `Label3D` 验证；后续换 `SubViewport` 贴图到地块顶面 |
| 正交相机 `size` 与棋盘尺寸不匹配 | 中 | 低 | 编辑器实时调 `size` 直到整盘入镜 |
| 阶段 1 的 `Camera2D` 死区/跟随逻辑残留 | 低 | 低 | Step 6 明确重写/禁用 `_follow_token` |
| 误改数据/逻辑层导致测试红 | 低 | 高 | 严格只动渲染层；每步跑 GUT 确认全绿 |

---

## 七、工作量估算

- **约 1–2 天**（纯渲染层改造，逻辑零改动，复用阶段 1 的移动代码）
- 关键路径：场景结构(Step1) → 相机(Step2) → 路径(Step3) → Tile/Token(Step4-5) → 适配 board.gd(Step6) → 验证(Step8)

---

## 八、完成定义（Definition of Done）

1. 等轴测正交棋盘显示正常，无透视。
2. 棋子按骰子点数沿路径移动，动画与阶段 1 一致。
3. Z 深度遮挡正确（楼体遮挡身后棋子）。
4. UI 完整可用且不被 3D 影响。
5. GUT 全绿。
6. 在 `docs/DEV_PLAN.md` 阶段 1 后补记「Phase 1.5 已完成」并归档实际工时。

---

*本方案随实现进展更新。每完成一步，在对应 checkbox 打勾；遇到偏离及时回写风险表。*
