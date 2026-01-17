use chrono::{DateTime, Utc};

#[derive(Debug, Clone)]
pub struct StockData {
    pub symbol: String,
    pub timestamps: Vec<DateTime<Utc>>,
    pub prices: Vec<f64>,
    pub current_price: f64,
    pub change: f64,
    pub change_percent: f64,
}

pub fn fetch_stock_data(symbol: &str) -> Result<StockData, Box<dyn std::error::Error>> {
    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?interval=1d&range=1mo",
        symbol
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
    })
}