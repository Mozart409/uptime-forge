# AGENTS.md - Coding Agent Guidelines for uptime-forge

**Last analyzed:** 2026-02-10 | **Git hash:** `9058b3033f59631f0ce9d1491bb85e67158ce5b1`

## Project Overview

Uptime monitoring service built with Rust. Uses axum web framework, maud templating, htmx for interactivity, and TOML configuration.

## Documentation Resources

When you need documentation for libraries or APIs:
- **Use Context7 MCP** to query up-to-date documentation for any dependency
- **Use grep MCP** to search real-world code examples from GitHub repositories
- Prefer these tools over training data for current API usage patterns

## Build & Development Commands

### Nix Development Environment (Recommended)

```bash
nix develop                  # Enter dev shell with all tools
```

### Just Commands (Primary Interface)

```bash
just                         # Show available commands
just css-watch               # Watch CSS for development
just css-build               # Build minified CSS for production
just docker-up               # Build CSS and start Docker container
just docker-down             # Stop Docker container
```

### Bacon (Continuous Build - Preferred for Development)

```bash
bacon                        # Default: continuous type checking
bacon run-long               # Run server with auto-restart on changes
bacon clippy                 # Continuous linting
bacon clippy-all             # Lint all targets including tests
bacon pedantic               # Pedantic clippy lints
bacon test                   # Run tests continuously
bacon nextest                # Run tests with cargo-nextest
bacon doc-open               # Generate and open docs
```

### Standard Cargo Commands

```bash
cargo build                  # Build debug
cargo build --release        # Build release
cargo run                    # Run server
cargo check                  # Quick compilation check
cargo fmt                    # Format code
cargo clippy                 # Lint
cargo doc --open             # Generate and view docs
```

### Testing

```bash
cargo test                           # Run all tests
cargo test <test_name>               # Run single test by name
cargo test <module>::               # Run tests in module
cargo test -- --nocapture            # Show println! output
cargo nextest run                    # Run with nextest (better output)
cargo nextest run -E 'test(name)'    # Single test with nextest
```

### Tailwind CSS (Styling)

```bash
just css-watch               # Watch mode - auto-rebuild on changes (development)
just css-build               # Build minified CSS (production)
```

**Note:** If the UI layout looks broken or unstyled, run `just css-watch` or `just css-build` to generate the Tailwind CSS output.

### Docker

```bash
just docker-up               # Build CSS and start container (detached)
just docker-down             # Stop and remove container
docker compose logs -f       # View logs
```

### Additional Tools (Available in Nix Shell)

```bash
cargo deny check             # Audit dependencies
cocogitto check              # Validate commit messages
lefthook install             # Install git hooks
```

## Git Hooks (Lefthook)

Pre-commit hooks are configured in `lefthook.yml`:
- **format** - Runs `cargo fmt` (auto-stages fixed files)
- **lint** - Runs `cargo clippy` with warnings as errors
- **pedantic** - Runs `cargo clippy` with pedantic lints
- **test** - Runs `cargo test`
- **build-css** - Rebuilds Tailwind CSS (auto-stages output)

Install hooks with: `lefthook install`

Pre-push hooks:
- **check** - Runs `cargo fmt --all --check` to verify formatting

## Project Structure

```
uptime-forge/
├── src/
│   ├── main.rs              # Entry point, routing, middleware setup
│   ├── config.rs            # Configuration structs and loading
│   ├── checker.rs           # Endpoint health checking and background tasks
│   ├── layout.rs            # Maud HTML templates (dashboard, cards)
│   ├── db.rs                # Database connection and utilities
│   └── public/              # Static assets (css/, js/, favicon)
├── Cargo.toml               # Dependencies and project config
├── forge.toml               # Runtime configuration
├── Dockerfile               # Multi-stage Docker build
├── compose.yml              # Docker Compose configuration
├── justfile                 # Build commands
├── lefthook.yml             # Git hooks configuration
└── bacon.toml               # Bacon build tool config
```

## Code Style Guidelines

### Formatting

- Use `cargo fmt` (rustfmt defaults)
- No custom rustfmt.toml - standard Rust formatting applies
- Run `cargo fmt` before committing

### Import Organization

Order imports in this sequence, separated by blank lines:

```rust
// 1. Module declarations
mod config;
mod layout;

// 2. Standard library (if any)
use std::collections::HashMap;

// 3. External crates (grouped by crate, alphabetically)
use axum::{Router, response::Html, routing::get};
use color_eyre::eyre::{Context, Result};
use serde::Deserialize;
use tokio::net::TcpListener;

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
| Type parameters | Single uppercase or descriptive | `T`, `S`, `State` |

### Error Handling

Use `color_eyre` for error handling:

```rust
use color_eyre::eyre::{Context, Result};

// Propagate with context
fn load_file(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .wrap_err_with(|| format!("failed to read file: {}", path.display()))
}

// In main
fn main() -> Result<()> {
    color_eyre::install()?;
    // ...
}
```

**Guidelines:**
- Use `Result` type alias from `color_eyre::eyre`
- Add context with `.wrap_err()` or `.wrap_err_with()`
- Context should explain what operation failed, not repeat the error
- Use `?` for propagation

### Async Patterns

```rust
// Use tokio runtime via macro
#[tokio::main]
async fn main() -> Result<()> { ... }

// Async handlers return impl IntoResponse or concrete types
async fn handler() -> Html<String> { ... }
async fn health() -> &'static str { "ok" }
```

### Configuration

- Use serde with `#[derive(Deserialize)]`
- Provide defaults via `#[serde(default)]` or `#[serde(default = "fn_name")]`
- Default functions should be `const fn` when possible

```rust
#[derive(Debug, Deserialize)]
pub struct Endpoint {
    pub addr: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default = "default_interval")]
    pub interval: u64,
}

const fn default_interval() -> u64 { 60 }
```

### Maud Templates

```rust
use maud::{html, Markup, DOCTYPE};

pub fn base(title: &str, content: Markup) -> Markup {
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { (title) }
            }
            body { (content) }
        }
    }
}
```

### Logging/Tracing

```rust
use tracing::{info, warn, error, debug, instrument};

// Use structured logging
tracing::info!(endpoint = %name, "checking endpoint");

// Use #[instrument] for function tracing
#[instrument(skip(config))]
async fn check_endpoint(config: &Config) -> Result<()> { ... }
```

**Log levels:**
- `error!` - Failures requiring attention
- `warn!` - Unexpected but handled situations  
- `info!` - Significant events (startup, config loaded)
- `debug!` - Detailed debugging info
- `trace!` - Very verbose, rarely used

### Comments

- Use `///` for public API documentation
- Use `//` for inline implementation notes
- Explain "why", not "what"
- No commented-out code in commits

## Dependencies Quick Reference

| Crate | Purpose |
|-------|---------|
| axum | Web framework |
| tokio | Async runtime |
| tower / tower-http | Middleware |
| maud | HTML templating |
| serde / toml | Configuration |
| color-eyre | Error handling |
| tracing | Structured logging |
| reqwest | HTTP client for endpoint checks |
| sqlx | PostgreSQL database driver |
| futures | Async utilities |
| tokio-util | Cancellation tokens for background tasks |
| dotenvy | Environment variable loading |

## Commit Messages

Follow conventional commits (enforced by cocogitto):

```
feat: add endpoint health checking
fix: correct timeout calculation
docs: update API documentation
refactor: extract config validation
test: add integration tests for monitoring
```
