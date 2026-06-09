use chrono::{DateTime, Utc};
use std::collections::{HashMap, VecDeque};

#[cfg(debug_assertions)]
pub fn log_debug(msg: &str) {
    use std::io::Write;
    use std::fs::OpenOptions;
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open("debug.log") {
        let _ = writeln!(f, "{}", msg);
    }
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn log_debug(_msg: &str) {}

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
    pub volumes: Vec<f64>,
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
        let timeout = std::time::Duration::from_secs(10);
        let agent = ureq::AgentBuilder::new()
            .cookie_store(cookie_store::CookieStore::default())
            .timeout(timeout)
            .build();

        // Hit homepage to populate cookie jar
        let _ = agent
            .get("https://finance.yahoo.com/")
            .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
            .set("Accept", "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8")
            .set("Accept-Language", "en-US,en;q=0.5")
            .call();

        // Try both query1 and query2 hosts for the crumb
        let crumb_result = ["https://query1.finance.yahoo.com/v1/test/getcrumb",
                            "https://query2.finance.yahoo.com/v1/test/getcrumb"]
            .iter()
            .find_map(|url| {
                agent.get(url)
                    .set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/125.0.0.0 Safari/537.36")
                    .set("Accept", "*/*")
                    .call()
                    .ok()
                    .and_then(|r| r.into_string().ok())
                    .filter(|s| !s.is_empty() && !s.contains('{'))
            });

        let crumb = crumb_result.ok_or("Failed to get crumb from all endpoints")?;
        log_debug(&format!("[session] crumb obtained: {:?}", &crumb[..crumb.len().min(80)]));

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
            MarketState::Regular => Some("O"),
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
            let raw_state = q["marketState"].as_str().unwrap_or("<missing>");
            log_debug(&format!("[quote API] {} marketState={:?}", sym, raw_state));
            let state = MarketState::from_str(raw_state);

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

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(10))
        .call()?;
    let json: serde_json::Value = response.into_json()?;

    let chart = &json["chart"]["result"][0];

    let market_state = {
        let now = Utc::now().timestamp();
        let tp = &chart["meta"]["currentTradingPeriod"];
        let reg_start = tp["regular"]["start"].as_i64().unwrap_or(0);
        let reg_end   = tp["regular"]["end"].as_i64().unwrap_or(0);
        let pre_start = tp["pre"]["start"].as_i64().unwrap_or(0);
        let pre_end   = tp["pre"]["end"].as_i64().unwrap_or(0);
        let post_start = tp["post"]["start"].as_i64().unwrap_or(0);
        let post_end   = tp["post"]["end"].as_i64().unwrap_or(0);
        log_debug(&format!("[chart API] {} now={} reg={}-{} pre={}-{} post={}-{}", symbol, now, reg_start, reg_end, pre_start, pre_end, post_start, post_end));
        if reg_start > 0 && now >= reg_start && now < reg_end {
            MarketState::Regular
        } else if pre_start > 0 && now >= pre_start && now < pre_end {
            MarketState::Pre
        } else if post_start > 0 && now >= post_start && now < post_end {
            MarketState::Post
        } else {
            MarketState::Closed
        }
    };

    let raw_timestamps = chart["timestamp"].as_array().ok_or("No timestamp data")?;
    let quote = &chart["indicators"]["quote"][0];
    let raw_closes  = quote["close"].as_array().ok_or("No close data")?;
    let raw_volumes = quote["volume"].as_array();

    let mut timestamps = Vec::new();
    let mut prices    = Vec::new();
    let mut volumes   = Vec::new();

    for i in 0..raw_closes.len().min(raw_timestamps.len()) {
        if let (Some(close), Some(ts)) = (raw_closes[i].as_f64(), raw_timestamps[i].as_i64()) {
            timestamps.push(DateTime::from_timestamp(ts, 0).unwrap());
            prices.push(close);
            let vol = raw_volumes
                .and_then(|v| v.get(i))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            volumes.push(vol);
        }
    }

    let current_price = *prices.last().ok_or("No price data")?;
    let first_price   = *prices.first().ok_or("No price data")?;
    let change         = current_price - first_price;
    let change_percent = (change / first_price) * 100.0;

    Ok(StockData {
        symbol: symbol.to_string(),
        timestamps,
        prices,
        volumes,
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
        .timeout(std::time::Duration::from_secs(10))
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

// ── Historical candles (Yahoo Finance v8) ────────────────────────────────────

pub fn fetch_historical_candles(
    symbol: &str,
    interval: &str,
) -> Result<Vec<crate::ui::Candlestick>, Box<dyn std::error::Error>> {
    use crate::ui::Candlestick;

    // Pick a range wide enough to yield ~60+ candles per interval
    let range = match interval {
        "1m" => "1d",
        "5m" => "1d",
        "15m" => "5d",
        "30m" => "5d",
        "1h" => "1mo",
        _ => "1d",
    };

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval={}&range={}&includePrePost=false",
        symbol, interval, range
    );

    let response = ureq::get(&url)
        .set("User-Agent", "Mozilla/5.0")
        .timeout(std::time::Duration::from_secs(10))
        .call()?;
    let json: serde_json::Value = response.into_json()?;

    let chart = &json["chart"]["result"][0];

    let timestamps = chart["timestamp"]
        .as_array()
        .ok_or("No timestamp data")?;

    let quote = &chart["indicators"]["quote"][0];
    let opens   = quote["open"].as_array().ok_or("No open data")?;
    let highs   = quote["high"].as_array().ok_or("No high data")?;
    let lows    = quote["low"].as_array().ok_or("No low data")?;
    let closes  = quote["close"].as_array().ok_or("No close data")?;
    let volumes = quote["volume"].as_array().ok_or("No volume data")?;

    let mut candles = Vec::new();

    for i in 0..timestamps.len() {
        if let (Some(t), Some(o), Some(h), Some(l), Some(c)) = (
            timestamps[i].as_i64(),
            opens[i].as_f64(),
            highs[i].as_f64(),
            lows[i].as_f64(),
            closes[i].as_f64(),
        ) {
            let volume = volumes[i].as_u64().unwrap_or(0);
            candles.push(Candlestick {
                open: o,
                high: h,
                low: l,
                close: c,
                volume,
                timestamp: DateTime::from_timestamp(t, 0).unwrap_or_else(Utc::now),
                trade_count: 0,
            });
        }
        // entries with null OHLC values (e.g. gaps) are skipped
    }

    Ok(candles)
}
