#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PLATFORM="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
# Cargo.toml is the version of record; the release workflow checks tags against it.
VERSION="${VERSION:-$(make -s version)}"
SERVER="invisible-ai-server-$VERSION-$PLATFORM"
CLIENT="invisible-ai-client-$VERSION-$PLATFORM"
DEST="package/$SERVER"
mkdir -p package
STAGING="$(mktemp -d "$ROOT/package/.stage.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
BUILD="$STAGING/$SERVER"
# Assemble only current release inputs; previous output may contain stale assets or .env files.
mkdir -p "$BUILD/bin" "$BUILD/dist" "$BUILD/docs"
cp bin/server "$BUILD/bin/server"
cp bin/client "$BUILD/bin/client"
for file in invisible-apple-ai invisible-mlx mlx.metallib; do
  test -f "bin/$file" || { echo "Missing bin/$file; run make package" >&2; exit 1; }
  cp "bin/$file" "$BUILD/bin/$file"
done
mkdir -p "$BUILD/scripts/local-ai"
cp scripts/local-ai/transcribe.py "$BUILD/scripts/local-ai/"
cp scripts/distribution-start.command "$BUILD/Start Invisible AI.command"
chmod 755 "$BUILD/Start Invisible AI.command"
cp native/mlx-ai/THIRD_PARTY_NOTICES "$BUILD/THIRD_PARTY_NOTICES"
cp -R dist/. "$BUILD/dist/"
cp packaging/README.md packaging/QUICK.md "$BUILD/"
cp docs/*.md "$BUILD/docs/"
cp PLAN.md PROTOCOL.md LICENSE "$BUILD/"
cp node_modules/motion/LICENSE.md "$BUILD/docs/MOTION-LICENSE.md"
cp .env.example "$BUILD/.env.example"
sed 's|frontend/assets/|dist/assets/|g' docs/artwork.md > "$BUILD/docs/artwork.md"
# COPYFILE_DISABLE keeps macOS from adding hidden AppleDouble files beside the executables.
(cd "$STAGING" && COPYFILE_DISABLE=1 /usr/bin/zip -q -X -r -y "$STAGING/server.zip" "$SERVER")
mv "$STAGING/server.zip" "package/$SERVER.zip"
mkdir -p "$DEST"
rsync -a --delete "$BUILD/" "$DEST/"
node scripts/package-client.mjs "$STAGING/$CLIENT" "$ROOT/package/$CLIENT.zip"
(cd package && shasum -a 256 "$SERVER.zip" "$CLIENT.zip" > SHA256SUMS)
echo "Server:  $ROOT/package/$SERVER.zip"
echo "Client:  $ROOT/package/$CLIENT.zip"
echo "Unpacked server: $ROOT/$DEST"
