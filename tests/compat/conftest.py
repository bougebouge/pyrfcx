"""Fixtures for the adapted pyrfc compat test suite.

Tests here are adapted from pyrfc 3.4 tests/ (https://github.com/SAP/PyRFC).
They import the compat facade `pyrfc` and run against the live destination
PYRFCX_TEST_DEST (default TEST_NW750; SNC via sapnwrfc.ini in the repo root).
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

from pyrfc import Connection
from tests.compat.config import CONNECTION_PARAMS

if TYPE_CHECKING:
    from collections.abc import Iterator


@pytest.fixture
def conn() -> Iterator[Connection]:
    """A fresh compat-facade connection per test (mirrors pyrfc's setup_method)."""
    c = Connection(**CONNECTION_PARAMS)
    assert c.alive
    yield c
    c.close()
    assert not c.alive
