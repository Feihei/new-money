#!/usr/bin/env python3
"""生成 assets/data/boards/taiwan.json（严格交替版，2026-11-04）。

形状：34×34 网格、内缩两圈的 **100 格**八角形主环（四边各 3 条 6 格街 + 四个角各
3 格斜街）；16 条街，每条街之间恰好 1 个特殊格；土地全部摆在环路**外侧**。
监狱与医院挂在底边的两个特殊格上，各带一条伸进棋盘内部的 5 格死胡同。

为什么网格从 28 放大到 34：2×2 商业地的底座要占两行/两列，外侧只留一格时它会压到
道路那一行 —— 两圈边距是「土地在外侧」这条要求的物理前提。

脚本是**作者化工具**，不是运行期依赖：JSON 才是权威，重跑它会整份重写。
重跑：`python tools/gen_taiwan_board.py`；校验：`python tools/validate_board.py`。
"""

from __future__ import annotations

from pathlib import Path

from board_layout import (
    Plot,
    Special,
    SpecialRun,
    Street,
    StreetRun,
    build_board,
    write_board,
)

GRID = 34
CELL = 2.0
START_CASH = 200_000
OUT = Path(__file__).resolve().parent.parent / "assets/data/boards/taiwan.json"

TIERS = {
    "high": {"res_base": 22_000, "res_step": 2_000, "comm_base": 55_000, "comm_step": 5_000},
    "mid": {"res_base": 15_000, "res_step": 1_500, "comm_base": 38_000, "comm_step": 3_000},
    "low": {"res_base": 10_000, "res_step": 1_000, "comm_base": 28_000, "comm_step": 2_000},
}

MALL_HIGH = {"random_amount": {"sides": 6, "tiers": [600, 1200, 1800, 2400, 3600, 4800]}}
MALL_MID = {"random_amount": {"sides": 6, "tiers": [400, 800, 1200, 1600, 2400, 3200]}}
HOTEL = {"random_stay": {"sides": 4, "per_day": 1200}}
GAS = {"per_step": {"rate": 300, "requires_mount": True}}


def res(name: str) -> Plot:
    return Plot(name, "res")


def comm(name: str, building: str | None = None, fee: dict | None = None) -> Plot:
    return Plot(name, "comm", building, fee)


STREETS = {
    # ===== 上边：三条 6 格街 =====
    "taipei": Street(
        "taipei", "台北", "high",
        [res("忠孝东路"), res("仁爱路"), res("信义路"), res("南京东路"),
         comm("台北101", "mall", MALL_HIGH)],
    ),
    "taoyuan": Street(
        "taoyuan", "桃园", "mid",
        [res("大溪老街"), res("石门水库"), res("慈湖"), res("拉拉山"),
         comm("中坜商业区", "mall", MALL_MID)],
    ),
    "hsinchu": Street(
        "hsinchu", "新竹", "mid",
        [res("城隍庙"), res("科学园区"), res("竹北"), res("南寮"),
         comm("竹北商圈")],
    ),
    # ===== 四个角的斜街：各 3 块住宅 =====
    "keelung": Street("keelung", "基隆", "low",
                      [res("基隆庙口"), res("中正路"), res("海洋广场")]),
    "miaoli": Street("miaoli", "苗栗", "mid",
                     [res("客家园区"), res("三义木雕"), res("苑里")]),
    "changhua": Street("changhua", "彰化", "mid",
                       [res("鹿港老街"), res("八卦山"), res("田尾花乡")]),
    "hualien": Street("hualien", "花莲", "low",
                      [res("太鲁阁"), res("七星潭"), res("瑞穗")]),
    # ===== 右边：三条 6 格街 =====
    "taichung": Street(
        "taichung", "台中", "high",
        [res("逢甲夜市"), res("一中街"), res("精明一街"), res("彩虹眷村"),
         comm("台中港", "gas_station", GAS)],
    ),
    "yunlin": Street(
        "yunlin", "云林", "mid",
        [res("北港朝天宫"), res("剑湖山"), res("斗六"), res("古坑"), res("虎尾"), res("西螺")],
    ),
    "chiayi": Street(
        "chiayi", "嘉义", "mid",
        [res("奋起湖"), res("布袋港"), res("民雄"), res("朴子"),
         comm("阿里山森林游乐区", "park")],
    ),
    # ===== 下边：三条 6 格街 =====
    "tainan": Street(
        "tainan", "台南", "high",
        [res("安平古堡"), res("赤崁楼"), res("花园夜市"), res("孔庙"),
         comm("台南科学园区", "lab")],
    ),
    "tamsui": Street(
        "tamsui", "淡水", "low",
        [res("淡水老街"), res("渔人码头"), res("红毛城"), res("小白宫"),
         comm("淡水名品城", "mall", MALL_MID)],
    ),
    "kaohsiung": Street(
        "kaohsiung", "高雄", "high",
        [res("西子湾"), res("旗津"), res("爱河"), res("六合夜市"),
         comm("梦时代", "mall", MALL_HIGH)],
    ),
    # ===== 左边：三条 6 格街 =====
    "pingtung": Street(
        "pingtung", "屏东", "mid",
        [res("垦丁大街"), res("东港"), res("潮州"), res("恒春"),
         comm("垦丁福华饭店", "hotel", HOTEL)],
    ),
    "taitung": Street(
        "taitung", "台东", "low",
        [res("知本温泉"), res("绿岛"), res("兰屿"), res("太麻里"), res("三仙台"), res("都兰")],
    ),
    "yilan": Street(
        "yilan", "宜兰", "low",
        [res("礁溪温泉"), res("罗东夜市"), res("冬山河"), res("龟山岛"), res("太平山"), res("南方澳")],
    ),
}


def special(kind: str, name: str, data: dict | None = None, branch: dict | None = None) -> Special:
    return Special(kind, name, data or {}, branch)


def company(name: str, stock: str, fee: dict | None = None) -> Special:
    data = {"stock": stock}
    if fee:
        data["fee"] = fee
    return special("company", name, data)


def coupon(face: int) -> Special:
    return special("coupon", "点券箱", {"coupon": face})


def branch(kind: str, direction: str = "up") -> dict:
    return {
        "type": kind,
        "dir": direction,
        "tiles": [
            {"type": "road", "name": "探监路" if kind == "jail" else "探病路"},
            {"type": "road", "name": "探监路" if kind == "jail" else "探病路"},
            {"type": "road", "name": "探监路" if kind == "jail" else "探病路"},
            {"type": "road", "name": "探监路" if kind == "jail" else "探病路"},
            {"type": kind, "name": "监狱" if kind == "jail" else "医院"},
        ],
    }


def row_cells(x0: int, x1: int, row: int, step: int = 1) -> list[tuple[int, int]]:
    return [(x, row) for x in range(x0, x1 + step, step)]


def col_cells(y0: int, y1: int, col: int, step: int = 1) -> list[tuple[int, int]]:
    return [(col, y) for y in range(y0, y1 + step, step)]


def main() -> None:
    # 八角形顶点（34×34；外圈两格留给外侧土地）
    nw, ne = (6, 2), (27, 2)
    et, eb = (31, 6), (31, 27)
    se, sw = (27, 31), (6, 31)
    wb, wt = (2, 27), (2, 6)

    items = [
        # 上边：NW 顶点 → 三条街 → NE 顶点
        SpecialRun(Special("bank", "银行"), nw),
        StreetRun(STREETS["taipei"], row_cells(7, 12, 2), "top"),
        # 卡片格：不是证券交易格（股市是常驻浮层），踩到随机抽一张道具卡进手牌
        SpecialRun(Special("card", "卡片格"), (13, 2)),
        StreetRun(STREETS["taoyuan"], row_cells(14, 19, 2), "top"),
        SpecialRun(Special("fate", "命运"), (20, 2)),
        StreetRun(STREETS["hsinchu"], row_cells(21, 26, 2), "top"),
        SpecialRun(company("航空公司", "stock_005", {"random_stay": {"sides": 6, "per_day": 800}}), ne),
        StreetRun(STREETS["keelung"], [(28, 3), (29, 4), (30, 5)], "ne"),
        # 右上 → 右边
        SpecialRun(Special("news", "新闻中心"), et),
        StreetRun(STREETS["taichung"], col_cells(7, 12, 31), "right"),
        SpecialRun(Special("magic", "魔法屋"), (31, 13)),
        StreetRun(STREETS["yunlin"], col_cells(14, 19, 31), "right"),
        SpecialRun(Special("arcade", "游乐场"), (31, 20)),
        StreetRun(STREETS["chiayi"], col_cells(21, 26, 31), "right"),
        # 右下 → 下边
        SpecialRun(company("保险公司", "stock_003",
                           {"insure": {"sides": 4, "premium_per_day": 500, "indemnity_per_day": 1500}}), eb),
        StreetRun(STREETS["miaoli"], [(30, 28), (29, 29), (28, 30)], "se"),
        SpecialRun(company("科技公司", "stock_001",
                           {"per_step": {"rate": 200, "requires_mount": False}}), se),
        StreetRun(STREETS["tainan"], row_cells(26, 21, 31, -1), "bottom"),
        SpecialRun(Special("jail", "监狱", branch=branch("jail")), (20, 31)),
        StreetRun(STREETS["tamsui"], row_cells(19, 14, 31, -1), "bottom"),
        SpecialRun(Special("hospital", "医院", branch=branch("hospital")), (13, 31)),
        StreetRun(STREETS["kaohsiung"], row_cells(12, 7, 31, -1), "bottom"),
        # 左下 → 左边
        SpecialRun(company("日产建设", "stock_002"), sw),
        StreetRun(STREETS["changhua"], [(5, 30), (4, 29), (3, 28)], "sw"),
        SpecialRun(Special("lottery", "彩票中心"), wb),
        StreetRun(STREETS["pingtung"], col_cells(26, 21, 2, -1), "left"),
        SpecialRun(Special("shop", "百货公司", {"stock": "stock_004"}), (2, 20)),
        StreetRun(STREETS["taitung"], col_cells(19, 14, 2, -1), "left"),
        SpecialRun(Special("coupon", "点券箱", {"coupon": 50}), (2, 13)),
        StreetRun(STREETS["yilan"], col_cells(12, 7, 2, -1), "left"),
        # 左上 → 回到 NW
        SpecialRun(Special("fate", "命运"), wt),
        StreetRun(STREETS["hualien"], [(3, 5), (4, 4), (5, 3)], "nw"),
    ]

    board = build_board(
        board_name="taiwan",
        cols=GRID,
        rows=GRID,
        cell_size=CELL,
        start_cash=START_CASH,
        tiers=TIERS,
        items=items,
        facility_fees={
            "mall": {"random_amount": {"sides": 6, "tiers": [300, 600, 900, 1200, 1800, 2400]}},
            "hotel": {"random_stay": {"sides": 4, "per_day": 800}},
            "gas_station": {"per_step": {"rate": 150, "requires_mount": True}},
        },
        expected_loop=100,
        expected_lands=75,
    )
    write_board(OUT, board)


if __name__ == "__main__":
    main()
