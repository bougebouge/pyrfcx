"""Table outputs larger than one conversion chunk, on the compat facade.

pyrfcx converts a top-level output table chunk by chunk, last rows first.
This test pins what a caller sees regardless: every row in its original
order. It uses no pyrfcx API, so real pyrfc 3.4 runs it unchanged (`just
test-compat-both`), which proves the expectations are pyrfc's.

ZPYRFCX_MEM_ROWS ($TMP, TEST_NW750) returns IV_ROWS rows of 40 CHAR(15)
fields F01..F40, valued R<7-digit row>F<2-digit field>XXXX, rows from 1.
"""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

if TYPE_CHECKING:
    from pyrfc import Connection

pytestmark = [pytest.mark.live, pytest.mark.compat]


def mem_row(row: int) -> dict[str, str]:
    return {f"F{f:02d}": f"R{row:07d}F{f:02d}XXXX" for f in range(1, 41)}


def test_large_table_rows_in_order(conn: Connection) -> None:
    # 1000 is not a multiple of the chunk size, 512 is, 0 is the empty table.
    for n in (1000, 512, 0):
        rows = conn.call("ZPYRFCX_MEM_ROWS", IV_ROWS=n, IV_FILL=15)["ET_ROWS"]
        assert len(rows) == n
        assert rows == [mem_row(r) for r in range(1, n + 1)]
    # Spot-check both ends and the rows around the chunk boundaries.
    rows = conn.call("ZPYRFCX_MEM_ROWS", IV_ROWS=1000, IV_FILL=15)["ET_ROWS"]
    for i in (0, 231, 232, 487, 488, 743, 744, 999):
        assert rows[i]["F01"] == f"R{i + 1:07d}F01XXXX"
        assert rows[i]["F40"] == f"R{i + 1:07d}F40XXXX"
