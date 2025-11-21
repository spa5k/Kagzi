SHELL := /bin/bash

WORKFLOW_TIMEOUT ?= 90
WELCOME_EMAIL_TRIGGER_NAME ?= Alice

.PHONY: help db-up db-down db-reset build test example-simple example-batch-processing example-data-pipeline example-order-fulfillment example-retry-pattern example-scheduled-reminder example-welcome examples clean

help:
	@echo "Kagzi Development Commands:"
	@echo "  make db-up                    - Start PostgreSQL with Docker Compose"
	@echo "  make db-down                  - Stop PostgreSQL"
	@echo "  make db-reset                 - Reset database (stop, remove volumes, start)"
	@echo "  make build                    - Build all crates"
	@echo "  make test                     - Run tests"
	@echo "  make example-simple           - Run simple example"
	@echo "  make example-batch-processing - Run batch processing example"
	@echo "  make example-data-pipeline    - Run data pipeline example"
	@echo "  make example-order-fulfillment - Run order fulfillment example"
	@echo "  make example-retry-pattern    - Run retry pattern example"
	@echo "  make example-scheduled-reminder - Run scheduled reminder example"
	@echo "  make example-welcome          - Run welcome email example (worker + trigger)"
	@echo "  make examples                 - Run all examples sequentially"
	@echo "  make clean                    - Clean build artifacts"

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

example-batch-processing: db-up
	@echo "🎯 Running batch processing example..."
	@sleep 2
	cargo run --example batch_processing

example-data-pipeline: db-up
	@echo "🎯 Running data pipeline example..."
	@sleep 2
	cargo run --example data_pipeline

example-order-fulfillment: db-up
	@echo "🎯 Running order fulfillment example..."
	@sleep 2
	cargo run --example order_fulfillment

example-retry-pattern: db-up
	@echo "🎯 Running retry pattern example..."
	@sleep 2
	cargo run --example retry_pattern

example-scheduled-reminder: db-up
	@echo "🎯 Running scheduled reminder example..."
	@sleep 2
	cargo run --example scheduled_reminder

define RUN_WORKFLOW_WITH_TRIGGER
	@echo "🎯 Running $(1) example (worker + trigger)..."
	@set -euo pipefail; \
		LOG_FILE=$$(mktemp -t kagzi_$(1)_worker.XXXX); \
		LOG_RETENTION="delete"; \
		cargo run --example $(1) -- worker > "$$LOG_FILE" 2>&1 & \
		WORKER_PID=$$!; \
		tail -n +1 -f "$$LOG_FILE" & \
		TAIL_PID=$$!; \
		cleanup() { \
			if kill -0 $$TAIL_PID >/dev/null 2>&1; then \
				kill $$TAIL_PID >/dev/null 2>&1 || true; \
				wait $$TAIL_PID 2>/dev/null || true; \
			fi; \
			if kill -0 $$WORKER_PID >/dev/null 2>&1; then \
				kill $$WORKER_PID >/dev/null 2>&1 || true; \
				wait $$WORKER_PID 2>/dev/null || true; \
			fi; \
			if [ "$$LOG_RETENTION" = "delete" ]; then \
				rm -f "$$LOG_FILE"; \
			else \
				echo "ℹ️ Worker logs preserved at $$LOG_FILE"; \
			fi; \
		}; \
		trap cleanup EXIT INT TERM; \
		sleep 2; \
		echo "🚀 Triggering workflow inputs: $(2)"; \
		cargo run --example $(1) -- trigger $(2); \
		echo "⏳ Waiting for completion signal '$(3)' (timeout $(WORKFLOW_TIMEOUT)s)..."; \
		SECONDS_WAITED=0; \
		while [ $$SECONDS_WAITED -lt $(WORKFLOW_TIMEOUT) ]; do \
			if grep -q "$(3)" "$$LOG_FILE"; then \
				echo "🎉 $(1) workflow finished."; \
				break; \
			fi; \
			sleep 1; \
			SECONDS_WAITED=$$((SECONDS_WAITED + 1)); \
		done; \
		if [ $$SECONDS_WAITED -ge $(WORKFLOW_TIMEOUT) ]; then \
			echo "⚠️ Workflow did not finish within $(WORKFLOW_TIMEOUT)s. Logs saved to $$LOG_FILE"; \
			LOG_RETENTION="keep"; \
		fi; \
		cleanup; \
		trap - EXIT INT TERM
endef

example-welcome: db-up
	$(call RUN_WORKFLOW_WITH_TRIGGER,welcome_email,$(WELCOME_EMAIL_TRIGGER_NAME),✅ Welcome workflow completed)

examples:
	@echo "📚 Running all Kagzi examples..."
	@$(MAKE) example-simple
	@$(MAKE) example-batch-processing
	@$(MAKE) example-data-pipeline
	@$(MAKE) example-order-fulfillment
	@$(MAKE) example-retry-pattern
	@$(MAKE) example-scheduled-reminder
	@$(MAKE) example-welcome
	@echo "✅ Finished running all examples"

clean:
	@echo "🧹 Cleaning build artifacts..."
	cargo clean
