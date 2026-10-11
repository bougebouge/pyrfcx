"""Run a test script in a fresh Python process.

For SDK state that is process-wide and cannot be reset, such as the crypto
library path. The child inherits the environment that conftest.py pinned
(SAPNWRFC_HOME, PATH) and runs in the repository root, where sapnwrfc.ini
lives.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
import textwrap
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent


def crypto_library_path() -> str:
    """Path of the SAP crypto library for this process, or "" when unset.

    On Windows SNC_LIB may name the 32-bit library; a 64-bit process must
    take SNC_LIB_64 when it is set.
    """
    if sys.platform == "win32":
        return os.environ.get("SNC_LIB_64") or os.environ.get("SNC_LIB", "")
    return os.environ.get("SNC_LIB", "")


def run_isolated(script: str) -> Any:
    """Run `script` in a new interpreter; return the JSON value it prints on
    its last line of output."""
    proc = subprocess.run(
        [sys.executable, "-u", "-c", textwrap.dedent(script)],
        cwd=REPO_ROOT,
        capture_output=True,
        text=True,
        timeout=120,
        check=False,
    )
    assert proc.returncode == 0, proc.stderr
    return json.loads(proc.stdout.splitlines()[-1])
