"""pyrfc 3.4's set_cryptolib_path, measured on real pyrfc 3.4 first.

The crypto library path is process-wide and final: the SDK keeps the first
path it gets, even when that library fails to load. So each test runs its
calls in a fresh Python process, and this pytest process never makes them.
Each child process then opens an SNC connection, to prove the call does not
disturb SNC.

The library path comes from SNC_LIB (SNC_LIB_64 on Windows): the same SAP
crypto library serves SNC and TLS.
"""

from __future__ import annotations

from pathlib import Path

import pytest

from tests.conftest import DEST
from tests.isolated import crypto_library_path, run_isolated

CRYPTOLIB = crypto_library_path()

pytestmark = [
    pytest.mark.live,
    pytest.mark.compat,
    pytest.mark.skipif(not CRYPTOLIB, reason="SNC_LIB (the SAP crypto library path) is not set"),
]

SCRIPT = """
import json
from pathlib import Path
from pyrfc import Connection, set_cryptolib_path

results = []
def attempt(*args):
    try:
        results.append(["returned", repr(set_cryptolib_path(*args))])
    except Exception as e:
        results.append([type(e).__name__, repr(e.args), str(e)])

{calls}

conn = Connection(dest={dest!r})
results.append(["snc", conn.call("STFC_CONNECTION", REQUTEXT="snc")["ECHOTEXT"]])
conn.close()
print(json.dumps(results))
"""


def run_calls(*calls: str) -> list[list[str]]:
    return run_isolated(SCRIPT.format(calls="\n".join(calls), dest=DEST))


def type_error(*args: object) -> list[str]:
    return ["TypeError", repr(args), str(args)]


def rfc_error(cls: str, code: int, key: str, message: str) -> list[str]:
    return [
        cls,
        repr((message,)),
        f"{code} (rc={code}): key={key}, message={message} "
        "[MSG: class=, type=, number=, v1-4:=;;;]",
    ]


def already_set(path: str) -> list[str]:
    return rfc_error(
        "ExternalRuntimeError",
        20,
        "RFC_INVALID_PARAMETER",
        f'Cannot set path to crypto library. It is already set to "{path}"',
    )


def test_path_is_final(tmp_path):
    other = tmp_path / "other.lib"
    other.write_text("")
    assert run_calls(
        f"attempt({CRYPTOLIB!r})",
        # The same path again is accepted, another path is not.
        f"attempt({CRYPTOLIB!r})",
        f"attempt({str(other)!r})",
    ) == [
        ["returned", "None"],
        ["returned", "None"],
        already_set(CRYPTOLIB),
        ["snc", "snc"],
    ]


def test_argument_errors_and_failed_load(tmp_path):
    junk = tmp_path / "junk.lib"
    junk.write_text("not a library")
    missing = str(tmp_path / "missing.lib")
    assert run_calls(
        "attempt(1)",
        f"attempt(Path({CRYPTOLIB!r}))",
        f"attempt({missing!r})",
        f"attempt({str(tmp_path)!r})",
        "attempt('')",
        # An embedded NUL fails the file check, before the SDK.
        "attempt('a\\x00b')",
        f"attempt({str(junk)!r})",
        # The failed load has set the path all the same.
        f"attempt({CRYPTOLIB!r})",
    ) == [
        # pyrfc's message names sapnwrfc.ini.
        type_error("sapnwrfc.ini path is not a string:", 1),
        type_error("sapnwrfc.ini path is not a string:", Path(CRYPTOLIB)),
        type_error("Crypto library not found:", missing),
        type_error("Crypto library not found:", str(tmp_path)),
        type_error("Crypto library not found:", ""),
        type_error("Crypto library not found:", "a\0b"),
        rfc_error(
            "CommunicationError",
            1,
            "RFC_COMMUNICATION_FAILURE",
            f'Could not initialize TLS library with path "{junk}". '
            "See socket trace for more details.",
        ),
        already_set(str(junk)),
        ["snc", "snc"],
    ]
