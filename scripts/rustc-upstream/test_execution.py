"""Exercise the compiletest runner as a subprocess, including Unix signals."""

import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import tempfile
import unittest

from checks import normalized_output


class CaptureRunnerTests(unittest.TestCase):
    def capture(self, script):
        directory = tempfile.TemporaryDirectory()
        self.addCleanup(directory.cleanup)
        root = Path(directory.name)
        env = dict(os.environ, KFRC_UPSTREAM_CAPTURE_ROOT=str(root / "runs"),
                   KFRC_UPSTREAM_BUILD_ROOT=str(Path(sys.executable).resolve().parent))
        process = subprocess.run([sys.executable, str(Path(__file__).with_name("capture-run.py")),
                                  sys.executable, "-c", script], env=env, capture_output=True)
        records = list((root / "runs").glob("*/run.json"))
        self.assertEqual(len(records), 1)
        return process, json.loads(records[0].read_text()), records[0].parent

    def test_binary_streams_and_nonzero_status_are_preserved(self):
        process, record, directory = self.capture(
            "import sys; sys.stdout.buffer.write(bytes(range(256))); "
            "sys.stderr.buffer.write(b'error\\x00detail'); sys.exit(7)")
        self.assertEqual(process.returncode, 7)
        self.assertEqual(record["returncode"], 7)
        self.assertEqual(process.stdout, bytes(range(256)))
        self.assertEqual(process.stderr, b"error\x00detail")
        self.assertEqual((directory / "stdout").read_bytes(), process.stdout)
        self.assertEqual((directory / "stderr").read_bytes(), process.stderr)

    @unittest.skipUnless(os.name == "posix", "Unix signal termination")
    def test_signal_termination_is_preserved(self):
        process, record, _ = self.capture("import os,signal; os.kill(os.getpid(), signal.SIGTERM)")
        self.assertEqual(process.returncode, -signal.SIGTERM)
        self.assertEqual(record["returncode"], -signal.SIGTERM)

    def test_normalization_preserves_diagnostic_numbers(self):
        record = {"executable": "/work/llvm/test"}
        self.assertEqual(
            normalized_output(b"/work/llvm/test: PID 123 address 0xabc panic 42", record, "/work/llvm"),
            b"$TEST_BINARY: PID 123 address 0xabc panic 42")


if __name__ == "__main__":
    unittest.main()
