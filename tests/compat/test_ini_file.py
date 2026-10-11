"""pyrfc 3.4's set_ini_file_directory and reload_ini_file, measured on real pyrfc 3.4 first.

The ini directory is process-wide SDK state. Every test here restores it to
the repository root, where sapnwrfc.ini lives, so later tests still find
their destination. The SDK keeps the destinations it has read even after it
switches directory, so a test proves a switch with a destination name that
is new in each run.
"""

from __future__ import annotations

import re
import uuid
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from pyrfc import Connection, ExternalRuntimeError, reload_ini_file, set_ini_file_directory
from tests.compat.config import DEST

if TYPE_CHECKING:
    from collections.abc import Iterator

pytestmark = [pytest.mark.live, pytest.mark.compat]

REPO_ROOT = Path(__file__).resolve().parent.parent.parent


@pytest.fixture
def restore_ini_directory() -> Iterator[None]:
    yield
    set_ini_file_directory(str(REPO_ROOT))


def test_switch_directory(tmp_path, restore_ini_directory):
    alias = f"PYRFCX_{uuid.uuid4().hex[:8].upper()}"
    # Unknown destination: the SDK finds no host parameter.
    with pytest.raises(ExternalRuntimeError) as ex:
        Connection(dest=alias)
    assert str(ex.value) == (
        "20 (rc=20): key=RFC_INVALID_PARAMETER, message=Parameter ASHOST, GWHOST, MSHOST, "
        "WSPORT or TCPPORT is missing. [MSG: class=, type=, number=, v1-4:=;;;]"
    )
    text = (REPO_ROOT / "sapnwrfc.ini").read_text()
    (tmp_path / "sapnwrfc.ini").write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={alias}", text))
    assert set_ini_file_directory(str(tmp_path)) is None
    conn = Connection(dest=alias)
    assert conn.alive
    conn.close()


def test_argument_errors(tmp_path):
    for value in (1, b".", tmp_path):
        with pytest.raises(TypeError) as ex:
            set_ini_file_directory(value)
        assert ex.value.args == ("sapnwrfc.ini path is not a string:", value)
    # The directory must hold a sapnwrfc.ini file. An embedded NUL fails
    # the same check, before the SDK.
    for value in (str(tmp_path), "/nonexistent/dir", "a\0b"):
        with pytest.raises(TypeError) as ex:
            set_ini_file_directory(value)
        assert ex.value.args == ("sapnwrfc.ini not found in:", value)


def test_reload_ini_file(tmp_path, restore_ini_directory):
    text = (REPO_ROOT / "sapnwrfc.ini").read_text()
    first, second = (f"PYRFCX_{uuid.uuid4().hex[:8].upper()}" for _ in range(2))
    ini = tmp_path / "sapnwrfc.ini"
    ini.write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={first}", text))
    set_ini_file_directory(str(tmp_path))
    ini.write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={second}", text))
    with pytest.raises(ExternalRuntimeError):
        Connection(dest=second)
    assert reload_ini_file() is None
    # The edited file is read; the destination read before stays known.
    for alias in (second, first):
        conn = Connection(dest=alias)
        assert conn.alive
        conn.close()
    # A missing file is not an error.
    ini.unlink()
    assert reload_ini_file() is None
