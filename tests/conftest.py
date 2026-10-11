"""Shared test setup.

**This module pins the SAP NW RFC SDK and must do so before anything imports
`pyrfcx` or `pyrfc`.** Both libraries resolve the SDK from `SAPNWRFC_HOME` at
import/load time, so the pin has to land first — pytest loads this conftest
before any test module, which is what makes that work.

Why it exists: with `SAPNWRFC_HOME` unset, `ffi::load_sdk` falls back to a
bare "sapnwrfc.dll" and Windows resolves it from the ambient DLL search order
— on the dev machine that is C:\\Windows\\System32\\sapnwrfc.dll (753 PL
1533), silently a *different* SDK from the vendored one (750 PL 18). The two
do not behave identically, so an unpinned run measures whichever
library happened to be found. Real pyrfc, meanwhile, *requires*
`SAPNWRFC_HOME` — so before this pin the two stacks loaded different SDKs and
were not comparable. Both are now pinned to the vendored copy; the pin is
asserted by tests/test_sdk_pin.py.
"""

from __future__ import annotations

import ctypes
import os
import sys
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from collections.abc import Iterator

    from pyrfcx import Connection

SDK_HOME = Path(__file__).resolve().parent.parent / "vendor" / "nwrfcsdk"
SDK_LIB = SDK_HOME / "lib"

if sys.platform == "win32":
    # FIRST, before any DLL can load: turn the OS loader's missing-DLL
    # failures into exceptions instead of modal dialogs. With no user to
    # dismiss one, a popup blocks the whole run — a hang rather than a
    # readable error.
    #
    # NOTE (measured 2026-07-17): this does NOT cover the SDK's "could not
    # open ICU library" dialog, which this file was long believed to
    # neutralize. That box is the SDK's own MessageBox, not the loader's,
    # and no process-wide flag turns it into an exception — a script calling
    # SetErrorMode as its first statement still hangs. What keeps this suite
    # from hanging is the PATH line below.
    # SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX
    ctypes.windll.kernel32.SetErrorMode(0x0001 | 0x0002)

# Force, not setdefault: an ambient SAPNWRFC_HOME is exactly the
# nondeterminism this guards against (mirrors force = true in
# .cargo/config.toml, which pins the same value for `cargo test`).
os.environ["SAPNWRFC_HOME"] = str(SDK_HOME)

if sys.platform == "win32":
    # Make the vendored SDK's own directory resolvable for BOTH stacks.
    #
    # sapnwrfc.dll pulls in siblings (icu*57.dll, libsapucum.dll) via plain
    # LoadLibrary. PATH does not help: CPython 3.8+ calls
    # SetDefaultDllDirectories at startup, which drops PATH from the search
    # order process-wide — and, for the same reason, makes the
    # SetDllDirectoryW that pyrfcx's Rust loader calls a no-op inside a
    # Python process. os.add_dll_directory is the supported mechanism here,
    # and it covers real pyrfc too (which does the same from SAPNWRFC_HOME
    # at import).
    os.add_dll_directory(str(SDK_LIB))
    # LOAD-BEARING — do not remove (measured 2026-07-17; the comment here
    # used to call it "harmless ... aligned as well"). `add_dll_directory`
    # above only governs *CPython's* DLL search, i.e. how pyrfc's .pyd finds
    # sapnwrfc.dll. The SDK then loads icu*57.dll ITSELF, lazily, at
    # RfcOpenConnection — and that lookup ignores added DLL directories and
    # honours PATH. Without this line, `just test-pyrfc-real` hangs on a
    # modal "could not open ICU library" dialog. (pyrfcx survives without it:
    # its Rust loader also calls SetDllDirectoryW, which the ICU lookup does
    # consult.)
    os.environ["PATH"] = os.environ["PATH"] + os.pathsep + str(SDK_LIB)

# The destination and the per-backend expectations, shared with tests/compat.
from tests.compat.config import BACKEND as BACKEND  # noqa: E402  (after the SDK pin)
from tests.compat.config import DEST as DEST  # noqa: E402


@pytest.fixture(scope="class")
def conn() -> Iterator[Connection]:
    """A live SAP connection reused across tests in a class."""
    # Imported lazily: this conftest is also loaded when running tests/compat
    # against *real* pyrfc, in a venv that has no pyrfcx installed.
    from pyrfcx import Connection

    c = Connection(dest=DEST)
    yield c
    c.close()
