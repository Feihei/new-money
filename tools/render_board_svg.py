#!/usr/bin/env python3
"""把 prototype.json 渲染成 8x8 网格 SVG 预览图。"""
import json
import sys

PATH = sys.argv[1] if len(sys.argv) > 1 else "data/boards/prototype.json"
OUT = sys.argv[2] if len(sys.argv) > 2 else "data/boards/prototype_preview.svg"

with open(PATH, encoding="utf-8") as f:
    b = json.load(f)

CS = 70          # cell size px
PAD = 30
cols, rows = b["grid_cols"], b["grid_rows"]
W = cols * CS + PAD * 2
H = rows * CS + PAD * 2 + 40

group_color = {
    "brown": "#8d6e63", "orange": "#fb8c00", "red": "#e53935",
    "yellow": "#f9a825", "cyan": "#00acc1", "purple": "#8e24aa",
    "green": "#43a047", "blue": "#1e88e5",
}
road_type_color = {
    # 没有 "start"：10-26 起这张棋盘没有起点格，普通路走下面的 `road_color`
    "fate": "#8e24aa", "bank": "#1e88e5",
    "chance": "#00acc1", "jail": "#b71c1c", "hospital": "#ad1457",
}
road_color = "#90a4ae"

def center(c, r):
    return (PAD + c * CS + CS / 2, PAD + r * CS + CS / 2)

loop = b["loop"]
lands = {l["id"]: l for l in b.get("lands", [])}
land_cells = {}
for l in b.get("lands", []):
    for dc in range(l["size"]):
        for dr in range(l["size"]):
            land_cells[(l["col"] + dc, l["row"] + dr)] = l
branch_cells = {}
for t in loop:
    if "branch" in t:
        for bt in t["branch"]["tiles"]:
            branch_cells[(bt["col"], bt["row"])] = bt

svg = [f'<svg xmlns="http://www.w3.org/2000/svg" width="{W}" height="{H}" font-family="sans-serif">']
svg.append(f'<rect width="{W}" height="{H}" fill="#0f1419"/>')
svg.append(f'<text x="{PAD}" y="22" fill="#e6e6e6" font-size="16">棋盘原型 \"{b["board_name"]}\"  {cols}x{rows}  cell={b["cell_size"]}m</text>')

# 已占用格（道路/分支/土地）集合，用于区分填充空格
occupied = set()
for t in loop:
    occupied.add((t["col"], t["row"]))
for (c, r) in branch_cells:
    occupied.add((c, r))
for (c, r) in land_cells:
    occupied.add((c, r))

# grid — 空格用双色棋盘绿填满（密铺），占用格保留深色底
for c in range(cols):
    for r in range(rows):
        x, y = PAD + c * CS, PAD + r * CS
        if (c, r) in occupied:
            fill = "#161b22"
        else:
            fill = "#566341" if (c + r) % 2 == 0 else "#4c5a3a"
        svg.append(f'<rect x="{x}" y="{y}" width="{CS}" height="{CS}" fill="{fill}" stroke="#222" />')

# road connecting lines (loop order)
for i in range(len(loop)):
    a, c2 = loop[i], loop[(i + 1) % len(loop)]
    x1, y1 = center(a["col"], a["row"])
    x2, y2 = center(c2["col"], c2["row"])
    diag = abs(a["col"] - c2["col"]) == 1 and abs(a["row"] - c2["row"]) == 1
    col = "#ffb74d" if diag else "#cfd8dc"
    w = 7 if diag else 5
    svg.append(f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="{col}" stroke-width="{w}" stroke-linecap="round"/>')

# lands
for (c, r), l in land_cells.items():
    x, y = PAD + c * CS, PAD + r * CS
    fill = group_color.get(l["group"], "#777")
    svg.append(f'<rect x="{x+3}" y="{y+3}" width="{CS-6}" height="{CS-6}" rx="4" fill="{fill}" opacity="0.9" stroke="#000" stroke-width="1"/>')
    svg.append(f'<text x="{x+CS/2}" y="{y+CS/2-4}" fill="#fff" font-size="10" text-anchor="middle">{l["name"][:4]}</text>')
    svg.append(f'<text x="{x+CS/2}" y="{y+CS/2+10}" fill="#fff" font-size="9" text-anchor="middle">L{l["id"]}</text>')

# branch dead-ends
for (c, r), bt in branch_cells.items():
    x, y = PAD + c * CS, PAD + r * CS
    fill = road_type_color.get(bt["type"], "#666")
    svg.append(f'<rect x="{x+3}" y="{y+3}" width="{CS-6}" height="{CS-6}" rx="4" fill="{fill}" opacity="0.85" stroke="#000"/>')
    svg.append(f'<text x="{x+CS/2}" y="{y+CS/2+4}" fill="#fff" font-size="10" text-anchor="middle">{bt["name"]}</text>')

# loop road tiles (drawn on top so labels show)
for t in loop:
    x, y = PAD + t["col"] * CS, PAD + t["row"] * CS
    cx, cy = center(t["col"], t["row"])
    if t["type"] == "property":
        fill = "#37474f"
        border = group_color.get(lands[t["land_id"]]["group"], "#90a4ae")
    else:
        fill = road_type_color.get(t["type"], road_color)
        border = "#000"
    svg.append(f'<rect x="{x+8}" y="{y+8}" width="{CS-16}" height="{CS-16}" rx="6" fill="{fill}" opacity="0.95" stroke="{border}" stroke-width="2"/>')
    label = t["name"][:3]
    svg.append(f'<text x="{cx}" y="{cy-2}" fill="#fff" font-size="10" text-anchor="middle">{label}</text>')
    svg.append(f'<text x="{cx}" y="{cy+11}" fill="#cfd8dc" font-size="8" text-anchor="middle">#{t["id"]}</text>')
    if "branch" in t:
        svg.append(f'<text x="{cx}" y="{y+12}" fill="#ff8a80" font-size="9" text-anchor="middle">⛓{t["branch"]["type"]}</text>')

svg.append('</svg>')
out = "\n".join(svg)
with open(OUT, "w", encoding="utf-8") as f:
    f.write(out)
print("wrote", OUT, f"({len(out)} bytes)")
