#!/usr/bin/env bash
set -euo pipefail

repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)
powershell_path=$(wslpath -u 'C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe')
script_path=$(wslpath -w "$repo_root/tools/start-sheet-scope.ps1")
source_path=$(wslpath -w "$repo_root")
binary_path=$(wslpath -w "$repo_root/target/x86_64-pc-windows-gnu/release/sheet-scope.exe")
shrink_path=$(wslpath -w "$repo_root/target/x86_64-pc-windows-gnu/release/excel-shrink-cli.exe")

exec "$powershell_path" -NoLogo -NoProfile -ExecutionPolicy Bypass -File "$script_path" \
    -SourceRepo "$source_path" -BinaryPath "$binary_path" -ShrinkBinaryPath "$shrink_path" "$@"
