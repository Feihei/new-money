#!/usr/bin/env python3
"""生成 assets/data/boards/china.json（P2d 之后的大地图）。

形状：40×40 网格、内缩一圈的 **148 格**矩形主环；**20 条街**沿四条边排开，
街与街之间是特殊格；监狱与医院是主环上的接入点，各带一条伸进棋盘内部的
5 格死胡同。它比台湾盘（28×28 / 100 格 / 16 条街）大一档，是设置页里
第二张真盘。

脚本是**作者化工具**，不是运行期依赖：JSON 才是权威，重跑它会整份重写。
重跑：`python tools/gen_china_board.py`；校验：`python tools/validate_board.py`。
"""

from __future__ import annotations

import json
from pathlib import Path

GRID = 40
RING = GRID - 2
CELL = 2.0
START_CASH = 300_000
OUT = Path(__file__).resolve().parent.parent / "assets/data/boards/china.json"


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
    # 起始现金 30 万：北京/上海/广州这种 6 块地的大街约 27 万，低档一条 3 块地约 4 万
    "high": {"res_base": 28_000, "res_step": 2_500, "comm_base": 70_000, "comm_step": 5_000},
    "mid": {"res_base": 18_000, "res_step": 2_000, "comm_base": 48_000, "comm_step": 4_000},
    "low": {"res_base": 12_000, "res_step": 1_500, "comm_base": 32_000, "comm_step": 2_500},
}

MALL_FEE = {"random_amount": {"sides": 6, "tiers": [800, 1600, 2400, 3200, 4800, 6400]}}
HOTEL_FEE = {"random_stay": {"sides": 4, "per_day": 1600}}
GAS_FEE = {"per_step": {"rate": 350, "requires_mount": True}}

STREETS = {
    "beijing": {
        "label": "北京",
        "tier": "high",
        "plots": [
            res("王府井"),
            res("前门大街"),
            res("三里屯"),
            comm("国贸", "mall", MALL_FEE),
            comm("中关村", "lab"),
            res("什刹海"),
        ],
    },
    "shanghai": {
        "label": "上海",
        "tier": "high",
        "plots": [
            comm("南京路", "mall", MALL_FEE),
            res("淮海路"),
            res("外滩"),
            comm("陆家嘴"),
            res("新天地"),
            res("徐家汇"),
        ],
    },
    "guangzhou": {
        "label": "广州",
        "tier": "high",
        "plots": [
            res("北京路"),
            res("上下九"),
            comm("珠江新城", "lab"),
            comm("天河路", "mall", MALL_FEE),
            res("沙面"),
            res("白鹅潭"),
        ],
    },
    "shenzhen": {
        "label": "深圳",
        "tier": "high",
        "plots": [
            res("东门老街"),
            res("华强北"),
            comm("南山科技园", "lab"),
            res("世界之窗"),
            res("深南大道"),
        ],
    },
    "hangzhou": {
        "label": "杭州",
        "tier": "mid",
        "plots": [
            res("西湖"),
            res("河坊街"),
            comm("钱江新城"),
            res("武林路"),
            res("龙井村"),
        ],
    },
    "chengdu": {
        "label": "成都",
        "tier": "mid",
        "plots": [
            res("春熙路"),
            res("宽窄巷子"),
            res("锦里"),
            comm("太古里", "mall", MALL_FEE),
            res("九眼桥"),
        ],
    },
    "chongqing": {
        "label": "重庆",
        "tier": "mid",
        "plots": [
            comm("解放碑", "mall", MALL_FEE),
            res("洪崖洞"),
            res("磁器口"),
            res("观音桥"),
            res("朝天门"),
        ],
    },
    "wuhan": {
        "label": "武汉",
        "tier": "mid",
        "plots": [
            res("江汉路"),
            res("黄鹤楼"),
            comm("光谷", "lab"),
            res("楚河汉街"),
            res("昙华林"),
        ],
    },
    "xian": {
        "label": "西安",
        "tier": "mid",
        "plots": [
            res("钟楼"),
            res("回民街"),
            res("大雁塔"),
            comm("大唐不夜城", "park"),
            res("城墙"),
        ],
    },
    "nanjing": {
        "label": "南京",
        "tier": "mid",
        "plots": [
            res("夫子庙"),
            comm("新街口", "mall", MALL_FEE),
            res("中山陵"),
            res("秦淮河"),
            res("玄武湖"),
        ],
    },
    "suzhou": {
        "label": "苏州",
        "tier": "mid",
        "plots": [res("观前街"), res("平江路"), res("拙政园"), res("山塘街")],
    },
    "tianjin": {
        "label": "天津",
        "tier": "mid",
        "plots": [res("五大道"), res("意式风情区"), res("古文化街"), res("海河")],
    },
    "qingdao": {
        "label": "青岛",
        "tier": "low",
        "plots": [res("栈桥"), res("八大关"), res("台东步行街"), res("崂山")],
    },
    "changsha": {
        "label": "长沙",
        "tier": "mid",
        "plots": [res("黄兴路"), res("橘子洲"), res("太平老街"), res("岳麓山")],
    },
    "xiamen": {
        "label": "厦门",
        "tier": "low",
        "plots": [res("中山路"), res("鼓浪屿"), res("曾厝垵"), res("环岛路")],
    },
    "kunming": {
        "label": "昆明",
        "tier": "low",
        "plots": [res("翠湖"), res("南屏街"), res("滇池"), res("石林")],
    },
    "harbin": {
        "label": "哈尔滨",
        "tier": "low",
        "plots": [res("中央大街"), res("索菲亚教堂"), res("冰雪大世界"), res("太阳岛")],
    },
    "shenyang": {
        "label": "沈阳",
        "tier": "low",
        "plots": [res("中街"), res("故宫"), res("太原街"), res("北陵")],
    },
    "zhengzhou": {
        "label": "郑州",
        "tier": "low",
        "plots": [res("二七广场"), res("德化街"), res("郑东新区"), res("郑州东站")],
    },
    "urumqi": {
        "label": "乌鲁木齐",
        "tier": "low",
        "plots": [res("大巴扎"), res("红山"), res("友好路"), res("天山")],
    },
}


# ===== 特殊格 =====


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


def company(name: str, stock: str, fee: dict | None = None) -> dict:
    data = {"stock": stock}
    if fee:
        data["fee"] = fee
    return special("company", name, data)


def shop(name: str = "百货公司") -> dict:
    return special("shop", name, {"stock": "stock_004"})


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

# 四边各 36 格（不含四角）。每条街的格数＝地块数＋商业地多占的那一格。
TOP = [
    special("fate", "命运"),
    street("beijing"),          # 8
    special("stock", "证券交易所"),
    street("shanghai"),         # 8
    company("航空公司", "stock_005", {"random_stay": {"sides": 6, "per_day": 1000}}),
    street("guangzhou"),        # 8
    special("news", "新闻中心"),
    street("shenzhen"),         # 6
    special("neutral", "公园"),
    special("bank", "银行"),
]

RIGHT = [
    special("chance", "机会"),
    street("hangzhou"),         # 6
    company("科技公司", "stock_001", {"per_step": {"rate": 250, "requires_mount": False}}),
    street("chengdu"),          # 6
    shop(),
    street("chongqing"),        # 6
    special("fate", "命运"),
    street("suzhou"),           # 4
    special("stock", "证券交易所"),
    street("tianjin"),          # 4
    special("bank", "银行"),
    street("qingdao"),          # 4
]

BOTTOM = [
    special("chance", "机会"),
    street("wuhan"),            # 6
    special("magic", "魔法屋"),
    street("xian"),             # 6
    special("jail", "监狱", branch=JAIL_BRANCH),
    coupon(10),
    arcade("pop_balloons"),
    coupon(30),
    arcade("catch_treasure"),
    special("lottery", "彩票中心"),
    coupon(50),
    arcade("penguin_dig"),
    special("hospital", "医院", branch=HOSPITAL_BRANCH),
    shop(),
    special("neutral", "公园"),
    company("地产集团", "stock_002"),
    special("chance", "机会"),
    street("changsha"),         # 4
    special("fate", "命运"),
    street("xiamen"),           # 4
]

LEFT = [
    special("fate", "命运"),
    street("nanjing"),          # 6
    company("保险公司", "stock_003", {"insure": {"sides": 4, "premium_per_day": 600, "indemnity_per_day": 2000}}),
    street("kunming"),          # 4
    special("bank", "银行"),
    street("harbin"),           # 4
    special("news", "新闻中心"),
    street("shenyang"),         # 4
    special("neutral", "公园"),
    shop(),
    street("zhengzhou"),        # 4
    special("stock", "证券交易所"),
    street("urumqi"),           # 4
    special("chance", "机会"),
    road(),
    road(),
]

CORNERS = {
    "tl": special("bank", "银行"),
    "tr": special("stock", "证券交易所"),
    "br": special("chance", "机会"),
    "bl": company("百货集团", "stock_004"),
}

CORNERS_AT = {"tl": (1, 1), "tr": (RING, 1), "br": (RING, RING), "bl": (1, RING)}

SIDE_CELLS = {
    "top": [(c, 1) for c in range(2, RING)],
    "right": [(RING, r) for r in range(2, RING)],
    "bottom": [(c, RING) for c in range(RING - 1, 1, -1)],
    "left": [(1, r) for r in range(RING - 1, 1, -1)],
}

SIDE_ORDER = ["top", "right", "bottom", "left"]
SIDE_ITEMS = {"top": TOP, "right": RIGHT, "bottom": BOTTOM, "left": LEFT}
CORNER_AFTER = {"top": "tr", "right": "br", "bottom": "bl"}


def rent_table(price: int, divisor: int, mults: list[float]) -> list[int]:
    base = r10(price / divisor)
    return [r10(base * m) for m in mults]


def tile_specs(item: dict) -> list[dict]:
    """把一个街/特殊格展开成沿环的格子；街的每块地按 1 或 2 格占位。"""
    if item["kind"] != "street":
        return [item]
    out: list[dict] = []
    for plot in STREETS[item["street"]]["plots"]:
        if plot["kind"] == "res":
            out.append({"kind": "property", "plot": plot, "street": item["street"]})
        else:
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
            plot_index = next(i for i, p in enumerate(street_def["plots"]) if p is plot)
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
                if size == 1:
                    origin = {
                        "top": (col, 2),
                        "right": (RING - 1, row),
                        "bottom": (col, RING - 1),
                        "left": (2, row),
                    }[side]
                else:
                    # 2×2 向盘内退一格，让建筑压在两格正中
                    origin = {
                        "top": (col, 2),
                        "right": (RING - 2, row),
                        "bottom": (col - 1, RING - 2),
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
            items.extend(tile_specs(item))
        if len(items) != len(cells):
            raise SystemExit(f"{side}: {len(items)} 格，边上有 {len(cells)} 格")
        for (col, row), spec in zip(cells, items):
            push_tile(col, row, spec, side)
        if side in CORNER_AFTER:
            corner = CORNER_AFTER[side]
            push_tile(*CORNERS_AT[corner], CORNERS[corner])

    if len(loop) != 148:
        raise SystemExit(f"主环 {len(loop)} 格，期望 148")
    if len(lands) != 93:
        raise SystemExit(f"地块 {len(lands)} 块，期望 93")

    seen = set()
    for t in loop:
        key = (t["col"], t["row"])
        if key in seen:
            raise SystemExit(f"道路格坐标重复 {key}")
        seen.add(key)

    branch_cells = {
        (bt["col"], bt["row"])
        for t in loop
        if "branch" in t
        for bt in t["branch"]["tiles"]
    }
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
                if key in branch_cells:
                    raise SystemExit(f"地块 {l['id']} {l['name']} 压在死胡同格 {key} 上")
                land_cells[key] = l["id"]
        # 2×2 的右下角也要在界内
        if l["col"] + l["size"] - 1 >= GRID or l["row"] + l["size"] - 1 >= GRID:
            raise SystemExit(f"地块 {l['id']} 越界")

    out = {
        "board_name": "china",
        "start_cash": START_CASH,
        "cell_size": CELL,
        "grid_cols": GRID,
        "grid_rows": GRID,
        "loop": loop,
        "lands": lands,
        "facility_fees": {
            "mall": {"random_amount": {"sides": 6, "tiers": [500, 1000, 1500, 2000, 3000, 4000]}},
            "hotel": {"random_stay": {"sides": 4, "per_day": 1200}},
            "gas_station": {"per_step": {"rate": 250, "requires_mount": True}},
        },
    }
    OUT.parent.mkdir(parents=True, exist_ok=True)
    OUT.write_text(json.dumps(out, ensure_ascii=False, indent="\t") + "\n", encoding="utf-8")
    print(f"{OUT}: loop={len(loop)} lands={len(lands)} streets={len(STREETS)}")


if __name__ == "__main__":
    main()
