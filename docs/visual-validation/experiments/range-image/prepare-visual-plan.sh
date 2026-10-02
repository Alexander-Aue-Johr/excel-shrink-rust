#!/usr/bin/env bash
set -euo pipefail

export PATH="$HOME/.cargo/bin:$PATH"

if [[ $# -ne 2 ]]; then
  echo "Usage: prepare-visual-plan.sh FILE_LIST OUTPUT_JSON" >&2
  exit 2
fi

script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_root/../../../../" && pwd)"
manifest="$script_root/rust-ooxml-plan/Cargo.toml"
target_dir="$repo_root/.tmp/visual-validation/rust-ooxml-plan-target"
exe="$target_dir/release/visual-validation-ooxml-plan"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is not on PATH inside WSL." >&2
  exit 1
fi

mkdir -p "$target_dir"
export CARGO_TARGET_DIR="$target_dir"

cargo build --quiet --release --manifest-path "$manifest"

"$exe" \
  --file-list "$1" \
  --output "$2"
