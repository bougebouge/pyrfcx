# SPDX-FileCopyrightText: 2013 SAP SE Srdjan Boskovic <srdjan.boskovic@sap.com>
# SPDX-FileCopyrightText: 2026 the pyrfcx contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Adapted from pyrfc 3.4. This file is modified; the docstring lists the changes.

"""Adapted from pyrfc 3.4 tests/test_connection.py (https://github.com/SAP/PyRFC).

Changes vs the original:
- Connection via dest=TEST_NW750 fixture instead of pyrfc.cfg dest=MME.
- test_pyrfc_version dropped (checks pyrfc's own pyproject.toml — meaningless
  for the facade; the facade's __version__ is pinned "3.4" in test_smoke.py).
- Added (measured on real pyrfc 3.4 first, anti-circularity): the
  values behind the key-presence checks — the version dict, the options dict,
  the attribute dict and its `dest == ""` — ping on a closed connection, and
  which errors reconnect (only a connection-killing error of RfcInvoke).
- test_STFC_STRUCTURE: RFCDATE is excluded from the row comparison like
  RFCTIME. The backend fills it with the server's local date, which differs
  from the client's date near midnight when the time zones differ.
"""

from __future__ import annotations

import datetime
import re
import sys

import pytest

from pyrfc import (
    ABAPApplicationError,
    ABAPRuntimeError,
    Connection,
    ExternalRuntimeError,
    RFCError,
)
from tests.compat.config import BACKEND, UNICODETEST
from tests.compat.config import CONNECTION_PARAMS as params
from tests.test_sdk_pin import vendored_dll_version

pytestmark = [pytest.mark.live, pytest.mark.compat]


class TestConnection:
    def test_sdk_version_and_options_getters(self, conn):
        version = conn.version
        assert "major" in version
        assert "minor" in version
        assert "patchLevel" in version
        assert all(attr in conn.options for attr in ("dtime", "return_import_params", "rstrip"))

    def test_connection_info(self, conn):
        connection_info = conn.get_connection_attributes()
        info_keys = (
            "dest",
            "host",
            "partnerHost",
            "sysNumber",
            "sysId",
            "client",
            "user",
            "language",
            "trace",
            "isoLanguage",
            "codepage",
            "partnerCodepage",
            "rfcRole",
            "type",
            "partnerType",
            "rel",
            "partnerRel",
            "kernelRel",
            "cpicConvId",
            "progName",
            "partnerBytesPerChar",
            "partnerIP",
            "partnerIPv6",
        )
        assert all(attr in connection_info for attr in info_keys)

    def test_connection_info_disconnected(self, conn):
        conn.close()
        assert not conn.alive
        assert conn.get_connection_attributes() == {}

    def test_reopen(self, conn):
        assert conn.alive
        conn.reopen()
        assert conn.alive
        conn.close()
        assert not conn.alive
        conn.reopen()
        assert conn.alive

    def test_call_over_closed_connection(self):
        conn = Connection(config={"rstrip": False}, **params)
        conn.close()
        assert conn.alive is False
        hello = "Hällo SAP!"
        with pytest.raises(RFCError) as ex:
            conn.call("STFC_CONNECTION", REQUTEXT=hello)
        error = ex.value
        assert error.args[0] == (
            "Remote function module 'STFC_CONNECTION' invocation rejected "
            "because the connection is closed"
        )

    def test_ping(self, conn):
        assert conn.alive
        conn.ping()

    def test_RFM_name_string(self, conn):
        res = conn.call("STFC_CONNECTION", REQUTEXT=UNICODETEST)
        assert res["ECHOTEXT"] == UNICODETEST

    def test_RFM_name_unicode(self, conn):
        res = conn.call("STFC_CONNECTION", REQUTEXT=UNICODETEST)
        assert res["ECHOTEXT"] == UNICODETEST

    def test_RFM_name_invalid_type(self, conn):
        with pytest.raises(Exception) as ex:
            conn.call(123)
        error = ex.value
        assert error.args == (
            "Remote function module name must be unicode string, received:",
            123,
            int,
        )

    def test_STFC_returns_structure_and_table(self, conn):
        IMPORTSTRUCT = {
            "RFCFLOAT": 1.23456789,
            "RFCCHAR1": "A",
            "RFCCHAR2": "BC",
            "RFCCHAR4": "DEFG",
            "RFCINT1": 1,
            "RFCINT2": 2,
            "RFCINT4": 345,
            "RFCHEX3": b"\x01\x02\x03",
            "RFCTIME": "121120",
            "RFCDATE": "20140101",
            "RFCDATA1": "1DATA1",
            "RFCDATA2": "DATA222",
        }
        INPUTROWS = 10
        IMPORTTABLE = []
        for idx in range(INPUTROWS):
            row = IMPORTSTRUCT
            row["RFCINT1"] = idx
            IMPORTTABLE.append(row)
        res = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT, RFCTABLE=IMPORTTABLE)
        # ECHOSTRUCT match IMPORTSTRUCT
        for attr in IMPORTSTRUCT:
            assert res["ECHOSTRUCT"][attr] == IMPORTSTRUCT[attr]

        # check if row added
        assert len(res["RFCTABLE"]) == INPUTROWS + 1

        # output table match import table
        for idx in range(INPUTROWS):
            row_in = IMPORTTABLE[idx]
            row_out = res["RFCTABLE"][idx]
            assert row_in == row_out

        # added row match incremented IMPORTSTRUCT
        added_row = res["RFCTABLE"][INPUTROWS]
        assert added_row["RFCFLOAT"] == IMPORTSTRUCT["RFCFLOAT"] + 1
        assert added_row["RFCINT1"] == IMPORTSTRUCT["RFCINT1"] + 1
        assert added_row["RFCINT2"] == IMPORTSTRUCT["RFCINT2"] + 1
        assert added_row["RFCINT4"] == IMPORTSTRUCT["RFCINT4"] + 1

    def test_STFC_STRUCTURE(self, conn):
        # STFC_STRUCTURE Inhomogene Struktur
        imp = {
            "RFCFLOAT": 1.23456789,
            "RFCINT2": 0x7FFE,
            "RFCINT1": 0x7F,
            "RFCCHAR4": "bcde",
            "RFCINT4": 0x7FFFFFFE,
            "RFCHEX3": b"fgh",
            "RFCCHAR1": "a",
            "RFCCHAR2": "ij",
            "RFCTIME": "123456",
            "RFCDATE": "20161231",
            "RFCDATA1": "k" * 50,
            "RFCDATA2": "l" * 50,
        }
        out = {
            "RFCFLOAT": imp["RFCFLOAT"] + 1,
            "RFCINT2": imp["RFCINT2"] + 1,
            "RFCINT1": imp["RFCINT1"] + 1,
            "RFCINT4": imp["RFCINT4"] + 1,
            "RFCHEX3": b"\xf1\xf2\xf3",
            "RFCCHAR1": "X",
            "RFCCHAR2": "YZ",
            "RFCDATE": str(datetime.date.today()).replace("-", ""),
            "RFCDATA1": "k" * 50,
            "RFCDATA2": "l" * 50,
        }
        table = []
        xtable = []
        records = ["1111", "2222", "3333", "4444", "5555"]
        for rid in records:
            imp["RFCCHAR4"] = rid
            table.append(imp)
            xtable.append(imp)
        res = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp, RFCTABLE=xtable)
        assert res["RESPTEXT"].startswith("SAP")
        assert len(res["RFCTABLE"]) == 1 + len(table)
        for idx in res["ECHOSTRUCT"]:
            assert res["ECHOSTRUCT"][idx] == imp[idx]
        for idx in res["RFCTABLE"][5]:
            # dont compare variable system id and server date and time
            if idx not in ["RFCCHAR4", "RFCDATE", "RFCTIME"]:
                assert res["RFCTABLE"][5][idx] == out[idx]

    def test_STFC_CHANGING(self, conn):
        # STFC_CHANGING example with CHANGING parameters
        start_value = 33
        counter = 88
        res = conn.call("STFC_CHANGING", START_VALUE=start_value, COUNTER=counter)
        assert res["COUNTER"] == counter + 1
        assert res["RESULT"] == start_value + counter


# ── added: values measured on real pyrfc 3.4 ────────────────────────────


def test_version_values(conn):
    # RfcGetVersion read from the vendored library by ctypes, without the
    # stack under test, plus sys.platform.
    major, minor, patch_level, _ = vendored_dll_version()
    assert conn.version == {
        "major": major,
        "minor": minor,
        "patchLevel": patch_level,
        "platform": sys.platform,
    }


def test_options_values():
    conn = Connection(**params)
    assert conn.options == {
        "rstrip": True,
        "return_import_params": False,
        "dtime": False,
        "check_date": True,
        "check_time": True,
        "timeout": None,
    }
    conn.close()
    conn = Connection(config={"check_date": False}, **params)
    options = conn.options
    assert options["check_date"] is False
    assert list(options) == [
        "rstrip",
        "return_import_params",
        "dtime",
        "check_date",
        "check_time",
        "timeout",
    ]
    # The dict itself is returned, not a copy.
    assert conn.options is options
    # A change to check_date after the open has no effect: the check stays off.
    options["check_date"] = True
    res = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATE": "20161232"})
    assert res["ECHOSTRUCT"]["RFCDATE"] == "20161232"
    conn.close()


def test_connection_attribute_values(conn):
    attrs = conn.get_connection_attributes()
    assert list(attrs) == [
        "dest",
        "host",
        "partnerHost",
        "sysNumber",
        "sysId",
        "client",
        "user",
        "language",
        "trace",
        "isoLanguage",
        "codepage",
        "partnerCodepage",
        "rfcRole",
        "type",
        "partnerType",
        "rel",
        "partnerRel",
        "kernelRel",
        "cpicConvId",
        "progName",
        "partnerBytesPerChar",
        "partnerSystemCodepage",
        "partnerIP",
        "partnerIPv6",
        "reserved",
        "active_unit",
    ]
    # The SDK does not report the destination, and pyrfc does not fill it in.
    assert attrs["dest"] == ""
    assert attrs["reserved"] == ""
    assert attrs["active_unit"] is False
    # Blank-padded fields come back stripped ("750 " in the SDK buffer).
    assert attrs["sysId"]
    expected = {
        "rfcRole": "C",
        "type": "E",
        "partnerType": "3",
        "partnerRel": BACKEND.partner_rel,
        "kernelRel": BACKEND.kernel_rel,
        "partnerBytesPerChar": "2",
    }
    assert {key: attrs[key] for key in expected} == expected
    # The SDK's own kernel release: it differs by patch level (753, 754).
    assert re.fullmatch(r"\d{3}", attrs["rel"])
    assert all(type(attrs[key]) is str for key in list(attrs)[:-1])


def test_ping_closed_connection(conn):
    conn.close()
    # pyrfc passes its NULL handle to RfcPing and raises the SDK's answer.
    with pytest.raises(ExternalRuntimeError) as ex:
        conn.ping()
    assert ex.value.args == (
        "An invalid handle 'RFC_CONNECTION_HANDLE' was passed to the API call",
    )
    assert str(ex.value) == (
        "13 (rc=13): key=RFC_INVALID_HANDLE, message=An invalid handle "
        "'RFC_CONNECTION_HANDLE' was passed to the API call "
        "[MSG: class=, type=, number=, v1-4:=;;;]"
    )
    assert not conn.alive


def test_reconnect_only_after_invoke(conn):
    # A new connection has a new CPI-C conversation ID.
    conv_id = conn.get_connection_attributes()["cpicConvId"]
    # Errors before RfcInvoke do not reconnect. RFC_INVALID_PARAMETER is in
    # the EXTERNAL_RUNTIME_FAILURE group, which reconnects after RfcInvoke.
    with pytest.raises(ExternalRuntimeError):
        conn.call("STFC_CONNECTION", {"not_requested": ["NOPE"]}, REQUTEXT="hi")
    with pytest.raises(ABAPApplicationError):
        conn.call("NO_SUCH_FM_XYZ")
    assert conn.alive
    assert conn.get_connection_attributes()["cpicConvId"] == conv_id
    # An ABAP message type A fails RfcInvoke and closes the connection:
    # pyrfc opens a new one.
    with pytest.raises(ABAPRuntimeError) as ex:
        conn.call("RFC_RAISE_ERROR", MESSAGETYPE="A")
    assert ex.value.code == 4
    assert conn.alive
    assert conn.get_connection_attributes()["cpicConvId"] != conv_id


def test_open_and_is_valid(conn):
    conv_id = conn.get_connection_attributes()["cpicConvId"]
    assert conn.is_valid() is True
    # open() on an open connection opens a new one.
    assert conn.open() is None
    assert conn.alive
    assert conn.is_valid() is True
    assert conn.get_connection_attributes()["cpicConvId"] != conv_id
    conn.close()
    assert conn.is_valid() is False
    conn.open()
    assert conn.alive
    assert conn.is_valid() is True


def test_is_valid_after_reconnect(conn):
    # The ABAP message type A closes the connection and pyrfc reconnects.
    with pytest.raises(ABAPRuntimeError):
        conn.call("RFC_RAISE_ERROR", MESSAGETYPE="A")
    assert conn.is_valid() is True
