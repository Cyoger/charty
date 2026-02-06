use chrono::{DateTime, Utc};
use std::collections::VecDeque;

#[derive(Debug, Clone)]
#[allow(dead_code)]
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

fn yahoo_to_finnhub_symbol(yahoo_symbol: &str) -> &str {
    // Map Yahoo Finance symbols to Finnhub symbols
    match yahoo_symbol {
        "^GSPC" => "SPX",      // S&P 500
        "^DJI" => "DJI",       // Dow Jones
        "^IXIC" => "IXIC",     // Nasdaq
        "^VIX" => "VIX",       // Volatility Index
        "BTC-USD" => "BINANCE:BTCUSDT",  // Bitcoin
        "ETH-USD" => "BINANCE:ETHUSDT",  // Ethereum
        _ => yahoo_symbol,     // Use as-is for stocks
    }
}

pub fn fetch_historical_candles(
    symbol: &str,
    resolution: &str,
    count: usize,
) -> Result<Vec<crate::ui::Candlestick>, Box<dyn std::error::Error>> {
    use crate::ui::Candlestick;

    let api_key = std::env::var("FINNHUB_API_KEY")
        .map_err(|_| "FINNHUB_API_KEY not set")?;

    // Convert Yahoo symbol to Finnhub symbol
    let finnhub_symbol = yahoo_to_finnhub_symbol(symbol);

    let now = Utc::now().timestamp();
    let seconds_per_candle = match resolution {
        "1" => 60,
        "5" => 300,
        "15" => 900,
        "30" => 1800,
        "60" => 3600,
        _ => 60,
    };
    let from = now - (seconds_per_candle * count as i64);

    let url = format!(
        "https://finnhub.io/api/v1/stock/candle?symbol={}&resolution={}&from={}&to={}&token={}",
        finnhub_symbol, resolution, from, now, api_key.trim()
    );

    let response = ureq::get(&url).call()?;
    let json: serde_json::Value = response.into_json()?;

    // Check if we got valid data
    if json["s"].as_str() != Some("ok") {
        return Err("No candle data available".into());
    }

    let opens = json["o"].as_array().ok_or("No open data")?;
    let highs = json["h"].as_array().ok_or("No high data")?;
    let lows = json["l"].as_array().ok_or("No low data")?;
    let closes = json["c"].as_array().ok_or("No close data")?;
    let volumes = json["v"].as_array().ok_or("No volume data")?;
    let timestamps = json["t"].as_array().ok_or("No timestamp data")?;

    let mut candles = Vec::new();

    for i in 0..opens.len() {
        if let (Some(o), Some(h), Some(l), Some(c), Some(v), Some(t)) = (
            opens[i].as_f64(),
            highs[i].as_f64(),
            lows[i].as_f64(),
            closes[i].as_f64(),
            volumes[i].as_f64(),
            timestamps[i].as_i64(),
        ) {
            candles.push(Candlestick {
                open: o,
                high: h,
                low: l,
                close: c,
                volume: v as u64,
                timestamp: DateTime::from_timestamp(t, 0).unwrap_or_else(Utc::now),
                trade_count: 0, // Not provided by Finnhub API
            });
        }
    }

    Ok(candles)
}