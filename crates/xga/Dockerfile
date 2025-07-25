# syntax=docker/dockerfile:1
FROM rust:1.83 AS builder

WORKDIR /app

# Copy workspace files
COPY Cargo.toml Cargo.lock ./
COPY bin bin
COPY crates crates

# Build the reserved gas module
RUN cargo build --release --bin reserved-gas

# Runtime stage
FROM ubuntu:22.04

# Install required runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /app/target/release/reserved-gas /usr/local/bin/reserved-gas

# Create non-root user
RUN useradd -m -u 1000 -s /bin/bash cbuser
USER cbuser

# Expose the default PBS port
EXPOSE 18550

ENTRYPOINT ["/usr/local/bin/reserved-gas"]