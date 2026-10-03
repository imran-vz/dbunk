"""Bundle assembly must not overwrite existing artifacts or follow output links."""
from pathlib import Path
import tempfile
import unittest

import package


class BundleOwnershipTests(unittest.TestCase):
    def test_existing_output_is_untouched(self):
        with tempfile.TemporaryDirectory() as temporary:
            output = Path(temporary).resolve()
            sentinel = output / "foreign"
            sentinel.write_text("keep")
            with self.assertRaises(FileExistsError):
                package.assemble(output / "missing-executable", output)
            self.assertEqual(sentinel.read_text(), "keep")
            self.assertEqual(list(output.iterdir()), [sentinel])

    def test_symlink_ancestor_refused_before_writing(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary).resolve()
            target = root / "foreign"
            target.mkdir()
            link = root / "link"
            link.symlink_to(target, target_is_directory=True)
            with self.assertRaisesRegex(RuntimeError, "symlink"):
                package.assemble(root / "missing-executable", link / "output")
            self.assertEqual(list(target.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
