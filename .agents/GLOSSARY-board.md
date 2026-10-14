# 棋盘地图术语表 (Glossary)

> 配合 `ADR-001-board-topology.md` 使用。统一本项目棋盘相关用语，避免"环/分支/路口"歧义。

- **主环 (Main Loop / Ring)**
  角色平时绕行的一圈有序路格。拓扑上是一个有向环，`loop[]` 数组顺序即绕行方向。

- **路网 (Road Network)**
  网格上所有可通车格 + 它们之间连接边的集合。是 token 实际通行的图。

- **网格 (Grid)**
  棋盘底层的 `grid_cols × grid_rows` 单元格阵列；每格 `cell_size` 米。

- **格距 (Cell Size)**
  单个网格在世界中的边长（米）。默认 **2.0**。

- **路格 (Road Cell / Tile)**
  网格上属于路网、可停留/可通行的格。其余格为装饰/空地。

- **世界坐标 (World Position)**
  `world(col,row) = (col*cell_size, 0, row*cell_size)`，由网格坐标推导，无需手填。

- **对角路 (Diagonal Road)**
  相邻两路格为对角相邻（`Δcol==±1 && Δrow==±1`）时形成的斜向街道。仅当数据显式设计时成立。

- **单向环 (One-way Ring)**
  主环绕行方向固定；转向卡使其反向，但仍沿同一环。

- **heading（朝向）**
  token 当前的行进方向（8 向之一，或环上"前/后"）。决定下一步走到哪。

- **接入点 (Junction)**
  主环上挂有分支的那一格；最多呈三岔（prev / next / branch）。

- **分支 (Branch)**
  从接入点延伸出的死胡同链，仅事件送入。

- **死胡同 (Dead-end)**
  分支的末端格，token 被送入后停留直至逃脱。

- **事件送入 (Event Teleport)**
  入狱/就医等事件把 token 传送到分支死胡同，而非沿路走进。

- **转向卡 (Turn Card)**
  改变 token heading 的道具（反转绕行方向 / 给他人转向）。

- **逃脱 (Escape)**
  满足分支退出条件（保释 / 等 N 回合等）后，token 传送回接入点并维持原 heading。

- **TileType（格类型）**
  `start / property / bank / stock / chance / fate / jail / hospital / lottery / god_deity / neutral / road / transport` 共 13 种。
  分支接入点仍为主环上的普通类型格（如 property）；`jail` / `hospital` 作为分支死胡同的 `type`。

- **原型 (Prototype)**
  8×8 网格 / 12 主环格 / 2 分支 的最小验证棋盘。

- **多地图 (Multi-map)**
  未来用不同 JSON 描述台湾/大陆/日本/美国等地形，数据驱动切换。

- **道路格 (Road Tile)**
  `loop[]` 中的格，是 token 实际走过的格。`type` 决定落到此格触发什么事件；`property` 类型经 `land_id` 触发土地逻辑。只渲染路面，不建建筑。

- **土地 (Land Plot)**
  `lands[]` 中的独立对象，只记录**归属权 (owner)** 与 **等级 (level)**，以及价格/租金/升级费等静态数据。渲染在环路**外侧**，建筑建在这、不在路上。

- **土地-道路绑定 (Land-Road Binding)**
  道路格用 `land_id` 指向 `lands[]` 的土地；token 落到该道路格时，操作对应土地的购买/收租/升级。建筑渲染在土地上。

- **绑定基数 (Binding Cardinality)**
  1×1 住宅 = 1 道路格 ↔ 1 土地（1:1）；2×2 商业 = 2 个相邻道路格共享同一 `land_id`；拐角格不绑土地。

- **拐角特殊格 (Corner Special Tile)**
  环路拐角必须是 `fate`/`bank`/`news` 等特殊地点，**无土地绑定**。

- **外侧 (Outer Side)**
  土地相对环路的方向约定：始终摆在环路**外侧**的相邻格（数据里显式写坐标）。

- **住宅/商业用地 (Residential / Commercial Land)**
  住宅 1×1（单道路格绑定）；商业/保险 2×2（双道路格共享），对应大富翁4 的商业区/保险公司等。
