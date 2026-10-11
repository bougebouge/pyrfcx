# SPDX-FileCopyrightText: 2013 SAP SE Srdjan Boskovic <srdjan.boskovic@sap.com>
# SPDX-FileCopyrightText: 2026 the pyrfcx contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Adapted from pyrfc 3.4. This file is modified; the docstring lists the changes.

"""Adapted from pyrfc 3.4 tests/test_errors_abap.py (https://github.com/SAP/PyRFC).

Changes vs the original:
- Connection via dest=TEST_NW750 fixture instead of user/passwd params.
- test_no_connection_params dropped (duplicate of test_errors.py).
- The commented-out ExternalRuntimeError / CommunicationError tests of the
  original remain out.

Every test asserts `conn.alive is True` after the ABAP error — this pins
the reconnect-on-error behavior.
"""

from __future__ import annotations

import pytest

from pyrfc import ABAPApplicationError, ABAPRuntimeError

pytestmark = [pytest.mark.live, pytest.mark.compat]


class TestErrorsABAP:
    def test_RFC_RAISE_ERROR(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", MESSAGETYPE="A")
        assert conn.alive is True
        error = ex.value
        assert error.code == 4
        assert error.key == "Function not supported"
        assert error.message == "Function not supported"
        assert error.msg_class == "SR"
        assert error.msg_type == "A"
        assert error.msg_number == "006"

    @pytest.mark.dumps  # ST22: DYNPRO_SEND_IN_BACKGROUND (SAPLSPO1)
    def test_STFC_SAPGUI(self, conn):
        # STFC_SAPGUI RFC-TEST:   RFC with SAPGUI
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("STFC_SAPGUI")
        assert conn.alive is True
        error = ex.value
        assert error.code == 3
        assert error.key == "DYNPRO_SEND_IN_BACKGROUND"

    def test_AbapRuntimeError_E0(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", METHOD="0", MESSAGETYPE="E")
        error = ex.value
        assert conn.alive is True
        assert error.code == 4
        assert error.message == "Function not supported"

    def test_AbapApplicationError_E1(self, conn):
        with pytest.raises(ABAPApplicationError) as ex:
            conn.call("RFC_RAISE_ERROR", METHOD="1", MESSAGETYPE="E")
        assert conn.alive is True
        error = ex.value
        assert error.code == 5
        assert error.key == "RAISE_EXCEPTION"
        assert error.msg_class == "SR"
        assert error.msg_type == "E"
        assert error.msg_number == "006"

    def test_AbapApplicationError_E2(self, conn):
        with pytest.raises(ABAPApplicationError) as ex:
            conn.call("RFC_RAISE_ERROR", METHOD="2", MESSAGETYPE="E")
        assert conn.alive is True
        error = ex.value
        assert error.code == 5
        assert error.key == "RAISE_EXCEPTION"
        assert error.msg_number == "000"

    @pytest.mark.dumps  # ST22: COMPUTE_INT_ZERODIVIDE (SAPLMRFC)
    def test_AbapRuntimeError_E3(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", METHOD="3", MESSAGETYPE="E")
        assert conn.alive is True
        error = ex.value
        assert error.code == 3
        assert error.key == "COMPUTE_INT_ZERODIVIDE"

    def test_AbapRuntimeError_A(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", MESSAGETYPE="A")
        assert conn.alive is True
        error = ex.value
        assert error.code == 4
        assert error.msg_class == "SR"
        assert error.msg_type == "A"
        assert error.msg_number == "006"
        assert error.message == "Function not supported"

    @pytest.mark.dumps  # ST22: MESSAGE_TYPE_X (SAPLMRFC)
    def test_AbapRuntimeError_X(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", MESSAGETYPE="X")
        assert conn.alive is True
        error = ex.value
        assert error.code == 4
        assert error.key == "MESSAGE_TYPE_X"
        assert error.msg_class == "SR"
        assert error.msg_type == "X"
        assert error.msg_number == "006"
        assert (
            error.message
            == "The current application has triggered a termination with a short dump."
        )

    @pytest.mark.dumps  # ST22: COMPUTE_INT_ZERODIVIDE (SAPLMRFC)
    def test_AbapRuntimeError_E36(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", METHOD="36", MESSAGETYPE="E")
        assert conn.alive is True
        error = ex.value
        assert error.code == 4
        assert "Division by 0" in str(error.message)

    @pytest.mark.dumps  # ST22: BLOCKED_COMMIT (SAPMSSY1)
    def test_AbapRuntimeError_E51(self, conn):
        with pytest.raises(ABAPRuntimeError) as ex:
            conn.call("RFC_RAISE_ERROR", METHOD="51", MESSAGETYPE="E")
        assert conn.alive is True
        error = ex.value
        assert error.code == 3
        assert error.key == "BLOCKED_COMMIT"

    def test_pyrfc_exc_string(self, conn):
        # message class/number are system-release-specific: pyrfc's suite saw
        # SV/029; this system's RFC_READ_TABLE raises DA/131 for the missing
        # table — the pinned thing is the str(error) template, byte-exact
        with pytest.raises(ABAPApplicationError) as ex:
            conn.call("RFC_READ_TABLE", QUERY_TABLE="T008X", DELIMITER=".")
        error = ex.value
        assert error.code == 5
        assert error.key == "TABLE_NOT_AVAILABLE"
        assert error.message == "ID:DA Type:E Number:131 T008X"
        assert error.msg_type == "E"
        assert error.msg_number == "131"
        assert error.msg_v1 == "T008X"
        assert (
            str(error) == "5 (rc=5): key=TABLE_NOT_AVAILABLE, message=ID:DA "
            "Type:E Number:131 T008X [MSG: class=DA, type=E, number=131, v1-4:=T008X;;;]"
        )
