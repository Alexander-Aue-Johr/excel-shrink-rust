#!/usr/bin/env bash
# Managed by patch_excel_json_diff.py. Launch only; no spreadsheet processing here.
set -Eeuo pipefail
here="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo="$(cd -- "$here/../.." && pwd)"
: "${WSL_DISTRO_NAME:?Run this launcher inside WSL (WSL_DISTRO_NAME is missing).}"
command -v wslpath >/dev/null || { echo 'ERROR: wslpath is unavailable.' >&2; exit 1; }
ps="$(command -v powershell.exe || true)"
if [[ -z "$ps" && -x /mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe ]]; then
  ps=/mnt/c/Windows/System32/WindowsPowerShell/v1.0/powershell.exe
fi
[[ -n "$ps" ]] || { echo 'ERROR: Windows powershell.exe is unavailable; check WSL interoperability and PATH.' >&2; exit 1; }
mkdir -p -- "$repo/.tmp/excel-json-diff-launch"
launch="$(mktemp -d "$repo/.tmp/excel-json-diff-launch/launch-XXXXXXXX")"
result="$launch/result.txt"
cancel="$launch/cancel"
child=''
cancel_run() {
  printf 'cancel\n' > "$cancel"
  echo '[json-diff] Cancellation requested; the supervisor will clean up its own Excel instance.' >&2
}
trap cancel_run INT TERM
printf '[json-diff] WSL launcher: %s\n' "$WSL_DISTRO_NAME"
"$ps" -NoLogo -NoProfile -STA -ExecutionPolicy Bypass -File "$(wslpath -w "$here/json_diff.ps1")" \
  -LinuxRepoRoot "$repo" -Distro "$WSL_DISTRO_NAME" -DeferDiff \
  -ResultFile "$(wslpath -w "$result")" -CancelFile "$(wslpath -w "$cancel")" "$@" &
child=$!
set +e
wait "$child"
status=$?
if [[ -f "$cancel" ]]; then
  wait "$child" 2>/dev/null
  status=130
fi
set -e
trap - INT TERM
[[ $status -eq 0 ]] || exit "$status"
# Cancelled picker and -NoDiff intentionally produce no diff request.
if [[ -f "$result" ]]; then
  mapfile -t paths < "$result"
  [[ ${#paths[@]} -eq 2 ]] || { echo 'ERROR: invalid diff request file.' >&2; exit 1; }
  original="$(wslpath -u "${paths[0]%$'\r'}")"
  shrunk="$(wslpath -u "${paths[1]%$'\r'}")"
  printf '[json-diff] VS Code diff:\n  %s\n  %s\n' "$original" "$shrunk"
  command -v code >/dev/null || { echo 'ERROR: code CLI not found in this WSL session. The JSON files are retained.' >&2; exit 1; }
  # Keep the original Remote-WSL VS Code IPC environment; do not round-trip it via wsl.exe.
  code --reuse-window --diff "$original" "$shrunk"
fi
