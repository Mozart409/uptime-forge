default:
    just --choose

# Watch CSS for development (auto-rebuild on changes)
css-watch:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --watch

# Build CSS for production (minified)
css-build:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --minify

# Run backend with auto-restart on Rust source changes
backend:
    cargo watch -c -x run

# Development mode: run CSS watch and backend in parallel
dev:
    #!/usr/bin/env bash
    trap 'kill 0' EXIT
    just css-watch &
    just backend &
    wait

# Run cargo check
check:
    cargo check

# Run cargo check on all targets
check-all:
    cargo check --all-targets

# Run clippy on the default target
clippy:
    cargo clippy

# Run clippy on all targets
clippy-all:
    cargo clippy --all-targets

# Run clippy in pedantic mode
pedantic:
    cargo clippy -- -W clippy::pedantic

# Run the test suite
test *args:
    cargo test {{args}}

# Run tests with cargo-nextest
nextest *args:
    cargo nextest run --hide-progress-bar --failure-output final {{args}}

# Build documentation
doc:
    cargo doc --no-deps

# Build documentation and open it in the browser
doc-open:
    cargo doc --no-deps --open

# Run the application once
run *args:
    cargo run -- {{args}}

# Run the application without auto-restart (useful for long-running programs)
run-long *args:
    cargo run -- {{args}}

# Run a specific example
ex name *args:
    cargo run --example {{name}} -- {{args}}

# Build and start Podman container in detached mode
prod-up: css-build
    podman-compose -f ./example/compose.yml up -d --build

# Stop and remove Podman container
prod-down:
    podman-compose -f ./example/compose.yml down

dev-up: css-build
    podman-compose -f compose.dev.yml up -d --build --remove-orphans

dev-down:
    podman-compose -f compose.dev.yml down
