#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if [[ -n "${MOBIUS_REVIEW_UV:-}" ]]; then
  UV="$MOBIUS_REVIEW_UV"
elif command -v uv >/dev/null 2>&1; then
  UV="$(command -v uv)"
else
  echo "mobius-review startup failed: uv not found; set MOBIUS_REVIEW_UV or install uv" >&2
  exit 127
fi

if [[ -z "${UV_PROJECT_ENVIRONMENT:-}" ]]; then
  if [[ -n "${PLUGIN_DATA:-}" ]]; then
    export UV_PROJECT_ENVIRONMENT="${PLUGIN_DATA}/uv-venv"
  else
    export UV_PROJECT_ENVIRONMENT="${XDG_CACHE_HOME:-${HOME:-/tmp}/.cache}/mobius-review/uv-venv"
  fi
fi

export PYTHONDONTWRITEBYTECODE="${PYTHONDONTWRITEBYTECODE:-1}"

if [[ "${1:-}" == "--self-check" ]]; then
  exec "$UV" run --project "$ROOT" python -c 'import pathlib, sys; sys.path.insert(0, str(pathlib.Path("'"$ROOT"'", "scripts"))); import mcp; import mobius; print("mobius-review-launcher-ok")'
fi

cd "$ROOT"
exec "$UV" run --project "$ROOT" python "$ROOT/scripts/mobius_review_mcp.py"
