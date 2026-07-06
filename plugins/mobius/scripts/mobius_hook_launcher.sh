#!/usr/bin/env sh
set -eu

root="${1-}"
if [ -z "$root" ]; then
  printf "%s\n" "mobius:hook-misconfigured: PLUGIN_ROOT missing" >&2
  exit 2
fi
shift

script="$root/scripts/mobius.py"
if [ ! -f "$script" ]; then
  if [ ! -d "$root" ]; then
    case "$root" in
      */.codex/plugins/cache/*/mobius/*)
        printf "%s\n" "mobius:hook-unavailable: installed plugin cache missing" >&2
        exit 0
        ;;
    esac
    printf "%s\n" "mobius:hook-misconfigured: PLUGIN_ROOT path missing" >&2
    exit 2
  fi
  printf "%s\n" "mobius:hook-corrupt-install: scripts/mobius.py missing" >&2
  exit 2
fi

if ! command -v python3 >/dev/null 2>&1; then
  printf "%s\n" "mobius:hook-runtime-missing: python3" >&2
  exit 2
fi

exec python3 "$script" "$@"
