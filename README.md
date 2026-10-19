# Invisible AI

<p align="center"><img src="frontend/assets/everyday-companion.png" alt="The Invisible AI companion, a small glassy lavender character waving hello" width="200"></p>

A little everyday help on your Mac. Ask a question, share a photo, keep an idea, or let your AI help where you write. Choose on-device AI, cloud AI, or a mix. The crates and the repository keep the `causewaybay` prefix from the project's origin; the product is Invisible AI.

![Invisible AI's lavender companion floating above an open notebook with orbiting notes and a photo](frontend/assets/invisible-ai-hero.png)

Meet the **companion**, Invisible AI's main character: a small glassy friend who waits on the empty chat, listens while you type, and shows when it's working (*Ready when you are*, *I'm all ears*, *Working…*). It also greets you while the workspace connects and pops in with a burst of stars when a capture's problem is solved. Its motion follows your system's Reduce Motion setting.

- **New thought** is one chat for anything: type, paste a picture, drop files, press Enter. Everything is kept, searched (BM25 plus local embeddings), tagged, and organized into a **Wiki**.
- **Capture** answers one screen of a connected Mac with no typed prompt. **Autonomous AI** watches a TextEdit or Apple Notes note on that Mac and writes the answer under your text, or takes requests from an iPhone or iPad.
- **Audio Capture** listens through that Mac's microphone and writes **Heard** messages into a synced chat (transcribed on the backend Mac by Apple Speech or MLX Whisper, never a cloud transcriber). Press **Solve** from any device for an answer to the newest speech, or **Note** for a write-up of the session. Start a **New chat** while listening or reopen an earlier room; transcripts and answers survive restarts, and a rolling summary keeps long sessions within the AI's context.
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
- **Delete all.** Erases submitted content and its derived data from this backend: chats, notes, attachments, captures, OCR/transcripts, answers, search indexes, Wiki pages/tags, projects (including Git history), published skills, and cached thumbnails. An interrupted deletion resumes at startup before requests are accepted. Connected browsers clear their saved copies, including this browser's capture choices, coding prompts, upload resumptions and Wiki views; offline browsers and capture clients receive deletion notices when they reconnect. The wallet key, login and settings remain. It sits in the main menu (**다 지워** in Korean). Original source files, backups, and copies retained independently by external AI services or CLI tools are outside this operation.
- **Wallet isolation.** Every row and file belongs to the signed-in wallet. Admins approve accounts but cannot read another wallet's content through the API.
- **Multichain wallet.** The Wallet page covers every network and coin of the Causewaybay Wallet, plus Arbitrum: Cronos (mainnet, testnet), Arbitrum One and Sepolia, Solana (mainnet, devnet, testnet), Cardano (mainnet, preprod, preview), Midnight (preview, devnet) and eCash (mainnet, testnet), each with its own coin and registry tokens (USDC, USDT, DAI, devUSDC.e, Cardano stablecoins), and any ERC-20 added by pasting its contract. Two main buttons, **CRONOS MAINNET** (CRO) and **USDC** on Cronos Mainnet, sit beside one search box that finds everything else. New accounts open on Cronos testnet, and the coin list ranks coins by how often they are used (counted in SQLite). Sending asks for the amount with its denomination, restates the recipient and amount, and above 20 CRO or 1 USD (or for a coin the wallet cannot price) requires the amount and coin to be typed again; a three.js send stage shows both wallets' balances before, during and after the transfer. EVM coins can be sent (legacy EIP-155 transactions, refused before signing for your own address, the token's own contract, a bad EIP-55 checksum, an amount over the balance, or a fee past the network's ceiling: 20 CRO, 0.005 ETH on Arbitrum); other chains are view and receive. The selected coin is always shown large with its network and contract, and the fee coin's balance stays in view. The private key and, after a phrase sign-in, the recovery phrase live only in this browser's local storage; Solana, Cardano, Midnight and eCash addresses derive from the phrase in the browser, checked against the Causewaybay Wallet's vectors. The backend's SQLite keeps only public choices: the active coin, recent picks, added tokens and receive addresses, so a later sign-in, even by private key alone, opens the wallet as it was left. Balances are read straight from each chain's public endpoint (Cronos and Arbitrum RPC, PublicNode for Solana mainnet because api.mainnet-beta refuses browsers, Chronik, the Midnight indexer); Koios sends no CORS headers, so Cardano is read through a narrow backend relay of two read-only queries for a public address. The card is a three.js scene per chain with Grok-generated artwork (`node scripts/generate-wallet-art.mjs <name>` with `XAI_API_KEY`), and every tween is motion.dev on an exponential in/out ease.
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
