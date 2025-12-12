# Justfile for Kagzi

set dotenv-load

# Ensure sqlx compile-time macros see our connection URL (hardcoded local default)
export DATABASE_URL := "postgres://postgres:postgres@localhost:54122/postgres"
export SQLX_DATABASE_URL := "postgres://postgres:postgres@localhost:54122/postgres"

# --- Database ---

# Start the database container
db-up:
    docker-compose up -d

# Stop the database container
db-down:
    docker-compose down

# Reset the database (destructive!)
db-reset: db-down
    docker-compose down -v
    docker-compose up -d
    sleep 2
    just migrate

# --- Migrations ---

# Prepare SQLx offline data (requires reachable DATABASE_URL)
sqlx-prepare:
    DATABASE_URL={{DATABASE_URL}} cargo sqlx prepare --workspace -- --all-targets

# Run pending migrations
migrate:
    DATABASE_URL={{DATABASE_URL}} sqlx migrate run

# Revert the last migration
migrate-revert:
    DATABASE_URL={{DATABASE_URL}} sqlx migrate revert

# Create a new migration file
migrate-add name:
    DATABASE_URL={{DATABASE_URL}} sqlx migrate add {{name}}

# --- Build ---

# Build the entire workspace
build: build-proto
    DATABASE_URL={{DATABASE_URL}} cargo build

# Build only the proto crate (generates code)
build-proto:
    DATABASE_URL={{DATABASE_URL}} cargo build -p kagzi-proto

# --- Development ---

# Run the gRPC server
dev: build-proto
    DATABASE_URL={{DATABASE_URL}} cargo run -p kagzi-server

# Launch gRPCui (requires grpcui to be installed: go install github.com/fullstorydev/grpcui/cmd/grpcui@latest)
grpcui:
    grpcui -plaintext localhost:50051

# Setup the environment from scratch
setup: db-reset build
    echo "Setup complete!"

# --- Examples ---

# Run a single examples crate binary with optional args (default variant if empty).
example name args="":
    DATABASE_URL={{DATABASE_URL}} cargo run -p examples --example {{name}} -- {{args}}

# Run all examples sequentially with defaults (requires server running).
# Excludes `worker_hub` because it runs indefinitely.
examples-all server="http://localhost:50051":
    DATABASE_URL={{DATABASE_URL}} bash scripts/run_examples.sh {{server}}

lint: build-proto
    DATABASE_URL={{DATABASE_URL}} cargo clippy --all-targets --all-features -- -D warnings

# --- Tests ---

# Run all tests
test:
    DATABASE_URL={{DATABASE_URL}} cargo test --all

# Run unit tests only (fast, no Docker required)
test-unit:
    DATABASE_URL={{DATABASE_URL}} cargo test --lib --all

# Run integration tests (requires Docker) with shorter poll timeout
test-integration:
    DATABASE_URL={{DATABASE_URL}} KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests -- --test-threads=1

# Run integration tests with output
test-integration-verbose:
    DATABASE_URL={{DATABASE_URL}} KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests -- --test-threads=1 --nocapture

# Run a specific integration test
test-one name:
    DATABASE_URL={{DATABASE_URL}} KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests {{name}} -- --test-threads=1 --nocapture

# Run new end-to-end tests in tests crate (requires Docker for testcontainers)
test-e2e:
    DATABASE_URL={{DATABASE_URL}} cargo test -p tests -- --test-threads=1

tidy:
    cargo fmt --all
    buf format -w
    dprint fmt

fix:
    cargo fix --all-targets --all-features --allow-dirty