# 棋盘数据 JSON Schema

> 新棋盘格式（替代现有 `full_board.json` 的 `position:float` 模型）。详见 `ADR-001-board-topology.md`，含 **土地-道路绑定机制**。

## 顶层结构

```json
{
  "board_name": "prototype",
  "cell_size": 2.0,
  "grid_cols": 8,
  "grid_rows": 8,
  "loop": [ /* 主环有序道路格，顺序=绕行方向 */ ],
  "lands": [ /* 路旁土地，只记归属/等级，经 land_id 与道路格绑定 */ ],
  "branches": [ /* 可选：也可内联在 loop 元素的 branch 字段里 */ ]
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `board_name` | string | 地图名 |
| `cell_size` | float | 格距（米），默认 2.0 |
| `grid_cols` / `grid_rows` | int | 网格规模（校验坐标越界） |
| `loop` | array\<Tile\> | 主环**道路格**，有序；相邻可正交/对角 |
| `lands` | array\<Land\> | 路旁土地（归属/等级），与道路格绑定 |
| `branches` | array\<Branch\> | 分支列表（也可内联，见下） |

## Tile（主环道路格）

```json
{
  "id": 0,
  "col": 1, "row": 1,
  "type": "start",
  "name": "起点",
  "data": {},
  "land_id": 0,
  "branch": { "type": "jail", "tiles": [ {"col":5,"row":1,"type":"jail","name":"监狱","data":{}} ] }
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | int | 主环内唯一索引（= 数组下标） |
| `col` / `row` | int | 网格坐标，世界坐标自动推导 |
| `type` | string | 道路格类型（见下） |
| `name` | string | 显示名 |
| `data` | object | 类型附加数据（`property` 类型此字段为空，数据在 `lands[]`） |
| `land_id` | int? | **仅 property 道路格**有；指向 `lands[]` 的土地 |
| `branch` | object? | **可选**；存在即为分支接入点 |

**道路格 `type` 取值**：`start` / `road` / `chance` / `fate` / `bank` / `stock` / `coupon` / `transport` / `property` / `jail`(接入点) / `hospital`(接入点) ……
- `property` 道路格：**必带 `land_id`**，落到此格经 `land_id` 触发土地的购买/收租/升级。
- 拐角格必须是 `fate`/`bank`/`news` 等特殊类型，**不绑土地**（无 `land_id`）。
- `jail` / `hospital` 作为**分支尽头格**时只是地址，不是触发器：踩不到（`walk_path` 从不进分支），人被关着时棋子停在那里（10-20 拍板，P4④）。作为**主环格**时它是保释入口：踩上去开保释窗口，花 `GameRules::bail_points`（30）点券把关在对应分支里的人放出来，没人可保就与路面一样收摊（P4③）。
- 演示盘把拘留分支挂在地产格上（2 汉口路 / 7 徐家汇），所以「走到监狱门口」表现为先结算地产、再弹保释窗口；将来的真棋盘可以把 `type: jail` 直接放在主环上，两条入口共用同一张窗口。
- 其余类型按各自事件处理，无 `land_id`。

### `data` 的字段与归属（P1 起）

| 字段 | 类型 | 属于哪些 `type` | 含义 |
|------|------|------|------|
| `coupon` | int? | **仅 `coupon`** | 点券面额，**必须为正**（三档 10/30/50 由数据决定，不随机抽，见 PLAN §五） |
| `refresh` | int? | **仅 `coupon`** | 取走后多少回合重新出现；缺省走 `GameRules::coupon_refresh_turns`（6）。写 `0` 等于每次踩都发，与「一次性」矛盾 |

> 拘留格**没有** `turns` / `fine`：拘留天数只来自卡面（10-20 拍板，P4④ 把字段与数据一起删了），而保释不动现金（P4③）。写回 `data.turns`/`data.fine` 会被 serde 忽略，而 `board.rs` 有一条断言钉住那两格的数据必须为空。

**不变式**（`BoardDef::validate` + `tools/validate_board.py` 两边都查）：
- `coupon` 格的 `data.coupon` 缺失或为 0 → 加载期报错。「踩上去什么都不发生」必须是数据错误而不是运行时的静默。
- 非 `coupon` 格带 `coupon`/`refresh` → 报错（与「非 property 格不该带 `land_id`」同一条理由）。
- `coupon` 格不绑土地：它是一次性拾取点，没有归属、等级、租金。

### 对角判定
相邻 `loop` 元素若 `abs(dcol)==1 && abs(drow)==1`，该段为**对角路**（斜向街道），渲染与移动均按斜线处理。

## Land（路旁土地）

```json
{
  "id": 0,
  "col": 2, "row": 0,
  "size": 1,
  "name": "商业街",
  "owner": null,
  "level": 0,
  "group": "red",
  "land_type": "residential",
  "price": 2000,
  "rent": [60,180,450,900,1500,2500],
  "upgrade_cost": 800
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `id` | int | 土地唯一索引（被 `land_id` 引用） |
| `col` / `row` | int | 土地格坐标，固定在环路**外侧**（作者摆放） |
| `size` | int | 1 = 住宅 1×1；2 = 商业/保险 2×2 |
| `name` | string | 地名 |
| `owner` | int? | 归属玩家 id；`null` = 无主 |
| `level` | int | 等级（0=空地，1~N=建筑等级） |
| `group` / `land_type` / `price` / `rent` / `upgrade_cost` | — | 沿用现有地产静态数据 |

**绑定规则**：
- 1×1 住宅：**1 个 property 道路格** 的 `land_id` 指向它（1:1）。
- 2×2 商业：**2 个相邻 property 道路格** 共享同一 `land_id`（对应 2 格道路）。
- 建筑（含 `level` 高度）渲染在**土地格**，不在道路格。

## Branch（死胡同链）

```json
{
  "junction_id": 4,
  "type": "jail",
  "tiles": [
    {"col":5,"row":1,"type":"jail","name":"监狱走廊","data":{}},
    {"col":6,"row":1,"type":"jail","name":"监狱","data":{}}
  ]
}
```

| 字段 | 类型 | 说明 |
|------|------|------|
| `junction_id` | int | 指向 `loop` 中接入点的 `id` |
| `type` | string | 分支类型（jail / hospital / ...） |
| `tiles` | array\<Tile\> | 死胡同链，**从靠近接入点 → 末端**排列 |

> 内联写法：`loop` 元素的 `branch` 字段直接放上述 Branch 内容（不含 `junction_id`）。两种写法二选一，实现时统一。

## 行为规则（实现约束）

1. token 被送入分支时置于 `tiles` 的**最后一项**（末端死胡同）。
2. 逃脱后 token 传送回 `junction_id` 对应主环格，**维持进入分支前的 heading**。
3. 主环步进：`i → (i+1)%N`；转向卡反转：`i → (i-1+N)%N`。
4. 分支**绝不**作为常规前进选项；只有事件能进入。
5. 落到 `property` 道路格：读 `land_id` → 操作对应 `lands[]` 的 `owner`/`level`（购买 / 收租 / 升级）。
6. 建筑渲染在 `lands[]` 坐标（环路外侧），道路格只渲染路面。

## 完整 prototype 样本（节选，含土地绑定）

```json
{
  "board_name": "prototype",
  "cell_size": 2.0,
  "grid_cols": 8, "grid_rows": 8,
  "loop": [
    {"id":0,"col":1,"row":1,"type":"start","name":"起点","data":{}},
    {"id":1,"col":2,"row":1,"type":"property","name":"商业街","land_id":0,"data":{}},
    {"id":2,"col":3,"row":1,"type":"property","name":"住宅区","land_id":1,"data":{}},
    {"id":3,"col":4,"row":1,"type":"fate","name":"命运","data":{}},
    {"id":4,"col":5,"row":1,"type":"property","name":"银行旁","land_id":2,"data":{},
     "branch":{"type":"jail","tiles":[{"col":6,"row":1,"type":"jail","name":"监狱","data":{}}]}},
    {"id":5,"col":5,"row":2,"type":"fate","name":"命运2","data":{}},
    {"id":6,"col":5,"row":3,"type":"property","name":"工业区","land_id":2,"data":{}},
    {"id":7,"col":4,"row":3,"type":"hospital","name":"医院入口","data":{},
     "branch":{"type":"hospital","tiles":[{"col":3,"row":3,"type":"hospital","name":"医院","data":{}}]}},
    {"id":8,"col":3,"row":3,"type":"property","name":"公园","land_id":3,"data":{}},
    {"id":9,"col":2,"row":3,"type":"chance","name":"机会","data":{}},
    {"id":10,"col":1,"row":3,"type":"property","name":"车站","land_id":4,"data":{}},
    {"id":11,"col":1,"row":2,"type":"bank","name":"银行","data":{}}
  ],
  "lands": [
    {"id":0,"col":2,"row":0,"size":1,"name":"商业街","owner":null,"level":0,"group":"red","land_type":"residential","price":2000,"rent":[60,180,450,900,1500,2500],"upgrade_cost":800},
    {"id":1,"col":3,"row":0,"size":1,"name":"住宅区","owner":null,"level":0,"group":"red","land_type":"residential","price":2000,"rent":[60,180,450,900,1500,2500],"upgrade_cost":800},
    {"id":2,"col":5,"row":0,"size":2,"name":"商业区","owner":null,"level":0,"group":"blue","land_type":"commercial","price":3000,"rent":[100,300,700,1400,2200,3500],"upgrade_cost":1000},
    {"id":3,"col":3,"row":4,"size":1,"name":"公园","owner":null,"level":0,"group":"green","land_type":"residential","price":1500,"rent":[40,120,300,600,1000,1800],"upgrade_cost":600},
    {"id":4,"col":1,"row":4,"size":1,"name":"车站","owner":null,"level":0,"group":"blue","land_type":"commercial","price":2800,"rent":[90,270,650,1300,2000,3200],"upgrade_cost":950}
  ]
}
```

> 注：上述 `col/row` 仅为示意拓扑（land 固定在环路外侧 row=0 / row=4），实现原型时按真实 8×8 小方环坐标微调。land 2 为 2×2 商业，被道路格 4 与 6 共享 `land_id:2`。
