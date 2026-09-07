# Common tasks. `make help` lists them.
.DEFAULT_GOAL := help
PY ?= python3

.PHONY: help install dev test tui web report doctor docker-build docker-up docker-down capture clean

help: ## Show this help
	@grep -hE '^[a-z-]+:.*##' $(MAKEFILE_LIST) | sed 's/:.*##/\t/' | expand -t22

install: ## Install runtime dependencies
	$(PY) -m pip install -r requirements.txt

dev: ## Install runtime + test dependencies
	$(PY) -m pip install -r requirements-dev.txt

test: ## Run the test suite
	$(PY) -m pytest

tui: ## Terminal dashboard against the offline simulator
	$(PY) -m btc5m.cli tui --profile demo --source simulated

web: ## Web console on http://127.0.0.1:8000 (local, no TLS)
	BTC5M_WEB_HTTPS=0 BTC5M_WEB_READONLY=0 \
	$(PY) -m uvicorn web.app:app --host 127.0.0.1 --port 8000

report: ## Print the PnL report
	$(PY) -m btc5m.cli report

doctor: ## Preflight checks
	$(PY) -m btc5m.cli doctor --source polymarket

docker-build: ## Build the image
	docker compose build

docker-up: ## Start the web console in Docker
	docker compose up -d web

docker-down: ## Stop everything
	docker compose down

capture: ## Regenerate docs/tui.svg
	$(PY) tools/capture_tui.py --out docs/tui.svg

clean: ## Remove caches and local state
	rm -rf .pytest_cache **/__pycache__ *.egg-info runtime
