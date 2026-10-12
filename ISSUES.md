# Public-release work plan

Findings from the 2026-10-08 review of the coding agent, the menu and flows, and the architecture, with the fixes being applied for the open-source release. Items are checked off as they land in the working tree. Status as of 2026-10-08: everything below is in the working tree; the native Rust suite passes apart from one baseline failure, and the browser suite result is recorded at the end.

## Decisions

- **Product name is Invisible AI.** The crate and repository prefix stays `causewaybay` and the README says so once. The system prompt, UI, and docs no longer say "Causewaybay Bot".
- **UI routes match their labels.** `/dashboard` is the Dashboard (statistics, previously `/status`) and `/devices` is Devices (previously `/dashboard`). `/status` stays as an alias of the Dashboard. The JSON API keeps `/api/dashboard` and `/api/status` unchanged and gains `/api/devices` as an alias of `/api/dashboard`.
- **Backend sessions are called Autonomous AI** in the menu, UI, and docs. The page moves to `/autonomous` with `/cloud` kept as an alias. The JSON API keeps `/api/invisible/sessions` for protocol stability; PROTOCOL.md notes the name.
- **One home for each control.** The Autonomous AI page owns the AI processing mode (On-device / Hybrid / Cloud AI / Codex / Claude Code). The New thought workspace panel shows the current mode read-only with a link. Backend settings keeps only admin server-level provider configuration. Chat's AI & tools panel is the only place to pick a provider and model. The chat checkbox "Allow file creation & compilation" is the only agent control; the Coding agent page and the composer toggle go away.
- **Pinned is a Library filter**, not a sidebar entry. The sidebar's duplicate "Backend settings" entry goes; the header gear stays.
- **UI assets** that need to be new or regenerated are produced with Grok image generation (`XAI_API_KEY`) and keep their prompt next to the image, like the existing `frontend/assets/*.prompt.txt` files.

## Coding agent (server)

- [x] Blank tool arguments are treated as `{}` in `agent::execute` as well as in the stream accumulator (which already did this); covered by tests for all three provider formats.
- [x] A tool name that is not enabled returns `{"error":…}` to the model instead of failing the whole turn; `agent::execute` receives the allowed set.
- [x] Reaching the round or call limit runs one final turn without tools and finishes normally instead of marking the memo failed.
- [x] `list_files` no longer exposes backend storage paths and is capped.
- [x] `compile_code` is offered only when the backend can compile (macOS with rustup); the compile tests are gated the same way.
- [x] The compiler sandbox uses an allow-list of readable roots (toolchain, temp dir, system libraries, SDK) instead of a deny list; a byte-exfiltration probe through diagnostics is part of the tests.
- [x] New `read_note` tool returns a full memo by id.
- [x] `read_file` extracts text from PDF, Office, and OpenDocument files through `documents::extract` and supports `offset`/`limit`.
- [x] Tool outputs are trimmed and a running budget bounds the context across a turn.
- [x] `save_memo` accepts `tags`.
- [x] `write_file` stores the MIME type derived from the filename.
- [x] `fetch_url` tool: https only, public addresses only (checked again on each redirect), 1 MB and 20 s caps, HTML reduced to text.
- [x] Web search calls are recorded in the usage ledger (tokens explicitly unknown).
- [x] Gemini function declarations omit `parameters` for parameterless tools.
- [x] Tests: `definitions_for(false)` excludes write tools, blank arguments, `edit_file` exact-once, and the accumulator's parameterless tool call per provider.

## Menu and flow (frontend)

- [x] Routes renamed as decided; the SPA path list and `location_page` agree.
- [x] AI mode selector lives only on the Autonomous AI page; provider and model only in chat's AI & tools.
- [x] Coding agent page, composer Agent toggle, and `/agent` route removed; `/ask` route and the uncompiled `ask.rs` removed.
- [x] 2026-10-10: the AI choice has one shared picker (`ai_settings.rs`): the **AI settings** page is its home, and Chat and Coding embed the same control, so the Autonomous AI page's mode cards are no longer the only place to change it. The pick lives in browser local storage and is mirrored to the account preference.
- [x] 2026-10-10: a project-centric **Coding** page (`/coding`) and a **Skills** page (`/skills`) were added behind the More tools fold. They are not the removed chat-side agent page: the chat keeps its one checkbox, Coding owns sandboxed per-project folders that build and run, and Skills lists the programs published from them.
- [x] Pinned becomes a Library filter; sidebar Backend settings entry removed; every sidebar entry has a distinct icon.
- [x] Chat honors the configured capture mode (entire display or region) instead of hardcoding the full screen.
- [x] ⌘Enter sends in chat as well.
- [x] When no provider is enabled, the composer says the thought is kept without asking AI.
- [x] Product name and Autonomous AI label applied to all UI strings and translations.
- [x] Playwright specs updated for the new routes and labels (full browser run pending in this checkout).

## Architecture and release hygiene

- [x] `think.rs` holds the answer loop and its prompts; `routes.rs` builds the Router. `lib.rs` keeps state and handlers.
- [x] README reduced to one screen (what it is, run, privacy model, links); the full guide moves to `docs/guide.md`.
- [x] PLAN.md architecture diagram lists every provider kind (API, Apple, MLX, Codex, Claude Code) and the current pages.
- [x] README states the single-SQLite-connection design and its one-household ceiling.
- [x] TODO.md references sibling projects by name, not by local absolute paths.
- [x] LICENSE (MIT, matching Cargo.toml), CONTRIBUTING.md, and SECURITY.md added.
- [x] GitHub Actions workflow: fmt, clippy, build, and tests on macOS; a Linux job only for the parts that compile there.
- [x] Git history scanned for provider keys (clean as of 2026-10-08).

## Found while fixing

- Hashtags written inside a kept document (for example `#launch` in an attached Markdown file) were never extracted, so `tests/keep.rs` failed on the untouched baseline. Kept files now contribute their hashtags like typed text.
- The compiler sandbox allow-list must include `(literal "/")`: a process cannot start without reading the root directory entry.
- The wallet login message still says "Causewaybay Bot login" on purpose: it is part of the signed challenge and changing it would invalidate the compatibility vectors in `crates/shared/src/identity.rs`.
- Backend settings keeps a *default AI mode for new users* selector (server-level admin configuration), relabeled so it does not read as a per-user picker.
- `rustup which rustc` blocks on a toolchain download when the home directory has no toolchain (rustup auto-install), which hung backend startup in the storage test's synthetic home. The compiler check now only looks at the file system; the per-compile lookup sets `RUSTUP_AUTO_INSTALL=0` and times out after ten seconds.
- Phones hid the header settings gear, which became the only Backend settings entry; the gear now stays visible on phones and the decorative avatar is hidden instead.
- Choosing a provider in chat updated the account first and the UI only after the round trip, so a model typed right after the choice was saved under the old provider. The choice now applies immediately and reverts if the account update fails.
- Yew re-applies an input's `value` from component state on every render, so a render that landed while a model name was being typed (the account update returning, a room event) wiped the typed text before its change event fired. The model field now updates state on every keystroke; the messenger spec that reloads after typing a model exposed this about once in twenty runs.
- The chat "Connect a device" hint and the "Set up this Mac" links still pointed at the old devices page id after the route swap.
- `tests/thumbnails.rs` expects `422` for the thumbnail of a non-image file, but every `anyhow` error maps to `400` and nothing in the server produces `422`; the test fails the same way on the untouched baseline (checked in a clean `HEAD` worktree) and is left as is. `tests/multimodal.rs` waits four seconds for the demo pipeline and times out when the whole suite runs in parallel; it passes alone.

## Verification (2026-10-08)

- `cargo fmt --check` and `make check` (clippy with `-D warnings`, server and WASM frontend) pass.
- Native tests: everything passes except `tests/thumbnails.rs`, which fails identically on the untouched `HEAD`.
- Full browser suite: 364 passed, 2 failed. One is `capture-image.spec`, which fails on the untouched baseline. The other was the messenger typed-model race above; after the fix, the messenger, local-AI and AI-policy specs passed 57 of 57 across three repeats.
- An earlier full run lost the shared test server late in the run, so 49 specs timed out on a blank page. The rerun, with a health watchdog attached, saw no stall. The cause was not found.

## Tags on pictures and clips, and a shorter menu (2026-10-09)

Found while using the app on-device: a picture or a clip added to the Wiki sometimes never got its tags, and even when it did there was no place but the chat to change them. The menu also listed twelve destinations before any had been tried.

### Decisions

- **A picture or a clip is always kept.** The analysis used to let a small on-device model's `keep: false` drop a photo it knew nothing about but the backend's own note (`[Picture kept. …]`). Media the user added on purpose joins the Wiki whatever the model says; text still follows the model, then the heuristic.
- **The backend's notes are not tag material.** `Attached file: x (ID …)`, `[Picture kept …]`, `Seen in the picture:` and the like are stripped before tagging (`wiki::analysis_text`), so neither the model nor the word-frequency fallback files a photo under `attached` or `picture`. File names and Apple Vision's labels stay.
- **No message stays "Tagging…" forever.** A completed message with no analysis row (the backend stopped, or the on-device worker failed, before its analysis ran) is tagged by the startup repair; an unreadable preference or an unconfigured tag model falls through to the next model or the heuristic instead of ending the analysis. The chat offers **Add tags** while the analysis is still pending.
- **Media kept without a word skips the tidy step.** It has nothing to tidy, and on-device the single model lane was being spent on the backend's own notes before tagging could start.
- **Tags are edited where the media is.** The Images and Videos pages and the Wiki's reader carry the same inline editor as the chat (`crates/frontend/src/tags.rs`, `PATCH /api/memos/{id}` with `tags`); `/api/images` and `/api/videos` return each file's `memo_id` and `tags`. The Wiki already refreshes open pages on the backend's `wiki.updated` event, so an edit anywhere shows everywhere.
- **Four places to go.** The sidebar lists Chat, Wiki, Autonomous AI and Devices; Library, Files, Images, Videos, Code, Dashboard, Multi-device capture and Quiz sit behind the one "More tools" fold, which opens by itself on those pages. "Chat rooms" is "Chat" and "Memo library" is "Library". The sidebar's "Your hashtags" list went: the Wiki is the home of tags. Every route and URL is unchanged.

### Proposed next steps (not done)

- Fold Images, Videos and Files into the Wiki's type filters (photo, video, note) with upload on those filters, and Code into the Wiki's code filter, so the "More tools" fold shrinks to Dashboard and the two tools. The Library then becomes the Wiki's search.
- Move Dashboard under Devices as a tab.
- Replace "Assistance settings" with "Settings" and put Account details, Delete all and Sign out inside it.
