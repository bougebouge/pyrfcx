# SPDX-FileCopyrightText: 2013 SAP SE Srdjan Boskovic <srdjan.boskovic@sap.com>
# SPDX-FileCopyrightText: 2026 the pyrfcx contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Adapted from pyrfc 3.4. This file is modified; the docstring lists the changes.

"""Adapted from pyrfc 3.4 tests/test_datatypes.py (https://github.com/SAP/PyRFC).

Changes vs the original:
- /COE/RBP_FE_DATATYPES → ZPYRFCX_FE_DATATYPES ($TMP recreation on TEST_NW750,
  same interface: IS_INPUT/IV_COUNT → ES_OUTPUT echo; see plan + docs).
- BAPI_USER_GET_DETAIL queries TEST_USER instead of "demo" (not on our system).
- UTCLONG tests dropped: release 750 has no UTCLONG; locale-radix tests
  dropped: no-ops on win32 in the original already.

Input-validation assertions are the originals, unmodified: the
compat facade mirrors pyrfc's client-side `fillVariable` checks and raises
the same TypeErrors with the same `.args` tuples, so pyrfc's own expectations
pin our behavior directly (the anti-circularity rule — these come from
pyrfc's suite, never from our compat profile).
"""

from __future__ import annotations

from decimal import Decimal

import pytest

from pyrfc import Connection
from tests.compat.config import (
    BYTEARRAY_TEST,
    BYTES_TEST,
    CONNECTION_PARAMS,
    RFC_MATH,
    TEST_USER,
    ABAP_to_python_date,
    ABAP_to_python_time,
    python_to_ABAP_date,
    python_to_ABAP_time,
    requires_test_user,
)

pytestmark = [pytest.mark.live, pytest.mark.compat]

DATATYPES_FM = "ZPYRFCX_FE_DATATYPES"  # $TMP echo twin of /COE/RBP_FE_DATATYPES


@pytest.fixture(scope="module")
def client():
    """Module-scoped connection, mirroring pyrfc's module-level `client`."""
    c = Connection(**CONNECTION_PARAMS)
    yield c
    c.close()


def test_structure_rejects_non_dict(client):
    with pytest.raises(TypeError) as ex:
        IMPORTSTRUCT = {"RFCINT1": "1"}
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=[IMPORTSTRUCT])
    error = ex.value
    # Note the shape: 3 args, carrying str(type(value)) and NOT the value —
    # asymmetric with the scalar arms' 4-arg template. pyrfc's, kept verbatim.
    assert error.args[0] == "dictionary required for structure parameter, received"
    assert error.args[1] == "<class 'list'>"
    assert error.args[2] == "IMPORTSTRUCT"


def test_table_rejects_non_list(client):
    with pytest.raises(TypeError) as ex:
        IMPORTSTRUCT = {"RFCINT1": "1"}
        client.call("STFC_STRUCTURE", RFCTABLE=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "list required for table parameter, received"
    assert error.args[1] == "<class 'dict'>"
    assert error.args[2] == "RFCTABLE"


def test_basic_datatypes(client):
    INPUTS = [
        {
            "ZFLTP": 0.123456789,
            "ZDEC": 12345.67,
            "ZCURR": 1234.56,
            "ZQUAN": 12.3456,
            "ZQUAN_SIGN": -12.345,
        },
        {
            "ZFLTP": Decimal("0.123456789"),
            "ZDEC": Decimal("12345.67"),
            "ZCURR": Decimal("1234.56"),
            "ZQUAN": Decimal("12.3456"),
            "ZQUAN_SIGN": Decimal("-12.345"),
        },
        {
            "ZFLTP": "0.123456789",
            "ZDEC": "12345.67",
            "ZCURR": "1234.56",
            "ZQUAN": "12.3456",
            "ZQUAN_SIGN": "-12.345",
        },
    ]

    for is_input in INPUTS:
        res = client.call(DATATYPES_FM, IS_INPUT=is_input)["ES_OUTPUT"]
        for attr in is_input:
            in_value = is_input[attr]
            out_value = res[attr]
            if attr == "ZFLTP":
                assert isinstance(out_value, float)
            else:
                assert isinstance(out_value, Decimal)
            if type(in_value) != type(out_value):  # noqa: E721 — verbatim pyrfc test logic
                assert str(in_value) == str(out_value)
            else:
                assert in_value == out_value


def test_string_unicode(client):
    hello = "Hällo SAP!"
    res = client.call("STFC_CONNECTION", REQUTEXT=hello)["ECHOTEXT"]
    assert res == hello

    hello = "01234" * 51
    res = client.call("STFC_CONNECTION", REQUTEXT=hello)["ECHOTEXT"]
    assert res == hello

    unicode_test = [
        "string",
        "四周远处都能望见",
        "\U0001f4aa",
        "\u0001\uf4aa",
        "a\xacሴ€\U0001f4aa",
    ]

    for ch in unicode_test:
        is_input = {"ZSHLP_MAT1": ch}
        res = client.call(DATATYPES_FM, IS_INPUT=is_input)["ES_OUTPUT"]
        assert is_input["ZSHLP_MAT1"] == res["ZSHLP_MAT1"]


@requires_test_user
def test_date_output(client):
    # Was xfailed for the empty-decimal crash (the SDK returning an empty decimal string for
    # UCLASS.COUNTRY_SURCHARGE, which pyrfc 3.4 crashes on identically).
    # That is 753-PL-1533 behavior, seen only while we were accidentally
    # loading System32's DLL; the vendored 7.50 SDK reads the field fine.
    # The empty-decimal handling stays in place — it is real on 7.53.
    lm = client.call("BAPI_USER_GET_DETAIL", USERNAME=TEST_USER)["LASTMODIFIED"]
    assert len(lm["MODDATE"]) > 0
    assert len(lm["MODTIME"]) > 0


def test_min_max_positive(client):
    IS_INPUT = {
        "ZFLTP_MIN": RFC_MATH["FLOAT"]["POS"]["MIN"],
        "ZFLTP_MAX": RFC_MATH["FLOAT"]["POS"]["MAX"],
        "ZDECF16_MIN": RFC_MATH["DECF16"]["POS"]["MIN"],
        "ZDECF16_MAX": RFC_MATH["DECF16"]["POS"]["MAX"],
        "ZDECF34_MIN": RFC_MATH["DECF34"]["POS"]["MIN"],
        "ZDECF34_MAX": RFC_MATH["DECF34"]["POS"]["MAX"],
    }

    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]

    assert isinstance(output["ZFLTP_MIN"], float)
    assert isinstance(output["ZFLTP_MAX"], float)
    assert isinstance(output["ZDECF16_MIN"], Decimal)
    assert isinstance(output["ZDECF16_MAX"], Decimal)
    assert isinstance(output["ZDECF34_MAX"], Decimal)
    assert isinstance(output["ZDECF16_MIN"], Decimal)

    assert float(IS_INPUT["ZFLTP_MIN"]) == output["ZFLTP_MIN"]
    assert float(IS_INPUT["ZFLTP_MAX"]) == output["ZFLTP_MAX"]
    assert Decimal(IS_INPUT["ZDECF16_MIN"]) == output["ZDECF16_MIN"]
    assert Decimal(IS_INPUT["ZDECF16_MAX"]) == output["ZDECF16_MAX"]
    assert Decimal(IS_INPUT["ZDECF34_MAX"]) == output["ZDECF34_MAX"]


def test_min_max_negative(client):
    IS_INPUT = {
        "ZFLTP_MIN": RFC_MATH["FLOAT"]["NEG"]["MIN"],
        "ZFLTP_MAX": RFC_MATH["FLOAT"]["NEG"]["MAX"],
        "ZDECF16_MIN": RFC_MATH["DECF16"]["NEG"]["MIN"],
        "ZDECF16_MAX": RFC_MATH["DECF16"]["NEG"]["MAX"],
        "ZDECF34_MIN": RFC_MATH["DECF34"]["NEG"]["MIN"],
        "ZDECF34_MAX": RFC_MATH["DECF34"]["NEG"]["MAX"],
    }

    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]

    assert isinstance(output["ZFLTP_MIN"], float)
    assert isinstance(output["ZFLTP_MAX"], float)
    assert isinstance(output["ZDECF16_MIN"], Decimal)
    assert isinstance(output["ZDECF16_MAX"], Decimal)
    assert isinstance(output["ZDECF34_MAX"], Decimal)

    assert float(IS_INPUT["ZFLTP_MIN"]) == output["ZFLTP_MIN"]
    assert float(IS_INPUT["ZFLTP_MAX"]) == output["ZFLTP_MAX"]
    assert Decimal(IS_INPUT["ZDECF16_MIN"]) == output["ZDECF16_MIN"]
    assert Decimal(IS_INPUT["ZDECF16_MAX"]) == output["ZDECF16_MAX"]
    assert Decimal(IS_INPUT["ZDECF34_MAX"]) == output["ZDECF34_MAX"]


def test_bcd_floats_accept_floats(client):
    IS_INPUT = {
        "ZFLTP": 0.123456789,
        "ZDEC": 12345.67,
        "ZDECF16_MIN": 12345.67,
        "ZDECF34_MIN": 12345.67,
        "ZCURR": 1234.56,
        "ZQUAN": 12.3456,
        "ZQUAN_SIGN": -12.345,
    }

    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]
    assert isinstance(output["ZFLTP"], float)
    assert IS_INPUT["ZFLTP"] == output["ZFLTP"]

    for field in ("ZDEC", "ZDECF16_MIN", "ZDECF34_MIN", "ZCURR", "ZQUAN", "ZQUAN_SIGN"):
        assert isinstance(output[field], Decimal)
        assert str(IS_INPUT[field]) == str(output[field])
        assert IS_INPUT[field] == float(output[field])


def test_bcd_floats_accept_strings(client):
    IS_INPUT = {
        "ZFLTP": "0.123456789",
        "ZDEC": "12345.67",
        "ZDECF16_MIN": "12345.67",
        "ZDECF34_MIN": "12345.67",
        "ZCURR": "1234.56",
        "ZQUAN": "12.3456",
        "ZQUAN_SIGN": "-12.345",
    }

    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]
    assert isinstance(output["ZFLTP"], float)
    assert float(IS_INPUT["ZFLTP"]) == output["ZFLTP"]

    for field in ("ZDEC", "ZDECF16_MIN", "ZDECF34_MIN", "ZCURR", "ZQUAN", "ZQUAN_SIGN"):
        assert isinstance(output[field], Decimal)
        assert IS_INPUT[field] == str(output[field])


def test_bcd_floats_accept_decimals(client):
    IS_INPUT = {
        "ZFLTP": Decimal("0.123456789"),
        "ZDEC": Decimal("12345.67"),
        "ZDECF16_MIN": Decimal("12345.67"),
        "ZDECF34_MIN": Decimal("12345.67"),
        "ZCURR": Decimal("1234.56"),
        "ZQUAN": Decimal("12.3456"),
        "ZQUAN_SIGN": Decimal("-12.345"),
    }

    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]
    assert isinstance(output["ZFLTP"], float)
    assert IS_INPUT["ZFLTP"] == Decimal(str(output["ZFLTP"]))

    for field in ("ZDEC", "ZDECF16_MIN", "ZDECF34_MIN", "ZCURR", "ZQUAN", "ZQUAN_SIGN"):
        assert isinstance(output[field], Decimal)
        assert IS_INPUT[field] == Decimal(str(output[field]))


def test_raw_types_accept_bytes(client):
    ZRAW = BYTES_TEST
    DIFF = b"\x00\x00\x00\x00"
    IS_INPUT = {"ZRAW": ZRAW, "ZRAWSTRING": ZRAW}
    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]
    assert output["ZRAW"] == ZRAW + DIFF
    assert output["ZRAWSTRING"] == ZRAW
    assert isinstance(output["ZRAW"], bytes)
    assert isinstance(output["ZRAWSTRING"], bytes)


def test_raw_types_accept_bytearray(client):
    ZRAW = BYTEARRAY_TEST
    DIFF = b"\x00\x00\x00\x00"
    IS_INPUT = {"ZRAW": ZRAW, "ZRAWSTRING": ZRAW}
    output = client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)["ES_OUTPUT"]
    assert output["ZRAW"] == ZRAW + DIFF
    assert output["ZRAWSTRING"] == ZRAW
    assert isinstance(output["ZRAW"], bytes)
    assert isinstance(output["ZRAWSTRING"], bytes)


def test_date_time(client):
    # The original in full: indices 0-5 round-trip, 6-17 are rejected
    # client-side by check_date/check_time (the compat input pre-pass).
    DATETIME_TEST = [
        {"RFCDATE": "20161231", "RFCTIME": "123456"},  # good, correct date
        {"RFCDATE": "", "RFCTIME": "123456"},  # good, empty date
        {"RFCDATE": "        ", "RFCTIME": "123456"},  # good, space date
        {"RFCDATE": "20161231", "RFCTIME": ""},  # good, empty time
        {"RFCDATE": "20161231", "RFCTIME": "      "},  # good, space time
        {"RFCDATE": "20161231", "RFCTIME": "000000"},  # good, zero time
        {"RFCDATE": "2016123", "RFCTIME": "123456"},  # shorter date
        {"RFCDATE": " ", "RFCTIME": "123456"},  # shorter empty date
        {"RFCDATE": "201612311", "RFCTIME": "123456"},  # longer date
        {"RFCDATE": "         ", "RFCTIME": "123456"},  # longer empty date
        {"RFCDATE": "20161232", "RFCTIME": "123456"},  # out of range date
        {"RFCDATE": 20161231, "RFCTIME": "123456"},  # wrong date type
        {"RFCDATE": "20161231", "RFCTIME": "12345"},  # shorter time
        {"RFCDATE": "20161231", "RFCTIME": " "},  # shorter empty time
        {"RFCDATE": "20161231", "RFCTIME": "1234566"},  # longer time
        {"RFCDATE": "20161231", "RFCTIME": "       "},  # longer empty time
        {"RFCDATE": "20161231", "RFCTIME": "123466"},  # out of range time
        {"RFCDATE": "20161231", "RFCTIME": 123456},  # wrong time type
    ]
    for index, dt in enumerate(DATETIME_TEST):
        if index < 6:
            res = client.call("STFC_STRUCTURE", IMPORTSTRUCT=dt)["ECHOSTRUCT"]
            assert dt["RFCDATE"] == res["RFCDATE"]
            if not dt["RFCTIME"]:
                assert res["RFCTIME"] == "000000"
            else:
                assert dt["RFCTIME"] == res["RFCTIME"]
        else:
            with pytest.raises(TypeError) as ex:
                client.call("STFC_STRUCTURE", IMPORTSTRUCT=dt)["ECHOSTRUCT"]
            error = ex.value
            if index < 12:
                assert error.args[0] == "date value required, received"
                assert error.args[1] == dt["RFCDATE"]
                assert isinstance(dt["RFCDATE"], error.args[3])
                assert error.args[4] == "RFCDATE"
            else:
                assert error.args[0] == "time value required, received"
                assert error.args[1] == dt["RFCTIME"]
                assert isinstance(dt["RFCTIME"], error.args[3])
                assert error.args[4] == "RFCTIME"
            assert error.args[5] == "IMPORTSTRUCT"


def test_date_accepts_string(client):
    TEST_DATE = "20180625"
    IMPORTSTRUCT = {"RFCDATE": TEST_DATE}
    IMPORTTABLE = [IMPORTSTRUCT]
    output = client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT, RFCTABLE=IMPORTTABLE)
    assert isinstance(output["ECHOSTRUCT"]["RFCDATE"], str)
    assert isinstance(output["RFCTABLE"][0]["RFCDATE"], str)
    assert output["ECHOSTRUCT"]["RFCDATE"] == TEST_DATE
    assert output["RFCTABLE"][0]["RFCDATE"] == TEST_DATE


def test_date_accepts_date(client):
    TEST_DATE = ABAP_to_python_date("20180625")
    IMPORTSTRUCT = {"RFCDATE": TEST_DATE}
    IMPORTTABLE = [IMPORTSTRUCT]
    output = client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT, RFCTABLE=IMPORTTABLE)
    assert isinstance(output["ECHOSTRUCT"]["RFCDATE"], str)
    assert isinstance(output["RFCTABLE"][0]["RFCDATE"], str)
    assert output["ECHOSTRUCT"]["RFCDATE"] == python_to_ABAP_date(TEST_DATE)
    assert output["RFCTABLE"][0]["RFCDATE"] == python_to_ABAP_date(TEST_DATE)


def test_time_accepts_string(client):
    TEST_TIME = "123456"
    IMPORTSTRUCT = {"RFCTIME": TEST_TIME}
    IMPORTTABLE = [IMPORTSTRUCT]
    output = client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT, RFCTABLE=IMPORTTABLE)
    assert isinstance(output["ECHOSTRUCT"]["RFCTIME"], str)
    assert isinstance(output["RFCTABLE"][0]["RFCTIME"], str)
    assert output["ECHOSTRUCT"]["RFCTIME"] == TEST_TIME
    assert output["RFCTABLE"][0]["RFCTIME"] == TEST_TIME


def test_time_accepts_time(client):
    TEST_TIME = ABAP_to_python_time("123456")
    IMPORTSTRUCT = {"RFCTIME": TEST_TIME}
    IMPORTTABLE = [IMPORTSTRUCT]
    output = client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT, RFCTABLE=IMPORTTABLE)
    assert isinstance(output["ECHOSTRUCT"]["RFCTIME"], str)
    assert isinstance(output["RFCTABLE"][0]["RFCTIME"], str)
    assert output["ECHOSTRUCT"]["RFCTIME"] == python_to_ABAP_time(TEST_TIME)
    assert output["RFCTABLE"][0]["RFCTIME"] == python_to_ABAP_time(TEST_TIME)


def test_error_int_rejects_string(client):
    IMPORTSTRUCT = {"RFCINT1": "1"}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "an integer required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCINT1"]
    assert error.args[3] is str
    assert error.args[4] == "RFCINT1"
    assert error.args[5] == "IMPORTSTRUCT"

    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", RFCTABLE=[IMPORTSTRUCT])
    error = ex.value
    assert error.args[0] == "an integer required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCINT1"]
    assert error.args[3] is str
    assert error.args[4] == "RFCINT1"
    assert error.args[5] == "RFCTABLE"


def test_error_int_rejects_float(client):
    IMPORTSTRUCT = {"RFCINT1": 1.0}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "an integer required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCINT1"]
    assert error.args[3] is float
    assert error.args[4] == "RFCINT1"
    assert error.args[5] == "IMPORTSTRUCT"

    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", RFCTABLE=[IMPORTSTRUCT])
    error = ex.value
    assert error.args[0] == "an integer required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCINT1"]
    assert error.args[3] is float
    assert error.args[4] == "RFCINT1"
    assert error.args[5] == "RFCTABLE"


def test_error_string_rejects_None(client):
    IMPORTSTRUCT = {"RFCCHAR4": None}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "an string is required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCCHAR4"]
    assert isinstance(None, error.args[3])  # error.args[3] is type(None)
    assert error.args[4] == "RFCCHAR4"
    assert error.args[5] == "IMPORTSTRUCT"

    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", RFCTABLE=[IMPORTSTRUCT])
    error = ex.value
    assert error.args[0] == "an string is required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCCHAR4"]
    assert isinstance(None, error.args[3])
    assert error.args[4] == "RFCCHAR4"
    assert error.args[5] == "RFCTABLE"


def test_error_string_rejects_int(client):
    IMPORTSTRUCT = {"RFCCHAR4": 1}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "an string is required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCCHAR4"]
    assert error.args[3] is int
    assert error.args[4] == "RFCCHAR4"
    assert error.args[5] == "IMPORTSTRUCT"

    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", RFCTABLE=[IMPORTSTRUCT])
    error = ex.value
    assert error.args[0] == "an string is required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCCHAR4"]
    assert error.args[3] is int
    assert error.args[4] == "RFCCHAR4"
    assert error.args[5] == "RFCTABLE"


def test_float_rejects_not_a_number_string(client):
    IMPORTSTRUCT = {"RFCFLOAT": "A"}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "a decimal value required, received"
    assert error.args[1] == IMPORTSTRUCT["RFCFLOAT"]
    assert error.args[3] is str
    assert error.args[4] == "RFCFLOAT"
    assert error.args[5] == "IMPORTSTRUCT"


def test_float_rejects_array(client):
    IMPORTSTRUCT = {"RFCFLOAT": []}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "a decimal value required, received"
    assert error.args[1] == []
    assert error.args[3] is list
    assert error.args[4] == "RFCFLOAT"
    assert error.args[5] == "IMPORTSTRUCT"


def test_float_rejects_comma_for_point_locale(client):
    IMPORTSTRUCT = {"RFCFLOAT": "1,2"}
    with pytest.raises(TypeError) as ex:
        client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
    error = ex.value
    assert error.args[0] == "a decimal value required, received"
    assert error.args[1] == "1,2"
    assert error.args[3] is str
    assert error.args[4] == "RFCFLOAT"
    assert error.args[5] == "IMPORTSTRUCT"


def test_float_accepts_point_for_point_locale(client):
    IMPORTSTRUCT = {"RFCFLOAT": "1.2"}
    output = client.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)["ECHOSTRUCT"]
    assert output["RFCFLOAT"] == 1.2


def test_bcd_rejects_not_a_number_string(client):
    # The SDK would SILENTLY garbage-convert this ('A' → 1.00, probed on the
    # vendored 750 PL 18); pyrfc's client-side Decimal parse is the only shield,
    # which is precisely why the compat pre-pass mirrors it.
    IS_INPUT = {"ZDEC": "A"}
    with pytest.raises(TypeError) as ex:
        client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)
    error = ex.value
    assert error.args[0] == "a decimal value required, received"
    assert error.args[1] == "A"
    assert error.args[3] is str
    assert error.args[4] == "ZDEC"
    assert error.args[5] == "IS_INPUT"


def test_numc_rejects_non_string(client):
    IS_INPUT = {"ZNUMC": 1}
    with pytest.raises(TypeError) as ex:
        client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)
    error = ex.value
    assert error.args[0] == "a numeric string is required, received"
    assert error.args[1] == IS_INPUT["ZNUMC"]
    assert error.args[3] is int
    assert error.args[4] == "ZNUMC"
    assert error.args[5] == "IS_INPUT"


def test_numc_rejects_non_numeric_string(client):
    IS_INPUT = {"ZNUMC": "a1"}
    with pytest.raises(TypeError) as ex:
        client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)
    error = ex.value
    assert error.args[0] == "a numeric string is required, received"
    assert error.args[1] == IS_INPUT["ZNUMC"]
    assert error.args[3] is str
    assert error.args[4] == "ZNUMC"
    assert error.args[5] == "IS_INPUT"


def test_numc_rejects_empty_string(client):
    IS_INPUT = {"ZNUMC": ""}
    with pytest.raises(TypeError) as ex:
        client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)
    error = ex.value
    assert error.args[0] == "a numeric string is required, received"
    assert error.args[1] == IS_INPUT["ZNUMC"]
    assert error.args[3] is str
    assert error.args[4] == "ZNUMC"
    assert error.args[5] == "IS_INPUT"


def test_numc_rejects_space_string(client):
    IS_INPUT = {"ZNUMC": " "}
    with pytest.raises(TypeError) as ex:
        client.call(DATATYPES_FM, IS_INPUT=IS_INPUT, IV_COUNT=0)
    error = ex.value
    assert error.args[0] == "a numeric string is required, received"
    assert error.args[1] == IS_INPUT["ZNUMC"]
    assert error.args[3] is str
    assert error.args[4] == "ZNUMC"
    assert error.args[5] == "IS_INPUT"
