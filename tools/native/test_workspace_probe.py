"""Probe compilation must not leave a stale fixture check before execution."""
import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock

import workspace_probe


class WorkspaceProbeTests(unittest.TestCase):
    def exercise(self, replaced):
        with tempfile.TemporaryDirectory() as temporary, contextlib.ExitStack() as stack:
            root = Path(temporary).resolve()
            executable = root / "target/debug/examples/native_workspace_probe"
            executable.parent.mkdir(parents=True)
            executable.write_bytes(b"synthetic binary; subprocesses are mocked")
            owned = {"instance": "2283820d-33ec-4c4c-ae03-7051092bd410"}
            observations = [(owned, {})]
            observations += [({"instance": "replaced-instance"}, {})] if replaced else [(owned, {}), (owned, {})]
            check = stack.enter_context(mock.patch.object(workspace_probe.fixture, "check", side_effect=observations))
            stack.enter_context(mock.patch.object(workspace_probe.fixture, "backend_count", return_value=0))
            stack.enter_context(mock.patch.object(workspace_probe.fixture, "wait_baseline"))
            stack.enter_context(mock.patch.object(workspace_probe.sys, "platform", "darwin"))
            run = stack.enter_context(mock.patch.object(workspace_probe.subprocess, "run"))
            stack.enter_context(mock.patch.object(workspace_probe.subprocess, "check_output", return_value=json.dumps({"target_directory": str(root / "target")})))
            stack.enter_context(contextlib.redirect_stdout(io.StringIO()))
            path = root / "general"
            if replaced:
                with self.assertRaisesRegex(RuntimeError, "ownership changed"):
                    workspace_probe.run(path, general_profile=True)
                self.assertEqual(run.call_count, 1, "replacement must refuse before either probe process")
                self.assertFalse(path.exists())
            else:
                workspace_probe.run(path, general_profile=True)
                self.assertEqual(check.call_count, 3)
                commands = [call.args[0] for call in run.call_args_list]
                self.assertEqual(commands[0][:2], ["cargo", "build"])
                self.assertEqual(commands[1][:3], [str(executable), "create-general", str(path)])
                self.assertEqual(commands[2][:3], [str(executable), "reopen-general", str(path)])

    def test_replaced_fixture_after_build_cannot_be_contacted(self):
        self.exercise(replaced=True)

    def test_create_and_reopen_use_the_prebuilt_exact_probe(self):
        self.exercise(replaced=False)
