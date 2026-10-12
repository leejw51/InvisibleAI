# Invisible AI — quick start

Two downloads, one per computer:

| Archive | Put it on | What it runs |
| --- | --- | --- |
| `invisible-ai-server-<version>-darwin-arm64.zip` | the Mac that keeps your data (e.g. a Mac Studio) | backend, web UI, on-device AI workers, and a local capture client |
| `invisible-ai-client-<version>-darwin-arm64.zip` | any other Mac whose screen you want to capture (e.g. a MacBook) | capture client only |

Both need macOS 26 on Apple silicon. The binaries are not notarized: if macOS blocks one, right-click it → **Open** once, or run `xattr -dr com.apple.quarantine <folder>` on the unzipped folder.

## 1. Run the server

```sh
unzip invisible-ai-server-*.zip
cd invisible-ai-server-*/
cp .env.example .env        # optional: add XAI_API_KEY or another cloud key
```

Then either double-click **Start Invisible AI.command**, or from Terminal:

```sh
./bin/server serve --bind 127.0.0.1:3000 --assets dist
```

Open <http://127.0.0.1:3000>. That is a complete single-Mac setup: the `.command` launcher also starts the local capture client. Keep its Terminal window open; Control-C stops everything.

No cloud key? Pick **On-device** in **AI settings** — Apple's model works immediately with Apple Intelligence on, and Qwen models download from **Backend settings**.

## 2. Let other devices connect (optional)

The default listener only accepts this Mac. To serve an iPad, iPhone or a second Mac, put both machines on [Tailscale](https://tailscale.com) and set a token in `.env`:

```sh
APP_TOKEN=pick-a-long-random-token-16plus
CWBBOT_ADMIN=0xYOUR_WALLET          # optional: wallet sign-in and approvals
```

```sh
./bin/server serve --bind 0.0.0.0:3000 --assets dist
```

Browse to `http://<server-tailscale-ip>:3000` from the other device.

## 3. Run the client on another Mac

```sh
unzip invisible-ai-client-*.zip
cd invisible-ai-client-*/
cp .env.example .env        # optional: fill BACKEND_URL, OWNER_WALLET, APP_TOKEN
./go.sh
```

`go.sh` asks for anything `.env` leaves empty: the backend URL (`http://100.x.x.x:3000`), the owner wallet shown on the server's Dashboard, and the node token. Allow **Screen Recording** for Terminal when macOS asks, then press **Capture** in the web UI.

Signed in on the server's web UI? **Dashboard → Download client.zip** gives you a launcher with the URL and wallet already filled in.

The client can also run directly:

```sh
APP_TOKEN=… ./client http://100.x.x.x:3000 0xYOUR_WALLET
./client            # later: reconnects with the saved settings
```

## Where things live

| | Server | Client |
| --- | --- | --- |
| Settings and keys | `~/.causewaybaybot/` (`.env` here also works) | `~/.causewaybaybotclient/` |
| Your data | `~/.causewaybaybot/data/` | nothing kept beyond settings |

Read `README.md` for the full manual.
