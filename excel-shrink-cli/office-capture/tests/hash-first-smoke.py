"""Opt-in native Excel comparison of live images and hash-first with neighbor context.

Runs live/hash/hash/live with the same binary, fixture and capture settings.
Fresh Excel instances balance launch order; OS/Office caches are retained.
Checks findings, coverage and every retained pixel against the live reference.
Hash-only tiles prove equality within their run; their PNG digest cannot be
compared to the live reference's RGBA digest after equal PNGs were discarded.
Use --pause-check --compare-to <completed-live-sample> for a separate untimed run.
Add --switch-during-pause to select live images for later phases while the active
hash-first phase keeps its transport and evidence unchanged.
"""

import argparse
import hashlib
import importlib.util
import json
from pathlib import Path
import re
import statistics
import time


HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location("pipeline_smoke", HERE / "pipeline-smoke.py")
PIPELINE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PIPELINE)
OUTPUT = PIPELINE.ROOT / ".tmp/hash-first"


def cell_rectangle(tile):
    """Use actual capture cells, not pixel offsets or neighbors in a filtered list."""
    match = re.fullmatch(r"([A-Z]+)([0-9]+):([A-Z]+)([0-9]+)", tile["range"])
    assert match, f"Unexpected capture address: {tile['range']}"

    def column(letters):
        number = 0
        for letter in letters:
            number = number * 26 + ord(letter) - ord("A") + 1
        return number - 1

    return column(match[1]), int(match[2]) - 1, column(match[3]) + 1, int(match[4])


def touching_tiles(left, right):
    ax0, ay0, ax1, ay1 = cell_rectangle(left)
    bx0, by0, bx1, by1 = cell_rectangle(right)
    return ((ax1 == bx0 or bx1 == ax0) and ay0 <= by1 and by0 <= ay1
            or (ay1 == by0 or by1 == ay0) and ax0 <= bx1 and bx0 <= ax1)


def image_asset(study, url, expected_size=None):
    from PIL import Image

    assert url.startswith("/data/"), f"Unexpected image URL: {url}"
    path = study / "control-room" / url.removeprefix("/data/")
    with Image.open(path) as image:
        rgba = image.convert("RGBA")
        if expected_size is not None:
            assert rgba.size == expected_size, f"Unexpected image size: {url}, {rgba.size} != {expected_size}"
        return {"size": list(rgba.size), "rgba_sha256": hashlib.sha256(rgba.tobytes()).hexdigest()}


def tile_images(study, tile, preview_width):
    images = {}
    for profile, metadata in tile["images"].items():
        size = (metadata["width"], metadata["height"])
        item = {key: metadata.get(key) for key in ("x", "y", "width", "height", "pixel_class", "changed_pixels")}
        urls = {key: metadata.get(key) for key in ("url", "preview_url", "full_url", "mask_url")}
        if not urls["url"]:
            assert not any(urls.values()), f"Partial image metadata without a preview: {tile['id']}"
            assert tile.get("comparison_method") == "png-sha256" and tile["status"] == "equal"
            assert not tile.get("context_for"), "Neighbor context must include images"
            item["assets"] = {}
        else:
            assert urls["url"] == urls["preview_url"], "Preview aliases disagree"
            maximum = 1024 if tile["status"] == "different" else preview_width
            preview_size = size if not maximum or size[0] <= maximum else (maximum, max(1, size[1] * maximum // size[0]))
            item["assets"] = {
                key: image_asset(study, url, preview_size if key == "preview_url" else size)
                for key, url in urls.items() if url and key != "url"
            }
            if tile["status"] == "different" or tile.get("context_for"):
                assert metadata.get("retained_full_resolution") and urls["full_url"], "Lost full-resolution difference/context"
            if urls["full_url"]:
                full = item["assets"]["full_url"]
                expected_class = f"{size[0]}x{size[1]}:{full['rgba_sha256']}"
                assert metadata["pixel_class"] == expected_class, "Retained full image differs from its pixel class"
        images[profile] = item
    return images


def collect_result(study):
    records = [PIPELINE.read_json(path) for path in (study / "records").glob("*.json")]
    assert len(records) == 1, "Expected one fixture record"
    record = records[0]
    assert record.get("complete") and record.get("status") == "differences", (
        f"Incomplete fixture: {record.get('status')}, {record.get('error')}"
    )
    assert record.get("whitespace_diagnosis", {}).get("status") == "complete"
    profiles = {entry["profile"]: entry for entry in record["results"]}
    for profile, name in (("maximum-cleaning", "Capture bands"), ("too-aggressive-cleaning", "Hidden data chart")):
        assert any(sheet["name"] == name and sheet.get("image_differences")
                   for sheet in profiles[profile].get("sheets", [])), f"Missing intended difference: {profile}/{name}"
    diagnoses = {entry["profile"]: entry for entry in record["whitespace_diagnosis"]["profiles"]}
    assert any(sheet["index"] == 1 and sheet["category"] == "whitespace_explained"
               for sheet in diagnoses["maximum-cleaning"]["sheets"])
    assert any(sheet["index"] == 2 and sheet["category"] == "other"
               for sheet in diagnoses["too-aggressive-cleaning"]["sheets"])
    state = PIPELINE.read_json(next((study / "control-room/groups").glob("*/state.json")))
    settings = PIPELINE.read_json(study / "control-room/control.json")
    mode = state["capture_pipeline"].get("transport", "live-images")
    coverage, evidence = [], {}
    hash_only = context_count = 0
    for sheet in state["sheets"]:
        assert sheet.get("capture_coverage", {}).get("omitted_tiles", 0) == 0
        differing = [tile for tile in sheet["tiles"] if tile["status"] == "different"]
        for tile in sheet["tiles"]:
            assert tile["status"] in ("equal", "different"), tile
            assert set(tile["images"]) == set(PIPELINE.PROFILES), tile
            coverage.append({"sheet": sheet["name"], **{key: tile[key] for key in (
                "range", "core_rows", "width", "height", "status",
            )}})
            images = tile_images(study, tile, settings["equal_preview_width"])
            no_images = all(not image["assets"] for image in images.values())
            assert no_images or all(image["assets"] for image in images.values()), "Partly retained profile group"
            if no_images:
                hash_only += 1
                classes = {image["pixel_class"] for image in images.values()}
                dimensions = {(image["width"], image["height"]) for image in images.values()}
                assert len(classes) == len(dimensions) == 1, "Hash equality requires matching dimensions and every peer hash"
                assert re.search(r"[0-9a-f]{64}$", next(iter(classes))), "Invalid hash equality proof"
            expected_context = sorted(other["id"] for other in differing
                                      if other["id"] != tile["id"] and touching_tiles(tile, other))
            if mode == "hash-first":
                assert sorted(tile.get("context_for", [])) == expected_context, f"Wrong adjacent context: {tile['id']}"
                if expected_context:
                    assert not no_images, f"Missing neighbor capture: {tile['id']}"
                    context_count += 1
            evidence[tile["id"]] = {
                "status": tile["status"], "method": tile.get("comparison_method", "rgba"),
                "images": images, "context_for": tile.get("context_for", []),
                "inspection_regions": tile.get("inspection_regions", []),
            }
    assert len(coverage) >= 24, "Too few capture bands to exercise the bounded buffer"
    if mode == "hash-first":
        assert hash_only > 0, "No distant equal images were freed without an upload"
        assert context_count > 0, "No actual neighbor context was exercised"
    else:
        assert hash_only == 0, "Live mode unexpectedly omitted previews"
    resident = record["resident_capture"]
    physical = len({resident["capture_client_aliases"][name] for name in PIPELINE.PROFILES})
    assert physical == 4, "Fixture must use four physical primary clients"
    throughput = state["throughput"]
    return {
        "signature": {
            "report": PIPELINE.result_signature(record, study), "coverage": coverage,
            "capture_pixels": {key: throughput[key] for key in ("captured_pixels", "core_pixels")},
        },
        "evidence": evidence,
        "metrics": {
            "physical_primary_clients": physical,
            "primary_pids": {name: resident["clients"][name]["pid"] for name in PIPELINE.PROFILES},
            "tiles": len(coverage), "hash_only_tiles": hash_only, "context_tiles": context_count,
            "peak_job_bytes": resident.get("peak_job_bytes"),
            "comparing_elapsed_seconds": record.get("comparing_elapsed_seconds"),
            "throughput": throughput, "pipeline_timings": state.get("pipeline_timings"),
            "capture_pipeline": state.get("capture_pipeline"),
        },
    }


def compare_with_live(sample, baseline):
    assert sample["signature"] == baseline["signature"], "Coverage, findings or captured pixel counts changed"
    for tile_id, tile in sample["evidence"].items():
        original = baseline["evidence"][tile_id]
        assert tile["inspection_regions"] == original["inspection_regions"], f"Inspection regions changed: {tile_id}"
        for profile, image in tile["images"].items():
            reference = original["images"][profile]
            for field in ("x", "y", "width", "height", "changed_pixels"):
                assert image[field] == reference[field], f"Changed image metadata: {tile_id}/{profile}/{field}"
            if not image["assets"]:
                assert tile["method"] == "png-sha256" and tile["status"] == "equal"
                continue
            assert image["pixel_class"] == reference["pixel_class"], f"Captured pixels changed: {tile_id}/{profile}"
            for kind, asset in image["assets"].items():
                if kind in reference["assets"]:
                    assert asset == reference["assets"][kind], f"Retained pixels changed: {tile_id}/{profile}/{kind}"
                else:
                    assert kind == "full_url" and tile["context_for"], "Unexpected extra evidence"
                    assert asset["rgba_sha256"] == reference["pixel_class"].rsplit(":", 1)[1], "Context PNG differs from baseline pixels"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--run", action="store_true", help="Explicitly enable native Excel runs")
    parser.add_argument("--binary", type=Path, default=PIPELINE.ROOT / "target/release/excel-shrink-cli.exe")
    parser.add_argument("--equal-preview-width", type=int, choices=(0, 128, 256, 512, 1024), default=512)
    parser.add_argument("--batch-tiles", type=int, choices=range(1, 17), default=4,
                        help="Requested tiles per batch in BOTH transport modes (default: 4)")
    parser.add_argument("--port", type=int, default=8827)
    parser.add_argument("--deadline", type=int, default=240)
    parser.add_argument("--pause-check", action="store_true")
    parser.add_argument("--switch-during-pause", action="store_true",
                        help="Request live images during the pause; verify the active hash phase stays unchanged")
    parser.add_argument("--compare-to", type=Path, help="Completed LIVE sample directory for a pause check")
    args = parser.parse_args()
    if not 1024 <= args.port <= 65535 or not 30 <= args.deadline <= 600:
        parser.error("Use a port from 1024 to 65535 and a deadline from 30 to 600 seconds")
    if args.pause_check != (args.compare_to is not None):
        parser.error("Use --pause-check and --compare-to together")
    if args.switch_during_pause and not args.pause_check:
        parser.error("--switch-during-pause requires --pause-check")
    archive = PIPELINE.prepare_fixture()
    if not args.run:
        print(json.dumps({"prepared": True, "excel_started": False, "archive": str(archive)}))
        return
    args.binary = args.binary.resolve(strict=True)
    modes = ["hash-first"] if args.pause_check else ["live-images", "hash-first", "hash-first", "live-images"]
    directory = OUTPUT / (("pause-" if args.pause_check else "abba-") + time.strftime("%Y%m%d-%H%M%S"))
    directory.mkdir(parents=True)
    result = {
        "status": "running", "binary_sha256": PIPELINE.digest(args.binary), "input_sha256": PIPELINE.digest(archive),
        "settings": {**PIPELINE.SETTINGS, "equal_preview_width": args.equal_preview_width,
                     "capture_batch_tiles": args.batch_tiles},
        "order": modes, "samples": [], "untimed_pause_check": args.pause_check,
        "switch_during_pause": args.switch_during_pause,
        "cache_policy": "Fresh Excel processes; OS/Office caches retained. ABBA balances order, not all noise.",
        "pixel_check_scope": "All retained pixels and all findings versus live reference; hash-only equality within each run.",
    }
    try:
        baseline = None
        if args.compare_to:
            previous = PIPELINE.read_json(args.compare_to / "receipt.json")
            assert previous["input_sha256"] == result["input_sha256"]
            assert previous["settings"]["equal_preview_width"] == args.equal_preview_width
            assert previous["settings"]["capture_batch_tiles"] == args.batch_tiles, "Reference used a different batch size"
            assert previous["settings"].get("capture_transport", "live-images") == "live-images"
            baseline = collect_result(args.compare_to / "study")
        for index, mode in enumerate(modes, 1):
            label = f"{index}-{mode}"
            sample = PIPELINE.run_sample(args, archive, directory / label, True,
                                         settings_patch={"capture_transport": mode,
                                                         "capture_batch_tiles": args.batch_tiles},
                                         collector=collect_result)
            assert sample["metrics"]["capture_pipeline"].get("transport", "live-images") == mode, "Requested initial transport was not used"
            if baseline is None:
                baseline = sample
            compare_with_live(sample, baseline)
            if mode == "hash-first":
                assert sample["metrics"]["throughput"]["uploaded_images"] < baseline["metrics"]["throughput"]["uploaded_images"], "Hash-first did not avoid image uploads"
            result["samples"].append({"directory": label, "mode": mode, "metrics": sample["metrics"],
                                      "runner_wall_seconds": sample["runner_wall_seconds"]})
            assert PIPELINE.digest(args.binary) == result["binary_sha256"], "Binary changed during experiment"
            assert PIPELINE.digest(archive) == result["input_sha256"], "Fixture changed during experiment"
            PIPELINE.write_json(directory / "receipt.json", result)
        if not args.pause_check:
            result["medians"] = {}
            for mode in set(modes):
                samples = [sample for sample in result["samples"] if sample["mode"] == mode]
                result["medians"][mode] = {
                    "capture_ms": statistics.median(sample["metrics"]["throughput"]["capture_elapsed_ms"] for sample in samples),
                    "comparison_seconds": statistics.median(sample["metrics"]["comparing_elapsed_seconds"] for sample in samples),
                    "peak_job_bytes": statistics.median(sample["metrics"]["peak_job_bytes"] for sample in samples),
                    "uploaded_images": statistics.median(sample["metrics"]["throughput"]["uploaded_images"] for sample in samples),
                    "runner_wall_seconds": statistics.median(sample["runner_wall_seconds"] for sample in samples),
                }
        result.update(status="passed", identical_findings_and_coverage=True, retained_pixels_match_live=True,
                      neighboring_context_complete=True)
    except Exception as error:
        result.update(status="failed", error=f"{type(error).__name__}: {error}")
        raise
    finally:
        PIPELINE.write_json(directory / "receipt.json", result)
        print(json.dumps({"status": result["status"], "receipt": str(directory / "receipt.json"),
                          "medians": result.get("medians")}), flush=True)


if __name__ == "__main__":
    main()
