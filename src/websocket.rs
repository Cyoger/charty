use tokio::sync::mpsc;
use std::sync::Arc;
use tokio::sync::Mutex;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::time::Duration;
use chrono::Utc;
use crate::ui::WebSocketStatus;

// Reconnection configuration constants
const MAX_RECONNECT_ATTEMPTS: u32 = 5;
const BASE_DELAY_SECS: u64 = 2;
const MAX_DELAY_SECS: u64 = 32;

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LivePrice {
    pub symbol: String,
    pub price: f64,
    pub timestamp: i64,
    pub volume: Option<u64>,
}

#[derive(Debug)]
struct ReconnectionPolicy {
    max_attempts: u32,
    base_delay: Duration,
    max_delay: Duration,
    current_attempt: u32,
}

impl ReconnectionPolicy {
    fn new() -> Self {
        Self {
            max_attempts: MAX_RECONNECT_ATTEMPTS,
            base_delay: Duration::from_secs(BASE_DELAY_SECS),
            max_delay: Duration::from_secs(MAX_DELAY_SECS),
            current_attempt: 0,
        }
    }

    fn calculate_delay(&self) -> Duration {
        let delay = self.base_delay * 2_u32.pow(self.current_attempt);
        delay.min(self.max_delay)
    }

    fn should_retry(&self) -> bool {
        self.current_attempt < self.max_attempts
    }

    fn increment(&mut self) {
        self.current_attempt += 1;
    }

    fn reset(&mut self) {
        self.current_attempt = 0;
    }
}

fn log_to_file(message: &str) {
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("debug.log")
    {
        let timestamp = Utc::now().format("%Y-%m-%d %H:%M:%S");
        let _ = writeln!(file, "[{}] {}", timestamp, message);
    }
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
        status_tx: mpsc::UnboundedSender<WebSocketStatus>,
        should_stop: Arc<Mutex<bool>>,
    ) {
        if let Some(ref api_key) = self.api_key {
            self.start_finnhub_websocket(symbol, api_key.clone(), tx, status_tx, should_stop).await;
        } else {
            *self.status.lock().await = ConnectionStatus::Error(
                "No API key configured. Set FINNHUB_API_KEY environment variable.".to_string()
            );
            let _ = status_tx.send(WebSocketStatus::Error {
                message: "No API key configured".to_string(),
                recoverable: false,
            });
        }
    }

    async fn start_finnhub_websocket(
        &self,
        symbol: String,
        api_key: String,
        tx: mpsc::UnboundedSender<LivePrice>,
        status_tx: mpsc::UnboundedSender<WebSocketStatus>,
        should_stop: Arc<Mutex<bool>>,
    ) {
        let mut reconnection_policy = ReconnectionPolicy::new();

        // Reconnection loop
        loop {
            // Check if we should stop before attempting connection
            if *should_stop.lock().await {
                let _ = status_tx.send(WebSocketStatus::Disconnected);
                *self.status.lock().await = ConnectionStatus::Disconnected;
                log_to_file("WebSocket stopped by user");
                return;
            }

            // Send connecting status
            *self.status.lock().await = ConnectionStatus::Connecting;
            let _ = status_tx.send(WebSocketStatus::Connecting);
            let trimmed_key = api_key.trim();
            let url = format!("wss://ws.finnhub.io/?token={}", trimmed_key);
            log_to_file(&format!("WebSocket connecting to Finnhub for {}", symbol));

            match connect_async(&url).await {
                Ok((ws_stream, _)) => {
                    // Connection successful - reset reconnection counter
                    reconnection_policy.reset();
                    *self.status.lock().await = ConnectionStatus::Connected;
                    let connected_since = Utc::now();
                    let _ = status_tx.send(WebSocketStatus::Connected { since: connected_since });
                    log_to_file(&format!("WebSocket connected successfully for {}", symbol));

                    let (mut write, mut read) = ws_stream.split();

                    // Subscribe to symbol
                    let subscribe_msg = serde_json::json!({
                        "type": "subscribe",
                        "symbol": symbol
                    });

                    if let Err(e) = write.send(Message::Text(subscribe_msg.to_string())).await {
                        let error_msg = format!("Failed to subscribe: {}", e);
                        *self.status.lock().await = ConnectionStatus::Error(error_msg.clone());
                        let _ = status_tx.send(WebSocketStatus::Error {
                            message: "Subscription failed".to_string(),
                            recoverable: true,
                        });
                        log_to_file(&format!("WebSocket subscription error: {}", error_msg));
                        // Don't return - try to reconnect
                        continue;
                    }

                    log_to_file(&format!("WebSocket subscribed to {}", symbol));

                    // Listen for updates
                    let connection_result = self.handle_websocket_messages(
                        symbol.clone(),
                        &mut write,
                        &mut read,
                        &tx,
                        &should_stop,
                    ).await;

                    // Connection ended - check why
                    if *should_stop.lock().await {
                        // User requested stop
                        let unsubscribe_msg = serde_json::json!({
                            "type": "unsubscribe",
                            "symbol": symbol
                        });
                        let _ = write.send(Message::Text(unsubscribe_msg.to_string())).await;
                        let _ = status_tx.send(WebSocketStatus::Disconnected);
                        *self.status.lock().await = ConnectionStatus::Disconnected;
                        log_to_file("WebSocket disconnected by user");
                        return;
                    }

                    // Connection error - should we reconnect?
                    match connection_result {
                        ConnectionResult::Error(msg) => {
                            log_to_file(&format!("WebSocket error: {}", msg));
                            // Determine if error is recoverable
                            let recoverable = !msg.to_lowercase().contains("auth")
                                && !msg.to_lowercase().contains("invalid")
                                && !msg.to_lowercase().contains("api key");

                            if !recoverable {
                                let _ = status_tx.send(WebSocketStatus::Error {
                                    message: msg.clone(),
                                    recoverable: false,
                                });
                                *self.status.lock().await = ConnectionStatus::Error(msg);
                                log_to_file("WebSocket encountered fatal error, not reconnecting");
                                return;
                            }
                            // Recoverable error - fall through to reconnection logic
                        }
                        ConnectionResult::Disconnected => {
                            log_to_file("WebSocket disconnected unexpectedly");
                            // Fall through to reconnection logic
                        }
                    }
                }
                Err(e) => {
                    let error_msg = format!("Failed to connect: {}", e);
                    *self.status.lock().await = ConnectionStatus::Error(error_msg.clone());
                    log_to_file(&format!("WebSocket connection error: {}", error_msg));

                    // Check if this is an auth error (fatal)
                    let error_str = e.to_string().to_lowercase();
                    if error_str.contains("auth") || error_str.contains("401") || error_str.contains("403") {
                        let _ = status_tx.send(WebSocketStatus::Error {
                            message: "Authentication failed".to_string(),
                            recoverable: false,
                        });
                        log_to_file("WebSocket authentication failed, not reconnecting");
                        return;
                    }
                }
            }

            // Attempt reconnection if policy allows
            if reconnection_policy.should_retry() {
                reconnection_policy.increment();
                let delay = reconnection_policy.calculate_delay();
                let _ = status_tx.send(WebSocketStatus::Reconnecting {
                    attempt: reconnection_policy.current_attempt,
                    next_retry_in: delay,
                });
                log_to_file(&format!(
                    "WebSocket reconnecting (attempt {}/{}) in {:?}",
                    reconnection_policy.current_attempt,
                    reconnection_policy.max_attempts,
                    delay
                ));
                tokio::time::sleep(delay).await;
            } else {
                // Max retries reached
                let error_msg = format!(
                    "Failed to connect after {} attempts",
                    reconnection_policy.max_attempts
                );
                let _ = status_tx.send(WebSocketStatus::Error {
                    message: error_msg.clone(),
                    recoverable: false,
                });
                *self.status.lock().await = ConnectionStatus::Error(error_msg.clone());
                log_to_file(&error_msg);
                return;
            }
        }
    }

    async fn handle_websocket_messages(
        &self,
        symbol: String,
        write: &mut futures_util::stream::SplitSink<
            tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
            Message
        >,
        read: &mut futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>
        >,
        tx: &mpsc::UnboundedSender<LivePrice>,
        should_stop: &Arc<Mutex<bool>>,
    ) -> ConnectionResult {
        loop {
            if *should_stop.lock().await {
                return ConnectionResult::Disconnected;
            }

            tokio::select! {
                msg = read.next() => {
                    match msg {
                        Some(Ok(Message::Text(text))) => {
                            if let Ok(json) = serde_json::from_str::<Value>(&text) {
                                if json["type"] == "trade" {
                                    if let Some(data) = json["data"].as_array() {
                                        for trade in data {
                                            if let (Some(price), Some(ts)) = (
                                                trade["p"].as_f64(),
                                                trade["t"].as_i64(),
                                            ) {
                                                let volume = trade["v"].as_u64();
                                                let live_price = LivePrice {
                                                    symbol: symbol.clone(),
                                                    price,
                                                    timestamp: ts / 1000,
                                                    volume,
                                                };

                                                if tx.send(live_price).is_err() {
                                                    return ConnectionResult::Disconnected;
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
                            return ConnectionResult::Error(format!("WebSocket error: {}", e));
                        }
                        None => {
                            return ConnectionResult::Disconnected;
                        }
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                }
            }
        }
    }
}

#[derive(Debug)]
enum ConnectionResult {
    Error(String),
    Disconnected,
}

pub async fn start_websocket(
    symbol: String,
    base_price: f64,
    tx: mpsc::UnboundedSender<LivePrice>,
    status_tx: mpsc::UnboundedSender<WebSocketStatus>,
    should_stop: Arc<Mutex<bool>>,
) {
    let api_key = std::env::var("FINNHUB_API_KEY")
        .ok()
        .map(|k| k.trim().trim_matches('"').trim_matches('\'').to_string());

    if api_key.is_none() || api_key.as_ref().map(|k| k.is_empty()).unwrap_or(true) {
        let _ = status_tx.send(WebSocketStatus::Error {
            message: "No API key configured. Set FINNHUB_API_KEY environment variable.".to_string(),
            recoverable: false,
        });
        log_to_file("WebSocket Error: No API key configured");
        return;
    }

    let manager = WebSocketManager::new(api_key);
    manager.start(symbol, base_price, tx, status_tx, should_stop).await;
}