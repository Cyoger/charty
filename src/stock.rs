use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};

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
    pub market_state: MarketState,
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

// ── Yahoo session (crumb + cookie jar) ───────────────────────────────────────

pub struct YahooSession {
    agent: ureq::Agent,
    crumb: String,
}

impl YahooSession {
    pub fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let agent = ureq::AgentBuilder::new()
            .cookie_store(cookie_store::CookieStore::default())
            .build();

        // Hit homepage to populate cookie jar
        let _ = agent
            .get("https://finance.yahoo.com/")
            .set("User-Agent", "Mozilla/5.0")
            .call();

        let crumb = agent
            .get("https://query1.finance.yahoo.com/v1/test/getcrumb")
            .set("User-Agent", "Mozilla/5.0")
            .call()?
            .into_string()?;

        if crumb.contains('{') {
            return Err("Failed to get crumb (auth rejected)".into());
        }

        Ok(Self { agent, crumb })
    }
}

// ── Quote snapshot ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq)]
pub enum MarketState {
    Regular,
    Pre,
    Post,
    Closed,
}

impl MarketState {
    fn from_str(s: &str) -> Self {
        match s {
            "REGULAR" => MarketState::Regular,
            "PRE" | "PREPRE" => MarketState::Pre,
            "POST" | "POSTPOST" => MarketState::Post,
            _ => MarketState::Closed,
        }
    }

    pub fn label(&self) -> Option<&'static str> {
        match self {
            MarketState::Regular => None,
            MarketState::Pre => Some("PM"),
            MarketState::Post => Some("AH"),
            MarketState::Closed => Some("C"),
        }
    }
}

#[derive(Debug, Clone)]
pub struct QuoteSnapshot {
    pub price: f64,
    pub change_percent: f64,
    pub market_state: MarketState,
}

pub fn fetch_batch_quotes(
    session: &YahooSession,
    symbols: &[&str],
) -> Result<HashMap<String, QuoteSnapshot>, Box<dyn std::error::Error>> {
    let joined = symbols.join(",");
    let url = format!(
        "https://query1.finance.yahoo.com/v7/finance/quote?symbols={}&crumb={}&fields=regularMarketPrice,regularMarketChangePercent,marketState",
        joined, session.crumb
    );

    let response = session
        .agent
        .get(&url)
        .set("User-Agent", "Mozilla/5.0")
        .call()?;
    let json: serde_json::Value = response.into_json()?;

    let results = json["quoteResponse"]["result"]
        .as_array()
        .ok_or("No quote results")?;

    let mut map = HashMap::new();
    for q in results {
        if let (Some(sym), Some(price), Some(chg)) = (
            q["symbol"].as_str(),
            q["regularMarketPrice"].as_f64(),
            q["regularMarketChangePercent"].as_f64(),
        ) {
            let state = q["marketState"]
                .as_str()
                .map(MarketState::from_str)
                .unwrap_or(MarketState::Closed);

            map.insert(sym.to_string(), QuoteSnapshot {
                price,
                change_percent: chg,
                market_state: state,
            });
        }
    }

    Ok(map)
}

// ── Stock chart data ──────────────────────────────────────────────────────────

pub fn fetch_stock_data(symbol: &str, timeframe: TimeFrame) -> Result<StockData, Box<dyn std::error::Error>> {
    // Include pre/post market data for intraday view
    let include_prepost = matches!(timeframe, TimeFrame::OneDay);
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}&includePrePost={}",
        symbol,
        timeframe.to_interval(),
        timeframe.to_api_string(),
        include_prepost,
    );

    let response = ureq::get(&url).call()?;
    let json: serde_json::Value = response.into_json()?;

    let chart = &json["chart"]["result"][0];

    let market_state = chart["meta"]["marketState"]
        .as_str()
        .map(MarketState::from_str)
        .unwrap_or(MarketState::Closed);

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
        market_state,
    })
}

// ── Market movers ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct MarketMover {
    pub symbol: String,
    pub name: String,
    pub price: f64,
    pub change: f64,
    pub change_percent: f64,
    pub volume: u64,
}

pub fn fetch_market_movers(scr_id: &str, count: usize) -> Result<Vec<MarketMover>, Box<dyn std::error::Error>> {
    let url = format!(
        "https://query1.finance.yahoo.com/v1/finance/screener/predefined/saved?scrIds={}&count={}",
        scr_id, count
    );

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0")
        .call()?;
    let json: serde_json::Value = response.into_json()?;

    let quotes = json["finance"]["result"][0]["quotes"]
        .as_array()
        .ok_or("No quotes data")?;

    let mut movers = Vec::new();
    for quote in quotes {
        let symbol = quote["symbol"].as_str().unwrap_or("").to_string();
        if symbol.is_empty() { continue; }

        let name = quote["shortName"].as_str()
            .or_else(|| quote["longName"].as_str())
            .unwrap_or(&symbol)
            .to_string();
        let price = quote["regularMarketPrice"].as_f64().unwrap_or(0.0);
        let change = quote["regularMarketChange"].as_f64().unwrap_or(0.0);
        let change_percent = quote["regularMarketChangePercent"].as_f64().unwrap_or(0.0);
        let volume = quote["regularMarketVolume"].as_u64().unwrap_or(0);

        movers.push(MarketMover { symbol, name, price, change, change_percent, volume });
    }

    Ok(movers)
}

// ── Historical candles (Finnhub) ──────────────────────────────────────────────

fn yahoo_to_finnhub_symbol(yahoo_symbol: &str) -> &str {
    match yahoo_symbol {
        "^GSPC" => "SPX",
        "^DJI" => "DJI",
        "^IXIC" => "IXIC",
        "^VIX" => "VIX",
        "BTC-USD" => "BINANCE:BTCUSDT",
        "ETH-USD" => "BINANCE:ETHUSDT",
        _ => yahoo_symbol,
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
                trade_count: 0,
            });
        }
    }

    Ok(candles)
}
