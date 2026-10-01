#!/usr/bin/env bash
set -euo pipefail

# rustup installs Cargo here for a normal per-user WSL setup. Do not rely on
# the caller being a login shell: this script is also invoked through wsl.exe
# from Windows PowerShell.
export PATH="$HOME/.cargo/bin:$PATH"

target="x86_64-pc-windows-gnu"
script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd "$script_root/../../../../.." && pwd)"
project="$script_root/hosts/rust"
target_dir="$repo_root/.tmp/visual-validation/experiment-rust-office-host-target"
release_dir="$target_dir/$target/release"
exe="$release_dir/visual-validation-office-host.exe"
fingerprint="$target_dir/source.sha256"

if ! command -v cargo >/dev/null 2>&1; then
  echo "cargo is not on PATH." >&2
  exit 1
fi

if ! command -v x86_64-w64-mingw32-gcc >/dev/null 2>&1; then
  echo "Missing MinGW-w64 cross compiler." >&2
  echo "Install in WSL with: sudo apt install mingw-w64" >&2
  exit 1
fi

if ! rustup target list --installed | grep -qx "$target"; then
  echo "Missing Rust target $target." >&2
  echo "Install with: rustup target add $target" >&2
  exit 1
fi

compute_fingerprint() {
  {
    sha256sum "$project/Cargo.toml"
    if [[ -f "$project/Cargo.lock" ]]; then
      sha256sum "$project/Cargo.lock"
    fi
    find "$project/src" -type f -name '*.rs' -print0 \
      | sort -z \
      | xargs -0 sha256sum
  } | sha256sum | awk '{print tolower($1)}'
}

expected="$(compute_fingerprint)"
actual=""
if [[ -f "$fingerprint" ]]; then
  actual="$(tr -d '\r\n' < "$fingerprint")"
fi

if [[ -x "$exe" && "$expected" == "$actual" ]]; then
  echo "REUSE $exe"
  exit 0
fi

mkdir -p "$target_dir"
export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
export CARGO_TARGET_DIR="$target_dir"

echo "BUILD $project -> $exe"
cargo build \
  --manifest-path "$project/Cargo.toml" \
  --release \
  --target "$target"

compute_fingerprint > "$fingerprint"
echo "BUILT $exe"
