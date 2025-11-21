.PHONY: help db-up db-down db-reset build test example-simple example-welcome clean

help:
	@echo "Kagzi Development Commands:"
	@echo "  make db-up           - Start PostgreSQL with Docker Compose"
	@echo "  make db-down         - Stop PostgreSQL"
	@echo "  make db-reset        - Reset database (stop, remove volumes, start)"
	@echo "  make build           - Build all crates"
	@echo "  make test            - Run tests"
	@echo "  make example-simple  - Run simple example"
	@echo "  make example-welcome - Run welcome email example (worker)"
	@echo "  make clean           - Clean build artifacts"

db-up:
	@echo "🚀 Starting PostgreSQL..."
	docker-compose up -d
	@echo "✅ PostgreSQL is running on port 5432"

db-down:
	@echo "🛑 Stopping PostgreSQL..."
	docker-compose down

db-reset:
	@echo "🔄 Resetting database..."
	docker-compose down -v
	docker-compose up -d
	@echo "✅ Database reset complete"

build:
	@echo "🔨 Building Kagzi..."
	cargo build --all

test:
	@echo "🧪 Running tests..."
	cargo test --all

example-simple: db-up
	@echo "🎯 Running simple example..."
	@sleep 2
	cargo run --example simple

example-welcome: db-up
	@echo "🎯 Running welcome email example (worker mode)..."
	@echo "   In another terminal, run: cargo run --example welcome_email -- trigger Alice"
	@sleep 2
	cargo run --example welcome_email -- worker

clean:
	@echo "🧹 Cleaning build artifacts..."
	cargo clean
