# AGENTS.md - Coding Agent Guidelines for uptime-forge

**Last updated:** 2026-02-11 | **Git hash:** `29189fb9e921e0c2e43e69c2bbd2461a0a0e4a57`

> **Maintenance:** When modifying this file, update the date and git hash above.
> Run `git rev-parse HEAD` to get the current hash after committing changes.

## Project Overview

Uptime monitoring service built with Rust using axum, maud templating, htmx, and TOML config.

## Documentation Resources

- **Use Context7 MCP** to query up-to-date documentation for any dependency
- **Use grep MCP** to search real-world code examples from GitHub repositories

## Build & Development Commands

### Primary Commands (Bacon - Recommended)

```bash
bacon                        # Default: continuous type checking
bacon run-long               # Run server with auto-restart on changes
bacon clippy-all             # Lint all targets
bacon pedantic               # Pedantic clippy lints
bacon test                   # Run tests continuously
bacon nextest                # Run tests with cargo-nextest
```

### Testing Commands

```bash
cargo test                           # Run all tests
cargo test <test_name>               # Run single test by name
cargo test <module>::                # Run tests in module
cargo test -- --nocapture            # Show println! output
cargo nextest run                    # Run with nextest (better output)
cargo nextest run -E 'test(name)'    # Single test with nextest
```

### Just Commands

```bash
just css-watch               # Watch CSS for development
just css-build               # Build minified CSS for production
just docker-up               # Build CSS and start Docker container
just docker-down             # Stop Docker container
```

### Standard Cargo

```bash
cargo build                  # Build debug
cargo run                    # Run server
cargo fmt                    # Format code
cargo clippy                 # Lint
```

**CSS Note:** If UI looks broken, run `just css-watch` or `just css-build`.

## Project Structure

```
src/
├── main.rs        # Entry point, routing, middleware
├── config.rs      # Configuration structs and loading
├── checker.rs     # Endpoint health checking, background tasks
├── layout.rs      # Maud HTML templates
├── db.rs          # Database connection
└── public/        # Static assets (css/, js/)
```

## Configuration (forge.toml)

```toml
[server]
addr = "0.0.0.0:3003"
# reload_config_interval = 60  # Seconds (default 60, 0 to disable)

[endpoints.example]
addr = "https://example.com"
description = "Example Site"
# interval = 60             # Check interval seconds (default)
# timeout = 10              # Request timeout seconds (default)
# expected_status = 200     # Expected HTTP status (default)
# skip_tls_verification = false  # Skip TLS cert verification (default)
```

## Code Style Guidelines

### Import Organization

```rust
// 1. Module declarations
mod config;
mod layout;

// 2. Standard library
use std::collections::HashMap;

// 3. External crates (alphabetically by crate)
use axum::{Router, routing::get};
use color_eyre::eyre::{Context, Result};
use serde::Deserialize;

// 4. Internal crate imports
use crate::config::Config;
```

### Naming Conventions

| Item | Convention | Example |
|------|------------|---------|
| Functions | snake_case | `init_tracing`, `load_config` |
| Types/Structs | PascalCase | `Config`, `Endpoint` |
| Constants | SCREAMING_SNAKE_CASE | `DEFAULT_TIMEOUT` |
| Modules | snake_case | `config`, `layout` |

### Error Handling (color_eyre)

```rust
use color_eyre::eyre::{Context, Result};

fn load_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .wrap_err_with(|| format!("failed to read file: {}", path.display()))
}
```

- Use `Result` from `color_eyre::eyre`
- Add context with `.wrap_err()` or `.wrap_err_with()`
- Context explains what operation failed, not the error itself

### Configuration Structs

```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Endpoint {
    pub addr: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

const fn default_interval() -> u64 { 60 }
```

### Logging (tracing)

```rust
use tracing::{info, warn, error, debug, instrument};

tracing::info!(endpoint = %name, "checking endpoint");

#[instrument(skip(config))]
async fn check_endpoint(config: &Config) -> Result<()> { ... }
```

### Maud Templates

```rust
use maud::{html, Markup, DOCTYPE};

pub fn page(title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head { title { (title) } }
            body { (content) }
        }
    }
}
```

## Git Hooks (Lefthook)

Pre-commit runs: `cargo fmt`, `cargo clippy -D warnings`, pedantic clippy, `cargo test`, CSS build.

Install: `lefthook install`

## Commit Messages (Conventional Commits)

```
feat: add endpoint health checking
fix: correct timeout calculation
docs: update API documentation
refactor: extract config validation
test: add integration tests
```

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| axum | Web framework |
| tokio | Async runtime |
| maud | HTML templating |
| color-eyre | Error handling |
| tracing | Structured logging |
| reqwest | HTTP client |
| sqlx | PostgreSQL driver |
