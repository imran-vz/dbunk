"""Ownership failures are checked before fixture access or profile deletion."""
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

import fixture
import launch
import local_fixture
import runtime


class FixtureOwnershipTests(unittest.TestCase):
    def info(self):
        return {
            "Config": {"Labels": {
                "com.docker.compose.project": fixture.PROJECT,
                "com.docker.compose.service": "postgres",
                "dev.dbunk.fixture": "native-stage03",
                "dev.dbunk.fixture.instance": "owned",
            }},
            "HostConfig": {
                "PortBindings": {"5432/tcp": [{"HostIp": "127.0.0.1", "HostPort": "15432"}]},
                "Tmpfs": {"/var/lib/postgresql/data": ""},
            },
        }

    def test_foreign_container_refused(self):
        info = self.info()
        info["Config"]["Labels"]["dev.dbunk.fixture.instance"] = "foreign"
        with self.assertRaisesRegex(RuntimeError, "ownership"):
            fixture.validate_container(info, {"instance": "owned"})

    def test_non_loopback_or_persistent_storage_refused(self):
        info = self.info()
        info["HostConfig"]["PortBindings"]["5432/tcp"][0]["HostIp"] = "0.0.0.0"
        with self.assertRaisesRegex(RuntimeError, "127.0.0.1"):
            fixture.validate_container(info, {"instance": "owned"})
        info = self.info()
        info["HostConfig"]["Tmpfs"] = {}
        with self.assertRaisesRegex(RuntimeError, "tmpfs"):
            fixture.validate_container(info, {"instance": "owned"})

    def test_existing_unowned_fixture_is_not_contacted(self):
        with tempfile.TemporaryDirectory() as directory:
            with patch.object(fixture, "STATE", Path(directory) / "state.json"), patch.object(fixture, "prerequisites"), patch.object(fixture, "containers", return_value=["foreign"]), patch.object(fixture, "sql") as sql:
                with self.assertRaisesRegex(RuntimeError, "refusing to adopt"):
                    fixture.up()
                sql.assert_not_called()

    def test_local_fixture_rejects_foreign_executable_before_process_check(self):
        with patch.object(fixture, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "task-private"):
                local_fixture.validate({"executable": "/usr/local/bin/postgres"})
            run.assert_not_called()

    def test_local_fixture_rejects_changed_pid_command(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            owned = {"executable": str(runtime.PREFIX / "bin/postgres"), "pid": 123}
            (root / ".dbunk-native-local").write_text(json.dumps(owned))
            with patch.object(local_fixture, "directory", return_value=root), patch.object(fixture, "run", return_value="foreign-postgres -D /daily-driver"):
                with self.assertRaisesRegex(RuntimeError, "process identity"):
                    local_fixture.validate(owned)

    def test_fault_injection_rejects_invalid_identity_before_fixture_access(self):
        with patch.object(fixture, "check") as check:
            for pid, start in [(-1, "2026-10-02 10:30:00+00"), (123, "2026-10-02 10:30:00+00'; SELECT 1;")]:
                with self.assertRaises(RuntimeError):
                    fixture.terminate_query(pid, start, "2283820d-33ec-4c4c-ae03-7051092bd410")
            check.assert_not_called()

    def test_fault_injection_rejects_foreign_instance_without_termination(self):
        with patch.object(fixture, "check", return_value=({"instance": "foreign"}, "owned")), patch.object(fixture, "sql") as sql:
            with self.assertRaisesRegex(RuntimeError, "instance differs"):
                fixture.terminate_query(123, "2026-10-02 10:30:00.123456+05:30", "2283820d-33ec-4c4c-ae03-7051092bd410")
            sql.assert_not_called()

    def test_stale_query_identity_is_not_reported_as_success(self):
        instance = "2283820d-33ec-4c4c-ae03-7051092bd410"
        with patch.object(fixture, "check", return_value=({"instance": instance}, "owned")), patch.object(fixture, "sql", return_value=""):
            with self.assertRaisesRegex(RuntimeError, "stale or mismatched"):
                fixture.terminate_query(123, "2026-10-02 10:30:00+00", instance)

    def test_cleanup_requires_unchanged_profile_marker(self):
        profile, marker = launch.create_profile()
        try:
            (profile / launch.MARKER).write_text(json.dumps({"profile_id": "foreign"}))
            with self.assertRaisesRegex(RuntimeError, "ownership changed"):
                launch.cleanup_profile(profile, marker)
            self.assertTrue(profile.exists())
        finally:
            (profile / launch.MARKER).write_text(json.dumps(marker))
            launch.cleanup_profile(profile, marker)

    def test_cleanup_does_not_follow_symlink(self):
        profile, marker = launch.create_profile()
        try:
            (profile / "foreign").symlink_to("/tmp")
            with self.assertRaisesRegex(RuntimeError, "symlink"):
                launch.cleanup_profile(profile, marker)
        finally:
            (profile / "foreign").unlink()
            launch.cleanup_profile(profile, marker)


if __name__ == "__main__":
    unittest.main()
