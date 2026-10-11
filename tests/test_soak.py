"""Soak tests, live (destination PYRFCX_TEST_DEST). Marker `soak`.

Long loops that look for leaks: process memory over many calls on one
connection and over many open/close cycles, timer threads left behind by
the `timeout` call option, and memory over many cancel-and-reconnect
rounds. Each test prints its samples and its slope.

The `pyrfc` stack is whatever `import pyrfc` finds: the pyrfcx facade in
the dev venv, or real pyrfc 3.4 when the file runs from the real pyrfc
venv. The native stack skips where `pyrfcx` is not installed.

Excluded by default; run with `pytest tests/test_soak.py -m soak -s`.
Run with the RFC and CPIC traces off: a trace distorts memory and time.
"""

from __future__ import annotations

import ctypes
import gc
import importlib.util
import os
import subprocess
import sys
import threading
import time
from dataclasses import dataclass
from typing import TYPE_CHECKING, Any

import pytest

from tests.conftest import DEST

if TYPE_CHECKING:
    from collections.abc import Callable, Iterator

pytestmark = [pytest.mark.live, pytest.mark.soak]

MEM = "ZPYRFCX_MEM_ROWS"
MEM_SMALL: dict[str, Any] = {"IV_ROWS": 1000, "IV_FILL": 15}
# Large enough that the call is still running when a 1 s timer fires.
MEM_BIG: dict[str, Any] = {"IV_ROWS": 200000, "IV_FILL": 15}


# ── process memory ─────────────────────────────────────────────────────────


def rss_bytes() -> int:
    """The current resident set size of this process (not the high-water mark)."""
    if sys.platform == "win32":

        class _Counters(ctypes.Structure):
            _fields_ = [
                ("cb", ctypes.c_ulong),
                ("PageFaultCount", ctypes.c_ulong),
                ("PeakWorkingSetSize", ctypes.c_size_t),
                ("WorkingSetSize", ctypes.c_size_t),
                ("QuotaPeakPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPagedPoolUsage", ctypes.c_size_t),
                ("QuotaPeakNonPagedPoolUsage", ctypes.c_size_t),
                ("QuotaNonPagedPoolUsage", ctypes.c_size_t),
                ("PagefileUsage", ctypes.c_size_t),
                ("PeakPagefileUsage", ctypes.c_size_t),
            ]

        counters = _Counters()
        counters.cb = ctypes.sizeof(counters)
        kernel32 = ctypes.windll.kernel32
        kernel32.GetCurrentProcess.restype = ctypes.c_void_p
        ok = kernel32.K32GetProcessMemoryInfo(
            ctypes.c_void_p(kernel32.GetCurrentProcess()), ctypes.byref(counters), counters.cb
        )
        assert ok, "K32GetProcessMemoryInfo failed"
        return int(counters.WorkingSetSize)
    if sys.platform.startswith("linux"):
        with open("/proc/self/statm") as f:
            pages = int(f.read().split()[1])
        return pages * os.sysconf("SC_PAGE_SIZE")
    out = subprocess.run(
        ["ps", "-o", "rss=", "-p", str(os.getpid())], capture_output=True, text=True, check=True
    )
    return int(out.stdout.strip()) * 1024


def slope(points: list[tuple[int, int]]) -> float:
    """Least-squares slope of y over x."""
    n = len(points)
    mx = sum(x for x, _ in points) / n
    my = sum(y for _, y in points) / n
    sxx = sum((x - mx) ** 2 for x, _ in points)
    sxy = sum((x - mx) * (y - my) for x, y in points)
    return sxy / sxx


def soak(
    label: str,
    unit: str,
    rounds: int,
    every: int,
    fit_from: int,
    limit: int,
    body: Callable[[int], None],
) -> None:
    """Run `body(i)` for i in 0..rounds-1. Sample RSS before round 0 and
    after every `every` rounds, after a gc. Fit the slope over the samples
    from round `fit_from` on and assert it is below `limit` bytes per unit."""
    samples: list[tuple[int, int]] = []

    def sample(done: int) -> None:
        gc.collect()
        samples.append((done, rss_bytes()))

    sample(0)
    t0 = time.perf_counter()
    for i in range(rounds):
        body(i)
        if (i + 1) % every == 0:
            sample(i + 1)
    elapsed = time.perf_counter() - t0
    fit = [(x, y) for x, y in samples if x >= fit_from]
    per_unit = slope(fit)
    print(f"\n[{label}] {rounds} {unit}s in {elapsed:.1f} s")
    for x, y in samples:
        print(f"[{label}]   after {x:4d}: RSS {y / 1024:10.0f} KiB")
    print(
        f"[{label}] start {samples[0][1] / 1024:.0f} KiB, end {samples[-1][1] / 1024:.0f} KiB, "
        f"slope {per_unit:.0f} B/{unit} over {unit}s {fit_from}..{rounds} (limit {limit} B)"
    )
    assert per_unit < limit, f"{label}: RSS grows {per_unit:.0f} B per {unit}"


# ── stacks ─────────────────────────────────────────────────────────────────


@dataclass(frozen=True)
class Stack:
    name: str
    open: Callable[[], Any]
    is_closed: Callable[[Any], bool]


def pyrfc_stack() -> Stack:
    pyrfc = pytest.importorskip("pyrfc")
    # The facade is built on pyrfcx; real pyrfc 3.4 is not.
    name = "facade" if importlib.util.find_spec("pyrfcx") is not None else "pyrfc-3.4"
    return Stack(name, lambda: pyrfc.Connection(dest=DEST), lambda c: not c.alive)


def native_stack() -> Stack:
    pyrfcx = pytest.importorskip("pyrfcx")
    return Stack("native", lambda: pyrfcx.Connection(dest=DEST), lambda c: c.closed)


@pytest.fixture(params=["native", "pyrfc"])
def stack(request: pytest.FixtureRequest) -> Stack:
    return native_stack() if request.param == "native" else pyrfc_stack()


@pytest.fixture
def pyrfc_conn() -> Iterator[tuple[str, Any]]:
    """A pyrfc-API connection (facade or real pyrfc) and the stack name."""
    s = pyrfc_stack()
    conn = s.open()
    yield s.name, conn
    conn.close()


# ── tests ──────────────────────────────────────────────────────────────────


def test_memory_over_calls_on_one_connection(stack: Stack) -> None:
    conn = stack.open()
    try:
        assert len(conn.call(MEM, **MEM_SMALL)["ET_ROWS"]) == MEM_SMALL["IV_ROWS"]

        def body(_i: int) -> None:
            conn.call(MEM, **MEM_SMALL)

        soak(f"calls/{stack.name}", "call", 500, 50, 100, 4096, body)
    finally:
        conn.close()


def test_memory_over_open_close_cycles(stack: Stack) -> None:
    def body(_i: int) -> None:
        conn = stack.open()
        conn.call("RFC_PING")
        conn.close()
        assert stack.is_closed(conn)

    soak(f"open-close/{stack.name}", "cycle", 200, 20, 40, 8192, body)


def cancel_threads() -> list[str]:
    return [
        t.name
        for t in threading.enumerate()
        if isinstance(t, threading.Timer) or "cancel" in t.name or "Timer" in t.name
    ]


def wait_for_threads(count: int, seconds: float) -> None:
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        if threading.active_count() == count and not cancel_threads():
            return
        time.sleep(0.05)


def test_timeout_leaves_no_thread(pyrfc_conn: tuple[str, Any]) -> None:
    import pyrfc

    name, conn = pyrfc_conn
    start_count = threading.active_count()
    start_names = {t.name for t in threading.enumerate()}
    for _ in range(200):
        conn.call("RFC_PING", options={"timeout": 30})
    time.sleep(1)
    after_pings = threading.active_count()
    print(f"\n[timeout/{name}] threads: start {start_count}, after 200 pings {after_pings}")
    assert after_pings == start_count
    assert not cancel_threads()
    assert {t.name for t in threading.enumerate()} == start_names

    t0 = time.perf_counter()
    with pytest.raises(pyrfc.CommunicationError) as info:
        conn.call(MEM, options={"timeout": 1}, **MEM_BIG)
    raised_after = time.perf_counter() - t0
    assert info.value.key == "RFC_CANCELED"
    wait_for_threads(start_count, 5)
    after_cancel = threading.active_count()
    print(
        f"[timeout/{name}] cancelled call raised after {raised_after:.2f} s; "
        f"threads after cancel {after_cancel}, cancel threads left {cancel_threads()}"
    )
    assert not cancel_threads()
    assert after_cancel == start_count
    conn.call("RFC_PING")


def test_memory_over_cancel_and_reconnect(pyrfc_conn: tuple[str, Any]) -> None:
    import pyrfc

    name, conn = pyrfc_conn
    latencies: list[float] = []

    def body(_i: int) -> None:
        old = getattr(conn, "_conn", None) if name == "facade" else None
        t0 = time.perf_counter()
        with pytest.raises(pyrfc.CommunicationError) as info:
            conn.call(MEM, options={"timeout": 1}, **MEM_BIG)
        latencies.append(time.perf_counter() - t0)
        assert info.value.key == "RFC_CANCELED"
        if old is not None:
            assert conn._conn is not old
        assert conn.call("RFC_PING") == {}

    soak(f"reconnect/{name}", "round", 50, 10, 10, 51200, body)
    print(
        f"[reconnect/{name}] cancelled call raised after "
        f"min {min(latencies):.2f} s, max {max(latencies):.2f} s"
    )
