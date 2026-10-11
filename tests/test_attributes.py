"""Live tests for Connection.attributes() — requires the SAP destination
PYRFCX_TEST_DEST (default TEST_NW750)."""

from __future__ import annotations

import dataclasses
import os

import pytest

from pyrfcx import Connection, ConnectionAttributes
from tests.conftest import BACKEND, DEST

pytestmark = pytest.mark.live


class TestAttributes:
    def test_describes_the_connection(self, conn: Connection) -> None:
        a = conn.attributes()

        # Portable: true of any client connection to any R/3 backend.
        assert a.rfc_role == "C"  # we are the RFC client
        assert a.type == "E"  # we are an external program
        assert a.partner_type == "3"  # partner is an R/3 system

        # System-local: the SID is checked only when PYRFCX_TEST_SID names it.
        expected_sid = os.environ.get("PYRFCX_TEST_SID")
        if expected_sid is None:
            assert len(a.sys_id) == 3
        else:
            assert a.sys_id == expected_sid
        assert a.client == BACKEND.client
        assert a.sys_number == BACKEND.sys_number
        assert a.partner_host

        # Trailing pad blank stripped: the SDK returns e.g. "750 ".
        assert a.partner_rel == BACKEND.partner_rel

    def test_dest_comes_from_the_connection_params(self, conn: Connection) -> None:
        # The SDK reports `dest` as empty (probe, 750 PL 18) — the value is
        # supplied from the parameters the connection was opened with.
        assert conn._native.attributes()["dest"] == ""
        assert conn.attributes().dest == DEST

    def test_dest_lookup_is_case_insensitive(self) -> None:
        # SDK parameter names are case-insensitive; capture-repository opens
        # with "DEST" while the fixtures use "dest".
        c = Connection(DEST=DEST)
        try:
            assert c.attributes().dest == DEST
        finally:
            c.close()

    def test_every_sdk_field_is_populated(self, conn: Connection) -> None:
        # Guards the PyO3 dict against a field being dropped: every key the
        # dataclass declares must arrive, and `reserved` must not.
        native = conn._native.attributes()
        names = {f.name for f in dataclasses.fields(ConnectionAttributes)}
        assert set(native) == names
        assert "reserved" not in native

    def test_attributes_on_closed_connection_raises(self) -> None:
        c = Connection(dest=DEST)
        c.close()
        with pytest.raises(ValueError, match="closed"):
            c.attributes()

    def test_params_are_not_publicly_exposed(self) -> None:
        # Params may hold `passwd`; they must not reach a public attribute or
        # the repr.
        c = Connection(dest=DEST, passwd="hunter2")
        try:
            assert not hasattr(c, "params")
            assert "hunter2" not in repr(c)
        finally:
            c.close()
