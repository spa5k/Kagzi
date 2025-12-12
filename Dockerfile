FROM rust:1.92-alpine as builder

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

COPY Cargo.toml Cargo.lock ./
COPY crates/ ./crates/
COPY examples/ ./examples/
COPY proto/ ./proto/
COPY migrations/ ./migrations/

RUN cargo build --release

FROM alpine:3.19

RUN apk add --no-cache \
    ca-certificates \
    libpq \
    netcat-openbsd

RUN addgroup -g 1001 kagzi && \
    adduser -D -s /bin/sh -u 1001 -G kagzi kagzi

WORKDIR /app

COPY --from=builder /app/target/release/kagzi-server /usr/local/bin/kagzi-server
COPY --from=builder /app/target/release/kagzi /usr/local/bin/kagzi

RUN chown kagzi:kagzi /usr/local/bin/kagzi-server /usr/local/bin/kagzi

USER kagzi

EXPOSE 50051

HEALTHCHECK --interval=30s --timeout=10s --start-period=5s --retries=3 \
    CMD nc -z localhost 50051 || exit 1

ENV KAGZI_SERVER_HOST=0.0.0.0
ENV KAGZI_SERVER_PORT=50051

ENTRYPOINT ["kagzi-server"]