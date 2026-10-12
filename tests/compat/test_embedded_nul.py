"""An embedded NUL in a name or setting, pyrfc 3.4 bug-for-bug.

Every expectation here was measured on real pyrfc 3.4 first (`just
test-pyrfc-real`), which makes it pyrfc's and not ours
(anti-circularity).

pyrfc hands the SDK a NUL-terminated copy of every connection parameter,
function name, parameter name, field name and `not_requested` name
(`fillString`), so the SDK reads each one only up to the first NUL. A name
that is valid before the NUL works; one that is not fails as the cut name.
Native pyrfcx raises ValueError for all of them instead.

pyrfc also sends a CHAR, STRING or UTCLONG value only up to its first NUL
(it passes the length with `strlenU`). Other values keep the NUL: NUMC and the decimal
types fail pyrfc's own check on the whole value, DATE and TIME go to the SDK
with their fixed width, and bytes go with their real length. Native pyrfcx
sends every value whole.
"""

from __future__ import annotations

from typing import Any

import pytest

from pyrfc import ABAPApplicationError, Connection, ExternalRuntimeError
from tests.compat.config import (
    CONNECTION_PARAMS,
    DEST,
    requires_utclong,
    skip_if_sdk_accepts_unknown_inactive_name,
)

pytestmark = [pytest.mark.live, pytest.mark.compat]


@pytest.fixture(scope="module")
def client() -> Any:
    c = Connection(**CONNECTION_PARAMS)
    yield c
    c.close()


def test_connection_parameters_are_cut_at_the_nul() -> None:
    for params in ({"dest": DEST + "\0junk"}, {"dest\0junk": DEST}):
        c = Connection(**params)
        assert c.alive
        c.close()
    with pytest.raises(ExternalRuntimeError) as exc_info:
        Connection(dest="NOPE\0" + DEST)
    assert exc_info.value.args == (
        "Parameter ASHOST, GWHOST, MSHOST, WSPORT or TCPPORT is missing.",
    )


def test_names_are_cut_at_the_nul(client: Any) -> None:
    assert client.call("STFC_CONNECTION\0junk", REQUTEXT="hi")["ECHOTEXT"] == "hi"
    assert client.call("STFC_CONNECTION", **{"REQUTEXT\0junk": "hi"})["ECHOTEXT"] == "hi"
    result = client.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4\0junk": "ABCD"})
    assert result["ECHOSTRUCT"]["RFCCHAR4"] == "ABCD"
    result = client.call("STFC_STRUCTURE", RFCTABLE=[{"RFCCHAR4\0junk": "ABCD"}])
    assert result["RFCTABLE"][0]["RFCCHAR4"] == "ABCD"
    # The cut name deactivates RESPTEXT; the input is still sent.
    result = client.call(
        "STFC_CONNECTION", options={"not_requested": ["RESPTEXT\0junk"]}, REQUTEXT="hi"
    )
    assert (result["ECHOTEXT"], result["RESPTEXT"]) == ("hi", "")
    assert client.get_function_description("STFC_CONNECTION\0junk").name == "STFC_CONNECTION"
    # The input check finds the field by the cut name and appends the
    # descriptor's spelling.
    with pytest.raises(TypeError) as type_info:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT1\0junk": "x"})
    assert type_info.value.args == (
        "an integer required, received",
        "x",
        "of type",
        str,
        "RFCINT1",
        "IMPORTSTRUCT",
    )


def test_a_cut_name_that_is_unknown_fails_as_the_cut_name(client: Any) -> None:
    skip_if_sdk_accepts_unknown_inactive_name()
    for call in (
        lambda: client.call("STFC_CONNECTION", **{"NOPE\0REQUTEXT": "hi"}),
        lambda: client.call("STFC_STRUCTURE", IMPORTSTRUCT={"NOPE\0RFCCHAR4": "ABCD"}),
        lambda: client.call("STFC_STRUCTURE", RFCTABLE=[{"NOPE\0RFCCHAR4": "ABCD"}]),
        lambda: client.call(
            "STFC_CONNECTION", options={"not_requested": ["NOPE\0ECHOTEXT"]}, REQUTEXT="hi"
        ),
    ):
        with pytest.raises(ExternalRuntimeError) as exc_info:
            call()
        assert exc_info.value.args == ("field 'NOPE' not found",)
    for call in (
        lambda: client.call("NOPE\0STFC_CONNECTION", REQUTEXT="hi"),
        lambda: client.get_function_description("NOPE\0STFC_CONNECTION"),
    ):
        with pytest.raises(ABAPApplicationError) as abap_info:
            call()
        assert abap_info.value.args == ("ID:FL Type:E Number:046 NOPE",)
    assert client.alive


def test_char_and_string_values_are_cut_at_the_nul(client: Any) -> None:
    assert client.call("STFC_CONNECTION", REQUTEXT="hi\0x")["ECHOTEXT"] == "hi"
    for value, expected in (("A\0B", "A"), ("AB\0", "AB"), ("\0", ""), ("ABCDE\0F", "ABCD")):
        result = client.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": value})
        assert result["ECHOSTRUCT"]["RFCCHAR4"] == expected
        result = client.call("STFC_STRUCTURE", RFCTABLE=[{"RFCCHAR4": value}])
        assert result["RFCTABLE"][0]["RFCCHAR4"] == expected
    for value, expected in (("A\0B", "A"), ("AB\0", "AB"), ("\0", "")):
        result = client.call("STFC_DEEP_STRUCTURE", IMPORTSTRUCT={"C": value, "STR": value})
        assert (result["ECHOSTRUCT"]["C"], result["ECHOSTRUCT"]["STR"]) == (expected, expected)
        result = client.call("STFC_DEEP_TABLE", IMPORT_TAB=[{"STR": value}])
        assert result["EXPORT_TAB"][0]["STR"] == expected


def test_checked_values_are_checked_whole(client: Any) -> None:
    cases = (
        (
            "ZPYRFCX_FE_DATATYPES",
            "IS_INPUT",
            "ZNUMC",
            "12\x0034",
            "a numeric string is required, received",
        ),
        (
            "ZPYRFCX_FE_DATATYPES",
            "IS_INPUT",
            "ZDEC",
            "1\x002",
            "a decimal value required, received",
        ),
        (
            "ZPYRFCX_FE_DATATYPES",
            "IS_INPUT",
            "ZFLTP",
            "1\x002",
            "a decimal value required, received",
        ),
        (
            "ZPYRFCX_FE_DATATYPES",
            "IS_INPUT",
            "ZDECF16_MIN",
            "1\x002",
            "a decimal value required, received",
        ),
        (
            "ZPYRFCX_FE_DATATYPES",
            "IS_INPUT",
            "ZDECF34_MIN",
            "1\x002",
            "a decimal value required, received",
        ),
        (
            "STFC_STRUCTURE",
            "IMPORTSTRUCT",
            "RFCDATE",
            "2026\x00101",
            "date value required, received",
        ),
        (
            "STFC_STRUCTURE",
            "IMPORTSTRUCT",
            "RFCTIME",
            "12\x00300",
            "time value required, received",
        ),
    )
    for function, param, field, value, message in cases:
        with pytest.raises(TypeError) as exc_info:
            client.call(function, **{param: {field: value}})
        assert exc_info.value.args == (message, value, "of type", str, field, param)


def test_date_time_and_bytes_keep_the_nul(client: Any) -> None:
    unchecked = Connection(config={"check_date": False, "check_time": False}, **CONNECTION_PARAMS)
    try:
        line = {"RFCDATE": "2026\x00101", "RFCTIME": "12\x00300"}
        result = unchecked.call("STFC_STRUCTURE", IMPORTSTRUCT=line, RFCTABLE=[line])
        for echoed in (result["ECHOSTRUCT"], result["RFCTABLE"][0]):
            assert (echoed["RFCDATE"], echoed["RFCTIME"]) == ("2026\x00101", "12\x00300")
    finally:
        unchecked.close()
    result = client.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZRAW": b"A\0B"})
    assert result["ES_OUTPUT"]["ZRAW"] == b"A\0B" + bytes(14)
    result = client.call("STFC_DEEP_STRUCTURE", IMPORTSTRUCT={"XSTR": b"A\0B"})
    assert result["ECHOSTRUCT"]["XSTR"] == b"A\0B"


@requires_utclong
def test_utclong_value_is_cut_at_the_nul(client: Any) -> None:
    # Measured on real pyrfc 3.4: a UTCLONG value goes to the SDK only up to
    # its first NUL, like CHAR and STRING.
    result = client.call(
        "ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZUTCLONG": "2024-01-31T12:34:56,1234567\0junk"}
    )
    assert result["ES_OUTPUT"]["ZUTCLONG"] == "2024-01-31T12:34:56.1234567"
    with pytest.raises(ExternalRuntimeError) as exc_info:
        client.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZUTCLONG": "2024-01-31\0T12:34:56,1234567"})
    assert exc_info.value.key == "RFC_CONVERSION_FAILURE"
    message = "Cannot convert 2024-01-31 to RFCTYPE_UTCLONG : illegal format"
    assert exc_info.value.args == (message,)
