"""DMG verification uses only new owned paths and a read-only attach."""
from pathlib import Path
import tempfile
import unittest

import dmg
import package


class DmgTests(unittest.TestCase):
    def test_round_trip_verifies_mounted_and_copied_hashes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            executable = root / "stand-in"
            executable.write_bytes(b"\x00stand-in executable\n")
            identity = package.assemble(executable, root / "bundle")
            image = root / "native.dmg"
            created = dmg.create(root / "bundle", image)
            self.assertEqual(created["files_sha256"], identity["files_sha256"])
            report = dmg.verify(image, identity, root / "Applications")
            self.assertTrue(report["read_only"])
            self.assertEqual(report["mounted_files"], len(identity["files_sha256"]))
            self.assertEqual(report["copied_files"], len(identity["files_sha256"]))
            with self.assertRaises(FileExistsError):
                dmg.create(root / "bundle", image)

    def test_tampered_bundle_is_refused_before_imaging(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            executable = root / "stand-in"
            executable.write_bytes(b"original")
            package.assemble(executable, root / "bundle")
            (root / "bundle" / package.APP_NAME / "Contents/MacOS/dbunk-native").write_bytes(b"changed")
            with self.assertRaisesRegex(RuntimeError, "changed="):
                dmg.create(root / "bundle", root / "native.dmg")
            self.assertFalse((root / "native.dmg").exists())


if __name__ == "__main__":
    unittest.main()
