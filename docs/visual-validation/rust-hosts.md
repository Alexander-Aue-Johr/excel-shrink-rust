# Rust capture hosts

The Windows `excel-shrink-cli.exe` includes three internal host modes:

| Mode | Responsibility | Source |
| --- | --- | --- |
| `capture-host classic` | One capture or an acknowledged original/variant image stream | `src/capture_host/classic.rs`, `paired.rs` |
| `capture-host resident <settings.json>` | Persistent Office.js sessions, jobs, image receipts and worker telemetry | `src/capture_host/resident.rs`, `resident_telemetry.rs` |
| `capture-host control-room <settings.json>` | Observatory assets, state queries and scoped controls | `src/capture_host/control_room.rs` |

Source paths in the table are relative to `excel-shrink-cli`. These are internal
launcher commands, not additional corpus parameters. The normal WPF task and
corpus commands select the appropriate host automatically.

## Deployment and ownership

There is no .NET SDK discovery, restore, host compilation at run time or separately
installed .NET runtime. The existing Rust release build produces the hosts too.
Keep the repository assets and scripts with the deployed build as before.
PowerShell still uses Windows' built-in .NET Framework for WPF, COM conversion
and saved recalculation, and the existing Win32 process helpers.

The runner passes its executable explicitly to the capture launcher. Direct
WSL capture callers can select a native companion with
`EXCEL_SHRINK_CAPTURE_HOST`; the path is passed to PowerShell as an argument.
The Observatory uses a versioned copy of the runner executable so an idle
historical view does not lock the next release build. Its version is the SHA-256
of the executable that actually runs, rather than a hash of source files.

Excel job ownership, private desktops, memory limits and bounded cleanup stay
with their existing launcher. Every host remains a separate process. Both
loopback address families are served: `localhost` must not incur IPv6 connection
fallback delays on every worker poll.

## Evidence and compatibility

HTTP routes and persisted result formats are preserved. Existing records,
prepared bundles, exports and imports retain their meaning. A changed runner
hash still requires the normal reviewed preparation compatibility setting;
the migration does not automatically approve arbitrary previous binaries.

Uploads are bounded and serialized per resident client. A slow client does not
hold the group dispatch gate. Session tokens are checked again at the mutation
boundary, so a reset cannot attach an old upload to a replacement pane. Heartbeat
activity and successful work remain separate observations.

Classic image pairs retain consumer acknowledgments and durable publication.
All hosts publish complete files atomically; a duplicate receipt cannot replace
existing evidence. Invalid duplicate geometry is rejected explicitly, and a
duplicate classic upload returns a structured error instead of an HTTP 500.

## Verification

The HTTP suites exercise the actual executable. Their optional DLL input is
retained only as a development oracle for comparing an older C# build.

```powershell
cargo test -p excel-shrink-cli --bin excel-shrink-cli capture_host::
python excel-shrink-cli/office-capture/resident-host/host_protocol_test.py --host target/release/excel-shrink-cli.exe
python excel-shrink-cli/office-capture/resident-host/control_room_test.py --host target/release/excel-shrink-cli.exe
python excel-shrink-cli/office-capture/tests/classic_host_protocol_test.py --host target/release/excel-shrink-cli.exe
```

Opt-in native tests additionally cover peer survival after cleanup and the full
study pipeline, including tall-sheet selection and byte-identical export/import:

```powershell
python excel-shrink-cli/office-capture/tests/cleanup-peer-smoke.py --run --groups 2 --clients 1 --reset-first --image-batches 3
python excel-shrink-cli/office-capture/tests/tall-sheet-smoke.py --run --hide-excel --binary target/release/excel-shrink-cli.exe
```

The readable Rust implementation is longer than the compact C# sources it
replaces. The intended reduction is in deployment paths and languages involved
in orchestration, not source line count. No HTTP parsing or async runtime is
implemented locally; the hosts use Axum and Tokio.
