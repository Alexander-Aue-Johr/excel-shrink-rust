"""Check a verified runtime's embedded web code against its packaged assets."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import socket
import subprocess
import time
import urllib.error
import urllib.request


spec = importlib.util.spec_from_file_location(
    "package_validation", Path(__file__).with_name("package-validation.py")
)
package_validation = importlib.util.module_from_spec(spec)
spec.loader.exec_module(package_validation)


def check(runtime, work):
    runtime = runtime.resolve()
    manifest = package_validation.verify(runtime)
    work = work.resolve()
    if work == runtime or work.is_relative_to(runtime):
        raise ValueError("Choose a working directory outside the runtime package")
    work.mkdir(parents=True)
    environment = {
        **os.environ,
        "DELTASIEVE_DATA_DIR": str(work / "data"),
        "TEMP": str(work / "temp"),
        "TMP": str(work / "temp"),
        "EXCEL_SHRINK_REPOSITORY_ROOT": str(runtime),
        "EXCEL_SHRINK_CAPTURE_DIR": str(runtime / "office-capture"),
    }
    (work / "temp").mkdir()
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))

    def get(path):
        request = urllib.request.Request(
            f"http://127.0.0.1:{port}{path}",
            headers={"Host": f"deltasieve.localhost:{port}"},
        )
        with opener.open(request, timeout=2) as response:
            return response.read()

    receipt = {
        "status": "running",
        "binary_sha256": manifest["files"]["deltasieve.exe"]["sha256"],
        "checked_assets": [],
    }
    with (work / "stdout.log").open("wb") as stdout, (work / "stderr.log").open("wb") as stderr:
        process = subprocess.Popen(
            [str(runtime / "deltasieve.exe"), "observatory", "--port", str(port), "--no-open"],
            cwd=runtime, env=environment, stdout=stdout, stderr=stderr,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
        start = time.monotonic()
        try:
            while True:
                try:
                    session = json.loads(get("/api/session"))
                    break
                except (OSError, urllib.error.URLError):
                    if process.poll() is not None or time.monotonic() - start >= 30:
                        raise RuntimeError("Runtime web host did not become ready within 30 seconds")
                    time.sleep(0.1)
            if session["application"] != "deltasieve":
                raise ValueError("Runtime reported an unexpected application identity")
            for name in ("sheet-scope.js", "sheet-scope.css"):
                relative = Path("office-capture/control-room") / name
                if get("/" + name) != (runtime / relative).read_bytes():
                    raise ValueError(f"Embedded {name} differs from the packaged asset; rebuild the executable")
                receipt["checked_assets"].append(relative.as_posix())
            state = json.loads(get("/api/state"))
            if state["active"] or state["progress"]["state"] != "idle":
                raise ValueError("Configuration host unexpectedly started processing")
            receipt["status"] = "passed"
        except BaseException as error:
            receipt.update(status="failed", error=str(error))
            raise
        finally:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
            receipt["elapsed_seconds"] = time.monotonic() - start
            (work / "web-check.json").write_text(json.dumps(receipt, indent=2) + "\n", encoding="utf-8")
    package_validation.verify(runtime)
    return receipt


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True, help="New directory outside the runtime")
    arguments = parser.parse_args()
    print(json.dumps(check(arguments.runtime, arguments.work)))


if __name__ == "__main__":
    main()
