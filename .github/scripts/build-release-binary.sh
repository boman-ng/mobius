#!/usr/bin/env bash
set -euo pipefail

if [ "$#" -ne 1 ]; then
  echo "usage: build-release-binary.sh <target>" >&2
  exit 64
fi

target=$1
if [ "$target" != "x86_64-unknown-linux-gnu" ]; then
  echo "unsupported release target: $target" >&2
  exit 64
fi

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
repo_root=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)

if [ -z "${HOME:-}" ] || [ ! -d "$HOME" ]; then
  echo "HOME must name an existing directory for release path remapping" >&2
  exit 64
fi
home_root=$(CDPATH= cd -- "$HOME" && pwd -P)

rust_release=$(rustc --version --verbose | sed -n 's/^release: //p')
if [ "$rust_release" != "1.85.0" ]; then
  echo "release builds require rustc 1.85.0, found $rust_release" >&2
  exit 69
fi

# CARGO_ENCODED_RUSTFLAGS preserves path arguments even when the checkout contains spaces. Both
# the checkout and toolchain/dependency cache under HOME are mapped to stable non-host paths.
unit_separator=$'\x1f'
encoded_rustflags="--remap-path-prefix=$repo_root=/mobius/source"
encoded_rustflags+="$unit_separator--remap-path-prefix=$home_root=/mobius/build-cache"

# libsqlite3-sys compiles the bundled SQLite source through cc. Give that compiler the same path
# policy for __FILE__, debug data, and macro expansions. printf %q produces words understood by
# the cc crate's shell-style flag parser when a source path contains spaces.
printf -v repo_file_map '%q' "-ffile-prefix-map=$repo_root=/mobius/source"
printf -v repo_debug_map '%q' "-fdebug-prefix-map=$repo_root=/mobius/source"
printf -v repo_macro_map '%q' "-fmacro-prefix-map=$repo_root=/mobius/source"
printf -v home_file_map '%q' "-ffile-prefix-map=$home_root=/mobius/build-cache"
printf -v home_debug_map '%q' "-fdebug-prefix-map=$home_root=/mobius/build-cache"
printf -v home_macro_map '%q' "-fmacro-prefix-map=$home_root=/mobius/build-cache"
cflags="$repo_file_map $repo_debug_map $repo_macro_map $home_file_map $home_debug_map $home_macro_map"
target_cflags="CFLAGS_${target//-/_}"

export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$repo_root/.tmp/cargo-target}"
env \
  -u RUSTFLAGS \
  CARGO_ENCODED_RUSTFLAGS="$encoded_rustflags" \
  CFLAGS="$cflags" \
  "$target_cflags=$cflags" \
  cargo build \
    --manifest-path "$repo_root/plugins/mobius/runtime/Cargo.toml" \
    --locked \
    --release \
    --target "$target" \
    --bin mobius
