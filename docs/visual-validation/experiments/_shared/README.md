# Shared visual-validation experiment infrastructure

This directory is intentionally shared **only by the experiments below
`docs/visual-validation/experiments/`**. Production code in `src/`, `crates/`, and
`tools/` must not depend on it.

The duplication boundary is deliberate: after an experiment chooses an implementation,
the production version is copied/reimplemented under the normal product tree. This
keeps historical experiments reproducible even when production tooling later changes.

## Office add-in

`office-addin/web/` contains the single task-pane implementation used by both host
variants. `office-addin/hosts/dotnet/` and `office-addin/hosts/rust/` implement the
same localhost protocol.

The Rust host is cross-compiled from WSL to `x86_64-pc-windows-gnu`, cached under
`.tmp/visual-validation/`, and copied to the current user's local application data.
Its experiment-only dependencies are declared by the Cargo manifest in this directory
and do not participate in the repository root Cargo workspace.

The ASP.NET Core host has its own `.csproj`; a Windows .NET 8 SDK is needed only when
that variant is selected.
