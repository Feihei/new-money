#!/usr/bin/env python3
"""把棋盘 JSON 导出成可缩放的全盘 SVG 示意图。

与 `render_board_svg.py` 的区别：那份是 prototype 的调试视图（等宽格子、单一灰色），
这份面向「一眼看清整张真盘」：每条街一种颜色、土地按实际外置坐标画、特殊格按类型
着色并标出全名、右侧图例列出街序与特殊格清单。

用法：
    python tools/render_board_map.py assets/data/boards/taiwan.json docs/maps/taiwan.svg
    python tools/render_board_map.py assets/data/boards/china.json  docs/maps/china.svg

只读 JSON，不改盘；要改盘请改 `tools/gen_*_board.py` 后重跑生成器。
"""

from __future__ import annotations

import colorsys
import html
import json
import math
import sys
from collections import OrderedDict
from pathlib import Path


TYPE_STYLE = {
    "bank": ("银行", "#1e88e5"),
    "stock": ("证券", "#00897b"),
    "fate": ("命运", "#8e24aa"),
    "card": ("卡片", "#c77d1a"),
    "news": ("新闻", "#3949ab"),
    "magic": ("魔法", "#6a1b9a"),
    "arcade": ("游乐", "#ef6c00"),
    "company": ("公司", "#b8860b"),
    "jail": ("监狱", "#c62828"),
    "hospital": ("医院", "#ad1457"),
    "lottery": ("彩票", "#2e7d32"),
    "shop": ("百货", "#d81b60"),
    "coupon": ("点券", "#f9a825"),
    "neutral": ("公园", "#607d8b"),
    "road": ("道路", "#78909c"),
}

BUILDING_LABEL = {
    "mall": "购物中心",
    "hotel": "旅馆",
    "gas_station": "加油站",
    "lab": "研究所",
    "park": "公园",
}


def street_color(i: int) -> str:
    # 黄金角取色，相邻街不会撞色；压暗到能托住白字
    h = (i * 0.618033988749895) % 1.0
    r, g, b = colorsys.hsv_to_rgb(h, 0.52, 0.58)
    return "#{:02x}{:02x}{:02x}".format(int(r * 255), int(g * 255), int(b * 255))


def darker(color: str, k: float = 0.7) -> str:
    r = int(color[1:3], 16)
    g = int(color[3:5], 16)
    b = int(color[5:7], 16)
    return "#{:02x}{:02x}{:02x}".format(int(r * k), int(g * k), int(b * k))


def esc(s: str) -> str:
    return html.escape(str(s), quote=True)


def short_special(t: dict) -> str:
    label = TYPE_STYLE.get(t["type"], (t["name"], "#555"))[0]
    if t["type"] == "company":
        return t["name"][:2]
    return label


def runs_of(loop: list[dict]) -> list[dict]:
    """把主环切成「街区 / 特殊格」交替的段。"""
    out: list[dict] = []
    i = 0
    n = len(loop)
    while i < n:
        tile = loop[i]
        if tile["type"] == "property":
            j = i + 1
            while j < n and loop[j]["type"] == "property":
                j += 1
            out.append({"kind": "street", "a": i, "b": j - 1})
            i = j
        else:
            out.append({"kind": "special", "a": i, "b": i})
            i += 1
    return out


def run_direction(run: dict, loop: list[dict]) -> tuple[int, int]:
    a = loop[run["a"]]
    b = loop[run["b"]]
    return b["col"] - a["col"], b["row"] - a["row"]


def land_rect(l: dict, cell: float, pad: float) -> tuple[float, float, float, float]:
    x = pad + l["col"] * cell
    y = pad + l["row"] * cell
    return x, y, cell * l["size"], cell * l["size"]


def wrap_name(name: str, per_line: int) -> list[str]:
    if len(name) <= per_line:
        return [name]
    return [name[i : i + per_line] for i in range(0, len(name), per_line)]


def build_svg(path: Path, out: Path) -> None:
    board = json.loads(path.read_text(encoding="utf-8"))
    loop: list[dict] = board["loop"]
    lands = {l["id"]: l for l in board["lands"]}
    street_labels: dict[str, str] = board.get("street_labels", {})
    cols = board["grid_cols"]
    rows = board["grid_rows"]

    cell = 54.0 if max(cols, rows) <= 36 else 40.0
    pad = cell * 3.4
    legend_w = 450.0
    board_w = cols * cell
    board_h = rows * cell
    width = pad * 2 + board_w + legend_w
    height = pad * 2 + board_h

    # 街道顺序与配色
    group_order: list[str] = []
    group_lands: dict[str, list[str]] = OrderedDict()
    for l in board["lands"]:
        g = l["group"]
        if g not in group_lands:
            group_lands[g] = []
            group_order.append(g)
        group_lands[g].append(l["id"])
    colors = {g: street_color(i) for i, g in enumerate(group_order)}

    def cx(t: dict) -> float:
        return pad + t["col"] * cell + cell / 2

    def cy(t: dict) -> float:
        return pad + t["row"] * cell + cell / 2

    svg: list[str] = []
    svg.append(
        f'<svg xmlns="http://www.w3.org/2000/svg" width="{width:.0f}" height="{height:.0f}" '
        f'font-family="Microsoft YaHei, Noto Sans SC, PingFang SC, sans-serif">'
    )
    svg.append(f'<rect width="{width:.0f}" height="{height:.0f}" fill="#11161d"/>')

    # 网格底
    for c in range(cols):
        for r in range(rows):
            x = pad + c * cell
            y = pad + r * cell
            fill = "#161d26" if (c + r) % 2 == 0 else "#141a22"
            svg.append(
                f'<rect x="{x:.1f}" y="{y:.1f}" width="{cell:.1f}" height="{cell:.1f}" '
                f'fill="{fill}" stroke="#1e2732" stroke-width="0.6"/>'
            )

    # 主环路
    pts = " ".join(f"{cx(t):.1f},{cy(t):.1f}" for t in loop)
    svg.append(
        f'<polyline points="{pts}" fill="none" stroke="#2b3644" '
        f'stroke-width="{cell * 0.46:.1f}" stroke-linejoin="round" stroke-linecap="round"/>'
    )
    svg.append(
        f'<polyline points="{pts}" fill="none" stroke="#4b5a6c" '
        f'stroke-width="{cell * 0.08:.1f}" stroke-dasharray="{cell * 0.12:.1f} {cell * 0.18:.1f}"/>'
    )

    # 土地
    for l in board["lands"]:
        x, y, w, h = land_rect(l, cell, pad)
        col = colors[l["group"]]
        svg.append(
            f'<rect x="{x + 1.5:.1f}" y="{y + 1.5:.1f}" width="{w - 3:.1f}" height="{h - 3:.1f}" '
            f'rx="{cell * 0.08:.1f}" fill="{col}" stroke="{darker(col, 0.55)}" stroke-width="1.4"/>'
        )
        badge = "商" if l["land_type"] == "commercial" else "宅"
        svg.append(
            f'<rect x="{x + 3:.1f}" y="{y + 3:.1f}" width="{cell * 0.32:.1f}" '
            f'height="{cell * 0.32:.1f}" rx="2" fill="#ffffff" opacity="0.82"/>'
        )
        svg.append(
            f'<text x="{x + 3 + cell * 0.16:.1f}" y="{y + 3 + cell * 0.245:.1f}" '
            f'font-size="{cell * 0.21:.1f}" text-anchor="middle" fill="#111">{badge}</text>'
        )
        name = l["name"]
        per_line = 6 if l["size"] == 2 else 4
        lines = wrap_name(name, per_line)
        fs = min(cell * 0.25, (w - 8) / max(len(x) for x in lines) / 0.98)
        fs = max(8.0, fs)
        total = len(lines) * fs * 1.12
        ty = y + h / 2 - total / 2 + fs * 0.9
        for k, line in enumerate(lines):
            svg.append(
                f'<text x="{x + w / 2:.1f}" y="{ty + k * fs * 1.12:.1f}" '
                f'font-size="{fs:.1f}" text-anchor="middle" fill="#fff">{esc(line)}</text>'
            )
        if l.get("building_kind"):
            svg.append(
                f'<text x="{x + w - 4:.1f}" y="{y + h - 4:.1f}" font-size="{cell * 0.18:.1f}" '
                f'text-anchor="end" fill="#fff" opacity="0.85">'
                f'{esc(BUILDING_LABEL.get(l["building_kind"], l["building_kind"]))}</text>'
            )

    # 支线
    for t in loop:
        if "branch" not in t:
            continue
        b = t["branch"]
        col = TYPE_STYLE.get(b["type"], ("", "#555"))[1]
        chain = [(t["col"], t["row"])] + [(x["col"], x["row"]) for x in b["tiles"]]
        bp = " ".join(
            f"{pad + c * cell + cell / 2:.1f},{pad + r * cell + cell / 2:.1f}" for c, r in chain
        )
        svg.append(
            f'<polyline points="{bp}" fill="none" stroke="{col}" stroke-width="{cell * 0.26:.1f}" '
            f'stroke-linecap="round" stroke-linejoin="round" opacity="0.9"/>'
        )
        end = b["tiles"][-1]
        ex = pad + end["col"] * cell + cell / 2
        ey = pad + end["row"] * cell + cell / 2
        svg.append(f'<circle cx="{ex:.1f}" cy="{ey:.1f}" r="{cell * 0.32:.1f}" fill="{col}" stroke="#fff" stroke-width="1.5"/>')
        svg.append(
            f'<text x="{ex:.1f}" y="{ey + cell * 0.1:.1f}" font-size="{cell * 0.22:.1f}" '
            f'text-anchor="middle" fill="#fff">{esc(TYPE_STYLE.get(b["type"], ("",))[0])}</text>'
        )

    # 特殊格
    center = ((cols - 1) / 2, (rows - 1) / 2)
    for t in loop:
        if t["type"] == "property":
            continue
        x, y = cx(t), cy(t)
        _, fill = TYPE_STYLE.get(t["type"], (t["name"], "#555"))
        r = cell * 0.40
        svg.append(f'<circle cx="{x:.1f}" cy="{y:.1f}" r="{r:.1f}" fill="{fill}" stroke="#fff" stroke-width="1.6"/>')
        svg.append(
            f'<text x="{x:.1f}" y="{y + cell * 0.09:.1f}" font-size="{cell * 0.23:.1f}" '
            f'text-anchor="middle" fill="#fff" font-weight="bold">{esc(short_special(t))}</text>'
        )
        # 名称朝棋盘内侧摆：外侧留给街道名与土地，内圈是空的
        dx = t["col"] - center[0]
        dy = t["row"] - center[1]
        dist = max(abs(dx), abs(dy))
        ux, uy = (dx / dist, dy / dist) if dist else (0, -1)
        ox = x - ux * cell * 1.05
        oy = y - uy * cell * 1.05
        anchor = "middle"
        if ux < -0.3:
            anchor = "start"
        elif ux > 0.3:
            anchor = "end"
        svg.append(
            f'<text x="{ox:.1f}" y="{oy + cell * 0.12:.1f}" font-size="{cell * 0.23:.1f}" '
            f'text-anchor="{anchor}" fill="#e8eef6" stroke="#11161d" stroke-width="3" '
            f'paint-order="stroke" font-weight="bold">{esc(t["name"])}</text>'
        )

    # 街道名：摆在土地外侧两格之外
    for run in runs_of(loop):
        if run["kind"] != "street":
            continue
        first = loop[run["a"]]
        lid = first["land_id"]
        group = lands[lid]["group"]
        dcol, drow = run_direction(run, loop)
        a, b = loop[run["a"]], loop[run["b"]]
        mx = (cx(a) + cx(b)) / 2
        my = (cy(a) + cy(b)) / 2
        # 沿线段方向旋转，再沿「离盘心更远」的那条法线外推
        seg = max(abs(dcol), abs(drow)) or 1
        ux, uy = dcol / seg, drow / seg
        nx, ny = -uy, ux
        if (mx - pad - board_w / 2) * nx + (my - pad - board_h / 2) * ny < 0:
            nx, ny = -nx, -ny
        lx, ly = mx + nx * cell * 2.9, my + ny * cell * 2.9
        rot = math.degrees(math.atan2(uy, ux))
        if rot > 90:
            rot -= 180
        elif rot < -90:
            rot += 180
        color = colors[group]
        plots = group_lands[group]
        text = " · ".join(lands[i]["name"] for i in plots)
        svg.append(
            f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="{cell * 0.30:.1f}" text-anchor="middle" '
            f'fill="{color}" stroke="#11161d" stroke-width="4" paint-order="stroke" '
            f'transform="rotate({rot} {lx:.1f} {ly:.1f})">{esc(text)}</text>'
        )

    # ===== 右侧图例 =====
    lx = pad + board_w + 34
    ly = pad + 8
    svg.append(
        f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="26" fill="#f4f7fb" font-weight="bold">'
        f'{esc(board["board_name"])} · {cols}×{rows}</text>'
    )
    ly += 30
    svg.append(
        f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="15" fill="#9fb0c3">'
        f'{len(loop)} 格 · {len(group_order)} 条街 · {len(board["lands"])} 块地 · '
        f'起始 {board["start_cash"]:,}</text>'
    )
    ly += 34

    svg.append(f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="18" fill="#f4f7fb" font-weight="bold">街道</text>')
    ly += 24
    for group in group_order:
        color = colors[group]
        names = [lands[i]["name"] for i in group_lands[group]]
        res = sum(1 for i in group_lands[group] if lands[i]["land_type"] == "residential")
        comm = len(group_lands[group]) - res
        kind = f"{res}宅" + (f" + {comm}商" if comm else "")
        svg.append(f'<rect x="{lx:.1f}" y="{ly - 12:.1f}" width="16" height="16" rx="3" fill="{color}"/>')
        svg.append(
            f'<text x="{lx + 24:.1f}" y="{ly:.1f}" font-size="14" fill="#e8eef6">'
            f'{esc(street_labels.get(group, group))}</text>'
        )
        svg.append(
            f'<text x="{lx + 430:.1f}" y="{ly:.1f}" font-size="12" fill="#9fb0c3" text-anchor="end">'
            f'{kind}</text>'
        )
        ly += 17
        svg.append(
            f'<text x="{lx + 24:.1f}" y="{ly:.1f}" font-size="11.5" fill="#c7d3e0">'
            f'{esc(" / ".join(names))}</text>'
        )
        ly += 23

    ly += 12
    svg.append(f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="18" fill="#f4f7fb" font-weight="bold">特殊格</text>')
    ly += 24
    special_key = lambda t: f'{t["type"]}:{t["name"]}:{t.get("data", {}).get("stock", "")}'
    special_counts: dict[str, int] = {}
    for t in loop:
        if t["type"] != "property":
            special_counts[special_key(t)] = special_counts.get(special_key(t), 0) + 1
    seen: set[str] = set()
    for t in loop:
        if t["type"] == "property":
            continue
        key = special_key(t)
        if key in seen:
            continue
        seen.add(key)
        _, fill = TYPE_STYLE.get(t["type"], (t["name"], "#555"))
        label = TYPE_STYLE.get(t["type"], ("",))[0]
        detail = ""
        if t["type"] == "company" and t.get("data", {}).get("stock"):
            detail = f' · {t["data"]["stock"]}'
        elif t["type"] == "coupon":
            detail = f' · {t["data"].get("coupon")} 点券'
        elif t["type"] in ("jail", "hospital") and "branch" in t:
            detail = " · 死胡同"
        count = special_counts[key]
        name = t["name"] + (f" ×{count}" if count > 1 else "")
        svg.append(f'<rect x="{lx:.1f}" y="{ly - 12:.1f}" width="16" height="16" rx="8" fill="{fill}"/>')
        svg.append(
            f'<text x="{lx + 24:.1f}" y="{ly:.1f}" font-size="13.5" fill="#e8eef6">'
            f'{esc(name)}</text>'
        )
        svg.append(
            f'<text x="{lx + 430:.1f}" y="{ly:.1f}" font-size="11.5" fill="#9fb0c3" text-anchor="end">'
            f'{esc(label)}{esc(detail)}</text>'
        )
        ly += 21

    ly += 16
    svg.append(
        f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="11.5" fill="#7f8f9f">'
        f'宅=住宅 1 格 · 商=商业 2 格 · 圆点=特殊格</text>'
    )
    ly += 18
    svg.append(
        f'<text x="{lx:.1f}" y="{ly:.1f}" font-size="11.5" fill="#7f8f9f">'
        f'数据源 {path.name}；改盘走 tools/gen_*_board.py</text>'
    )

    svg.append("</svg>")
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text("\n".join(svg), encoding="utf-8")
    print(f"wrote {out} ({len(svg)} nodes)")


def main() -> None:
    if len(sys.argv) < 2:
        print(__doc__)
        raise SystemExit(2)
    src = Path(sys.argv[1])
    dst = Path(sys.argv[2]) if len(sys.argv) > 2 else src.with_suffix(".svg")
    build_svg(src, dst)


if __name__ == "__main__":
    main()
