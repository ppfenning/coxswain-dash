"""The coxswain harness runs its configured `pytest -q` check in every repository; here that check runs the crate's own tests."""
import shutil
import subprocess

import pytest


@pytest.mark.skipif(shutil.which("cargo") is None, reason="cargo is not installed")
def test_cargo_test_passes():
    done = subprocess.run(["cargo", "test", "--quiet"], capture_output=True, text=True, check=False)
    assert done.returncode == 0, done.stdout + done.stderr
