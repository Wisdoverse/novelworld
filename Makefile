.PHONY: start stop restart logs status clean setup dev test

# One-click start (interactive setup on first run)
start:
	@bash start.sh

# Stop all services
stop:
	docker compose down

# Restart
restart: stop start

# View logs
logs:
	docker compose logs -f

# Service status
status:
	@docker compose ps

# Remove all data (volumes)
clean:
	docker compose down -v

# Development mode (requires Rust + Node.js)
dev:
	docker compose up -d postgres redis
	@echo "Infrastructure ready. Run services manually:"
	@echo "  cargo run -p gateway &"
	@echo "  cargo run -p user-service &"
	@echo "  cargo run -p novel-service &"
	@echo "  cargo run -p agent-service &"
	@echo "  cargo run -p narrative-service &"
	@echo "  cd frontend && pnpm dev"

# Run all tests
test:
	cargo test --workspace
	cd frontend && pnpm test
