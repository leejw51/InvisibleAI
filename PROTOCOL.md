# Invisible AI protocol

Updated: 2026-10-02. This describes the implemented protocol. Architecture, completed work and remaining work are in [PLAN.md](PLAN.md); test evidence is in [docs/verification.md](docs/verification.md).

## Addressing and transport

The backend defaults to `0.0.0.0:3000`. Connect using its actual LAN/VPN address, replacing `backend.example` in `http://backend.example:3000` with your backend's hostname or IP address. `0.0.0.0` is the listening address, not the remote destination.

| Channel | Endpoint | Purpose |
| --- | --- | --- |
| HTTP | `/` and local static assets | Rust/Yew WASM interface, compiled Tailwind, artwork |
| HTTP JSON | `/api/...` | Authentication and all ordinary API operations |
| Browser WebSocket | `/api/ws` | Presence, heartbeats, JSON requests and responses |
| Browser SSE | `/api/memos/{id}/events` | Durable progress and answer deltas |
| Browser SSE | `/api/rooms/{roomid}/events` | Durable complete message revisions |
| Capture WebSocket | `/api/client/ws` | Node registration, heartbeats, screenshot results and acknowledgements |
| Capture SSE | `/api/client/events?client_id=...` | Capture commands and node-name changes |
| HTTP bytes | Image, download and export endpoints | Authenticated images, original files, Markdown |

Use `ws://` with HTTP and `wss://` with HTTPS. HTTPS can terminate at a VPN reverse proxy; preserve the public Host and allow WebSocket upgrades and unbuffered SSE. The backend prints web/API/health/dashboard/Tailwind URLs on startup. Tailwind's generated asset filename is discovered from the served page, not a fixed CDN address.

`GET /api/dashboard` includes `addresses: [{"label":"Tailscale IP","web_url":"http://100.64.0.10:3000"}, ...]`, discovered from the actual listener and local interfaces at startup. Labels distinguish local, network, Tailscale DNS, and Tailscale IP entries; append `/api` for the API base. The frontend treats a missing list from an older backend as empty. The UI prefers Tailscale IP for the copyable Rust setup command and appends the signed-in wallet address. Saved active/default device choices stay in wallet-scoped web local storage; chat sends the selected node explicitly as `client_id` and does not replace disconnected saved devices automatically.

JSON requests use `Content-Type: application/json`. Browser connections use the same-origin session cookie. External browser origins are rejected; native clients can make requests without an Origin header. Timestamps in content records are strings; presence, challenge expiry and JSONL update timestamps use Unix seconds. GUIDs are UUID strings.

Application errors generally have `{"error":"Explanation"}`: 400 validation, 401 missing/expired authentication, 403 insufficient approval/role or cross-origin access, 404 missing resources, 409 conflicts, 429 capacity limits, and 500 database errors. Framework body/JSON extraction errors may have a different body. A successful Think response means work was accepted; inspect events and the final memo status for success.

## Login, approval and authority

Set backend `CWBBOT_ADMIN` to enable wallet login and registration approval. It is the permanent owner and cannot be demoted or blocked through the API. Addresses are normalized to lowercase internally.

1. The browser derives Ethereum account 0 from a BIP-39 mnemonic with an empty passphrase at `m/44'/60'/0'/0/0`.
2. It requests a challenge for the public address.
3. It signs the exact returned message using EIP-191 in browser memory.
4. The server recovers the signer, checks host/expiry/nonce, and creates a session.
5. An unknown wallet may apply for registration but cannot access workspace APIs.
6. An admin approves the application. The waiting page refreshes without a new login.

The recovery phrase and private key never belong in an HTTP/WS request, localStorage, IndexedDB or backend database. Their persistence is deferred. This is identity signing; no blockchain transaction is submitted.

| HTTP operation | Body | Result/access |
| --- | --- | --- |
| `GET /api/health` | — | Public `{"ok":true,"app":"Invisible AI"}` |
| `GET /api/auth/info` | — | Public session state: `required,wallet_login,authenticated,address,name,is_admin,role,approved` |
| `POST /api/auth/challenge` | `{"address":"0x..."}` | `{nonce,message,expires}`; wallet mode required |
| `POST /api/auth/verify` | `{"nonce":"...","signature":"0x..."}` | `{"ok":true}` and session cookie |
| `POST /api/auth/logout` | — | Revoke current session and clear cookie |
| `POST /api/auth/apply` | `{}` | Signed-in wallet; display name is derived from the wallet (legacy name input is ignored); `{ok,role}` |
| `GET /api/admin/users` | — | Admin only; `{users,owner}` |
| `PATCH /api/admin/users/{address}` | `{"role":"user"}` | Admin only; `user`, `admin`, or `blocked` |
| `POST /api/session` | `{"token":"..."}` | Legacy token mode only, when `CWBBOT_ADMIN` is unset |

Challenges expire after 120 seconds and can be successfully used once. At most 1,024 pending challenges are retained. Sign the returned `message` verbatim rather than reconstructing its text. Signatures are 65-byte Ethereum recoverable signatures encoded as hex.

The opaque `cwb_session` cookie lasts seven days, is HttpOnly and SameSite=Strict, and adds Secure for HTTPS login origins. SQLite stores only its SHA-256 hash, address and expiry. Login challenges live in memory; sessions survive backend restarts until expiration/logout.

Roles are `unregistered → pending → user → admin`, with `blocked` for denied/revoked access. Approval must precede promotion. Admins may approve a blocked user again. The permanent owner is included in the user list. Approval/admin changes append full audited snapshots to `users.jsonl` and update SQLite.

Every protected HTTP operation and every WebSocket dispatch checks current authorization. Active browser WebSocket/SSE loops recheck approval/session validity. Setup operations require an admin: settings mutations, node renames, AI connection checks, and all admin endpoints. Hiding menus is accompanied by backend enforcement.

Every content route, room/event stream, search, AI retrieval tool and browser-presence query is scoped to the authenticated normalized wallet address, with no administrator bypass. Node ownership separately limits capture requests; explicitly shared hardware does not share the captured data. Display names are deterministic labels; only the full address is the identity key. Existing ownerless data remains in the `local` namespace. Data is not end-to-end encrypted.

Without `CWBBOT_ADMIN`, the existing local/demo mode or optional access-token mode remains available. In wallet mode `APP_TOKEN` is a capture-node credential, not a browser login or admin credential. Capture requires a bearer credential only when APP_TOKEN is set; then APP_TOKEN or the private capture-token file (shared with the same-Mac client) is accepted. Without APP_TOKEN, capture connections are not token-checked. A registered ID is bound to the client device-key hash; mismatched or absent keys cannot replace it. This is possession-based pairing, not hardware attestation.

## Browser WebSocket

Connect to `/api/ws` after login and approval. Send a hello within 10 seconds. Keep the same tab ID across retries.

Client → server:

```json
{"type":"hello","id":"web-tab-id","name":"Safari · iPad","platform":"iPad"}
```

Server → client:

```json
{"type":"ready","client_id":"web-tab-id"}
```

A request carries a correlation ID, HTTP method, API path and JSON body:

```json
{"type":"request","id":1,"method":"POST","path":"/api/think","body":{"input_mode":"text","capture":true,"client_id":"macbook-id","prompt":"Help me respond"}}
```

The response echoes that ID and the HTTP status:

```json
{"type":"response","id":1,"status":200,"data":{"id":"memo-guid","title":"Help me respond","input":"Help me respond","answer":"","kind":"question","tags":[],"provider":"grok","model":"configured-model","status":"running","created_at":"2026-10-02T00:00:00Z","pinned":false,"has_image":false,"error":null}}
```

The example values are illustrative. `data` contains the same JSON value as the equivalent HTTP API response. Responses can arrive out of order. The dispatcher forwards the authenticated handshake headers and rechecks permissions for each operation.

Supported methods are GET, POST, PATCH and DELETE on ordinary JSON API routes. Use HTTP directly for authentication bootstrap, SSE, raw downloads/exports and paths ending in `/image`, including image deletion. WebSocket endpoints cannot be nested inside a request. GET can use a null body.

Both sides exchange `{"type":"heartbeat"}`. The browser reconnects after a two-second delay, registers again, and resumes event streams. It does not automatically resubmit an uncertain AI request.

Legacy memo subscriptions remain supported:

```json
{"type":"subscribe","memo_id":"memo-guid","after":42}
{"type":"event","memo_id":"memo-guid","event":{"id":43,"kind":"delta","text":"Answer"}}
{"type":"unsubscribe","memo_id":"memo-guid"}
```

New clients use SSE for callbacks. At most eight legacy subscriptions and 32 in-flight JSON requests are allowed per browser socket.

## Capture CLI WebSocket and SSE

Connection shortcuts (replace the example hostname and owner wallet with your own values):

```sh
rust-cli http://backend.example:3000
rust-cli http://backend.example:3000 0xYOUR_OWNER_WALLET_ADDRESS
```

The first explicitly shares the node with approved users; the second allows only the matching approved wallet to capture. Admins do not bypass ownership. Release packages provide the same client as `bin/client` and the backend as `bin/server`; source builds also provide `rust_cli` and `causewaybay` compatibility aliases. The URL and owner wallet are saved in client JSONL before connecting. No arguments restores that saved connection, including ownership, node ID and computer name; without a saved URL it prints connection help. `--help` always prints help. Advanced `client --owner-wallet ADDRESS` and `client --shared` set ownership; omitting both in the advanced command restores the saved owner.

Connect to `/api/client/ws`, adding `Authorization: Bearer <APP_TOKEN-or-capture-token>` when the backend sets APP_TOKEN. The node ID is stable in client JSONL; the name defaults to the computer name.

```json
{"type":"hello","id":"stable-client-id","name":"MacBook","platform":"macos","callbacks":"sse","owner_wallet":null,"device_key":"64-hex-character-private-device-key"}
{"type":"ready","client_id":"stable-client-id","session":"connection-secret","callbacks":"sse"}
```

After ready, open HTTP `GET /api/client/events?client_id=stable-client-id` with `x-capture-session: connection-secret` and the same bearer credential. This secret is bound to the current WebSocket and invalidated on disconnect. Do not persist or reuse it for a later connection.

Commands are JSON in SSE data frames:

```text
retry: 2000

id: capture-request-guid
data: {"type":"capture","request_id":"capture-request-guid","mode":"screen","display":1}

data: {"type":"node_config","name":"Travel MacBook"}

```

The CLI runs native capture, saves a visible UUID-named PNG in its screenshots folder, then returns the bytes on WebSocket:

```json
{"type":"capture_result","request_id":"capture-request-guid","image_base64":"BASE64_PNG"}
{"type":"capture_ack","request_id":"capture-request-guid","ok":true,"error":null}
```

Failure example:

```json
{"type":"capture_result","request_id":"capture-request-guid","error":"Screenshot was not created"}
{"type":"capture_ack","request_id":"capture-request-guid","ok":false,"error":"Screenshot was not created"}
```

An acknowledgement with `ok:true` means the backend accepted the screenshot, not that OCR or AI completed. The backend verifies node identity, connection, request state and running memo before accepting results. It rejects duplicate/cancelled results and times out a capture after 100 seconds.

The SSE stream can replay an unacknowledged capture on reconnect. The CLI remembers its most recent 256 request IDs to avoid recapturing the same command during that connection's callbacks. An entire WebSocket reconnect may requeue unfinished work. Node-config callbacks update and persist the CLI name. Clients omitting `callbacks:"sse"` retain legacy WebSocket capture-command delivery.

CLI heartbeats run every 10 seconds. The server closes sockets after 35 seconds without incoming messages. CLI reconnection sleeps for its configured interval: default three seconds, bounded 1–60. SSE retries after two seconds and has a 35-second chunk timeout; the server sends keepalive comments every 10 seconds. Old connection cleanup cannot mark a newer connection offline.

## Think, capture batches and memo callbacks

`POST /api/think` accepts the following fields; unknown fields, including API keys, are rejected.

A typed line sent to `POST /api/think` without capture, attachments, agent mode or a parent is first classified as a question or a record: a question mark, an interrogative or imperative opener or a request word (English or Korean) means question; a diary-like line, a to-do, a link or a long statement means record; otherwise the local gate model (the account's local provider) answers QUESTION or NOTE, and when none is available the question wins. A record becomes a memo of kind `note`: the post is kept as written and the account's AI (the same model that would answer) tidies it under a fixed prompt (fix typos and grammar, keep every fact, title line plus bullets or one paragraph, add nothing, never answer) into the memo's `answer`, with no tools; the chat shows the post as "Kept as a note" and the tidied text as "Tidied note", and "Ask AI instead" resends the line with `"ask": true`, which skips the classifier. `ChatMessage.memo_kind` carries the memo's kind so clients can label it. `ChatMessage.skills` lists the user's skills the reply ran (`[{name,success}]`, from `run_skill` tool results), so a chat can show which skill was used. Explicit `POST /api/memos` keeps stay kind `memo` with no AI pass. In the Wiki a note's category comes from the model or the heuristic (never q&a), feed items carry `tidied: true` and the tidied text as `answer_snippet`, and the tag document labels it "Tidied note". Demo mode uses the surface cues only.

| Field | Type/default | Meaning |
| --- | --- | --- |
| `prompt` | string, empty | User question/instructions, at most 100,000 bytes |
| `input_mode` | `text` (API default) or `image` | OCR text versus direct cloud image; new browser chats default to `image` unless the wallet has a saved mode |
| `capture` | boolean, false | Ask a connected Rust node for a screenshot |
| `client_id` | optional string | Pin a node; otherwise select an available permitted node |
| `capture_mode` | optional `screen` / `region` | Region requires local interaction on the capture computer |
| `display` | optional integer, 1–16 | Display index; falls back to backend default |
| `provider`, `model` | optional strings | Override configured defaults; credentials stay on backend |
| `file_id` | optional GUID | Legacy single stored upload; still accepted |
| `file_ids` | GUID array, empty | Ordered photo/file set, up to 8 attachments in total; may accompany a comment and a single capture |
| `agent` | boolean, false | Enable the bounded coding tool loop |
| `parent_id` | optional memo GUID | Follow-up to a memo |
| `room_id`, `message_id` | optional GUIDs | Chat room and submitted message identity |

A question needs a prompt, capture or attachment. The server returns a complete Memo JSON object immediately, usually with `status:"running"`. The saved original image is available as soon as the image event appears.

`POST /api/capture/batch` accepts `{"client_ids":["a","b"],"all":false,"request":{...ThinkRequest}}` or `{"all":true,"request":{...ThinkRequest}}`. It snapshots online permitted nodes and overrides capture/file/client fields per node. Response: an array of `{client,memo,error}`. One node's failure does not discard another node's result.

Memo callbacks use `GET /api/memos/{id}/events?after=0`:

```text
retry: 2000

id: 42
data: {"id":42,"kind":"phase","text":"ocr"}

id: 43
data: {"id":43,"kind":"ocr_text","text":"What is 7 + 5?"}

id: 44
data: {"id":44,"kind":"delta","text":"12"}

```

Event data has `id:int64, kind:string, text:string`. Events are durable in SQLite. Reconnect with `Last-Event-ID` or `?after=`; ignore already applied IDs. Keepalive comments are not application events.

Kinds include `mode`, `phase`, `stage`, `image`, `ocr_text`, `ocr`, `transcript`, `context`, `delta`, `tool_start`, `tool_result`, `kind`, `done` and `error`. `ocr_text` is the exact extracted text; legacy `ocr` can contain combined processing context. Image mode has no OCR event. `phase` distinguishes capture/OCR/image/AI; `stage` is display text. On done/error, refresh the memo for its canonical answer/status. Backend restart marks unfinished work interrupted and retains partial answers.

## Chatrooms, synchronization and continuous capture

`POST /api/rooms` accepts `{"title":"My room"}` and optional `roomid` GUID; `GET /api/rooms` lists rooms. A Room has `roomid,title,created_at,updated_at,deleted`. `POST /api/rooms/thought` with `{"fresh":true}` creates a new independent primary room for the current wallet without deleting earlier rooms. An empty body object resumes that primary room (for reload/reconnect), creating one only if needed. Explicit New thought actions always send `fresh:true`. Room-based AI requests use only that room’s history and explicitly attached context; they do not automatically retrieve answers from the library.

`GET /api/rooms/thought` reads the wallet's current primary Room or `null`, without creating a room. Room creation atomically records a wallet-scoped `room.created` notification; primary-room creation records `room.thought`. Both carry `data.roomid` through `/api/notifications` SSE with durable event IDs and `Last-Event-ID` replay. Repeating creation of an existing room GUID or resuming a thought does not emit another creation event. Existing rename, clear and delete notifications share the same stream. Other wallets and capture-device notification streams do not receive these events.

On login and SSE reconnect, clients reconcile the room list and current thought. A `room.thought` notification prompts a read of the current primary room, rather than following a potentially historical event's room ID. New thought screens on the same wallet follow the shared primary room; selecting a saved chatroom keeps that device on its selected conversation. Message changes continue through `/api/rooms/{id}/events`. Room-list refreshes are driven by notifications and reconnection, without a periodic room-list poll.

Submit through Think with `room_id` and a new `message_id`. Resubmitting the identical request with the same message GUID returns the same memo without another AI job. Reusing that GUID for a different request is rejected. A deleted memo cannot be resurrected by that retry.

The backend creates user and assistant message records, updates them as capture/OCR/AI progresses, and transactionally replaces the previous full snapshot for each message in the change log. Superseded message content is deleted; only the newest state or content-free tombstone is retained.

Photo sets use `file_ids` and an optional `prompt` comment. Later comments and uploads in the same room share recent conversation context. The server reconstructs the latest 24 message records, skips deleted/incomplete messages, uses edited comment text, includes attached document text, and sends original photos through bounded cloud copies in Image mode. Up to 16 images are included per request, reserving space for current photos/capture before the most recent historical photos. Older omitted images are labeled as omitted. Text mode never forwards image bytes, including historical images. Attachment ownership is checked, and all files are claimed for the room atomically. Room clear/delete removes every attached original and all associated message data.

Uploaded photos render directly from their attachment download URLs; `image_path` is used for captured screens and legacy memo copies. A photo can be removed from the composer individually before sending. Batch/continuous capture clears pending file fields instead of reattaching the same photos repeatedly.

| ChatMessage field | Meaning |
| --- | --- |
| `roomid`, `messageid` | Stable room/message GUIDs |
| `messagesyncid` | Signed 64-bit Unix-microsecond revision, allocated as max(current time, persisted clock + 1), including after restart/clock rollback |
| `sequence` | Immutable initial order within the displayed conversation |
| `deleted`, `edited_at` | Tombstone flag and nullable edit timestamp |
| `role`, `text` | User/assistant role and full current content |
| `ocr_text`, `transcript` | Exact OCR and audio transcription, when present |
| `image_path` | Nullable backend-relative image path, never a browser-local file |
| `attachments` | FileInfo records |
| `sources` | Verified web-tool source links as `{title,url}`; defaults to an empty array |
| `input_mode`, `provider`, `model` | Processing configuration |
| `status`, `stage`, `error` | Current progress and optional error |
| `tags` | Wiki tags of the memo behind the message, the same on a post and its generated response (the user's hashtags or edited list, then those inferred on-device from both texts); empty until analyzed. A tag change is a new revision of the message, so every device learns it through room sync. Defaults to an empty array. |
| `memo_id`, `client_name`, `created_at` | Associated job, optional capture node name, creation timestamp |

`GET /api/rooms/{roomid}/sync?after=0` returns `{messages,next,has_more,cleared_through}`, up to 128 latest message snapshots. `cleared_through` is the last room erasure barrier; it advances even when an empty room is cleared. Continue from `next` while `has_more` is true. Cursors can have gaps; never infer that consecutive numbers belong to a single room.

`GET /api/rooms/{roomid}/events?after=0` sends SSE with `id: messagesyncid` and a complete ChatMessage JSON object in `data:`. Reconnection uses Last-Event-ID. Upsert by message GUID only when the revision is newer; order display by `sequence`. Each update is a full snapshot, not a text delta. A clear additionally sends `{kind:"room.cleared",data:{roomid,messagesyncid}}` on the same SSE channel. Apply this barrier before merging later records. After the final page, `next` is at least `cleared_through`. Browsers reconcile all pages before rendering cached content on room entry or later login.

The Rust browser caches only message JSON strings in wallet-scoped IndexedDB, store `messages`. It reads cached messages, then replays after the saved cursor. Its writes are serialized to prevent a newer cursor being saved before an earlier record. Preserve int64 values: JavaScript consumers must not round large JSON integer cursors through an IEEE-754 number.

The backend supplies up to 24 preceding completed messages as AI context. Images/files stay on the backend; localStorage holds UI preferences/drafts only. Private keys are not stored.

Room/message management:

| Method/path | Body/result |
| --- | --- |
| `PATCH /api/rooms/{roomid}` | `{"title":"Daily work"}`; returns Room |
| `POST /api/rooms/{roomid}/clear` | Erases content, preserves empty room; `{deleted:false,cleared:true,messagesyncid}` |
| `DELETE /api/rooms/{roomid}` | Erases content and removes room; `{deleted:true,cleared:true,messagesyncid}` |
| `PATCH /api/rooms/{roomid}/messages/{messageid}` | `{"text":"Revised","messagesyncid":CURRENT_REVISION}`; returns revised ChatMessage |
| `DELETE /api/rooms/{roomid}/messages/{messageid}` | `{"messagesyncid":CURRENT_REVISION}`; returns ChatMessage tombstone |

Both roles may be edited/removed, including during generation. Mutations cancel the related job and retain the original message GUID/order. An outdated revision returns HTTP 409. Hide tombstones but advance the cursor. The latest edited state remains replayable; superseded revision bodies are erased on every write. Submission idempotency retains only a SHA-256 digest. Deleting a user message erases its generated response, associated generated files/notes, and unreferenced attachments; deleting an assistant message erases its generated output. Both removals emit content-free `message.deleted` wallet notifications carrying `{message: ChatMessage}`. Files explicitly attached to another live message remain in use.

Room cleanup cancels/awaits jobs, stops continuous capture, erases attached and generated files, captured images, Markdown, memos, embeddings, FTS entries, events, compilation/processing results and capture queues. Only content-free room/message identifiers, ownership and cursors remain for deletion replay. A durable clearing marker prevents new submissions while cleanup runs and resumes cleanup on server restart. Downloaded copies and external backups are outside this protocol. Legacy capture-client images without a request-GUID filename cannot be mapped automatically.

`GET /api/notifications?after=0` is an authenticated SSE stream for arbitrary wallet-targeted JSON: `{"type":"notification","id":123,"kind":"room.cleared","data":{"roomid":"GUID","messagesyncid":123456789}}`. SSE IDs are notification cursors, separate from message revisions. Replay uses Last-Event-ID; heartbeat runs every 10 seconds and reconnect delay is two seconds. A connection that opens with neither `Last-Event-ID` nor a positive `after` is a fresh page catching up on history: every notification already recorded at that moment carries `replay: true`, and clients apply such events to their caches (message cache, erasure cutoffs) but not to live view state, since what they have just loaded is newer. A resuming connection with a cursor receives what it missed unflagged. Kinds include `answer.ready`, `room.renamed`, `room.cleared`, `room.deleted`, and `wiki.updated` (sent once per Wiki sync that changed a tag, a message's tags or a tag document, with `data:{pages:<bool>}`; open Wiki pages reload their feed, map, related panel and document on it, and the Wiki toolbar has a Refresh button that does the same by hand). Delete/clear notifications contain no old content. Clients persist a per-room cleared cursor and reject delayed message records at or below it.

Rust clients receive separately targeted notifications on their existing authenticated `/api/client/events` SSE stream, alongside capture commands. `capture.remove` carries `{request_id:GUID}` and instructs the client to cancel that capture and remove `screenshots/<request_id>.png` idempotently. Offline clients receive it on reconnect. `computer.renamed` carries updated device details; existing `node_config` remains supported. Client notification cursors are acknowledged in the reconnect query, and full connection retries may safely replay deletions.

Continuous capture endpoints:

| Method/path | Body/result |
| --- | --- |
| `GET /api/rooms/{roomid}/capture` | ContinuousStatus |
| `POST /api/rooms/{roomid}/capture` | Start with the JSON below; returns ContinuousStatus |
| `DELETE /api/rooms/{roomid}/capture` | Stop loop and cancel outstanding memos; returns status |
| `POST /api/rooms/{roomid}/capture/now` | ThinkRequest; queue one manual capture on the active polling loop and reset its deadline |

```json
{"client_ids":["macbook-id"],"all":false,"interval_seconds":5,"request":{"input_mode":"text","prompt":"Explain what changed"}}
```

Each room accepts exactly one capture node. `client_ids` must contain that node (or be empty to reuse the room binding); legacy `all:true` is accepted only when it resolves to exactly one node. The first capture/TextEdit action fixes the room binding. `ContinuousStatus` contains `roomid,active,interval_seconds,client_ids,error,capturing,capture_queued,next_capture_at,unchanged_count`. `capturing` also reports a one-off room capture in progress, even when polling is inactive. There is one active loop per room, full-screen capture only, and an interval of 2–300 seconds after completion. Manual captures use the same loop, with at most one queued request, so they cancel the pending deadline without overlapping work. Use `/capture/now` while polling; `/think` and staged snapshots reject competing captures. Account and node permissions are rechecked for every frame. Stop/restart is generation-guarded; backend restarts preserve the binding and history but do not resume loops.

The pipeline compares decoded pixels to the last retained screenshot in this room. Identical frames lose their new image file and trigger capture-client cleanup. Without an explicit prompt or attachment, the duplicate memo and turn are tombstoned before inference. Explicit questions still run against the captured context without keeping a duplicate screenshot. `POST /api/memos` accepts optional `room_id,clear_epoch,data_epoch` to save a memo as a room message without an AI response. Standalone memos remain supported. `POST /api/rooms/thought` also accepts `room_id` to select an existing owned session as the shared current thought.

This is periodic screenshot capture; there is no encoded video/WebRTC stream.

## HTTP JSON API catalogue

All routes below require approval when wallet mode is enabled. Ordinary JSON operations can also use the browser WebSocket unless excluded above.

| Method/path | Request/result |
| --- | --- |
| `GET /api/settings` | Input/provider/model/capture defaults, provider availability, semantic/demo status; no secrets |
| `PATCH /api/settings` | **Admin**: optional `input_mode,provider,model,capture_mode,display`; returns settings. Drafts belong to wallet-scoped browser storage; nonempty backend draft updates are rejected |
| `GET /api/ai` | `provider,model,endpoint,configured,demo,audio_provider,status` |
| `POST /api/ai/check` | **Admin**: optional `{provider,model}` for an actual inference check using backend credentials; inspect returned status even on HTTP 200 |
| `GET /api/clients` | CaptureClient array, including caller-specific `can_capture` |
| `PATCH /api/clients/{id}` | **Admin**: `{"name":"Travel MacBook"}`, at most 120 bytes, no controls |
| `GET /api/devices` (alias `GET /api/dashboard`) | `rust_clients,web_clients,rust_connected,web_connected,uptime_seconds,server_version,addresses`; shown on the Devices page (`/devices`) |
| `GET /api/memos?q=...&kind=...&offset=0&limit=50` | `{memos,semantic,total}`; hashtag/BM25/semantic search, limit at most 100 |
| `POST /api/memos` | `{"content":"Memo text","title":"Optional title"}`; returns Memo |
| `GET /api/memos/{id}` | Full Memo |
| `PATCH /api/memos/{id}` | Optional `title,tags,pinned`; returns Memo. `tags` curates the memo: the normalized list (12 at most, else 400) replaces hashtags and inferred tags alike, the memo's Wiki tags and its chat message are rewritten in the same transaction, and no later analysis overrides it. A later text edit keeps the curated list and adds the hashtags written. |
| `DELETE /api/memos/{id}` | Cancel job, delete memo/backend artifacts; `{ok:true}` |
| `POST /api/memos/{id}/cancel` | Cancel generation, retain partial work |
| `GET /api/memos/{id}/image` | Raw saved image |
| `DELETE /api/memos/{id}/image` | Delete backend image and current chat image references |
| `GET /api/memos/{id}/export` | Markdown attachment |
| `GET /api/files` | FileInfo array |
| `POST /api/files` | `{"name":"example.rs","data_base64":"...","room_id":"optional GUID"}`; returns FileInfo. A room-owned file cannot be reused in a different room; upload a separate copy |
| `POST /api/uploads` | Begin a resumable video upload (MP4, MOV, M4V, WebM, up to 1 GB). `{"name","size","sha256?","roomId?","clearEpoch?","dataEpoch?","wiki?"}`; 201 `{id,offset,size,chunkSize}`. `chunkSize` is advisory (500 KiB). At most 8 open uploads per account |
| `PATCH /api/uploads/{id}?offset=N` | Append one raw chunk (`application/octet-stream`, at most 16 MiB) at offset `N`. 409 `"Upload is at offset M; resume from there"` unless `N` is the backend's offset, so a retried chunk is never written twice. Returns `{offset,size}` |
| `GET /api/uploads/{id}` | `{id,offset,size,chunkSize}`; polling keeps the session alive |
| `POST /api/uploads/{id}/finish` | Re-hashes the assembled file: 400 if incomplete or the SHA-256 differs from the declared one, 415 unless it starts with an MP4 `ftyp` box or WebM header. Moves it under `files/` (a video is binary, so SQLite stores only its path), deletes the session, returns FileInfo |
| `DELETE /api/uploads/{id}` | 204; abandon the upload and delete its part file. Sessions idle for 24 hours are swept at startup and hourly |
| `DELETE /api/files/{id}` | Delete file bytes, metadata, results and current attachment references |
| `GET /api/files/{id}/download` | Original bytes with download headers; binary media is served from disk with Range support, a text file from SQLite |
| `GET /api/files/{id}/text` | `{file,text,compilation}`; UTF-8 preview up to 500,000 bytes |
| `POST /api/files/{id}/compile` | Compile stored Rust source; persist compiler result |
| `POST /api/files/{id}/process` | `{"input_mode":"text"}` or `image`; persists ProcessedFile; `{}` defaults to text |
| `GET /api/images` | ImageInfo array covering captures and uploaded images |
| `GET /api/coding` | `{languages:[{id,name,available,detail,entry}],projects:[Project],folder,ai:{mode,provider,model,engine,demo}}`; `engine` is `harness` (On-device, Hybrid, Cloud AI) or `cli` (Codex, Claude Code) |
| `POST /api/coding/projects` | `{"language":"python","name":"optional"}` (`rust`, `go`, `python`, `c`, `lua`, `typescript`); scaffolds `projects/<uuid>` and returns Project |
| `GET /api/coding/projects/{id}` | `{project,folder,files:[{path,size}],memos,history,stats,example:{stdin,args}}`; `example` is the folder's `example.json`, the input the verification run uses; hidden tool folders (`.git`, `.cache`, `target`, `node_modules`) are never listed |
| `PATCH /api/coding/projects/{id}` | Optional `name`; `skill:{name,description}` publishes (name normalized to letters, digits and dashes, unique per account), `skill:null` retires; returns Project |
| `DELETE /api/coding/projects/{id}` | Removes the folder, its skill and its row; the request memos stay |
| `GET /api/coding/projects/{id}/file?path=` | `{path,text}`; relative paths only, no `..`, no symbolic links, UTF-8 up to 400 KB |
| `POST /api/coding/projects/{id}/file` | `{path,text}` writes one file (POST, so the WebSocket can carry it); returns the file list |
| `DELETE /api/coding/projects/{id}/file?path=` | Deletes a file or folder inside the project |
| `POST /api/coding/projects/{id}/example` | `{provider?,model?}`: the picked AI reads the program and writes a representative `example.json`; returns `{stdin,args,memo}` (a small `coding` memo records the call). The same happens automatically before a verification run when the folder has no example |
| `GET /api/coding/projects/{id}/history` | `{history:[{commit,at,message}]}`, newest first: every agent request, save and delete is a git checkpoint in the folder |
| `POST /api/coding/projects/{id}/revert` | `{"commit":"<sha>"}` from the history: restores the files (caches stay), recorded as a new checkpoint; returns `{ok,files,history}` |
| `POST /api/coding/projects/{id}/test` | Runs the project's tests (`cargo test`, `go test ./...`, pytest/unittest, `node --test`) in the sandbox; same result shape as a run, `phase` is `test` (or `deps` when fetching failed) |
| `POST /api/coding/projects/{id}/run` | `{"stdin":"","args":[]}`; builds and runs in the sandbox and returns `{success,phase,exit_code,stdout,stderr,duration_ms,timed_out}` (429 while the workspace is busy) |
| `POST /api/coding/projects/{id}/ask` | `{"prompt":"…","provider":"optional","model":"optional"}` (the browser's AI pick, checked like Think's overrides); returns a Memo of kind `coding` whose events (`stage`, `progress`, `tool_start`/`tool_result` for the harness and tool-loop tools (`list_files`, `read_file`, `grep`, `glob`, `write_file`, `edit_file`, `delete_file`, `run_project`/`run_program`, `run_tests`, `run_command`, `todo_write`, `publish_skill`, `web_search`, `fetch_url`), `todos` with the agent's current task list as JSON and for the CLIs' own steps (`claude_code.<tool>`, `codex.command`, `codex.file_change`), `delta` text streamed as the agent writes, then `done`) show the work; one request at a time per project |
| `GET /api/skills` | `{skills:[{id,name,description,language,language_name,project_name,updated_at,available,last_run,ready,building,memo}],projects:[unpublished projects],languages}`; `ready` means the last workbench or agent run succeeded (only ready skills reach the chat's `run_skill`), `building` means a request is running; `stats` is the use ledger `{runs,failures,chat_runs,last_used_at,last_error}` filled by chat and Skills-page runs, which never change `ready` |
| `POST /api/skills` | `{name,description,language,prompt?,example_stdin?,example_args?,provider?,model?}`: creates the project named after the skill, publishes the name at once, writes the example to `example.json` when given, and starts the agent with the command-line contract; returns `{project,memo}` |
| `POST /api/skills/{id}/run` | `{"input":"","args":[]}`: stdin and arguments; same result shape as a project run |

CaptureClient has `id,name,platform,online,last_seen,owner_wallet,can_capture`. FileInfo has `id,name,mime,size,created_at,path`; size is bytes and path is relative to the backend data directory. ImageInfo has `id,name,path,url,source,created_at`; use the authenticated `url` to display it. ProcessedFile has `file,kind,text,truncated,processor`.

Memo has `id,title,input,answer,kind,tags,provider,model,status,created_at,pinned,has_image,error`. Memo creation/update is separate from chat submission; use Think with room/message GUIDs for chat synchronization.

## Media, coding and storage

Text mode images use Apple Vision on the macOS backend. Image mode uses the configured cloud vision provider. Audio uses backend xAI speech-to-text or Gemini; CSV becomes a bounded structured text representation; Markdown/JSON/UTF-8/UTF-16 become text. Unsupported binary formats yield metadata and a bounded hex preview. Explicit processing results are saved in SQLite.

Uploads and screenshot results are base64 JSON, at most 25 MiB decoded. HTTP bodies and WebSocket messages/frames allow 36 MiB for encoding overhead. Gemini audio has an additional 10 MiB limit. Text processing is bounded, CSV output previews at most 100 rows, and raw binary previews at most 256 bytes.

AI keys come only from backend environment/`.env`: `XAI_API_KEY`, `ANTHROPIC_API_KEY`, `OPENAI_API_KEY`, `GEMINI_API_KEY`. They are neither accepted nor returned by workspace APIs. Restart the backend after credential changes.

The server permits four concurrent AI/compile jobs, up to 64 queued/running Think jobs, and bounded tool execution (eight model rounds, 24 tool calls). Rust compilation in chat has a 30-second timeout, does not run binaries, and does not install Cargo dependencies. Coding projects are the one place programs run: under `sandbox-exec` with no network, writes only inside the project folder, reads only from the toolchains and the system, 64 KB per output stream, 60 seconds per run, 300 per build, 180 per test run, 600 per dependency fetch, the process group killed on timeout. The dependency fetch step (cargo, go mod, pip into a project virtualenv, npm) is the only step with the network on, and it still writes inside the folder only. Codex and Claude Code in that mode run inside the same read allow-list plus their own state folders. When an account has published skills, chat answers from tool-capable providers may call `run_skill` (`{name,input}`), which runs that project with the text on stdin. Tool/file/compilation results are persisted. macOS compilation uses a restricted sandbox with no network and no user-storage reads or writes outside the build directory (the toolchain remains readable); unsupported platforms fail closed. This is not a general-purpose execution environment. Arithmetic, web search and private retrieval tools are available in ordinary chat. File writes/compilation require `agent:true` (the chat's **Allow file creation & compilation**); a tool outside the allowed catalogue is answered with an error result instead of failing the turn, and the compile tool is offered only when the backend can compile.

Backend persistent files default to `~/.causewaybaybot`; CLI files default to `~/.causewaybaybotclient`. Backend image/file paths in SQLite are relative to its data directory. Uploaded originals and memo image copies are separate objects when both exist. Individual image/file deletion updates current attachment references. Full room clear/delete also erases prior OCR/transcript/answer text and old revisions, and requests removal of associated capture-client originals as described above.

Configuration JSONL uses append-and-sync snapshots:

```json
{"version":1,"updated_at":1790899200,"config":{"provider":"grok","input_mode":"text"}}
```

The last complete valid record wins; interrupted trailing writes are ignored. `backend.jsonl`, `settings.jsonl`, `nodes.jsonl`, `users.jsonl` and client `client.jsonl` serve different configuration/audit purposes. They exclude provider secrets. SQLite holds messages/revisions, memos/events, file metadata and the content of every text file (`files.text`, UTF-8 without NUL; `path` is empty for such a row), Wiki documents, code notes, processing results, session hashes and search indexes, so one database is the complete text record. Only binary media bytes live in `data/screenshots` and `data/files` (`path` set, `text` empty). Nothing textual is mirrored to disk. Back up the complete persistence folder while stopped; SQLite alone cannot restore missing attachments.

### Staged capture and user content erasure

- `POST /api/rooms/{id}/snapshot` accepts `{ "client_id": "optional-device-id", "display": 1 }` (both fields optional) and returns a room-owned image `FileInfo`. It waits for a screenshot without creating a chat message or calling an AI provider. Multi Capture stages these IDs; Done submits one existing `/api/think` request with `capture: false` and `file_ids` (up to eight attachments). Device ownership and room ownership are enforced.
- `DELETE /api/user/data` erases all content for the authenticated wallet, including standalone memos/files and every room, their cached thumbnails (`thumbnails/<id>.jpg`), unfinished chunked uploads (`upload_sessions` rows and their `.part` files), and everything the Wiki derived for the wallet: `wiki_tags`, `wiki_topics`, `wiki_sources`, `wiki_pages`, `wiki_spaces` and `code_notes`. The main menu's **Delete all** entry calls it after a confirmation dialog. The UI requires explicit confirmation. The response is `{ "deleted": true, "messagesyncid": <int64> }`. Login and approval are retained. A durable `data.deleted` wallet notification carries the erasure cursor; browsers discard every cached message at or before that cursor and ignore delayed older events. Newer messages survive notification replay. Content-free room/message tombstones and device erasure requests remain for synchronization. Interrupted cleanup resumes on startup.


### Workspace status and client packages

- `GET /api/status` requires an approved workspace session. It returns `stats` and `connections` scoped to the current wallet (shared capture nodes included), `updated_at`, and `package`. Counts exclude deleted rooms/messages. `file_bytes` and `screenshot_bytes` measure stored media only, excluding database/text/index overhead. Online totals and device lists include all matching clients. The interface initially shows six per type and expands the remaining devices on demand. Presence uses the same heartbeat thresholds as Devices.
- Only administrators receive the optional `system` object: aggregate content counts, registered-user counts by role, all client connections, provider configuration flags, semantic status, and demo status. No other wallet's message text, file contents, or memo titles are exposed. Root admin is counted even when not in the registration table.
- `package` reports availability, an actionable reason if unavailable, the host build platform, detected Tailscale IP URL, current wallet, and whether a node token is required.
- `GET /api/download/client.zip` requires an approved wallet session and returns `application/zip` with `Content-Disposition: attachment; filename="client.zip"`. The archive contains executable `client` and `go.sh` entries. URL and owner come exclusively from server address discovery and the authenticated session; the caller cannot select another owner. Tokens and user content are excluded. The endpoint is intentionally outside `/client/*`, which is reserved for node transport permissions. It returns 409 when the wallet, Tailscale IP, or client binary is unavailable.

Content-creating browser requests (`/api/think`, `/api/files`, and room `/snapshot`) include `data_epoch` and, for rooms, `clear_epoch`: the latest observed user-erasure and room-clear cursors. The server compares these under the submission gate before writing. A request that began before erasure is rejected even if a delayed file read or network delivery finishes afterward. Legacy callers may omit these fields; upgrade clients to obtain this protection.

## Invisible AI notes and personal execution policy

`GET /api/clients` and `GET /api/devices` include a per-account `textedit` state on capture devices: `ready`, `needs_enrollment`, `offline`, `update_required`, or `unavailable`. `can_capture` is independent: a shared Mac may allow screenshots while requiring wallet enrollment for TextEdit. These fields guide the UI; session creation, start and worker access retain server-side authorization checks.

See [docs/invisible-ai.md](docs/invisible-ai.md) for setup, lifecycle, and limitations.

- `GET /api/ai/preferences` returns the wallet's saved selection, otherwise the admin's saved default, or `null` when neither exists.
- `PATCH /api/ai/preferences` accepts `{ "mode": "ondevice|hybrid|cloudai", "local_provider": "apple|mlx", "cloud_provider": "grok|openai|anthropic|gemini" }`. Ordinary approved users may change their own selection. Hybrid always requires a per-note cloud decision; there is no automatic escalation flag.
- `GET /api/capture/choices` returns the wallet's saved capture computer choices `{ "active", "default", "continuous": [] }`, or `null` before any choice. `PATCH /api/capture/choices` saves them (ids are validated for shape only, so a disconnected computer stays chosen). `Settings.user_capture` carries the same value, and a browser without a local copy adopts it on load, so a new login or device starts with the same capture computer.
- `Settings.local_ai` is admin-owned: `local_only`, `apple_enabled`, `mlx_enabled`, `mlx_url`, `speech_provider`, `speech_locale`, `mlx_speech_model`, and `auto_interval_seconds`. `Settings.user_ai` is the requesting user's effective selection. `Settings.default_ai` holds the optional admin default with the same mode/local-provider/cloud-provider shape. Admins save it through `PATCH /api/settings`; it persists in JSONL and SQLite, applies only when a user has no saved preference, and never permits unapproved Hybrid cloud requests. Without either, `GET /api/ai/preferences` returns the computed default shared with the browser: Hybrid when a local and a cloud provider are both configured, On-device when only a local provider is configured or Local-only is enforced, otherwise Cloud AI with the selected provider. Local-only enforcement takes precedence. Invalid updates do not change either store.
- `GET /api/rooms/{id}/notes` returns `active`, `client_id`, `document`, `stage`, `error`, and nullable `approval`.
- `POST /api/rooms/{id}/notes` accepts `{ "client_id": "…" }`; it uses the admin interval and current wallet policy.
- `DELETE /api/rooms/{id}/notes` stops scheduled processing and sends a stop command to the CLI. Backend restart and CLI disconnect do not resume Auto.
- `UserAiPreferences.mode` also accepts `codex` and `claude_code`; `provider()` then returns the same id. These providers appear in `/api/settings` `providers` with `configured` true when the `codex` or `claude` executable is found. Requests run the tool as a subprocess on the backend (no API key; the tool's own login) with the conversation flattened to one prompt; they are cloud-located, text-only, and refused under Local-only.
- `GET /api/memos/{id}` includes `usage` (`elapsed_ms,first_token_ms,input_tokens,output_tokens,estimated_usd,calls`) summed over the memo's model calls. Wiki feed items and tag-document `sources` carry `cloud_calls`, `input_tokens`, `output_tokens`, `estimated_usd`, `cloud_model`, `has_image`, `tags`, `room` and `attachments` (`id,name,kind` with kind image/video/file); document Markdown embeds `/api/memos/{id}/image`, `/api/files/{id}/image` and video frames from `/api/files/{id}/thumbnail`. Uploaded videos (`video/*`, detected by the MPEG-4 `ftyp` signature or extension) are processed as one Quick Look frame.
- SQLite schema: `schema::ensure` adds missing tables and columns from the declarations at startup; `PRAGMA user_version` tracks `schema::VERSIONED` steps.
- `GET /api/wiki/graph` returns `{nodes:[...],edges:[...]}` for the signed-in wallet: tag nodes `{id:"tag:<tag>",kind:"tag",title,size,document}` (the 150 most used tags) and message nodes `{id,kind,title,url,thumbnail,media,created_at,tags}` (the 300 newest messages; `media` lists what the node can show or play in place, in the order it was kept: `{kind:"image"|"video",url,poster,name}` with `url` at `/api/memos/{id}/image`, `/api/files/{id}/image` or `/api/files/{id}/stream` and `poster` the backend's thumbnail; a photo is a message with a capture or an image attachment and `thumbnail` points at `/api/memos/{id}/thumbnail` or `/api/files/{id}/thumbnail`); edges are `{from:<memo>,to:"tag:<tag>",kind:"tag",weight:1}` and `{from:"tag:a",to:"tag:b",kind:"related",weight:<messages that carry both>}`. `GET /api/wiki/related/{id}` returns `{item,related:[...],semantic}`: up to 12 messages that share the most tags with the message, fused (reciprocal rank) with its nearest stored-embedding neighbours; 404 for another wallet's message. Each completed memo is analyzed once into `wiki_topics` (tags, keywords) by the model the account's AI mode calls for: the on-device model in On-device mode; the chosen cloud provider in Hybrid and Cloud AI modes (in Hybrid the local model only decides whether an answer needs the cloud, it never tags), falling back to the local model and then to a word-frequency heuristic; Codex and Claude Code modes tag locally. The call is recorded in `ai_usage` like any other, so the message and its Wiki card show it. The prompt lists the owner's most used tags so the model reuses them. Existing rows are not re-analyzed when a memo is edited; at startup, rows a model left with no tags (`topics` empty, provider neither `user` nor `heuristic`) are reset and re-analyzed one at a time in the background.
- `PATCH /api/assistance/preferences` also accepts `local_gate` (boolean, default true): in `cloudai`, `codex` and `claude_code` modes a changed note is first classified by the enabled local provider ("Checking with the local LLM…"); a reply starting with NO logs "No request found · cloud AI skipped" and calls nothing else. Unchanged notes call no model in any mode.
- `PATCH /api/assistance/preferences` also accepts `hybrid_confirm` (boolean, default false). When false, a Hybrid worker whose local model answers `[CLOUD_HELP]` immediately runs the configured cloud provider on the note text, logs "Thinking with cloud help…", and appends `[Invisible AI · cloud]`; no approval is created. When true, the single-use approval below is used.
- `POST /api/rooms/{id}/notes/approval` accepts `{ "id": "…", "approve": true|false }`. The server consumes a matching approval once, checks current policy and exact note contents before a cloud request, and checks again before insertion.
- `GET /api/usage` returns wallet-scoped provider aggregates plus nullable credit/savings fields. Missing provider token counts are not replaced with estimates.
- `GET /api/activity/summary` returns the caller's wallet-wide processing numbers: `day` (last 24 hours) and `total`, each with `ocr_runs`, `skipped_unchanged`, and `local`/`cloud` buckets of `{requests,running,errors,elapsed_ms,input_tokens,output_tokens,estimated_usd,providers}`; plus `last_at` and `price_date`. Chat messages from AI replies carry `usage` (`elapsed_ms,first_token_ms,input_tokens,output_tokens,estimated_usd,calls`) summed over the reply's provider calls once the reply finishes. Providers report tokens, not prices, so per-call cost is always tokens × the dated list price, or null.
- Text-mode captures in a room compare their recognized text with the room's previous capture before any model runs. Identical text (a clock or cursor change is not a change) discards the capture, counts it in `capture.unchanged_count`, logs "Screen text unchanged · AI skipped", and calls no local or cloud model. Pixel-identical screenshots are discarded earlier, as before.

A Mac CLI advertises `"textedit_notes_v1": true` in its existing hello. The server sends `notes_command` frames over the capture WebSocket even if screen callbacks use SSE. Each frame has `request_id`, `generation`, `operation` (`begin`, `read`, `append`, `stop`), `app` (`textedit`, `notes`, or `none` on `open_url` frames of a remote-only run), and `expires_at` (Unix milliseconds). A read is bound to `document`. An append also carries exact `expected` text and a literal `suffix`. The CLI responds with `notes_result`, matching `request_id`, `ok`, and either `{ "snapshot": { "document": "…", "text": "…" } }` or a bounded error. Session checks, generation invalidation, expiry, and replay rejection apply. This capability never executes model-generated code or controls Terminal.

With `app: "auto"` (the default for new sessions) the CLI resolves the app itself: it reads the front application's bundle identifier through AppKit (no Apple event), prefers Notes when Notes is in front and TextEdit otherwise, and falls back to the other app when the first reports `no_document`. Every snapshot then carries `app` (`textedit` or `notes`) and failed results carry the `app` that was tried. Auto-mode reads are sent without `document`, so a changed document is a rebinding rather than an error; appends always name the concrete `app` and `document`. With `app: "terminal"` the document is the front Terminal.app tab (identity `terminal:` + TTY) or iTerm2 session (`iterm:` + session id), `text` is the visible contents, and an append is verified by the script, then written by the CLI to the reported `/dev/ttys*` device as display output (no shell input, no command); the server passes the last 40 entered lines after the previous answer to the local model's terminal triage (`[NO_REQUEST]`, a direct answer, or `[CLOUD_HELP]`); only `@cloud`/`@codex`/`@claude` skips that check. With `app: "notes"` the CLI drives Apple Notes instead of TextEdit: the document is the single selected note (identity `notes:` + the note id), `text` is the note's `plaintext`, and an append writes the suffix as escaped `<div>` HTML to `body`, then verifies that the resulting plaintext ends with the suffix. A CLI that also advertises `"mac_actions_v1": true` accepts one more operation, `open_url`, carrying only a validated `http`/`https` `url`; it runs `/usr/bin/open -a Safari <url>` and answers `{ "ok": true, "opened": "…" }`. No other application, keystroke, script, or page content is involved.

- `POST /api/invisible/sessions/{id}/prompt` accepts `{ "text": "…" }` (1–4000 characters) for a session whose autonomous run is active, queues it (at most 8 waiting), and returns 202 with the session. The worker answers queued prompts between note checks: explicit commands (`open …`, a bare URL or domain, `search …`, `calc …`, plain arithmetic; Korean verbs 열어/검색/계산 too) run without a model; other text goes to the configured model with an action guide that permits at most three `[OPEN_URL] …`, `[SEARCH] …`, or `[CALC] …` marker lines, each validated by the backend before it runs. Note text never triggers an action. The exchange is saved as chat history (provider `remote` when no model ran) and appended to the watched note as `[Invisible AI · remote]`; the status carries `queued_prompts`, `last_prompt`, `last_reply`, and `last_action`. Hybrid sessions answer remote prompts locally without a cloud approval; a remote reply appended to the note while a Hybrid approval is pending changes the note, so that approval expires as any edit would.
- `app` values: `auto`, `textedit`, `notes`, `terminal`, `none`.
- `PATCH /api/invisible/sessions/{id}` also accepts `read_screen` (boolean, applied live; status field `read_screen`, default true): when set, every new note or remote request first requests one `screen` capture on the bound Mac through the ordinary capture request path, saved as the message's image; local providers get its Apple Vision text, cloud providers get the image in Image mode. It also accepts `app` (`auto`, `textedit`, `notes`, or `none`) while the run is stopped; `POST /api/invisible/sessions` accepts the same `app`. With `none` the worker reads no note and only serves remote prompts.

Failed results may also include `error_code`: `note_changed`, `document_changed`, `no_document`, `permission_denied`, `expired`, `timed_out`, or `operation_failed`. These codes carry no script output or note text. Older clients can still return `error` alone. For ordinary Auto replies, only `note_changed` keeps the worker active: the generated answer remains in the room's history, the stale append is discarded, and the updated note is read after the next interval and settling period. Document changes and other failures stop the worker with a specific recovery message. A Hybrid approval remains single-use and is never automatically retried against changed text.

## Audio Capture

Audio Capture is a room-backed listening session, shared by every device signed in to the account. The capture Mac records its microphone; the backend transcribes each utterance locally and writes it into the room as a **Heard** message. **Solve** asks the configured AI about the newest speech; **Note** asks it to write up the session. Cloud requests happen only on these explicit actions. When available, a local model can condense older speech in the background while listening.

A Mac CLI advertises `"audio_capture_v1": true` in its hello (also set with the test-only `--fixture-audio FILE.wav`, which replays a WAV instead of the microphone). `GET /api/clients` reports the capability as `audio`, true only while that connection is online and the caller may capture with it. The microphone opens only on a `start` command, never when the CLI launches.

The server sends `audio_command` frames over the capture WebSocket (never SSE), each with `request_id`, `generation`, `operation` (`start` or `stop`) and `expires_at` (Unix milliseconds). The CLI answers `audio_result` with the matching `request_id`, `ok`, and an `error` text when it could not open the microphone (no input device, or microphone access not granted to the app that started the CLI). While a generation listens, the CLI cuts the signal into utterances at pauses of about a second (at most 30 seconds each, 16 kHz mono 16-bit PCM WAV) and sends each one:

```json
{"type":"audio_chunk","generation":"session-generation","seq":1,"seconds":2.4,"wav_base64":"BASE64_WAV"}
{"type":"audio_ack","seq":1,"ok":true,"error":null}
```

A chunk is accepted only for the live generation of that connection, in increasing `seq`, as a RIFF WAV of at most 4 MB; the acknowledgement says whether it was queued, not whether it was transcribed. The backend queues at most 16 utterances and transcribes them one at a time with the **local** speech engine: an explicit Apple/MLX Speech setting wins; otherwise a configured MLX speech model selects MLX Whisper, with Apple Speech as the default. Audio never goes to a cloud transcriber, regardless of AI mode. Three consecutive transcription failures end the session with an error. Disconnecting the CLI ends it with a visible disconnected stage.

Live feedback follows `rust-cli → backend → browser`: the CLI sends `audio_activity` frames (`generation`, `listening`, normalized `level`, optional `error`) every 200 ms of microphone input and `audio_preview` frames (`generation`, upcoming utterance `seq`, `seconds`, `wav_base64`) about once per second while speech continues. Previews contain the current utterance so provisional words can be revised; they are best-effort, local-only, and do not create chat messages. The backend keeps only the newest pending preview, gives each preview decode a five-second timeout, and rejects stale generations/connections and previews superseded by final chunks. Final `audio_chunk` frames still follow the reliable utterance path. Stop lets the recorder flush its last words and waits up to ten seconds for final transcription before closing the backend run.

`GET /api/audio/events` streams changed `AudioStatus` snapshots as SSE, scoped to the authenticated account and rechecking authorization during the stream. `listening`, `input_level` and `partial_text` are ephemeral: the frontend shows the microphone animation and input meter only with fresh capture activity, and displays a revisable draft until the final transcript is saved. Activity older than two seconds stops the animation; stop, failure and disconnect clear the live state. The frontend reconnects automatically and retains ordinary status polling as a fallback. Recognition latency depends on the local speech engine; previews are incremental snapshots, not guaranteed word-by-word timestamps. Reduced-motion preferences disable decorative pulsing while retaining the level indicator and status text.

One active listener per account, with multiple saved rooms:

| Method/path | Request/result |
| --- | --- |
| `GET /api/audio` | `AudioStatus`: `active,listening,input_level,partial_text,client_id,client_name,room_id,room_title,rooms,stage,error,segments:[{seq,text,seconds,at}],started_at,pending_chunks,heard_seconds,solve_memo,solved_through,summary,summarized_through,speech_provider,provider,model`. `rooms` contains up to 30 Audio Capture rooms, newest first, as ordinary `Room` records |
| `POST /api/audio` | `{"client_id":"stable-client-id"}`; starts listening into the current room (created on first use) on that online, permitted, capable Mac. Returns the status; 409 if already listening. Demo mode accepts an empty `client_id` and supplies fixture speech |
| `DELETE /api/audio` | Stops listening and closes the microphone. Already transcribed speech remains in the room; stopping does not run the AI or create a separate note |
| `POST /api/audio/solve` | Posts the short user message `Solve` and starts an answer to the newest speech, with earlier speech and answers as context. Returns status with `solve_memo` identifying the answer memo |
| `POST /api/audio/note` | Posts `Note` and starts a write-up of the session using its summary and remaining transcript. The result is a memo of kind `note` in the room, processed by the ordinary Wiki pipeline. Returns status with `solve_memo`; does not advance `solved_through` |
| `POST /api/audio/room` | `{"fresh":true}` creates and selects a new Audio Capture room; `{"room_id":"…"}` selects an existing, non-deleted Audio Capture room belonging to this account; `{}` gets or creates the current one. Returns status. An active microphone continues listening and writes subsequent speech into the selected room |

The current-room pointer is `audio.room.<owner>`; each room's transcript, sequence counters, heard duration, rolling summary and Solve cursor are saved as `audio.state.<roomid>`. History and these values survive backend restarts; listening must be explicitly restarted. Room selection is shared across devices. **Open in Ask anything** selects that same room through `POST /api/rooms/thought`.

Heard messages have `role:"user"` and `memo_kind:"audio"`. A message grows as utterances arrive, keeping its message ID and sequence while receiving newer sync revisions. A new block starts at about 6,000 bytes or after Solve/Note, preserving conversation order. The Audio Capture page reads the conversation through `GET /api/rooms/{id}/sync?after=…`, including streamed answer revisions and erasure cutoffs; there is no separate audio message store.

Solve and Note use the ordinary think pipeline with `ask:true`, a short visible prompt and a separate bounded `context`. Earlier answers come from normal room history, which excludes raw `audio` messages to avoid repeating the transcript. Speech older than the 12,000-byte window is folded into a rolling summary of at most about 4,000 bytes. Background condensation uses only an available local model; Solve/Note finish any outstanding condensation using the local model or the configured answer provider. Solve includes the summary, at most 4,000 bytes of already answered speech and the newest speech, within a 24,000-byte transcript budget plus instructions. `solved_through` marks the last Solve's coverage and `summarized_through` marks the summary's coverage.

Clearing or deleting an audio room stops any listener writing into it and erases its transcript state with its messages; a cleared room remains an empty Audio Capture room, while a deleted room disappears from the picker. Delete all also forgets the account's active listener and current-room pointer.

## Personal assistance settings and Wiki

- `GET /api/assistance/preferences` returns the authenticated wallet's settings. `PATCH` merges only supplied fields atomically and rejects unknown/invalid fields. Defaults: `wiki_enabled:false`, `wiki_auto_update:true`, `wiki_group_by:"topic"` (`topic|day`), `wiki_include_answers:true`, `textedit_enabled:true`, `textedit_settle_ms:1200` (500–10000), `textedit_style:"concise"` (`concise|detailed`). TextEdit changes stop that wallet's active Auto runs and invalidate approvals. These controls never override admin AI policy or grant macOS permissions.
- `GET /api/rooms/{id}/notes` additionally returns `textedit_enabled`. Starting Auto while this setting is false is rejected before a device command.
- `GET /api/wiki?q=...&tag=...&offset=0` returns the message feed for the authenticated wallet: `{items:[{id,title,snippet,kind:"message"|"photo",memo_kind,created_at,provider,has_answer,tags,room:{id,title,url}|null,url,thumbnail,media,attachments,cloud_calls,cloud_model,input_tokens,output_tokens,estimated_usd}],total,page_size:30,query,active_tags,tags:[{tag,count,document}],related_tags:[{tag,count}],settings,semantic}`. Words in `q` rank messages by BM25 over the full-text index fused with semantic similarity (reciprocal rank); `#tag` words in `q` and the `tag` parameter filter (a message must carry every active tag); with no words the feed is newest first. `tags` counts every tag of the wallet (300 most used) and `document` is the id of the tag's derived document (null under day grouping); `related_tags` are the tags that appear together with the active ones. `room.url` is `/chat?room={roomid}`; a message kept outside a chat has `room:null`. Query length is limited to 1000 bytes, `tag` to 200, offset to 1000000.
- Apple Vision names what is in a kept picture, or in one frame of a kept video (the cached thumbnail is what gets classified): its confident labels (0.3 and up, six at most) become the memo's hashtags (`memos.tags`, so they lead like typed hashtags) and a "Seen in the picture: …" line in the memo text, for kept files, library uploads and chat attachments alike. Files kept without a chat message are linked to their memo through `files.origin_memo`, so the Wiki shows them as photo or video nodes with thumbnails.
- Thumbnails (`/api/memos/{id}/thumbnail`, `/api/files/{id}/thumbnail`) are JPEGs made once and cached under `data/thumbnails/<id>.jpg` (warmed right after an upload, dropped with the file or image); a video gives one frame, and a picture the decoder cannot read (HEIC, RAW) is rendered by QuickLook. `POST /api/files` with `"wiki": true` and no room (the Images and Videos pages) also keeps the upload as a memo in the background, so it is tidied, tagged and filed in the Wiki as a photo or video; deleting the file deletes that memo.
- `GET /api/videos` lists uploaded videos (files with a `video/*` MIME type), newest first: `{id,name,mime,size,created_at,url,download,thumbnail}` where `url` is `/api/files/{id}/stream`. `GET /api/files/{id}/stream` serves a file inline for playback with `Accept-Ranges: bytes`; a `Range: bytes=a-b` request returns 206 with `Content-Range`, an unsatisfiable range 416. The web route `/videos` is the Videos page (upload by button or drop, in-place playback, name search, download, delete). The Wiki map has zoom buttons, a map-only fullscreen, pinch-to-zoom on touch (the canvas sets `touch-action: none`), a single tap through the `wiki-select` custom event (a tag filters; a message opens in place: a photo or video node shows its pictures and clips in a lightbox, any other message opens the in-Wiki reader) and a double tap through `wiki-open` (a tag shows its messages, a message opens the reader). Nothing on the Wiki page navigates away: card thumbnails open the lightbox (a clip plays from `/stream` with controls and autoplay), card titles open the reader, and inside a tag document a click on a picture, a clip's frame or an `/memos/{id}` link is intercepted the same way. Readers and the lightbox are overlays above the collection, which keeps its scroll, map camera and search; the one Back (or Esc) closes the top layer only, so a message opened from a document returns to that document. An open tag shows a Close control beside its name: the page keeps a stack (20 deep) of the screen each tag was opened from (tag, search, page offset, feed or map, related panel, open reader, scroll position) and Close restores the latest one, pushing its route; showing everything empties the stack. The search box offers earlier searches that found something (kept in the browser's `localStorage`, 30 at most) and matching tags after the first typed letter.
- `GET /api/wiki/{id}` returns a tag document `{id,title,markdown,sources:[{id,title,created_at,provider,url,tags,room,...}],updated_at}` or 404. Source URLs point to `/memos/{id}`; a video frame in the Markdown links to `/api/files/{id}/stream`. `GET /api/wiki/memo/{id}` returns one message for the in-Wiki reader: the feed item plus the full `input`, the `answer` (kind markers stripped) and `media`; 404 for a message that is not in the Wiki. Web routes are `/wiki`, `/wiki/tag/{tag}`, `/wiki/map`, `/wiki/memo/{id}` and `/wiki/{id}`.
- `GET /api/code?q=...&lang=...&offset=0` lists the reusable code in the wallet's Wiki, function by function: `{items:[{id,memo_id,memo_title,memo_url,created_at,language,name,signature,params:[{name,type}],returns,code,description,tags}],total,offset,page_size:60,languages:[{language,count}],messages}`. The newest 400 kept messages with a fenced block are read; each fence is split by signature (Rust `fn`, Python `def`, JavaScript/TypeScript `function` and arrow `const`, Go `func`, Swift `func`, Kotlin `fun`) with the doc comments, attributes and decorators directly above a function kept with it; `params` is the parameter list (receivers such as `self` left out) and `returns` the declared return type; a block with no function and at least two lines is one item named by the message. Words in `q` must all appear in the name, signature, note, code, message title or `#tags`; `lang` keeps one canonical language (`rust`, `python`, `javascript`, `typescript`, `go`, `swift`, `kotlin`, `shell`, ... `text`, `other`); `languages` counts every language before the filters. `id` is 24 hex characters of a hash over the memo id, language, name and signature, so it survives edits to the body. `PATCH /api/code/{id}` with `{description}` keeps the user's own note on that function in `code_notes` (empty deletes it; 2000 characters at most) and returns `{id,description}`. Deleting a message drops its functions. The web route `/code` is the Code page (menu **Code**): one card per function with Input and Output sections, the code to copy, the note, a link to the message in the Wiki, search and language chips.
- `DELETE /api/wiki/tag/{tag}` removes one tag from every message of the wallet that carries it (typed hashtags, model-inferred topics and Wiki rows alike) and returns `{deleted:<tag>,messages:<count>}`. Each touched message becomes curated (`wiki_topics.provider='user'`, its remaining tags kept as the user's list in `memos.tags`), so a rebuild or a fresh analysis never brings the tag back; its document and chat-message tags are rewritten in the same transaction. A type, a category or `inbox` cannot be deleted (400). The Wiki heading offers **Delete tag** with a confirmation when a deletable tag is open; afterwards the screen the tag was opened from comes back.
- `POST /api/wiki/rebuild` adds all completed saved sources, rewrites the wallet's tags and regenerates documents, returning `{rebuilt:true}`. Requires Wiki enabled. With automatic updates enabled, source persistence performs the same organization; a rebuild itself invokes no AI provider.

The Wiki keeps organized material worth remembering; the chat keeps everything. A completed memo joins `wiki_sources` only after its analysis judged it worth keeping (`wiki_topics.keep`, from the tagging model's `keep` field or a heuristic: media, code, hashtags and records of three or more words are kept; one- to three-word questions, greetings and Ask results are not) or after the user kept it by hand (`PATCH /api/memos/{id}` with `{"wiki": true|false}`, which also removes one). `ChatMessage.wiki_state` is `pending`, `kept` or `skipped`, and the chat offers Keep in Wiki / Remove from Wiki accordingly. Every message leads with one type tag (`origin:type`, one of `q&a`, `note`, `code`, `photo`, `video`, listed in `causewaybay_shared::TYPES`; media first, then fenced code or a coding answer, then an answered question, otherwise a note), followed, for everything but a q&a, by one category tag (`origin:category`, one of `personal`, `information`, `idea`, `task`, `other` in `causewaybay_shared::CATEGORIES`): the category the tagging model returns (`wiki_topics.category`), or a heuristic one (first-person or diary words → `personal`, to-do words → `task`, otherwise `information`) once analyzed. A kept memo (`POST /api/memos`, or a chat line filed as a record) is tidied in the background by the account's AI into the memo's `answer` and becomes kind `note`; the Wiki shows the tidied text as the body with the original beneath it, and the tag document lists "Tidied note" before "Original". Feed items and map nodes carry `kind` (and `type`) equal to the type tag (a video node shows a frame thumbnail like a photo); the rail lists types, then categories, then topic tags, and `tags[]` entries carry `type` and `category` flags. Types and categories count in the rail and filter the feed like tags, are kept out of the map, of related-message scoring and of "also tagged", and are not part of the editable tag list (an edit preserves them). Feed items carry `category` and, for q&a, `answer_snippet` (the answer's first 240 characters) so the question and its answer travel together. Each message's tags are stored in `wiki_tags(memo_id,owner,tag,origin)` (owner-scoped, cascading on memo deletion, rewritten inside the same transaction as the documents): the user's own first (`origin:user`: hashtags, or the list edited in the chat or through `PATCH /api/memos/{id}`, twelve at most), then the local model's (`model`) until there are five; word-frequency fallback tags (`heuristic`) apply only when the user wrote no hashtag, a curated message keeps exactly the user's list (`wiki_topics.provider='user'`, never overwritten by analysis), and an analyzed message with no tag at all is `inbox` (`system`); before its analysis has run a message has no tags. After every rewrite, each chat message (post and generated response alike) whose tags changed gets a new revision carrying them in `tags`. Tags are normalized to one spelling (lowercase, `#` stripped, words joined by `-`, letters/digits/`_`/`/` only, 40 characters at most, a few shorthands such as `btc` expanded). Document IDs are hashes of grouping keys and part numbers. SQLite stores canonical membership and document content; nothing is mirrored to disk. Sources are joined against their owning wallet on every build/search. One document holds at most 40 source excerpts. Deletion cascades source membership and regenerates/removes derived pages inside the erasure critical section. Tag documents are derived rows, not independently editable canonical documents.


### Autonomous AI sessions

The UI and docs call these Autonomous AI sessions (page `/autonomous`); the API path keeps the older `invisible` name for compatibility. A backend session is a chatroom. `POST /api/invisible/sessions` uses the shared room creation path and accepts `{client_id?, title?, room_id?, start?}`. Omit `client_id` for memos/chat only; the first capture can bind one later. `start: true` also starts the TextEdit worker on `client_id` through the normal start checks; a refused start leaves the created session with `status.error` set instead of a 4xx. An existing `room_id` reuses that room and can only retain its existing node. Responses include `{id,title,client_id,created_at,updated_at,status,capture,capture_count,memo_count,recent_captures,activity}`; `status` describes optional TextEdit assistance (including `provider` and `model` for the running worker) and `capture` describes optional screenshot polling. `activity` is the session's progress view: `thinking` (the running inference's `memo_id,provider,model,location,waited_ms`, measured on the backend at read time), `entries` (newest-first steps with `at,source,stage,provider,model,location`, where `source` is `textedit`, `capture` or `chat` and `location` is `local`, `cloud` or empty), `inferences` (newest-first usage-ledger rows for this room: `memo_id,created_at,provider,model,location,status,elapsed_ms,first_token_ms,input_tokens,output_tokens,estimated_usd`), `totals` (`requests,elapsed_ms,input_tokens,output_tokens,estimated_usd,unpriced_cloud_requests`), and `price_date`. Token counts are provider-reported or null; `estimated_usd` is null unless the model has a listed price and both token counts exist. `recent_captures` contains up to four memo IDs for the existing thumbnail endpoint. The session ID is the chatroom ID, and listing sessions includes every owned live room. Every session created here gets a session row even without a client; the first capture or TextEdit start binds the client. A shared capture-only Mac may hold a session; starting TextEdit replies on it is refused with a 400 that explains enrollment, and `start: true` reports that refusal in `status.error`.

`GET /api/invisible/sessions` lists only the caller's sessions; `GET /api/invisible/sessions/{id}` reads one. `PATCH /api/invisible/sessions/{id}` accepts `{interval_seconds}`: 2–300 sets this session's own check interval for the autonomous AI run, 0 returns it to the admin default; a running worker applies it at its next pause, and a plain chatroom becomes an unbound session when it gets its own setting. The value is returned as `status.interval_seconds` (0 means the admin default applies). `POST .../{id}/start`, `POST .../{id}/stop`, and `POST .../{id}/approval` manage the worker; approval uses the existing `{id, approve}` single-use decision. These endpoints return the full session. The older `/api/rooms/{id}/notes` endpoints still return `NoteStatus` and register their runs in the same durable collection. Starting a worker answers the connect-time snapshot without a typing pause; the pause applies to every later read.

Workers survive UI disconnect, logout, and cookie expiry, but account revocation and device access loss stop work. Session metadata/status survives backend restart; input actions require explicit start after restart/reconnect. Approvals are never restored. Clearing history cancels workers and removes content while keeping the room and its capture binding; deleting the room also removes the session. See [Invisible AI](docs/invisible-ai.md#backend-owned-sessions) for a headless API example.


### Account model preference

`PATCH /api/ai/models` accepts `{"provider":"grok","model":"model-name"}` and stores it for the authenticated owner. `GET /api/settings` returns the owner-scoped `user_models` map. It applies to backend jobs as well as the picker; local installed models remain server-managed.
