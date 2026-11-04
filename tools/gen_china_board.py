#!/usr/bin/env python3
"""生成 assets/data/boards/china.json（严格交替版，2026-11-04）。

形状：47×47 网格、内缩两圈的 **140 格**八角形主环（四边各 4 条 6 格街 + 四个角各
6 格斜街）；20 条街，每条街之间恰好 1 个特殊格；土地全部摆在环路**外侧**。
监狱与医院挂在底边的两个特殊格上，各带一条伸进棋盘内部的 5 格死胡同。

20 条街 × 每街最多 6 格 + 20 个特殊格 = 140，这是严格交替下的上限；原先的 148 格
在「街间 1 格 + 每街 ≤6 格」下放不进去，所以这一版缩到 140。网格放大到 47 是为了
2×2 商业地在外侧有两格退让。

脚本是**作者化工具**，不是运行期依赖：JSON 才是权威，重跑它会整份重写。
重跑：`python tools/gen_china_board.py`；校验：`python tools/validate_board.py`。
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

GRID = 47
CELL = 2.0
START_CASH = 300_000
OUT = Path(__file__).resolve().parent.parent / "assets/data/boards/china.json"

TIERS = {
    "high": {"res_base": 28_000, "res_step": 2_500, "comm_base": 70_000, "comm_step": 5_000},
    "mid": {"res_base": 18_000, "res_step": 2_000, "comm_base": 48_000, "comm_step": 4_000},
    "low": {"res_base": 12_000, "res_step": 1_500, "comm_base": 32_000, "comm_step": 2_500},
}

MALL = {"random_amount": {"sides": 6, "tiers": [800, 1600, 2400, 3200, 4800, 6400]}}


def res(name: str) -> Plot:
    return Plot(name, "res")


def comm(name: str, building: str | None = None, fee: dict | None = None) -> Plot:
    return Plot(name, "comm", building, fee)


STREETS = {
    # ===== 上边：四条 6 格街 =====
    "beijing": Street(
        "beijing", "北京", "high",
        [res("王府井"), res("前门大街"), res("三里屯"), res("什刹海"), comm("中关村", "lab")],
    ),
    "tianjin": Street(
        "tianjin", "天津", "mid",
        [res("五大道"), res("意式风情区"), res("古文化街"), res("海河"), comm("天津之眼", "park")],
    ),
    "qingdao": Street(
        "qingdao", "青岛", "low",
        [res("栈桥"), res("八大关"), res("台东步行街"), res("崂山"), comm("青岛啤酒城", "mall", MALL)],
    ),
    "harbin": Street(
        "harbin", "哈尔滨", "low",
        [res("中央大街"), res("索菲亚教堂"), res("冰雪大世界"), res("太阳岛"), res("亚布力"), res("松花江")],
    ),
    # ===== 右边：四条 6 格街 =====
    "shanghai": Street(
        "shanghai", "上海", "high",
        [res("淮海路"), res("外滩"), res("新天地"), res("徐家汇"), comm("南京路", "mall", MALL)],
    ),
    "nanjing": Street(
        "nanjing", "南京", "mid",
        [res("夫子庙"), res("中山陵"), res("秦淮河"), res("玄武湖"), comm("新街口", "mall", MALL)],
    ),
    "suzhou": Street(
        "suzhou", "苏州", "mid",
        [res("观前街"), res("平江路"), res("拙政园"), res("山塘街"), comm("苏州中心", "mall", MALL)],
    ),
    "hangzhou": Street(
        "hangzhou", "杭州", "mid",
        [res("西湖"), res("河坊街"), res("武林路"), res("龙井村"), comm("钱江新城")],
    ),
    # ===== 下边：四条 6 格街 =====
    "guangzhou": Street(
        "guangzhou", "广州", "high",
        [res("北京路"), res("上下九"), res("沙面"), res("白鹅潭"), comm("珠江新城", "lab")],
    ),
    "shenzhen": Street(
        "shenzhen", "深圳", "high",
        [res("东门老街"), res("华强北"), res("世界之窗"), res("深南大道"), comm("南山科技园", "lab")],
    ),
    "xiamen": Street(
        "xiamen", "厦门", "low",
        [res("中山路"), res("鼓浪屿"), res("曾厝垵"), res("环岛路"), res("集美学村"), res("厦门大学")],
    ),
    "changsha": Street(
        "changsha", "长沙", "mid",
        [res("黄兴路"), res("橘子洲"), res("太平老街"), res("岳麓山"), res("梅溪湖"), res("世界之窗")],
    ),
    # ===== 左边：四条 6 格街 =====
    "chengdu": Street(
        "chengdu", "成都", "mid",
        [res("春熙路"), res("宽窄巷子"), res("锦里"), res("九眼桥"), comm("太古里", "mall", MALL)],
    ),
    "chongqing": Street(
        "chongqing", "重庆", "mid",
        [res("洪崖洞"), res("磁器口"), res("观音桥"), res("朝天门"), comm("解放碑", "mall", MALL)],
    ),
    "xian": Street(
        "xian", "西安", "mid",
        [res("钟楼"), res("回民街"), res("大雁塔"), res("城墙"), comm("大唐不夜城", "park")],
    ),
    "wuhan": Street(
        "wuhan", "武汉", "mid",
        [res("江汉路"), res("黄鹤楼"), res("楚河汉街"), res("昙华林"), comm("光谷", "lab")],
    ),
    # ===== 四个角的斜街：各 6 块住宅 =====
    "shenyang": Street(
        "shenyang", "沈阳", "low",
        [res("中街"), res("故宫"), res("太原街"), res("北陵"), res("棋盘山"), res("九一八历史博物馆")],
    ),
    "kunming": Street(
        "kunming", "昆明", "low",
        [res("翠湖"), res("南屏街"), res("滇池"), res("石林"), res("西山"), res("民族村")],
    ),
    "zhengzhou": Street(
        "zhengzhou", "郑州", "low",
        [res("二七广场"), res("德化街"), res("郑东新区"), res("郑州东站"), res("黄河风景名胜区"), res("嵩山")],
    ),
    "urumqi": Street(
        "urumqi", "乌鲁木齐", "low",
        [res("大巴扎"), res("红山"), res("友好路"), res("天山"), res("天池"), res("南山牧场")],
    ),
}


def special(kind: str, name: str, data: dict | None = None, branch: dict | None = None) -> Special:
    return Special(kind, name, data or {}, branch)


def company(name: str, stock: str, fee: dict | None = None) -> Special:
    data = {"stock": stock}
    if fee:
        data["fee"] = fee
    return special("company", name, data)


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
    nw, ne = (9, 2), (37, 2)
    et, eb = (44, 9), (44, 37)
    se, sw = (37, 44), (9, 44)
    wb, wt = (2, 37), (2, 9)

    items = [
        # 上边：NW → 四条街 → NE
        SpecialRun(Special("bank", "银行"), nw),
        StreetRun(STREETS["beijing"], row_cells(10, 15, 2), "top"),
        SpecialRun(Special("card", "卡片格"), (16, 2)),
        StreetRun(STREETS["tianjin"], row_cells(17, 22, 2), "top"),
        SpecialRun(Special("fate", "命运"), (23, 2)),
        StreetRun(STREETS["qingdao"], row_cells(24, 29, 2), "top"),
        SpecialRun(Special("magic", "魔法屋"), (30, 2)),
        StreetRun(STREETS["harbin"], row_cells(31, 36, 2), "top"),
        SpecialRun(Special("news", "新闻中心"), ne),
        StreetRun(STREETS["shenyang"], [(38, 3), (39, 4), (40, 5), (41, 6), (42, 7), (43, 8)], "ne"),
        # 右上 → 右边
        SpecialRun(company("科技公司", "stock_001",
                           {"per_step": {"rate": 250, "requires_mount": False}}), et),
        StreetRun(STREETS["shanghai"], col_cells(10, 15, 44), "right"),
        SpecialRun(company("航空公司", "stock_005", {"random_stay": {"sides": 6, "per_day": 1000}}), (44, 16)),
        StreetRun(STREETS["nanjing"], col_cells(17, 22, 44), "right"),
        SpecialRun(company("保险公司", "stock_003",
                           {"insure": {"sides": 4, "premium_per_day": 600, "indemnity_per_day": 2000}}), (44, 23)),
        StreetRun(STREETS["suzhou"], col_cells(24, 29, 44), "right"),
        SpecialRun(Special("card", "卡片格"), (44, 30)),
        StreetRun(STREETS["hangzhou"], col_cells(31, 36, 44), "right"),
        # 右下 → 下边
        SpecialRun(company("地产集团", "stock_002"), eb),
        StreetRun(STREETS["kunming"], [(43, 38), (42, 39), (41, 40), (40, 41), (39, 42), (38, 43)], "se"),
        SpecialRun(company("百货集团", "stock_004"), se),
        StreetRun(STREETS["guangzhou"], row_cells(36, 31, 44, -1), "bottom"),
        SpecialRun(Special("jail", "监狱", branch=branch("jail")), (30, 44)),
        StreetRun(STREETS["shenzhen"], row_cells(29, 24, 44, -1), "bottom"),
        SpecialRun(Special("coupon", "点券箱", {"coupon": 30}), (23, 44)),
        StreetRun(STREETS["xiamen"], row_cells(22, 17, 44, -1), "bottom"),
        SpecialRun(Special("hospital", "医院", branch=branch("hospital")), (16, 44)),
        StreetRun(STREETS["changsha"], row_cells(15, 10, 44, -1), "bottom"),
        # 左下 → 左边
        SpecialRun(Special("lottery", "彩票中心"), sw),
        StreetRun(STREETS["zhengzhou"], [(8, 43), (7, 42), (6, 41), (5, 40), (4, 39), (3, 38)], "sw"),
        SpecialRun(Special("bank", "银行"), wb),
        StreetRun(STREETS["chengdu"], col_cells(36, 31, 2, -1), "left"),
        SpecialRun(Special("shop", "百货公司", {"stock": "stock_004"}), (2, 30)),
        StreetRun(STREETS["chongqing"], col_cells(29, 24, 2, -1), "left"),
        SpecialRun(Special("arcade", "游乐场"), (2, 23)),
        StreetRun(STREETS["xian"], col_cells(22, 17, 2, -1), "left"),
        SpecialRun(Special("fate", "命运"), (2, 16)),
        StreetRun(STREETS["wuhan"], col_cells(15, 10, 2, -1), "left"),
        # 左上 → 回到 NW
        SpecialRun(Special("news", "新闻中心"), wt),
        StreetRun(STREETS["urumqi"], [(3, 8), (4, 7), (5, 6), (6, 5), (7, 4), (8, 3)], "nw"),
    ]

    board = build_board(
        board_name="china",
        cols=GRID,
        rows=GRID,
        cell_size=CELL,
        start_cash=START_CASH,
        tiers=TIERS,
        items=items,
        facility_fees={
            "mall": {"random_amount": {"sides": 6, "tiers": [500, 1000, 1500, 2000, 3000, 4000]}},
            "hotel": {"random_stay": {"sides": 4, "per_day": 1200}},
            "gas_station": {"per_step": {"rate": 250, "requires_mount": True}},
        },
        expected_loop=140,
        expected_lands=107,
    )
    write_board(OUT, board)


if __name__ == "__main__":
    main()
