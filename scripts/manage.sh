#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
CONFIG="${CAUSEWAYBAY_CONFIG_DIR:-$HOME/.causewaybaybot}"
CLIENT_CONFIG="${CAUSEWAYBAY_CLIENT_CONFIG_DIR:-$HOME/.causewaybaybotclient}"
RUNTIME="$CONFIG/run"
CLIENT_RUNTIME="$CLIENT_CONFIG/run"
mkdir -p "$RUNTIME" "$CONFIG/logs" "$CLIENT_RUNTIME" "$CLIENT_CONFIG/logs"
chmod 700 "$CONFIG" "$RUNTIME" "$CONFIG/logs" "$CLIENT_CONFIG" "$CLIENT_RUNTIME" "$CLIENT_CONFIG/logs"

alive() {
  local dir="$RUNTIME"
  [[ "$1" == client ]] && dir="$CLIENT_RUNTIME"
  [[ -f "$dir/$1.pid" ]] || return 1
  local pid
  pid="$(cat "$dir/$1.pid")"
  [[ "$pid" =~ ^[0-9]+$ ]] && kill -0 "$pid" 2>/dev/null || return 1
  ps -p "$pid" -o command= | grep -Fq "$2"
}
start_server() {
  # Compare the actual build, not timestamps: Cargo can leave the executable
  # untouched, and an existing process keeps the old binary after a rebuild.
  local build running_build bind="${BIND_ADDR:-0.0.0.0:3000}"
  # A network listener needs wallet sign-in or a token; otherwise stay on this Mac.
  if [[ "$bind" != 127.* && "$bind" != localhost:* && "$bind" != '[::1]:'* && -z "${CWBBOT_ADMIN:-}" && -z "${APP_TOKEN:-}" ]]; then
    echo "Remote access needs CWBBOT_ADMIN or APP_TOKEN; listening on 127.0.0.1:3000 only." >&2
    bind=127.0.0.1:3000
  fi
  # The listen address is part of the build marker so a bind change restarts.
  build="$(cksum < "$ROOT/target/debug/causewaybay-server") $bind"
  if alive server causewaybay-server; then
    running_build="$(cat "$RUNTIME/server.build" 2>/dev/null || true)"
    if [[ "$running_build" == "$build" ]]; then
      echo 'Backend already running with the current build.'
      grep 'Web URL' "$CONFIG/logs/backend.log" || true
      return
    fi
    echo 'Backend build changed; restarting to load the current build.'
    stop_one server causewaybay-server
  fi
  cd "$ROOT"
  node "$ROOT/scripts/spawn-detached.mjs" "$CONFIG/logs/backend.log" "$ROOT/target/debug/causewaybay-server" serve --bind "$bind" --assets "$ROOT/dist" > "$RUNTIME/server.pid"
  printf '%s\n' "$build" > "$RUNTIME/server.build"
  for _ in {1..50}; do
    if ! alive server causewaybay-server; then cat "$CONFIG/logs/backend.log"; return 1; fi
    if grep -q 'Tailwind CSS' "$CONFIG/logs/backend.log"; then
      cat "$CONFIG/logs/backend.log"
      return
    fi
    sleep .2
  done
  echo "Backend did not become ready. Check $CONFIG/logs/backend.log" >&2
  return 1
}
report_client_status() {
  local log="$CLIENT_CONFIG/logs/client.log" state
  # This is the capturing process's permission check, not this shell's access.
  # Never echo its full startup log (which includes the saved owner wallet).
  grep -m 1 '^Screen capture permission:' "$log" || true
  grep -m 1 '^TextEdit Auto:' "$log" || true
  state="$(grep -E '^Connected\. Waiting|^Connection failed:|^Connection closed;' "$log" | tail -n 1 || true)"
  case "$state" in
    'Connected. Waiting'*) echo 'Capture client connected to backend.' ;;
    *)
      echo 'Capture client is running but not connected yet; it will retry automatically.'
      [[ -z "$state" ]] || printf '%s\n' "$state"
      ;;
  esac
  echo "Capture client log: $log"
}
wait_client() {
  local log="$CLIENT_CONFIG/logs/client.log"
  for _ in {1..50}; do
    if ! alive client 'causewaybay client'; then
      echo "Capture client exited during startup. Check $log" >&2
      grep -E '^(Error:|error:|Connection failed:)' "$log" >&2 || true
      rm -f "$CLIENT_RUNTIME/client.pid"
      return 1
    fi
    if grep -Eq '^Connected\. Waiting|^Connection failed:' "$log"; then
      # A fatal --once/argument failure must not count as a connected client.
      sleep .2
      if alive client 'causewaybay client'; then report_client_status; return; fi
    fi
    sleep .2
  done
  report_client_status
}
start_client() {
  local build running_build owner_changed=false
  build="$(cksum < "$ROOT/target/debug/causewaybay")"
  # An explicit enrollment request must reach the CLI even when its binary is
  # unchanged. Without a wallet argument the CLI preserves its saved owner.
  if [[ -n "${OWNER_WALLET:-}" ]] && ! node - "$CLIENT_CONFIG/client.jsonl" <<'JS'
const fs = require('node:fs');
try {
  const rows = fs.readFileSync(process.argv[2], 'utf8').trim().split('\n');
  const config = JSON.parse(rows.at(-1)).config;
  process.exit(config?.owner_wallet === process.env.OWNER_WALLET?.toLowerCase() ? 0 : 1);
} catch { process.exit(1); }
JS
  then owner_changed=true; fi
  if alive client 'causewaybay client'; then
    running_build="$(cat "$CLIENT_RUNTIME/client.build" 2>/dev/null || true)"
    if [[ "$running_build" == "$build" && "$owner_changed" == false ]]; then
      echo 'Capture client already running with the current build.'
      report_client_status
      return
    fi
    if [[ "$owner_changed" == true ]]; then
      echo 'Capture client account changed; restarting to enroll this Mac.'
    else
      echo 'Capture client build changed; restarting to load the current build.'
    fi
    stop_one client 'causewaybay client'
  fi
  cd "$ROOT"
  local client_args=(client --backend "${CAUSEWAYBAY_BACKEND:-http://127.0.0.1:3000}")
  if [[ -n "${OWNER_WALLET:-}" ]]; then client_args+=(--owner-wallet "$OWNER_WALLET"); fi
  if [[ -n "${CAUSEWAYBAY_CLIENT_NAME:-}" ]]; then client_args+=(--name "$CAUSEWAYBAY_CLIENT_NAME"); fi
  node "$ROOT/scripts/spawn-detached.mjs" "$CLIENT_CONFIG/logs/client.log" "$ROOT/target/debug/causewaybay" "${client_args[@]}" > "$CLIENT_RUNTIME/client.pid"
  printf '%s\n' "$build" > "$CLIENT_RUNTIME/client.build"
  echo "Capture client started. Log: $CLIENT_CONFIG/logs/client.log"
  wait_client
}
stop_one() {
  local name="$1" expected="$2" pid dir="$RUNTIME"
  [[ "$name" == client ]] && dir="$CLIENT_RUNTIME"
  if alive "$name" "$expected"; then
    pid="$(cat "$dir/$name.pid")"
    kill -TERM "$pid"
    for _ in {1..100}; do kill -0 "$pid" 2>/dev/null || break; sleep .1; done
    if kill -0 "$pid" 2>/dev/null; then
      echo "$name did not stop in 10 s; killing PID $pid"
      kill -KILL "$pid" 2>/dev/null
      for _ in {1..20}; do kill -0 "$pid" 2>/dev/null || break; sleep .1; done
    fi
    echo "$name stopped."
  else echo "$name is not running."; fi
  # Remove only a PID file managed by this script, never user data.
  rm -f "$dir/$name.pid"
}
# The backend stops its on-device workers itself; this sweeps any that an
# earlier build or a hard kill left behind, matched by this repository's path
# so another checkout's workers are untouched.
stop_workers() {
  local pattern found
  for pattern in "$ROOT/bin/invisible-mlx" "$ROOT/bin/invisible-apple-ai" "$ROOT/target/debug/causewaybay-server" "$ROOT/target/release/causewaybay-server"; do
    found="$(pgrep -f -- "$pattern" || true)"
    [[ -z "$found" ]] && continue
    pkill -TERM -f -- "$pattern" 2>/dev/null || true
    for _ in {1..30}; do pgrep -f -- "$pattern" >/dev/null || break; sleep .1; done
    if pgrep -f -- "$pattern" >/dev/null; then pkill -KILL -f -- "$pattern" 2>/dev/null || true; fi
    echo "stopped leftover ${pattern##*/} ($found)"
  done
}
case "${1:-status}" in
  start) start_server; start_client ;;
  start-server) start_server ;;
  start-client) start_client ;;
  stop) stop_one client 'causewaybay client'; stop_one server causewaybay-server; stop_workers ;;
  stop-client) stop_one client 'causewaybay client' ;;
  status) for name in server client; do if alive "$name" causewaybay; then echo "$name running"; else echo "$name stopped"; fi; done ;;
  *) echo 'Usage: manage.sh start|start-server|start-client|stop|stop-client|status'; exit 2 ;;
esac
