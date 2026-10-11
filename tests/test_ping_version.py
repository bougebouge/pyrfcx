"""Live tests for Connection.ping() and sdk_version() — requires the SAP
destination PYRFCX_TEST_DEST (default TEST_NW750).

There is deliberately no reopen() here: it has no SDK call behind it (close +
open composed), so it is a pure projection living only on the compat facade —
covered by tests/compat/test_connection.py.
"""

from __future__ import annotations

import pytest

from pyrfcx import Connection, SdkVersion, sdk_version
from pyrfcx._native import sdk_version as native_sdk_version
from tests.conftest import DEST

pytestmark = pytest.mark.live


class TestPing:
    def test_ping_round_trips_and_leaves_the_connection_usable(self, conn: Connection) -> None:
        conn.ping()
        # Not consumed: another ping and a real call still work.
        conn.ping()
        assert conn.call("RFC_PING") == {}

    def test_ping_on_closed_connection_raises(self) -> None:
        c = Connection(dest=DEST)
        c.close()
        with pytest.raises(ValueError, match="closed"):
            c.ping()

    def test_native_connection_has_no_reopen(self) -> None:
        # Pure projection: reopen exists only on the
        # compat facade; the native surface must not grow it back silently.
        assert not hasattr(Connection, "reopen")


class TestSdkVersion:
    def test_reports_the_loaded_sdk(self) -> None:
        v = sdk_version()
        assert isinstance(v, SdkVersion)
        # Self-checking, not version-hardcoded: must match what the binding
        # reports (which tests/test_sdk_pin.py ties to the vendored DLL).
        assert (v.major, v.minor, v.patch_level, v.description) == native_sdk_version()
        # The SDK's convention: major is the release (e.g. 7500), and the
        # description is its human-readable form ("750 Patch Level 18" —
        # probe, 750 PL 18).
        assert v.major >= 1000
        assert str(v.patch_level) in v.description
