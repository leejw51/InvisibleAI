# Security policy

Invisible AI runs on your own Mac and listens on loopback by default and holds your provider keys, your notes, and screenshots of your screen. Please report vulnerabilities privately.

## Reporting

Use GitHub's private vulnerability reporting on this repository (Security → Report a vulnerability). If that is unavailable, open an issue titled "Security contact request" without details and a maintainer will reply with a private channel. Please include the affected component, reproduction steps, and the impact you see. You will get an acknowledgement within a week.

Please do not file public issues for anything that could expose another user's data or a provider key.

## What the design promises

- **Local by default.** A network listener requires an explicit `APP_TOKEN`. Unauthenticated local-browser access accepts only loopback hostnames/IPs, preventing DNS rebinding through arbitrary Host headers.
- **Capture credential.** When `APP_TOKEN` is set, capture connections must present it (or the generated 0600 `capture-token` the same Mac's CLI shares automatically). Without `APP_TOKEN`, the capture transport is not token-checked, so keep the listener on loopback or a trusted VPN. Each client also keeps a private `device-key`; a different key cannot replace its registered ID or owner. Reinstalling a client requires preserving that key or enrolling a new ID.
- **Hybrid tagging stays local.** Background Wiki analysis never sends a Hybrid account's notes to a cloud model. Cloud AI mode explicitly permits cloud tagging.
- **Checkpoint isolation.** Generated code cannot write the project's `.git` directory. Git checkpoints disable hooks and fsmonitor and run inside a separate filesystem/network sandbox with bounded runtime and output, including for existing repository filters.

- **Keys stay on the backend.** Provider keys are read from the backend's environment or `.env` only. They are never returned by an API, saved to SQLite or JSONL, sent to the browser or capture client, or passed to the Codex and Claude Code child processes.
- **Wallet isolation.** Every room, memo, file, capture, and search result belongs to the signed-in wallet. Admins approve accounts and manage devices but cannot read another wallet's content through the API. Reports that break this boundary are the highest priority.
- **Bounded chat tools.** The chat agent cannot run arbitrary shell commands. Besides explicitly enabled sandboxed skills, it can start `rustc` for a library-only, standard-library compile check, under the macOS `sandbox-exec` profile with no network, writes limited to the temporary build directory, and reads limited to an allow-list of the toolchain and system libraries. Compiler diagnostics are bounded.
- **Programs run only inside a project sandbox.** The Coding page builds and runs a project's program (and a published skill) under a `sandbox-exec` profile: no network, writes only inside that project's folder, reads only from the toolchains, the system and that folder, output bounded to 64 KB per stream, a 60-second run limit (300 for builds, 180 for tests, 600 for a dependency fetch) enforced on the whole process group. The dependency fetch (`cargo fetch`, `go mod download`, `pip install` into a project virtualenv, `npm install`) is the one step that gets the network, with the same write confinement, so a package's install script can touch nothing outside the folder. Symbolic links inside a project are refused by the file API. Codex and Claude Code, when chosen as the AI mode, work inside a profile with the same read allow-list plus their own state folders (`~/.claude`, `~/.codex`) and network for their services; the rest of the home folder is unreadable to them. A `run_skill` tool call from a chat runs a skill in that sandbox with the model's text on stdin and nothing else.
- **Mac automation is narrow.** The capture client reads and appends to TextEdit, Apple Notes, and a terminal tab, opens validated http(s) addresses in Safari, and computes arithmetic. Note text never triggers an action; the backend validates every action before the client runs it.
- **Untrusted content.** Screen text, files, web results, and earlier notes are treated as data in the prompts, not as instructions. Prompt-injection reports that lead to an external action are in scope.

## Out of scope

- Attacks that require the backend Mac's local account or its `.env` file.
- Denial of service against a single-household deployment.
- Issues in third-party model providers.
