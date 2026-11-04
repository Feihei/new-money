#!/usr/bin/env python3
"""把棋盘 JSON 导出成按主环顺序排的纯文本清单（地图 → 文本）。

这张清单读的是「一条街一段、特殊格一格一行」，两侧对齐，适合用编辑器/终端直接看
整盘的街序与特殊格分布；不是运行期依赖。

反向（文本 → 地图）走作者层生成器：
    python tools/gen_taiwan_board.py   # STREETS + items 就是文本布局
    python tools/gen_china_board.py
    python tools/validate_board.py assets/data/boards/*.json

用法：
    python tools/board_text.py assets/data/boards/taiwan.json
    python tools/board_text.py assets/data/boards/china.json .tmp/china.layout.txt
"""

from __future__ import annotations

import json
import sys
from pathlib import Path

TYPE_LABEL = {
    "bank": "银行",
    "stock": "证券交易所",
    "fate": "命运",
    "card": "卡片格",
    "news": "新闻中心",
    "magic": "魔法屋",
    "arcade": "游乐场",
    "company": "公司",
    "jail": "监狱",
    "hospital": "医院",
    "lottery": "彩票中心",
    "shop": "百货公司",
    "coupon": "点券箱",
    "neutral": "公园",
    "road": "道路",
}


def fee_label(data: dict) -> str:
    fee = data.get("fee") or {}
    if "random_amount" in fee:
        return "摇金额"
    if "random_stay" in fee:
        return "摇住宿"
    if "per_step" in fee:
        return "按骰面"
    if "insure" in fee:
        return "保险"
    return ""


def special_detail(t: dict) -> str:
    bits: list[str] = []
    if t["type"] == "company":
        if t.get("data", {}).get("stock"):
            bits.append(t["data"]["stock"])
        if fee_label(t.get("data", {})):
            bits.append(fee_label(t["data"]))
    elif t["type"] == "coupon":
        bits.append(f'{t["data"].get("coupon", "?")} 点券')
    elif t["type"] == "shop":
        bits.append(t["data"].get("stock", ""))
    elif t["type"] == "card":
        bits.append("道具池")
    if "branch" in t:
        bits.append(f'支线 {t["branch"]["type"]} ×{len(t["branch"]["tiles"])}')
    return ("  " + " · ".join(b for b in bits if b)) if bits else ""


def build_text(board: dict) -> str:
    loop = board["loop"]
    lands = {l["id"]: l for l in board["lands"]}
    street_labels: dict[str, str] = board.get("street_labels", {})
    tile_count: dict[int, int] = {}
    for t in loop:
        if t["type"] == "property":
            tile_count[t["land_id"]] = tile_count.get(t["land_id"], 0) + 1

    res = sum(1 for l in lands.values() if l["land_type"] == "residential")
    comm = len(lands) - res
    specials = sum(1 for t in loop if t["type"] != "property")
    streets = len({l["group"] for l in lands.values()})
    out: list[str] = []
    out.append(
        f'# {board["board_name"]} · {board["grid_cols"]}×{board["grid_rows"]}'
        f' · cell {board["cell_size"]}m · 起始 {board["start_cash"]:,}'
    )
    out.append(
        f"# 主环 {len(loop)} 格：{streets} 条街（{res} 宅 / {comm} 商），"
        f"{specials} 个特殊格；顺序 = 绕行方向，街与特殊格严格交替"
    )
    out.append("")

    shown: set[int] = set()
    i = 0
    while i < len(loop):
        t = loop[i]
        if t["type"] != "property":
            detail = special_detail(t)
            type_label = TYPE_LABEL.get(t["type"], t["type"])
            label = t["name"] if type_label == t["name"] else f'{t["name"]}（{type_label}）'
            out.append(f'[{i:>3}]  {label:<22}{detail}')
            i += 1
            continue

        j = i
        group = lands[t["land_id"]]["group"]
        while j < len(loop) and loop[j]["type"] == "property":
            if lands[loop[j]["land_id"]]["group"] != group:
                break
            j += 1
        plots: list[str] = []
        for k in range(i, j):
            lid = loop[k]["land_id"]
            if lid in shown:
                continue
            shown.add(lid)
            l = lands[lid]
            kind = "宅" if l["land_type"] == "residential" else "商"
            size = f'{tile_count[lid]}格'
            extra = ""
            if l.get("building_kind"):
                extra = f'/{l["building_kind"]}'
            plots.append(f'{l["name"]}({kind}{size}{extra})')
        span = f"{i}..{j - 1}" if j - 1 > i else f"{i}"
        out.append(
            f'[{span:>3}]  {street_labels.get(group, group)}街（{group}）：{" → ".join(plots)}'
        )
        i = j

    out.append("")
    out.append("# 特殊格清单（按 type 归并）")
    by_type: dict[str, list[dict]] = {}
    for t in loop:
        if t["type"] != "property":
            by_type.setdefault(t["type"], []).append(t)
    for typ, items in by_type.items():
        names = " / ".join(dict.fromkeys(t["name"] for t in items))
        at = ", ".join(str(t["id"]) for t in items)
        out.append(f"{TYPE_LABEL.get(typ, typ):<10} ×{len(items):<2} [{at}]  {names}")

    out.append("")
    out.append("# 地块清单：id  坐标  尺寸  名称  街  类型  地价  建筑")
    for l in sorted(lands.values(), key=lambda x: x["id"]):
        kind = "宅" if l["land_type"] == "residential" else "商"
        building = l.get("building_kind", "")
        out.append(
            f'{l["id"]:>3}  ({l["col"]:>2},{l["row"]:>2})  {l["size"]}×{l["size"]}  '
            f'{l["name"]:<10} {l["group"]:<10} {kind}  {l["price"]:>7}  {building}'
        )
    return "\n".join(out) + "\n"


def main() -> None:
    args = [a for a in sys.argv[1:] if not a.startswith("--")]
    if not args:
        print(__doc__)
        raise SystemExit(2)
    src = Path(args[0])
    board = json.loads(src.read_text(encoding="utf-8"))
    text = build_text(board)
    if len(args) > 1:
        dst = Path(args[1])
        dst.parent.mkdir(parents=True, exist_ok=True)
        dst.write_text(text, encoding="utf-8")
        print(f"wrote {dst}")
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main()
