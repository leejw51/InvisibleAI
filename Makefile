.DEFAULT_GOAL := help
SHELL := /bin/bash
CONFIG_DIR ?= $(HOME)/.causewaybaybot
CLIENT_CONFIG_DIR ?= $(HOME)/.causewaybaybotclient
BACKEND_URL ?= http://127.0.0.1:3000
BIND_ADDR ?= 0.0.0.0:3000
OWNER_WALLET ?=
export OWNER_WALLET
CLIENT_NAME ?=
export CAUSEWAYBAY_CONFIG_DIR := $(CONFIG_DIR)
export CAUSEWAYBAY_CLIENT_CONFIG_DIR := $(CLIENT_CONFIG_DIR)
export CAUSEWAYBAY_BACKEND := $(BACKEND_URL)
export BIND_ADDR
export CAUSEWAYBAY_CLIENT_NAME := $(CLIENT_NAME)

.PHONY: help install setup css visuals frontend backend cli build build-debug start start2 start3 stop stop3 status start-server start-client run demo package test test-rust test-e2e check fmt verify version test-vision test-semantic test-tags-live test-cloud test-live test-package
help:
	@echo 'Causewaybay Bot — Mac Studio backend · MacBook capture · iPad web UI'
	@echo ''
	@echo '  make                  Show this help'
	@echo '  make install          Install dependencies, build tools, Chromium and WebKit'
	@echo '  make setup            Alias for make install'
	@echo '  make build            Build release binaries into ./bin and web assets into ./dist'
	@echo '  make build-debug      Build debug binaries and web assets for development/tests'
	@echo '  make start            Build and start backend only'
	@echo '  make start2           Build and start backend + Rust capture client'
	@echo '  make start3           Build and start Rust capture client only'
	@echo '  make stop             Stop both managed processes'
	@echo '  make stop3            Stop only the Rust capture client'
	@echo '  make status           Show process state'
	@echo '  make start-server     Alias for make start (Mac Studio)'
	@echo '  make start-client     Alias for make start3 (MacBook)'
	@echo '  make start3 BACKEND_URL=http://STUDIO:3000  Connect to a remote backend'
	@echo '  make start2 OWNER_WALLET=0x…  Enroll this Mac for TextEdit Auto; saved for later launches'
	@echo '  make package          Build release bundle + package/client.zip'
	@echo '  make version          Print the workspace version a release tag must match'
	@echo '  make demo             Run an explicit demo workspace in the foreground'
	@echo '  make test             Unit, integration, launcher, Chromium/iPad/iPhone tests'
	@echo '  make test-backend     Rust backend + capture CLI tests; no frontend needed'
	@echo '  make test-scripts     Isolated client ZIP and launcher regression tests'
	@echo '  make verify           Formatting, Clippy, Rust, and browser checks'
	@echo '  make test-vision      Real Apple Vision OCR test (macOS)'
	@echo '  make test-semantic    Real local embedding test (model downloads once)'
	@echo '  make test-cloud       Live Grok text and image tests (XAI_API_KEY)'
	@echo '  make test-live        Full Playwright flow with real OCR, Grok, and CLI'
	@echo '  make test-native-capture  Real desktop capture + OCR/image + Grok in Playwright'
	@echo '  make test-native-notes    Rust CLI + Apple AI + TextEdit Auto in Playwright (local only)'
	@echo '  make mlx-ai           Build the Rust MLX worker (Qwen on-device models) into ./bin'
	@echo '  make test-mlx         Real on-device Qwen answer through the Rust MLX worker'
	@echo '  make test-mlx-download  Real 2.5 GB Qwen3.5 4B download from Hugging Face, then an answer'
	@echo '  make test-native-qwen   Real Qwen3.5 4B answering in the browser through Playwright'
	@echo '  make test-native-solve  Capture with nothing typed: on-device 27B solves the screen math and celebrates'
	@echo '  make test-package     Build and smoke-test the release package in Playwright'
	@echo ''
	@echo '  Backend: $(CONFIG_DIR) · Capture client: $(CLIENT_CONFIG_DIR)'
	@echo '  Web preferences: browser local storage'
	@echo '  API keys: backend environment or .env (see .env.example)'

install:
	@for tool in rustup cargo node npm npx; do \
		command -v "$$tool" >/dev/null 2>&1 || { echo "Missing $$tool. Install Rust via rustup and Node.js 22.13+ first."; exit 1; }; \
	done
	@node -e 'const [major, minor] = process.versions.node.split(".").map(Number); if (major < 22 || (major === 22 && minor < 13)) { console.error("Node.js 22.13+ is required."); process.exit(1); }'
	rustup target add wasm32-unknown-unknown
	rustup component add rustfmt clippy
	@command -v trunk >/dev/null || cargo install trunk --locked
	cargo fetch --locked
	npm ci --include=dev
	npx --no-install playwright install chromium webkit
setup: install
css:
	npm run css
visuals:
	npm run visuals
frontend: css visuals
	env -u NO_COLOR trunk build
backend:
	cargo build -p causewaybay-server
cli:
	cargo build -p causewaybay-cli
build: frontend
	cargo build --release -p causewaybay-server -p causewaybay-cli --bin causewaybay-server --bin rust-cli
	mkdir -p bin
	install -m 755 target/release/causewaybay-server bin/server
	install -m 755 target/release/rust-cli bin/client
build-debug: frontend backend cli
start: frontend backend
	bash scripts/manage.sh start-server
start2: build-debug
	bash scripts/manage.sh start
start3: cli
	bash scripts/manage.sh start-client
start-server: start
start-client: start3
stop:
	bash scripts/manage.sh stop
# Stop the backend, rebuild frontend and backend, start again.
restart: stop start
stop3:
	bash scripts/manage.sh stop-client
status:
	bash scripts/manage.sh status
run: build
	./bin/server serve --bind '$(BIND_ADDR)' --assets dist
demo: build
	./bin/server serve --demo --no-semantic --data-dir '$(CONFIG_DIR)/demo' --assets dist
package: build apple-ai mlx-ai
	bash scripts/package.sh
# The number a release tag `vX.Y.Z` is compared against.
version:
	@cargo metadata --no-deps --format-version 1 --manifest-path Cargo.toml \
		| node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).packages.find(p=>p.name==="causewaybay-server").version))'
.PHONY: test-backend
test-backend:
	env CWBBOT_ADMIN= cargo test -p causewaybay-server -p causewaybay-shared -p causewaybay-cli
test-rust: test-backend
	cargo test -p causewaybay-frontend
test-e2e: build-debug
	npm run test:e2e
.PHONY: test-scripts restart
test-scripts:
	npm run test:scripts
# Discovers fresh-context unit/API/browser regressions alongside the full suite.
test: test-rust test-scripts test-e2e
test-vision:
	cargo test -p causewaybay-server --test vision -- --ignored --nocapture
test-semantic:
	cargo test -p causewaybay-server --test semantic -- --ignored --nocapture
	env -u CWBBOT_ADMIN cargo test -p causewaybay-server --lib search_tests -- --ignored --nocapture
test-tags-live: mlx-ai
	env -u CWBBOT_ADMIN cargo test -p causewaybay-server --lib search_tests::saved_notes_are_tagged -- --ignored --nocapture
test-cloud:
	cargo test -p causewaybay-server --test live_cloud -- --ignored --nocapture
test-live: build-debug test-vision test-audio-fixture test-cloud
	RUN_LIVE_AI=1 npm run test:e2e
.PHONY: test-native-capture
test-native-capture: build-debug
	RUN_NATIVE_CAPTURE=1 npm run test:e2e -- tests/e2e/native-capture.spec.mjs
.PHONY: test-native-notes
test-native-notes: build-debug apple-ai
	RUN_NATIVE_NOTES=1 npm run test:e2e -- tests/e2e/native-notes.spec.mjs --project=chromium
.PHONY: test-native-qwen
# Real on-device Qwen3.5 4B through the browser (make mlx-ai, model installed).
test-native-qwen: build-debug mlx-ai
	RUN_NATIVE_QWEN=1 npm run test:e2e -- tests/e2e/native-qwen.spec.mjs --project=chromium
.PHONY: test-native-solve
# Capture → on-device answer → Solved badge and celebration (27B full, 9B basic).
test-native-solve: build-debug mlx-ai
	RUN_NATIVE_QWEN=1 npm run test:e2e -- tests/e2e/native-capture-solve.spec.mjs --project=chromium
.PHONY: test-native-screen
test-native-screen: build-debug
	RUN_NATIVE_SCREEN=1 npm run test:e2e -- tests/e2e/native-screen.spec.mjs
test-package: package test-scripts
	TEST_PACKAGE=1 npm run test:e2e -- tests/e2e/package.spec.mjs tests/e2e/client-package.spec.mjs
check:
	cargo fmt --all -- --check
	cargo clippy -p causewaybay-server -p causewaybay-shared -p causewaybay-cli --all-targets -- -D warnings
	cargo clippy -p causewaybay-frontend --target wasm32-unknown-unknown -- -D warnings
fmt:
	cargo fmt --all
verify: check test

.PHONY: test-audio-fixture test-auth
test-audio-fixture:
	mkdir -p target/test-fixtures
	/usr/bin/say -v Samantha -r 140 --file-format=WAVE --data-format=LEI16@16000 -o target/test-fixtures/audio-question.wav 'What is seven plus five?'
test-auth: build-debug
	cargo test -p causewaybay-shared identity::tests
	cargo test -p causewaybay-server --test auth
	npm run test:e2e -- tests/e2e/auth.spec.mjs

.PHONY: mlx-ai test-mlx
# Standalone crate: compiles MLX from source once (cmake), then installs the worker.
mlx-ai:
	cargo build --release --manifest-path native/mlx-ai/Cargo.toml
	mkdir -p bin
	install -m 755 native/mlx-ai/target/release/invisible-mlx bin/invisible-mlx
	@KERNEL=$$(find native/mlx-ai/target/release/build -path '*/out/build/lib/mlx.metallib' -type f -print -quit); test -n "$$KERNEL"; install -m 644 "$$KERNEL" bin/mlx.metallib
test-mlx: mlx-ai
	cargo test --manifest-path native/mlx-ai/Cargo.toml --release
	env CWBBOT_ADMIN= cargo test -p causewaybay-server --test local_models -- --ignored --nocapture live_mlx_worker
# Downloads the real 2.5 GB Qwen3.5 4B into a temp dir, then answers through the worker.
.PHONY: test-mlx-download
test-mlx-download: mlx-ai
	env CWBBOT_ADMIN= cargo test -p causewaybay-server --test local_models -- --ignored --nocapture live_download

.PHONY: apple-ai
apple-ai:
	swift build -c release --package-path native/apple-ai
	mkdir -p bin
	install -m 755 "$$(swift build -c release --package-path native/apple-ai --show-bin-path)/invisible-apple-ai" bin/invisible-apple-ai
