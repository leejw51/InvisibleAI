# Contributing

Thanks for helping with Invisible AI. The project is a Rust workspace (`crates/server`, `crates/cli`, `crates/frontend`, `crates/shared`) with a Swift worker in `native/apple-ai`, Tailwind and Playwright driven by Node.

## Set up

Requires macOS, Rust 1.90+ installed through rustup, and Node.js 22.13+.

```sh
make install        # WASM target, rustfmt, Clippy, Trunk, npm packages, Playwright browsers
cp .env.example .env
make start          # build the web UI and backend, start the backend
make stop
```

`make` alone prints every target. `make start2` also starts a local capture client; `make start3 BACKEND_URL=...` starts only the client.

## Check and test

```sh
make check          # cargo fmt --check, strict Clippy for native and WASM crates
env -u CWBBOT_ADMIN cargo test     # Rust unit and integration tests (offline)
make test-scripts   # launcher and packaging script tests
make test-e2e       # Playwright browser tests against a demo backend
make verify         # check + everything above
```

Notes that save time:

- If your shell exports `CWBBOT_ADMIN`, run the Rust tests with `env -u CWBBOT_ADMIN cargo test`. With the variable set, the test backends start in wallet mode and around forty tests fail with 401.
- The Rust test suite is macOS only. Apple Vision OCR, the `sandbox-exec` compiler sandbox, and screen capture are gated to macOS, and the compile tests expect `rustup` on `PATH`.
- Live tests are opt-in and make paid API calls: `make test-cloud`, `make test-live`, `make test-native-capture` read provider keys from your environment and never write them anywhere.
- Playwright reports land in `playwright-report/` and `test-results/`. A couple of specs depend on real OCR timing; rerun a failing spec alone before assuming a regression.

## Pull requests

- Keep a change to one concern. Behavior changes, pure moves, and documentation are easier to review as separate commits.
- Run `make check` before pushing. CI runs the same formatting, Clippy, build, and offline test steps on a macOS runner.
- User-facing strings live in `crates/frontend/src/i18n.rs` with seven languages; add every new string to all of them.
- Protocol changes need a matching update in `PROTOCOL.md`. New pages need the server's single-page-app path list in `crates/server/src/lib.rs` and the `location_page` match in `crates/frontend/src/main.rs`.
- Generated artwork keeps its prompt beside the file (`frontend/assets/*.prompt.txt`).
- Never commit `.env`, recovery phrases, private keys, or provider keys. `.env.example` lists every variable the server reads and holds no values.

## Reporting security issues

See [SECURITY.md](SECURITY.md).
