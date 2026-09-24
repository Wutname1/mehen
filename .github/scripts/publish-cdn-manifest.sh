#!/usr/bin/env bash
# Publish the updater manifest the app reads to the CDN.
#
# The app checks https://cdn.gitwyrm.com/mehen/updates/stable.json, a plain R2
# object served straight through Cloudflare, so an update check is one cached
# GET and costs nothing no matter how many copies of Mehen are running.
#
# The manifest is copied from the GitHub release after resolve-updater-urls.sh
# has pointed it at the public releases/download URLs and verified them. The
# signatures are the ones minisign made at build time, and the installer itself
# is served by GitHub, not stored on R2.
#
# Usage: publish-cdn-manifest.sh <tag>
# Env:   GH_REPO, GH_TOKEN, R2_ENDPOINT, AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY
set -euo pipefail

TAG="${1:?usage: publish-cdn-manifest.sh <tag>}"
REPO="${GH_REPO:?GH_REPO must be set}"
R2_ENDPOINT="${R2_ENDPOINT:?R2_ENDPOINT must be set}"
BUCKET="${R2_BUCKET:-gitwyrm-cdn}"
KEY="mehen/updates/stable.json"

workdir=$(mktemp -d)
trap 'rm -rf "$workdir"' EXIT

# Signatures cannot be regenerated here - the private key only exists in the
# build job - so the manifest is copied, never rebuilt.
gh release download "$TAG" --repo "$REPO" --pattern latest.json --dir "$workdir"

python3 - "$workdir/latest.json" "$REPO" "$TAG" <<'PY'
import json, sys

path, repo, tag = sys.argv[1], sys.argv[2], sys.argv[3]
with open(path, encoding="utf-8") as fh:
    manifest = json.load(fh)

platforms = manifest.get("platforms", {})
if not platforms:
    sys.exit("::error::manifest has no platforms")

# tauri emits both "windows-x86_64" and "windows-x86_64-nsis" for the same
# installer; older updater clients look up the first, newer ones the second.
if "windows-x86_64" not in platforms and "windows-x86_64-nsis" not in platforms:
    sys.exit(f"::error::manifest has no Windows x64 entry. Present: {', '.join(sorted(platforms))}")

# The api.github.com asset URLs tauri-action writes serve JSON metadata, not the
# installer, and the updater would hand that to NSIS. resolve-updater-urls.sh
# rewrites them first; this refuses to publish if that ever did not happen.
public_base = f"https://github.com/{repo}/releases/download/{tag}/"
for key, entry in platforms.items():
    if not entry.get("signature"):
        sys.exit(f"::error::platform '{key}' has no signature")
    if not entry.get("url", "").startswith(public_base):
        sys.exit(f"::error::platform '{key}' url {entry.get('url')!r} is not a public download of {tag}")

# The updater compares this against the running build, so it must be the plain
# version.
if manifest.get("version") != tag:
    print(f"::warning::manifest version {manifest.get('version')!r} != tag {tag!r}; using tag")
    manifest["version"] = tag

with open(path, "w", encoding="utf-8") as fh:
    json.dump(manifest, fh, indent=2)

for key, val in platforms.items():
    print(f"  {key} -> {val['url']}")
PY

# no-store: this is the pointer every copy of Mehen polls. Cached at the edge it
# would keep offering the previous version until the TTL ran out.
aws s3 cp "$workdir/latest.json" "s3://${BUCKET}/${KEY}" \
  --endpoint-url "$R2_ENDPOINT" \
  --content-type "application/json" \
  --cache-control "no-cache, no-store, must-revalidate"

echo "Published ${KEY} -> ${TAG}"

# An installer URL that 404s breaks every update, and it is cheap to rule out
# here rather than hear about from a user.
for url in $(python3 -c "
import json
m = json.load(open('$workdir/latest.json', encoding='utf-8'))
for u in sorted({p['url'] for p in m['platforms'].values()}):
    print(u)
"); do
  code=$(curl -s -o /dev/null -w '%{http_code}' -I -L "$url" || echo 000)
  if [ "$code" != "200" ]; then
    echo "::error::${url} returned HTTP ${code} - Mehen could not update from it."
    exit 1
  fi
  echo "  OK ${url}"
done
