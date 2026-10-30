#!/usr/bin/env python3
"""生成 assets/data/boards/taiwan.json（P2c 真棋盘）。

棋盘形状见 PLAN §四・十七：28×28 网格、内缩一圈的 100 格矩形主环；
16 条街（住宅 1×1 / 商业 2×2）沿边排开，街与街之间是特殊格；
监狱与医院是主环上的接入点，各带一条伸进棋盘内部的 5 格死胡同。

脚本是**作者化工具**，不是运行期依赖：JSON 才是权威，重跑它会整份重写。
重跑：`python tools/gen_taiwan_board.py`；校验：`python tools/validate_board.py`。
"""

from __future__ import annotations

import json
from pathlib import Path

GRID = 28
CELL = 2.0
START_CASH = 200_000
OUT = Path(__file__).resolve().parent.parent / "assets/data/boards/taiwan.json"


def r10(x: float) -> int:
    return int(round(x / 10.0)) * 10


def r100(x: float) -> int:
    return int(round(x / 100.0)) * 100


# ===== 城市与地块 =====


def res(name: str) -> dict:
    return {"kind": "res", "name": name}


def comm(name: str, building: str | None = None, fee: dict | None = None) -> dict:
    return {"kind": "comm", "name": name, "building": building, "fee": fee}


TIERS = {
    # 起始现金 20 万：高档一条 6 块地约 21 万，低档一条 4 块地约 5 万
    "high": {"res_base": 22_000, "res_step": 2_000, "comm_base": 55_000, "comm_step": 5_000},
    "mid": {"res_base": 15_000, "res_step": 1_500, "comm_base": 38_000, "comm_step": 3_000},
    "low": {"res_base": 10_000, "res_step": 1_000, "comm_base": 28_000, "comm_step": 2_000},
}

STREETS = {
    "keelung": {
        "label": "基隆",
        "tier": "low",
        "plots": [res("基隆庙口"), res("中正路"), res("海洋广场")],
    },
    "taipei": {
        "label": "台北",
        "tier": "high",
        "plots": [
            res("忠孝东路"),
            res("仁爱路"),
            comm("台北101", "mall", {"random_amount": {"sides": 6, "tiers": [600, 1200, 1800, 2400, 3600, 4800]}}),
            comm("西门町"),
            res("信义路"),
            res("南京东路"),
        ],
    },
    "taoyuan": {
        "label": "桃园",
        "tier": "mid",
        "plots": [
            comm("中坜商业区", "mall", {"random_amount": {"sides": 6, "tiers": [400, 800, 1200, 1600, 2400, 3200]}}),
            comm("桃园站前"),
        ],
    },
    "hsinchu": {
        "label": "新竹",
        "tier": "mid",
        "plots": [res("城隍庙"), res("科学园区"), res("竹北"), res("南寮"), res("青草湖")],
    },
    "miaoli": {
        "label": "苗栗",
        "tier": "mid",
        "plots": [res("客家园区"), res("三义木雕"), res("苑里")],
    },
    "taichung": {
        "label": "台中",
        "tier": "high",
        "plots": [
            res("逢甲夜市"),
            res("一中街"),
            comm("台中港", "gas_station", {"per_step": {"rate": 300, "requires_mount": True}}),
            comm("丽宝乐园", "park"),
            res("精明一街"),
        ],
    },
    "changhua": {
        "label": "彰化",
        "tier": "mid",
        "plots": [res("鹿港老街"), res("八卦山"), res("田尾花乡")],
    },
    "yunlin": {
        "label": "云林",
        "tier": "mid",
        "plots": [res("北港朝天宫"), res("剑湖山"), res("斗六"), res("古坑"), res("虎尾"), res("西螺")],
    },
    "chiayi": {
        "label": "嘉义",
        "tier": "mid",
        "plots": [res("阿里山"), res("布袋港"), res("民雄"), res("朴子")],
    },
    "tainan": {
        "label": "台南",
        "tier": "high",
        "plots": [
            comm("台南科学园区", "lab"),
            res("安平古堡"),
            res("赤崁楼"),
            res("花园夜市"),
            res("孔庙"),
        ],
    },
    "tamsui": {
        "label": "淡水",
        "tier": "low",
        "plots": [res("淡水老街"), res("渔人码头"), res("红毛城"), res("小白宫")],
    },
    "kaohsiung": {
        "label": "高雄",
        "tier": "high",
        "plots": [
            comm("梦时代", "mall", {"random_amount": {"sides": 6, "tiers": [600, 1200, 1800, 2400, 3600, 4800]}}),
            comm("义大世界", "hotel", {"random_stay": {"sides": 4, "per_day": 1200}}),
        ],
    },
    "pingtung": {
        "label": "屏东",
        "tier": "mid",
        "plots": [res("垦丁大街"), res("东港"), res("潮州"), res("恒春")],
    },
    "taitung": {
        "label": "台东",
        "tier": "low",
        "plots": [res("知本温泉"), res("绿岛"), res("兰屿"), res("太麻里")],
    },
    "hualien": {
        "label": "花莲",
        "tier": "low",
        "plots": [res("太鲁阁"), res("七星潭"), res("瑞穗")],
    },
    "yilan": {
        "label": "宜兰",
        "tier": "low",
        "plots": [res("礁溪温泉"), res("罗东夜市"), res("冬山河"), res("龟山岛")],
    },
}


# ===== 主环：四角 + 四边 =====

# 每条边 24 格（不含两端街角）。每边第一格是「缓冲特殊格」：
# 街角与下一条街的第一块地若在斜角上相邻，两块地的足迹会撞在同一个内缩格上。
def street(key: str) -> dict:
    return {"kind": "street", "street": key}


def special(kind: str, name: str, data: dict | None = None, branch: dict | None = None) -> dict:
    return {"kind": kind, "name": name, "data": data or {}, "branch": branch}


def coupon(face: int) -> dict:
    return special("coupon", "点券箱", {"coupon": face})


def arcade(game: str) -> dict:
    return special("arcade", "游乐场", {"game": game})


def road(name: str = "大马路") -> dict:
    return special("road", name)


TOP = [
    special("fate", "命运"),
    street("keelung"),
    special("stock", "证券交易所"),
    street("taipei"),
    special("company", "航空公司", {"stock": "stock_005", "fee": {"random_stay": {"sides": 6, "per_day": 800}}}),
    street("taoyuan"),
    special("neutral", "公园"),
    street("hsinchu"),
]

RIGHT = [
    special("bank", "银行"),
    street("miaoli"),
    special("news", "新闻中心"),
    street("taichung"),
    special("company", "保险公司", {"stock": "stock_003", "fee": {"insure": {"sides": 4, "premium_per_day": 500, "indemnity_per_day": 1500}}}),
    street("changhua"),
    special("magic", "魔法屋"),
    street("yunlin"),
    special("fate", "命运"),
]

JAIL_BRANCH = {
    "type": "jail",
    "dir": "up",
    "tiles": [
        {"type": "road", "name": "探监路"},
        {"type": "road", "name": "探监路"},
        {"type": "road", "name": "探监路"},
        {"type": "road", "name": "探监路"},
        {"type": "jail", "name": "监狱"},
    ],
}
HOSPITAL_BRANCH = {
    "type": "hospital",
    "dir": "up",
    "tiles": [
        {"type": "road", "name": "探病路"},
        {"type": "road", "name": "探病路"},
        {"type": "road", "name": "探病路"},
        {"type": "road", "name": "探病路"},
        {"type": "hospital", "name": "医院"},
    ],
}

BOTTOM = [
    special("chance", "机会"),
    street("chiayi"),
    special("jail", "监狱", branch=JAIL_BRANCH),
    coupon(30),
    arcade("pop_balloons"),
    road(),
    coupon(50),
    arcade("penguin_dig"),
    road(),
    special("hospital", "医院", branch=HOSPITAL_BRANCH),
    street("tainan"),
    special("shop", "百货公司", {"stock": "stock_004"}),
    street("tamsui"),
]

LEFT = [
    special("company", "日产建设", {"stock": "stock_002"}),
    street("kaohsiung"),
    special("bank", "银行"),
    street("pingtung"),
    special("stock", "证券交易所"),
    street("taitung"),
    special("chance", "机会"),
    street("hualien"),
    special("fate", "命运"),
    street("yilan"),
]

CORNERS = {
    "tl": special("bank", "银行"),
    "tr": special("shop", "百货公司", {"stock": "stock_004"}),
    "br": special("chance", "机会"),
    "bl": special(
        "company",
        "科技公司",
        {"stock": "stock_001", "fee": {"per_step": {"rate": 200, "requires_mount": False}}},
    ),
}

CORNERS_AT = {"tl": (1, 1), "tr": (26, 1), "br": (26, 26), "bl": (1, 26)}

SIDE_CELLS = {
    "top": [(c, 1) for c in range(2, 26)],
    "right": [(26, r) for r in range(2, 26)],
    "bottom": [(c, 26) for c in range(25, 1, -1)],
    "left": [(1, r) for r in range(25, 1, -1)],
}

SIDE_ORDER = ["top", "right", "bottom", "left"]
SIDE_ITEMS = {"top": TOP, "right": RIGHT, "bottom": BOTTOM, "left": LEFT}
# 每条边走完之后经过的街角；左下（bl）在下一轮开始时就是 tl 的同一格，不重复推
CORNER_AFTER = {"top": "tr", "right": "br", "bottom": "bl"}


def rent_table(price: int, divisor: int, mults: list[float]) -> list[int]:
    base = r10(price / divisor)
    return [r10(base * m) for m in mults]


def tile_specs(item: dict, side: str) -> list[dict]:
    """把一个街/特殊格展开成沿环的格子；街的每块地按 1 或 2 格占位。"""
    if item["kind"] != "street":
        return [item]
    out: list[dict] = []
    for plot in STREETS[item["street"]]["plots"]:
        if plot["kind"] == "res":
            out.append({"kind": "property", "plot": plot, "street": item["street"]})
        else:
            # 商业地 2×2 骑两个道路格
            out.append({"kind": "property", "plot": plot, "street": item["street"]})
            out.append({"kind": "property", "plot": plot, "street": item["street"], "second": True})
    return out


def comm_suffix(side: str, second: bool) -> str:
    # 主环走向：上边向左→右，右边上→下，下边右→左，左边下→上
    if side == "top":
        return "·东" if second else "·西"
    if side == "right":
        return "·南" if second else "·北"
    if side == "bottom":
        return "·西" if second else "·东"
    return "·北" if second else "·南"


def main() -> None:
    loop: list[dict] = []
    lands: list[dict] = []
    land_ids: dict[tuple[str, int], int] = {}
    next_land = 0

    def push_tile(col: int, row: int, spec: dict, side: str | None = None) -> None:
        nonlocal next_land
        tile = {
            "id": len(loop),
            "col": col,
            "row": row,
            "type": spec["kind"],
            "name": spec.get("name") or spec["plot"]["name"],
        }
        if spec["kind"] == "property":
            plot = spec["plot"]
            street_key = spec["street"]
            street_def = STREETS[street_key]
            tier = TIERS[street_def["tier"]]
            plot_index = next(
                i for i, p in enumerate(street_def["plots"]) if p is plot
            )
            if plot["kind"] == "res":
                res_idx = sum(1 for p in street_def["plots"][:plot_index] if p["kind"] == "res")
                price = r100(tier["res_base"] + tier["res_step"] * res_idx)
                rent = rent_table(price, 25, [1, 3, 7, 14, 24, 40])
                size = 1
            else:
                comm_idx = sum(1 for p in street_def["plots"][:plot_index] if p["kind"] == "comm")
                price = r100(tier["comm_base"] + tier["comm_step"] * comm_idx)
                rent = rent_table(price, 16, [1, 3, 6, 12, 18, 28])
                size = 2
            key = (street_key, plot_index)
            if key not in land_ids:
                # 地块原点按边定：1×1 紧贴主环内侧；2×2 退一格，让建筑压在两格正中
                if size == 1:
                    origin = {
                        "top": (col, 2),
                        "right": (25, row),
                        "bottom": (col, 25),
                        "left": (2, row),
                    }[side]
                else:
                    origin = {
                        "top": (col, 2),
                        "right": (24, row),
                        "bottom": (col - 1, 24),
                        # 左边主环是自下而上，第一格在下面，2×2 的足迹反而要退到 row-1
                        "left": (2, row - 1),
                    }[side]
                land = {
                    "id": next_land,
                    "col": origin[0],
                    "row": origin[1],
                    "size": size,
                    "name": plot["name"],
                    "group": street_key,
                    "land_type": "residential" if plot["kind"] == "res" else "commercial",
                    "price": price,
                    "rent": rent,
                    "upgrade_cost": r100(price / 2),
                }
                if plot.get("building"):
                    land["building_kind"] = plot["building"]
                if plot.get("fee"):
                    land["fee"] = plot["fee"]
                land_ids[key] = next_land
                lands.append(land)
                next_land += 1
            tile["land_id"] = land_ids[key]
            if plot["kind"] == "comm":
                tile["name"] = plot["name"] + comm_suffix(side, bool(spec.get("second")))
        if spec.get("data"):
            tile["data"] = spec["data"]
        if spec.get("branch"):
            branch = dict(spec["branch"])
            direction = branch.pop("dir", "up")
            tiles = []
            for i, bt in enumerate(branch["tiles"], start=1):
                if direction != "up":
                    raise SystemExit(f"未实现的分支方向 {direction}")
                tiles.append({**bt, "col": col, "row": row - i})
            tile["branch"] = {"type": branch["type"], "tiles": tiles}
        loop.append(tile)

    # 左上角起点，然后沿上→右→下→左走一圈；左下角是起点，不再重复
    push_tile(*CORNERS_AT["tl"], CORNERS["tl"])
    for side in SIDE_ORDER:
        cells = SIDE_CELLS[side]
        items: list[dict] = []
        for item in SIDE_ITEMS[side]:
            if item["kind"] == "street":
                items.extend(tile_specs(item, side))
            else:
                items.append(item)
        if len(items) != len(cells):
            raise SystemExit(f"{side}: {len(items)} 格，边上有 {len(cells)} 格")
        for (col, row), spec in zip(cells, items):
            push_tile(col, row, spec, side)
        if side in CORNER_AFTER:
            corner = CORNER_AFTER[side]
            push_tile(*CORNERS_AT[corner], CORNERS[corner])

    if len(loop) != 100:
        raise SystemExit(f"主环 {len(loop)} 格，期望 100")

    # 街角是最后补的；用坐标校验一次没有重复/重叠
    seen = set()
    for t in loop:
        key = (t["col"], t["row"])
        if key in seen:
            raise SystemExit(f"道路格坐标重复 {key}")
        seen.add(key)
    if len(lands) != 63:
        raise SystemExit(f"地块 {len(lands)} 块，期望 63")

    land_cells: dict[tuple[int, int], int] = {}
    for l in lands:
        for dc in range(l["size"]):
            for dr in range(l["size"]):
                key = (l["col"] + dc, l["row"] + dr)
                if key in land_cells:
                    raise SystemExit(
                        f"地块 {l['id']} {l['name']} 的足迹 {key} 与 {land_cells[key]} 重叠"
                    )
                if key in seen:
                    raise SystemExit(f"地块 {l['id']} {l['name']} 与道路格重叠 {key}")
                land_cells[key] = l["id"]
        if l["col"] >= GRID or l["row"] >= GRID:
            raise SystemExit(f"地块 {l['id']} 越界")

    out = {
        "board_name": "taiwan",
        "start_cash": START_CASH,
        "cell_size": CELL,
        "grid_cols": GRID,
        "grid_rows": GRID,
        "loop": loop,
        "lands": lands,
        "facility_fees": {
            "mall": {"random_amount": {"sides": 6, "tiers": [300, 600, 900, 1200, 1800, 2400]}},
            "hotel": {"random_stay": {"sides": 4, "per_day": 800}},
            "gas_station": {"per_step": {"rate": 150, "requires_mount": True}},
        },
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent="\t") + "\n", encoding="utf-8")
    print(f"{OUT}: loop={len(loop)} lands={len(lands)} streets={len(STREETS)}")


if __name__ == "__main__":
    main()
