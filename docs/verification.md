# Verification

> Naming note (2026-10-08): entries below record the UI as tested at the time. Since then the Cloud page became **Autonomous AI** at `/autonomous` (`/cloud` still opens it), Dashboard moved to `/dashboard` (`/status` still works), Devices moved to `/devices`, the Coding agent page and Pinned menu entry were folded into chat and the library, and the product name is Invisible AI.

## Clarify Cloud TextEdit controls — 2026-10-06

Renamed Cloud's ambiguous **Start session / Stop session** buttons to **Start TextEdit replies / Stop TextEdit replies**, including Korean **TextEdit 답변 시작 / 중지** and the other five translations. The optional TextEdit section now explains that replies are added to TextEdit and screen capture works while this feature is off. Existing navigation, capture controls, session context and history are unchanged. The frontend was rebuilt; no backend or client restart was needed.

The existing permission recovery, capture/polling/history, and TextEdit enrollment/start/stop browser scenarios passed **9/9** across Chromium and emulated iPad/iPhone WebKit. Updated their button selectors and expanded the optional section before checking a stopped session after reload. Reviewed the rendered Cloud controls. Translation catalog, Rust formatting and diff checks passed.

## TextEdit failure reporting and typing recovery — 2026-10-06

The reported Cloud screenshot contained the CLI's generic TextEdit failure. That message erased the distinction between denied Automation access, a changed document, and normal typing while AI was answering, so the historical failure cannot be attributed to permission alone. A read-only probe and a disposable-document read/write test succeeded from the current launcher.

The native script now selects the document belonging to the front visible document window, rechecks the selection and expected text before insertion, and verifies the written text. It returns safe typed errors without logging note contents or raw script output. The CLI transports these codes to the backend. A `note_changed` append keeps Auto active, retains the generated reply in chat history, and waits for the updated note; other failures stop with specific recovery instructions. Hybrid approval remains single-use. Error messages and the retry stage have all six translations, and Cloud automatically expands the TextEdit section when it has an error, including after reload.

Validation passed: **21/21 backend TextEdit tests**, **15/15 CLI unit tests**, the explicit native disposable-document test (read/write plus stale-text, wrong-document, and expiry rejection), and a full browser → Rust CLI → real local Apple AI → TextEdit insertion/repeated-question/Stop test. Korean permission errors, reload persistence, recovery in the same session, and mobile overflow checks passed **3/3** on Chromium and emulated iPad/iPhone WebKit. The first browser run had a test-only assertion expecting a hidden Start button after Stop; it was corrected to assert the visible stopped status. Builds, strict Clippy, translation validation, formatting, and diff checks passed. No paid AI was used and no macOS consent settings were changed.

The idle managed backend and capture client were restarted with the new build. The client reported Screen Recording permission granted, TextEdit account enrollment, and a backend connection. Existing history and personal notes were preserved, and Auto was left off for an explicit user start.

## Backend capture controls in Cloud — 2026-10-06

Cloud session cards now own Capture now, polling interval, Start polling and Stop polling. They reuse the room capture APIs and the selected account AI policy, and submit room/workspace erasure generations. Unbound rooms can select one permitted computer; the first capture binds it. Existing binding, screenshots, memos and AI context remain shared with chat. Chat keeps manual Capture and links an active backend loop to Manage in Cloud, without polling settings or start/stop controls.

One-off captures now expose their in-flight state through the existing capture status so Cloud cannot start an overlapping loop. The interval is stored while editing and validated at start, survives two-second UI refreshes, and remains visible after stopping a restored session. Errors and status stay on the session page. Controls and hints have all six translations, with wrapping layouts for small screens.

Backend session/transport regressions passed 8/8; backend and frontend strict Clippy, builds, and diff checks passed. The managed backend was updated while no jobs or TextEdit sessions were active; the existing capture client reconnected. Browser checks reproduced and fixed an interval edit lost before blur, and a missing rebuilt stylesheet causing phone overflow. Existing desktop chat, Cloud mode, TextEdit and language regressions passed. Paid AI and the optional live-cloud suite were not run.

Final shared-session browser run passed **6/6 checks** across Chromium and emulated iPad/iPhone WebKit, including interval validation/preservation, one-node binding, manual captures from Cloud and chat, timer reset, reload, Stop, Korean controls and no horizontal overflow.

## Capture permission precheck regression — 2026-10-06

The reported capture failed at the Rust client's permission precheck before `/usr/sbin/screencapture` ran. The managed client logged a negative check; a separate local diagnostic successfully captured the display. The client now validates the display, attempts the native screenshot, and only checks/requests permission after an actual failure. A successful screenshot is never rejected by the Rust process's precheck. Missing access invokes `CGRequestScreenCaptureAccess` once per client run. Apple's SDK documents that a previously denied process cannot prompt again, so the error includes System Settings and restart instructions. Startup and successful captures never request additional consent.

Chat now labels an empty failed capture as “Capture failed · No AI response,” instead of attributing the error to the selected model, and shows the diagnostic once. Other empty failed requests use “Request failed.” All six translations were added.

CLI unit tests passed **15/15**, including successful capture without a precheck, the permission request, denial without repeated prompts, granted permission and retry guidance; the opt-in TextEdit test remained ignored. CLI/frontend builds and strict Clippy passed. The idle managed capture client was restarted with the fix and reported granted permission and a backend connection. No production notes or settings were changed.

All **3/3 native capture browser checks passed** in Chromium and emulated iPad/iPhone WebKit. They capture the actual display, recognize its test marker with Apple Vision, receive a loopback AI response, reject an invalid display without calling inference, and successfully capture again after recovery. The failure label and absence of duplicate error text are also asserted. Permission grant/denial behavior uses stub callbacks in unit tests; the Mac's consent settings were not reset or changed.

## Visible Auto action on iPad — 2026-10-06

The follow-up screenshot already showed a selected capture Mac and Cloud AI. Auto was off, and its Start button could sit behind the fixed composer. WebKit reproduced overlap at 1152×730 and 1024×648 CSS pixels; the previous 1180×760 check had missed it.

Moved the single Start/Stop button into the TextEdit card heading, gave the enabled Start action a purple primary style, and retained the separate Running/Off state. At viewport heights of 760 CSS pixels or less, the decorative introduction is hidden to prioritize controls. The saved Mac, AI processing mode, enrollment and session lifecycle are unchanged.

**9/9 Playwright tests passed** across desktop Chromium, iPad WebKit and iPhone WebKit. The new regression checks eight browser/viewport combinations (1280×812, 1152×730, 1024×648, 390×700 and 390×844), verifies both button bounds and hit testing above the composer, and clicks actual coordinates to start and stop. It deliberately avoids locator click's automatic scrolling, which could conceal the obstruction. Existing capture separation and policy persistence checks also passed. Frontend build, strict Clippy, formatting and diff checks passed. All five live frontend assets match the repaired build; backend health is 200.

Inspected iPad and phone results. Logs are in `target/verification/auto-action-*`. Tests use isolated demo inference and simulated Macs; this frontend change does not start production Auto or send notes to cloud AI.


## TextEdit Auto enrollment repair — 2026-10-06

The reported “Choose a capture Mac enrolled to your account” error came from the managed My MacBook CLI running with no wallet owner. Shared screenshots were allowed, but TextEdit writes correctly required the signed-in account. The UI had treated screenshot access as permission to start Auto. Account authorization remains enforced.

Capture clients now expose account-specific TextEdit availability. Home and Cloud explain enrollment, offline and outdated-CLI states before starting, provide the signed-in account's connection command through Devices, and retain manual capture on shared Macs. Cloud now refreshes device permissions at the same two-second cadence as Home. `make start2 OWNER_WALLET=…` and `make start3` pass explicit enrollment to the CLI, restart an already-running shared client when needed, and preserve saved ownership on subsequent normal launches.

Verification:

- **24/24 Playwright cases passed** across Chromium, iPad WebKit and iPhone WebKit (2.5 minutes). The new suite enables wallet authentication and checks shared capture versus Auto permission, localized setup, unchanged device identity after enrollment, starting without reloading, note reply insertion, stopping from Cloud, ownership loss, CLI capability and disconnect recovery. Existing capture/Auto separation and Cloud lifecycle/error-recovery checks also passed.
- The final rebuilt frontend also passed **2/2 focused iPad checks** for enrolled Auto start/stop and Hybrid approval. The live server returned health 200 and all five referenced frontend assets matched the final build.
- **12 authentication tests**, **20 notes tests**, **13 CLI unit tests + 1 CLI/backend integration test**, **11 frontend unit tests**, and **7 launcher tests passed**. One opt-in native CLI test remains ignored. Native all-target Clippy with warnings denied, formatting and diff whitespace checks passed.
- The first browser run found a real stale-permissions issue in Cloud and a test matcher issue (`toBeDisabled` does not recognize an HTML option's disabled property); both were corrected. Notes tests were rerun with the intended isolated admin configuration, and CLI network tests with loopback access.
- Applied the backend and CLI to the running installation, enrolled the existing My MacBook to the confirmed signed-in account, and checked its heartbeat and Screen Recording permission. A normal launcher call reused the same registered process. The persisted AI settings file stayed unchanged; Auto was left off for the user to start.

Browser note-writing checks use a simulated Mac and demo inference over real authenticated HTTP/WebSocket connections. This repair did not send production note content to cloud AI or rerun native TextEdit insertion. Logs and reports: `target/verification/enrollment-*`; regressions: `tests/e2e/textedit-enrollment.spec.mjs`, `crates/server/tests/auth.rs`, and `tests/scripts/manage.test.mjs`.


## Clear Capture, TextEdit Auto and AI-location controls — 2026-10-06

Follow-up: added a circular check beside the current AI policy, enlarged the selected mode-card check across home/Cloud/admin, and show a check for running TextEdit Auto. Indicators retain the existing accessible selection/state labels. The rebuilt frontend passed all six focused Playwright cases on Chromium and iPad/iPhone WebKit; light/dark screenshots were inspected and refreshed. Evidence: `target/verification/checkmark-*`.

The home workspace now separates one-time **Capture** (answer in this chat), **TextEdit automatic replies** (answer in the same Mac document), and **AI processing location** (saved account policy). The current policy is always visible; Change expands the three large cards. The duplicate upper Capture action and empty-session creation step were removed. Auto explicitly says Start/Stop, names its Mac, shows Running/Off, and creates a session when needed. A compact hero, neutral stopped state, theme-aware colors, larger native select and revised vertical spacing keep the iPad landscape Auto button clear of the fixed dock. The input setting is labelled as the capture reading method; local Apple/MLX providers expose OCR and disable unsupported image analysis.

The new regression found a real routing-display defect: changing the manual capture device cleared the selected running TextEdit session in the UI. The session now stays selected, shows its original Mac and remains stoppable. The test also checks that changing policy does not create/start sessions, Capture sends one request without starting Auto, cloud image input returns to OCR for Hybrid, start/stop labels match backend state, and a second Mac receives no commands from the first Mac's session.

Verification:

- **11 frontend Rust tests passed**, strict WASM Clippy, formatting, whitespace checks and the production frontend build passed.
- The broad **150-case Playwright run completed with 147 passed and three failures**: the same test still looked for English mode labels after navigating with Korean selected. Corrected the language-aware locator. The final **6/6 focused tests passed** across Chromium, iPad WebKit and iPhone WebKit, including all three corrected cases and the expanded action-separation regression. The iPad case additionally verifies a 44-pixel capture-device selector and the entire Auto button above the fixed dock.
- **24/24 final responsive and motion tests passed** on the final build, covering landscape/narrow/short viewports, dock controls and touch targets, theme changes, Expo tween frames, fade transitions and reduced-motion behavior.
- **15 live browser route/reload checks passed** without authentication. Five served JS/CSS/WASM assets matched the final build by SHA-256. The running backend serves the rebuilt frontend; no restart or production settings/session change was required.

Inspected iPad controls, expanded AI choices, and phone dark controls. Evidence is under `target/verification/clarity-*`. Functional browser tests use isolated databases, real HTTP/WebSocket transport, simulated capture Macs and demo inference. Native TextEdit writing, native screenshot permissions, physical iOS devices and paid cloud AI were not rerun for this UI change.

## `make start2` startup review — 2026-10-06

Executed the exact command against the existing managed services, then stopped both managed processes and repeated `make start2` from a cold start. Both completed with exit 0, and the new client reported screen-capture permission granted and connected over WebSocket/SSE. No active Auto sessions were present before the restart. The user's unspecified failure was not reproduced in this execution environment; a command error, browser access issue and capture failure remain distinct cases.

Fixed misleading startup reporting: the launcher now waits for the client's initial connection result, prints its permission check and connected/retrying state, returns failure for immediate client exit, and returns failure when the backend never announces readiness. It also prints Web URLs and client status when reusing current processes. It does not dump the client's startup owner-wallet line. Permission output is the CLI's startup check, not a guarantee that another terminal app has the same macOS permission.

**6/6 launcher regressions passed** using isolated fixture processes, covering changed-build replacement, saved-data preservation, combined cold/repeat startup, private startup-line exclusion, immediate exit, reconnecting/offline clients and backend readiness timeout. Shell/JavaScript syntax and whitespace checks passed. A final real `make start2` returned exit 0 with the new diagnostics.

Local, LAN and Tailscale checks returned 200 for health, home and Cloud. A fresh emulated iPad WebKit context loaded and reloaded home and Cloud through `http://100.64.0.10:3000`, rendered the login screen and reported no JavaScript errors. This verifies host-side access to that address, not the physical iPad's network/VPN. No wallet login, AI request or production note change was performed. Logs, HTTP results and the browser screenshot are under `target/verification/start2-*`.

## Capture failure recovery — 2026-10-06

The reported button request reached the capture CLI, which repeatedly returned `could not create image from display` for display 1. A direct same-Mac capture succeeded at 1080 × 1920 and the diagnostic process had Screen Recording access; the generic error alone did not establish whether the older background process lacked permission or had an unusable launch/session environment.

The CLI now checks permission in its own process and validates active displays before capture. Its first explicit live Capture request can request missing macOS consent once; startup and fixture captures never prompt. Missing permission, unavailable display numbers, absent displays, and failure after a successful permission check have distinct recovery instructions. TextEdit Automation permission is explicitly identified as separate. The managed launcher now replaces changed/legacy CLI builds and detaches the client like the backend, preserving saved client data instead of leaving an older executable running.

**13 CLI unit tests passed** (the unrelated opt-in TextEdit test stayed ignored), **one real Rust CLI/backend fixture-transport test passed**, and **two managed-process launcher tests passed**. The CLI build, strict all-target Clippy, formatting, JavaScript syntax and whitespace checks passed.

**3/3 native Playwright acceptance checks passed in 26.8 seconds**, covering desktop Chromium and emulated iPad/iPhone WebKit. Each drives the actual Capture button through a live Mac screenshot, Rust transport, Apple Vision OCR, and a deterministic loopback inference endpoint. It then selects nonexistent display 16, verifies the diagnostic and that inference is not called, restores display 1 and captures successfully again. This adds `tests/e2e/native-screen.spec.mjs` and `make test-native-screen`. Initial fixture failures were fixed: inherited device options conflicted with a null viewport, bringing a browser tab forward did not activate its macOS app, and first-launch dialogs obscured the centered OCR marker. The final fixture activates its dedicated browser and places a unique marker at the top. AI quality and paid cloud inference are outside this test.

Applied the updated managed client and restored the production backend, which was found stopped during verification. Final checks show backend health 200, the running CLI reporting Screen Recording permission granted, WebSocket/SSE reconnection and recent capture-client heartbeats. Production notes/settings were not edited, and no production cloud request was issued. Local logs and results are `target/verification/capture-fix-*`; passing screenshots are in `capture-fix-native-results/`.

## Live service update — 2026-10-06

Applied the verified build to the managed backend on port 3000 with `make start`. Before the update, the running older binary returned 404 for `/cloud`, `/wiki` and `/assistance`. After the restart, those routes, a Wiki reader route, `/chat`, health and auth-info return 200; an unknown route still returns 404.

The pre-restart read-only check found no active Auto sessions or pending cloud approvals. Stored memo, room, file and Invisible AI session counts were unchanged after restart. The existing capture CLI process was preserved, and recent capture-client heartbeats were present afterward.

**12/12 live Playwright browser/route checks passed** across desktop Chromium and emulated iPad/iPhone WebKit. Each checks direct navigation, reload, rendered login controls, no horizontal overflow and no JavaScript errors. Five served JavaScript/CSS/WASM assets matched the current `dist` files by SHA-256. These fresh browser contexts remained unauthenticated; authenticated menu/session behavior is covered by the isolated 51-case run below. No real note, AI request or session was created by these live checks.

Visual inspection also found the former product name in wallet login and connected-device status. Both visible labels now use Invisible AI, including all six status translations. Rebuilt the frontend and repeated all 12 live checks with an explicit login-brand assertion. The existing translation-catalog and English/Korean UI tests passed **3/3** against an isolated demo backend; formatting and whitespace checks passed. The authentication challenge protocol was unchanged.

Local evidence: `target/verification/live-menu-{before,after,playwright}.json`, `live-menu-restart.log`, `live-menu-smoke.mjs`, and the three `live-menu-login-*.png` screenshots.

## Menu, interface and logic review — 2026-10-06

Walked every main menu destination, its selected state, direct URL, reload, and browser history on desktop Chromium and emulated iPad/iPhone WebKit. Confirmed the distinction between Cloud modes/sessions, Wiki knowledge, original memos, stored files/images, workspace Dashboard, connected Devices, personal assistance settings, and admin backend settings.

Fixed three navigation defects: `/cloud`, `/wiki`, `/wiki/{id}` and `/assistance` rendered the app through a fallback with HTTP 404 instead of a successful page response; mobile sidebar search left its overlay open when already in the library; and sidebar search from Pinned stayed inside the pinned filter. New browser regressions reproduced the first two before their fixes, and a backend integration test verifies successful page responses without turning unknown paths into successes.

The workspace, usage strip, Wiki controls and assistance settings now use the existing seven-language catalog. Language props update existing components rather than remounting them. Tests switch all seven languages while retaining an unsent chat draft, unsaved typing-pause preference, Wiki query and original source text. Stored Wiki Markdown and user content remain unchanged. Quick Auto now checks its saved session's own Mac and AI provider, explains offline/missing-setup/competing-session conditions, and keeps Stop usable for a running session.

The first 56-case browser review reported **53 passed and 3 failed**; the latter were one new assertion repeated across browsers, comparing the spelling of the boolean `open` attribute instead of its presence. The assertion was corrected. Backend API tests initially inherited the developer's wallet-admin environment; with that variable cleared only for the test process, **all 14 API tests passed**. **11 frontend Rust tests**, strict native/WASM Clippy, frontend/backend builds, formatting, JavaScript syntax and whitespace checks passed.

After the final Auto changes and rebuild, **51/51 Playwright checks passed in 4.9 minutes**. This final run includes every corrected menu assertion, all 12 new menu/language checks, the complete 30-case Auto suite (including the three new offline/busy/provider guards), six AI-policy checks and three TextEdit-disable checks. The earlier broader review's unaffected Wiki and responsive tests were not repeated. All browser runs use isolated demo backends and simulated capture Macs; no paid cloud request, real note write or production restart was performed.

Logs are in `target/verification/menu-ui-*.log`; `playwright-report/index.html` contains the final passing run. Reproduce with `npm run test:e2e -- tests/e2e/menu-ui.spec.mjs tests/e2e/invisible-ai.spec.mjs tests/e2e/ai-policy.spec.mjs tests/e2e/wiki.spec.mjs --grep 'main menus|mobile sidebar|sidebar search|workspace, Wiki|invisible-ai|ai-policy|TextEdit disabled'`. Visually checked Korean examples: desktop menu, phone menu, workspace, assistance settings, and Wiki reader.

## Cloud workflow review — 2026-10-06

Reviewed mode selection → capture Mac selection → stopped session creation → Start → TextEdit assistance → optional one-time Hybrid approval → Stop. Session creation and navigation never start Auto. Modes apply to all sessions owned by the account, and On-device inference runs on the trusted backend Mac. Browser closure leaves a started session running; capture CLI disconnect or backend restart requires an explicit new start.

The review reproduced a stale status after a mode change revoked a pending Hybrid approval: the session still said it was waiting for approval. Revocation now saves **AI settings changed. Write a new note to continue.** for the affected owner. A running worker processes the next new note under the updated policy without resending the unchanged note or accepting the old approval. Start controls now explain disabled TextEdit assistance, offline Macs, missing provider setup, and another active session on the same Mac. Running sessions retain Stop. Failed background refresh displays **Waiting for backend**.

The strengthened running-worker regression failed before the backend fix and passed afterward. **47 server library tests and 11 frontend tests passed**. Frontend/backend builds, strict native and WASM Clippy, formatting, JavaScript syntax, and diff whitespace checks passed. A new owner-isolation test also verifies that admin-wide revocation persists status under each session's correct owner.

The selected **63-case Playwright review** (`cloud.spec.mjs`, `invisible-ai.spec.mjs`, and `ai-policy.spec.mjs`) initially reported **60 passed, 3 failed in 7.1 minutes**. All three failures were the new missing-provider fixture, which intercepted WebSocket RPC although settings GET uses HTTP. Correcting the fixture to intercept HTTP produced **3/3 passing rechecks** on desktop Chromium and emulated iPad/iPhone WebKit in 2.9 seconds. The remaining 60 checks were not repeated because no production code changed after their run. Coverage includes mode persistence and cross-browser sync, stale polling responses, browser detach, one session per Mac, cloud approval revocation followed by a fresh local answer, disabled/offline devices, room erasure, reconnect, and repeated note questions.

Logs: `target/verification/cloud-flow-{playwright,recheck,server-tests,frontend-tests,server-clippy,frontend-clippy,build}.log`. The main HTML report retains the initial 60/3 result; the focused recheck log records the final resolution. The iPad Cloud screenshot was visually inspected. These checks use isolated demo backends and simulated Macs, without paid AI requests or changes to real TextEdit notes. Native automation was not rerun in this review, and the existing production backend was not restarted.

## Cloud AI mode selection — 2026-10-06

The Cloud page now provides large **On-device / Hybrid / Cloud AI** cards. They save the account's policy through the existing backend API, preserve provider choices, update the selected card only after success, and refresh changes made in another browser. Local-only restrictions disable cloud choices; rejected saves retain the prior mode and display an error. Changing a mode revokes pending Hybrid approvals without creating or starting sessions. Phone layouts stack the cards vertically; desktop and tablet layouts use three columns.

Validation: **39 Playwright checks passed in 2.8 minutes** (`cloud.spec.mjs` plus `language.spec.mjs`), including the nine Cloud scenarios on desktop Chromium and iPad/iPhone WebKit. New cases exercise all three modes, backend persistence after reload, live cross-browser synchronization, provider preservation, New thought consistency, admin restrictions and restoration of the personal choice, failed saves, delayed polling responses, and rejection of an approval revoked by a mode change. **11 frontend Rust tests**, the frontend build, strict WASM Clippy, formatting, and whitespace checks also passed. These tests use isolated demo backends and simulated capture Macs, without paid AI calls.

Logs: `target/verification/cloud-modes-{playwright,rust,clippy,build}.log`. The desktop, iPad and phone screenshots linked below were refreshed and visually checked with the mode selector present.

## Cloud session manager — 2026-10-06

Added the **Cloud / 클라우드** main-menu entry and `/cloud` page using the existing owner-scoped backend session API. The page creates named stopped sessions, starts/stops individual sessions, shows device/document/progress and summary counts, and handles exact-note Hybrid approval or rejection. Navigation does not change the saved AI mode. New thought links to the manager and retains its quick controls. The UI includes six translations in addition to English, light/dark styles, responsive layouts, and finite Expo entrance tweens that respect reduced motion.

- **63 Playwright checks passed in 5.3 minutes** across desktop Chromium and emulated iPad/iPhone WebKit. This includes 15 new Cloud checks (five scenarios on each target) plus existing Auto, language, and responsive regressions.
- **11 frontend Rust tests passed**. Frontend build, strict WASM Clippy, formatting and diff whitespace checks passed. The translation coverage check now includes the Cloud component; all 32 explicitly marked label references have six nonempty translations.
- New tests cover localized navigation/direct-link reload; create without starting; start and continued work after closing the browser; reopening and stopping from another browser; exact-content one-time Hybrid approval/rejection; TextEdit disabled/offline Mac gating; independent session status; and recovery from failed list/action requests while retaining persisted sessions.
- The initial error-injection test intercepted HTTP, but this UI uses WebSocket RPC. The fixture was corrected to inject failures on the actual transport; all recovery assertions then passed. No backend behavior was changed for this feature.

Reproduce with `npm run test:e2e -- tests/e2e/cloud.spec.mjs tests/e2e/invisible-ai.spec.mjs tests/e2e/language.spec.mjs tests/e2e/responsive.spec.mjs` after `make frontend`. Logs are under `target/verification/cloud-*.log`; the HTML report is `playwright-report/index.html`. Visually reviewed captures: desktop, iPad, and phone dark.

These checks use isolated demo backends and a simulated capture Mac. They verify browser/backend session behavior without paid cloud calls or changes to real TextEdit documents. Earlier native acceptance results below remain separate; the entire broader project suite was not rerun for this menu/UI change.

## Flow fixes and regression verification — 2026-10-06

The six reported flow defects are resolved and covered by regression tests:

| Flow | Verified behavior |
| --- | --- |
| Clear a session's chatroom | Cancels Auto, removes the durable session and rejects the old Hybrid approval; no erased history is recreated |
| New thought | Keeps the independent running Auto session visible and stoppable |
| Admin Hybrid setup | Editing the cloud provider/model preserves Hybrid and its per-note approval requirement |
| Admin changes an MLX model | Open browsers adopt the installed model; stale browser caches cannot override Apple/MLX models, and local model fields are read-only |
| Admin enables Local-only | API and UI expose effective On-device processing, select an enabled local runtime and reject cloud requests; lifting the restriction restores the saved personal policy |
| Repeat the same note question | A fresh document snapshot below the previous answer receives one new answer; unchanged notes and declined approvals do not loop |

The chat provider selector now permits browser-specific choices in Cloud AI mode even when an admin default is inherited. Overview follows the same policy and preserves permitted browser choices across reload. New sessions immediately publish their source room to the chat list. TextEdit assistance settings are loaded even when no sessions exist; changing that setting updates an open workspace without starting Auto. The existing headless capture test's PNG fixture had an invalid checksum, which was corrected without changing its upload/processing assertions. Reconnect tests now distinguish chat errors from the independent notes status.

| Check | Result |
| --- | --- |
| Default Rust unit/integration suite | **150 passed**; 7 opt-in checks excluded from the default run |
| Launcher/package script tests | **8 passed** |
| Full Playwright review | **253 executed: 247 passed, 6 failed** before the final fixes; two causes reproduced across desktop Chromium and iPad/iPhone WebKit |
| Final Playwright verification after rebuilding | **64 passed, 0 failed in 5.9 minutes**, including every failed case, new regressions and affected policy/Auto/chat flows on all three browser targets |
| Final native Playwright acceptance | **2 passed, 0 skipped, 0 failed in 20.2 seconds** |
| Actual Apple speech-to-text and answer through Rust HTTP | **1 passed**, using the installed local models and synthetic speech |
| Formatting, strict native/WASM Clippy and frontend/backend builds | Passed |

The full review exposed one actual UI defect (TextEdit disabled was not shown before a session existed) and one assertion race (streamed answer text arrived before the stored memo reached `complete`). The UI now reads account assistance settings independently of session existence, and the model test waits for the persisted completion event while checking both provider and model. Both cases passed on all three browser targets in the final run. The full 253-case suite was not repeated after those final localized changes; the 64-case run covers their affected flows and every prior failure.

The final native checks use the actual Rust CLI, Apple Vision OCR and Apple Foundation Models. Capture uses a known screenshot fixture and checks saved answers, Wiki, local usage and restart persistence. Auto opens a uniquely named disposable TextEdit note, answers `7 + 5`, answers the same question again below the previous answer, verifies Stop prevents further writing, and verifies reconnect leaves Auto off. Updated screenshots are in `docs/screenshots/native-local-capture-wiki.png` and `native-textedit-auto.png`.

Regression code is in `tests/e2e/ai-policy.spec.mjs`, `invisible-ai.spec.mjs`, `local-ai.spec.mjs`, `native-notes.spec.mjs`, and the existing Wiki/reconnect suites; backend and model-choice regressions are in `crates/server/src/notes_tests.rs` and `crates/frontend/src/chat.rs`. The final selected browser command is:

```sh
npm run test:e2e -- tests/e2e/ai-policy.spec.mjs tests/e2e/invisible-ai.spec.mjs tests/e2e/local-ai.spec.mjs tests/e2e/wiki.spec.mjs tests/e2e/app.spec.mjs tests/e2e/messenger.spec.mjs --grep 'ai-policy|invisible-ai|local-ai|TextEdit disabled|browser stores preferences|polished home|question and draft|Reload resumes'
```

Local logs and the full-review report are retained under `target/verification/`; `playwright-report/index.html` contains the final 64-case report. Browser cloud/MLX inference uses isolated fixtures, not paid/live cloud requests. iPad/iPhone coverage is WebKit device emulation, not physical mobile capture. Native acceptance uses real Apple models with no cloud credentials or downloads. Live MLX speech, deployment network isolation, transport encryption and the other explicitly deferred milestones remain outside this verification.

## Real TextEdit automation accepted after consent — 2026-10-06

The user reported that System Settings → Automation was empty. The preflight had used `AEDeterminePermissionToAutomateTarget(..., false)`, which returns **−1744** when consent is needed without displaying a request. This is not an existing denied permission. An explicit request with `askUserIfNeeded=true` displayed the macOS dialog; the user approved it and the API returned **0**. Setup guidance no longer assumes a Codex → TextEdit entry already exists. The opt-in native Playwright test now requests missing consent using the standard dialog, without granting or resetting permissions automatically.

The real Rust TextEdit smoke test **passed**: it read and appended to a disposable document, rejected a replay with stale expected text, and rejected the wrong document. The browser test then exposed a fixture-opening issue: a dynamic AppleScript POSIX-file argument did not give sandboxed TextEdit access to the temporary file. The test now opens that exact fixture through LaunchServices (`open -a TextEdit <fixture>`) and waits for the named document. Application read/append code was unchanged.

The final native Playwright run reported **2 passed, 0 skipped, 0 failed in 14.7 seconds**:

- Browser Capture → actual Rust CLI → Apple Vision OCR → real Apple Foundation Models → answer, Wiki, measured local usage and backend restart persistence.
- Browser Auto → actual Rust CLI → real TextEdit note → local Apple answer inserted in that same document. The test checked `7 + 5` → `12`, one answer marker, Stop preventing an answer to a newly appended question, and CLI reconnection leaving Auto off.

Only uniquely named disposable notes and isolated backend/client folders were used; no cloud inference was called. Browser result screenshots are saved under `docs/screenshots/native-local-capture-wiki.png` and `native-textedit-auto.png`. JavaScript syntax and whitespace checks passed. The earlier permission-blocked results below are historical and superseded for native note insertion. At that point the six flow defects and chat-provider regression were still unresolved. The later flow-fix verification above supersedes that status.

## Complete flow review and native local capture — 2026-10-06

Added `make test-native-notes` and `tests/e2e/native-notes.spec.mjs` for repeatable browser/native acceptance checks. The capture case passed using the actual Rust backend and capture CLI, a synthetic screenshot, real Apple Vision OCR, and real Apple Foundation Models. The screenshot question `What is 7 + 5?` produced `12`; image bytes, Markdown, Wiki search/source links, local usage, rejected cloud requests, and persistence after a backend restart were checked. The backend ran without demo mode or cloud credentials in isolated temporary directories.

The native suite reported **1 passed, 1 skipped** in 26.7 seconds. The skipped case is real TextEdit Auto, insertion, Stop, and reconnect. macOS returned **−1744 (Automation consent required)**; no successful native note write is claimed. The separate Rust TextEdit smoke test also timed out while opening its disposable document. An isolated loopback harness against the real Rust CLI passed five command guards: expired commands, excessively future expiries, missing generations, writes before Auto starts, and writes using a stopped generation.

A fresh **45/45 Playwright regression run passed in 2.4 minutes** across desktop Chromium and iPad/iPhone WebKit emulation (`invisible-ai`, `local-ai`, and `wiki` suites). These tests use demo inference and simulated note devices to verify mode persistence, Auto/Stop/disconnect, exact-content Hybrid approval, admin local settings, Wiki/search/deletion, and assistance settings. They do not substitute for the skipped real TextEdit write. JavaScript syntax, test discovery, the Make target dry run, and whitespace checks passed.

The preceding full `make verify` review passed formatting, strict native/WASM Clippy, default Rust tests, launcher tests, and builds. Playwright reported **228 passed, 1 failed**: `messenger.spec.mjs` could no longer change the chat provider after an inherited admin default disabled its selector. Separate isolated probes reproduced six additional unresolved flow issues: clearing a room leaves note Auto/approval active; New thought hides the previous running Auto; provider selection changes Hybrid to Cloud AI; browser model state can outlive an admin MLX model change; Local-only leaves a personal Cloud choice selected but disabled; and a newly repeated identical note question is ignored. That review did not fix these findings; the later flow-fix verification above records their resolution.

No macOS permission was changed, personal data was modified, or cloud inference was used. Native capture here means the real CLI transport with a known screenshot fixture; real desktop capture and actual TextEdit insertion are separate acceptance boundaries. This run does not establish encrypted transport or an offline deployment audit.

## Backend storage and large admin AI choices — 2026-10-06

Backend persistence defaults to `~/.causewaybaybot/`, with application data under `data/` and managed logs under `logs/`. Startup no longer reads or writes a remembered data-directory override. Explicit launch overrides remain available for test/deployment isolation. A populated legacy location stops startup with migration instructions without moving or deleting files. The root is resolved once before loading its `.env`, and a missing home directory cannot silently redirect storage to the working directory. Storage paths are absent from browser settings and rejected by the settings API.

Admin setup now begins with large On-device, Hybrid, and Cloud AI buttons. Their default and provider choices persist on the backend; personal choices remain intact, and Hybrid still requires per-note approval. The separate Local-only safeguard blocks Hybrid/Cloud selections. Cards have selected states, keyboard focus, existing Expo tweens, and a vertical phone layout. Screenshot review caught crowded runtime checkboxes and an undefined globe icon; both were corrected before the final run.

All **66 targeted Rust checks passed**: three startup-folder checks, 13 API checks, three persistence checks, 36 server unit checks, and 11 local-runtime checks. Two live model tests remained opt-in. The existing compilation regression required the host compiler sandbox and passed when run with that access. All **31 targeted Playwright checks passed** across desktop Chromium and iPad/iPhone WebKit; after the visual refinements, all **15 admin/local-AI checks passed again**. Coverage includes a synthetic home directory, restart persistence, Wiki/file exports, rejected path settings, `.env` precedence, all three admin modes, personal overrides, keyboard activation, touch targets, light/dark appearance, Auto, and single-use Hybrid approval. No personal data was moved and no paid AI was used.

Production frontend/backend builds, formatting, strict native/WASM Clippy, and whitespace checks passed. Desktop/light and phone/dark screenshots were inspected and saved as `docs/screenshots/admin-modes-desktop.png` and `admin-modes-phone-dark.png`. Device coverage uses browser emulation, not physical iOS devices.

## Wiki as a tag feed with map, relations and fullscreen — 2026-10-08

Replaced topic pages with one node per message: tags from the user's hashtags and the on-device model (told the owner's existing tags; five at most, one spelling), stored in `wiki_tags` and rewritten with the documents in the same transaction. New `GET /api/wiki` feed (newest first, or BM25 fused with semantic similarity, `#tag` filters, chat-room links, media thumbnails, cloud cost), `GET /api/wiki/related/{id}` (shared tags fused with embedding neighbours), and a tag/message graph with photo thumbnails. The page is one screen: toolbar, tag rail, feed or map, a related panel, tag documents on request, and a Wiki-only fullscreen mode that Esc or the button leaves.

With `CWBBOT_ADMIN` unset (the shell here exports it, which makes unrelated auth fixtures return 401 on the committed code as well), **all Rust checks passed**: 99 server unit tests, 14 API, 12 authenticated API, 11 chat/media erasure, 2 keep, 7 documents, 11 local runtime, 5 messenger sync, 3 persistence and the rest, plus 11 frontend unit tests including the translation catalog. **27/27 Wiki Playwright checks passed** on desktop Chromium and emulated iPad/iPhone WebKit, and **67 checks passed** in the broader run of the Wiki, UX-flow, motion, menu/language and keep suites after the first selector corrections (`npx playwright test tests/e2e/wiki.spec.mjs tests/e2e/ux-flow.spec.mjs tests/e2e/motion.spec.mjs tests/e2e/menu-ui.spec.mjs tests/e2e/keep.spec.mjs tests/e2e/language.spec.mjs`). Screenshots: `test-results/wiki-*.png`, including `wiki-fullscreen-*` and `wiki-map-view-*`.

Limits: demo mode and these checks use the word-frequency fallback, so the real on-device tagging prompt was not exercised; the native-notes and native-screen specs were updated for the new selectors but not run (they need macOS automation). Editing a message keeps its tags; **Update Wiki** rebuilds documents without re-tagging. Editable tag aliases and vision captioning for photos are not implemented.

## Wiki fade-in and fade-out — 2026-10-06

Wiki collection and article views now fade out for 180 ms before the replacement fades in for 220 ms, using the shared Expo.InOut curve. Outgoing content remains mounted but inert until its exit completes. Focus moves to the incoming view; same-view refreshes do not restart the transition. Reduced Motion settles an active swap immediately, and navigation discards obsolete timers and animation callbacks.

All **33 targeted Playwright checks passed in 2.4 minutes** across desktop Chromium and emulated iPad/iPhone WebKit. The 15 motion checks include measured incoming/outgoing opacity at quarter points and the midpoint, retained outgoing content, focus restoration, live reduced-motion changes, and interrupted browser-history navigation. The 18 Wiki/settings checks cover persistence, search, source deletion, organization, and TextEdit disable enforcement. The production frontend build, strict WASM Clippy, formatting, JavaScript syntax, and whitespace checks passed. Tests use isolated demo fixtures; no native automation or cloud inference was exercised by this UI change.

## Exponential ease-in-out — 2026-10-06

The new interface tweens now share an Expo.InOut curve. CSS `linear()` samples the exponential function at 2.5% intervals, with exact endpoints and a cubic Bézier fallback for older browsers. Existing durations, staggering, and reduced-motion behavior are preserved. All **9 motion checks passed** across Chromium and iPad/iPhone WebKit, including measured opacity/position at the curve's endpoints, quarter points, and midpoint. The frontend build, JavaScript syntax check, and whitespace check passed.

## Tweened interface motion — 2026-10-06

Added native CSS tweening with cubic Bézier easing to the workspace, Wiki, and assistance settings. Effects include short fade/rise entrances, bounded card staggering, eased hover/press feedback, switches, disclosure content, and a confirmation after a successful save. Entrance animations release their properties when finished. Card keys preserve identity during background refreshes; user-requested filtered results can enter again. The new effects are finite and respect live changes to the system's reduced-motion preference.

The **18 Wiki/settings regression cases passed** in Chromium and iPad/iPhone WebKit. All **9 motion checks passed** after correcting the test instrumentation. They check intermediate opacity/position values, stagger delays, actual background WebSocket polls without replay, filtered-result navigation, failed saves without success feedback, successful saves, and reduced motion changed mid-animation. Instrumentation uses computed CSS easing and the app's actual WebSocket transport. Strict WASM Clippy, production frontend build, formatting, and whitespace checks passed. No animation package or external service was added.

## Wiki reading and navigation polish — 2026-10-06

Wiki pages now open in a dedicated reading view with a compact back control, comfortable line lengths, clearer source metadata, and keyboard focus placed on the article. Returning to the collection preserves the search. Search includes a 44-pixel clear button and an explicit reset action for empty results. Settings have direct Wiki/TextEdit section links with current on/off states. Secondary text has stronger contrast on the pastel cards; the language selector now has a consistent 44-pixel touch target and arrow in light/dark WebKit.

All **21 targeted Playwright checks passed** in desktop Chromium and emulated iPad/iPhone WebKit. Coverage includes reading focus and search restoration, keyboard search reset, settings section navigation, 320-pixel layouts, settings persistence/validation, Wiki source updates/deletion, history, and theme persistence. The three reading tests also passed after the final scroll adjustment, which keeps the back button in view. All **9 frontend unit tests**, strict WASM Clippy, formatting, production frontend build, and whitespace checks passed. Screenshots were inspected in desktop light and phone dark themes; the reading previews are saved as `docs/screenshots/wiki-reader-desktop.png` and `wiki-reader-phone-dark.png`. This pass changes presentation/navigation only; AI routing, saved settings, and native automation permissions are unchanged.

## Wiki and assistance design refinement — 2026-10-06

Refined the Wiki and TextEdit settings interface with the existing generated notebook artwork, softer light/dark colors, clearer typography, accessible on/off switches, concise explanations, and expandable setup help. Wiki has a prominent search field, compact collection status, topic cards, and readable source pages. Settings show the typing pause in seconds while preserving backend milliseconds, validate the range before saving, expose unsaved/saved status and Discard, and keep Save reachable during edits. The save bar returns to the normal layout when there are no changes.

All **20 distinct targeted Playwright cases passed** across navigation and the Wiki/settings flows; the **15 Wiki/settings cases** also passed after the final interaction refinements. Coverage includes desktop Chromium, iPad/iPhone WebKit, light/dark layouts, 320-pixel overflow and reachable Save controls, keyboard switching, discard without backend changes, seconds-to-milliseconds persistence, invalid-input feedback, cross-browser settings, Wiki search/source deletion, and reload/history navigation. All **9 frontend unit tests passed**, along with strict WASM Clippy, formatting, the production frontend build, and whitespace checks. No backend behavior or native TextEdit permissions were changed for this design work.

Desktop and phone screenshots were visually inspected and saved as `docs/screenshots/assistance-desktop.png`, `assistance-phone-dark.png`, `wiki-desktop.png`, and `wiki-phone-dark.png`. These previews use isolated demo data. The artwork is reused from `frontend/assets/invisible-ai-hero.png`; no external assets or runtime CDN were added.

## Configurable Wiki and TextEdit — 2026-10-06

Added wallet-owned assistance settings, a Wiki menu and source-linked pages, automatic/manual organization, topic/day grouping, full-source search, and optional AI-answer inclusion. TextEdit can be disabled and has configurable typing pause and answer length. Changes stop active TextEdit Auto and revoke pending approvals; Wiki-only changes leave Auto running. Settings use partial backend updates so unrelated changes from another browser are preserved.

**97 Rust checks passed** across server unit tests (36), authenticated API tests (11), general API (12), chat/media erasure (11), local runtime fixtures (11), messenger sync (5), persistence (3), and frontend units (8). New tests cover defaults/validation, per-wallet persistence, admin exclusion from another wallet's Wiki, automatic/manual builds, grouping and Markdown mirrors, source provenance, full-source search beyond excerpts, literal query safety, zero AI usage for organization, source/room erasure, crash repair, and TextEdit cancellation/approval revocation. Native and WASM Clippy with warnings denied, formatting, builds, and whitespace checks passed.

The first browser regression run passed 38 of 41 cases and caught the same typing-pause input bug on three browser targets: a selection change could rerender the field before its value was saved. The input now tracks its draft on every input event. The subsequent **12/12 Wiki and settings tests passed** in desktop Chromium, iPad WebKit, and iPhone WebKit, including cross-browser persistence, reload/history URLs, source deletion, manual builds, and backend disable enforcement. All **41 distinct affected browser cases** have passing results across those runs. Visual inspection then identified short native WebKit select controls; explicit 46-pixel styling and a minimum-touch-height assertion were added, and the three settings checks passed again. Light/dark screenshots were inspected; representative settings previews are saved under `docs/screenshots/`.

These checks used synthetic data and local fixture services without paid/cloud AI. Native TextEdit control still requires macOS Automation permission and its previously blocked smoke test remains pending. Wiki currently organizes existing content deterministically; semantic research, generated synthesis, independently editable pages, and revision history remain future work. Existing limitations for local runtime packaging and transport encryption still apply. See [configuration and Wiki behavior](invisible-ai.md).

## Invisible AI local runtimes, notes, and interface — 2026-10-06

Implemented a responsive Invisible AI workspace with generated artwork, light/dark appearance, large Auto/Capture controls, backend-persisted per-wallet On-device/Hybrid/Cloud AI choices, one-time Hybrid approval, and a usage strip. The backend now supports Apple Foundation Models, Apple speech recognition, a loopback MLX language service, and an offline MLX Whisper adapter. The notes workflow uses a session-bound TextEdit read/append protocol with deduplication, document checks, cancellation, and Auto off after reconnect/restart.

Verification used isolated temporary data/config directories, synthetic notes/audio/images, and local fixture services. No paid cloud inference was invoked for this implementation.

| Check | Result |
| --- | --- |
| Rust regression suite | Passed across server, shared, capture CLI, and frontend |
| Final server unit checks | 29 passed, including notes policy/approval and provider usage protocols |
| Final local-runtime integration checks | 11 passed; 2 live checks gated separately |
| Frontend unit checks | 8 passed |
| Launcher/package script checks | 8 passed |
| Full Playwright regression run | 186 passed; 3 failures identified and corrected |
| Final focused Playwright run | All 51 passed in 2.2 minutes, including the 3 corrected cases and 3 added same-device reconnect cases |
| Formatting and strict native/WASM Clippy | Passed |
| Frontend/native builds and whitespace checks | Passed |
| Real Apple Vision OCR | Passed against a synthetic image |
| Real Apple speech and Foundation Models | Passed through the Rust HTTP pipeline with synthetic English audio |
| Real MLX language inference | Passed through Rust HTTP with `mlx-community/Qwen3.5-9B-4bit`; answer and token usage verified |
| Native TextEdit insertion | Pending: macOS Automation permission blocked the disposable-document smoke test |

Across the full and focused runs, all **192 distinct browser cases** have passing results. The focused rerun covered connection recovery, translations, the new workspace, and local runtime settings in desktop Chromium and iPad/iPhone WebKit. The three full-run failures concerned two stale connection-error displays and five missing translated labels; their regression tests passed after the fixes. The added reconnect test verifies that replacing a CLI connection with the same device ID immediately turns Auto off and cannot replay note commands. Device browser coverage is emulation, not physical iOS hardware.

Tests cover per-wallet mode persistence and isolation; admin local-only enforcement before job creation; rejection of remote MLX endpoints; local worker bounds, sanitized errors, timeout and cancellation; actual loopback streaming; speech routing by job provider; Auto default-off, deduplication, stop, and disconnect; exact-content one-time cloud approval; stale-note/mode rejection; and usage isolation/erasure. Token-accounting checks also caught and corrected the placement of the OpenAI-compatible `stream_options.include_usage` option. The real MLX run used an already-installed model with Hugging Face/Transformers offline flags and a temporary service on port 18081; the service was stopped afterward.

Desktop and phone light/dark screenshots were visually inspected. Generated imagery and its prompt are stored in `frontend/assets/invisible-ai-hero.png` and `frontend/assets/invisible-ai-hero.prompt.txt`. Browser artifacts remain under ignored `test-results/` and `playwright-report/`; saved design previews are under `docs/screenshots/`.

Limits: actual TextEdit permission and insertion are not yet verified on this Mac. MLX speech has fixture coverage only. Local-runtime checks are not a packet-level egress audit. Wiki generation, PocketSkynet encryption, runtime supervision, full billing/credit APIs, and broad retention policy remain planned. The Apple worker does not report token counts; absent metrics, credit, and USD savings remain unknown. See [setup and current limits](invisible-ai.md) and [work plan](../TODO.md).

```sh
make test-rust
npm run test:scripts
npx playwright test
npx playwright test tests/e2e/connection.spec.mjs tests/e2e/language.spec.mjs tests/e2e/invisible-ai.spec.mjs tests/e2e/local-ai.spec.mjs
make check
make test-vision
cargo test -p causewaybay-server --test local_ai live_apple -- --ignored --nocapture
# Requires an already-installed local MLX service; see invisible-ai.md:
cargo test -p causewaybay-server --test local_ai live_mlx -- --ignored --nocapture
# After granting macOS Automation permission:
cargo test -p causewaybay-cli live_textedit -- --ignored --nocapture
```

## Earlier verification — 2026-10-02

Tested locally on Apple Silicon macOS with Rust 1.98.1, Trunk 0.21.14 and Playwright Chromium. Tests used isolated temporary backend/config folders, so the default user workspace was not used as a fixture.

## Interface languages

The web interface supports English (`en`), Korean (`ko`), Cantonese (`yue-HK`, Traditional Chinese), Simplified Chinese (`zh-CN`), Japanese (`ja`), Spanish (`es`) and Czech (`cs`). A native-script language selector is available before login and in the app header. The per-site preference uses browser local storage (`causewaybay.language.v1`), persists through reload/logout, updates other open tabs, and changes the document language and title. Blocked storage falls back to an in-memory selection with an explanatory notice; invalid saved values use English.

The translation catalog covers menus, chat actions, destructive confirmations, accessible labels, login/registration, device setup, notes/files/images, settings, administration and common status messages. Named placeholders permit translated word order without rewriting interpolated values. User content, saved room titles, files, AI output and code remain unchanged. Language changes update existing components without discarding drafts, attachments, login inputs or active room streams.

Three frontend unit tests cover locale parsing/fallback, template interpolation, literal user-supplied placeholder values and incomplete templates. Thirteen added browser checks cover catalog completeness and placeholders, all seven languages through menus/dialogs/reload, wallet login/logout, message editing with a translated UI, preserved drafts/content/subscriptions, cross-tab preferences, unavailable storage, and mobile layouts. Screenshots are under `test-results/language-<locale>-mobile.png`.

The two-tab test reproduced an existing HTTP connection-limit problem: each chat tab opened two identical notification SSE connections. Notification listeners now share one stream per tab, receive every event directly, and release the stream when the last listener leaves or the authenticated session changes. The test asserts one notification connection per tab and successful reload while another chat remains open.

Final verification passed **all 53 fixture/process/browser tests** and **all 3 new frontend unit tests**, plus formatting, strict native/WASM Clippy and the frontend build. Desktop and translated mobile screenshots were inspected. The existing live-provider checks were not repeated for this UI change; their earlier results are recorded below.

## Menu and navigation review

The primary menu now contains New thought, private note search, Chat, Wiki, Autonomous AI and Devices (as of 2026-10-09). Library, Files, Images, Videos, Code, Dashboard, Multi-device capture and the quiz playground are under More tools, which opens by itself on those pages. Server administration is labelled Backend settings. Personal AI/model choices remain in the chat's AI & tools controls.

Menu navigation updates the URL, handles browser Back/Forward, and restores every menu page after reload. Saved memos have reloadable `/memos/{id}` links; known app routes return HTTP 200. Stale memo responses cannot replace a newer navigation. Creating/selecting another chat leaves the fixed New thought view. Keyboard submission only runs from the visible composer; Escape and menu selections close the mobile drawer, and hidden navigation is removed from keyboard focus order.

New users get an explicit capture setup path. Devices shows a command using the actual backend origin and the signed-in wallet, including for non-admin users. Files → Use in chat now uploads a copy into the destination room, instead of dropping the attachment while switching views. Clearing that room removes its copy and preserves the independent library original. Compact chat controls keep Capture visible on desktop and mobile.

Five browser regressions cover navigation/reload/history, saved memo details, fixed-room and keyboard behavior, file handoff/erasure, and mobile drawer/settings behavior. Existing environment and wallet tests also verify the device setup path and owner-specific command. Screenshots are saved as `navigation-desktop.png`, `navigation-mobile.png`, `messenger-desktop.png` and `messenger-mobile.png` under `test-results/`.

Final menu/flow verification passed **58 native tests** and **all 50 integrated Playwright tests** (40 fixture/process/browser checks and 10 live Grok workflows). Formatting, strict native/WASM Clippy, native builds and the frontend build also passed. Desktop and mobile screenshots were visually inspected. Live AI coverage uses Grok and synthetic capture fixtures; this does not establish compatibility with every provider or physical device.

## Private messenger refactor

Final follow-up verification passed **58 native tests**, **35 fixture/process/browser tests**, and **10 live Grok workflows**, plus strict native/WASM Clippy, formatting and the WASM build. Native tests comprise 11 CLI, 12 server unit, 12 API, 5 authentication, 7 chat/media, 2 messenger sync, 2 connections, 3 persistence and 4 shared tests. A separate real MiniLM embedding test also passed during the refactor. All live checks used synthetic capture fixtures and isolated temporary storage.

The integrated browser run passed 44 tests and found one outdated provider selector after AI options moved into a collapsed control. After correcting that test and checking both legacy and wallet-scoped provider preferences, the complete 35-test fixture/browser suite passed in 1.1 minutes. All ten live workflows passed in the integrated run; they did not need repeating for that test-only selector change.

The follow-up regression tests add coverage for concurrent edits to both user and AI messages, paginated sync with int64 cursors above `2^53`, clock persistence after erasure/restart, and rejection of stale writes after a clear. Notification SSE tests replay more than one batch, verify `Last-Event-ID` takes precedence, exclude another wallet's and device's data, and confirm an open stream ends after logout. Pending-capture tests now use the actual capture queue rather than demo mode, assert the job is still running before mutation, and verify late image results cannot restore content after editing or room clear/delete.

Two process tests clear/delete a room while its Rust client is offline, restart the backend, reconnect using saved client identity, and verify its capture file disappears while another room's capture remains. Capture records and local files are synthetic fixtures; notification delivery and deletion use the actual backend and CLI. These tests require Node.js 22.13+ for the built-in SQLite fixture API.

A browser regression holds an IndexedDB transaction to force overlapping room-cache reads, then injects delayed/duplicate SSE records after a clear and checks the UI and persistent cache. It reproduced a lost-subscription bug: dropping an older room subscription could close its replacement. Cache-load generations and closing the old subscription before registering the new one fix this; the same deterministic test failed before the fix and passed afterwards. `make test-rust` now clears ambient wallet-admin configuration, matching the isolated browser fixtures.

Verified behaviors include:

- Wallet-only room/memo/file/image reads and mutations, BM25/vector search, AI retrieval tools and notifications; administrators cannot bypass content ownership. Legacy backend drafts are excluded from API settings.
- Deterministic wallet display names, stable New thought rooms, per-wallet browser preferences, GUID message/room identities, monotonic microsecond revisions, edit conflicts, edits/removal during active capture, and replay after reconnect/restart.
- Clear/delete erases attachments, generated files and notes, screenshots, Markdown, search entries, revisions, submissions and tool results; interrupted cleanup completes before startup serves data. Clearing preserves the room and accepts new messages. Background indexing cannot recreate deleted vectors.
- Two-browser SSE edit/remove/rename/delete; an offline browser reconnecting on the Files page removes its cached room content. Rust capture deletion validates the request GUID, is idempotent, and cannot escape its screenshot directory.
- Persistent browser/CLI WebSockets, reconnecting SSE, backend-only API credentials, automatic calculation/private-note search, web search with clickable source references, code generation/compilation, image and text capture, continuous capture, and audio transcription.
- Desktop/mobile layout inspection, automatic scroll that respects reading older messages, local message times, and compact AI options on phones.

```sh
make check
make test-rust
make frontend
npm run test:e2e
cargo test -p causewaybay-server --test semantic -- --ignored --nocapture
RUN_LIVE_AI=1 npm run test:e2e
```

The live provider used for this refactor was Grok; OpenAI, Anthropic and Gemini search adapters were implemented against their official request/response contracts but were not live-tested. Release packaging and capture of the actual desktop were verified in the baseline below; the refactor's capture tests use known fixture images through the real Rust client and real Apple Vision. No production account data was migrated or erased during these tests.

## Baseline results (before the private messenger refactor)

| Check | Final result |
| --- | --- |
| Rust formatting and strict Clippy, native + WASM | Passed, no warnings |
| Native unit/integration tests | **39 passed** |
| Apple Vision OCR on a rendered image | Passed; recognized `CAUSEWAYBAY` and `123` |
| Real local MiniLM embedding inference | Passed in the baseline implementation; related meanings outranked unrelated text |
| Full integrated Playwright suite | **38 passed in 1.9 minutes** |
| Native macOS desktop capture | Text and Image modes passed, plus failed-capture reporting |
| Live Grok | Text, image, coding tools, multi-client capture, chat, continuous capture and AI status check passed |
| Real audio processing | xAI transcription of a WAV upload and the resulting chat answer passed |
| Release build | Passed; server, CLI, both aliases and WASM/local Tailwind assets |
| Release browser smoke test | Passed against packaged release binary and assets |
| Backend credential handling | Environment/`.env` precedence, malformed-file redaction and no browser key setup passed |
| Secret scan | Actual configured API keys absent from Git-candidate files; `.env` ignored |

The 39 native tests comprise 2 CLI, 10 server unit, 12 API, 3 authentication, 3 chat/media, 2 connections, 3 persistence and 4 shared identity/quiz/config tests. Separately gated real OCR, embedding and cloud tests are not included in that count.

The final integrated command was:

```sh
TEST_PACKAGE=1 RUN_LIVE_AI=1 RUN_NATIVE_CAPTURE=1 npm run test:e2e
```

## Reproduce

```sh
make check package       # formatting, strict Clippy and release build
make test-rust
make test-vision         # macOS Apple Vision; generates the known screenshot fixture
make test-audio-fixture  # macOS Samantha voice, fixed English arithmetic question
make test-semantic       # real local embedding model, first download may be needed
make test-cloud          # live Grok, requires backend XAI_API_KEY in environment
TEST_PACKAGE=1 RUN_LIVE_AI=1 RUN_NATIVE_CAPTURE=1 npm run test:e2e
```

For targeted checks, use `make test-auth`, `make test-live`, `make test-native-capture` or `make test-package`. Live/native tests require the API key and macOS Screen Recording permission. Real OCR required running outside the execution sandbox so Apple frameworks could initialize. Fixtures never silently replace production OCR/AI.

`make test` runs ordinary Rust and fixture-based browser checks. The Playwright configuration clears the developer's wallet-admin environment for ordinary fixtures; wallet tests start a separate approval-enabled backend with public test identities. No real mnemonic or private key is needed by the tests.

## Coverage by workflow

| Area | Verified behavior |
| --- | --- |
| UI | Desktop/mobile layout, generated artwork, compiled local CSS, image/text modes, chat, dashboard, admin and quiz views |
| Credentials | Working-directory/config-directory `.env`, exported values win (including empty values), redacted parse failures, no API-key UI or accepted key request fields |
| Wallet login | BIP-39/BIP-32 address vectors, external EIP-191 signature vector, challenge verification/replay, cookies, no mnemonic transmission/persistence |
| Approval/admin | Registration-only access before approval, waiting list, approve/promote/demote/block, permanent owner, HTTP/WS authorization, logout/revocation |
| Node ownership | Positional URL plus owner, shared URL, persistence, owner-only capture with no admin bypass |
| Capture | Real CLI WebSocket and SSE, selected node, subset/all nodes, preview before completion, correct mode phases, errors without false delivery acknowledgement |
| Reconnection | CLI before backend startup, backend restart, stable JSONL identity, browser two-second retry, severed SSE replay without duplicate answer or AI submission |
| Presence | Rust/web join/leave counts, stale heartbeat cleanup, old disconnect cannot clear a newer connection |
| Chat | GUID room/message identities, idempotent requests, int64 revisions, HTTP sync/SSE replay, wallet-scoped IndexedDB message cache, reconnect/mobile layout |
| Continuous capture | Repeated real CLI callbacks/OCR/Grok messages, selection, visible Stop, cancellation and no new captures after Stop |
| Media/library | Original images on disk, text/image chat display, exact OCR, audio transcript, text/Markdown/CSV/raw processing, downloads and deletion |
| Storage | JSONL restore/validation, SQLite restart, relative paths, legacy blob migration, relocation, file versions, Markdown regeneration, deleted/missing bytes not recreated |
| Search | Hashtags, FTS5 BM25, local embeddings and ranking fusion |
| Coding agent | Tool events, actual write_file/compile_code, saved compiler diagnostics, valid/invalid Rust, no program execution |
| Quiz playground | Random math correctness/new numbers and Rust quicksort/sliding-window/producer-consumer prompts |
| Lifecycle/package | Makefile start/stop, 0.0.0.0 bind, web/API/dashboard/Tailwind URL output, non-loopback access, packaged WASM/assets |

## Real capture and cloud evidence

The regular live browser tests used the actual Rust CLI with a known screenshot fixture, the backend, Apple Vision, Grok, SQLite, and an iPad-sized Chromium viewport. Text mode extracted the arithmetic question; Image mode sent the picture without an OCR event. A real Grok coding-agent response invoked `write_file` and `compile_code`, and the saved Rust function compiled successfully.

The separate native suite displayed an arithmetic page on the primary desktop and invoked the actual `/usr/sbin/screencapture` through the CLI, with no fixture override. Both modes answered `12`. Assertions covered screenshot visibility, OCR/AI events, matching client/backend PNG bytes, relative backend storage, Markdown and final SQLite state. An invalid display produced a visible error without a false successful-capture acknowledgement.

Audio testing uploaded an actual WAV file, obtained a real xAI transcript, used it in a chat answer and checked backend file storage. Continuous capture produced multiple real captures/answers and stopped on request.

## Regressions and test-fixture corrections

- Native capture originally failed with “No such file or directory” because macOS could return exit status zero while refusing hidden temporary PNG filenames. The CLI now uses visible UUID PNG names and checks both process status and output-file existence.
- Stream/reconnect tests exercise split SSE frames, durable event cursors and connection generations, preventing duplicate deltas and stale presence.
- The new-room test initially read the previous selected room before the asynchronous create response completed. It now waits for the selected room GUID to change before starting/stopping a capture loop.
- The machine's default speech voice initially produced an unsuitable English audio fixture. The fixture now explicitly uses Samantha at a fixed rate.
- One native Text-mode test captured the previous desktop while macOS was animating the fullscreen Space transition. The test now waits for that native transition and verifies OCR contains the intended arithmetic question before checking the AI answer. The final full run passed all 38 tests.

These earlier failed runs are superseded by the final integrated pass, not counted as successful tests.

## Artifacts and limits

Generated screenshots and failure traces are under `test-results/`; the HTML report is `playwright-report/index.html`. These are ignored by Git and replaced by later runs. Representative screenshots include `home-desktop.png`, `home-mobile.png`, `dashboard-desktop.png`, `dashboard-mobile.png`, `admin-waiting.png`, `chat-desktop.png`, `chat-mobile.png`, `live-chat-modes.png`, `native-text-capture.png`, `native-image-capture.png` and `release-desktop.png`. Desktop/mobile screenshots were visually inspected.

The server release archive is `package/invisible-ai-server-<version>-darwin-arm64.zip`. Its `bin/` directory contains exactly `server`, `client` and the local AI workers. It also includes web assets, `.env.example`, QUICK, README, PLAN, PROTOCOL and supporting docs; it excludes real `.env`, runtime user data and source-build compatibility aliases. Package regression tests verify the exact binary list in both the directory and archive, including removal of aliases left by older builds.

Live account verification used Grok/xAI only. Anthropic, OpenAI and Gemini provider request/stream/tool formats have fixture tests; live credentials for those providers were not tested. The three-device topology was exercised as separate local processes and browser viewports, not a physical Mac Studio/MacBook/iPad deployment. Actual iPad Safari and a separate MacBook's Screen Recording permission remain device checks.

Continuous capture is periodic screenshots. The compiler checks standard-library Rust without running output or installing packages; macOS now restricts it with an OS sandbox. Wallet data is private through the API, including administrator requests. Cryptographic device enrollment and end-to-end encryption are not implemented: capture nodes still rely on the configured node credential and trusted network. Backups, downloaded copies, and legacy client captures without request-GUID filenames cannot be recalled automatically.

See [PLAN.md](../PLAN.md) for architecture/progress and [PROTOCOL.md](../PROTOCOL.md) for HTTP/WebSocket/SSE contracts.

## Additional client persistence and retry checks

After adding no-argument connection restoration, the targeted CLI suite passed **10 unit tests** and **6 process/browser integration tests**. These are separate from the baseline full-suite results above. Formatting and CLI Clippy with warnings denied also passed.

Unit coverage includes saved URL/owner/node settings, explicit overrides and shared access, invalid configuration, interrupted JSONL writes, exclusion of connection tokens from saved files, the default retry interval and its bounds. Tokio's paused clock checks that retry waits cannot complete early at 1, 3 or 60 seconds and that shutdown interrupts a pending wait.

Integration coverage launches real CLI/backend processes in temporary directories. It verifies first-run help, no-argument restoration for shared and owned nodes, rename persistence, spaced retries while the backend is offline, ownership-preserving recovery after a backend restart, and prompt Ctrl+C exit during a 60-second retry wait.

```sh
cargo test -p causewaybay-cli
cargo build -p causewaybay-cli -p causewaybay-server
npm run test:e2e -- tests/e2e/clients.spec.mjs tests/e2e/reconnect.spec.mjs --grep 'without saved|no-argument|URL shortcut|Rust CLI'
cargo clippy -p causewaybay-cli --all-targets -- -D warnings
cargo fmt --all -- --check
```

## Device choices, connection addresses, and themes — 2026-10-02

The final full fixture Playwright run passed **58 tests in 1.7 minutes**. The native server/shared/CLI Rust suite passed; the frontend passed **8 unit tests**. `make check` passed formatting and strict native/WASM Clippy, and `make frontend` plus server/CLI builds succeeded.

New coverage launches two real Rust capture clients to verify explicit single-device routing, default selection, local-storage persistence, cross-tab updates, independent continuous-capture selections, and offline/reconnect behavior without an implicit device switch. Wallet tests verify that an ordinary user can select an owned default, an admin cannot capture that private node, and switching wallets restores the correct browser preferences. A transient initial New thought request now recovers after reconnection.

Address tests use an isolated Tailscale status fixture with real backend interface discovery and confirm that the Rust command prefers the Tailscale IP. Every Web/API URL and command is copied and compared against the actual Chromium clipboard. Native tests cover listener restrictions, stopped Tailscale, actual ports, IPv6 brackets, and malformed DNS. Theme tests cover reload, other tabs, login/logout, unchanged drafts, and mobile access. Light/dark desktop and mobile screenshots were inspected; initial-viewport Capture access is covered by the messenger regression test.

These tests use demo AI responses and temporary data/config directories. They do not verify a physical remote capture computer or live tailnet reachability; opt-in cloud, Vision, and embedding-model checks were not rerun for this UI/address change. Restart an already-running backend to publish the newly discovered address list, then refresh the browser.


## Automatic Capture intent — 2026-10-02

Verified all five requested behaviors against live Grok using synthetic screenshots, real Rust capture clients, isolated backend data, and an empty prompt on each request: coding produced a complete Python addition function; math solved `3x + 5 = 26` as `x = 7`; the idea case assessed tradeoffs and suggested an improvement; messenger context identified the request, blocker, and Thursday deadline; the visual question correctly counted three blue squares without relying on OCR text. The idea and image cases passed a targeted rerun after refining evidence/uncertainty guidance and accepting an equally correct concise answer format.

The live checks are opt-in: `RUN_LIVE_AI=1 npm run test:e2e -- tests/e2e/automatic-capture.spec.mjs`. They use the backend's configured Grok key and make live provider calls. They validate representative examples, not every possible model response. Chat now defaults to full-image input when no local mode preference exists; the browser regression test also verifies that an explicit Text mode choice survives reload.

After the Capture-intent change, the full normal browser suite passed **59 tests in 1.7 minutes**. The frontend's **8 unit tests**, frontend/server builds, formatting, and strict native/WASM Clippy also passed.

## Multiple photos and comment context — 2026-10-02

Added multiple attachments per message (up to eight), individual draft removal, photo previews, comments, and follow-up messages sharing the same chat context. Provider requests now contain all current photos and recent historical photos, with an explicit 16-image budget. History uses current message revisions, so edits update comments and deleted messages/attachments leave subsequent context. Text mode never forwards historical image bytes. Small images are no longer enlarged before sending.

The full native server/shared/CLI suite passed, including new tests for distinct images in context, attached document text, edited comments, removed photos/messages, room erasure, Text mode, image budgets, atomic file claims, wallet isolation, and idempotency. All four provider request formats are checked with two images plus a comment. The frontend's eight unit tests and strict native/WASM Clippy passed.

The full normal Playwright suite passed **61 tests in 1.8 minutes**, including photo sets, individual removal, later photo/comment messages, reload, the eight-file limit, and permanent room cleanup. A final mobile layout adjustment expands image conversations instead of squeezing them into the compact capture area; **10 targeted chat/messenger checks passed** after that adjustment. The resulting mobile screenshot was visually inspected.

The opt-in live Grok test passed a three-turn conversation in **11.8 seconds**: two original photos with a project code, a third photo with a comparison request, then a comment-only follow-up. The first answer was deliberately limited to an acknowledgement, so the second answer had to inspect the original images to correctly count two red circles, three blue squares, and one green triangle. The final answer retained the project code and identified green as the smallest group. Synthetic pictures only; no private screen contents were submitted.

Run that live check with `RUN_LIVE_AI=1 npm run test:e2e -- tests/e2e/automatic-capture.spec.mjs --grep 'follow-up photos'`. Other providers have request-format coverage but were not called live for this change.

### Final confirmation — 2026-10-02

Rechecked the current build: **62 native tests**, formatting/strict native and WASM Clippy, and **10 chat/messenger browser tests** passed. The live recheck caught one correct messenger explanation that omitted its hidden category marker and was therefore saved as a generic question. Tightened the prompt to require the marker on every answer and explicitly classify messenger context explanations as `reply`. All **six live scenarios passed on the subsequent run (42.8 seconds)**, including all five prompt-free Capture tasks and the three-turn photo/comment conversation. These checks use synthetic images and isolated servers; they do not restart the user's running backend. The updated prompt takes effect when that backend starts the rebuilt executable.


## Capture thumbnails and iPad WebKit — 2026-10-03

The Rust capture client still transfers the original PNG to the backend byte for byte. The browser now requests a JPEG thumbnail bounded to 1600 pixels on either edge. Screenshot and uploaded-photo thumbnails open the original in a new tab; explicit downloads retain their download behavior. Derivatives are generated on demand, with bounded decoding and two conversion workers, and are never persisted. The thumbnail and full-image endpoints enforce the same wallet ownership as the source files.

Verification passed 9 targeted native tests (thumbnail conversion/endpoints, wallet isolation, and multimodal context), formatting and strict native/WASM Clippy, debug builds, and `make build` release output. All 15 targeted browser checks passed, including Chromium and iPad WebKit through the LAN address with separate authenticated localhost and network browser contexts. An additional iPad WebKit run through the Tailscale IP also passed. The image test exercises the real Rust client, WebSocket transfer, backend persistence, room SSE updates, image decoding, full-image clicks, uploaded-photo previews, reload, and permanent room clear. It uses a multi-megabyte synthetic Retina image; demo AI and a fixture capture-queue/message revision avoid paid model calls. A first-login settings race found by this test was fixed so Capture cannot submit an empty provider/model while settings arrive.

The iPad WebKit screenshot was visually inspected. These are browser-engine tests on macOS, not verification on the physical iPad Mini. To repeat just this path: `npm run test:e2e -- tests/e2e/capture-image.spec.mjs`; set `CWB_E2E_NETWORK_HOST` to a reachable address on the test machine to exercise a particular LAN/Tailscale IP or DNS name. `make install` now installs WebKit alongside Chromium.

## Privacy sync, capture workflow, Dashboard, and client package — 2026-10-03

Added confirmed all-content erasure scoped to the authenticated wallet, durable recovery after interrupted cleanup, fresh room-clear cursors (including empty rooms), latest-state-only message replay, and removal of duplicate submission text. Removing a prompt also removes its generated answer, generated files/notes, and unreferenced attachments. SQLite/FTS/WAL regression checks verify that unique erased payloads do not remain in application records. This is application erasure, not a forensic disk-wipe guarantee; external backups/downloads are outside its scope.

Browser checks cover offline and signed-out sessions returning after message deletion, clear, room deletion, and Delete all; other-page IndexedDB cleanup; delayed/duplicate replay; preservation of new content after old erasure notifications; and file reads that finish after erasure. Browser uploads, staged snapshots, and Think requests carry observed erasure generations so the backend can reject stale requests before writing. Real Rust client processes verify durable screenshot removal after an offline period and backend restart, without recording a real screen. In-progress screenshots use request-specific temporary directories, preventing a cancelled native capture from recreating the final image.

Multi Capture stages screenshots without a model call and submits exactly one turn on Done. Browser clipboard checks compare code-only, whole-message, and conversation text. The Dashboard uses real personal counts and admin-only system totals, all web/Rust client records, queue state, media storage, users, and provider configuration. Three.js exponential tweening and cosine motion respect reduced-motion preferences; readable HTML remains when WebGL is unavailable. Desktop/mobile light/dark screenshots were visually inspected, and new interface text is localized across the existing seven languages.

`make package` creates the server release plus `package/invisible-ai-client-<version>-darwin-arm64.zip`, one folder with `client`, `go.sh`, `.env.example`, `QUICK.md` and `README.md`; `client` and `go.sh` are executable. Script tests check that `go.sh` fills missing values from `.env` without executing it, and that baked values and the environment win. Tests unpack into a directory with spaces and verify the exact launcher arguments. A wallet-authenticated browser download uses a Tailscale discovery fixture to verify the actual ZIP response, current wallet, backend port, executable bytes, and absence of embedded secrets. The test does not claim connectivity to a physical remote node or notarized distribution.

Formatting, native/WASM Clippy, and all 80 offline Rust tests passed; the four opt-in cloud/model/Vision tests were not rerun for this change. All 78 browser tests passed in the complete release run (`TEST_PACKAGE=1 npm run test:e2e`), including Chromium, iPad WebKit, both ZIP paths, and the unpacked release server. Dependency audit found no advisory in the added Three.js/esbuild packages; the existing Tailwind 3 build dependency chain still reports five high-severity entries stemming from the same braces advisory. A Tailwind major-version migration was not included.

### Additional privacy and packaging regressions — 2026-10-03

Added two Rust integration tests. Reply-only deletion preserves the prompt; deleting a prompt removes its paired answer. An explicitly reused attachment survives while another live message references it and disappears with the last reference, including after reopening the database. Authenticated repeated Delete all requests erase only the session owner even when a different wallet is supplied, retain login/approval, return zero content totals on the Dashboard, advance the deletion epoch even for an empty account, and reject pre-erasure uploads after restart while accepting fresh ones.

Added seven standalone Node.js tests in `tests/scripts/client-package.test.mjs`. These build and unpack ZIPs in temporary directories using a harmless executable fixture. They cover stale archive entries, executable permissions, invalid wallets/non-Tailscale URLs, IPv4 range boundaries, IPv6 and custom ports, exact launcher arguments from a directory with spaces, child exit status, missing wallet/token prompts, environment tokens, and exclusion of build-time secrets. The stale-archive test failed before the fix: a reused staging ZIP retained an old `.env` entry because `zip` updates existing archives. Packaging now removes the temporary ZIP first and explicitly restores `go.sh` permissions on rebuild.

`make test-scripts` runs the standalone tests without a backend or release build. Both `make test` and `make verify` include them; `make test-package` also exercises the authenticated personalized Dashboard download.

Final rerun passed **82 Rust tests, 7 launcher tests, and all 78 browser/release tests** (2.9 minutes), including iPad WebKit. Formatting, strict native/WASM Clippy, JavaScript/shell syntax checks, and the frontend/debug builds also passed. The macOS compiler test required running outside the outer sandbox so its own `sandbox-exec` could start. Four opt-in native cloud/model/Vision tests and opt-in live/physical capture browser workflows were not run. No production data was erased or running production backend restarted.

```sh
make check test-rust build-debug
make test-scripts
TEST_PACKAGE=1 npm run test:e2e
```

### Playwright capture/privacy coverage — 2026-10-03

The full Playwright run passed **84 tests in 3.1 minutes**: 80 Chromium checks and four iPad WebKit checks. All six new capture/privacy checks passed across the two engines. Existing clipboard, wallet login, deletion replay, Dashboard, client ZIP download and release package checks also passed. The HTML report is `playwright-report/index.html`; these checks emulate iPad WebKit on macOS rather than running on a physical iPad.

Added `tests/e2e/capture-privacy.spec.mjs` and enabled it in both Chromium and iPad WebKit. Two scenarios hold unsent Multi Capture photos in an offline browser while another browser clears the room or deletes all data. On reconnect, staged photos and the draft disappear; the original downloads and thumbnails return 404; a fresh single Capture produces one new turn without erased content. A third scenario fills all eight capture slots, verifies the capture limit and Enter guard, removes all attachments to disable empty Done, then verifies one remaining capture produces exactly one submission and response. These checks use the real UI, WebSocket transport, server persistence and replay, with synthetic images and an isolated demo backend. Before/after screenshots are attached to the Playwright HTML report.

```sh
npm run test:e2e -- tests/e2e/capture-privacy.spec.mjs
TEST_PACKAGE=1 npm run test:e2e
```

## Responsive layouts and visible appearance toggle — 2026-10-03

Added a labeled Light / Dark toggle to the sticky application header and login screen, with a 44-pixel touch target and translated labels. Theme choice remains available before authentication and persists through reload, login/logout, and other tabs. Phone and tablet portrait layouts use a navigation drawer; the composer uses larger touch controls, dialogs respect the dynamic viewport, and safe-area spacing supports inset screens. AI connection details remain available under AI & tools. Touch inputs use at least 16-pixel text to avoid Safari's input zoom.

New Playwright coverage exercises desktop Chromium and iPad Mini/iPhone 13 WebKit profiles: both themes across chat, Dashboard, Files, Memos, and Devices; drawer dismissal; long URLs and code; draft preservation during rotation; settings-dialog controls at shorter heights; pre-login appearance; and all seven languages at a 320-pixel viewport. Existing privacy/capture and cross-tab theme checks now also run in iPhone WebKit. Checks caught and fixed hidden file-input overflow in WebKit and a drawer backdrop whose clickable bounds extended beneath the drawer. The image-library capture test now opens the tablet menu before selecting Images.

The complete Playwright run passed **all 98 tests in 3.5 minutes**: 83 Chromium checks, eight iPad WebKit checks, and seven iPhone WebKit checks, including release/client packages. Desktop, iPad, and iPhone screenshots in both themes were visually inspected. Screenshot review then found that populated chats could shrink to a 60-pixel reading area on phones; they now reserve at least 220 pixels. Empty chats still hide that area, and populated chats allow page scrolling to the capture controls. A regression assertion checks readable height and separation from the composer across viewport sizes. After this final layout adjustment, **all 32 targeted chat, responsive, theme, capture, and privacy tests passed in 1.5 minutes** across the three browser profiles. The current HTML report contains this final targeted run.

Formatting, strict native/WASM Clippy, and all eight frontend unit tests passed. The frontend and release archives were rebuilt. These are browser-engine/device-emulation checks on macOS; physical iPhone/iPad keyboards, display insets, and Safari behavior still require device testing. Paid live AI and physical screen-capture workflows were not rerun for this UI change.

```sh
make check
cargo test -p causewaybay-frontend
make package
TEST_PACKAGE=1 npm run test:e2e
```

## Faster response image previews — 2026-10-03

Reduced browser thumbnail output from a 1600-pixel edge at JPEG quality 82 to a 960-pixel edge at quality 78. Chat screenshots, message attachments, staged photos, saved responses, multi-device answers, file previews, and the image library use thumbnails with lazy loading and asynchronous decoding. Full-resolution images remain available on click; original capture and upload bytes are unchanged. Thumbnail responses retain wallet authorization and `no-store`, with no derivative files or new persistent cache.

Both native thumbnail tests passed, covering landscape/portrait bounds, transparency, small images, decoder limits, unchanged originals, and deletion. All **17 targeted Playwright tests passed in 1.5 minutes** in desktop Chromium and iPad/iPhone WebKit profiles. The capture test now also verifies the saved response page: only the thumbnail is requested, preview decoding succeeds at 960 × 620, and the full-image link targets the original. The large synthetic PNG's thumbnail must transfer less than one-fifth of the original bytes. Existing multi-photo, full-image click, offline deletion, and Multi Capture checks passed. Formatting, strict native/WASM Clippy, and frontend/debug/release builds passed; the release archive was refreshed.

```sh
env CWBBOT_ADMIN= cargo test -p causewaybay-server --test thumbnails
make check
npm run test:e2e -- tests/e2e/capture-image.spec.mjs tests/e2e/capture-privacy.spec.mjs tests/e2e/chat.spec.mjs
```

## Add files for AI context — 2026-10-03

The chat composer now exposes **Add files**, with per-file names, sizes, context labels, removal controls, and audio previews. Markdown, text/code, images, and sound files can be mixed and sent without a typed prompt. Unknown binary formats remain attachable and are explicitly labeled **Metadata only** rather than claiming their contents were read. Audio players use `preload="none"`; transcription starts on submission through the existing configured Grok/Gemini path. Labels and guidance are translated into all seven interface languages.

Extended audio recognition for ID3-tagged files without an extension, MPGA, WebA, Ogg, FLAC, and AIFF filename variants. Fixed a history budget bug where a long typed prompt consumed the entire initial text allowance and dropped its audio transcript or OCR from follow-up context. Those sections now have separate bounded allowances within the existing final history limit.

All **15 targeted native tests** passed across `chat_media`, `file_context`, and `multimodal`. The new regression submits Markdown, audio, and an opaque file together with a 9,000-character comment, inspects the resulting model input and follow-up history, then verifies permanent room cleanup. All **38 targeted Playwright tests passed in 1.1 minutes**, including the new mixed-file workflow on desktop Chromium and iPad/iPhone WebKit, all seven languages, responsive light/dark layouts, capture privacy, and existing chat attachments. The browser test checks actual memo context, no automatic audio download, remove/re-add, reload, follow-up, and 404s for erased files and memo. Desktop/light and iPhone/dark screenshots were visually inspected.

Formatting, strict native/WASM Clippy, frontend and release builds passed. The release archive was refreshed. Audio transcription in these checks uses the explicit demo fixture; paid live transcription providers were not called. Device coverage uses WebKit emulation on macOS.

```sh
env CWBBOT_ADMIN= cargo test -p causewaybay-server --test file_context --test multimodal --test chat_media
make check
npm run test:e2e -- tests/e2e/file-context.spec.mjs tests/e2e/responsive.spec.mjs tests/e2e/chat.spec.mjs tests/e2e/language.spec.mjs tests/e2e/capture-privacy.spec.mjs
```

## Add any file and document context — 2026-10-03

Renamed the composer action to **Add any file** and added local text extraction for selectable-text PDF, DOCX, XLSX, PPTX, and OpenDocument text/spreadsheet/presentation files. ZIP attachments provide an entry listing without extracting nested content. Dotfiles and extensionless files are accepted, while uploaded filenames cannot contain paths or control characters. Unsupported binary formats remain attachable with an explicit **Metadata only** label. Original bytes remain unchanged and downloadable.

Document parsing uses two blocking workers, bounded text output, and bounded Office XML decompression. PDF excerpts cover at most 200 pages; encrypted or image-only PDFs return an actionable error. Office documents are limited to 200 slides/sheets, 4,096 archive entries, 8 MB per XML part, and 32 MB total XML reads. DTDs are rejected; external relationships are not fetched, and macros/formulas are not executed. Spreadsheet formulas use cached values. Each attachment gets a share of the current context allowance, preserving space for other files. Extracted text is not stored as an additional derivative file; normal memo/history data follows the existing erasure lifecycle.

All **25 targeted native checks passed** across document parsing, file context, multimodal context, chat media/privacy, and thumbnails. Coverage includes Unicode, entities, logical slide order, shared strings, cached spreadsheet values, malformed documents, XML expansion limits, external links, eight large attachments, document follow-up context, and physical file removal. The large-file test uses a short prompt because the demo provider echoes and streams the prompt; a 90,000-character test prompt exceeded the test deadline while streaming, so it was reduced without changing the eight large attachments being verified.

All **41 targeted Playwright checks passed in 1.2 minutes** across desktop Chromium and iPad/iPhone WebKit. The new document scenario uploads eight mixed files through the real picker, checks their context labels and extracted memo text, compares downloaded originals byte for byte, reloads the chat, then clears it and verifies file/memo 404s. Existing light/dark, responsive layouts, seven languages, capture privacy, and chat checks passed. Desktop and iPhone document screenshots were visually inspected. Device coverage is WebKit emulation on macOS; the demo backend avoids paid AI/transcription calls.

Formatting, strict native/WASM Clippy, frontend/debug/release builds, and whitespace checks passed. The release bundle and client ZIP were refreshed.

```sh
env CWBBOT_ADMIN= cargo test -p causewaybay-server --test documents --test file_context --test multimodal --test chat_media --test thumbnails
make check
npm run test:e2e -- tests/e2e/file-context.spec.mjs tests/e2e/responsive.spec.mjs tests/e2e/chat.spec.mjs tests/e2e/language.spec.mjs tests/e2e/capture-privacy.spec.mjs
```

## Fresh problem context and fixed capture toolbar — 2026-10-03

Explicit **New thought** now sends `fresh:true` and creates an independent room. **New chat** also clears the current messages, prompt, staged attachments and retry state immediately. Room and draft generations reject late room creation, sync, snapshot, upload and submission callbacks. A failed reset leaves an empty draft instead of automatically reopening the previous room. Ordinary reload resumes the current thought; saved rooms remain available through the picker.

The production AI request builder no longer injects automatically retrieved library answers into room conversations. It uses only current-room history and explicitly supplied context. Existing conversations retain their follow-up history, and standalone memo requests retain their existing retrieval behavior.

The fixed bottom toolbar has four controls in order: **Capture · Text prompt · Multi Capture · Send message**. A single Capture starts one answer. Multi Capture stages screenshots; Send message or Enter submits the batch. Empty messages stay disabled, Shift+Enter keeps a newline, and reset/loading/empty-room states keep the toolbar visible but disabled. Thumbnail previews scroll within the dock. Connection and copy notifications are placed above the toolbar.

Regression coverage included in `make test`:

- Three server unit tests exercise text, one image and eight images across fresh thought rooms, new chat rooms and an existing conversation. They inspect OpenAI, Anthropic, Grok and Gemini request serialization, reject unrelated library answers and stale parent IDs, verify image counts, and retain valid same-room follow-ups.
- Three API integration tests exercise repeated resets, persistence across backend restart, pending responses in previous rooms, cross-room attachment rejection, intact saved photos, malformed reset requests and empty history for a new problem.
- Nine browser scenarios run in desktop Chromium and iPad/iPhone WebKit. They cover both reset buttons, repeated resets, one-click capture, multi-capture Enter submission, deliberately delayed snapshot/upload/submission/room responses, and failed resets from both home and chat routes. Delays are controlled at the WebSocket transport rather than by arbitrary sleeps.
- Responsive checks verify all four controls stay in the viewport at both scroll extremes, retain touch targets of at least 44 pixels, fit narrow/rotated/short viewports, and preserve the prompt across appearance changes. Existing tests retain eight-attachment limits, erasure replay, document/audio context and all interface translations.

Verification runs locally with `make check` and `make test` (or `make verify` for both). No GitHub Actions workflow is configured, as requested for this private repository. Browser reports, screenshots and traces remain local in `playwright-report/` and `test-results/`.

Final verification: **`make test` passed with 100 Rust tests, 8 launcher/package script tests, and 141 browser tests** (97 Chromium, 22 iPad WebKit, 22 iPhone WebKit). The browser suite completed in 6.0 minutes, including all 27 fresh-context browser cases. Four existing Rust tests remain opt-in: two paid cloud calls, one embedding model download, and one native Apple Vision check. **`make check` passed** formatting and strict native/WASM Clippy. Desktop/mobile screenshots, including the 320-pixel iPhone layout, were inspected; `git diff --check` passed.

Verification uses isolated fixture servers and WebKit device emulation. It validates context construction and application behavior, not paid-provider answer quality or physical iOS keyboard behavior. Paid AI, model-download and native screen-recording suites remain explicit opt-in checks.

## Same-account device synchronization over SSE — 2026-10-03

Creating a chatroom or a new primary thought now commits a wallet-scoped notification in the same database transaction. The existing `/api/notifications` SSE stream delivers those changes to other sessions. New thought screens follow the shared current primary room; saved chatroom screens retain their selection and draft while their room list and messages update. Login resumes the shared thought without repeating an earlier local New thought action.

`GET /api/rooms/thought` reads current state without creating a room. The browser reconciles it on relevant notifications, SSE connection/reconnection and WebSocket recovery, replacing periodic room-list polling. Historical notification room IDs cannot move the UI backwards: the browser reads the current primary room and rejects superseded responses with request and room generations. Reconnecting also works after room erasure has removed older creation notifications.

Two authenticated API regressions verify shared current state, empty read behavior, idempotent room creation, atomic/durable notifications, wallet isolation, backend reopen and SSE `Last-Event-ID` replay. Concurrent reset requests run in separate tasks behind a barrier and converge on one shared primary room.

Seven browser scenarios run in Chromium and iPad/iPhone WebKit with separate authenticated contexts. They cover bidirectional New thought/message sync, room creation/rename/clear/delete, preserving a selected saved chatroom, in-place logout/login, account isolation, delayed current-thought reads, delayed initial room lists, and notification-stream recovery. The reconnect regression cuts the actual local TCP stream while keeping WebSocket connected, then reopens SSE while suppressing replay payloads to verify reconciliation on the connection handshake itself. Browser offline emulation alone did not close established SSE connections and was replaced with this controlled transport test. The initial-list regression suppresses SSE replay and holds HTTP-over-WebSocket responses until the resumed room is selected, then delivers list responses in reverse order; the room-change snapshot ensures the saved-room list is complete.

All checks remain local through `make test` and `make check`; no CI workflow is configured.

Final local verification: **`make test` passed all 272 default checks: 102 Rust tests, 8 script tests and 162 browser tests** (104 Chromium, 29 iPad WebKit, 29 iPhone WebKit). The complete browser run took 7.7 minutes and includes all 21 new device-sync cases and all 27 fresh-context cases. **`make check` passed** formatting and strict native/WASM Clippy; `git diff --check` passed. Four existing native tests remain opt-in, as described above. These checks use authenticated fixture accounts and real local SSE transport with emulated device browsers, not physical iOS devices or paid cloud AI.

## Backend-owned Invisible AI sessions — 2026-10-06

Sessions are created in SQLite through `/api/invisible/sessions`; their workers run independently of browser connections and login cookies. The UI lists and controls those backend sessions. Restart preserves session metadata and reports interrupted runs as stopped, requiring an explicit start before further document input. Existing room APIs remain compatible. Source erasure removes sessions and cancels their workers.

Verification for this change:

- `env CWBBOT_ADMIN= cargo test -p causewaybay-server -p causewaybay-shared -p causewaybay-cli` passed the default Rust suites, including headless creation/execution, restart recovery, cross-wallet isolation, browser logout, account revocation, and erasure cancellation.
- The additional `notes::tests::continuous_capture_also_outlives_the_initiating_browser_cookie` regression passed with the isolated `CWBBOT_ADMIN=` test environment. It verifies repeated screen processing after the initiating login is revoked and explicit stop using a new login.
- `npm run test:e2e -- tests/e2e/invisible-ai.spec.mjs` passed all 18 scenarios across Chromium, iPad WebKit, and iPhone WebKit. A session created without a UI continued after its observing page closed, then was stopped from a separate browser. Creating a session remained non-executing until explicit start. Screenshots were inspected.
- Frontend build, strict server/all-target and frontend/WASM Clippy, formatting of changed Rust files, and `git diff --check` passed.

These checks use isolated databases, demo inference, and a simulated capture Mac over the real WebSocket transport. Native TextEdit control, local model inference, and paid cloud calls were not rerun for this session-lifecycle change. Managed backend/client processes were stopped; no running personal session was interrupted.

## Session progress, LLM visibility, cost estimates, and Create and start — 2026-10-06

Each Cloud session card gained a Progress panel: an animated live row naming the current step, the provider and model the backend is waiting on, whether it is a local LLM or cloud AI, and the seconds waited; a durable step timeline fed by the TextEdit worker, capture polling, and chat/capture jobs; and a table of recent AI requests from the usage ledger with wait, time to first output, tokens, state, and an estimated cloud cost from a dated list-price table. Chat replies show the same waiting row while running and an "Answered by Local LLM / Cloud AI" line afterwards. New session suggests a random name and offers Create and start. Without a saved personal or admin policy, backend and browser now share one computed default (Hybrid when a local and a cloud provider are both configured).

Verification for this change:

- `env CWBBOT_ADMIN= cargo test -p causewaybay-server --no-fail-fast`, `cargo test -p causewaybay-shared`, and `cargo test -p causewaybay-cli` passed: 57 server unit tests (including the new progress/LLM/clear and create-and-start regressions), 14 API, 12 auth, 11 local AI, 6 shared (including the computed-default rules), and the CLI suites. `local_ai::timeout_and_cancellation_kill_the_worker` hit its 3-second worker-spawn deadline in two of three back-to-back whole-workspace runs and passed three of three runs of its own test binary; it is timing-sensitive under load and its code was not changed here.
- `cargo fmt --all -- --check`, strict Clippy for server/shared/CLI all targets and for the WASM frontend, and `git diff --check` passed.
- `npm run test:scripts` passed 14 launcher/package checks.
- `npm run test:e2e` passed 333 of 334 checks across Chromium, iPad WebKit, and iPhone WebKit, including the new Chromium/WebKit scenarios: the Cloud Progress panel after a TextEdit answer (running state, Local LLM badge, Apple local · system, one complete measured request with unknown tokens and no cloud cost, totals, API fields), Create and start with a suggested name (immediate start, one worker per Mac), the chat "Answered by Local LLM" and "Answered by Cloud AI" lines, and the translation catalog check for the new labels.
- The remaining failure is `connection.spec.mjs › an interrupted message keeps its draft and retry after recovery` on iPad WebKit only. It expects the interrupted-request banner text, but on WebKit the two-second poll's "Retrying in 2 seconds…" text replaces it first. It fails the same way when rerun alone, passes on Chromium, and the reconnect code was not changed by this work; it remains open.
- `file-context.spec.mjs` assumes an earlier spec already created a chatroom on the shared demo server: it fails on a fresh backend or when run after specs that delete their rooms, and passes in the full alphabetical run and directly after `chat.spec.mjs`. That ordering dependency predates this change and is noted here rather than fixed.
- Three Playwright expectations (two in committed specs, one in the uncommitted policy spec) that predated the backend-owned sessions rewrite were brought in line with the Rust tests: a shared capture-only Mac may hold a session but cannot start TextEdit replies (400 "enrolled", also reported in `status.error` for `start: true`), the workspace's open chatroom is listed as an unbound idle session, and the new-session form preselects the workspace's active Mac and marks an unenrolled Mac "Account setup required" with a setup link.

Cost estimates use list prices dated 2026-10-06 (Claude Sonnet 4.5, GPT-4.1 mini, Grok 4, Gemini 2.5 Flash) confirmed from public pricing pages on that date; they are labelled as estimates and never shown as a provider bill. Demo inference records measured turns with unknown tokens, so these checks do not exercise provider token reporting; live cloud accounting remains covered by the earlier ledger tests. No native TextEdit, local model, or paid cloud calls were made for this change.

## Autonomous AI run, manual capture add-on, processing stats, and immediate start — 2026-10-06

Each Cloud session card now leads with **Autonomous AI run** (renamed from TextEdit automatic replies) with per-session check interval, typing pause and answer length, a connecting-time hint, and a large Start/Stop; **Manual capture (add-on)** with optional polling is collapsed below the Progress panel. Start answers the connect-time note without a typing pause. Text-mode captures whose recognized text matches the previous capture are discarded before any model runs. A **Processing stats** strip counts OCR runs, local LLM and cloud AI requests (tokens and estimated cost), and skipped screens for the last 24 hours and all time, and chat replies carry their own wait, tokens and estimated cost.

Verification for this change:

- `env CWBBOT_ADMIN= cargo test -p causewaybay-server -p causewaybay-shared -p causewaybay-cli --no-fail-fast` passed every target, including the new regressions for the per-session interval (validated, applied to a running worker, reset to the admin default, plain chatroom becomes a session), unchanged-screen-text detection per room, the wallet-scoped processing summary and per-reply usage, and the stop/disable/erase invariants restated as "no append after the stop" now that Start answers immediately. The load-sensitive `local_ai::timeout_and_cancellation_kill_the_worker` passed in this run.
- Strict Clippy (server/shared/CLI all targets, WASM frontend), `cargo fmt --all -- --check`, and `git diff --check` passed.
- `npm run test:scripts` passed 14 checks.
- `npm run test:e2e` passed 334 of 334 checks across Chromium, iPad WebKit, and iPhone WebKit, with the renamed controls, the collapsed add-on opened by the tests that use capture controls, the Korean labels, the dashboard assertions after a TextEdit answer, and the chat usage line.
- Screenshots of the card and the stats strip were regenerated on an isolated demo backend (`docs/screenshots/cloud-progress-*.png`, `docs/screenshots/cloud-stats-*.png`).

The stall the user saw (Start, then "Connecting to TextEdit…" for 23 seconds before a manual Stop) was traced through the persisted session status and the new activity log: the backend delivered the begin command within its 250 ms tick, and the time went to TextEdit automation on the capture Mac; the card now counts those seconds and explains the 30-second limit. Providers report tokens rather than prices, so every per-call cost remains an estimate from the dated list-price table.


### Everyday actions and processing location (2026-10-11)

The primary menu now has three actions: Ask anything, Save anything, and Find anything. The save page keeps text and files through the existing durable memo pipeline, then organizes them in the background when AI is available. Original content remains available if organization fails. Optional sessions, libraries and tools are folded; device and AI setup live in settings. Each page has a contextual companion. Helper controls use plain language, hide advanced options and keep exact cloud-approval content visible.

Inference activity is instrumented at the actual provider boundary, including intent/setup checks, chat, Wiki, coding, provider web search and speech. The global authenticated SSE stream sends only owner-scoped provider/model/location snapshots, handles concurrent calls and clears on completion/cancellation. Service-mode MLX is separately labeled as a configured AI server, rather than implying device-local privacy. No prompts or file contents are included in these snapshots.

MLX users can select the 9B or 27B catalog models from the AI picker; account model choices are used by backend jobs and fresh browsers. 9B is recommended for 16 GB Macs. A simulated 16 GB memory budget verifies that 9B fits and 27B exceeds the recommended GPU budget. This is not a real 16 GB hardware benchmark or a promise that 27B fits that machine.

Validation for this increment: strict native/WASM checks passed; the 10 offline local-model integration tests passed (2 opt-in live model tests were skipped), including the simulated 16 GB budget. Account model isolation and actual-request activity tests passed. Across Chromium, WebKit iPad and WebKit iPhone, 33 companion/location checks, 3 save/retry checks, 3 model-persistence checks and 6 helper lifecycle/Hybrid approval checks passed. All 15 distribution script checks passed. The Apple Silicon archive was rebuilt and its SHA-256 checksum verified. Live cloud calls and a real 16 GB Mac were not tested for this increment.
