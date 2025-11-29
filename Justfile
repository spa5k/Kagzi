# Justfile for Kagzi

set dotenv-load

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
build:
    cargo build

# Build only the proto crate (generates code)
build-proto:
    cargo build -p kagzi-proto

# --- Development ---

# Setup the environment from scratch
setup: db-reset build
    echo "Setup complete!"
