# Invisible AI: local processing and notes

The implementation combines the Rust backend/capture CLI with a notes workspace, configurable Wiki, and personal assistance settings. The frontend has Auto, Capture, backend-persisted AI modes, Hybrid approval, and measured usage. Auto is **off by default**. AI research synthesis, mobile capture adapters, PocketSkynet encryption, and general desktop automation remain in [TODO.md](../TODO.md).

## Use it

Two ways to use the AI:

- **New thought and chat**: press **Capture** (or Multi Capture) and the screen is captured and answered on the spot. One click, no typed prompt required.
- **Autonomous mode (Invisible AI)**: write in Apple Notes or TextEdit on the capture Mac and keep working. The backend detects the change, lets the local model decide whether the text asks for help, uses cloud AI, Codex or Claude Code only when that is needed (per the chosen mode), and writes the answer below your text. No AI app has to be opened; an iPhone or iPad can also send requests to the Mac from the Autonomous AI page.


1. Start the backend and updated capture CLI using the existing Makefile commands. The web UI is optional; sessions can also be created and started through the API. In wallet mode, enroll the capture Mac to the same wallet that will enable Auto. A shared capture node does not grant permission to write notes.
2. Admin: open Backend settings and choose one of the large **On-device**, **Hybrid**, or **Cloud AI** cards. Save to apply the workspace default. Configure an enabled provider and set the two polling intervals (2–300 seconds each): the **on-device polling interval** (default 5 seconds) used by On-device, Hybrid, Codex and Claude Code, where polling runs on the local model and costs nothing, and the **Cloud AI polling interval** (default 60 seconds) used by Cloud AI mode, where a change may cost a cloud call. Hybrid is the default mode whenever a local model is ready. In Cloud AI mode both the autonomous run and manual capture polling are floored at the admin's **Cloud AI minimum interval** (60 seconds by default, 2–300) whatever the session asked for, an unchanged screenshot is discarded before any model runs, and a note answer whose screenshot is identical to the previous round's skips the cloud call ("Screen unchanged · cloud AI skipped"). Provider credentials and executable paths belong to the backend.
3. User: select On-device, Hybrid, or Cloud AI in the new workspace. The selection and local/cloud providers are stored in SQLite under the user's wallet and restored on other browsers. Existing accounts without an explicit selection initially use the existing backend/browser defaults.
4. Open a **plain text** document in TextEdit on the capture Mac. Select that Mac in the frontend and press **Start TextEdit Auto**. If no session is selected, this creates and starts a backend session automatically. Existing sessions can be selected below the TextEdit controls or managed in **Autonomous AI**. Grant macOS Automation permission for the CLI's host app to control TextEdit when prompted. Screen Recording permission is separate and required for Capture.
5. Write an ordinary question, topic, or idea. After it settles, the backend generates useful help and the CLI adds a marked answer. Continue writing **below** `[End Invisible AI]` for the next answer. Capture takes one screenshot without enabling Auto.

If you type while AI is answering, Auto preserves your edits and waits for the updated note to settle before answering again. The unused reply remains in the same chatroom's history. Switching or closing the selected document stops Auto so it cannot write to a different note. TextEdit errors identify missing permission, a changed document, expiration, timeout, or a failed edit, with translated recovery instructions. On an iPad, Automation permission must be allowed on the **capture Mac**, for the app that launched its CLI. macOS prompts on the first request; after a denial, enable access under **System Settings → Privacy & Security → Automation** and start Auto again.

In Cloud, **Start TextEdit replies / Stop TextEdit replies** control this optional feature. Screen **Capture now** and **Start polling / Stop polling** work independently, including while TextEdit replies are off.

The capture Mac runs no LLM. It communicates with the trusted backend, which performs OCR, transcription, inference, and persistence. Closing the browser, logging out, or letting a browser login expire leaves enabled notes assistance running. Session IDs, device bindings, source rooms, and status are saved in SQLite. Open another UI to list and manage them. Disconnecting the CLI or restarting the backend stops Auto; the saved session remains available and requires an explicit start. Account revocation, device ownership changes, and source erasure also stop the worker. Changing the user mode invalidates pending cloud approvals. If it changes during inference, the old result is discarded and Auto reports that it needs to be enabled again.

The admin's mode and local/cloud provider choices are saved in backend JSONL and SQLite. Users without a personal choice inherit this default; existing personal choices remain intact. When no admin default is saved either, the backend and the browser compute the same fallback from real provider readiness: **Hybrid** when a local and a cloud provider are both ready, **On-device** when only a local provider is ready or Local-only is enforced, otherwise **Cloud AI** with the selected provider. Demo mode does not count as readiness. Hybrid Auto asks before sending each note to the selected cloud provider when cloud confirmation is enabled; when it is disabled, cloud help is automatic. Text such as `@cloud` never bypasses confirmation. Hybrid Chat and remote requests stay on-device. Coding can fall back to cloud only when confirmation is disabled. The separate **Local-only mode** safeguard blocks cloud use for everyone and requires an On-device default. The Backend settings dialog keeps only server-level runtime and provider configuration; an admin default mode can still be saved through `PATCH /api/settings` (`default_ai`), and each account chooses its own mode on the Autonomous AI page.

Editing a provider or its model does not change the selected processing mode. In Hybrid, configuring a cloud provider keeps local-first execution and the selected cloud-confirmation policy. While Local-only is enabled, the API and UI expose an effective On-device policy with an enabled local provider; the saved personal choice is restored when the restriction is lifted. Apple/MLX model fields in chat are read-only and follow the backend's installed model, including updates from another admin browser. Old browser model caches cannot override them. Cloud-mode chat and Overview provider overrides remain browser preferences and survive reload; the workspace mode and notes providers remain backend preferences. Overview and chat both lock provider selection to the account policy in On-device and Hybrid modes.

## If Capture cannot read the Mac screen

Screen capture requires **Screen & System Audio Recording** permission on the capture Mac. This is separate from **Automation → TextEdit**, which controls note reading/writing. The CLI validates active displays and attempts the native screenshot before checking/requesting permission after a failure. A successful screenshot is not rejected by the CLI's own permission precheck. The first failed Capture request can show the macOS permission prompt; starting the CLI alone and fixture tests do not prompt. Allow the app that launched the CLI (for example Terminal, iTerm or Codex), then restart that app and the CLI as macOS requests. See [Apple's screen recording permission guide](https://support.apple.com/en-gb/guide/mac-help/mchl592e5686/mac).

An unavailable display number is reported separately, with the available number range. When permission is granted but macOS still cannot capture, unlock the Mac and restart the CLI from its signed-in desktop. `make start3` loads a changed CLI build while retaining the saved connection. To restart an unchanged managed client after a permission change, run `make stop3` followed by `make start3`.

`make test-native-screen` tests the actual Capture button with a live Mac screen, the real Rust CLI, Apple OCR, invalid-display failure and recovery on Chromium and iPad/iPhone WebKit emulation. It uses isolated data and a deterministic loopback AI endpoint, without paid cloud requests. Run it on an unlocked Mac with screen recording allowed; it temporarily opens a dedicated test browser.

## Backend-owned sessions

The main menu lists four places: **Chat**, **Wiki** (organized saved knowledge), **Autonomous AI** (AI modes and backend sessions, at `/autonomous`; the older `/cloud` address still opens it) and **Devices** (`/devices`, connections). Behind the **More tools** fold, which opens by itself on those pages, sit **Library** (original saved notes, with a Pinned filter), **Files / Images / Videos** (stored assets; pictures and clips show their Wiki tags, editable in place), **Code** (`/code`, every kept function with its Input and Output, ready to copy), **Dashboard** (`/dashboard`, workspace status), **Multi-device capture** and **Quiz playground**. **Assistance settings** controls personal Wiki/TextEdit behavior; the admin-only header gear opens **Backend settings** for runtimes and server defaults. The AI processing mode is chosen only on the Autonomous AI page. The menu does not require cloud inference: On-device sessions are managed there too. Sidebar search opens the full memo library and closes mobile navigation before focusing the search field.

Open **Autonomous AI** (자율 AI), or `/autonomous`, for the same chatrooms shown under **Chat**. Each card opens its chat history, with capture thumbnails and memo counts. **New session** creates a chatroom with an optional capture computer; memos and chat work without one. The first capture or TextEdit start binds one computer to the room. TextEdit controls are optional and keep their enrollment, AI-provider, and approval checks. Screenshot polling is an optional chatroom control; a manual Capture resets the next polling deadline and unchanged screenshots are discarded. Saved replies identify their actual local/cloud provider and model. Existing room-owned TextEdit memos are imported into chat history on backend startup.

The Autonomous AI page is a session manager for every AI mode. Its large **On-device / Hybrid / Cloud AI** cards change the processing policy for all sessions owned by your account. A choice is saved immediately through the backend preferences API, preserving your configured local and cloud providers; the selected card changes only after a successful save. Reloading or opening another browser restores the same policy, and an already-open Autonomous AI page picks up remote changes. Save failures keep the previous selection and show an error. Admin Local-only restrictions disable Hybrid and Cloud AI. Changing the policy revokes pending cloud approvals; it does not create or start a session. Opening the page alone leaves the policy unchanged. New thought shows the saved mode read-only in its workspace panel with a link to the Autonomous AI page, which is the only place that changes it.

When a mode change cancels a pending approval, the session displays **AI settings changed. Write a new note to continue.** This status is saved with the session. The unchanged note is not resubmitted; the next new note uses the updated policy. If background refresh fails, Autonomous AI displays **Waiting for backend** instead of claiming the displayed settings are synchronized.

Cloud session cards manage screen capture using the same chatroom API. **Capture now** saves to that room; **Start polling / Stop polling** controls the backend loop and its interval. A session without a bound computer can select one before its first capture. Once bound, it keeps that computer. Manual capture while polling resets the next interval, unchanged frames are discarded, and closing the browser does not stop the backend. **Open chat history** shows the same memos, thumbnails, and AI replies. Chatrooms keep their manual Capture button and link active background capture to **Manage in Cloud**; they have no polling setup or Start/Stop controls.

The workspace separates **Capture** (one screenshot and an answer in this web chat), **TextEdit automatic replies** (answers in the same TextEdit document), and **AI processing location** (where inference runs). Capture appears once in the fixed toolbar. Auto uses explicit **Start TextEdit Auto / Stop TextEdit Auto** labels and an independent Running/Off status. The Start/Stop button sits in the card heading, beside the status. On viewports at most 760 CSS pixels tall, the decorative welcome banner is hidden so the action stays above the fixed composer. The TextEdit panel names its target Mac and polling interval. TextEdit and manual capture use the same room and capture computer. Selecting a TextEdit session also selects its chatroom and restores its context. Mode changes do not start Auto, and manual Capture does not require an Auto session. **How to read the capture** identifies the separate OCR/image input setting; Apple and MLX currently use OCR text and disable image analysis.

The workspace's Auto button checks the saved session's bound Mac and selected AI provider before starting, and explains offline, missing setup, or competing-session conditions. An already-running session remains stoppable. Workspace, usage, Wiki controls and assistance settings follow all seven interface languages without remounting their components, so changing language preserves unsent text, Wiki searches and unsaved settings. Saved source text and Wiki document contents remain as stored.

On wallet-enabled backends, **shared screen capture does not grant permission to read or write TextEdit**. New thought and Cloud use the backend's account-scoped `textedit` device state (`ready`, `needs_enrollment`, `offline`, `update_required`, or `unavailable`). Auto stays disabled with a localized setup link until the account and live CLI are ready; manual Capture remains available on shared devices. Cloud device permissions refresh every two seconds, including reconnects and ownership changes. The backend rechecks authorization when starting and throughout execution.

To enroll a shared Mac, open **Devices**, stop its existing capture CLI, and run the displayed account command on that Mac. For the managed development launcher, `make start2 OWNER_WALLET=0xYOUR_ACCOUNT_ADDRESS` (or `make start3` with the same argument) now passes the wallet to the CLI and restarts a running shared client when necessary. The CLI saves the owner in `~/.causewaybaybotclient/client.jsonl`; later `make start2` calls preserve it. Enrollment does not start Auto or change the AI processing mode.

The backend owns session creation, polling, inference, approved input, and persistence. The frontend only lists sessions, displays their state, changes settings, and sends start/stop/approval requests. Its polling timer refreshes the display; it never schedules AI work. The capture CLI remains necessary to read or write the target Mac's TextEdit document.

`POST /api/invisible/sessions` reuses the chatroom creation logic with a server-generated ID, or accepts an existing owned `room_id`. The client is optional until capture or TextEdit starts. TextEdit replies and saved memos use the same chat messages, context, ownership, and erasure logic. Creating or selecting a room does not start a worker unless the request sets `start: true`; the Autonomous AI page's **Create and start** button does exactly that for the chosen Mac and reports a refused start on the new card instead of failing the creation. New sessions open with a suggested random name that can be edited or re-rolled. A session uses one fixed capture computer, and one TextEdit worker may run on a Mac at a time. Choosing a new thought creates a separate session; the previous worker remains controllable from Autonomous AI or by selecting its room again.

| Method | Endpoint | Behavior |
| --- | --- | --- |
| POST | `/api/invisible/sessions` | Create using optional `client_id`, `title`, `room_id`, and `start` |
| GET | `/api/invisible/sessions` | List the signed-in account's sessions and live status |
| GET | `/api/invisible/sessions/{id}` | Read one session |
| POST | `/api/invisible/sessions/{id}/start` | Start its backend worker |
| POST | `/api/invisible/sessions/{id}/stop` | Stop its worker and revoke pending actions |
| POST | `/api/invisible/sessions/{id}/approval` | Submit the existing single-use `id` and `approve` decision |

For a local backend without authentication, a headless controller can create a session directly:

```sh
curl -sS http://127.0.0.1:3000/api/invisible/sessions \
  -H 'Content-Type: application/json' \
  -d '{"client_id":"your-connected-mac-id","title":"Daily notes"}'
# Use the ID returned above to start or stop the worker:
curl -sS -X POST http://127.0.0.1:3000/api/invisible/sessions/SESSION_ID/start
curl -sS -X POST http://127.0.0.1:3000/api/invisible/sessions/SESSION_ID/stop
```

#### Autonomous AI run and manual capture

Each session card leads with **Autonomous AI run** (자율 AI 실행), the session's main job: watch the selected TextEdit note on the bound Mac and write each answer below the user's text. The section carries its own settings: **Checks every** (a per-session interval saved through `PATCH /api/invisible/sessions/{id}`, applied live, with a one-tap return to the admin default), **Typing pause** and **Answer length** (the wallet-wide assistance settings, editable here; changing them stops a running session as before). While the worker is still at "Connecting to TextEdit…" the card counts the seconds and, after eight, explains that TextEdit on the capture Mac is slow to respond and that the attempt gives up after 30 seconds. The large Start/Stop button sits at the bottom of this section. Start acts immediately: the snapshot taken while connecting is the first read, and no typing pause applies to it, so a note that already holds a question is answered as soon as the model replies. Later reads keep the typing pause.

#### Codex and Claude Code modes

Beside On-device, Hybrid and Cloud AI, the mode cards offer **Codex** and **Claude Code**. These run the `codex` and `claude` command-line tools on the backend Mac as non-interactive subprocesses, signed in with that Mac's own login, so answers are billed to the Codex or Claude subscription instead of an API key. The backend finds the tools on `PATH`, `~/.local/bin`, `~/.npm-global/bin`, `~/.cargo/bin`, `/opt/homebrew/bin` or `/usr/local/bin` (override with `CODEX_BIN` / `CLAUDE_BIN`; restart the backend after installing). Each request runs in an empty scratch directory with every tool disabled (`claude -p --tools "" --no-session-persistence`, `codex exec -s read-only --ephemeral`), gets the conversation as one text prompt, and returns the final answer with the token counts the tool reports; estimated cost is shown as $0 because the subscription pays. `ANTHROPIC_API_KEY` and `OPENAI_API_KEY` are removed from the child environment so the tools never fall back to API billing. Prompts still leave the Mac for OpenAI or Anthropic, so Local-only disables both modes, screenshots are read with Apple Vision rather than sent as images, and the chat shows them as cloud answers.

#### No change, no model; the local model gates expensive calls

A note that has not changed since the last check never reaches any model: the worker compares the exact document text, and manual capture polling discards unchanged screens before OCR or AI. When a note does change in **Cloud AI**, **Codex** or **Claude Code** mode, the local model (Apple or MLX, whichever is enabled) goes first and makes a three-way call: it replies `[NO_REQUEST]` for notes to self, lists, drafts or statements (the card shows "No request found · cloud AI skipped", nothing is written, and the check leaves no chat message); it answers simple requests itself, which are written as `[Invisible AI]` at no cloud cost; or it replies `[CLOUD_HELP]` for web research, long reasoning, code generation or expertise beyond it, and only then does the mode's expensive model run and append `[Invisible AI · cloud]`. Writing **@cloud**, **@codex**, **@claude**, "use cloud" or "클라우드로" in the note (or in a phone request) skips the local check and goes straight to the expensive model; in Hybrid it goes to the cloud provider without the approval step. The **Cost at a glance** banner at the top of Cloud shows cloud spend today and all time, local LLM calls, and cloud calls avoided (no request, answered locally, unchanged screens). The check is **Let the local LLM decide when cloud AI is needed** under the mode cards (per wallet, default on). It never applies to Hybrid, whose local pass already answers first, to On-device, or to requests typed on another device, which are explicit.

#### Hybrid without the approval step

Hybrid is local-first: the local model answers, and when it replies `[CLOUD_HELP]` the note goes to the configured cloud provider. By default that happens immediately: the card shows "Thinking with cloud help…" with the cloud provider, the answer is appended as `[Invisible AI · cloud]`, and nothing waits for a tap. Only the note text is sent on that pass (never the screenshot). The approval dialog still exists for people who want it: turn on **Ask before each cloud request** under the mode cards (saved per wallet with the assistance settings, applied to the next note without restarting), and Hybrid shows the exact note and provider and waits, as before.

#### Watched app, Apple Notes, and remote control from an iPhone or iPad

Each session's **Watched app** defaults to **Detect automatically**: the CLI asks macOS which app is in front (no screen capture, no extra permission) and reads the note in TextEdit or Apple Notes accordingly, trying the other app when the front one has no note open. Every poll repeats the check, so bringing a different note or app to the front simply moves the run to that note ("Following the note in front."); earlier answers are not repeated. Safari or other apps in front leave the run on the last note app that has one open. A session can instead be pinned, while stopped, to **TextEdit note**, **Apple Notes (메모)**, **Terminal (ai- …)**, or **Remote prompts only**; a pinned choice is always used as selected.

**Terminal** (Terminal.app or iTerm2, whichever is in front) works differently from a note: the recent entered lines (never the line still being typed) are read by the local model, which decides whether the user addressed the assistant, for example `ai, what's bitcoin?`, `ai- …` or `invisible ai …`, rather than matching fixed patterns. Commands, output and prompts are judged not to be requests and cost nothing beyond that local check. A simple request is answered by the local model itself; a hard one is handed to the mode's main model; `@cloud`, `@codex` or `@claude` in the line forces that model directly. The CLI shows the answer by writing it to that tab's TTY device as display output (`/dev/ttys…`, reported by Terminal or iTerm2), so `[Invisible AI]…[End Invisible AI]` appears in place without any command being sent to the shell and without anything being typed into the prompt. The agent never runs a command on the user's behalf; running whatever the answer suggests stays entirely with the user. Terminal automation needs the same Automation consent as the note apps. A request still being typed (no line after it yet) is never answered early.

**Read from the screen** (on by default) reads the note as it is displayed: each time a new note or a remote request is answered, the backend takes one screenshot of the capture Mac through the normal capture path and attaches it to the chat message. Local AI (On-device, and the local pass of Hybrid) reads it with Apple Vision on the backend and nothing leaves the Mac; Cloud AI receives the image itself when the workspace is in Image mode, or its OCR text in Text mode. The exact note text from TextEdit or Notes is still what detects changes and anchors the appended answer, and the approved-cloud pass of Hybrid sends only the note text shown in the approval. A failed capture (for example, missing Screen Recording permission) is logged as "Screenshot unavailable · note text used" and the answer proceeds from the text. With Apple Notes the CLI reads the single note selected in the Notes app on the capture Mac and appends each answer below it, using the same change detection as TextEdit; Notes needs its own Automation consent for the CLI's host app, and the error messages name Notes. With **Remote prompts only** no note is read: the run just waits for requests.

**Remote control** sits at the bottom of the autonomous section so the Mac can be driven from the iPhone or iPad that is showing the Autonomous AI page. While the run is active, type a request and **Send to Mac** (Enter sends; Shift+Enter inserts a line). Explicit commands never touch a model: `open youtube.com` or a pasted address opens Safari on the Mac, `search bitcoin price` (or 검색 …) opens a DuckDuckGo search in Safari, and `calc 12*34`, `계산 …`, or plain arithmetic such as `12 * 34 = ?` is computed on the backend. Anything else is answered by the configured provider, which in On-device mode is the local Apple or MLX model; the model may ask for at most three Safari or calculator actions through fixed markers, and the backend validates every address (http or https only) before the CLI runs `/usr/bin/open -a Safari`. Text found in a note never triggers an action. The card shows how many requests are waiting, the last request, the Mac action taken, and the reply; the full exchange is in the chatroom (**Mac action** rows for requests that needed no model) and is appended to the watched note as `[Invisible AI · remote]`. Hybrid sessions answer remote requests locally without a cloud approval; because the reply is appended to the note, a pending Hybrid approval for that note expires, as it would after any edit. Only http(s) addresses in Safari, arithmetic, and the two note apps are automated; Terminal, shell commands, and other applications remain out of scope.

**Manual capture (add-on)** (수동 캡처) is collapsed below the Progress panel. **Capture now** saves one screenshot to the chatroom; **Optional polling** inside it repeats that on a timer. Text-mode captures whose recognized text matches the room's previous capture are discarded before any model runs, so an unchanged screen never triggers the local or cloud AI.

#### Processing stats

Above the session list, **Processing stats** shows, for the last 24 hours and all time: OCR runs, local LLM requests (with total wait and per-provider counts), cloud AI requests (with tokens and estimated cost), and captures skipped because the screen did not change. The strip is part of the page's two-second poll, so it refreshes after every AI job, shows how many requests are running right now, and otherwise shows the time of the newest measured inference. Each finished chat reply also prints its own wait, tokens and estimated cost next to **Answered by**; providers report tokens rather than prices, so a per-call cost is an estimate from the dated list-price table and is shown as unknown when tokens or prices are missing.

#### Session progress

Each Cloud session card has a **Progress** panel so nobody has to guess what the backend is doing or which model is doing it:

- A live row shows the current step with an animated indicator while anything runs. When the backend is waiting on a model, the row names the provider and model, labels it **Local LLM** or **Cloud AI**, and counts the wait in seconds; the count combines the backend-measured wait with the time since the browser's last poll, so clock skew between devices does not distort it.
- A timeline lists every recorded step newest first: TextEdit or Notes worker stages, remote request stages, capture polling events (started, stopped, manual capture queued, unchanged screenshot discarded), and chat/capture job stages. AI steps carry the provider and model that actually ran. The backend keeps the last 200 steps per session and shows 40; clearing or deleting the chatroom removes them.
- **Recent AI requests** comes from the wallet-scoped usage ledger joined to the session's messages: time, LLM, Local/Cloud, total wait and time to first output, provider-reported tokens, estimated cost, and state (running, complete, error, interrupted). Demo mode records measured turns with unknown tokens.
- Totals cover requests, total wait, tokens and **Estimated cloud cost**. Cost is computed only for cloud requests whose model has a listed price and whose provider reported both token counts; everything else is shown as unknown rather than zero. Local requests show **No cloud cost**. The estimate uses list prices dated 2026-10-06 (Claude Sonnet 4.5 $3/$15, GPT-4.1 mini $0.40/$1.60, Grok 4 $3/$15, Gemini 2.5 Flash $0.30/$2.50 per million input/output tokens) stored in `crates/server/src/activity.rs`, is labelled with that date, and is never a provider bill. No price or billing request is made at runtime.

Stage labels are fixed strings, plus the capture device's own name for capture steps; note text, prompts, answers and attachment names never enter the progress table.

The chat view shows the same facts per reply. While a reply is running, an animated waiting row names the stage and, once a model is working, labels it **Local LLM** or **Cloud AI** with the provider and model and counts the seconds waited. Every finished AI reply carries an **Answered by** line stating whether a local LLM or cloud AI produced it, with the provider and model that actually ran.

Configured authentication is still required for management requests. A worker retains the account's delegated authority, not the initiating HTTP headers or browser cookie; current account approval and device access are checked throughout execution. Existing room-based notes APIs remain supported and register sessions in the same collection. Continuous screen capture also uses backend account authority rather than retaining browser login credentials.

Backend restart restores metadata and reports previously running sessions as stopped. It does not replay input or cloud approvals. Hybrid work waits for an explicit approval if no UI or API controller is attached. Clearing the source room cancels workers and removes content while retaining the room and its capture computer. Deleting the room or all account data removes the session too. Saved session selection is only a UI convenience and has no effect on background execution.

New thought starts a separate chatroom/session without stopping the previous worker. Select the previous session again, or open Cloud, to return to its context and Stop control. Clearing an unrelated room does not stop that worker.

## Files, pictures, and video

Everything processed becomes a memo and therefore a Wiki source: typed thoughts, captures, uploaded documents, Markdown, HTML (scripts, styles and tags stripped to readable text), pictures (OCR or the vision model), audio (transcribed), and video. Keeping does not need AI: files dropped or pasted into New thought are staged, and **Save memo** (or Enter when no provider is enabled) posts them to `POST /api/memos` with `file_ids`. The backend reads each file locally (document and Markdown text, Apple Vision text of pictures and of one Quick Look frame per video, a transcript only when speech runs locally or in demo mode) into the memo's input, writes one chat message carrying the files, and completes the memo, so the FTS5 index, the embedding indexer, the Wiki, and later chat context all see it. A kept audio file is transcribed by a cloud speech provider only when the user later asks AI about the chat. The request is idempotent per `message_id`. Any file can be attached in chat with **Attach to chat** (up to 8 files, 25 MB each); the browser flow is covered by `tests/e2e/ux-flow.spec.mjs`. A video is represented by one frame rendered with macOS Quick Look (`qlmanage`, no extra install): in Image mode the frame goes to the vision model, otherwise Apple Vision reads its text; `/api/files/{id}/thumbnail` serves the same frame, which the Wiki shows as the video's picture linking to the download. Video audio is not transcribed yet.

## Schema migration

The SQLite schema migrates automatically. Every table is declared once with `CREATE TABLE IF NOT EXISTS` (`crates/server/src/schema.sql` and the module `init` functions); on startup `schema::ensure` creates missing tables and adds any column a declaration names that an existing table lacks (`ALTER TABLE … ADD COLUMN`, with `NOT NULL` columns given an empty or zero default and constraints SQLite cannot add stripped). Adding a column to a definition is therefore the whole migration. Changes that are not additive go into `schema::VERSIONED`, an ordered list run once each under `PRAGMA user_version`. The older blob-to-file migration in `storage.rs` still runs after it.

## Backend folder

Persistent application data defaults to `~/.causewaybaybot/`: configuration snapshots in the root, SQLite/uploads/captures/memo exports/Wiki/embedding cache under `data/`, and managed logs under `logs/`. The folder is not a browser setting. Launch-directory changes do not move the data, and startup no longer remembers a data-path override. Explicit CLI/environment overrides remain for isolated tests and deployments. Existing nondefault archives trigger migration instructions instead of silently disappearing; see [storage layout](../README.md#storage-and-search). Apple-managed assets and independently installed MLX runtimes are outside the application's data store.

## Configure Wiki and TextEdit

Open **Assistance settings** in the sidebar or **Customize Wiki & TextEdit** below the Auto controls. These are personal settings saved in backend SQLite, isolated by wallet and available on other browsers. The screen saves only fields you changed, so unrelated changes from another device are preserved. Existing settings refresh every five seconds while the form is unedited. The save bar shows unsaved changes and stays within reach while editing; Discard restores the last saved values. Longer setup guidance is under “How your Wiki works” and “Set up your Mac.”

| Setting | Default | Behavior |
| --- | --- | --- |
| Enable Wiki | Off | Enables manual builds and, when selected, automatic inclusion |
| Update Wiki automatically | On | Once Wiki is enabled, includes completed saved sources, including existing history |
| Organize Wiki by | Topics from hashtags | Derived documents are one per tag; alternatively one per UTC capture day. The feed and the map always use tags |
| Include AI answers | On | Includes labeled AI output alongside the original text |
| Enable TextEdit assistance | On | Permits the separate Auto control; enabling this never starts Auto |
| Typing pause | 1.2 seconds | Configurable from 0.5 to 10 seconds; stored as milliseconds in the backend |
| Answer length | Short and practical | Alternatively request detailed explanations with examples |

Saving a changed TextEdit setting stops active Auto sessions and revokes pending cloud approvals. Re-enable Auto explicitly afterward. Changing only Wiki settings leaves Auto running. The admin's separate polling interval remains authoritative. macOS Automation permission must still be granted on the capture Mac; a web setting cannot grant it.

The TextEdit enable setting also applies before the first session is created and refreshes in an open workspace. Enabling assistance permits the Auto button; it never starts Auto by itself.

## Wiki

The **Wiki** menu opens `/wiki`. It is on by default and updates automatically: every completed capture from New thought or chat, every autonomous-run answer (TextEdit, Notes, Terminal) and every remote request is saved to SQLite as a memo and, in the same step, tagged and organized into the Wiki. Turn it off in assistance settings, or choose **Update Wiki** when automatic updates are off.

The unit is one message. Each message gets tags: the hashtags you wrote come first, then the on-device model (Apple or MLX; never a cloud provider) adds two to five more, told which tags you already use so it reuses them; five at most, normalized to one spelling (`#Bitcoin`, `btc` and `bitcoin` are one tag). Demo mode and backends without a local model use a word-frequency fallback, and only when you wrote no hashtag. A message with nothing to tag lands in `#inbox`. In On-device and Hybrid modes, nothing leaves the computer for tagging.

The page is one screen. The toolbar holds the search box (words, or `#tag` filters, or both), the **Feed**/**Map** switch, **Fullscreen** and settings. The rail on the left lists every tag with its count; one click filters. The feed shows messages newest first as cards: title, a short excerpt, the tags (click one to filter), the chat room the message was said in (a link back to that conversation, or "Library" for a kept note), whether it stayed local or used cloud AI and at what estimated cost, a thumbnail for a capture, picture or video frame, **Related** and **Open**. A tag view also offers the tags that travel with it ("Also tagged") and **Open as document**: every message under that tag as one readable Markdown page with its media and cost lines. Routes `/wiki`, `/wiki/tag/{tag}`, `/wiki/map` and `/wiki/{id}` support reload and browser history; the bare Wiki menu entry reopens the tag or the map you were on (`causewaybay.wiki.tag.v2`, `causewaybay.wiki.view.v2` in browser local storage), and **Show everything** clears it.

**Related** shows the messages that share the most tags with a message, fused with its nearest neighbours in the local embedding index when that is ready. **Map** draws the same organization in 3D with three.js, bundled locally: tags are the large spheres (sized by count), messages the small ones, and pictures show their own thumbnail; tags are linked when they appear on the same message. Clicking a tag filters, clicking a message opens its relations. Without WebGL the map lists the tags instead. **Fullscreen** hides everything but the Wiki (the browser's own fullscreen is requested as well where allowed); Esc or **Exit fullscreen** brings the app back, the mode is remembered for the Wiki only, and leaving the page always restores the app. Searching ranks messages by BM25 over the full-text index fused with semantic similarity (when the embedding model is ready); `#tag` words filter rather than search.

Each document embeds its messages' media inline (the capture's screenshot, uploaded pictures, and one Quick Look frame of each uploaded video, all served only from this backend) and, when cloud AI answered a message, the cloud calls, tokens in/out and estimated cost; the document header sums these. The reading view shows the total, or "No cloud AI was used for this page." The memo page itself (`/memos/{id}`) shows the same measured wait, tokens and estimated cost for that memo.

SQLite stores tags, source membership, document content (`wiki_pages.markdown`), and metadata; nothing is mirrored to disk, so one database file carries the whole Wiki. User-authored Wiki pages/revision history and editable tag aliases remain future work. Regrouping removes obsolete pages; source edits, message removal, room clearing, and account erasure remove corresponding tags and Wiki content, even when automatic additions are off. Disabling Wiki preserves the existing collection and stops new inclusion. A message is analyzed once; editing it later keeps its tags.

Page excerpts are limited to 2,000 source characters and 4,000 answer characters; original data remains in the linked source. The feed shows 30 messages per page. Automatic rebuilding currently runs synchronously with source persistence; very large archives will need incremental background indexing. Search combines BM25 and semantic similarity over sources with literal title/text matching; it is not AI research.

## Apple local

Apple's language model and MLX are separate runtime choices. Apple Foundation Models is **not** implemented through MLX. Rust orchestrates a small Swift subprocess to access Apple frameworks.

Requires a compatible Apple silicon Mac, macOS 26+, a suitable Swift toolchain, and an available Apple Intelligence on-device model. Build the adapter:

```sh
make apple-ai
```

The worker explicitly selects `SystemLanguageModel.default`. It does not select Apple's Private Cloud Compute model. Enable **Apple local** in Backend settings; its model ID is `system`. If unavailable or if the model rejects a request/context, the job fails without cloud fallback. This model has limited context and capabilities; longer tasks can use one of the Qwen models below.

Speech-to-text uses Apple's `SpeechTranscriber`/`SpeechAnalyzer` on the backend. Install a language explicitly during setup, with network access:

```sh
printf '%s\n' '{"operation":"prepare-speech","locale":"en-US"}' | ./bin/invisible-apple-ai
```

Normal transcription checks installed languages and fails if one is missing; it does not download language assets automatically. Uploaded audio is temporarily stored in a private file and removed on completion or cancellation. The UI's Speech language controls the selected locale.

Backend launch overrides, if needed:

- `INVISIBLE_APPLE_WORKER`: absolute path to the built Swift executable.
- `INVISIBLE_MLX_PYTHON`: Python executable with `mlx_whisper` installed.
- `INVISIBLE_WHISPER_WORKER`: absolute path to `scripts/local-ai/transcribe.py`.

These are deployment settings, not browser-supplied commands.

## MLX local

MLX local runs the Qwen models on the **backend Mac** through the backend's own Rust worker (`native/mlx-ai`, built on [mlx-rs](https://github.com/oxideai/mlx-rs) and derived from CausewaybayMaker). Build it once:

```sh
make mlx-ai      # compiles MLX from source the first time (cmake); installs bin/invisible-mlx
```

**On-device model** in Backend settings lists the choices, one tap each:

| Model | Repo | Download | Peak memory |
|---|---|---|---|
| Apple LLM | `system` | none, part of macOS | — |
| Qwen3.5 9B (default) | `mlx-community/Qwen3.5-9B-4bit` | 6 GB | 5.2 GB |
| Qwen3.5 4B | `mlx-community/Qwen3.5-4B-4bit` | 2.5 GB | 2.5 GB |
| Qwen3.8 27B | `mlx-community/Qwen3.8-27B-4bit` | 16 GB | 15.4 GB |

Choosing a card sets the server default provider (Apple local or MLX local) and the MLX model together; **Save backend settings** persists it. The picker shows the backend Mac's unified memory and the GPU budget a model may use (two thirds of unified memory up to 36 GB, three quarters above), and marks each model as fitting, tight, or too large. A model that cannot fit is not downloaded unless the admin chooses **Download anyway**.

A download starts in one of two ways: the admin presses **Download** on a card, or a person asks a question and the admin's chosen model is not installed yet. Users cannot pick an MLX model themselves (the chat's model field follows the admin's choice), so the second case is the admin's own decision taking effect. Background work (Wiki tagging, note checks, remote prompts) never starts a download: it waits on one already running or fails with a plain "not installed" message. A model that cannot fit GPU memory is never downloaded for a request, and nothing outside the catalog ever is. The backend fetches only the config, tokenizer and weights from Hugging Face into `~/.causewaybaybot/data/models/mlx/`, one model at a time, resuming a cancelled or interrupted file with a Range request (`POST /api/admin/local-ai/models/fetch` is the admin route). A model already in the Hugging Face cache (`~/.cache/huggingface/hub`, as CausewaybayMaker or `hf download` leave it) is detected as installed and reused in place.

Progress is pushed, not polled. Backend settings subscribes to `GET /api/local-ai/models/events`, a Server-Sent Events stream that sends the catalog whenever a download moves or the worker loads, unloads, starts or finishes a request; its bar eases toward each value (exponential in and out) and shows bytes, speed and time left. A user whose request is waiting on a download, on their turn, or on the model loading sees the same thing in the chat: the backend writes `progress` events on the memo (throttled to one per percent or half second), the room stream carries them on the assistant message, and the reply shows a measured bar for a download or a sweeping one while waiting or loading, until the first words arrive.

The MLX worker is one single-lane manager. The backend starts `invisible-mlx --serve` once, on the first request, and keeps the model loaded in unified memory between requests; users are served one at a time in arrival order, and each waiting request is told how many are ahead of it. The 4B and 9B load in about a second the first time and answer at once afterwards; switching models drops the old one before loading the new one. A request the user stops is cancelled inside the worker without unloading the model; a worker that dies is restarted on the next request; after twenty idle minutes the worker is stopped so other apps get the memory back. Backend settings shows which model is loaded and whether the worker is answering. The worker is handed an installed directory and runs with Hugging Face offline flags, so a request can never download on its own. The worker reports token counts, which the chat's usage line shows.

Set **MLX engine** to *OpenAI-compatible MLX service* to connect to an already-running loopback service instead, such as `mlx_lm server` from CausewaybayMaker; the backend then neither downloads nor launches anything. Only numeric loopback HTTP URLs are accepted: `http://127.0.0.1:PORT/v1` or `http://[::1]:PORT/v1`. Redirects and proxies are disabled for these requests. The endpoint must actually serve a local model: loopback validation cannot prevent an independently configured service from forwarding requests elsewhere.

For MLX speech, install `mlx_whisper` in the backend Python environment and download a compatible model explicitly before offline use. Set **MLX speech model directory** to its absolute directory containing `config.json`. The adapter forces Hugging Face offline mode and disables its telemetry. No model download or cloud fallback occurs through the adapter.

Backend launch overrides: `INVISIBLE_MLX_WORKER` (path to the built worker), `INVISIBLE_HF_BASE` (Hugging Face base URL, for mirrors and tests), `HF_HUB_CACHE` / `HF_HOME` (where cached models are looked for), `INVISIBLE_UNIFIED_MEMORY_BYTES` (memory override for tests).

## Erasure stamp

**Delete all** must not be undone by a page that was still open elsewhere. The guard is the backend's alone: when a browser page connects, the backend notes which erasure that page starts from, writes it into every content-creating request itself (questions, captures, uploads, snapshots), and refuses a write from a page that has not yet been told about a later Delete all. The browser's own value is discarded, so nothing a browser remembers, whether from another backend, an earlier database or a bad clock, can block it or let stale content back in. A page that hears about the erasure reconnects and continues from the new stamp.

## Modes and disclosure

- **On-device:** inference and speech use local providers. Screenshots use backend Apple Vision OCR before local text inference. Web and coding tools are unavailable for these providers. Explicit cloud requests are rejected by the backend.
- **Hybrid:** starts with the chosen local model. If the model requests cloud help, the frontend displays the exact note and target provider. Approve once or keep it local. Approvals expire on note edits, AI mode changes, stop, and disconnect; they cannot be replayed. Only that note is submitted, with no screenshot, conversation history, or tools. This is available for Auto notes; ordinary chat remains local in Hybrid.
- **Cloud AI:** requests may use the chosen cloud provider and existing cloud speech configuration. Keys remain on the backend.

Admin **Local-only mode** overrides cloud permission globally. Selecting local mode never authorizes a cloud fallback. An unavailable local runtime produces an error.

“On-device” means the user's **trusted backend Mac**, which can be different from the capture Mac. It does not mean end-to-end encryption or encrypted storage. Existing HTTP connections need a trusted LAN/VPN or separately configured TLS. PocketSkynet encryption, key custody, broad retention policy, and encryption at rest are still planned. Initial model/dependency installation requires downloads. Local-only at backend startup also disables automatic semantic-model setup; existing local keyword search remains available.

## Notes behavior and current limits

- Terminal: the local model reads only the lines visible after the previous `[End Invisible AI]` marker. If that marker scrolls out of the visible area while an old `ai, …` line stays visible, the request can be judged again on the next change; clear the screen or ask again to move on. Answers are typed to the TTY as output, so they are not part of shell history.

The CLI advertises `textedit_notes_v1`. Commands use the existing authenticated capture WebSocket and are bound to a connection session and Auto generation. Only fixed read/append operations are supported; captured content and model responses are never executed as AppleScript or shell code.

The document binding includes its TextEdit window ID and name. Reads must settle for the user-configured typing pause (1.2 seconds by default) before inference. Before writing, the client rechecks the document identity and full expected text. A changed or closed document fails safely. The first read is immediate; subsequent reads wait for the admin interval after processing. There is one active Auto run per Mac and one inference at a time for that run, with no accumulating capture queue. Interval changes apply to following waits.

Deduplication compares the document snapshot, not just the extracted question. Writing the same question again below an answer receives a new answer. An unchanged note does not generate repeated answers or reopen a cloud approval after it was declined.

This first version supports plain text notes. It writes the verified text plus the answer through TextEdit's text property, so rich text formatting is not supported. macOS does not expose an atomic compare-and-append transaction: there is a narrow event boundary between the final comparison and assignment. Native acceptance testing on the target Mac remains necessary before relying on unattended edits. Stale results fail instead of retrying an ambiguous write; command IDs and generation checks reject replay.

Answers are retained in the existing memo library/Markdown storage under the room's owner. The TextEdit note and Auto status show insertion progress; this version does not synthesize a chat message for each background note answer.

## Usage

The top strip reads a wallet-scoped SQLite ledger for normal answer inference and Auto notes, including errors and interrupted calls. It records total inference runtime and time to first output. Supported stream usage fields supply input/output token counts; repeated final usage chunks replace counts rather than add them twice. Apple does not report tokens through this worker. Missing usage remains unknown.

The current display reports aggregate local elapsed seconds and reported cloud tokens. Credit balance, USD savings, cached/reasoning token detail, speech accounting, web-search billing, and provider billing APIs are not implemented. Demo responses are labeled and do not create paid-inference measurements. Removing a source memo erases its linked measurements.

## Interface motion

The workspace, Wiki, and assistance settings use browser-native tweening with exponential ease-in-out: a gentle start, a quick middle, and a gentle finish. The Expo.InOut curve is sampled for CSS `linear()`, with a cubic Bézier approximation for older browsers. Short, staggered entrances introduce cards; switches, button presses, disclosure content, and successful saves have subtle feedback. Wiki cards retain their identity during background refreshes, so those refreshes do not replay their entrances. The new effects run once per action and require no animation library, external asset service, or AI call.

Motion follows the device's **Reduce Motion** preference, including changes made while the page is open. Enabling it immediately settles active entrance effects and leaves all controls usable. Browser tests verify intermediate tween frames, polling stability, keyboard interaction, save failures, and live preference changes.

Moving between the Wiki collection and a source page uses an Expo.InOut fade-out (180 ms), followed by a fade-in (220 ms). The outgoing view stays mounted until its fade completes and cannot receive clicks or keyboard input during the swap. The incoming view receives focus. Reduced Motion skips the transition immediately; navigation cancels obsolete transition timers, and updates to the same view do not restart the fade.

## Verification

- Rust unit/API tests cover policy persistence/isolation, local-only rejection, worker bounds/cancellation, loopback restrictions, actual HTTP streaming against a local fixture, Auto deduplication/stop/disconnect, single-use approval, note changes, and usage accounting/isolation.
- Playwright covers the new workspace in Chromium, iPad WebKit, and iPhone WebKit: mode changes across browsers, Auto via a simulated Mac over a real WebSocket, explicit cloud decisions, settings persistence, light/dark layouts, and horizontal overflow.
- Real Apple Vision OCR passed with a synthetic image. The Apple worker and Rust HTTP pipeline passed with synthetic English audio and on-device inference on the development Mac.
- Native TextEdit read/append, stale/wrong-document rejection, and the browser-to-note Auto flow passed after the user approved macOS Automation consent. The smoke test remains opt-in:

```sh
cargo test -p causewaybay-cli live_textedit -- --ignored --nocapture
```

Real MLX inference also passed through Rust HTTP using the already-installed `mlx-community/Qwen3.5-9B-4bit` model from CausewaybayMaker, with Hugging Face/Transformers offline mode enabled. The test verified the answer and provider-reported token usage. It used a temporary loopback server on port 18081, which was stopped afterward. This verifies the local runtime path; it is not a packet-level egress audit. MLX speech recognition still has fixture coverage only.

To repeat the MLX check with an already-installed model:

```sh
HF_HUB_OFFLINE=1 TRANSFORMERS_OFFLINE=1 HF_HUB_DISABLE_TELEMETRY=1 \
  /path/to/python -m mlx_lm server --host 127.0.0.1 --port 18081 \
  --model mlx-community/Qwen3.5-9B-4bit --chat-template-args '{"enable_thinking":false}'
# In another terminal:
cargo test -p causewaybay-server --test local_ai live_mlx -- --ignored --nocapture
```

The TextEdit smoke test creates its own disposable document and verifies append, stale-text rejection, and wrong-document rejection.

For the browser-to-native flow, run `make test-native-notes` on an Apple Intelligence-capable Mac with its model already installed. This builds the local Apple worker and runs two opt-in Playwright checks using temporary backend/client folders and empty cloud credentials:

- Capture from a synthetic screenshot through the actual Rust CLI, Apple Vision OCR, and Apple Foundation Models; verify the answer, Wiki source/search, measured local usage, rejected cloud requests, and persistence after restarting the backend.
- Auto from the browser through the actual Rust CLI and a disposable plain-text TextEdit document; verify the inserted local answer, Stop, and reconnect staying off. Only the uniquely named test document is modified and closed.

The second check reads macOS Automation permission and, if consent has not been requested, asks macOS to display its standard dialog. Approve TextEdit control there to continue. The Automation settings list can be empty before that request; do not assume a Codex entry already exists. Permissions belong to the host named in the dialog, which can differ between running from Codex and from Terminal. A denied/unavailable permission produces an explicit **skip**, not a successful writing result; a consent request that times out fails the test. No permission is granted automatically or reset. This suite does not capture your real screen, call cloud AI, or verify speech transcription. The Rust native smoke test above separately checks stale/wrong-document rejection.
