#!/usr/bin/env python3
"""原型棋盘拓扑校验器。

校验新 Schema（loop[]/lands[]/branch）的不变式：
  1. 网格坐标不越界
  2. 主环闭合：相邻格（含末->首）必须正交或对角相邻（Δ≤1）
  3. 对角路段数量（用于验证"斜向道路"特性）
  4. property 道路格必带 land_id，且 land 存在、binding 唯一
  5. size=2 商业用地恰好被 2 个道路格共享
  6. 土地格不得与道路格重叠
  7. 分支接入点（junction）存在，死胡同格不在主环上
  8. 点券格（coupon）必带正的 data.coupon 面额，且点券字段只属于点券格
  9. data.game 只属于游乐场格（arcade）；那一格缺省合法，意思是「这一趟随机一款」
  10. land_id 只属于 property 格；彩票格（lottery）不带任何 data 字段（价与号码池都在 GameRules 里）
  11. 新闻中心格（news）与彩票格同一条：格子上没有参数，池子在 data/news/news_items.json 里
"""
import json
import sys

# Windows 控制台默认 cp936，无法编码 emoji；不加这句的话校验成功也会因 UnicodeEncodeError 返回退出码 1
if hasattr(sys.stdout, "reconfigure"):
    sys.stdout.reconfigure(errors="replace")

PATH = sys.argv[1] if len(sys.argv) > 1 else "data/boards/prototype.json"

with open(PATH, "r", encoding="utf-8") as f:
    board = json.load(f)

cols = board["grid_cols"]
rows = board["grid_rows"]
loop = board["loop"]
lands = {l["id"]: l for l in board.get("lands", [])}

errors = []
warns = []

# 1. 网格越界
for t in loop:
    if not (0 <= t["col"] < cols and 0 <= t["row"] < rows):
        errors.append(f"loop[{t['id']}] ({t['col']},{t['row']}) 越界 {cols}x{rows}")
for l in board.get("lands", []):
    if not (0 <= l["col"] < cols and 0 <= l["row"] < rows):
        errors.append(f"land[{l['id']}] ({l['col']},{l['row']}) 越界")

loop_cells = {(t["col"], t["row"]) for t in loop}

# 2. 主环闭合 + 3. 对角统计
def step(a, b):
    dc, dr = abs(a["col"] - b["col"]), abs(a["row"] - b["row"])
    return dc <= 1 and dr <= 1 and (dc + dr) > 0

diagonals = 0
n = len(loop)
for i in range(n):
    a, b = loop[i], loop[(i + 1) % n]
    if not step(a, b):
        errors.append(f"loop 不连通: {i}({a['col']},{a['row']}) -> {(i+1)%n}({b['col']},{b['row']})")
    else:
        if abs(a["col"] - b["col"]) == 1 and abs(a["row"] - b["row"]) == 1:
            diagonals += 1

# 4 + 5. land 绑定
land_refs = {}
for t in loop:
    if t["type"] == "property":
        if "land_id" not in t:
            errors.append(f"loop[{t['id']}] 是 property 但缺 land_id")
        else:
            lid = t["land_id"]
            if lid not in lands:
                errors.append(f"loop[{t['id']}] 引用了不存在的 land_id={lid}")
            else:
                land_refs.setdefault(lid, []).append(t["id"])

for lid, refs in land_refs.items():
    sz = lands[lid]["size"]
    if sz == 1 and len(refs) != 1:
        errors.append(f"land[{lid}] size=1 但被 {len(refs)} 个道路格绑定: {refs}")
    if sz == 2 and len(refs) != 2:
        errors.append(f"land[{lid}] size=2 但被 {len(refs)} 个道路格绑定(应为2): {refs}")

# 6. 土地不与道路重叠
for l in board.get("lands", []):
    sz = l["size"]
    for dc in range(sz):
        for dr in range(sz):
            cell = (l["col"] + dc, l["row"] + dr)
            if cell in loop_cells:
                errors.append(f"land[{l['id']}] 占格 {cell} 与道路重叠")

# 7. 分支
for t in loop:
    if "branch" in t:
        b = t["branch"]
        for bt in b.get("tiles", []):
            if (bt["col"], bt["row"]) in loop_cells:
                errors.append(f"分支 '{b['type']}' 死胡同格 {bt['col'],bt['row']} 落在主环上")

# 8. 点券格（与 Rust 侧 `BoardDef::validate` 同一条不变式，两边都要响）
for t in loop:
    data = t.get("data") or {}
    if t["type"] == "coupon":
        if not data.get("coupon"):
            errors.append(f"loop[{t['id']}] 是 coupon 但缺正的 data.coupon 面额")
        if data.get("refresh") == 0:
            errors.append(f"loop[{t['id']}] coupon 的 refresh=0 等于每次踩都发，与「一次性」矛盾")
    elif data.get("coupon") is not None or data.get("refresh") is not None:
        errors.append(f"loop[{t['id']}] 不是 coupon 格，却带了点券字段")

# 9. 游乐场（P7，与 Rust 侧 `BoardDef::validate` 同一条不变式，两边都要响）。
#    与点券那一组**方向相反**：点券格缺面额是错的（那颗箱子踩上去什么都不发），
#    而游乐场格缺 game 是合法的 —— 它的意思是「这一趟随机一款」
for t in loop:
    data = t.get("data") or {}
    if t["type"] != "arcade" and data.get("game") is not None:
        errors.append(f"loop[{t['id']}] 不是 arcade 格，却带了 data.game")

# 10. `land_id` 的归属，与彩票格（P8）。
#     `land_id` 只有 `property` 用得上，而规则 4 只管到了「property 缺 land_id」这一半 ——
#     别的格类型带上它，两边都不响，于是「这一格绑了地块」成了一个只有校验器看不见的字段。
#     彩票格反过来：**格子上没有任何要读的数据**（票价、号码池、月息率都在 `GameRules` 里），
#     所以它多写一个字段就是「写了没人读」那一类静默失效。
for t in loop:
    data = t.get("data") or {}
    if t["type"] != "property" and "land_id" in t:
        errors.append(f"loop[{t['id']}] 是 {t['type']} 格，却绑了 land_id")
    if t["type"] == "lottery" and data:
        errors.append(f"loop[{t['id']}] 是 lottery 格，但彩票没有格子级参数（都在 GameRules）")
    # 新闻中心（P11）同一条：它身上没有「这一格播哪一条」那种参数 —— 抽哪一条是池子与
    # 随机流的事（data/news/news_items.json + `news::draw`）。格子上写一个字段就等于
    # 开出第二条真相来源：改了它的人以为自己在改规则，而结算压根不读它
    if t["type"] == "news" and data:
        errors.append(f"loop[{t['id']}] 是 news 格，但新闻中心没有格子级参数（池子在 news_items.json）")

# 报告
print(f"== 棋盘 '{board['board_name']}' 校验 ==")
print(f"网格: {cols}x{rows}  cell_size={board['cell_size']}")
print(f"主环格数: {n}  对角路段: {diagonals}")
print(f"土地数: {len(lands)}  分支接入点: {sum(1 for t in loop if 'branch' in t)}")
print(f"property 道路格: {sum(1 for t in loop if t['type']=='property')}")
print(f"点券格: {sum(1 for t in loop if t['type']=='coupon')}")
print(f"彩票格: {sum(1 for t in loop if t['type']=='lottery')}")
print(f"新闻中心格: {sum(1 for t in loop if t['type']=='news')}")
arcades = [t for t in loop if t["type"] == "arcade"]
print(f"游乐场格: {len(arcades)}  " + "、".join(
    f"{t['id']}({(t.get('data') or {}).get('game') or '随机一款'})" for t in arcades))

if warns:
    print("\n[警告]")
    for w in warns:
        print("  -", w)
if errors:
    print("\n[错误]")
    for e in errors:
        print("  -", e)
    print(f"\n校验失败：{len(errors)} 个错误")
    sys.exit(1)
else:
    print("\n✅ 校验通过：拓扑不变式全部满足")
