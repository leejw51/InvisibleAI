# Invisible AI

<p align="center"><img src="frontend/assets/everyday-companion.png" alt="The Invisible AI companion, a small glassy lavender character waving hello" width="200"></p>

A little everyday help on your Mac. Ask a question, share a photo, keep an idea, or let your AI help where you write. Choose on-device AI, cloud AI, or a mix. The crates and the repository keep the `causewaybay` prefix from the project's origin; the product is Invisible AI.

![Invisible AI's lavender companion floating above an open notebook with orbiting notes and a photo](frontend/assets/invisible-ai-hero.png)

Meet the **companion**, Invisible AI's main character: a small glassy friend who waits on the empty chat, listens while you type, and shows when it's working (*Ready when you are*, *I'm all ears*, *Working…*). It also greets you while the workspace connects and pops in with a burst of stars when a capture's problem is solved. Its motion follows your system's Reduce Motion setting.

- **New thought** is one chat for anything: type, paste a picture, drop files, press Enter. Everything is kept, searched (BM25 plus local embeddings), tagged, and organized into a **Wiki**.
- **Capture** answers one screen of a connected Mac with no typed prompt. **Autonomous AI** watches a TextEdit or Apple Notes note on that Mac and writes the answer under your text, or takes requests from an iPhone or iPad.
- **AI modes:** On-device (Apple's LLM, or Qwen3.5 4B / 9B / Qwen3.8 27B through the backend's Rust MLX worker, downloaded from Backend settings with a GPU-memory check), Hybrid, Cloud AI (Anthropic, OpenAI, Grok, Gemini), or the `codex` / `claude` command-line tools signed in on the backend Mac. One **AI settings** page holds the choice, saved in the browser and mirrored to the account; the same picker sits in Chat and Coding.
- **Coding agent** in any chat: versioned file writes, note and file reading, web search, and a sandboxed standard-library Rust compile check. No shell, no program execution in chat.
- **Coding** page: one folder per project (Rust, Go, Python via Anaconda, C, Lua, TypeScript) under the backend's data folder. Describe the program; the AI picked on the page writes it (On-device, Hybrid and Cloud AI through the backend's own harness, which keeps iterating until the program runs; Codex and Claude Code directly in the folder with their tools), the backend fetches dependencies (cargo, go mod, pip, npm, the only step with network), builds, runs and tests it in a seatbelt sandbox with no access outside that folder, and the output comes back to the page. Cloud providers work with OpenCode-style tools (read, grep, edit, run, test); Claude Code and Codex work in the folder with their own tools and a per-project session. Edit files in place, run with any stdin, and publish a finished program as a **skill**. **Skills** are the goal: command-line programs of your own (stdin and arguments in, stdout out) that the Skills page manages (make from a description, add from a project, rename, run, retire, delete) and Cloud AI chat can run (`run_skill`). On-device, Hybrid, and CLI chat cannot run published skills; use the Skills page to run them directly. The agent is limited to building skills in their folders; it cannot change anything else.

## Install a built release

Run `make package` on an Apple Silicon Mac to produce two archives and `package/SHA256SUMS`:

- `package/invisible-ai-server-<version>-darwin-arm64.zip` — backend, web UI, on-device AI workers and a local capture client. Unzip it and double-click **Start Invisible AI.command**; it opens your browser and runs the backend and capture client, and Control-C in its Terminal window stops both.
- `package/invisible-ai-client-<version>-darwin-arm64.zip` — the capture client alone, for another Mac pointing at that backend. Unzip it and run `./go.sh`.

Each archive includes `.env.example`, [QUICK.md](packaging/QUICK.md) (a five-minute tutorial for server and client) and [README.md](packaging/README.md) (the full manual). No Node.js or Rust is needed for ordinary use.

The bundle includes Apple AI, the Rust MLX worker and Metal kernels. Apple AI requires macOS 26 and Apple Intelligence enabled; MLX models are downloaded explicitly in Settings. Releases are currently unsigned/not notarized. Optional coding toolchains and Python speech packages are separate installations.

## Build from source

Requires macOS, Rust 1.90+ via rustup, and Node.js 22.13+.

```sh
make install
cp .env.example .env     # optional cloud keys; wallet setup is optional locally
make start2              # app + local capture on http://127.0.0.1:3000
```

Open the printed loopback URL in your browser. Local capture authenticates automatically using a private credential. To connect another device, explicitly configure a network listener and `APP_TOKEN`; use a trusted encrypted connection such as Tailscale. The full walkthrough is in the [guide](docs/guide.md).

## Privacy model

- **On-device first.** In On-device or admin Local-only mode, captures, OCR text, prompts, answers, indexes, and history never leave your Mac. Hybrid keeps Chat and remote requests on-device. Auto lets the local model request cloud help and honors the cloud-confirmation setting; watched text never counts as approval. Coding can fall back to cloud when the local model cannot finish, only if cloud confirmation is off.
- **Keys stay on the backend.** Provider keys come only from the backend's environment or `.env`; the browser and the capture client never see them.
- **Delete all.** Erases submitted content and its derived data from this backend: chats, notes, attachments, captures, OCR/transcripts, answers, search indexes, Wiki pages/tags, projects (including Git history), published skills, and cached thumbnails. An interrupted deletion resumes at startup before requests are accepted. Connected browsers clear their saved copies; offline browsers and capture clients receive deletion notices when they reconnect. Login and settings remain. Original source files, backups, and copies retained independently by external AI services or CLI tools are outside this operation.
- **Wallet isolation.** Every row and file belongs to the signed-in wallet. Admins approve accounts but cannot read another wallet's content through the API.
- **Cronos wallet.** The Wallet page holds and sends CRO, USDC, or any ERC-20 added by contract address. USDC is on Cronos mainnet (chain 25, `0xc21223249CA28397B4B6541dfFaEcC539BfF0c59`) and devUSDC.e on Cronos testnet (chain 338, `0xc01efAaF7C5C61bEbFAeb358E1161b537b8bC0e0`). Signing in with a recovery phrase or a private key saves only the derived private key, in the browser's local storage; the phrase is never stored. The saved key is never rendered and leaves only through a confirmed **Copy**; a new phrase is shown only when the user presses **Show**. The selected token is always displayed large with its contract address. Fees are paid in CRO, so the CRO balance stays in view whatever is being sent. The backend's SQLite keeps only public wallet choices per account: the network, the asset picked on each network, and saved tokens' address, symbol, name and decimals. A later sign-in reopens the wallet as it was left. The browser talks to the public Cronos JSON-RPC directly (the CSP allows exactly those two hosts), so neither the backend nor any AI provider ever receives a key or a phrase (the settings API accepts only those named fields); only the signed transaction goes out, to the Cronos node. Transfers are legacy EIP-155 transactions, refused before signing when the recipient is your own address or the token's own contract, a mixed-case address fails EIP-55, the amount exceeds the balance, or the fee would pass a 20 CRO ceiling. The core is ported from the Causewaybay Wallet and checked against its test vectors (`crates/shared/testvectors`). Card artwork: `node scripts/generate-wallet-art.mjs` with `XAI_API_KEY` set.
- **Bounded tools.** The chat agent has 8 rounds and 24 tool calls per request. Compilation runs under an OS sandbox with an allow-list of readable paths and no network.
- **Project sandbox.** Coding projects build and run under `sandbox-exec` with no network, writes only inside the project folder and reads only from the toolchains and the system; a run is killed after 60 seconds (builds after 120). Codex and Claude Code work inside the same read allow-list plus their own state folders, so the rest of the home folder stays unreadable to them.

## Design limits

- **One household.** The backend holds a single SQLite connection behind a mutex and serializes chat submissions. That is correct for a home Mac Studio serving a few wallets, and a ceiling for anything larger.
- **macOS backend.** Apple Vision OCR, the `sandbox-exec` compiler sandbox, Apple Foundation Models, and screen capture are macOS only. Other platforms fail closed for those features.
- **Trusted network.** The server speaks HTTP and WebSocket inside your LAN or Tailscale network; put HTTPS in front of it for anything else.

## Documentation

- [Guide](docs/guide.md): setup, every page and control, storage layout, tests, release and packaging.
- [Invisible AI sessions](docs/invisible-ai.md): autonomous runs, local models, Wiki configuration.
- [PLAN.md](PLAN.md): architecture and flows. [PROTOCOL.md](PROTOCOL.md): HTTP, WebSocket, and SSE contracts.
- [ISSUES.md](ISSUES.md): the public-release work plan. [TODO.md](TODO.md): roadmap.
- [CONTRIBUTING.md](CONTRIBUTING.md) and [SECURITY.md](SECURITY.md).

MIT licensed. See [LICENSE](LICENSE).
