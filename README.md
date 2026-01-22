# Charty

> **Note:** This project is a work in progress. Features may be incomplete or change.

A terminal-based stock market charting and analysis application built in Rust. View historical price charts, stream real-time market data, and analyze stocks directly from your terminal.

## Features

- **Browse Popular Stocks** - Quick access to major indexes (S&P 500, Dow Jones) and tech stocks (AAPL, MSFT, GOOGL, AMZN, TSLA, NVDA, META)
- **Historical Charts** - Interactive candlestick charts with multiple timeframes (1 Day, 1 Week, 1 Month, 3 Months, 1 Year)
- **Live Ticker Mode** - Stream real-time price ticks as they occur
- **Live Candles Mode** - Aggregate real-time trades into 1-minute candlestick charts
- **Stock Search** - Search for any stock symbol

## Prerequisites

- Rust toolchain ([rustup.rs](https://rustup.rs/))
- Finnhub API key ([finnhub.io](https://finnhub.io/))

## Installation

```bash
git clone <repository-url>
cd charty
cargo build --release
```

## Configuration

Create a `.env` file in the project root:

```
FINNHUB_API_KEY=your_api_key_here
```

Or export the environment variable:

```bash
export FINNHUB_API_KEY=your_api_key_here
```

## Usage

```bash
cargo run --release
```

### Keyboard Controls

**Landing Page:**
| Key | Action |
|-----|--------|
| `↑/↓` | Navigate stock list |
| `Enter` | Select stock |
| `s` | Search for a symbol |
| `q` | Quit |

**Chart View:**
| Key | Action |
|-----|--------|
| `←/→` | Change timeframe |
| `l` | Enter live mode |
| `r` | Refresh data |
| `s` | Search |
| `b` | Back to landing |
| `e` | Toggle error log |
| `q` | Quit |

**Live Mode:**
| Key | Action |
|-----|--------|
| `1` | Switch to Live Ticker |
| `2` | Switch to Live Candles |
| `h` | Back to historical chart |
| `e` | Toggle error log |
| `q` | Quit |

## Project Structure

```
charty/
├── src/
│   ├── main.rs        # Entry point and event loop
│   ├── stock.rs       # Stock data fetching
│   ├── ui.rs          # UI rendering and state
│   └── websocket.rs   # Real-time data streaming
├── Cargo.toml
└── .env
```

## Releases

Pre-built binaries are available on the [Releases](../../releases) page for:
- Linux (x86_64)
- macOS (x86_64, Apple Silicon)
- Windows (x86_64)

### Creating a Release

Releases are automated via GitHub Actions. To create a new release:

```bash
# Tag the commit with a version
git tag v1.0.0

# Push the tag to trigger the release workflow
git push origin v1.0.0
```

The workflow will automatically:
1. Build binaries for all supported platforms
2. Create a GitHub release with auto-generated release notes
3. Attach the compiled binaries

## License

MIT
