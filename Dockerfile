# Multi-stage build for a small runtime image
FROM rust:1.83-bullseye as builder

WORKDIR /app

# Create a new empty project so dependencies can cache
RUN USER=root cargo new --bin zabbixbot
WORKDIR /app/zabbixbot

# Copy manifests and fetch dependencies first
COPY Cargo.toml Cargo.toml
# Create dummy src to allow cargo to resolve dependencies
RUN mkdir -p src && echo "fn main(){}" > src/main.rs
RUN cargo fetch

# Now copy the real source
COPY src src

# Build with release profile
RUN cargo build --release

# Runtime image
FROM debian:bullseye-slim

# Minimal runtime deps (ca-certificates for TLS, procps for healthcheck)
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates procps && rm -rf /var/lib/apt/lists/*

# Non-root user
RUN useradd -m -u 10001 appuser

WORKDIR /app

# Copy binary
COPY --from=builder /app/zabbixbot/target/release/zabbixbot /usr/local/bin/zabbixbot

# Default path for allowed users
ENV ALLOWED_USERS_PATH=/bot/allowed_users.txt

USER appuser

# Healthcheck: ensure the process stays up (no endpoint)
HEALTHCHECK --interval=30s --timeout=5s --retries=3 CMD pgrep zabbixbot || exit 1

CMD ["/usr/local/bin/zabbixbot"]
