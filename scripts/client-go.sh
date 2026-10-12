#!/bin/sh
set -eu
# This launcher contains connection details only, never a recovery phrase or API key.
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
backend_url=@@BACKEND_URL@@
owner_wallet=@@OWNER_WALLET@@
# Optional .env beside this script (see .env.example). Only known keys are read and
# nothing in it is executed. Values baked into this launcher and variables already
# in the environment take precedence.
if [ -f "$script_dir/.env" ]; then
  while IFS='=' read -r key value || [ -n "$key" ]; do
    [ -n "$value" ] || continue
    case $key in
      BACKEND_URL) [ -n "$backend_url" ] || backend_url=$value ;;
      OWNER_WALLET) [ -n "$owner_wallet" ] || owner_wallet=$value ;;
      APP_TOKEN|CAUSEWAYBAY_CLIENT_CONFIG_DIR)
        eval "current=\${$key-}"
        [ -n "$current" ] || export "$key=$value" ;;
    esac
  done < "$script_dir/.env"
fi
if [ -z "$backend_url" ]; then
  printf 'Backend Tailscale URL (http://100.x.x.x:3000): '
  IFS= read -r backend_url
fi
if [ -z "$owner_wallet" ]; then
  printf 'Owner wallet address (from your signed-in Dashboard): '
  IFS= read -r owner_wallet
fi
if [ -z "$backend_url" ] || [ -z "$owner_wallet" ]; then
  printf '%s\n' 'A backend URL and owner wallet address are required.' >&2
  exit 1
fi
if [ @@TOKEN_REQUIRED@@ = true ] && [ -z "${APP_TOKEN:-}" ]; then
  printf 'Backend node token (input hidden): '
  if [ -t 0 ]; then
    trap 'stty echo' EXIT HUP INT TERM
    stty -echo
  fi
  IFS= read -r APP_TOKEN
  if [ -t 0 ]; then stty echo; trap - EXIT HUP INT TERM; fi
  printf '\n'
  [ -n "$APP_TOKEN" ] || { echo "A backend node token is required." >&2; exit 1; }
  export APP_TOKEN
fi
exec "$script_dir/client" "$backend_url" "$owner_wallet" "$@"
