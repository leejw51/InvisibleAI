#!/usr/bin/env bash
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"
PLATFORM="$(uname -s | tr '[:upper:]' '[:lower:]')-$(uname -m)"
DEST="package/causewaybay-$PLATFORM"
mkdir -p package
STAGING="$(mktemp -d "$ROOT/package/.stage.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
BUILD="$STAGING/causewaybay-$PLATFORM"
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
sed 's|frontend/assets/|dist/assets/|g' README.md > "$BUILD/README.md"
cp docs/*.md "$BUILD/docs/"
cp PLAN.md PROTOCOL.md LICENSE "$BUILD/"
cp node_modules/motion/LICENSE.md "$BUILD/docs/MOTION-LICENSE.md"
cp .env.example "$BUILD/.env.example"
sed 's|frontend/assets/|dist/assets/|g' docs/artwork.md > "$BUILD/docs/artwork.md"
# Prevent macOS from adding hidden AppleDouble files beside the executables.
COPYFILE_DISABLE=1 tar -czf "$STAGING/release.tar.gz" -C "$STAGING" "causewaybay-$PLATFORM"
mv "$STAGING/release.tar.gz" "package/causewaybay-$PLATFORM.tar.gz"
mkdir -p "$DEST"
rsync -a --delete "$BUILD/" "$DEST/"
node scripts/package-client.mjs "$STAGING/client" "$ROOT/package/client.zip"
(cd package && shasum -a 256 "causewaybay-$PLATFORM.tar.gz" > "causewaybay-$PLATFORM.tar.gz.sha256")
echo "Package: $ROOT/package/causewaybay-$PLATFORM.tar.gz"
echo "Binaries: $ROOT/$DEST/bin"
echo "Web assets: $ROOT/$DEST/dist"
