# Justfile for Kagzi

set dotenv-load

# Ensure sqlx compile-time macros see our connection URL
export DATABASE_URL := env_var_or_default(
    "KAGZI_DB_URL",
    env_var_or_default("DATABASE_URL", ""),
)
export SQLX_DATABASE_URL := env_var_or_default(
    "KAGZI_DB_URL",
    env_var_or_default("DATABASE_URL", ""),
)

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

# Run pending migrations
migrate:
    sqlx migrate run

# Revert the last migration
migrate-revert:
    sqlx migrate revert

# Create a new migration file
migrate-add name:
    sqlx migrate add {{name}}

# --- Build ---

# Build the entire workspace
build: build-proto
    cargo build

# Build only the proto crate (generates code)
build-proto:
    cargo build -p kagzi-proto

# --- Development ---

# Run the gRPC server
dev: build-proto
    cargo run -p kagzi-server

# Launch gRPCui (requires grpcui to be installed: go install github.com/fullstorydev/grpcui/cmd/grpcui@latest)
grpcui:
    grpcui -plaintext localhost:50051

# Setup the environment from scratch
setup: db-reset build
    echo "Setup complete!"

# --- Examples ---

# Run a single examples crate binary with optional args (default variant if empty).
example name args="":
    cargo run -p examples --example {{name}} -- {{args}}

# Run all examples sequentially with defaults (requires server running).
# Excludes `worker_hub` because it runs indefinitely.
examples-all server="http://localhost:50051":
    bash scripts/run_examples.sh {{server}}

lint: build-proto
    cargo clippy --all-targets --all-features -- -D warnings

# --- Tests ---

# Run all tests
test:
    cargo test --all

# Run unit tests only (fast, no Docker required)
test-unit:
    cargo test --lib --all

# Run integration tests (requires Docker) with shorter poll timeout
test-integration:
    KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests -- --test-threads=1

# Run integration tests with output
test-integration-verbose:
    KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests -- --test-threads=1 --nocapture

# Run a specific integration test
test-one name:
    KAGZI_POLL_TIMEOUT_SECS=2 cargo test -p kagzi-server --test integration_tests {{name}} -- --test-threads=1 --nocapture

tidy:
    cargo fmt --all
    buf format -w

fix:
    cargo fix --all-targets --all-features --allow-dirty