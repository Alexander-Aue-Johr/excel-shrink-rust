"""Verify deterministic ownership, legacy resume, and foreign-only viewport traversal."""

from argparse import Namespace
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import AsyncMock, MagicMock, patch

from . import runner as flow
from . import visible_batches as visible
from .workers import bind_worker, owned_names, worker_for_name


class WorkerTests(unittest.TestCase):
    def test_hash_protocol_and_disjoint_complete_partition(self):
        self.assertEqual(worker_for_name("alpha.xlsx", 2), 1)
        self.assertEqual(worker_for_name("alpha.xlsx", 7), 12739248936139767823 % 7)
        names = [f"file-{index}.xlsx" for index in range(100)] + ["Änderung.xlsx", "测量.xlsx"]
        first = set(owned_names(names, Namespace(worker_count=2, worker_id=0)))
        second = set(owned_names(names, Namespace(worker_count=2, worker_id=1)))
        self.assertFalse(first.intersection(second))
        self.assertEqual(first.union(second), set(names))
        self.assertEqual(owned_names(names, Namespace()), names)

    def test_cli_rejects_invalid_assignments_and_foreign_target(self):
        for options in (
            ["--worker-count", "0"], ["--worker-count", "2", "--worker-id", "2"],
            ["--worker-id", "-1"],
            ["--worker-count", "2", "--worker-id", "0", "--only-file", "alpha.xlsx"],
        ):
            with self.subTest(options=options), patch.object(
                    flow.sys, "argv", ["runner", "https://1drv.ms/f/test", *options]), patch(
                    "sys.stderr", io.StringIO()):
                with self.assertRaises(SystemExit) as raised:
                    flow.parse_arguments()
                self.assertEqual(raised.exception.code, 2)

    def test_legacy_results_survive_partition_binding_and_identity_is_checked(self):
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                progress.discover(["alpha.xlsx"])
                attempt = progress.start_attempt("alpha.xlsx")
                progress.skip("alpha.xlsx", attempt, "performance_unavailable")
                bind_worker(progress, Namespace(worker_count=2, worker_id=0))
                self.assertEqual(progress.status("alpha.xlsx"), "skipped")
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                bind_worker(progress, Namespace(worker_count=2, worker_id=0))
                with self.assertRaisesRegex(ValueError, "different worker"):
                    bind_worker(progress, Namespace(worker_count=2, worker_id=1))
                bind_worker(progress, Namespace(worker_count=2, worker_id=0))


class WorkerFlowTests(unittest.IsolatedAsyncioTestCase):
    async def test_inventory_discovery_keeps_only_owned_names(self):
        args = Namespace(worker_count=2, worker_id=0, retry_skipped=False,
                         move_optimized=False, refresh_list=True, list_only=False)
        names = [f"file-{index}.xlsx" for index in range(20)]
        expected = owned_names(names, args)
        with tempfile.TemporaryDirectory() as temporary:
            with flow.ProgressStore(Path(temporary), "folder") as progress:
                with patch.object(flow, "discover_workbooks", new=AsyncMock(return_value=names)):
                    self.assertEqual(await flow.load_workbook_inventory(None, progress, args), expected)
                self.assertEqual(progress.names(), expected)

    async def test_foreign_only_viewport_scrolls_and_foreign_results_are_not_moved(self):
        with tempfile.TemporaryDirectory() as temporary:
            args = Namespace(folder_url="https://1drv.ms/f/test", output_root=Path(temporary),
                             profile_dir=None, headed=True, refresh_list=False, list_only=False,
                             only_file=None, retry_skipped=False, move_optimized=True,
                             max_files=10, worker_count=2, worker_id=0)
            own = owned_names([f"file-{i}.xlsx" for i in range(20)], args)[:3]
            foreign = "alpha.xlsx"
            directory = args.output_root / flow.folder_id(args.folder_url)
            with flow.ProgressStore(directory, flow.folder_id(args.folder_url)) as progress:
                progress.discover([foreign, *own])
                for name in (foreign, own[1]):
                    attempt = progress.start_attempt(name)
                    progress.transition(name, flow.JobState.CHECKING)
                    progress.finish(name, attempt, "already_clean")
                attempt = progress.start_attempt(own[2])
                progress.skip(own[2], attempt, "performance_unavailable")
            files = [foreign, *own]
            stage = 0
            browser = None, None, MagicMock()
            browser[2].locator.return_value.first.wait_for = AsyncMock()
            checked, moved = [], []

            async def snapshot(_page):
                return {"files": [foreign] if stage == 0 else list(files),
                        "scroll_top": stage * 100, "more_below": stage == 0}

            async def scroll(*args, **kwargs):
                nonlocal stage
                stage = 1
                return 100, 100

            async def process(*values, **kwargs):
                progress, name = values[5], values[7]
                checked.append(name)
                attempt = progress.start_attempt(name)
                progress.transition(name, flow.JobState.CHECKING)
                progress.finish(name, attempt, "already_clean")
                return *browser, False

            async def move(current, _args, _profile, progress, names, destination):
                moved.extend(names)
                batch = progress.start_move_batch(names, destination)
                progress.finish_move_batch(batch)
                for name in names:
                    files.remove(name)
                return current

            with (patch.object(flow, "restart_folder_browser", new=AsyncMock(return_value=browser)),
                  patch.object(flow, "wait_for_folder_rows", new=AsyncMock()),
                  patch.object(flow, "sort_newest_first", new=AsyncMock()),
                  patch.object(flow, "scroll_file_list", new=AsyncMock(side_effect=scroll)) as scrolling,
                  patch.object(flow, "process_selected_workbook", new=process),
                  patch.object(flow, "close_browser", new=AsyncMock()),
                  patch.object(flow, "ConsoleProgress"),
                  patch.object(visible, "snapshot", new=snapshot),
                  patch.object(visible, "move_with_recovery", new=move),
                  patch.object(visible.asyncio, "sleep", new=AsyncMock())):
                self.assertEqual(await flow.run_folder_check_performance(args), 0)
            self.assertEqual(checked, [own[0]])
            self.assertEqual(set(moved), set(own))
            self.assertEqual(files, [foreign])
            scrolling.assert_awaited_once()
