#!/bin/bash
# Double-click to run. Closing this Terminal session stops both processes.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
export CAUSEWAYBAY_CONFIG_DIR="${CAUSEWAYBAY_CONFIG_DIR:-$HOME/.causewaybaybot}"
export CAUSEWAYBAY_CLIENT_CONFIG_DIR="${CAUSEWAYBAY_CLIENT_CONFIG_DIR:-$HOME/.causewaybaybotclient}"
umask 077
mkdir -p "$CAUSEWAYBAY_CONFIG_DIR"
# A directory lock avoids launching two copies from repeated double-clicks.
LOCK="$CAUSEWAYBAY_CONFIG_DIR/desktop-launch.lock"
if ! mkdir "$LOCK" 2>/dev/null; then
  echo "Invisible AI is already open, or a previous session ended unexpectedly."
  echo "Close the other session. If none is running, remove $LOCK and try again."
  exit 1
fi
server_pid= client_pid=
cleanup() {
  trap - EXIT INT TERM HUP
  [ -z "$client_pid" ] || kill "$client_pid" 2>/dev/null || true
  [ -z "$server_pid" ] || kill "$server_pid" 2>/dev/null || true
  # A browser WebSocket can keep graceful server shutdown open indefinitely.
  # Give owned children a short grace period, then ensure Terminal can close.
  for attempt in {1..30}; do
    alive=false
    for pid in "$client_pid" "$server_pid"; do
      if [ -n "$pid" ] && kill -0 "$pid" 2>/dev/null; then alive=true; fi
    done
    [ "$alive" = true ] || break
    sleep 0.1
  done
  for pid in "$client_pid" "$server_pid"; do
    [ -z "$pid" ] || kill -KILL "$pid" 2>/dev/null || true
  done
  [ -z "$client_pid" ] || wait "$client_pid" 2>/dev/null || true
  [ -z "$server_pid" ] || wait "$server_pid" 2>/dev/null || true
  rmdir "$LOCK" 2>/dev/null || true
}
trap cleanup EXIT INT TERM HUP
"$ROOT/bin/server" serve --bind 127.0.0.1:3000 --assets "$ROOT/dist" &
server_pid=$!
ready=false
for attempt in {1..60}; do
  kill -0 "$server_pid" 2>/dev/null || { wait "$server_pid"; exit 1; }
  if [ -s "$CAUSEWAYBAY_CONFIG_DIR/capture-token" ] && /usr/bin/curl --silent --fail http://127.0.0.1:3000/api/health >/dev/null; then ready=true; break; fi
  sleep 1
done
[ "$ready" = true ] || { echo "Startup timed out. See the error above."; exit 1; }
"$ROOT/bin/client" client --backend http://127.0.0.1:3000 &
client_pid=$!
/usr/bin/open http://127.0.0.1:3000
printf '\nInvisible AI is ready. Keep this window open. Press Control-C to stop.\n'
wait "$server_pid"
