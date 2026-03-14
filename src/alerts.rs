use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceAlert {
    pub symbol: String,
    pub target: f64,
    pub above: bool,     // true = alert when price >= target
    pub triggered: bool,
}

fn alerts_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("charty").join("alerts.json"))
}

pub fn load() -> Vec<PriceAlert> {
    let path = match alerts_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(alerts: &[PriceAlert]) {
    let path = match alerts_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(alerts) {
        let _ = std::fs::write(path, json);
    }
}
