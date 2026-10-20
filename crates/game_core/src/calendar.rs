//! 真实公历与节假日（P2b）。
//!
//! P2④ 那版日历是「一个月固定 12 天、一周 7 天」的自造历 —— 它能跑，但玩家没法把
//! 「每月 15 号分红」对上任何一个真实日子，而 P2b 之后的分红、结息、节假日都要按
//! 真实历法读。这里因此有三件事：
//!
//! 1. **公历换算**（[`Date`]）：`tick` 数的是开局以来的天数，`epoch` 是开局那天
//!    （2048 年随机一个月的 1 日）。闰年、世纪年、跨年、跨月全部由 `days_from_civil`
//!    统一处理，不另写一套「月长表」。
//! 2. **节假日表**（[`HolidaySet`]，数据在 `assets/data/calendar/holidays.json`）：
//!    固定公历的四条（元旦/劳动/国庆/圣诞）由日期直接算出，农历四节（春节/清明/端午/中秋）
//!    查表。表只覆盖 2048–2052：**表外年份显式降级** —— 农历三节与清明不放假、公历四条照常，
//!    启动时 `warn!`。不循环回 2048，农历 19 年一周且闰月不规则，循环会造出可被戳穿的假日期。
//! 3. **休市判据**（[`HolidaySet::is_market_closed`]）：周末 ∪ 节假日，**只喂股市**。
//!    10-20 实报更正：银行全年无休，只要停到银行格就能办 —— P2④ 那条「股市休市与银行休业
//!    是同一件事的两面」是拍错的设计，见 PLAN 的 P2b ②。
//!
//! 节假日天数是我们设计的，不是抄国务院：元旦/清明/端午/中秋/圣诞各 1 天，
//! 劳动节/国庆/春节各 3 天（再长股市就停太多天）。调休补班因此不存在。

use bevy::asset::Asset;
use bevy::prelude::*;
use serde::Deserialize;

/// 开局年份。起始月由 [`crate::rng::Rng::pick_epoch_month`] 在 1..=12 里抽。
///
/// 2048 是闰年：随机到 2 月开局时，2/29 就在这一局里 —— 闰日因此不是只在单测里存在。
pub const EPOCH_YEAR: i32 = 2048;

/// 一个公历日期。`year` 可以小于 0（算法本身支持），但本局只会在 2048–2052 附近。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub const fn new(year: i32, month: u32, day: u32) -> Self {
        Self { year, month, day }
    }

    /// `"YYYY-MM-DD"`（`NM_EPOCH` 的格式）。月/日可写一位，便于手敲。
    pub fn parse(text: &str) -> Option<Self> {
        let mut parts = text.trim().split('-');
        let year = parts.next()?.parse().ok()?;
        let month = parts.next()?.parse().ok()?;
        let day = parts.next()?.parse().ok()?;
        if parts.next().is_some() || !(1..=12).contains(&month) {
            return None;
        }
        if day == 0 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// 距 1970-01-01 的天数（Howard Hinnant 的 civil-from-days 正向式）。
    pub fn to_days(self) -> i64 {
        days_from_civil(self.year, self.month, self.day)
    }

    pub fn from_days(days: i64) -> Self {
        let (year, month, day) = civil_from_days(days);
        Self { year, month, day }
    }

    pub fn add_days(self, days: i64) -> Self {
        Self::from_days(self.to_days() + days)
    }

    /// 周几：1 = 周一 … 7 = 周日。1970-01-01 是周四（4）。
    pub fn weekday(self) -> u32 {
        ((self.to_days() + 3).rem_euclid(7) + 1) as u32
    }

    pub fn is_weekend(self) -> bool {
        self.weekday() >= 6
    }

    pub fn month_end(self) -> Self {
        Self {
            day: days_in_month(self.year, self.month),
            ..self
        }
    }
}

impl std::fmt::Display for Date {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

/// 开发/测试用的历法锚点（`NM_EPOCH=2048-02-01`）。
///
/// 正经对局里是 `None` —— 起始月由会话种子的历法子流抽。钉死它是为了截图与验收：
/// `NM_DAY` 只钉 tick，钉不住日期；而「闰日」「圣诞节」这类状态必须落到具体某一天。
#[derive(Resource, Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EpochOverride(pub Option<Date>);

pub fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// `tick 0` 是开局那天。一天 = 一个回合结束（行情在那里 tick 一次），所以不需要第二根钟。
pub fn date_of(tick: u32, epoch: Date) -> Date {
    epoch.add_days(tick as i64)
}

/// 月末吗 —— 储金与贷款结息的那一天。口径与 P2④ 相同：回合结束时 tick 已推进到新的一天，
/// 所以这问的是「今天开局是不是月末」，利息在月末那天的开头到账。
pub fn is_month_end(date: Date) -> bool {
    date.day == days_in_month(date.year, date.month)
}

/// 今天发不发股息。`dividend_day` 超出当月天数时收拢到月末 —— 配置写错该表现为
/// 「派息日挪到月末」，而不是「这局永远不发钱」。
pub fn is_dividend_day(date: Date, dividend_day: u32) -> bool {
    date.day == dividend_day.clamp(1, days_in_month(date.year, date.month))
}

/// 下一个派息日还有几天。同一天不返回 0（那笔已经发过了，屏幕要说的是下一次）。
pub fn days_until_dividend(date: Date, dividend_day: u32) -> u32 {
    let wanted = dividend_day.clamp(1, 28);
    let this_month = Date::new(
        date.year,
        date.month,
        wanted.min(days_in_month(date.year, date.month)),
    );
    let next = if this_month.to_days() > date.to_days() {
        this_month
    } else {
        let (year, month) = if date.month == 12 {
            (date.year + 1, 1)
        } else {
            (date.year, date.month + 1)
        };
        Date::new(year, month, wanted.min(days_in_month(year, month)))
    };
    (next.to_days() - date.to_days()) as u32
}

/// 一年的农历四节（公历日期）。`[月, 日]`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct LunarYear {
    pub year: i32,
    pub spring_festival: [u32; 2],
    pub qingming: [u32; 2],
    pub dragon_boat: [u32; 2],
    pub mid_autumn: [u32; 2],
}

impl LunarYear {
    fn at(&self, md: [u32; 2]) -> Date {
        Date::new(self.year, md[0], md[1])
    }
}

/// 节假日表。既是资产（从 JSON 加载）也是资源（规则层与 UI 都直接读）。
#[derive(Debug, Clone, Default, PartialEq, Asset, TypePath, Deserialize, Resource)]
pub struct HolidaySet {
    pub years: Vec<LunarYear>,
}

impl HolidaySet {
    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();
        let mut last = i32::MIN;
        for y in &self.years {
            if y.year <= last {
                errs.push(format!("年份必须严格递增且不重复：{}", y.year));
            }
            last = y.year;
            for (what, md) in [
                ("春节", y.spring_festival),
                ("清明", y.qingming),
                ("端午", y.dragon_boat),
                ("中秋", y.mid_autumn),
            ] {
                if !(1..=12).contains(&md[0]) || md[1] == 0 || md[1] > days_in_month(y.year, md[0])
                {
                    errs.push(format!("{} 年{what}日期非法：{:?}", y.year, md));
                }
            }
        }
        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }

    /// 表覆盖的年份区间（`None` = 空表）。
    pub fn coverage(&self) -> Option<(i32, i32)> {
        let first = self.years.first()?.year;
        let last = self.years.last()?.year;
        Some((first, last))
    }

    pub fn lunar(&self, year: i32) -> Option<&LunarYear> {
        self.years.iter().find(|y| y.year == year)
    }

    /// 春节第一天（正月初一）。表外年份返回 `None` —— 红包因此只在表内年份发。
    pub fn spring_festival(&self, year: i32) -> Option<Date> {
        self.lunar(year).map(|y| y.at(y.spring_festival))
    }

    /// 今天是哪个节日。固定公历四条不查表；农历四节查表，表外年份只剩前者。
    pub fn holiday_name(&self, date: Date) -> Option<&'static str> {
        let fixed = match (date.month, date.day) {
            (1, 1) => Some("元旦"),
            (5, 1..=3) => Some("劳动节"),
            (10, 1..=3) => Some("国庆节"),
            (12, 25) => Some("圣诞"),
            _ => None,
        };
        if fixed.is_some() {
            return fixed;
        }
        let y = self.lunar(date.year)?;
        if date == y.at(y.spring_festival)
            || date == y.at(y.spring_festival).add_days(1)
            || date == y.at(y.spring_festival).add_days(2)
        {
            return Some("春节");
        }
        if date == y.at(y.qingming) {
            return Some("清明");
        }
        if date == y.at(y.dragon_boat) {
            return Some("端午");
        }
        if date == y.at(y.mid_autumn) {
            return Some("中秋");
        }
        None
    }

    /// 股市今天开不开。周末 ∪ 节假日 —— **银行不看这一条**（10-20 实报更正）。
    pub fn is_market_closed(&self, date: Date) -> bool {
        date.is_weekend() || self.holiday_name(date).is_some()
    }
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = (month as i32 + 9) % 12; // 3 月 = 0
    let doy = (153 * mp + 2) / 5 + day as i32 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era as i64 * 146_097 + doe as i64 - 719_468
}

fn civil_from_days(days: i64) -> (i32, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = y + i64::from(month <= 2);
    (year as i32, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table() -> HolidaySet {
        serde_json::from_str(include_str!("../../../assets/data/calendar/holidays.json"))
            .expect("holidays.json 应能解析")
    }

    #[test]
    fn civil_conversion_matches_known_dates() {
        assert_eq!(Date::new(1970, 1, 1).to_days(), 0);
        assert_eq!(Date::new(1970, 1, 1).weekday(), 4, "1970-01-01 是周四");
        assert_eq!(Date::new(2048, 1, 1).weekday(), 3, "2048-01-01 是周三");
        assert_eq!(
            Date::new(2048, 2, 29).to_days() - Date::new(2048, 2, 28).to_days(),
            1
        );
        assert_eq!(
            Date::new(2048, 3, 1).to_days() - Date::new(2048, 2, 29).to_days(),
            1
        );
        assert_eq!(
            Date::new(2049, 1, 1).to_days() - Date::new(2048, 12, 31).to_days(),
            1,
            "跨年"
        );
    }

    #[test]
    fn every_day_roundtrips_for_four_centuries() {
        for days in (-719_468..100_000).step_by(37) {
            let d = Date::from_days(days);
            assert_eq!(d.to_days(), days, "{d} 换算回去偏了");
            assert!((1..=12).contains(&d.month));
            assert!((1..=days_in_month(d.year, d.month)).contains(&d.day));
        }
    }

    #[test]
    fn leap_rules_include_the_century_exception() {
        assert!(is_leap_year(2048));
        assert!(!is_leap_year(2049));
        assert!(is_leap_year(2000), "400 年一闰");
        assert!(!is_leap_year(2100), "世纪年不闰");
        assert_eq!(days_in_month(2100, 2), 28);
        assert_eq!(days_in_month(2048, 2), 29);
    }

    #[test]
    fn epoch_plus_tick_is_a_civil_date() {
        let epoch = Date::new(2048, 1, 1);
        assert_eq!(date_of(0, epoch), epoch);
        assert_eq!(date_of(59, epoch), Date::new(2048, 2, 29), "闰日真的在局内");
        assert_eq!(date_of(60, epoch), Date::new(2048, 3, 1));
        assert_eq!(date_of(365, epoch), Date::new(2048, 12, 31));
        assert_eq!(date_of(366, epoch), Date::new(2049, 1, 1), "跨年到 2049");
    }

    #[test]
    fn month_end_follows_the_real_month_length() {
        assert!(is_month_end(Date::new(2048, 2, 29)));
        assert!(!is_month_end(Date::new(2048, 2, 28)));
        assert!(is_month_end(Date::new(2049, 2, 28)), "平年 2 月只有 28 天");
        assert!(is_month_end(Date::new(2048, 4, 30)));
        assert!(!is_month_end(Date::new(2048, 4, 29)));
    }

    #[test]
    fn the_fifteenth_is_a_date_not_a_tick_modulus() {
        assert!(!is_dividend_day(Date::new(2048, 1, 14), 15));
        assert!(is_dividend_day(Date::new(2048, 1, 15), 15));
        assert!(!is_dividend_day(Date::new(2048, 2, 15), 14));
    }

    #[test]
    fn days_until_dividend_rolls_into_the_next_month() {
        assert_eq!(
            days_until_dividend(Date::new(2048, 1, 14), 15),
            1,
            "明天就派息"
        );
        // 2048 是闰年：2 月有 29 天，2/15 → 3/15 是 29 天
        assert_eq!(days_until_dividend(Date::new(2048, 2, 15), 15), 29);
        assert_eq!(
            days_until_dividend(Date::new(2048, 12, 20), 15),
            26,
            "跨年：12/20 → 1/15"
        );
    }

    #[test]
    fn the_holiday_table_covers_only_the_documented_years() {
        let t = table();
        t.validate().expect("内置表必须自洽");
        assert_eq!(t.coverage(), Some((2048, 2052)));
        assert_eq!(t.spring_festival(2052), Some(Date::new(2052, 2, 1)));
        assert_eq!(t.holiday_name(Date::new(2052, 9, 7)), Some("中秋"));
        assert_eq!(t.spring_festival(2053), None, "表外年份必须显式降级");
    }

    #[test]
    fn fixed_holidays_do_not_need_the_table() {
        let empty = HolidaySet::default();
        for (date, name) in [
            (Date::new(2053, 1, 1), "元旦"),
            (Date::new(2053, 5, 3), "劳动节"),
            (Date::new(2053, 10, 2), "国庆节"),
            (Date::new(2053, 12, 25), "圣诞"),
        ] {
            assert_eq!(empty.holiday_name(date), Some(name), "{date} 应照常放假");
        }
        assert_eq!(
            empty.holiday_name(Date::new(2053, 2, 14)),
            None,
            "表外的农历节不放假"
        );
        assert_eq!(empty.holiday_name(Date::new(2053, 5, 4)), None);
    }

    #[test]
    fn spring_festival_lasts_three_days_and_other_lunar_ones_one() {
        let t = table();
        for day in 0..3 {
            let d = Date::new(2048, 2, 14).add_days(day);
            assert_eq!(t.holiday_name(d), Some("春节"), "{d}");
        }
        assert_eq!(t.holiday_name(Date::new(2048, 2, 17)), None);
        assert_eq!(t.holiday_name(Date::new(2048, 6, 15)), Some("端午"));
        assert_eq!(t.holiday_name(Date::new(2048, 6, 16)), None);
    }

    #[test]
    fn market_closes_on_weekends_and_holidays_but_the_set_is_a_union_not_a_sum() {
        let t = table();
        // 2048-01-04 是周六（1/1 周三）
        assert!(t.is_market_closed(Date::new(2048, 1, 4)));
        assert!(t.is_market_closed(Date::new(2048, 1, 1)), "元旦");
        assert!(t.is_market_closed(Date::new(2048, 5, 1)), "劳动节");
        assert!(!t.is_market_closed(Date::new(2048, 1, 3)));
    }

    /// 连跑 400 天（跨年、跨闰日），逐日与一套**独立写法**的月长循环对齐。
    /// 这不是把同一个公式抄两遍：参考实现只按「月满进位、2 月看闰年」推，不碰 civil-from-days。
    #[test]
    fn four_hundred_days_match_a_simple_reference_calendar() {
        let epoch = Date::new(2048, 1, 1);
        let (mut y, mut m, mut d) = (epoch.year, epoch.month, epoch.day);
        for tick in 0..400u32 {
            let got = date_of(tick, epoch);
            assert_eq!(got, Date::new(y, m, d), "第 {tick} 天");
            d += 1;
            if d > days_in_month(y, m) {
                d = 1;
                m += 1;
                if m > 12 {
                    m = 1;
                    y += 1;
                }
            }
        }
        assert_eq!(date_of(400, epoch), Date::new(2049, 2, 4));
    }

    /// 派息（15 号）与月末结息必须是两个日子 —— 这是 P2b ⑤ 的口径。
    #[test]
    fn dividend_and_interest_never_share_a_day() {
        let mut d = Date::new(2048, 1, 1);
        for _ in 0..366 {
            assert!(
                !(is_dividend_day(d, 15) && is_month_end(d)),
                "{d} 同时是派息日与月末"
            );
            d = d.add_days(1);
        }
    }

    /// 验收 ⑥：12 个起始月各跑到「现算的首个节日下限」回合，每局至少遇到一个节日。
    /// 下限从表里算（不写死 84）：起始日被钉在 1 号，最坏是 7-1 开局等中秋。
    #[test]
    fn every_starting_month_meets_a_holiday_within_the_computed_bound() {
        let t = table();
        let mut worst = 0;
        for month in 1..=12 {
            let start = Date::new(2048, month, 1);
            let found = (0..=200)
                .find(|day| t.holiday_name(start.add_days(*day)).is_some())
                .unwrap_or_else(|| panic!("{start} 开局 200 天内没有节日"));
            worst = worst.max(found);
        }
        assert_eq!(worst + 1, 84, "PLAN 记的下限是 84；表改了就跟着变");
    }

    #[test]
    fn the_worst_gap_between_holidays_is_computed_from_the_table() {
        // 验收 ⑥ 的「下限回合」就是这张表算出来的最长空档：别把 98/84 写死。
        let t = table();
        let start = Date::new(2048, 1, 1);
        let end = Date::new(2052, 12, 31);
        let mut longest = 0;
        let mut gap = 0;
        let mut d = start;
        while d <= end {
            if t.holiday_name(d).is_some() {
                longest = longest.max(gap);
                gap = 0;
            } else {
                gap += 1;
            }
            d = d.add_days(1);
        }
        assert!(longest > 0, "表里一个节日都没有？");
        assert!(
            longest <= 98,
            "五年表里最长空档 {longest} 天，PLAN 记的是 98"
        );
    }

    #[test]
    fn a_broken_holiday_row_is_rejected() {
        let mut t = table();
        t.years[0].spring_festival = [2, 30];
        assert!(t.validate().is_err(), "2 月 30 日必须被拒");
    }
}
