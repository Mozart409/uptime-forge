# AGENTS.md - Coding Agent Guidelines for uptime-forge

**Last updated:** 2026-02-12 | **Git hash:** `848268f608e8178e11b5101a4d807d6a4f924150`

> **Maintenance:** When modifying this file, update the date and git hash above.
> Run `git rev-parse HEAD` to get the current hash after committing changes.

## Project Overview

Uptime monitoring service built with Rust using axum, maud templating, htmx, and TOML config.

## Documentation Resources

- **Use Context7 MCP** to query up-to-date documentation for any dependency
- **Use grep MCP** to search real-world code examples from GitHub repositories

## Build & Development Commands

### Nix Development Environment

This project uses Nix flakes for reproducible development environments.

#### Automatic Environment (Recommended)

Using **direnv**, the environment activates automatically when you `cd` into the project:

```bash
# 1. Install direnv (if not installed)
nix-env -iA nixpkgs.direnv

# 2. Hook direnv into your shell (add to ~/.zshrc or ~/.bashrc)
eval "$(direnv hook zsh)"  # or: eval "$(direnv hook bash)"

# 3. Allow the .envrc file (one-time, in project directory)
direnv allow

# Now the environment loads automatically when you cd into the directory!
```

#### Manual Environment Activation

Alternatively, enter the environment manually:

```bash
# Enter the development shell
nix develop

# Or run a single command without entering the shell
nix develop --command <command>

# Examples:
nix develop --command cargo build
nix develop --command bacon
```

### Primary Commands (Bacon - Recommended)

Run these inside `nix develop` or prefix with `nix develop --command`:

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

example/           # Ready-to-use deployment files
├── compose.yml    # Docker Compose for production
├── forge.toml     # Example endpoint configuration
└── postgres/      # PostgreSQL + TimescaleDB config
    ├── postgresql.conf
    └── initdb/001-timescaledb.sql
```

## Configuration (forge.toml)

```toml
[server]
addr = "0.0.0.0:3003"
# reload_config_interval = 60  # Seconds (default 60, 0 to disable)

[endpoints.example]
addr = "https://example.com"
description = "Example Site"
# type = "http"             # Check type: "http" (default), "tcp", "dns"
# group = "backend"         # Optional group for organizing endpoints
# tags = ["production"]     # Optional tags for filtering
# interval = 60             # Check interval seconds (default, warn if < 10)
# timeout = 10              # Request timeout seconds (default, must be < interval)
# expected_status = 200     # Expected HTTP status (default)
# skip_tls_verification = false  # Skip TLS cert verification (default)
# method = "GET"            # HTTP method: GET, POST, PUT, etc. (default: GET)
# headers = { Authorization = "Bearer ${API_TOKEN}" }  # Custom headers (env var support)
# body = '{"check": "deep"}'  # Request body for POST/PUT
# retries = 0               # Retry count before marking failed (default: 0)
# retry_delay = 5           # Delay between retries in seconds (default: 5)
# alert_after_failures = 3  # Alert after N consecutive failures (default: 3)
# alert_channels = ["webhook"]  # Alert channels to notify

# TCP check example
[endpoints.database]
addr = "tcp://db.example.com:5432"
type = "tcp"

# DNS check example
[endpoints.dns-check]
addr = "dns://example.com"
type = "dns"
expected_records = ["1.2.3.4"]  # Optional: verify specific DNS records
```

### Environment Variable Substitution

Config values support `${VAR_NAME}` syntax for environment variables:

```toml
[endpoints.api]
addr = "https://api.example.com/health"
headers = { Authorization = "Bearer ${API_TOKEN}" }
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
