# SPDX-FileCopyrightText: 2013 SAP SE Srdjan Boskovic <srdjan.boskovic@sap.com>
# SPDX-FileCopyrightText: 2026 the pyrfcx contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Adapted from pyrfc 3.4. This file is modified; the docstring lists the changes.

"""Adapted from pyrfc 3.4 tests/test_client_config.py (https://github.com/SAP/PyRFC).

The facade supports every `config` key. BAPI_USER_GET_DETAIL queries
TEST_USER instead of "demo". The values behind these key-presence checks are
pinned in test_config_output.py; the timeout behaviour in test_timeout.py.
"""

from __future__ import annotations

import datetime

import pytest

from pyrfc import Connection, RFCError
from tests.compat.config import CONNECTION_PARAMS, TEST_USER, requires_test_user

pytestmark = [pytest.mark.live, pytest.mark.compat]


class TestClientConfig:
    def test_config_rstrip_false(self):
        conn = Connection(config={"rstrip": False}, **CONNECTION_PARAMS)
        hello = "Hällo SAP!" + " " * 245
        res = conn.call("STFC_CONNECTION", REQUTEXT=hello)
        assert res["ECHOTEXT"] == hello
        conn.close()

    def test_config_rstrip_true(self):
        conn = Connection(**CONNECTION_PARAMS)

        # Test with rstrip=True (input length=255 char)
        hello = "Hällo SAP!" + " " * 245
        res = conn.call("STFC_CONNECTION", REQUTEXT=hello)
        assert res["ECHOTEXT"] == hello.rstrip()

        # Test with rstrip=True (input length=10 char)
        res = conn.call("STFC_CONNECTION", REQUTEXT=hello.rstrip())
        assert res["ECHOTEXT"] == hello.rstrip()
        conn.close()

    @requires_test_user
    def test_config_dtime_true(self):
        conn = Connection(config={"dtime": True}, **CONNECTION_PARAMS)
        dates = conn.call("BAPI_USER_GET_DETAIL", USERNAME=TEST_USER)["LASTMODIFIED"]
        assert type(dates["MODDATE"]) is datetime.date
        assert type(dates["MODTIME"]) is datetime.time
        conn.close()

    @requires_test_user
    def test_config_dtime_false(self):
        # Was xfailed for the empty-decimal crash — see test_datatypes.test_date_output: that
        # empty-decimal crash is 753-specific and does not occur on the
        # vendored 7.50 SDK we now pin.
        conn = Connection(**CONNECTION_PARAMS)
        # dates as strings
        dates = conn.call("BAPI_USER_GET_DETAIL", USERNAME=TEST_USER)["LASTMODIFIED"]
        assert type(dates["MODDATE"]) is not datetime.date
        assert type(dates["MODDATE"]) is not datetime.time
        conn.close()

    def test_config_return_import_params_false(self):
        # no import params return
        conn = Connection(**CONNECTION_PARAMS)
        hello = "Hällo SAP!"
        res = conn.call("STFC_CONNECTION", REQUTEXT=hello)
        assert "REQTEXT" not in res
        conn.close()

    def test_config_return_import_params_true(self):
        conn = Connection(config={"return_import_params": True}, **CONNECTION_PARAMS)
        res = conn.call("STFC_CONNECTION", REQUTEXT="hello")
        assert "REQUTEXT" in res
        conn.close()

    def test_config_timeout(self):
        conn = Connection(config={"timeout": 123}, **CONNECTION_PARAMS)
        assert conn.options["timeout"] == 123
        conn.close()

    def test_config_not_supported(self):
        with pytest.raises(RFCError) as ex:
            Connection(config={"xtimeout": 123}, **CONNECTION_PARAMS)
        error = ex.value
        assert error.args[0] == "Connection configuration option 'xtimeout' is not supported"
