//! 股票定义。运行时状态（现价、历史、tick 数）不进这里 —— 那是实体组件。
//!
//! 对应 Godot 端 `stock_data.gd`：那边 `load_from_dict` 会把 `current_price` 也塞进定义，
//! 于是静态配置与对局状态混在同一个对象里，存档时难以区分。

use bevy::asset::Asset;
use bevy::reflect::TypePath;
use serde::Deserialize;

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StockDef {
    pub id: String,
    pub name: String,
    pub base_price: f32,
    /// 每 tick 的随机游走幅度上限。
    pub volatility: f32,
    pub price_floor: f32,
    pub price_ceiling: f32,
    pub outstanding_shares: u32,
}

#[derive(Debug, Clone, PartialEq, Asset, TypePath, Deserialize)]
#[serde(transparent)]
pub struct StockSet(pub Vec<StockDef>);

impl StockSet {
    pub fn from_json(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(text)
    }

    pub fn find(&self, id: &str) -> Option<&StockDef> {
        self.0.iter().find(|s| s.id == id)
    }

    pub fn validate(&self) -> Result<(), Vec<String>> {
        let mut errs = Vec::new();

        if self.0.is_empty() {
            return Err(vec!["股票列表为空".into()]);
        }

        let mut seen = std::collections::HashSet::new();
        for s in &self.0 {
            if s.id.is_empty() {
                errs.push("有股票 id 为空".into());
            } else if !seen.insert(s.id.as_str()) {
                errs.push(format!("股票 id 重复: {}", s.id));
            }
            if s.name.is_empty() {
                errs.push(format!("股票 {} 缺 name", s.id));
            }
            if !(s.price_floor > 0.0 && s.price_floor < s.price_ceiling) {
                errs.push(format!(
                    "股票 {} 的价格区间非法: [{}, {}]",
                    s.id, s.price_floor, s.price_ceiling
                ));
            }
            if !(s.base_price > 0.0
                && s.base_price >= s.price_floor
                && s.base_price <= s.price_ceiling)
            {
                errs.push(format!(
                    "股票 {} 的 base_price={} 不在区间内",
                    s.id, s.base_price
                ));
            }
            if !(s.volatility > 0.0 && s.volatility <= 1.0) {
                errs.push(format!(
                    "股票 {} 的 volatility={} 应落在 (0,1]",
                    s.id, s.volatility
                ));
            }
            if s.outstanding_shares == 0 {
                errs.push(format!("股票 {} 的流通股数为 0", s.id));
            }
        }

        if errs.is_empty() { Ok(()) } else { Err(errs) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const STOCKS: &str = include_str!("../../../assets/data/stocks/stocks.json");

    fn set() -> StockSet {
        StockSet::from_json(STOCKS).expect("stocks.json 应能解析")
    }

    #[test]
    fn stocks_parse_and_validate() {
        let s = set();
        assert_eq!(s.0.len(), 5);
        assert!(
            s.validate().is_ok(),
            "校验失败: {:?}",
            s.validate().unwrap_err()
        );
        assert_eq!(s.find("stock_001").unwrap().name, "暴富科技");
        assert!(s.find("不存在").is_none());
    }

    #[test]
    fn broken_stock_definitions_are_rejected() {
        type Breaker = fn(&mut StockSet);
        let cases: Vec<(Breaker, &str)> = vec![
            (|s| s.0.clear(), "空列表"),
            (|s| s.0[0].id = s.0[1].id.clone(), "id 重复"),
            (|s| s.0[0].price_floor = 500.0, "区间倒挂"),
            (|s| s.0[0].base_price = 999.0, "基准价越界"),
            (|s| s.0[0].volatility = 0.0, "波动率为 0"),
            (|s| s.0[0].outstanding_shares = 0, "零流通股"),
        ];

        for (break_it, label) in cases {
            let mut s = set();
            break_it(&mut s);
            assert!(s.validate().is_err(), "坏数据被放过了（{label}）");
        }
    }
}
