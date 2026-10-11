"""Live tests for Connection.describe_function(), Connection.is_valid() and
set_ini_path(), reload_ini_file() and load_crypto_library() — requires the SAP
destination PYRFCX_TEST_DEST (default TEST_NW750).

These are the native primitives behind the facade's
get_function_description, is_valid, set_ini_file_directory, reload_ini_file
and set_cryptolib_path.
"""

from __future__ import annotations

import copy
import pickle
import re
import uuid
from pathlib import Path
from typing import TYPE_CHECKING

import pytest

from pyrfcx import (
    AbapApplicationError,
    Connection,
    FieldDescription,
    FunctionDescription,
    ParameterDescription,
    RfcDirection,
    RfcError,
    RfcType,
    TypeDescription,
    load_crypto_library,
    reload_ini_file,
    set_ini_path,
)
from tests.conftest import DEST
from tests.isolated import crypto_library_path, run_isolated

if TYPE_CHECKING:
    from collections.abc import Iterator

pytestmark = pytest.mark.live

REPO_ROOT = Path(__file__).resolve().parent.parent


class TestDescribeFunction:
    def test_full_descriptor(self, conn: Connection) -> None:
        desc = conn.describe_function("STFC_STRUCTURE")
        assert isinstance(desc, FunctionDescription)
        assert desc.name == "STFC_STRUCTURE"
        assert [p.name for p in desc.parameters] == [
            "ECHOSTRUCT",
            "RESPTEXT",
            "IMPORTSTRUCT",
            "RFCTABLE",
        ]
        assert desc.parameters[1] == ParameterDescription(
            name="RESPTEXT",
            type=RfcType.CHAR,
            direction=RfcDirection.EXPORT,
            nuc_length=255,
            uc_length=510,
            decimals=0,
            default_value="",
            text="Exporting response message",
            optional=False,
            type_description=None,
        )
        table = desc.parameters[3]
        assert (table.type, table.direction) == (RfcType.TABLE, RfcDirection.TABLES)
        line = table.type_description
        assert isinstance(line, TypeDescription)
        assert (line.name, line.nuc_length, line.uc_length, len(line.fields)) == (
            "RFCTEST",
            144,
            264,
            12,
        )
        assert line.fields[4] == FieldDescription(
            name="RFCCHAR4",
            type=RfcType.CHAR,
            nuc_length=4,
            nuc_offset=13,
            uc_length=8,
            uc_offset=14,
            decimals=0,
            type_description=None,
        )

        count = conn.describe_function("ZPYRFCX_FE_DATATYPES").parameters[2]
        assert (count.name, count.type, count.default_value, count.optional) == (
            "IV_COUNT",
            RfcType.INT,
            "0",
            True,
        )

    def test_errors(self, conn: Connection) -> None:
        # The name goes to the SDK verbatim; the SDK does not upper-case it.
        with pytest.raises(AbapApplicationError) as ex:
            conn.describe_function("stfc_connection")
        assert (ex.value.key, ex.value.sdk_function) == ("FU_NOT_FOUND", "RfcGetFunctionDesc")
        with pytest.raises(RfcError) as empty:
            conn.describe_function("")
        assert empty.value.key == "RFC_INVALID_PARAMETER"
        assert not conn.closed

    def test_closed(self) -> None:
        c = Connection(dest=DEST)
        assert c.is_valid() is True
        c.close()
        assert c.is_valid() is False
        with pytest.raises(ValueError, match="closed"):
            c.describe_function("RFC_PING")


def test_unknown_sdk_code_is_a_pseudo_member() -> None:
    # No offline SDK defines code 99 for a type or a direction.
    t = RfcType(99)
    assert isinstance(t, RfcType)
    assert t is RfcType(99)
    assert (t == 99, int(t), t.value, t.name) == (True, 99, 99, "UNKNOWN_99")
    assert repr(t) == "<RfcType.UNKNOWN_99: 99>"
    assert pickle.loads(pickle.dumps(t)) is t
    assert copy.copy(t) is t
    assert copy.deepcopy(t) is t
    # Not a listed member: iteration and name lookup do not see it.
    assert t not in list(RfcType)
    assert "UNKNOWN_99" not in RfcType.__members__
    d = RfcDirection(99)
    assert (d is RfcDirection(99), d == 99, d.name) == (True, True, "UNKNOWN_99")
    # Known codes still give the declared members; non-int values still fail.
    assert RfcType(6) is RfcType.NUMC
    with pytest.raises(ValueError):
        RfcType("6")  # type: ignore[arg-type]


def test_embedded_nul_in_a_path(conn: Connection) -> None:
    # Rejected before any SDK call, so the ini directory and the crypto
    # library path stay as they were.
    with pytest.raises(ValueError, match="embedded null character"):
        set_ini_path("a\0b")
    with pytest.raises(ValueError, match="embedded null character"):
        load_crypto_library("a\0b")
    with Connection(dest=DEST) as c:
        assert c.is_valid()
    assert conn.is_valid()


@pytest.fixture
def restore_ini_path() -> Iterator[None]:
    yield
    set_ini_path(REPO_ROOT)


def test_set_ini_path(tmp_path: Path, restore_ini_path: None) -> None:
    # The SDK accepts a directory that does not exist.
    set_ini_path(tmp_path / "missing")
    text = (REPO_ROOT / "sapnwrfc.ini").read_text()
    first, second = (f"PYRFCX_{uuid.uuid4().hex[:8].upper()}" for _ in range(2))
    ini = tmp_path / "sapnwrfc.ini"
    ini.write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={first}", text))
    set_ini_path(tmp_path)
    with Connection(dest=first) as c:
        assert c.is_valid()
    # Setting the directory the SDK already has does not read the file again.
    ini.write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={second}", text))
    set_ini_path(str(tmp_path))
    with pytest.raises(RfcError, match="ASHOST"):
        Connection(dest=second)


def test_reload_ini_file(tmp_path: Path, restore_ini_path: None) -> None:
    text = (REPO_ROOT / "sapnwrfc.ini").read_text()
    first, second = (f"PYRFCX_{uuid.uuid4().hex[:8].upper()}" for _ in range(2))
    ini = tmp_path / "sapnwrfc.ini"
    ini.write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={first}", text))
    set_ini_path(tmp_path)
    ini.write_text(re.sub(rf"(?im)^DEST={DEST}$", f"DEST={second}", text))
    reload_ini_file()
    # The edited file is read; the destination read before stays known.
    for alias in (second, first):
        with Connection(dest=alias) as c:
            assert c.is_valid()
    # A missing file is not an error.
    ini.unlink()
    reload_ini_file()


@pytest.mark.skipif(
    not crypto_library_path(), reason="SNC_LIB (the SAP crypto library path) is not set"
)
def test_load_crypto_library(tmp_path: Path) -> None:
    # Process-wide and final: runs in its own process, never in this one.
    lib = crypto_library_path()
    other = tmp_path / "other.lib"
    other.write_text("")
    script = f"""
        import json
        import pyrfcx

        results = []
        def attempt(path):
            try:
                results.append(repr(pyrfcx.load_crypto_library(path)))
            except pyrfcx.RfcError as e:
                results.append(f"{{type(e).__name__}} {{e.code}} {{e.key}}: {{e.message}}")
            except ValueError:
                results.append("ValueError")

        attempt("a\\0b")  # rejected before the SDK: the path stays unset
        attempt("")  # rejected, and the path stays unset
        attempt({lib!r})
        attempt({lib!r})
        attempt({str(other)!r})
        with pyrfcx.Connection(dest={DEST!r}) as c:
            results.append(c.is_valid())
        print(json.dumps(results))
    """
    assert run_isolated(script) == [
        "ValueError",
        "RfcError 19 RFC_ILLEGAL_STATE: Cannot set empty path to crypto library.",
        "None",
        "None",
        "RfcError 20 RFC_INVALID_PARAMETER: Cannot set path to crypto library. "
        f'It is already set to "{lib}"',
        True,
    ]
