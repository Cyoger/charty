use tokio::sync::mpsc;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct LivePrice {
    pub symbol: String,
    pub price: f64,
    pub timestamp: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
    Connecting,
    Error(String),
}

pub struct WebSocketManager {
    pub status: Arc<Mutex<ConnectionStatus>>,
    api_key: Option<String>,
}

impl WebSocketManager {
    pub fn new(api_key: Option<String>) -> Self {
        Self {
            status: Arc::new(Mutex::new(ConnectionStatus::Disconnected)),
            api_key,
        }
    }

    pub async fn start(
        &self,
        symbol: String,
        _base_price: f64,
        tx: mpsc::UnboundedSender<LivePrice>,
        should_stop: Arc<Mutex<bool>>,
    ) {
        if let Some(ref api_key) = self.api_key {
            self.start_finnhub_websocket(symbol, api_key.clone(), tx, should_stop).await;
        } else {
            *self.status.lock().await = ConnectionStatus::Error(
                "No API key configured. Set FINNHUB_API_KEY environment variable.".to_string()
            );
        }
    }

    async fn start_finnhub_websocket(
        &self,
        symbol: String,
        api_key: String,
        tx: mpsc::UnboundedSender<LivePrice>,
        should_stop: Arc<Mutex<bool>>,
    ) {
        *self.status.lock().await = ConnectionStatus::Connecting;

        let url = format!("wss://ws.finnhub.io?token={}", api_key);
        
        match connect_async(&url).await {
            Ok((ws_stream, _)) => {
                *self.status.lock().await = ConnectionStatus::Connected;
                
                let (mut write, mut read) = ws_stream.split();
                
                // Subscribe to symbol
                let subscribe_msg = serde_json::json!({
                    "type": "subscribe",
                    "symbol": symbol
                });
                
                if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                    *self.status.lock().await = ConnectionStatus::Error(format!("Failed to subscribe: {}", e));
                    return;
                }
                
                // Listen for updates
                loop {
                    // Check if we should stop
                    if *should_stop.lock().await {
                        // Unsubscribe before closing
                        let unsubscribe_msg = serde_json::json!({
                            "type": "unsubscribe",
                            "symbol": symbol
                        });
                        let _ = write.send(Message::Text(unsubscribe_msg.to_string())).await;
                        break;
                    }

                    // Use select to check both the WebSocket and stop flag
                    tokio::select! {
                        msg = read.next() => {
                            match msg {
                                Some(Ok(Message::Text(text))) => {
                                    if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                        // Finnhub sends trades in this format:
                                        // {"type":"trade","data":[{"p":price,"s":"SYMBOL","t":timestamp,...}]}
                                        if json["type"] == "trade" {
                                            if let Some(data) = json["data"].as_array() {
                                                for trade in data {
                                                    if let (Some(price), Some(ts)) = (
                                                        trade["p"].as_f64(),
                                                        trade["t"].as_i64(),
                                                    ) {
                                                        let live_price = LivePrice {
                                                            symbol: symbol.clone(),
                                                            price,
                                                            timestamp: ts / 1000, // Convert ms to seconds
                                                        };
                                                        
                                                        if tx.send(live_price).is_err() {
                                                            return;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                                Some(Ok(Message::Ping(data))) => {
                                    let _ = write.send(Message::Pong(data)).await;
                                }
                                Some(Err(e)) => {
                                    *self.status.lock().await = ConnectionStatus::Error(format!("WebSocket error: {}", e));
                                    break;
                                }
                                None => {
                                    *self.status.lock().await = ConnectionStatus::Disconnected;
                                    break;
                                }
                                _ => {}
                            }
                        }
                        _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                        }
                    }
                }
                
                *self.status.lock().await = ConnectionStatus::Disconnected;
            }
            Err(e) => {
                *self.status.lock().await = ConnectionStatus::Error(format!("Failed to connect: {}", e));
            }
        }
    }
}

pub async fn start_websocket(
    symbol: String,
    base_price: f64,
    tx: mpsc::UnboundedSender<LivePrice>,
    should_stop: Arc<Mutex<bool>>,
) {
    let api_key = std::env::var("FINNHUB_API_KEY").ok();
    
    let manager = WebSocketManager::new(api_key);
    manager.start(symbol, base_price, tx, should_stop).await;
}