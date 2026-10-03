"""Launcher guards never adopt foreign profiles or altered packaged binaries."""
import json
import os
from pathlib import Path
import stat
import tempfile
import unittest
from unittest import mock
import uuid

import package
import profile as stage04_profile
import workspace_launch


class WorkspaceLaunchTests(unittest.TestCase):
    def test_existing_profile_requires_matching_path_and_fixture_instance(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            owned = {"instance": "2283820d-33ec-4c4c-ae03-7051092bd410"}
            workspace = root / "workspace"
            workspace_launch.validate_path(workspace, owned)
            workspace.mkdir()
            sentinel = workspace / "foreign"
            sentinel.write_text("keep")
            with self.assertRaisesRegex(RuntimeError, "owned stage04"):
                workspace_launch.validate_path(workspace, owned)
            marker = {"version": 1, "path": str(workspace), "fixtures": stage04_profile.manifest(owned)}
            (workspace / ".dbunk-native-stage04").write_text(json.dumps(marker))
            workspace_launch.validate_path(workspace, owned)
            with self.assertRaisesRegex(RuntimeError, "different fixture"):
                workspace_launch.validate_path(workspace, {"instance": "another-instance"})
            self.assertEqual(sentinel.read_text(), "keep")

    def test_symlink_ancestor_is_refused_without_modifying_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            target = root / "target"
            target.mkdir()
            alias = root / "alias"
            alias.symlink_to(target, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "canonical"):
                workspace_launch.validate_path(alias / "new-workspace", {"instance": "unused"})
            self.assertEqual(list(target.iterdir()), [])

    def test_packaged_executable_hash_is_required_and_tampering_is_refused(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            source = root / "probe-binary"
            source.write_bytes(b"synthetic test executable; never run")
            identity = package.assemble(source, root / "output")
            bundle = Path(identity["bundle"])
            executable = Path(identity["executable"])
            self.assertEqual(workspace_launch.bundle_executable(bundle), executable)
            executable.write_bytes(b"altered")
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                workspace_launch.bundle_executable(bundle)
            identity["files_sha256"] = {}
            (bundle.parent / package.MARKER).write_text(json.dumps(identity))
            with self.assertRaisesRegex(RuntimeError, "missing the executable hash"):
                workspace_launch.bundle_executable(bundle)

    def test_manifest_is_private_and_cannot_overwrite_existing_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary) / "fixture.json"
            workspace_launch.private_json(output, {"instance": "owned"})
            self.assertEqual(stat.S_IMODE(output.stat().st_mode), 0o600)
            with self.assertRaises(FileExistsError):
                workspace_launch.private_json(output, {"instance": "replacement"})
            self.assertEqual(json.loads(output.read_text()), {"instance": "owned"})


class GeneralProfileTests(unittest.TestCase):
    owned = {"instance": "2283820d-33ec-4c4c-ae03-7051092bd410"}

    def marker(self, path):
        path.mkdir(mode=0o700)
        marker = {
            "version": 1, "kind": "general-postgres", "path": str(path),
            "profile_id": str(uuid.uuid4()), "credential_namespace": str(uuid.uuid4()),
        }
        workspace_launch.private_json(path / workspace_launch.GENERAL_MARKER, marker)
        return marker

    def receipt(self, root, path, marker, tls=None):
        evidence = root / "prior-evidence"
        evidence.mkdir(mode=0o700)
        receipt = evidence / workspace_launch.GENERAL_RECEIPT
        intent = {
            "version": 1, "mode": "create-general-profile", "profile": str(path),
            "fixtures": stage04_profile.manifest(self.owned, tls),
            "launch_id": str(uuid.uuid4()), "executable_sha256": "a" * 64,
        }
        workspace_launch.private_json(evidence / "general-profile-intent.json", intent)
        value = {
            "version": 1, "kind": workspace_launch.RECEIPT_KIND, "marker": marker,
            "fixtures": intent["fixtures"], "launch_id": intent["launch_id"],
            "executable_sha256": intent["executable_sha256"],
        }
        workspace_launch.private_json(receipt, value)
        return receipt

    def test_creation_is_explicit_absent_only_and_never_adopts_fixture(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "profile"
            workspace_launch.validate_general(path, self.owned, None, create=True)
            path.mkdir(mode=0o700)
            (path / ".dbunk-native-stage04").write_text("fixture sentinel")
            with self.assertRaisesRegex(RuntimeError, "absent"):
                workspace_launch.validate_general(path, self.owned, None, create=True)
            with self.assertRaisesRegex(RuntimeError, "prior launcher"):
                workspace_launch.validate_general(path, self.owned, None)
            with self.assertRaisesRegex(RuntimeError, "cannot be mixed"):
                workspace_launch.validate_general(path, self.owned, None, create=True, owner=root / "receipt")
            self.assertEqual((path / ".dbunk-native-stage04").read_text(), "fixture sentinel")

    def test_receipt_matches_marker_fixture_tls_and_creation_intent(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "profile"
            marker = self.marker(path)
            tls = {"fixture": "owned-tls", "instance": str(uuid.uuid4())}
            receipt = self.receipt(root, path, marker, tls)
            workspace_launch.validate_general(path, self.owned, tls, owner=receipt)
            for owned, target_tls in [(self.owned, None), ({"instance": str(uuid.uuid4())}, tls)]:
                with self.assertRaisesRegex(RuntimeError, "fixtures differ"):
                    workspace_launch.validate_general(path, owned, target_tls, owner=receipt)
            intent = receipt.parent / "general-profile-intent.json"
            data = json.loads(intent.read_text())
            data["profile"] = str(root / "foreign")
            intent.write_text(json.dumps(data))
            with self.assertRaisesRegex(RuntimeError, "creation intent"):
                workspace_launch.validate_general(path, self.owned, tls, owner=receipt)

    def test_marker_refuses_future_mixed_duplicate_and_nonindependent_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "profile"
            marker = self.marker(path)
            file = path / workspace_launch.GENERAL_MARKER
            for changed in [dict(marker, version=2), dict(marker, version=True),
                            dict(marker, credential_namespace=marker["profile_id"]),
                            dict(marker, profile_id=str(uuid.uuid1())),
                            dict(marker, path=str(root / "foreign")), dict(marker, future=True)]:
                file.write_text(json.dumps(changed))
                with self.assertRaisesRegex(RuntimeError, "identity differs"):
                    workspace_launch.general_marker(path)
            file.write_text('{"version":1,"version":2}')
            with self.assertRaisesRegex(RuntimeError, "Duplicate"):
                workspace_launch.general_marker(path)
            file.write_text(json.dumps(marker))
            (path / ".dbunk-native-stage04").write_text("foreign")
            with self.assertRaisesRegex(RuntimeError, "mixed fixture"):
                workspace_launch.general_marker(path)

    def test_marker_and_receipt_must_be_private_owned_regular_unlinked_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "profile"
            marker = self.marker(path)
            receipt = self.receipt(root, path, marker)
            for mode in [0o644, 0o660]:
                receipt.chmod(mode)
                with self.assertRaisesRegex(RuntimeError, "private owned"):
                    workspace_launch.validate_general(path, self.owned, None, owner=receipt)
            receipt.chmod(0o600)
            link = root / "hardlink"
            os.link(receipt, link)
            with self.assertRaisesRegex(RuntimeError, "one link"):
                workspace_launch.validate_general(path, self.owned, None, owner=receipt)
            link.unlink()
            alias = root / "alias"
            alias.symlink_to(receipt)
            with self.assertRaisesRegex(RuntimeError, "canonical"):
                workspace_launch.validate_general(path, self.owned, None, owner=alias)
            with mock.patch.object(workspace_launch.os, "getuid", return_value=os.getuid() + 1):
                with self.assertRaisesRegex(RuntimeError, "private owned"):
                    workspace_launch.validate_general(path, self.owned, None, owner=receipt)
            marker_path = path / workspace_launch.GENERAL_MARKER
            os.link(marker_path, root / "linked-marker")
            with self.assertRaisesRegex(RuntimeError, "linked"):
                workspace_launch.general_marker(path)

    def test_bounded_marker_and_receipt_outside_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "profile"
            self.marker(path)
            with self.assertRaisesRegex(RuntimeError, "outside"):
                workspace_launch.validate_general(path, self.owned, None, owner=path / "receipt.json")
            (path / workspace_launch.GENERAL_MARKER).write_text(" " * 8193)
            with self.assertRaisesRegex(RuntimeError, "oversized"):
                workspace_launch.general_marker(path)

    def test_foreign_or_future_receipt_does_not_adopt_profile(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "profile"
            marker = self.marker(path)
            receipt = self.receipt(root, path, marker)
            original = json.loads(receipt.read_text())
            for changed in [dict(original, version=2), dict(original, kind="foreign"),
                            dict(original, marker=dict(marker, credential_namespace=str(uuid.uuid4())))]:
                receipt.write_text(json.dumps(changed))
                with self.assertRaisesRegex(RuntimeError, "receipt identity"):
                    workspace_launch.validate_general(path, self.owned, None, owner=receipt)
                self.assertEqual(workspace_launch.general_marker(path), marker)

    def test_pre_spawn_recheck_refuses_replaced_owned_fixtures(self):
        with mock.patch.object(workspace_launch.fixture, "check", return_value=({"instance": str(uuid.uuid4())}, None)):
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                workspace_launch.recheck_fixtures(self.owned, None)
        import tls_fixture
        tls = {"fixture": "owned-tls", "instance": str(uuid.uuid4())}
        changed = dict(tls, instance=str(uuid.uuid4()))
        with mock.patch.object(workspace_launch.fixture, "check", return_value=(self.owned, None)), \
             mock.patch.object(tls_fixture, "check", return_value=({}, None)), \
             mock.patch.object(tls_fixture, "manifest", return_value=changed):
            with self.assertRaisesRegex(RuntimeError, "identity changed"):
                workspace_launch.recheck_fixtures(self.owned, tls)

    def test_marker_wait_is_bounded_and_detects_child_failure(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary).resolve() / "absent"
            child = mock.Mock()
            child.poll.return_value = None
            with self.assertRaisesRegex(RuntimeError, "Timed out"):
                workspace_launch.wait_general_marker(path, child, timeout=0)
            child.poll.return_value = 7
            with self.assertRaisesRegex(RuntimeError, "exited 7"):
                workspace_launch.wait_general_marker(path, child)
            child.poll.return_value = 0
            with self.assertRaisesRegex(RuntimeError, "before creating"):
                workspace_launch.wait_general_marker(path, child)

    def launch_mocked(self, root, path, output, child, **kwargs):
        executable = root / "dbunk-native"
        executable.write_bytes(b"synthetic binary, never executed")
        def start(command, **options):
            if "--create-native-profile" in command:
                self.marker(path)
            return child
        with mock.patch.object(workspace_launch.sys, "platform", "darwin"), \
             mock.patch.object(workspace_launch.platform, "machine", return_value="arm64"), \
             mock.patch.object(workspace_launch.fixture, "check", return_value=(self.owned, None)), \
             mock.patch.object(workspace_launch.fixture, "backend_count", return_value=0), \
             mock.patch.object(workspace_launch.fixture, "wait_baseline"), \
             mock.patch.object(workspace_launch, "bundle_executable", return_value=executable), \
             mock.patch.object(workspace_launch.subprocess, "Popen", side_effect=start) as popen:
            workspace_launch.launch(path, bundle=root / "owned.app", out=output, **kwargs)
            return popen.call_args

    def child(self):
        child = mock.Mock(pid=123456)
        child.poll.return_value = None
        def wait(**kwargs):
            child.poll.return_value = 0
            return 0
        child.wait.side_effect = wait
        return child

    def test_explicit_create_and_receipted_reopen_argv_identity(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path, output = root / "profile", root / "evidence"
            call = self.launch_mocked(root, path, output, self.child(), create_general=True)
            self.assertEqual(call.args[0][1:], ["--create-native-profile", str(path)])
            receipt = output / workspace_launch.GENERAL_RECEIPT
            self.assertEqual(stat.S_IMODE(receipt.stat().st_mode), 0o600)
            identity = json.loads((output / "identity.json").read_text())
            self.assertEqual(identity["profile_mode"], "create-general-profile")
            self.assertEqual(identity["general_profile_owner"], str(receipt))
            self.assertEqual(identity["executable_sha256"], package.digest(root / "dbunk-native"))
            call = self.launch_mocked(root, path, root / "reopen", self.child(), general_owner=receipt)
            self.assertEqual(call.args[0][1:], ["--native-profile", str(path)])

    def test_receipt_write_failure_stops_only_launched_child(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path, output = root / "profile", root / "evidence"
            child = self.child()
            original = workspace_launch.private_json
            def write(file, value):
                if file.name == workspace_launch.GENERAL_RECEIPT:
                    raise OSError("simulated receipt refusal")
                original(file, value)
            with mock.patch.object(workspace_launch, "private_json", side_effect=write):
                with self.assertRaisesRegex(OSError, "receipt refusal"):
                    self.launch_mocked(root, path, output, child, create_general=True)
            child.send_signal.assert_called_once_with(workspace_launch.signal.SIGTERM)
            child.wait.assert_called_once_with(timeout=5)
            self.assertFalse((output / workspace_launch.GENERAL_RECEIPT).exists())
            self.assertTrue((path / workspace_launch.GENERAL_MARKER).exists())

    def test_fresh_profile_path_is_absent_canonical_and_private(self):
        path = workspace_launch.fresh_profile_path()
        try:
            self.assertFalse(path.exists())
            self.assertEqual(path, path.resolve())
            self.assertEqual(stat.S_IMODE(path.parent.stat().st_mode), 0o700)
        finally:
            path.parent.rmdir()

    def test_default_fixture_argv_is_unchanged(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            path = root / "new-stage04"
            call = self.launch_mocked(root, path, root / "evidence", self.child())
            self.assertEqual(call.args[0][1:3], ["--workspace-profile", str(path)])
            self.assertEqual(call.args[0][3], "--fixture-manifest")
            self.assertFalse(path.exists())


if __name__ == "__main__":
    unittest.main()
