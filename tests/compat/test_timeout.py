"""The `timeout` config and call option, cancel() and get_nwrfclib_version().

pyrfc's own suite has no timeout test beyond the config getter. Every
expectation here was measured on real pyrfc 3.4 first (`just
test-pyrfc-real`), which is what makes them pyrfc's and not ours
(anti-circularity). RFC_PING_AND_WAIT waits SECONDS on the backend.

pyrfc behaviour pinned here (_cyrfc.pyx Connection.call, cancel_connection):
- a threading.Timer starts before the input fill and calls
  cancel_connection, which runs RfcCancel in a new thread;
- the cancelled RfcInvoke fails with RFC_CANCELED (COMMUNICATION_FAILURE
  group), so pyrfc reconnects and rewrites the message to
  "Connection was canceled: <old handle>. New handle: <new handle>";
- the per-call option wins over the config, and an explicit None disables
  the timer; `timeout` is read from `options` at every call;
- an RfcCancel error (RFC_ILLEGAL_STATE, no call in flight) is raised in
  the cancel thread, never in the caller's thread.

Delta: the handle values are run-varying in pyrfc. The facade holds no SDK
handle and prints two object ids instead, so only the message shape is
pinned.
"""

from __future__ import annotations

import re
import sys
import threading
import time

import pytest

from pyrfc import CommunicationError, Connection, ExternalRuntimeError, get_nwrfclib_version
from tests.compat.config import CONNECTION_PARAMS
from tests.test_sdk_pin import vendored_dll_version

pytestmark = [pytest.mark.live, pytest.mark.compat]

_CANCELED_MESSAGE = re.compile(r"Connection was canceled: \d+\. New handle: \d+")


def _assert_canceled(error: CommunicationError) -> None:
    assert type(error) is CommunicationError
    assert error.code == 7
    assert error.key == "RFC_CANCELED"
    assert len(error.args) == 1
    assert _CANCELED_MESSAGE.fullmatch(error.args[0])
    assert error.message == error.args[0]
    assert str(error) == (
        f"7 (rc=7): key=RFC_CANCELED, message={error.message} "
        "[MSG: class=, type=, number=, v1-4:=;;;]"
    )


@pytest.fixture
def thread_errors(monkeypatch: pytest.MonkeyPatch) -> list[BaseException | None]:
    """Exceptions raised in other threads (threading.excepthook)."""
    errors: list[BaseException | None] = []
    monkeypatch.setattr(threading, "excepthook", lambda args: errors.append(args.exc_value))
    return errors


def test_call_timeout_cancels_and_reconnects(conn, thread_errors):
    conv_id = conn.get_connection_attributes()["cpicConvId"]
    start = time.monotonic()
    with pytest.raises(CommunicationError) as ex:
        conn.call("RFC_PING_AND_WAIT", {"timeout": 1}, SECONDS=5)
    assert time.monotonic() - start < 4
    _assert_canceled(ex.value)
    # The cancel closed the connection; pyrfc opened a new one.
    assert conn.alive
    assert conn.get_connection_attributes()["cpicConvId"] != conv_id
    assert conn.call("STFC_CONNECTION", REQUTEXT="hi")["ECHOTEXT"] == "hi"
    assert thread_errors == []


def test_config_timeout_is_read_per_call(thread_errors):
    conn = Connection(config={"timeout": 60}, **CONNECTION_PARAMS)
    # Editing `options` after the open changes the timeout of the next call.
    conn.options["timeout"] = 0.5
    with pytest.raises(CommunicationError) as ex:
        conn.call("RFC_PING_AND_WAIT", SECONDS=3)
    _assert_canceled(ex.value)
    assert conn.alive
    conn.close()
    assert thread_errors == []


def test_call_option_overrides_config(thread_errors):
    conn = Connection(config={"timeout": 0.5}, **CONNECTION_PARAMS)
    # The per-call option wins; an explicit None disables the config timer.
    assert conn.call("RFC_PING_AND_WAIT", {"timeout": None}, SECONDS=1) == {}
    assert conn.call("RFC_PING_AND_WAIT", {"timeout": 3}, SECONDS=1) == {}
    conn.close()
    assert thread_errors == []


def test_timeout_not_reached(conn, thread_errors):
    # The timer is cancelled when the call returns: no cancel fires later.
    assert conn.call("STFC_CONNECTION", {"timeout": 0.5}, REQUTEXT="hi")["ECHOTEXT"] == "hi"
    time.sleep(0.8)
    assert conn.alive
    assert conn.call("STFC_CONNECTION", REQUTEXT="hi")["ECHOTEXT"] == "hi"
    assert thread_errors == []


def test_cancel(conn, thread_errors):
    # cancel() from another thread cancels the call in flight.
    threading.Timer(0.5, conn.cancel).start()
    with pytest.raises(CommunicationError) as ex:
        conn.call("RFC_PING_AND_WAIT", SECONDS=3)
    _assert_canceled(ex.value)
    assert conn.alive
    assert thread_errors == []
    # With no call in flight, RfcCancel fails in the cancel thread; the
    # caller sees nothing and the connection stays open.
    conn.cancel()
    deadline = time.monotonic() + 2
    while not thread_errors and time.monotonic() < deadline:
        time.sleep(0.05)
    assert len(thread_errors) == 1
    error = thread_errors[0]
    assert type(error) is ExternalRuntimeError
    assert error.code == 19
    assert error.key == "RFC_ILLEGAL_STATE"
    assert error.args == ("RFC connection is currently not in call.",)
    assert conn.alive
    assert conn.call("STFC_CONNECTION", REQUTEXT="hi")["ECHOTEXT"] == "hi"


def test_get_nwrfclib_version(conn):
    # RfcGetVersion read from the vendored library by ctypes, without the
    # stack under test, plus sys.platform.
    major, minor, patch_level, _ = vendored_dll_version()
    version = get_nwrfclib_version()
    assert version == {
        "major": major,
        "minor": minor,
        "patchLevel": patch_level,
        "platform": sys.platform,
    }
    assert conn.version == version
