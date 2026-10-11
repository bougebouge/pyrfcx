"""Integer overflow and BYTE/XSTRING input errors, pyrfc 3.4 bug-for-bug.

Every expectation here was measured on real pyrfc 3.4 first (`just
test-pyrfc-real`), which makes it pyrfc's and not ours
(anti-circularity). The behaviour-fact collector found both as compat deltas.

pyrfc behaviour pinned here (_cyrfc.pyx fillVariable, fillBytes):
- INT/INT1/INT2 call `RfcSetInt(..., value, ...)` with a C `int` parameter
  and INT8 calls `RfcSetInt8` with a C `long long`. Cython converts the
  Python int at the call, so an out-of-range value raises an OverflowError
  from CPython or from Cython's narrowing; the message depends on the width
  of a C long. It is not a TypeError, so no field name is appended.
- INT1/INT2 values that fit a C `int` reach the SDK, which wraps them
  silently. They must keep wrapping.
- BYTE/XSTRING go through `fillBytes`: `len(value)` first (CPython's
  TypeError for int/None), then a `<char*>` cast that accepts bytes and
  bytearray and raises "expected bytes, <type> found" for anything else.
  Both are TypeErrors, so field names are appended innermost-first.
"""

from __future__ import annotations

import ctypes
from typing import Any

import pytest

from pyrfc import Connection
from tests.compat.config import CONNECTION_PARAMS

pytestmark = [pytest.mark.live, pytest.mark.compat]

# IS_INPUT → ES_OUTPUT echo; ZRAW is BYTE, ZRAWSTRING is XSTRING.
DATATYPES_FM = "ZPYRFCX_FE_DATATYPES"
INT8_FM = "SEWS_REF_DIRECT_TYPES"  # remote-enabled SAP-standard FM with an INT8 parameter

# The message depends on the width of a C long, measured on real pyrfc 3.4.
# Where a C long is as wide as a C int (Windows), the C long conversion
# fails for INT and the long long conversion for INT8. Where a C long is
# 64 bits wide (macOS), the C long conversion succeeds for INT and Cython's
# narrowing to RFC_INT fails, and INT8 goes through the C long conversion.
if ctypes.sizeof(ctypes.c_long) == ctypes.sizeof(ctypes.c_int):
    INT_OVERFLOW = "Python int too large to convert to C long"
    INT8_OVERFLOW = "int too big to convert"
else:
    INT_OVERFLOW = "value too large to convert to RFC_INT"
    INT8_OVERFLOW = "Python int too large to convert to C long"


@pytest.fixture(scope="module")
def client() -> Any:
    c = Connection(**CONNECTION_PARAMS)
    yield c
    c.close()


@pytest.mark.parametrize(
    ("field", "value"),
    [
        ("RFCINT4", 2**31),
        ("RFCINT4", -(2**31) - 1),
        # INT1/INT2 share the C int cast: beyond a C int they overflow too.
        ("RFCINT1", 2**31),
        ("RFCINT2", -(2**31) - 1),
    ],
)
def test_int_beyond_c_int_raises_overflow(client: Any, field: str, value: int) -> None:
    with pytest.raises(OverflowError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT={field: value})
    assert type(ex.value) is OverflowError
    assert ex.value.args == (INT_OVERFLOW,)


@pytest.mark.parametrize("value", [2**63, -(2**63) - 1])
def test_int8_beyond_long_long_raises_overflow(client: Any, value: int) -> None:
    with pytest.raises(OverflowError) as ex:
        client.call(INT8_FM, CV_INT8=value)
    assert type(ex.value) is OverflowError
    assert ex.value.args == (INT8_OVERFLOW,)


def test_int1_int2_within_c_int_wrap_silently(client: Any) -> None:
    echo = client.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT1": 300, "RFCINT2": 70000})[
        "ECHOSTRUCT"
    ]
    assert echo["RFCINT1"] == 300 % 256
    assert echo["RFCINT2"] == 70000 - 65536


@pytest.mark.parametrize(
    ("value", "message"),
    [
        ("DEAD", "expected bytes, str found"),
        (5, "object of type 'int' has no len()"),
        (None, "object of type 'NoneType' has no len()"),
    ],
)
@pytest.mark.parametrize("field", ["ZRAW", "ZRAWSTRING"])
def test_byte_and_xstring_reject_non_bytes(
    client: Any, field: str, value: object, message: str
) -> None:
    with pytest.raises(TypeError) as ex:
        client.call(DATATYPES_FM, IS_INPUT={field: value})
    assert type(ex.value) is TypeError
    assert ex.value.args == (message, field, "IS_INPUT")


def test_xstring_str_in_table_row_names_field_then_table(client: Any) -> None:
    with pytest.raises(TypeError) as ex:
        client.call("STFC_DEEP_TABLE", IMPORT_TAB=[{"I": 1, "XSTR": "DEAD"}])
    assert ex.value.args == ("expected bytes, str found", "XSTR", "IMPORT_TAB")


@pytest.mark.parametrize("field", ["RFCHEX3"])
def test_byte_accepts_bytearray(client: Any, field: str) -> None:
    echo = client.call("STFC_STRUCTURE", IMPORTSTRUCT={field: bytearray(b"\x01\x02\x03")})[
        "ECHOSTRUCT"
    ]
    assert echo[field] == b"\x01\x02\x03"
