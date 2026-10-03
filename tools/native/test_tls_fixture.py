"""TLS fixture ownership is established before SQL, process control or deletion."""
import json
import os
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import patch

import profile as stage04_profile
import tls_fixture
import tls_runtime


class TlsFixtureTests(unittest.TestCase):
    def prepared(self, root):
        owned = {"version": 1, "project": tls_fixture.PROJECT, "instance": root.name, "executable": str(tls_runtime.PREFIX / "bin/postgres")}
        root.mkdir(parents=True)
        (root / "ca.pem").write_text("synthetic certificate")
        owned["files_sha256"] = {"ca.pem": tls_runtime.digest(root / "ca.pem")}
        (root / tls_fixture.MARKER).write_text(json.dumps(owned))
        return dict(owned, phase="prepared")

    def test_occupied_port_is_not_contacted_or_replaced(self):
        with tempfile.TemporaryDirectory() as temporary:
            with patch.object(tls_fixture, "STATE", Path(temporary) / "tls.json"), patch.object(tls_fixture.socket, "socket") as socket, patch.object(tls_runtime, "prepare") as prepare, patch.object(tls_fixture.fixture, "run") as run:
                socket.return_value.__enter__.return_value.bind.side_effect = OSError("occupied")
                with self.assertRaisesRegex(RuntimeError, "refusing to contact"):
                    tls_fixture.prepare()
                prepare.assert_not_called()
                run.assert_not_called()

    def test_foreign_runtime_is_rejected_before_process_or_sql_access(self):
        owned = {"project": tls_fixture.PROJECT, "instance": "2283820d-33ec-4c4c-ae03-7051092bd410", "executable": "/daily-driver/postgres"}
        with patch.object(tls_fixture.fixture, "run") as run:
            with self.assertRaisesRegex(RuntimeError, "owned TLS runtime"):
                tls_fixture.validate(owned)
            run.assert_not_called()

    def test_certificate_changes_fail_before_postgres_is_contacted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "2283820d-33ec-4c4c-ae03-7051092bd410"
            owned = self.prepared(root)
            (root / "ca.pem").write_text("replaced certificate")
            with patch.object(tls_fixture, "directory", return_value=root), patch.object(tls_runtime, "validate"), patch.object(tls_fixture.fixture, "run") as run:
                with self.assertRaisesRegex(RuntimeError, "certificate/configuration identity"):
                    tls_fixture.validate(owned)
                run.assert_not_called()

    def test_reused_pid_cannot_be_stopped_or_deleted(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "2283820d-33ec-4c4c-ae03-7051092bd410"
            owned = dict(self.prepared(root), phase="running", pid=123)
            with patch.object(tls_fixture, "state", return_value=owned), patch.object(tls_fixture, "directory", return_value=root), patch.object(tls_runtime, "validate"), patch.object(tls_fixture.subprocess, "run", return_value=subprocess.CompletedProcess([], 0)), patch.object(tls_fixture.fixture, "run", return_value="foreign-postgres -D /daily-driver"), patch.object(tls_fixture.shutil, "rmtree") as remove:
                with self.assertRaisesRegex(RuntimeError, "process identity"):
                    tls_fixture.down()
                remove.assert_not_called()
                self.assertTrue(root.exists())

    def test_prepared_directory_with_postmaster_is_preserved(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve() / "2283820d-33ec-4c4c-ae03-7051092bd410"
            owned = self.prepared(root)
            (root / "data").mkdir()
            (root / "data/postmaster.pid").write_text("foreign PID")
            with patch.object(tls_fixture, "directory", return_value=root), patch.object(tls_runtime, "validate") as validate:
                with self.assertRaisesRegex(RuntimeError, "postmaster"):
                    tls_fixture.validate(owned, running=False)
                validate.assert_not_called()

    def test_libpq_cannot_inherit_foreign_endpoint_or_ambient_certificates(self):
        root = Path("/owned/tls")
        with patch.dict(os.environ, {"PGHOSTADDR": "foreign.example", "PGSERVICE": "daily-driver", "PGSSLCERT": "/private/real-client.pem"}):
            env = tls_fixture.connection_env(root, server_name="wrong.dbunk.invalid")
        self.assertEqual(env["PGHOSTADDR"], "127.0.0.1")
        self.assertEqual(env["PGHOST"], "wrong.dbunk.invalid")
        self.assertEqual(env["PGPORT"], "15433")
        self.assertNotIn("PGSERVICE", env)
        self.assertEqual(env["PGSSLCERT"], str(root / "no-client.pem"))

    def test_tls_manifest_requires_explicit_optin(self):
        plain = {"instance": "plain-instance"}
        tls = tls_fixture.manifest({"instance": "tls-instance"})
        self.assertNotIn("tls", stage04_profile.manifest(plain))
        self.assertEqual(stage04_profile.manifest(plain, tls)["tls"], tls)


if __name__ == "__main__":
    unittest.main()
