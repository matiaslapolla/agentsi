.PHONY: help release debug dist dist-portable dist-clean app core pill tests clippy fmt ci install guide guide-shots checks-bin checks checks-stop clean dev up start db down logs
.DEFAULT_GOAL := help

# The gate of devflow runs any target named check, typecheck, lint, test or build as its own
# check, so no target here has those names.
# JOBS=4 limits the build jobs. LOW=1 runs cargo at low disk priority, as the workers did.
JOBS ?=
LOW ?=
CARGO := $(if $(JOBS),CARGO_BUILD_JOBS=$(JOBS)) $(if $(LOW),ionice -c3) cargo
PROGRAMS := agentsi agentsi-core agentsi-app agentsi-pill
CHECKS ?= $(HOME)/.cache/agentsi/desk-workers/owner-checks
DEVFLOW := MISE_NODE_VERSION=26.7.0 python3 $(HOME)/dev/personal/productos/claudeconfig/scripts/devflow

help: ## List the targets
	@grep -E '^[a-z-]+:.*## ' $(MAKEFILE_LIST) | awk -F':.*## ' '{printf "  \033[1m%-12s\033[0m %s\n", $$1, $$2}'

# Desktop app (Rust)

release: ## Build the four programs in target/release
	$(CARGO) build --release -p agentsi-cli -p agentsi-core -p agentsi-app -p agentsi-pill

debug: ## Build the four programs in target/debug
	$(CARGO) build -p agentsi-cli -p agentsi-core -p agentsi-app -p agentsi-pill

app: release ## Build, then open the app (it starts its own core)
	target/release/agentsi-app

core: release ## Build, then run the core in the foreground
	target/release/agentsi-core serve

pill: release ## Build, then run the voice pill in the foreground
	target/release/agentsi-pill

tests: ## Run the tests of the whole workspace
	$(CARGO) test --workspace

clippy: ## Run clippy with warnings as errors
	$(CARGO) clippy --workspace --all-targets -- -D warnings

fmt: ## Format the Rust code
	cargo fmt --all

ci: ## Run the full commit gate (typecheck, lint, tests, clippy)
	$(if $(JOBS),CARGO_BUILD_JOBS=$(JOBS)) $(DEVFLOW) check --timeout 1500

dist: release ## Pack the programs into dist/agentsi-<commit>-linux-<arch>.tar.gz (glibc of this machine)
	scripts/dist.sh

dist-portable: ## Build in an ubuntu:24.04 container, then pack (runs on glibc 2.39 or newer)
	scripts/dist.sh --portable

dist-clean: ## Delete dist/ and the Docker volumes of dist-portable
	rm -rf dist
	-docker volume rm agentsi-dist-cargo agentsi-dist-target agentsi-dist-cache

install: release ## Install the programs in ~/.local/bin (add ENABLE=1 to start the services)
	packaging/install.sh $(if $(ENABLE),--enable)

guide: ## Open the usage guide (docs/guide/README.md) with the default program
	xdg-open docs/guide/README.md >/dev/null 2>&1 || true

guide-shots: ## Take the screenshots of the guide again, in a nested Hyprland
	scripts/guide-shots.sh

clean: ## Delete the build folders (target/ and the worker folders)
	cargo clean
	rm -rf $(HOME)/.cache/agentsi-targets

# Owner checks on a copy of the data (runbook, part 1)

checks-bin: release ## Build, then copy the programs into the owner-checks folder
	@pgrep -f '$(CHECKS)/bin/[a]gentsi-app' >/dev/null && { echo "Close the check app first (make checks-stop)."; exit 1; } || true
	@for p in $(PROGRAMS); do cp target/release/$$p $(CHECKS)/bin/.$$p.new && mv -f $(CHECKS)/bin/.$$p.new $(CHECKS)/bin/$$p; done
	@echo "Copied $(PROGRAMS) into $(CHECKS)/bin. Restart the check core and pill to use them."

checks: ## Open the app on the copy of the data
	$(CHECKS)/start.sh

checks-stop: ## Stop the check app, core and pill, by pid
	@pids=$$(pgrep -f '$(CHECKS)/bin/[a]gentsi-' || true); [ -n "$$pids" ] && kill $$pids && echo "Stopped $$pids" || echo "Nothing runs."

# Web app (TypeScript). Step 7 of the switch-over deletes this part.

dev: db ## Postgres + migrations, then the web app in the foreground (http://127.0.0.1:3361)
	pnpm dev

up: dev ## Alias of dev

start: dev ## Alias of dev

db: ## Start Postgres and apply migrations
	node packages/cli/bin/agentsi.mjs up

down: ## Stop Postgres (the data volume stays)
	docker compose down

logs: ## Follow the Postgres logs
	docker compose logs -f postgres
