#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 3 ]; then
  echo "usage: assemble-release-bundle.sh <target> <release-binary> <bundle-root>" >&2
  exit 64
fi

target=$1
release_binary=$2
bundle_root=$3

if [ "$target" != "x86_64-unknown-linux-gnu" ]; then
  echo "unsupported release target: $target" >&2
  exit 64
fi

if [ ! -f "$release_binary" ] || [ ! -x "$release_binary" ]; then
  echo "release binary must be an executable regular file: $release_binary" >&2
  exit 66
fi

if [ -e "$bundle_root" ]; then
  echo "bundle root already exists: $bundle_root" >&2
  exit 73
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd)
plugin_source="$repo_root/plugins/mobius"
plugin_bundle="$bundle_root/plugins/mobius"

install -d \
  "$bundle_root/.agents/plugins" \
  "$plugin_bundle/.codex-plugin" \
  "$plugin_bundle/bin" \
  "$plugin_bundle/hooks" \
  "$plugin_bundle/skills"

install -m 0644 "$repo_root/LICENSE" "$bundle_root/LICENSE"
install -m 0644 \
  "$plugin_source/.codex-plugin/plugin.json" \
  "$plugin_bundle/.codex-plugin/plugin.json"
install -m 0644 "$plugin_source/.mcp.json" "$plugin_bundle/.mcp.json"
install -m 0644 "$plugin_source/hooks/hooks.json" "$plugin_bundle/hooks/hooks.json"
cp -R "$plugin_source/skills/." "$plugin_bundle/skills/"
install -m 0755 "$release_binary" "$plugin_bundle/bin/mobius"

# The checked-in source catalog stays unavailable. Availability is promoted only in this
# assembled, target-specific copy after the source and runtime gates have passed.
jq '
  if ([.plugins[] | select(.name == "mobius")] | length) != 1 then
    error("marketplace must contain exactly one mobius entry")
  else
    (.plugins[] | select(.name == "mobius") | .policy.installation) = "AVAILABLE"
  end
' "$repo_root/.agents/plugins/marketplace.json" \
  > "$bundle_root/.agents/plugins/marketplace.json"
chmod 0644 "$bundle_root/.agents/plugins/marketplace.json"

find "$plugin_bundle" -type d -exec chmod 0755 {} +
find "$plugin_bundle" -type f ! -path "$plugin_bundle/bin/mobius" -exec chmod 0644 {} +

(
  cd "$bundle_root"
  find . -type f ! -name SHA256SUMS -print0 \
    | sort -z \
    | xargs -0 sha256sum
) > "$bundle_root/SHA256SUMS"
chmod 0644 "$bundle_root/SHA256SUMS"
