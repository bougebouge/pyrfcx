"""Thread-safety stress tests, live (destination PYRFCX_TEST_DEST).

16 threads open their own connections, wait until all are open, then run
the same calls in the same order, so each function's first lookup is a
cold miss on many threads at once. Each thread then runs `ping` and
`close`.
Once on the native `pyrfcx.Connection`, once on the pyrfc facade.

A broken SDK metadata cache does not fail a test: the process dies at
exit, after every test passed. Run this file in a loop and check the exit
code of each run.
"""

from __future__ import annotations

import threading
from typing import TYPE_CHECKING, Any

import pyrfc
import pyrfcx
from tests.conftest import DEST

if TYPE_CHECKING:
    from collections.abc import Callable

THREADS = 16
MEM = "ZPYRFCX_MEM_ROWS"

# Function modules and inputs for the plain calls. The facade test walks
# them backwards, so it starts on a name the native test looked up last.
CALLS: list[tuple[str, dict[str, Any]]] = [
    ("STFC_CONNECTION", {"REQUTEXT": "threads"}),
    ("STFC_STRUCTURE", {}),
    ("RFC_SYSTEM_INFO", {}),
    ("RFC_PING", {}),
    ("STFC_DEEP_TABLE", {"IMPORT_TAB": [{"I": 1, "C": "Row1", "STR": "s", "XSTR": b"\xab"}]}),
    (MEM, {"IV_ROWS": 5, "IV_FILL": 15}),
]


def run_threads(worker: Callable[[int, threading.Barrier], None]) -> None:
    """Run `worker(i, barrier)` on THREADS threads that start together;
    re-raise the first error any of them hit. A worker waits on the barrier
    once more, after its open."""
    barrier = threading.Barrier(THREADS)
    errors: list[BaseException] = []

    def body(i: int) -> None:
        try:
            barrier.wait()
            worker(i, barrier)
        except BaseException as exc:
            errors.append(exc)
            barrier.abort()

    threads = [threading.Thread(target=body, args=(i,)) for i in range(THREADS)]
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    if errors:
        raise errors[0]


def native_worker(_i: int, barrier: threading.Barrier) -> None:
    conn = pyrfcx.Connection(dest=DEST)
    barrier.wait()
    for name, inputs in CALLS:
        conn.call(name, **inputs)
        assert conn.describe_function(name).name == name
    conn.ping()
    conn.close()
    assert conn.closed


def facade_worker(_i: int, barrier: threading.Barrier) -> None:
    conn = pyrfc.Connection(dest=DEST)
    barrier.wait()
    for name, inputs in reversed(CALLS):
        conn.call(name, **inputs)
        assert conn.get_function_description(name.lower()).name == name
    conn.ping()
    conn.close()
    assert not conn.alive


def test_native_connections_on_many_threads() -> None:
    run_threads(native_worker)


def test_facade_connections_on_many_threads() -> None:
    run_threads(facade_worker)
