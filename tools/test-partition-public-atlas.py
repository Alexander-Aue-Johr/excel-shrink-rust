import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location("partition_atlas", Path(__file__).with_name("partition-public-atlas.py"))
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class PublicationPartitionTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.sources = []
        for index in range(2):
            source = self.root / f"source-{index}"
            (source / "images").mkdir(parents=True)
            mapping = {}
            for name, payload in (("common", b"shared-image"), ("own", bytes([index]) * 5)):
                digest = hashlib.sha256(payload).hexdigest()
                relative = f"images/{digest}.webp"
                (source / relative).write_bytes(payload)
                mapping[f"evidence/{name}.png"] = relative
            (source / "presentation-image-map.js").write_text(
                MODULE.MAP_PREFIX + json.dumps(mapping) + MODULE.MAP_SUFFIX + "\n",
                encoding="utf-8",
            )
            (source / "report.json").write_bytes(b'{"status":"failed","different_regions":0}')
            self.sources.append(source)
        self.config = {
            "site_base_url": "https://example.github.io/paper/",
            "shared_base_url": "https://example.github.io/tool/atlas-images/",
            "shared_budget_bytes": len(b"shared-image"),
            "site_output": str(self.root / "site"),
            "shared_output": str(self.root / "shared"),
            "atlases": [
                {"source": str(source), "destination": f"atlas/stage-{index}"}
                for index, source in enumerate(self.sources)
            ],
        }

    def test_shared_asset_is_deduplicated_and_scientific_reports_are_unchanged(self):
        receipt = MODULE.partition(self.config)
        self.assertEqual(receipt["shared_image_bytes"], len(b"shared-image"))
        self.assertEqual(len(receipt["shared_images"]), 1)
        for index, source in enumerate(self.sources):
            output = self.root / "site/atlas" / f"stage-{index}"
            self.assertEqual((source / "report.json").read_bytes(), (output / "report.json").read_bytes())
            _, _, mapping = MODULE.read_mapping(source)
            generated = (output / "presentation-image-map.js").read_text()
            line = generated.splitlines()[0]
            relocated = json.loads(line[len(MODULE.MAP_PREFIX):-len(MODULE.MAP_SUFFIX)])
            self.assertTrue(relocated["evidence/common.png"].startswith(self.config["shared_base_url"]))
            self.assertEqual(relocated["evidence/own.png"], mapping["evidence/own.png"])
            self.assertFalse((output / mapping["evidence/common.png"]).exists())
        self.assertFalse((self.root / "site/.incomplete").exists())
        with self.assertRaises(FileExistsError):
            MODULE.partition(self.config)

    def test_corrupted_asset_is_rejected_before_outputs_are_created(self):
        asset = next((self.sources[0] / "images").iterdir())
        asset.write_bytes(b"corrupted")
        with self.assertRaisesRegex(ValueError, "checksum"):
            MODULE.partition(self.config)
        self.assertFalse((self.root / "site").exists())

    def test_cross_origin_images_are_rejected(self):
        self.config["shared_base_url"] = "https://other.example/images/"
        with self.assertRaisesRegex(ValueError, "origin"):
            MODULE.partition(self.config)

    def test_output_inside_source_is_rejected(self):
        self.config["site_output"] = str(self.sources[0] / "nested")
        with self.assertRaisesRegex(ValueError, "disjoint"):
            MODULE.partition(self.config)

    def test_traversal_and_overlapping_destinations_are_rejected(self):
        for destination in ("../escape", "C:/escape", "atlas/stage-0/nested"):
            with self.subTest(destination=destination):
                self.config["atlases"][1]["destination"] = destination
                with self.assertRaises(ValueError):
                    MODULE.partition(self.config)

    def test_incomplete_source_is_rejected(self):
        (self.sources[0] / ".incomplete").touch()
        with self.assertRaisesRegex(ValueError, "incomplete"):
            MODULE.partition(self.config)


if __name__ == "__main__":
    unittest.main()
