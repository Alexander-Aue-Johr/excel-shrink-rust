"""Assign exact XLSX filenames consistently across independent worker machines."""

import hashlib


def worker_for_name(name: str, worker_count: int) -> int:
    """SHA-256 of exact UTF-8 filename, first eight bytes as an unsigned big-endian integer."""
    if worker_count < 1:
        raise ValueError("Worker count must be positive")
    return int.from_bytes(hashlib.sha256(name.encode("utf-8")).digest()[:8], "big") % worker_count


def owned_names(names, args) -> list[str]:
    count = getattr(args, "worker_count", 1)
    worker_id = getattr(args, "worker_id", 0)
    return [name for name in names if worker_for_name(name, count) == worker_id]


def bind_worker(progress, args) -> None:
    """Preserve legacy results and prevent reuse of a partition's database by a different worker."""
    count = getattr(args, "worker_count", 1)
    worker_id = getattr(args, "worker_id", 0)
    assignment = f"sha256-64be-v1/{count}/{worker_id}"
    with progress.connection:
        previous = progress.connection.execute(
            "SELECT value FROM metadata WHERE key = 'worker_assignment'").fetchone()
        if previous and previous[0] not in {assignment, "sha256-64be-v1/1/0"}:
            raise ValueError("This progress database belongs to a different worker assignment. "
                             "Use a separate --output-root.")
        progress.connection.execute(
            "INSERT OR REPLACE INTO metadata (key, value) VALUES ('worker_assignment', ?)",
            (assignment,))
    print(f"Worker {worker_id} of {count}: SHA-256 filename partition; progress {progress.path}", flush=True)
