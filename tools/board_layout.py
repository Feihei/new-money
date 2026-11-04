#!/usr/bin/env python3
"""两张真盘共用的作者化排版器（仅在重跑生成器时使用，运行期读 JSON）。

它把三件容易在 JSON 里写错、而错了又不会当场报的事收进一处：

1. **严格交替**：主环按「街区（3-6 格）→ 1 个特殊格 → 街区」排下去。街区的格数
   就是它的地块展开后的格数（1×1 住宅占 1 格、2×2 商业占 2 格），所以「每两条街
   之间只有 1 个非土地格」是生成器的结构，不是事后的校验。
2. **土地在环路外侧**：按街区的朝向把 1×1 摆在紧贴道路的一格；2×2 商业在
   top/left 退两格、right/bottom 退一格 —— 这样 2×2 的方块不会压到道路那一行/列。
   斜街只能摆 1×1（一个 2×2 正交方块必然压到斜线中的一格）。
3. **坏数据在这里就喊**：坐标重复、土地压路/压支线/互相重叠、支线越界、街块少于
   3 块住宅、环路不连通 —— 与 `BoardDef::validate` / `validate_board.py` 同一条纪律，
   只是提前到作者改数据的那一刻。

星号：`StreetRun.cells` 必须按**绕行方向**排列；`side` 决定土地往哪边退。
"""

from __future__ import annotations

import json
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class Plot:
    name: str
    kind: str  # "res" | "comm"
    building: str | None = None
    fee: dict | None = None


@dataclass
class Street:
    key: str
    label: str
    tier: str
    plots: list[Plot]


@dataclass
class Special:
    kind: str
    name: str
    data: dict = field(default_factory=dict)
    branch: dict | None = None


@dataclass
class StreetRun:
    street: Street
    cells: list[tuple[int, int]]
    side: str  # top / right / bottom / left / ne / se / sw / nw


@dataclass
class SpecialRun:
    spec: Special
    cell: tuple[int, int]


def r10(x: float) -> int:
    return int(round(x / 10.0)) * 10


def r100(x: float) -> int:
    return int(round(x / 100.0)) * 100


def rent_table(price: int, divisor: int, mults: list[float]) -> list[int]:
    base = r10(price / divisor)
    return [r10(base * m) for m in mults]


def _comm_suffix(side: str, second: bool) -> str:
    # 主环走向：上边向左→右，右边上→下，下边右→左，左边下→上
    if side == "top":
        return "·东" if second else "·西"
    if side == "right":
        return "·南" if second else "·北"
    if side == "bottom":
        return "·西" if second else "·东"
    return "·北" if second else "·南"


def _land_origin(side: str, cells: list[tuple[int, int]], size: int) -> tuple[int, int]:
    """一块地该从哪一格起画。`cells` 是绑定到它的道路格（1×1 一格、2×2 两格）。"""
    cols = [c for c, _ in cells]
    rows = [r for _, r in cells]
    if side == "top":
        return (min(cols), min(rows) - (1 if size == 1 else 2))
    if side == "bottom":
        return (min(cols), min(rows) + 1)
    if side == "right":
        return (min(cols) + 1, min(rows))
    if side == "left":
        return (min(cols) - (1 if size == 1 else 2), min(rows))

    # 斜街：只有 1×1。NE/SE 往东侧退，SW/NW 往西侧退 —— 三种盘的斜街都不带商业。
    if size != 1:
        raise SystemExit(f"{side} 斜街上不能摆 2×2 土地（正交方块必然压到斜线路格）")
    c, r = cells[0]
    if side in ("ne", "se"):
        return (c + 1, r)
    if side in ("sw", "nw"):
        return (c - 1, r)
    raise SystemExit(f"未知的街区朝向 {side}")


def _branch_cells(cell: tuple[int, int], branch: dict) -> list[tuple[int, int]]:
    direction = branch.get("dir", "up")
    col, row = cell
    out = []
    for step in range(1, len(branch["tiles"]) + 1):
        if direction == "up":
            out.append((col, row - step))
        elif direction == "down":
            out.append((col, row + step))
        elif direction == "left":
            out.append((col - step, row))
        elif direction == "right":
            out.append((col + step, row))
        else:
            raise SystemExit(f"未实现的分支方向 {direction}")
    return out


def build_board(
    *,
    board_name: str,
    cols: int,
    rows: int,
    cell_size: float,
    start_cash: int,
    tiers: dict,
    items: list,
    facility_fees: dict,
    expected_loop: int,
    expected_lands: int,
) -> dict:
    loop: list[dict] = []
    lands: list[dict] = []
    street_labels: dict[str, str] = {}
    land_cells: dict[tuple[int, int], int] = {}
    branch_cells: set[tuple[int, int]] = set()
    loop_cells: set[tuple[int, int]] = set()

    def push_tile(col: int, row: int, kind: str, name: str, *, data=None, branch=None, land_id=None):
        if kind == "chance":
            raise SystemExit("机会格已从规则里删除：主环上不许再出现 chance")
        tile = {
            "id": len(loop),
            "col": col,
            "row": row,
            "type": kind,
            "name": name,
        }
        if data:
            tile["data"] = data
        if land_id is not None:
            tile["land_id"] = land_id
        if branch is not None:
            cells = _branch_cells((col, row), branch)
            for (bc, br) in cells:
                if not (0 <= bc < cols and 0 <= br < rows):
                    raise SystemExit(f"支线格 ({bc},{br}) 越界 {cols}x{rows}")
                if (bc, br) in loop_cells:
                    raise SystemExit(f"支线格 ({bc},{br}) 与主环重叠")
                if (bc, br) in branch_cells:
                    raise SystemExit(f"支线格 ({bc},{br}) 被两条支线共用")
                branch_cells.add((bc, br))
            tile["branch"] = {
                "type": branch["type"],
                "tiles": [
                    {**bt, "col": bc, "row": br}
                    for bt, (bc, br) in zip(branch["tiles"], cells)
                ],
            }
        if (col, row) in loop_cells:
            raise SystemExit(f"主环格坐标重复 ({col},{row})")
        loop_cells.add((col, row))
        loop.append(tile)

    for item in items:
        if isinstance(item, SpecialRun):
            push_tile(
                item.cell[0],
                item.cell[1],
                item.spec.kind,
                item.spec.name,
                data=item.spec.data,
                branch=item.spec.branch,
            )
            continue

        assert isinstance(item, StreetRun)
        street = item.street
        side = item.side
        street_labels[street.key] = street.label
        cells = list(item.cells)
        if not (1 <= len(cells) <= 6):
            raise SystemExit(f"{street.key} 占 {len(cells)} 格，街区应为 1–6 格（含商业的两格）")
        if sum(1 for p in street.plots if p.kind == "res") < 3:
            raise SystemExit(f"{street.key} 只有 {sum(1 for p in street.plots if p.kind == 'res')} 块住宅，至少要 3 块")
        if any(p.kind not in ("res", "comm") for p in street.plots):
            raise SystemExit(f"{street.key} 有未知的地块类型")

        tier = tiers[street.tier]
        res_idx = 0
        comm_idx = 0
        cursor = 0
        for plot in street.plots:
            if plot.kind == "res":
                price = r100(tier["res_base"] + tier["res_step"] * res_idx)
                rent = rent_table(price, 25, [1, 3, 7, 14, 24, 40])
                size = 1
                res_idx += 1
                land_type = "residential"
            else:
                price = r100(tier["comm_base"] + tier["comm_step"] * comm_idx)
                rent = rent_table(price, 16, [1, 3, 6, 12, 18, 28])
                size = 2
                comm_idx += 1
                land_type = "commercial"

            plot_cells = cells[cursor : cursor + size]
            if len(plot_cells) != size:
                raise SystemExit(f"{street.key} 的 {plot.name} 需要 {size} 格，但街区已经到头")
            origin = _land_origin(side, plot_cells, size)
            land = {
                "id": len(lands),
                "col": origin[0],
                "row": origin[1],
                "size": size,
                "name": plot.name,
                "group": street.key,
                "land_type": land_type,
                "price": price,
                "rent": rent,
                "upgrade_cost": r100(price / 2),
            }
            if plot.building:
                land["building_kind"] = plot.building
            if plot.fee:
                land["fee"] = plot.fee
            lands.append(land)

            for i, cell in enumerate(plot_cells):
                tile_name = plot.name
                if plot.kind == "comm":
                    tile_name = plot.name + _comm_suffix(side, i == 1)
                push_tile(
                    cell[0],
                    cell[1],
                    "property",
                    tile_name,
                    land_id=land["id"],
                )
            cursor += size

        if cursor != len(cells):
            raise SystemExit(f"{street.key} 只用到 {cursor} 格，但排了 {len(cells)} 格道路")

    if len(loop) != expected_loop:
        raise SystemExit(f"主环 {len(loop)} 格，期望 {expected_loop}")
    if len(lands) != expected_lands:
        raise SystemExit(f"地块 {len(lands)} 块，期望 {expected_lands}")

    n = len(loop)
    for i in range(n):
        a, b = loop[i], loop[(i + 1) % n]
        if max(abs(a["col"] - b["col"]), abs(a["row"] - b["row"])) > 1 or (
            a["col"] == b["col"] and a["row"] == b["row"]
        ):
            raise SystemExit(f"主环不连通: {i}{a['col'],a['row']} -> {(i + 1) % n}{b['col'],b['row']}")

    for l in lands:
        if not (0 <= l["col"] and l["col"] + l["size"] - 1 < cols):
            raise SystemExit(f"土地 {l['id']} {l['name']} col 越界")
        if not (0 <= l["row"] and l["row"] + l["size"] - 1 < rows):
            raise SystemExit(f"土地 {l['id']} {l['name']} row 越界")
        for dc in range(l["size"]):
            for dr in range(l["size"]):
                cell = (l["col"] + dc, l["row"] + dr)
                if cell in loop_cells:
                    raise SystemExit(f"土地 {l['id']} {l['name']} 压在主环格 {cell} 上")
                if cell in branch_cells:
                    raise SystemExit(f"土地 {l['id']} {l['name']} 压在支线格 {cell} 上")
                if cell in land_cells:
                    raise SystemExit(f"土地 {l['id']} 与 {land_cells[cell]} 重叠于 {cell}")
                land_cells[cell] = l["id"]

    return {
        "board_name": board_name,
        "start_cash": start_cash,
        "cell_size": cell_size,
        "grid_cols": cols,
        "grid_rows": rows,
        "loop": loop,
        "lands": lands,
        "street_labels": street_labels,
        "facility_fees": facility_fees,
    }


def write_board(path: Path, board: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(board, ensure_ascii=False, indent="\t") + "\n", encoding="utf-8")
    print(
        f"{path}: loop={len(board['loop'])} lands={len(board['lands'])} "
        f"streets={len({l['group'] for l in board['lands']})}"
    )
