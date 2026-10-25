# Excel diagnostic log ownership

The corpus launcher periodically cleans `%LOCALAPPDATA%\Temp\Diagnostics\EXCEL`.
Age alone never permits deletion. A log must be exclusively open throughout
inspection and deletion, and its known owner must have exited. This also protects
live loggers that allow other programs to delete their open files.

The header identifies the writer; observed file users provide additional protection:

- A bounded read of the Excel log header supplies `EXCEL (0xPID)` and a UTC
  timestamp. A missing PID proves that writer has exited. If the PID now belongs
  to a process born more than five seconds after both the header and the file's
  creation time, the older writer has exited. A live or ambiguous PID is retained.
  Header and file creation times must agree within a minute.
- Blank preallocated logs have no header. Windows Restart Manager can identify
  their live file users by PID and process creation time. The cleanup helper
  records those identities beside its status file in `*.owners.json`, together
  with the volume, file ID and file creation time. These users may be readers,
  so their exit does not establish that the writer exited. Headerless logs remain
  untouched even when all observed users have exited. For logs with a header,
  both the identified writer and recorded users must have exited before deletion.
  Replacing a path invalidates recorded observations. Restart Manager is queried
  only; cleanup never requests shutdown or restart.

Opening the same file without sharing prevents a new reader or writer from
acquiring it between verification and deletion. File deletion is set on that
handle; it is not a separate path-based `Remove-Item` operation. Paths outside the
diagnostic root, reparse points, unreadable files, ambiguous timestamps and logs
without ownership evidence are retained. In particular, old blank logs cannot be
assigned to a process using the GUID in their filename alone.

The status JSON reports retained live, unknown and locked bytes, cleanup outcomes,
ownership-query counts and free disk space. At most 32 previously unknown logs
are queried per pass, with newly created logs considered first. Historical unknown
logs may remain; a low-space warning must not be interpreted as permission to
delete them.

On native Windows, the interactive scheduler samples free space on the Excel
diagnostic volume at most once per second. Below 2 GiB, or when the query fails,
it stops admitting new groups while existing work finishes. It preserves the
user's group target and resumes admission automatically when the query reports
enough space. `queue.admission_guard` and the Observatory show the reason and
available space. WSL does not infer Windows free space from Linux counters.

Run `powershell.exe -NoProfile -ExecutionPolicy Bypass -File
tools/corpus-launcher/diagnostics-cleanup.tests.ps1` for the native ownership tests.
They cover live/dead/reused PIDs, delete-sharing handles, persistent file-user
observations, a reader exiting while the writer remains alive, blank logs that
remain unassigned, and replacement of the same path.
