#!/usr/bin/env bash
set -euo pipefail
mode="${1:-ui}"
case "$mode" in ui|build|self-test) ;; *) echo "Invalid Excel Lab action: $mode" >&2; exit 2;; esac
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
repo="$(cd -- "$here/../.." && pwd -P)"
command -v powershell.exe >/dev/null || { echo "Windows powershell.exe is required for Excel COM." >&2; exit 1; }
control="$(mktemp -d)"
child=""
cleanup() { rm -f -- "$control/result.txt" "$control/cancel"; rmdir -- "$control" 2>/dev/null || true; }
cancel() { : > "$control/cancel"; echo '[excel-lab] Cancellation requested.' >&2; }
trap cancel INT TERM
trap cleanup EXIT
printf '[excel-lab] WSL launcher: %s action=%s\n' "${WSL_DISTRO_NAME:-unknown}" "$mode"
powershell.exe -NoLogo -NoProfile -STA -ExecutionPolicy Bypass \
    -File "$(wslpath -w "$here/run.ps1")" -Action "$mode" \
    -WslRepo "$repo" -WslDistro "${WSL_DISTRO_NAME:?WSL_DISTRO_NAME not set}" \
    -DeferDiff -ResultFile "$(wslpath -w "$control/result.txt")" \
    -CancelFile "$(wslpath -w "$control/cancel")" &
child=$!
status=0
while true; do
    wait "$child" && { status=0; break; } || status=$?
    # An interrupted wait does not mean the supervised process has exited.
    kill -0 "$child" 2>/dev/null || break
done
if ((status != 0)); then exit "$status"; fi
if [[ -s "$control/result.txt" ]]; then
    mapfile -t paths < "$control/result.txt"
    ((${#paths[@]} == 2)) || { echo 'Invalid diff marker.' >&2; exit 1; }
    left="$(wslpath -u "${paths[0]%$'\r'}")"
    right="$(wslpath -u "${paths[1]%$'\r'}")"
    command -v code >/dev/null || { printf 'VS Code CLI not found. Compare:\n%s\n%s\n' "$left" "$right" >&2; exit 1; }
    code --reuse-window --diff "$left" "$right"
fi
