"""The output config keys `rstrip`, `dtime` and `return_import_params`, as pyrfc 3.4 does them.

Every expectation was measured on real pyrfc 3.4 first (`just test-pyrfc-real`),
which is what makes them pyrfc's and not ours (anti-circularity).

pyrfc behaviour pinned here (_cyrfc.pyx wrapVariable, wrapString,
functionContainerGet, Connection.__init__/call):
- `rstrip` controls CHAR and BCD/DECF only. STRING and NUMC are always
  stripped, DATE/TIME never. The strip is `bytes.rstrip()` on UTF-8, so only
  ASCII whitespace goes; U+3000 and U+00A0 stay.
- `dtime` sends DATE/TIME through `datetime.strptime`. Initial DATE is None,
  but initial TIME is midnight. A blank or invalid value raises strptime's
  ValueError. Input is not affected: date/time objects are always accepted.
- `return_import_params` also returns IMPORT parameters, in descriptor order,
  with whatever the container holds after the call.
- The flags are copied at the open: a later change to `options` does nothing.
- UTCLONG output is stripped and gets a dot at index 19, whatever `rstrip`
  says (TEST_NW757 only: release 754+). An empty UTCLONG input goes to the
  SDK, so the answer depends on the SDK patch level.
"""

from __future__ import annotations

import datetime
from decimal import Decimal

import pytest

from pyrfc import Connection, ExternalRuntimeError
from tests.compat.config import CONNECTION_PARAMS, requires_utclong
from tests.test_sdk_pin import vendored_dll_version

pytestmark = [pytest.mark.live, pytest.mark.compat]


def test_rstrip_per_type():
    for rstrip in (True, False):
        conn = Connection(config={"rstrip": rstrip}, **CONNECTION_PARAMS)

        # CHAR follows the flag, at full field width.
        echo = conn.call("STFC_CONNECTION", REQUTEXT="Hällo \t")["ECHOTEXT"]
        assert echo == ("Hällo" if rstrip else "Hällo \t".ljust(255))
        res = conn.call("STFC_DEEP_STRUCTURE", IMPORTSTRUCT={"STR": "ab  \t", "C": "x"})
        assert res["ECHOSTRUCT"]["C"] == ("x" if rstrip else "x".ljust(10))
        # STRING is always stripped.
        assert res["ECHOSTRUCT"]["STR"] == "ab"

        out = conn.call(
            "ZPYRFCX_FE_DATATYPES",
            IS_INPUT={"ZNUMC": "12", "ZSHLP_MAT1": "ab", "ZDEC": Decimal("1.5")},
        )["ES_OUTPUT"]
        assert out["ZSHLP_MAT1"] == ("ab" if rstrip else "ab".ljust(40))
        # NUMC and BCD show no difference: no trailing blanks to strip.
        assert out["ZNUMC"] == "0000000012"
        assert out["ZDEC"] == Decimal("1.50")

        # DATE is never stripped; initial stays "".
        struct = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": "a"})["ECHOSTRUCT"]
        assert struct["RFCCHAR4"] == ("a" if rstrip else "a   ")
        assert struct["RFCDATE"] == ""
        assert struct["RFCTIME"] == "000000"

        # Only ASCII whitespace is stripped.
        for text, stripped in (
            ("a\u3000", "a\u3000"),
            ("a\u00a0 ", "a\u00a0"),
            ("a\x1c", "a\x1c"),
            ("a\x0b\x0c", "a"),
        ):
            echo = conn.call("STFC_CONNECTION", REQUTEXT=text)["ECHOTEXT"]
            assert echo == (stripped if rstrip else text.ljust(255))
        conn.close()


def test_dtime_output():
    conn = Connection(
        config={"dtime": True, "check_date": False, "check_time": False}, **CONNECTION_PARAMS
    )
    struct = conn.call(
        "STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATE": "20240102", "RFCTIME": "030405"}
    )["ECHOSTRUCT"]
    assert type(struct["RFCDATE"]) is datetime.date
    assert struct["RFCDATE"] == datetime.date(2024, 1, 2)
    assert type(struct["RFCTIME"]) is datetime.time
    assert struct["RFCTIME"] == datetime.time(3, 4, 5)

    # Initial values: DATE "00000000" is None, TIME "000000" is midnight.
    # An empty-string input is skipped, so it reads back initial too.
    for inputs in ({"RFCINT4": 1}, {"RFCDATE": "", "RFCTIME": ""}):
        struct = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=inputs)["ECHOSTRUCT"]
        assert struct["RFCDATE"] is None
        assert struct["RFCTIME"] == datetime.time(0, 0)

    # Blank or invalid values: strptime's ValueError, uncaught.
    for inputs, message in (
        ({"RFCDATE": "        "}, "time data '        ' does not match format '%Y%m%d'"),
        ({"RFCTIME": "      "}, "time data '      ' does not match format '%H%M%S'"),
        ({"RFCDATE": "20161232"}, "unconverted data remains: 2"),
        ({"RFCDATE": "2016123"}, "unconverted data remains: \x00"),
    ):
        with pytest.raises(ValueError) as ex:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT=inputs)
        assert ex.value.args == (message,)
    assert conn.alive
    conn.close()


def test_date_time_objects_input_with_and_without_dtime():
    inputs = {"RFCDATE": datetime.date(2020, 1, 2), "RFCTIME": datetime.time(1, 2, 3)}
    for dtime, date, time in (
        (True, datetime.date(2020, 1, 2), datetime.time(1, 2, 3)),
        (False, "20200102", "010203"),
    ):
        conn = Connection(config={"dtime": dtime}, **CONNECTION_PARAMS)
        struct = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=inputs)["ECHOSTRUCT"]
        assert struct["RFCDATE"] == date
        assert struct["RFCTIME"] == time
        conn.close()


def test_return_import_params():
    conn = Connection(config={"return_import_params": True}, **CONNECTION_PARAMS)

    # IMPORT parameters come in descriptor order, CHAR stripped as usual.
    res = conn.call("STFC_CONNECTION", REQUTEXT="hi  ")
    assert list(res) == ["ECHOTEXT", "RESPTEXT", "REQUTEXT"]
    assert res["REQUTEXT"] == "hi"

    res = conn.call(
        "STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": 5, "RFCDATE": ""}, RFCTABLE=[{"RFCINT4": 7}]
    )
    assert list(res) == ["ECHOSTRUCT", "RESPTEXT", "IMPORTSTRUCT", "RFCTABLE"]
    assert res["IMPORTSTRUCT"]["RFCINT4"] == 5
    # The skipped empty DATE reads back initial.
    assert res["IMPORTSTRUCT"]["RFCDATE"] == ""
    assert len(res["RFCTABLE"]) == 2
    assert list(conn.call("STFC_STRUCTURE")) == [
        "ECHOSTRUCT",
        "RESPTEXT",
        "IMPORTSTRUCT",
        "RFCTABLE",
    ]

    # CHANGING parameters were already returned; only IMPORT is added.
    res = conn.call("STFC_CHANGING", START_VALUE=2, COUNTER=3)
    assert list(res.items()) == [("COUNTER", 4), ("RESULT", 5), ("START_VALUE", 2)]

    # An IMPORT not passed reads back its initial value, not the ABAP default.
    res = conn.call("ZPYRFCX_FE_DATATYPES")
    assert list(res) == ["ES_OUTPUT", "IS_INPUT", "IV_COUNT"]
    assert res["IV_COUNT"] == 0
    assert res["IS_INPUT"]["ZNUMC"] == "0000000000"
    conn.close()


def test_return_import_params_with_not_requested():
    conn = Connection(config={"return_import_params": True}, **CONNECTION_PARAMS)
    # A deactivated scalar with a value: the setter re-activates it.
    res = conn.call("STFC_CONNECTION", {"not_requested": ["REQUTEXT"]}, REQUTEXT="hi")
    assert res["REQUTEXT"] == "hi"
    assert res["ECHOTEXT"] == "hi"
    # Without a value it is returned initial.
    res = conn.call("STFC_CONNECTION", {"not_requested": ["REQUTEXT"]})
    assert res["REQUTEXT"] == ""
    assert res["ECHOTEXT"] == ""
    # A deactivated structure is not sent, but the local value is returned.
    res = conn.call(
        "STFC_STRUCTURE", {"not_requested": ["IMPORTSTRUCT"]}, IMPORTSTRUCT={"RFCINT4": 5}
    )
    assert res["IMPORTSTRUCT"]["RFCINT4"] == 5
    assert res["ECHOSTRUCT"]["RFCINT4"] == 0
    conn.close()


def test_options_change_after_open_has_no_effect():
    conn = Connection(**CONNECTION_PARAMS)
    conn.options["rstrip"] = False
    conn.options["return_import_params"] = True
    conn.options["dtime"] = True
    res = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": "a", "RFCDATE": "20240102"})
    assert list(res) == ["ECHOSTRUCT", "RESPTEXT", "RFCTABLE"]
    assert res["ECHOSTRUCT"]["RFCCHAR4"] == "a"
    assert res["ECHOSTRUCT"]["RFCDATE"] == "20240102"
    conn.close()


@requires_utclong
def test_utclong_output():
    # pyrfc reads the SDK's 27-char form, strips it and swaps index 19 for a
    # dot. The SDK always returns 27 chars, the initial value too, so the
    # separator swap is the only visible change. `rstrip` does not affect it.
    for rstrip in (True, False):
        conn = Connection(config={"rstrip": rstrip}, **CONNECTION_PARAMS)
        for field, expected in (
            ({}, "0000-00-00T00:00:00.0000000"),
            ({"ZUTCLONG": "2024-01-31T12:34:56,1234567"}, "2024-01-31T12:34:56.1234567"),
            ({"ZUTCLONG": "2024-01-31T12:34:56.1234567"}, "2024-01-31T12:34:56.1234567"),
        ):
            out = conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZNUMC": "1", **field})
            assert out["ES_OUTPUT"]["ZUTCLONG"] == expected
        # An empty value: the SDK sets the initial value from patch level 18
        # and rejects the value below it. pyrfc passes "" to the SDK as is.
        empty = {"ZNUMC": "1", "ZUTCLONG": ""}
        if vendored_dll_version()[2] >= 18:
            out = conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT=empty)
            assert out["ES_OUTPUT"]["ZUTCLONG"] == "0000-00-00T00:00:00.0000000"
        else:
            with pytest.raises(ExternalRuntimeError) as ex:
                conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT=empty)
            assert str(ex.value) == (
                "22 (rc=22): key=RFC_CONVERSION_FAILURE, message=Cannot convert  to "
                "RFCTYPE_UTCLONG : illegal format [MSG: class=, type=, number=, v1-4:=;;;]"
            )
            assert conn.alive
        # The SDK rejects a value without the seven fraction digits.
        with pytest.raises(ExternalRuntimeError) as ex:
            conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZUTCLONG": "2024-01-31T12:34:56"})
        assert ex.value.key == "RFC_CONVERSION_FAILURE"
        conn.close()
