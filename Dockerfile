# Stage 1: Chef - prepare recipe for dependency caching
FROM rust:1.92-alpine AS chef

RUN apk add --no-cache musl-dev && \
    cargo install cargo-chef --locked

WORKDIR /app

# Stage 2: Planner - analyze dependencies
FROM chef AS planner

COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY build.rs ./

RUN cargo chef prepare --recipe-path recipe.json

# Stage 3: Builder - build dependencies then application
FROM chef AS builder

# Install build dependencies (git needed for build.rs)
RUN apk add --no-cache git

# Copy recipe and build dependencies (cached layer)
COPY --from=planner /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

# Copy source and build application
COPY Cargo.toml Cargo.lock build.rs ./
COPY src ./src
COPY migrations ./migrations

RUN cargo build --release --locked

# Stage 4: Runtime image
FROM alpine:3.21 AS runtime

# OCI Image Labels
LABEL org.opencontainers.image.title="uptime-forge"
LABEL org.opencontainers.image.description="Uptime monitoring service built with Rust"
LABEL org.opencontainers.image.url="https://github.com/mozart409/uptime-forge"
LABEL org.opencontainers.image.source="https://github.com/mozart409/uptime-forge"
LABEL org.opencontainers.image.licenses="MIT"
LABEL org.opencontainers.image.vendor="mozart409"

WORKDIR /app

# Install CA certificates for HTTPS requests
RUN apk add --no-cache ca-certificates

# Create non-root user
RUN addgroup -S appgroup && adduser -S appuser -G appgroup

# Copy the binary
COPY --from=builder /app/target/release/uptime-forge /app/uptime-forge

# Copy static assets (CSS is pre-built and committed to repo)
COPY src/public /app/src/public

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
