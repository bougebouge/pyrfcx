# SPDX-FileCopyrightText: 2013 SAP SE Srdjan Boskovic <srdjan.boskovic@sap.com>
# SPDX-FileCopyrightText: 2026 the pyrfcx contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Adapted from pyrfc 3.4. This file is modified; the docstring lists the changes.

"""Adapted from pyrfc 3.4 tests/test_errors.py (https://github.com/SAP/PyRFC).

Changes vs the original:
- Connection params: dest=TEST_NW750 (SNC) instead of user/passwd from pyrfc.cfg.
- test_incomplete_params, test_denied_users: skipped — they need user/passwd
  auth (strip ashost / wrong password); our system is SNC-only.
- test_call_without_RFM_name: pyrfc's Cython signature produced
  "call() takes at least 1 positional argument (0 given)"; plain Python 3
  wording differs — assertion relaxed to the raise itself.
"""

from __future__ import annotations

import pytest

from pyrfc import (
    ABAPApplicationError,
    CommunicationError,
    Connection,
    ExternalRuntimeError,
    RFCError,
)

pytestmark = [pytest.mark.live, pytest.mark.compat]


class TestErrors:
    def test_no_connection_params(self):
        with pytest.raises(RFCError) as ex:
            Connection()
        error = ex.value
        assert error.args[0] == "Connection parameters missing"

    @pytest.mark.skip(reason="needs user/passwd auth; the test destinations are SNC-only")
    def test_incomplete_params(self):
        pass

    @pytest.mark.skip(reason="needs user/passwd auth; the test destinations are SNC-only")
    def test_denied_users(self):
        pass

    def test_error_text_is_right_stripped(self):
        # pyrfcx addition: pyrfc's wrapError passes every RFC_ERROR_INFO text
        # through wrapString, which right-strips ASCII whitespace. The SDK
        # text of an unknown host ends in a newline; pyrfc drops it.
        with pytest.raises(CommunicationError) as ex:
            Connection(ashost="pyrfcx-facts.invalid", sysnr="00", client="000")
        error = ex.value
        assert error.code == 1
        assert error.key == "RFC_COMMUNICATION_FAILURE"
        assert "hostname 'pyrfcx-facts.invalid' unknown" in error.message
        assert error.message == error.message.rstrip()
        assert error.args == (error.message,)
        assert str(error).endswith("[MSG: class=, type=, number=, v1-4:=;;;]")
        assert "\n [MSG:" not in str(error)

    def test_call_without_RFM_name(self, conn):
        with pytest.raises(Exception) as ex:
            conn.call()
        error = ex.value
        assert isinstance(error, TypeError) is True
        # pyrfc 3.4 (Cython): "call() takes at least 1 positional argument (0 given)"
        assert "positional argument" in error.args[0]

    def test_call_non_existing_RFM(self, conn):
        with pytest.raises(ABAPApplicationError) as ex:
            conn.call("undefined")
        error = ex.value
        assert error.code == 5
        assert error.key == "FU_NOT_FOUND"
        assert error.message == "ID:FL Type:E Number:046 undefined"

    def test_call_non_string_RFM_name(self, conn):
        with pytest.raises(RFCError) as ex:
            conn.call(1)
        error = ex.value
        assert error.args == (
            "Remote function module name must be unicode string, received:",
            1,
            int,
        )

    def test_call_non_existing_RFM_parameter(self, conn):
        with pytest.raises(ExternalRuntimeError) as ex:
            conn.call("STFC_CONNECTION", undefined=0)
        error = ex.value
        assert error.code == 20
        assert error.key == "RFC_INVALID_PARAMETER"
        assert error.message == "field 'undefined' not found"

    def test_non_existing_field_structure(self, conn):
        IMPORTSTRUCT = {
            "XRFCCHAR1": "A",
            "RFCCHAR2": "BC",
            "RFCCHAR4": "DEFG",
        }
        with pytest.raises(ExternalRuntimeError) as ex:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT=IMPORTSTRUCT)
        error = ex.value
        assert error.code == 20
        assert error.key == "RFC_INVALID_PARAMETER"
        assert error.message == "field 'XRFCCHAR1' not found"

    def test_non_existing_field_table(self, conn):
        IMPORTSTRUCT = {
            "XRFCCHAR1": "A",
            "RFCCHAR2": "BC",
            "RFCCHAR4": "DEFG",
        }
        with pytest.raises(ExternalRuntimeError) as ex:
            conn.call("STFC_STRUCTURE", RFCTABLE=[IMPORTSTRUCT])
        error = ex.value
        assert error.code == 20
        assert error.key == "RFC_INVALID_PARAMETER"
        assert error.message == "field 'XRFCCHAR1' not found"
