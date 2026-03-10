use std::path::PathBuf;

fn watchlist_path() -> Option<PathBuf> {
    dirs::config_dir().map(|p| p.join("charty").join("watchlist.json"))
}

pub fn load() -> Vec<String> {
    let path = match watchlist_path() {
        Some(p) => p,
        None => return Vec::new(),
    };
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(symbols: &[String]) {
    let path = match watchlist_path() {
        Some(p) => p,
        None => return,
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string(symbols) {
        let _ = std::fs::write(path, json);
    }
}
