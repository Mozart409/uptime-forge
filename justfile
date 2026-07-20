set unstable
set dotenv-load
default:
    just --choose

# Watch CSS for development (auto-rebuild on changes)
css-watch:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --watch --minify

# Build CSS for production (minified)
css-build:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --minify

# Run backend with auto-restart on Rust source changes
dev:
    cargo watch -c -x run

# Run cargo check
check:
    cargo check

# Run cargo check on all targets
check-all:
    cargo check --all-targets

# Run clippy in pedantic mode
pedantic:
    cargo clippy -- -W clippy::pedantic

# Run the test suite
test *args:
    cargo test {{ args }}

# Run tests with cargo-nextest
nextest *args:
    cargo nextest run --hide-progress-bar --failure-output final {{ args }}

# Build documentation
doc:
    cargo doc --no-deps

# Build documentation and open it in the browser
doc-open:
    cargo doc --no-deps --open

# Run the application once
run *args:
    cargo run -- {{ args }}

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
