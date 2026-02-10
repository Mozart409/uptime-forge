default:
    just --choose

# Watch CSS for development (auto-rebuild on changes)
css-watch:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --watch

# Build CSS for production (minified)
css-build:
    tailwindcss -i src/public/css/input.css -o src/public/css/output.css --minify

# Build and start Docker container in detached mode
docker-up: css-build
    docker compose up -d --build

# Stop and remove Docker container
docker-down:
    docker compose down
