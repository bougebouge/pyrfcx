# SPDX-FileCopyrightText: 2013 SAP SE Srdjan Boskovic <srdjan.boskovic@sap.com>
# SPDX-FileCopyrightText: 2026 the pyrfcx contributors
#
# SPDX-License-Identifier: Apache-2.0
#
# Adapted from pyrfc 3.4. This file is modified; the docstring lists the changes.

"""Test data adapted from pyrfc 3.4's tests/config.py (mine + adapt).

Pure test constants and converters only — the connection setup lives in
conftest.py (our system is SNC/destination-based, not user/passwd). pyrfc's
config calls set_ini_file_directory("tests"); our sapnwrfc.ini is in the
repository root, the SDK's default directory for this suite, so the call is
dropped (test_ini_file.py tests the function itself).

The destination and the per-backend expectations live here and serve the
native suite too (tests/conftest.py imports them).
"""

from __future__ import annotations

import dataclasses
import datetime
import os

import pytest

# pyrfc's suite reads user/passwd params from pyrfc.cfg; our system is
# SNC-only, addressed by destination (sapnwrfc.ini in the repo root).
# PYRFCX_TEST_DEST selects the destination for the native and the compat
# suite alike; this is the only place that reads it.
DEST: str = os.environ.get("PYRFCX_TEST_DEST", "TEST_NW750")
CONNECTION_PARAMS: dict[str, str] = {"dest": DEST}


@dataclasses.dataclass(frozen=True)
class Backend:
    """The expected values that differ between the test backends."""

    # Release of the backend, as the connection attributes report it.
    partner_rel: str
    # Kernel release of the backend (pyrfc's `kernelRel`, our `kernel_rel`).
    kernel_rel: str
    client: str
    sys_number: str
    # nuc/uc length of ZPYRFCX_FE_INPUT. The copy on a 754+ backend has the
    # extra field ZUTCLONG.
    fe_input_length: tuple[int, int]
    # The constant answer of STFC_XSTRING; its ABAP source differs by release.
    stfc_xstring_answer: bytes
    # UTCLONG exists from release 754.
    utclong: bool


BACKENDS: dict[str, Backend] = {
    "TEST_NW750": Backend("750", "754", "040", "10", (200, 248), b"\xde\xad", utclong=False),
    "TEST_NW757": Backend(
        "757", "793", "040", "15", (208, 256), b"\xde\xad\xee\xd0", utclong=True
    ),
}
if DEST not in BACKENDS:
    raise RuntimeError(f"PYRFCX_TEST_DEST={DEST!r}: add its expected values to BACKENDS")
BACKEND: Backend = BACKENDS[DEST]

requires_utclong = pytest.mark.skipif(
    not BACKEND.utclong, reason="UTCLONG needs release 754+; run with PYRFCX_TEST_DEST=TEST_NW757"
)


def skip_if_sdk_accepts_unknown_inactive_name() -> None:
    """Skip the running test when the SDK accepts an unknown inactive name.

    The facade, like pyrfc, lets the SDK judge an inactive parameter name.
    Call this inside the test, after a connection is open: on Linux, loading
    the SDK before the client library has pinned ICU exits the process.
    The import is here, not at the top: tests.test_sdk_pin imports
    tests.conftest, which imports this module.
    """
    from tests.test_sdk_pin import vendored_dll_version

    major, _, patch_level, _ = vendored_dll_version()
    if (major, patch_level) < (7500, 15):
        pytest.skip(
            "SDK below 7.50 PL 15 accepts an unknown name in RfcSetParameterActive "
            "(release notes 7.50.15)"
        )


# SAP user queried where pyrfc's suite queried user "demo"
# (BAPI_USER_GET_DETAIL) — that demo user does not exist on our system.
# Set PYRFCX_TEST_USER to a user that exists on the test system; the tests
# that need it skip when it is unset.
TEST_USER: str | None = os.environ.get("PYRFCX_TEST_USER")
requires_test_user = pytest.mark.skipif(
    TEST_USER is None, reason="set PYRFCX_TEST_USER to a user on the test system"
)

RFC_MATH = {
    "RFC_INT1": {"MIN": 0, "MAX": 255},
    "RFC_INT2": {"NEG": -32768, "POS": 32767},
    "RFC_INT4": {"NEG": -2147483648, "POS": 2147483647},
    "RFC_INT8": {"NEG": -9223372036854775808, "POS": 9223372036854775807},
    "FLOAT": {
        "NEG": {"MIN": "-2.2250738585072014E-308", "MAX": "-1.7976931348623157E+308"},
        "POS": {"MIN": "2.2250738585072014E-308", "MAX": "1.7976931348623157E+308"},
    },
    "DECF16": {
        "NEG": {"MIN": "-1E-383", "MAX": "-9.999999999999999E+384"},
        "POS": {"MIN": "1E-383", "MAX": "9.999999999999999E+384"},
    },
    "DECF34": {
        "NEG": {"MIN": "-1E-6143", "MAX": "-9.999999999999999999999999999999999E+6144"},
        "POS": {"MIN": "1E-6143", "MAX": "9.999999999999999999999999999999999E+6144"},
    },
    "DATE": {"MIN": "00010101", "MAX": "99991231"},
    "TIME": {"MIN": "000000", "MAX": "235959"},
}


def ABAP_to_python_date(abap_date: str) -> datetime.date:
    return datetime.datetime.strptime(abap_date, "%Y%m%d").date()


def ABAP_to_python_time(abap_time: str) -> datetime.time:
    return datetime.datetime.strptime(abap_time, "%H%M%S").time()


def python_to_ABAP_date(py_date: datetime.date) -> str:
    return f"{py_date.year:04d}{py_date.month:02d}{py_date.day:02d}"


def python_to_ABAP_time(py_time: datetime.time) -> str:
    return f"{py_time.hour:02d}{py_time.minute:02d}{py_time.second:02d}"


UNICODETEST = "ทดสอบสร้างลูกค้าจากภายนอกครั้งที่" * 7
UNICODE1 = "四周远处都"
BYTEARRAY_TEST = bytearray.fromhex("01414243444549500051fdfeff")
BYTES_TEST = bytes(BYTEARRAY_TEST)
