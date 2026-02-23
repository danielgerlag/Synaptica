# Synaptica — Distributed Graph Database
# Makefile for building, testing, and running all components

.PHONY: all build build-server build-ui clean test test-server test-ui \
        run run-server run-ui dev dev-server dev-ui install-ui \
        lint bench help

# ─── Configuration ───────────────────────────────────────────────────
CARGO       := cargo
NPM         := npm
DATA_DIR    := ./data
LISTEN_ADDR := 0.0.0.0:9090
UI_ADDR     := 0.0.0.0:8080
UI_DIR      := ui/dist

# ─── Default ─────────────────────────────────────────────────────────
all: build ## Build everything (server + UI)

# ─── Build Targets ───────────────────────────────────────────────────
build: build-server build-ui ## Build server (release) and UI (production)

build-server: ## Build the Rust server in release mode
	$(CARGO) build --release -p synaptica-server

build-debug: ## Build the Rust server in debug mode
	$(CARGO) build -p synaptica-server

build-ui: install-ui ## Build the UI for production
	cd ui && $(NPM) run build

install-ui: ## Install UI dependencies
	cd ui && $(NPM) install --prefer-offline

# ─── Test Targets ────────────────────────────────────────────────────
test: test-server ## Run all tests

test-server: ## Run Rust workspace tests
	$(CARGO) test --workspace

test-storage: ## Run storage engine tests only
	$(CARGO) test -p synaptica-storage

test-tx: ## Run transaction layer tests only
	$(CARGO) test -p synaptica-tx

test-gql: ## Run GQL parser tests only
	$(CARGO) test -p synaptica-gql

test-exec: ## Run execution engine tests only
	$(CARGO) test -p synaptica-exec

test-cluster: ## Run cluster layer tests only
	$(CARGO) test -p synaptica-cluster

test-integration: ## Run integration tests only
	$(CARGO) test --test gql_compliance_tests --test cluster_tests

# ─── Run Targets ─────────────────────────────────────────────────────
run: build ## Build everything and launch server with UI
	./target/release/synaptica-server \
		--data-dir $(DATA_DIR) \
		--listen $(LISTEN_ADDR) \
		--ui-dir $(UI_DIR) \
		--ui-addr $(UI_ADDR)

run-server: build-server ## Build and run server only (no UI)
	./target/release/synaptica-server \
		--data-dir $(DATA_DIR) \
		--listen $(LISTEN_ADDR)

# ─── Development ─────────────────────────────────────────────────────
dev: ## Start server (debug) and UI dev server concurrently
	@echo "Starting Synaptica dev environment..."
	@echo "  Server: $(LISTEN_ADDR) (gRPC)"
	@echo "  UI:     http://localhost:3000"
	@echo ""
	$(MAKE) dev-server &
	$(MAKE) dev-ui
	@wait

dev-server: build-debug ## Run server in debug mode
	./target/debug/synaptica-server \
		--data-dir $(DATA_DIR) \
		--listen $(LISTEN_ADDR)

dev-ui: install-ui ## Start Vite dev server with hot reload
	cd ui && $(NPM) run dev

# ─── Quality ─────────────────────────────────────────────────────────
lint: ## Run clippy lints on Rust code
	$(CARGO) clippy --workspace -- -D warnings

check: ## Type-check Rust and TypeScript without building
	$(CARGO) check --workspace
	cd ui && npx tsc --noEmit

fmt: ## Format Rust code
	$(CARGO) fmt --all

bench: ## Run benchmarks
	$(CARGO) bench -p synaptica-storage
	$(CARGO) bench -p synaptica-gql

# ─── Cleanup ─────────────────────────────────────────────────────────
clean: ## Remove build artifacts
	$(CARGO) clean
	rm -rf ui/dist ui/node_modules/.vite

clean-all: clean ## Remove all generated files including node_modules and data
	rm -rf ui/node_modules $(DATA_DIR)

# ─── Help ────────────────────────────────────────────────────────────
help: ## Show this help
	@grep -E '^[a-zA-Z_-]+:.*?## .*$$' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-18s\033[0m %s\n", $$1, $$2}'
