# Stage 1: Build CSS with standalone Tailwind CLI (no npm required)
FROM debian:bookworm-slim AS css-builder

WORKDIR /app

# Download standalone Tailwind CSS v4 binary
RUN apt-get update && apt-get install -y --no-install-recommends curl ca-certificates && \
    curl -sLO https://github.com/tailwindlabs/tailwindcss/releases/download/v4.1.7/tailwindcss-linux-x64 && \
    chmod +x tailwindcss-linux-x64 && \
    rm -rf /var/lib/apt/lists/*

# Copy CSS source
COPY src/public/css/input.css src/public/css/input.css

# Copy source files for Tailwind to scan for classes
COPY src/*.rs src/

# Build minified CSS
RUN ./tailwindcss-linux-x64 -i src/public/css/input.css -o src/public/css/output.css --minify


# Stage 2: Build the Rust application
FROM rust:1.92-alpine AS builder

WORKDIR /app

# Install build dependencies (git needed for build.rs)
RUN apk add --no-cache musl-dev git

# Copy manifests and build.rs first for dependency caching
COPY Cargo.toml Cargo.lock build.rs ./

# Create dummy src to build dependencies
RUN mkdir src && \
    echo 'fn main() { println!("dummy"); }' > src/main.rs

# Build dependencies only (this layer is cached)
RUN cargo build --release && \
    rm -rf src target/release/deps/uptime_forge* target/release/.fingerprint/uptime_forge*

# Copy actual source code
COPY src ./src
COPY migrations ./migrations

# Build the application
RUN cargo build --release --locked


# Stage 3: Runtime image
FROM alpine:3.21 AS runtime

WORKDIR /app

# Install CA certificates for HTTPS requests
RUN apk add --no-cache ca-certificates

# Create non-root user
RUN addgroup -S appgroup && adduser -S appuser -G appgroup

# Copy the binary
COPY --from=builder /app/target/release/uptime-forge /app/uptime-forge

# Copy static assets
COPY src/public/js /app/src/public/js
COPY src/public/favicon.svg /app/src/public/favicon.svg

# Copy built CSS from css-builder stage
COPY --from=css-builder /app/src/public/css/output.css /app/src/public/css/output.css

# Copy default config (can be overridden with volume mount)
COPY forge.toml /app/forge.toml

# Set ownership
RUN chown -R appuser:appgroup /app

# Switch to non-root user
USER appuser

# Expose port
EXPOSE 3000

# Health check
HEALTHCHECK --interval=30s --timeout=3s --start-period=5s --retries=3 \
    CMD wget --no-verbose --tries=1 --spider http://localhost:3000/health || exit 1

# Run the application
CMD ["./uptime-forge"]
