"""Prepare native Excel artifacts with bounded private-desktop workers."""
import argparse
import concurrent.futures
import ctypes
import datetime as dt
import hashlib
import json
import os
from pathlib import Path
import shutil
import statistics
import subprocess
import time
import uuid
import zipfile


class MemoryStatus(ctypes.Structure):
    _fields_ = [("length", ctypes.c_ulong), ("load", ctypes.c_ulong)] + [
        (name, ctypes.c_ulonglong) for name in
        ("total", "available", "page_total", "page_available", "virtual_total", "virtual_available", "extended")]


def available_memory():
    status = MemoryStatus()
    status.length = ctypes.sizeof(status)
    if not ctypes.windll.kernel32.GlobalMemoryStatusEx(ctypes.byref(status)):
        raise ctypes.WinError()
    return status.available


def write_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(path.name + ".pending")
    pending.write_text(json.dumps(value, indent=2) + "\n", encoding="utf8")
    for retry in range(100):
        try:
            os.replace(pending, path)
            return
        except PermissionError:
            if retry == 99:
                raise
            time.sleep(.05)


def checksum(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def execute(command, log, timeout, environment, cwd):
    with log.open("wb") as stream:
        result = subprocess.run(command, stdout=stream, stderr=subprocess.STDOUT,
                                timeout=timeout, env=environment, cwd=cwd,
                                creationflags=subprocess.CREATE_NO_WINDOW)
    if result.returncode:
        raise RuntimeError(f"Command exited with {result.returncode}; see {log}")


def prepare_one(case, root, config, phase, operation_timeout):
    started = time.monotonic()
    attempt = Path(config["working_root"]) / phase / case["id"] / uuid.uuid4().hex[:8]
    attempt.mkdir(parents=True)
    inputs = attempt / "inputs"
    inputs.mkdir()
    temporary = Path(config["working_root"]) / "temp" / attempt.name
    temporary.mkdir(parents=True)
    source = inputs / "original.xlsx"
    result = {"id": case["id"], "bundle": case["bundle"], "phase": phase,
              "attempt": str(attempt), "status": "failed"}
    environment = dict(os.environ, TEMP=str(temporary), TMP=str(temporary),
                       EXCEL_SHRINK_SYSTEM_RESERVE_MIB="2048",
                       EXCEL_SHRINK_PREPARATION_START_SLOTS="2",
                       EXCEL_SHRINK_CHECK_PERFORMANCE_CACHE_DIR=config["cp_cache"],
                       EXCEL_SHRINK_CONTROL_ARTIFACT_CACHE_DIR=config["control_cache"])
    try:
        if shutil.disk_usage(root).free < 20 * 1024**3:
            raise RuntimeError("Data drive reserve reached")
        if shutil.disk_usage(config["working_root"]).free < 6 * 1024**3:
            raise RuntimeError("Temporary workspace reserve reached")
        copy_started = time.monotonic()
        with zipfile.ZipFile(case["bundle"]) as archive:
            with archive.open("original.xlsx") as incoming, source.open("wb") as outgoing:
                shutil.copyfileobj(incoming, outgoing, 1024 * 1024)
        result["input_copy_seconds"] = time.monotonic() - copy_started
        record_path = Path(config["records"]) / (case["id"] + ".json")
        record = json.loads(record_path.read_bytes())
        case["label"] = record["member_name"]
        expected = record.get("baseline_sha256") or record.get("converted_sha256")
        actual = checksum(source)
        if expected != actual:
            raise ValueError("Prepared Original checksum differs from its preparation record")
        result.update(original_sha256=actual, source_member=record["member_name"],
                      prepared_source_sha256=record.get("source_sha256"))
        shrink_started = time.monotonic()
        default = inputs / "default" / "original.xlsx"
        result["default_source"] = config.get("default_source", "current_binary")
        if result["default_source"] == "prepared":
            default.parent.mkdir()
            with zipfile.ZipFile(case["bundle"]) as archive:
                with archive.open("default.xlsx") as incoming, default.open("wb") as outgoing:
                    shutil.copyfileobj(incoming, outgoing, 1024 * 1024)
            result["default_copy_seconds"] = time.monotonic() - shrink_started
            result["default_producer_sha256"] = record.get("prepared_by")
            result["historical_default_preparation"] = record.get("provenance", {}).get("preparation")
            result["historical_default_shrink_seconds"] = next(
                (row.get("shrink_seconds") for row in record.get("results", [])
                 if row.get("profile") == "default"), None)
        elif result["default_source"] == "current_binary":
            execute([config["binary"], str(source), str(inputs / "default"), "--rayon-threads", "2"],
                    attempt / "shrink.log", 300, environment, attempt)
            result["shrink_seconds"] = time.monotonic() - shrink_started
        else:
            raise ValueError("Unknown Default source " + result["default_source"])
        if not default.is_file():
            raise RuntimeError("Default shrink output is missing")
        result["default_sha256"] = checksum(default)
        group = attempt / "control/groups/one"
        group.mkdir(parents=True)
        command = ["powershell.exe", "-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File",
                   config["launcher"], "-Binary", config["binary"], "-Original", str(source),
                   "-Default", str(default), "-Output", str(attempt / "artifacts"),
                   "-ObservatoryGroup", str(group), "-PrepareArtifactsOnly",
                   "-OperationTimeoutSeconds", str(operation_timeout)]
        execute(command, attempt / "excel.log", config["workbook_timeout_seconds"], environment, attempt)
        preparation = json.loads((attempt / "artifacts/preparation.json").read_bytes())
        if preparation["status"] != "artifacts_prepared":
            raise RuntimeError("Artifact generation did not complete")
        artifacts = []
        for path in sorted((attempt / "artifacts").glob("*.xlsx")):
            artifacts.append({"file": str(path), "sha256": checksum(path), "bytes": path.stat().st_size})
        result.update(status="prepared", artifacts=artifacts,
                      default_state=preparation["default_state"],
                      preparation_report=str(attempt / "artifacts/preparation.json"),
                      operations_report=str(attempt / "artifacts/operations.json"),
                      visual_validation="pending")
    except Exception as error:
        result["error"] = type(error).__name__ + ": " + str(error)
    result["elapsed_seconds"] = time.monotonic() - started
    result["finished_utc"] = dt.datetime.now(dt.timezone.utc).isoformat()
    write_json(attempt / "result.json", result)
    retained = root / phase / case["id"] / attempt.name
    retained.mkdir(parents=True, exist_ok=True)
    for path in attempt.glob("*.log"):
        shutil.copy2(path, retained / path.name)
    shutil.copytree(inputs, retained / "inputs", dirs_exist_ok=True)
    artifact_names = {
        "original.xlsx":"original.saved.xlsx", "default.xlsx":"default.saved.xlsx",
        "OR-rebuilt.xlsx":"original.rebuilt.saved.xlsx",
        "OR-rebuilt.before-rebuild.xlsx":"original.rebuild-copy.saved.xlsx",
        "SDR-rebuilt.xlsx":"default.rebuilt.saved.xlsx",
        "SDR-rebuilt.before-rebuild.xlsx":"default.rebuild-copy.saved.xlsx",
        "CPR-rebuilt.xlsx":"check-performance.optimized.rebuilt.saved.xlsx",
        "CPR-rebuilt.before-rebuild.xlsx":"check-performance.rebuild-copy.saved.xlsx",
        "check-performance.after-optimization.xlsx":"check-performance.optimized.saved.xlsx",
    }
    if result.get("default_state") == "skipped_identical_input":
        artifact_names["default.xlsx"] = "default.identical-input.unopened.xlsx"
    if (attempt / "artifacts").exists():
        (retained / "artifacts").mkdir(exist_ok=True)
        for path in (attempt / "artifacts").iterdir():
            if path.is_file() and path.suffix in (".json", ".xlsx"):
                name = artifact_names.get(path.name, path.name)
                shutil.copy2(path, retained / "artifacts" / name)
                if checksum(path) != checksum(retained / "artifacts" / name):
                    raise RuntimeError("Retained artifact checksum differs")
        if result["status"] != "prepared":
            for path in (attempt / "artifacts").rglob("*"):
                if not path.is_file() or (path.suffix != ".log" and path.name not in {
                        "launcher-error.json", "launcher-result.json", "metrics.json"}):
                    continue
                destination = retained / "artifacts" / path.relative_to(attempt / "artifacts")
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(path, destination)
    result["execution_directory"] = str(attempt)
    result["attempt"] = str(retained)
    for artifact in result.get("artifacts", []):
        name = Path(artifact["file"]).name
        artifact["file"] = str(retained / "artifacts" / artifact_names.get(name, name))
    if result["status"] == "prepared":
        result["preparation_report"] = str(retained / "artifacts/preparation.json")
        result["operations_report"] = str(retained / "artifacts/operations.json")
    write_json(retained / "result.json", result)
    # Only this UUID-named, completed private attempt is disposable.
    scratch = Path(config["working_root"]).resolve()
    relative = attempt.resolve().relative_to(scratch)
    if len(relative.parts) != 3:
        raise ValueError("Unexpected scratch path")
    if temporary.resolve().parent != (scratch / "temp").resolve():
        raise ValueError("Unexpected temporary path")
    for directory in (attempt, temporary):
        try:
            shutil.rmtree(directory)
        except OSError as error:
            result["scratch_cleanup_warning"] = str(error)
    return result


def run_batch(cases, root, config, phase, parallelism, timeout):
    pending = iter(cases)
    active = {}
    results = []
    started = time.monotonic()
    deadline = dt.datetime.fromisoformat(config["deadline"])
    with concurrent.futures.ThreadPoolExecutor(max_workers=parallelism) as executor:
        exhausted = False
        while active or not exhausted:
            while len(active) < parallelism and not exhausted:
                if (root / "stop.request").exists() or dt.datetime.now(dt.timezone.utc) >= deadline:
                    exhausted = True
                    break
                if (available_memory() < 3 * 1024**3
                        or shutil.disk_usage(config["working_root"]).free < 6 * 1024**3
                        or shutil.disk_usage(root).free < 20 * 1024**3):
                    break
                try:
                    case = next(pending)
                except StopIteration:
                    exhausted = True
                    break
                future = executor.submit(prepare_one, case, root, config, phase, timeout)
                active[future] = case
            if not active:
                if exhausted:
                    break
                write_json(root / "status.json", {"state":"waiting_for_resources", "phase":phase,
                    "available_memory_bytes":available_memory(), "deadline":config["deadline"],
                    "free_workspace_bytes":shutil.disk_usage(config["working_root"]).free,
                    "free_output_bytes":shutil.disk_usage(root).free,
                    "updated_utc":dt.datetime.now(dt.timezone.utc).isoformat()})
                time.sleep(2)
                continue
            completed, _ = concurrent.futures.wait(active, timeout=10,
                                                   return_when=concurrent.futures.FIRST_COMPLETED)
            for future in completed:
                case = active.pop(future)
                try:
                    result = future.result()
                except Exception as error:
                    result = {"id": case["id"], "status": "failed", "elapsed_seconds": None,
                              "error": type(error).__name__ + ": " + str(error)}
                results.append(result if phase not in ("main", "retry") else {
                    key: result.get(key) for key in ["id", "status", "elapsed_seconds", "attempt", "error"]})
                if phase in ("main", "retry"):
                    write_json(root / "results" / (case["id"] + ".json"), result)
            write_json(root / "status.json", {
                "state": "running", "phase": phase, "parallelism": parallelism,
                "finished_in_phase": len(results), "total_in_phase": len(cases),
                "prepared_in_phase": sum(r["status"] == "prepared" for r in results),
                "active": [case["id"] for case in active.values()],
                "active_workbooks": [case.get("label", case["id"]) for case in active.values()],
                "elapsed_seconds": time.monotonic() - started,
                "available_memory_bytes": available_memory(),
                "updated_utc": dt.datetime.now(dt.timezone.utc).isoformat()})
    return {"phase": phase, "parallelism": parallelism, "seconds": time.monotonic() - started,
            "results": results, "attempted": len(results),
            "prepared": sum(r["status"] == "prepared" for r in results)}


def main(root):
    # Keep the system awake while this bounded batch owns active work.
    if not ctypes.windll.kernel32.SetThreadExecutionState(0x80000001):
        raise ctypes.WinError()
    config = json.loads((root / "config.json").read_bytes())
    for name in ["temp", "results", "pilot"]:
        (root / name).mkdir(exist_ok=True)
    cases = json.loads((root / "cases.json").read_bytes())
    pilot_file = root / "pilot.json"
    pilot = json.loads(pilot_file.read_bytes()) if pilot_file.exists() else {
        "trials": [], "selection": "Eight bundle-size strata from 2nd percentile through maximum"}
    ordered = sorted(cases, key=lambda c: c["bundle_bytes"])
    positions = [.02, .15, .35, .5, .7, .85, .95, 1.0]
    sample = [ordered[min(len(ordered)-1, int(q * len(ordered)))] for q in positions]
    for workers in config.get("pilot_parallelism", [1, 2, 3, 4]):
        if any(t["parallelism"] == workers and t["attempted"] == len(sample) for t in pilot["trials"]):
            continue
        if (root / "stop.request").exists():
            write_json(root / "status.json", {"state": "stopped", "phase": "pilot"})
            return
        trial_config = dict(config)
        cache_root = root / "pilot" / config.get("pilot_generation", "initial") / f"cache-{workers}"
        trial_config["cp_cache"] = str(cache_root / "cp")
        trial_config["control_cache"] = str(cache_root / "controls")
        trial = run_batch(sample, root, trial_config, f"pilot-{workers}", workers, 240)
        pilot["trials"].append(trial)
        write_json(pilot_file, pilot)
        write_json(root / "pilot-partial.json", pilot["trials"])
        if trial["prepared"] == 0:
            break
    viable = [t for t in pilot["trials"] if t["attempted"] and t["prepared"] / t["attempted"] >= .75]
    if not viable:
        write_json(root / "status.json", {"state": "pilot_failed", "report": str(pilot_file)})
        return
    best = max(viable, key=lambda t: t["prepared"] / t["seconds"])
    durations = [r["elapsed_seconds"] for r in best["results"] if r["status"] == "prepared"]
    rate = best["prepared"] / best["seconds"]
    operation_durations = []
    operation_counts = []
    memory_samples = []
    for record in best["results"]:
        if record["status"] == "prepared":
            operations = json.loads(Path(record["operations_report"]).read_bytes())
            operation_durations.extend(operation["elapsed_seconds"] for operation in operations)
            operation_counts.append(len(operations))
            for operation in operations:
                for point in ("before", "after"):
                    sample = operation.get(point) or {}
                    memory_samples.extend(sample if isinstance(sample, list) else [sample])
    operation_timeout = min(600, max(config.get("minimum_operation_timeout_seconds", 180),
                                    int(max(operation_durations, default=60) * 3 + 30)))
    recommendation = {"parallelism": best["parallelism"], "operation_timeout_seconds": operation_timeout,
                      "workbook_timeout_seconds": config["workbook_timeout_seconds"],
                      "pilot_successes": best["prepared"], "pilot_attempts": best["attempted"],
                      "median_workbook_seconds": statistics.median(durations),
                      "largest_successful_workbook_seconds": max(durations),
                      "median_operations_per_workbook": statistics.median(operation_counts),
                      "largest_observed_operation_seconds": max(operation_durations, default=0),
                      "largest_excel_private_bytes": max((s.get("private_bytes") or 0 for s in memory_samples), default=0),
                      "largest_excel_working_set_bytes": max((s.get("working_set_bytes") or 0 for s in memory_samples), default=0),
                      "largest_owned_process_private_bytes": max((s.get("job_private_bytes") or 0
                          for s in memory_samples), default=0),
                      "estimated_hours_at_observed_success_rate": len(cases) / rate / 3600,
                      "working_root":config["working_root"],
                      "pilot_working_root":config.get("pilot_working_root", config["working_root"]),
                      "throughput_estimate_requires_remeasurement":config.get("pilot_working_root", config["working_root"]) != config["working_root"],
                      "deadline": config["deadline"]}
    write_json(root / "recommendation.json", recommendation)
    # Successful pilot artifacts already satisfy the same preparation contract.
    for trial in pilot["trials"]:
        for result in trial["results"]:
            if result["status"] == "prepared":
                destination = root / "results" / (result["id"] + ".json")
                if not destination.exists():
                    write_json(destination, result)
    completed = {p.stem for p in (root / "results").glob("*.json")}
    remaining = [case for case in cases if case["id"] not in completed]
    batch = run_batch(remaining, root, config, "main", best["parallelism"], operation_timeout)
    write_json(root / "main-result.json", batch)
    failed_ids = {record["id"] for record in batch["results"] if record["status"] != "prepared"}
    if failed_ids and dt.datetime.now(dt.timezone.utc) < dt.datetime.fromisoformat(config["deadline"]):
        retry = run_batch([case for case in cases if case["id"] in failed_ids], root, config,
                          "retry", best["parallelism"], min(600, operation_timeout * 2))
        write_json(root / "retry-result.json", retry)
    terminal = [json.loads(path.read_bytes()) for path in (root / "results").glob("*.json")]
    prepared = sum(record["status"] == "prepared" for record in terminal)
    write_json(root / "status.json", {"state":"finished" if prepared == len(cases) else "finished_with_open_cases",
        "total":len(cases), "prepared":prepared, "failed":len(terminal)-prepared,
        "remaining":len(cases)-len(terminal), "deadline":config["deadline"]})


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", type=Path, required=True)
    arguments = parser.parse_args()
    try:
        main(arguments.root.resolve())
    except Exception as error:
        write_json(arguments.root / "status.json", {"state":"failed", "error":type(error).__name__+": "+str(error)})
        raise
