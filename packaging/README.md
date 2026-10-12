# Invisible AI — manual

Invisible AI is a little everyday help on your Mac: ask a question, share a photo, keep an idea, capture a screen and get the answer, or let the AI write under your notes. One Mac runs the **server** (backend, web UI, data, AI); any other Mac can run the **client** to send screen captures to it. Use the web UI from any browser that can reach the server.

New here? `QUICK.md` gets you running in five minutes. This manual covers everything else.

## Contents

- [Requirements](#requirements)
- [What is in each archive](#what-is-in-each-archive)
- [Server](#server)
- [Client](#client)
- [Security and privacy](#security-and-privacy)
- [Troubleshooting](#troubleshooting)
- [Updating and uninstalling](#updating-and-uninstalling)

## Requirements

- macOS 26 on Apple silicon (both server and client).
- Apple AI needs Apple Intelligence enabled in System Settings.
- Qwen models need enough unified memory: Backend settings checks before it downloads.
- Optional: [Tailscale](https://tailscale.com) to reach the server from other devices.
- Optional: an API key for Grok (xAI), Anthropic, OpenAI or Gemini for cloud AI.
- Optional, server only: language toolchains (Rust, Go, Python, …) for the Coding page; Python speech packages for local transcription.

Nothing else: no Rust, Node.js or Xcode is needed to run either archive.

The binaries are not signed or notarized. On first launch, right-click → **Open**, or clear the quarantine flag on the unzipped folder:

```sh
xattr -dr com.apple.quarantine invisible-ai-server-*/
```

Verify a download against the release's `SHA256SUMS`:

```sh
shasum -a 256 -c SHA256SUMS --ignore-missing
```

## What is in each archive

**`invisible-ai-server-<version>-darwin-arm64.zip`**

| Path | Purpose |
| --- | --- |
| `Start Invisible AI.command` | double-click launcher: server + local client + browser |
| `bin/server` | the backend |
| `bin/client` | the capture client (also served by **Dashboard → Download client.zip**) |
| `bin/invisible-apple-ai` | Apple Foundation Models worker |
| `bin/invisible-mlx`, `bin/mlx.metallib` | MLX worker and GPU kernels for Qwen models |
| `dist/` | the web UI |
| `scripts/local-ai/transcribe.py` | optional local speech transcription |
| `.env.example` | server configuration template |
| `QUICK.md`, `README.md`, `docs/`, `PLAN.md`, `PROTOCOL.md` | documentation |
| `LICENSE`, `THIRD_PARTY_NOTICES` | licenses |

**`invisible-ai-client-<version>-darwin-arm64.zip`**

| Path | Purpose |
| --- | --- |
| `client` | the capture client |
| `go.sh` | launcher: reads `.env`, asks for anything missing, starts `client` |
| `.env.example` | client configuration template |
| `QUICK.md`, `README.md` | documentation |

The folder can be moved anywhere; the server finds its workers and web UI relative to `bin/`.

## Server

### Starting and stopping

**Double-click** `Start Invisible AI.command`. It starts the server on `127.0.0.1:3000`, starts a capture client for this Mac, and opens the browser. Keep the Terminal window open; Control-C or closing it stops both. Launching twice is refused while the first copy runs.

**From Terminal**, for more control:

```sh
./bin/server serve [options]
```

| Option | Environment | Default | Meaning |
| --- | --- | --- | --- |
| `--bind ADDR` | `BIND_ADDR` | `127.0.0.1:3000` | listen address; anything other than loopback needs `APP_TOKEN` or `CWBBOT_ADMIN` |
| `--assets DIR` | `DIST_DIR` | `dist` beside `bin/` | built web UI |
| `--config-dir DIR` | `CAUSEWAYBAY_CONFIG_DIR` | `~/.causewaybaybot` | settings, keys, and the default data folder |
| `--data-dir DIR` | `DATA_DIR` | `<config dir>/data` | database and files, for this launch only |
| `--token TOKEN` | `APP_TOKEN` | none | node token for remote clients and devices (16+ URL-safe characters) |
| `--no-semantic` | `SEMANTIC_SEARCH=off` | on | skip local embeddings; search uses BM25 only |
| `--demo` | `DEMO_MODE=1` | off | canned answers, never calls a provider (testing only) |

Stop it with Control-C.

### Configuration (`.env`)

Copy `.env.example` to `.env`. The server reads `.env` from the folder you start it in, then `~/.causewaybaybot/.env` (or `<config dir>/.env`). Variables already exported in your shell win. Restart the server after changes.

| Variable | Purpose |
| --- | --- |
| `XAI_API_KEY` | Grok; chosen automatically when it is the only key |
| `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY` | other cloud providers |
| `CWBBOT_ADMIN` | owner wallet address: enables wallet sign-in and account approval |
| `APP_TOKEN` | token remote clients must present; required for a network listener without `CWBBOT_ADMIN` |
| `BIND_ADDR`, `DATA_DIR`, `DIST_DIR`, `SEMANTIC_SEARCH` | same as the options above |
| `INVISIBLE_APPLE_WORKER`, `INVISIBLE_MLX_PYTHON`, `INVISIBLE_WHISPER_WORKER` | override worker locations |

Keys stay on the server. The browser and clients never receive them. Never put a recovery phrase in `.env`.

### AI modes

Choose in **AI settings** (also in Chat and Coding):

- **On-device** — Apple's model, or Qwen3.5 4B / 9B / Qwen3.8 27B through the bundled MLX worker. Download Qwen weights in **Backend settings**; nothing leaves the Mac.
- **Hybrid** — chat and remote requests stay on-device; the rest may use cloud.
- **Cloud AI** — Anthropic, OpenAI, Grok or Gemini, using the keys in `.env`.
- **CLI** — the `codex` or `claude` command-line tools, if signed in on the server Mac.

### Other devices

1. Install Tailscale on the server and the other device.
2. In `.env`, set `APP_TOKEN` (and optionally `CWBBOT_ADMIN`).
3. Start with `--bind 0.0.0.0:3000` (or set `BIND_ADDR`).
4. Browse to `http://<server-tailscale-ip>:3000`. **Devices** lists the exact addresses and client commands.

The server speaks plain HTTP and WebSocket. Keep it on a trusted network such as Tailscale; put HTTPS in front of it for anything else.

### Data

Everything is in `~/.causewaybaybot/` unless you moved it: `data/` (SQLite database, files, captures, projects, models), `capture-token` (the local client's credential), and `.env`. Back up that folder to back up the app. **Delete all** in the web UI erases your content but keeps login and settings.

## Client

The client captures this Mac's screen when you press **Capture** in the web UI and, if enrolled, watches a TextEdit or Notes document for **Autonomous AI**. It stores no content beyond its settings.

### With go.sh

```sh
cp .env.example .env     # optional
./go.sh
```

`go.sh` takes its values in this order: values baked in by a Dashboard download, variables already exported in your shell (`APP_TOKEN`, `CAUSEWAYBAY_CLIENT_CONFIG_DIR`), `.env` beside it, and finally a prompt.

| `.env` key | Purpose |
| --- | --- |
| `BACKEND_URL` | the server's Tailscale URL, e.g. `http://100.64.0.20:3000` |
| `OWNER_WALLET` | owner wallet address from the server's Dashboard |
| `APP_TOKEN` | node token from the server owner; prompted without echo if missing |
| `CAUSEWAYBAY_CLIENT_CONFIG_DIR` | settings folder (default `~/.causewaybaybotclient`) |

`.env` is read as plain `KEY=value` lines, never executed. If you store a token there, `chmod 600 .env`.

The first capture triggers macOS's **Screen Recording** prompt for Terminal (or whichever app launched the client). Allow it and relaunch.

### Running client directly

```sh
./client http://100.64.0.20:3000 0xYOUR_WALLET   # only this wallet may capture
./client http://100.64.0.20:3000                 # any approved user may capture
./client                                          # reconnect with saved settings
./client client --backend URL --name "MacBook" --owner-wallet 0x… [--retry-seconds N]
./client --help
```

The token comes from `APP_TOKEN` in the environment. Settings are saved to `~/.causewaybaybotclient/client.jsonl`. The client waits and reconnects if the server is unavailable.

## Security and privacy

- On-device mode keeps captures, text, prompts and answers on the server Mac.
- Provider keys live only in the server's environment or `.env`.
- Each account (wallet) sees only its own data; admins approve accounts but cannot read others' content.
- Neither archive contains a token, key, wallet secret or user data. A Dashboard `client.zip` contains only the server URL and your public wallet address.
- Coding projects build and run in a macOS sandbox without network access, limited to their own folder.

## Troubleshooting

| Symptom | Fix |
| --- | --- |
| "cannot be opened because the developer cannot be verified" | right-click → Open, or `xattr -dr com.apple.quarantine <folder>` |
| `Invisible AI is already open` | close the other launcher window; if none is open, remove `~/.causewaybaybot/desktop-launch.lock` |
| Server refuses to start on `0.0.0.0` | set `APP_TOKEN` (16+ characters) or `CWBBOT_ADMIN` in `.env` |
| Client keeps reconnecting | check the URL and port, that the server listens on the network, and that both are on Tailscale |
| Client gets `401` | wrong or missing `APP_TOKEN` |
| Captures are blank | allow Screen Recording for the launching app, then restart the client |
| Apple AI unavailable | enable Apple Intelligence; it needs macOS 26 |
| No cloud answers | add a key to `.env` and restart the server |

## Updating and uninstalling

Unzip the new version, copy your `.env` across, and start it. Data in `~/.causewaybaybot/` carries over.

To uninstall, delete the unzipped folders. To remove all data too, delete `~/.causewaybaybot/` (server) and `~/.causewaybaybotclient/` (client).
