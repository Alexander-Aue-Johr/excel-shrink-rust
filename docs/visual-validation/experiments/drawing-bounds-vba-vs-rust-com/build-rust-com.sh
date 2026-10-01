#!/usr/bin/env bash
set -euo pipefail

target="x86_64-pc-windows-gnu"
root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
project="$root/rust-com"
release_dir="$project/target/$target/release"
fingerprint_file="$release_dir/source.sha256"

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

export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc

cargo build \
  --locked \
  --manifest-path "$project/Cargo.toml" \
  --release \
  --target "$target"

mkdir -p "$release_dir"

sha256_file() {
  sha256sum "$1" | awk '{print tolower($1)}'
}

{
  printf 'Cargo.toml=%s\n' "$(sha256_file "$project/Cargo.toml")"
  printf 'Cargo.lock=%s\n' "$(sha256_file "$project/Cargo.lock")"
  printf 'src/main.rs=%s\n' "$(sha256_file "$project/src/main.rs")"
} > "$fingerprint_file"

echo "Built:"
echo "  $release_dir/drawing-bounds-rust-com.exe"
echo "Source fingerprint:"
echo "  $fingerprint_file"
