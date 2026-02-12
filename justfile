default:
    just --choose

# Watch CSS for development (auto-rebuild on changes)
css-watch:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --watch

# Build CSS for production (minified)
css-build:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --minify

# Run backend with bacon (auto-restart on changes)
backend:
    bacon run-long

# Development mode: run CSS watch and backend in parallel
dev:
    #!/usr/bin/env bash
    trap 'kill 0' EXIT
    just css-watch &
    just backend &
    wait

# Build and start Docker container in detached mode
prod-up: css-build
    docker compose -f ./example/compose.yml up -d --build

# Stop and remove Docker container
prod-down:
    docker compose -f ./example/compose.yml down

dev-up: css-build
    COMPOSE_BAKE=true docker compose -f compose.dev.yml up -d --build --remove-orphans

dev-down: 
    docker compose -f compose.dev.yml down
