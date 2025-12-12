# Multi-stage build for Kagzi
FROM rust:1.92-alpine as builder

# Install build dependencies
RUN apk add --no-cache \
    musl-dev \
    libpq-dev \
    pkgconfig \
    openssl-dev \
    openssl-libs-static \
    protobuf \
    protoc \
    protobuf-dev

WORKDIR /app

# Copy source code
COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY examples/ ./examples/
COPY proto/ ./proto/
COPY migrations/ ./migrations/

# Build all binaries
RUN cargo build --release

# Final stage
FROM alpine:3.19

# Install runtime dependencies
RUN apk add --no-cache \
    ca-certificates \
    libpq \
    netcat-openbsd

# Create a non-root user
RUN addgroup -g 1001 kagzi && \
    adduser -D -s /bin/sh -u 1001 -G kagzi kagzi

WORKDIR /app

# Copy binaries from builder stage
COPY --from=builder /app/target/release/kagzi-server /usr/local/bin/kagzi-server
COPY --from=builder /app/target/release/kagzi /usr/local/bin/kagzi

# Change ownership to non-root user
RUN chown kagzi:kagzi /usr/local/bin/kagzi-server /usr/local/bin/kagzi

# Switch to non-root user
USER kagzi

# Expose the gRPC port
EXPOSE 50051

# Health check (basic socket check)
HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD nc -z localhost 50051 || exit 1

# Set environment variables
ENV KAGZI_SERVER_HOST=0.0.0.0
ENV KAGZI_SERVER_PORT=50051

# Default to running the server
ENTRYPOINT ["kagzi-server"]