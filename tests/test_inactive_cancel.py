"""Live tests for parameter deactivation (pyrfcx.INACTIVE) and cross-thread
call cancellation (Connection.cancel) — requires the SAP destination
PYRFCX_TEST_DEST (default TEST_NW750).

Both are native SDK surface — RfcSetParameterActive and RfcCancel.
There is deliberately no native timeout option: the SDK has no timeout call
(pyrfc's is a client-side timer composed with cancel), so by the rule "native
exposes only what the SDK offers" it is a pure projection for the compat facade.
"""

from __future__ import annotations

import threading
import time

import pytest

from pyrfcx import INACTIVE, CommunicationError, Connection, InactiveType, RfcError
from tests.conftest import DEST

pytestmark = pytest.mark.live


class TestInactiveSentinel:
    def test_singleton_and_repr(self) -> None:
        assert INACTIVE is InactiveType()
        assert repr(INACTIVE) == "pyrfcx.INACTIVE"

    def test_inactive_output_is_omitted(self, conn: Connection) -> None:
        full = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATA1": "payload"})
        assert "ECHOSTRUCT" in full

        trimmed = conn.call(
            "STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATA1": "payload"}, ECHOSTRUCT=INACTIVE
        )
        assert "ECHOSTRUCT" not in trimmed
        assert trimmed["RESPTEXT"].rstrip() != ""

    def test_inactive_tables_param_is_omitted(self, conn: Connection) -> None:
        # The guide's canonical use case (§4.2.2): don't request a big table.
        result = conn.call("RFC_READ_TABLE", QUERY_TABLE="T000", DATA=INACTIVE)
        assert "DATA" not in result
        assert "FIELDS" in result

    def test_inactive_input_is_not_sent(self, conn: Connection) -> None:
        # The sentinel replaces the value, so "set a value AND deactivate the
        # same parameter" is unrepresentable from Python — the backend just
        # sees no REQUTEXT and echoes the initial value.
        result = conn.call("STFC_CONNECTION", REQUTEXT=INACTIVE)
        assert result["ECHOTEXT"].rstrip() == ""

    def test_unknown_inactive_name_raises(self, conn: Connection) -> None:
        # Unknown parameter names error, never silently skipped, on every
        # SDK patch level: the core looks the name up first and raises the
        # lookup's own error (RFC_INVALID_PARAMETER, code 20).
        with pytest.raises(RfcError) as excinfo:
            conn.call("STFC_CONNECTION", NO_SUCH_PARAM=INACTIVE)
        assert excinfo.value.key == "RFC_INVALID_PARAMETER"
        assert excinfo.value.code == 20
        assert excinfo.value.sdk_function == "RfcGetParameterDescByName"


class TestCancel:
    def test_cancel_interrupts_blocked_call(self) -> None:
        # Fresh connection: a successful cancel closes it (SDK behavior),
        # so the shared class-scoped fixture must not be used here.
        c = Connection(dest=DEST)
        errors: list[BaseException] = []

        def worker() -> None:
            try:
                c.call("RFC_PING_AND_WAIT", SECONDS=10)
            except BaseException as e:  # recorded and re-asserted below
                errors.append(e)

        started = time.monotonic()
        t = threading.Thread(target=worker)
        t.start()
        # Let the worker get past the descriptor fetch and into RfcInvoke.
        time.sleep(1.5)
        c.cancel()
        t.join(timeout=30)
        elapsed = time.monotonic() - started

        assert not t.is_alive(), "worker never returned from the cancelled call"
        assert elapsed < 9, f"call was not interrupted (elapsed {elapsed:.1f}s)"
        # The cancelled call raises in its own thread: RFC_CANCELED comes
        # back in the communication-failure group (probe: code=7, group=4).
        assert len(errors) == 1
        assert isinstance(errors[0], CommunicationError)
        assert errors[0].key == "RFC_CANCELED"
        assert errors[0].sdk_function == "RfcInvoke"
        # Both seams agree the connection is gone.
        assert c.closed is True

    def test_cancel_without_call_in_flight_raises_and_leaves_open(self) -> None:
        c = Connection(dest=DEST)
        try:
            with pytest.raises(RfcError) as excinfo:
                c.cancel()
            assert excinfo.value.key == "RFC_ILLEGAL_STATE"
            assert excinfo.value.sdk_function == "RfcCancel"
            # Probe: a failed cancel does NOT close the connection.
            assert c.closed is False
            c.ping()
        finally:
            c.close()

    def test_cancel_on_closed_connection_raises_valueerror(self) -> None:
        c = Connection(dest=DEST)
        c.close()
        with pytest.raises(ValueError, match="closed"):
            c.cancel()
