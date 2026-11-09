import copy
import json
import unittest

from build_enron_source_profile import ITEM, build_profile


class EnronSourceProfileTests(unittest.TestCase):
    def setUp(self):
        self.metadata = {
            "metadata": {"identifier": ITEM, "title": "EDRM Enron v2 XML", "creator": "Enron"},
            "files": [
                {"name": name, "size": "10", "sha1": "a" * 40, "md5": "b" * 32}
                for name in [
                    "edrm-enron-v2_kean-s_xml_2of8.zip",
                    "edrm-enron-v2_allen-p_xml.zip",
                    "edrm-enron-v2_kean-s_xml_1of8.zip",
                ]
            ] + [{"name": "metadata.xml"}],
        }

    def build(self, metadata=None):
        body = json.dumps(metadata or self.metadata).encode("utf-8")
        return build_profile(body, "2026-11-09T01:20:11Z")

    def test_retains_split_archives_and_explicitly_pending_extraction(self):
        profile = self.build()
        self.assertEqual(len(profile["archives"]), 3)
        self.assertEqual(sum(item["bytes"] for item in profile["archives"]), 30)
        self.assertEqual(profile["archives"][0]["name"], "edrm-enron-v2_allen-p_xml.zip")
        self.assertEqual(profile["extraction"]["status"], "pending-recipe")
        self.assertEqual(len(profile["source"]["metadata_document_sha256"]), 64)
        self.assertNotIn("workbooks", profile)
        self.assertNotIn("sha256", profile["archives"][0])

    def test_rejects_wrong_collection_and_unsafe_names(self):
        metadata = copy.deepcopy(self.metadata)
        metadata["metadata"]["identifier"] = "other-item"
        with self.assertRaises(ValueError):
            self.build(metadata)
        metadata = copy.deepcopy(self.metadata)
        metadata["files"][0]["name"] = "../escape.zip"
        with self.assertRaises(ValueError):
            self.build(metadata)

    def test_rejects_duplicate_names_and_invalid_checksums(self):
        metadata = copy.deepcopy(self.metadata)
        metadata["files"].append(metadata["files"][0])
        with self.assertRaises(ValueError):
            self.build(metadata)
        for checksum in ["sha1", "md5"]:
            metadata = copy.deepcopy(self.metadata)
            metadata["files"][0][checksum] = "invalid"
            with self.assertRaises(ValueError):
                self.build(metadata)

    def test_rejects_empty_collection_and_zero_length_archives(self):
        metadata = copy.deepcopy(self.metadata)
        metadata["files"] = []
        with self.assertRaises(ValueError):
            self.build(metadata)
        metadata = copy.deepcopy(self.metadata)
        metadata["files"][0]["size"] = "0"
        with self.assertRaises(ValueError):
            self.build(metadata)


if __name__ == "__main__":
    unittest.main()
