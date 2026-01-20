use chrono::{DateTime, Utc};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
pub struct LiveTick {
    pub price: f64,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct StockData {
    pub symbol: String,
    pub timestamps: Vec<DateTime<Utc>>,
    pub prices: Vec<f64>,
    pub current_price: f64,
    pub change: f64,
    pub change_percent: f64,
    pub live_ticks: VecDeque<LiveTick>,
    pub live_current_price: Option<f64>,
    pub base_historical_price: f64,
}

#[derive(Debug, Clone, Copy)]
pub enum TimeFrame {
    OneDay,
    OneWeek,
    OneMonth,
    ThreeMonths,
    OneYear,
}

impl TimeFrame {
    pub fn to_api_string(&self) -> &str {
        match self {
            TimeFrame::OneDay => "1d",
            TimeFrame::OneWeek => "5d",
            TimeFrame::OneMonth => "1mo",
            TimeFrame::ThreeMonths => "3mo",
            TimeFrame::OneYear => "1y",
        }
    }

    pub fn to_interval(&self) -> &str {
        match self {
            TimeFrame::OneDay => "5m",
            TimeFrame::OneWeek => "30m",
            TimeFrame::OneMonth => "1d",
            TimeFrame::ThreeMonths => "1d",
            TimeFrame::OneYear => "1wk",
        }
    }

    pub fn display(&self) -> &str {
        match self {
            TimeFrame::OneDay => "1 Day",
            TimeFrame::OneWeek => "1 Week",
            TimeFrame::OneMonth => "1 Month",
            TimeFrame::ThreeMonths => "3 Months",
            TimeFrame::OneYear => "1 Year",
        }
    }

    pub fn next(&self) -> Self {
        match self {
            TimeFrame::OneDay => TimeFrame::OneWeek,
            TimeFrame::OneWeek => TimeFrame::OneMonth,
            TimeFrame::OneMonth => TimeFrame::ThreeMonths,
            TimeFrame::ThreeMonths => TimeFrame::OneYear,
            TimeFrame::OneYear => TimeFrame::OneDay,
        }
    }

    pub fn prev(&self) -> Self {
        match self {
            TimeFrame::OneDay => TimeFrame::OneYear,
            TimeFrame::OneWeek => TimeFrame::OneDay,
            TimeFrame::OneMonth => TimeFrame::OneWeek,
            TimeFrame::ThreeMonths => TimeFrame::OneMonth,
            TimeFrame::OneYear => TimeFrame::ThreeMonths,
        }
    }
}


pub fn fetch_stock_data(symbol: &str, timeframe: TimeFrame) -> Result<StockData, Box<dyn std::error::Error>> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}",
        symbol,
        timeframe.to_interval(),
        timeframe.to_api_string()
    );
    
    let response = ureq::get(&url).call()?;
    let json: serde_json::Value = response.into_json()?;
    
    // Parse the response
    let chart = &json["chart"]["result"][0];

    let timestamps: Vec<DateTime<Utc>> = chart["timestamp"]
        .as_array()
        .ok_or("No timestamp data")?
        .iter()
        .filter_map(|v| v.as_i64())
		.map(|ts| DateTime::from_timestamp(ts, 0).unwrap())
        .collect();
    
    let prices: Vec<f64> = chart["indicators"]["quote"][0]["close"]
        .as_array()
        .ok_or("No close data")?
        .iter()
        .filter_map(|v| v.as_f64())
        .collect();
    
    let current_price = *prices.last().ok_or("No price data")?;
    let first_price = *prices.first().ok_or("No price data")?;
    let change = current_price - first_price;
    let change_percent = (change / first_price) * 100.0;
    
    Ok(StockData {
        symbol: symbol.to_string(),
        timestamps,
        prices,
        current_price,
        change,
        change_percent,
        live_ticks: VecDeque::new(),
        live_current_price: None,
        base_historical_price: current_price,
    })
}