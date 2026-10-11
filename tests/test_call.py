"""Live tests for Connection.call() — requires the SAP destination
PYRFCX_TEST_DEST (default TEST_NW750)."""

from __future__ import annotations

from typing import TYPE_CHECKING

import pytest

from pyrfcx import INACTIVE, AbapApplicationError, AbapRuntimeError, Connection, RfcError
from tests.conftest import BACKEND, DEST

if TYPE_CHECKING:
    from collections.abc import Callable

pytestmark = pytest.mark.live


# ── STFC_STRUCTURE ──────────────────────────────────────────────────────
# ECHOSTRUCT = verbatim copy of IMPORTSTRUCT (before ABAP mutation).
# RFCTABLE  = one row: the mutated IMPORTSTRUCT (ints +1, float +1,
#             RFCCHAR1='X', RFCCHAR4=SY-SYSID, RFCHEX3=X'F1F2F3',
#             RFCCHAR2='YZ', RFCTIME=SY-UZEIT, RFCDATE=SY-DATUM).
# RESPTEXT  = system info string (contains "Sysid:").


class TestStfcStructure:
    def test_echo_scalars(self, conn: Connection) -> None:
        imp = {
            "RFCFLOAT": 3.14,
            "RFCINT4": 42,
            "RFCINT2": -1,
            "RFCINT1": 7,
            "RFCCHAR1": "A",
            "RFCCHAR2": "BC",
            "RFCCHAR4": "DEFG",
            "RFCDATE": "20260714",
            "RFCTIME": "120000",
            "RFCHEX3": b"\xde\xad\x00",
        }
        result = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)
        echo = result["ECHOSTRUCT"]

        assert echo["RFCFLOAT"] == pytest.approx(3.14)
        assert echo["RFCINT4"] == 42
        assert echo["RFCINT2"] == -1
        assert echo["RFCINT1"] == 7
        assert echo["RFCCHAR1"] == "A"
        assert echo["RFCCHAR2"] == "BC"
        assert echo["RFCCHAR4"] == "DEFG"
        assert echo["RFCDATE"] == "20260714"
        assert echo["RFCTIME"] == "120000"
        assert echo["RFCHEX3"] == b"\xde\xad\x00"

    def test_echo_unicode(self, conn: Connection) -> None:
        imp = {"RFCCHAR1": "Ü", "RFCCHAR4": "Ü日", "RFCDATA1": "𝄞𠀀"}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCCHAR1"] == "Ü"
        assert echo["RFCCHAR4"] == "Ü日"
        assert echo["RFCDATA1"] == "𝄞𠀀"

    def test_table_has_mutated_row(self, conn: Connection) -> None:
        imp = {"RFCFLOAT": 10.0, "RFCINT4": 100}
        result = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)
        table = result["RFCTABLE"]
        assert len(table) == 1
        row = table[0]
        assert row["RFCFLOAT"] == pytest.approx(11.0)
        assert row["RFCINT4"] == 101
        assert row["RFCHEX3"] == b"\xf1\xf2\xf3"

    def test_table_rows_share_key_objects(self, conn: Connection) -> None:
        """Every row dict reuses one `str` per field name, on both profiles."""
        rows = [{"RFCINT4": 1, "RFCCHAR4": "AAAA"}, {"RFCINT4": 2, "RFCCHAR4": "BBBB"}]
        native = conn.call("STFC_STRUCTURE", RFCTABLE=rows)
        compat = conn._call_compat("STFC_STRUCTURE", {"RFCTABLE": rows})
        for result in (native, compat):
            table = result["RFCTABLE"]
            assert len(table) == 3  # the two input rows + the mutated row
            assert list(table[0]) == list(table[1]) == list(result["ECHOSTRUCT"])
            assert all(a is b for a, b in zip(table[0], table[1], strict=True))
            assert all(a is b for a, b in zip(table[0], result["ECHOSTRUCT"], strict=True))
            assert (table[0]["RFCINT4"], table[0]["RFCCHAR4"]) == (1, "AAAA")
            assert (table[1]["RFCINT4"], table[1]["RFCCHAR4"]) == (2, "BBBB")

    def test_large_tables_in_order_and_sharing_keys(self, conn: Connection) -> None:
        """Tables larger than one conversion chunk: rows in order, one key
        object per field across chunks, on both profiles. ZPYRFCX_MEM_ROWS
        ($TMP) returns rows of F01..F40 valued R<7-digit row>F<2-digit field>XXXX.
        """

        def expected(n: int) -> list[dict[str, str]]:
            return [
                {f"F{f:02d}": f"R{r:07d}F{f:02d}XXXX" for f in range(1, 41)}
                for r in range(1, n + 1)
            ]

        # 1000 is not a multiple of the chunk size, 512 is, 0 is the empty table.
        for n in (1000, 512, 0):
            native = conn.call("ZPYRFCX_MEM_ROWS", IV_ROWS=n, IV_FILL=15)
            compat = conn._call_compat("ZPYRFCX_MEM_ROWS", {"IV_ROWS": n, "IV_FILL": 15})
            for result in (native, compat):
                rows = result["ET_ROWS"]
                assert rows == expected(n)
                if n:
                    assert all(a is b for a, b in zip(rows[0], rows[-1], strict=True))

    def test_large_table_conversion_error_is_the_earliest_row(self, conn: Connection) -> None:
        """A conversion error in a table spanning several chunks raises the
        error of the earliest bad row, as converting in row order did, and
        leaves the connection usable. Real pyrfc 3.4 differs: it converts
        from the last row backwards and raises the LAST bad row's error.
        """
        # STFC_STRUCTURE echoes RFCTABLE (plus one row). Two dates `dtime`
        # cannot parse: row 10 is in the first chunk, row 590 in the last.
        # strptime names only the unparsed tail: "40" for row 10, "99" for 590.
        rows: list[dict[str, object]] = [{"RFCINT4": i} for i in range(600)]
        rows[10]["RFCDATE"] = "20231340"
        rows[590]["RFCDATE"] = "20231399"
        params = {"RFCTABLE": rows}
        with pytest.raises(ValueError, match=r"unconverted data remains: 40$"):
            conn._call_compat("STFC_STRUCTURE", params, check_date=False, dtime=True)
        assert not conn.closed
        table = conn.call("STFC_STRUCTURE", RFCTABLE=rows[:10])["RFCTABLE"]
        assert [r["RFCINT4"] for r in table[:10]] == list(range(10))

    def test_float_precision(self, conn: Connection) -> None:

        imp = {"RFCFLOAT": 1.23456789012345}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCFLOAT"] == pytest.approx(1.23456789012345)

    def test_int_boundaries(self, conn: Connection) -> None:
        imp = {"RFCINT1": 0, "RFCINT2": -32768, "RFCINT4": 2147483646}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCINT1"] == 0
        assert echo["RFCINT2"] == -32768
        assert echo["RFCINT4"] == 2147483646

    def test_invalid_date(self, conn: Connection) -> None:
        imp = {"RFCDATE": "99999999"}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCDATE"] == "99999999"

    def test_initial_date(self, conn: Connection) -> None:
        imp = {"RFCDATE": "00000000"}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCDATE"] == "00000000"

    def test_empty_date_sent_as_initial_in_native_mode(self, conn: Connection) -> None:
        # Native sends an empty DATE as the initial value, so every SDK patch
        # level gives the same answer (compat skips the setter instead).
        # Structure field and table row both go through the field setter.
        result = conn.call(
            "STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATE": ""}, RFCTABLE=[{"RFCDATE": ""}]
        )
        assert result["ECHOSTRUCT"]["RFCDATE"] == "00000000"
        assert result["RFCTABLE"][0]["RFCDATE"] == "00000000"

    def test_invalid_time(self, conn: Connection) -> None:
        imp = {"RFCTIME": "995959"}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCTIME"] == "995959"

        imp = {"RFCTIME": "000000"}
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT=imp)["ECHOSTRUCT"]
        assert echo["RFCTIME"] == "000000"

    def test_empty_time_sent_as_initial_in_native_mode(self, conn: Connection) -> None:
        # As for DATE above.
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCTIME": ""})["ECHOSTRUCT"]
        assert echo["RFCTIME"] == "000000"


# ── STFC_STRING ─────────────────────────────────────────────────────────
# IMPORTING QUESTION TYPE STRING, EXPORTING MYANSWER TYPE STRING.
# "How are you" → "Fine thanks.", anything else → "Sorry, I can not understand you".


class TestStfcString:
    def test_known_answer(self, conn: Connection) -> None:
        result = conn.call("STFC_STRING", QUESTION="How are you")
        assert result["MYANSWER"] == "Fine thanks."

    def test_unknown_answer(self, conn: Connection) -> None:
        result = conn.call("STFC_STRING", QUESTION="hello")
        assert result["MYANSWER"] == "Sorry, I can not understand you"

    def test_unicode_input(self, conn: Connection) -> None:
        result = conn.call("STFC_STRING", QUESTION="𝄞 café 你好")
        assert result["MYANSWER"] == "Sorry, I can not understand you"


# ── STFC_XSTRING ────────────────────────────────────────────────────────
# IMPORTING QUESTION TYPE XSTRING, EXPORTING MYANSWER TYPE XSTRING.
# Always returns one constant, which differs by release: X'DEAD' on 750 (the
# ABAP literal 'DEAD MEET' is parsed as hex pairs DE AD, then the space
# terminates parsing — confirmed in SAP GUI), X'DEADEED0' on 757.


class TestStfcXstring:
    def test_str_input_rejected(self, conn: Connection) -> None:
        """Native keeps its own TypeError; pyrfc's template is compat-only."""
        with pytest.raises(TypeError) as ei:
            conn.call("STFC_XSTRING", QUESTION="DEAD")
        assert ei.value.args == ("QUESTION: XSTRING field requires bytes input, got String",)

    def test_output_is_bytes(self, conn: Connection) -> None:
        result = conn.call("STFC_XSTRING", QUESTION=b"\xca\xfe")
        assert isinstance(result["MYANSWER"], bytes)
        assert result["MYANSWER"] == BACKEND.stfc_xstring_answer


# ── STFC_DEEP_TABLE ─────────────────────────────────────────────────────
# EXPORT_TAB = copy of IMPORT_TAB + one appended row
#   {I: 10, C: "Appended", STR: SY-DATUM, XSTR: X'DEADBEEF'}.
# Tests STRING and XSTRING (variable-length) types in table context.


class TestStfcDeepTable:
    def test_echo_and_appended_row(self, conn: Connection) -> None:
        rows = [
            {"I": 1, "C": "Row1", "STR": "hello", "XSTR": b"\x01\x02"},
            {"I": 2, "C": "Row2", "STR": "world", "XSTR": b"\xff"},
        ]
        result = conn.call("STFC_DEEP_TABLE", IMPORT_TAB=rows)
        tab = result["EXPORT_TAB"]
        assert len(tab) == 3

        assert tab[0]["I"] == 1
        assert tab[0]["C"] == "Row1"
        assert tab[0]["STR"] == "hello"
        assert tab[0]["XSTR"] == b"\x01\x02"

        assert tab[1]["I"] == 2
        assert tab[1]["STR"] == "world"
        assert tab[1]["XSTR"] == b"\xff"

        assert tab[2]["I"] == 10
        assert tab[2]["C"] == "Appended"
        assert tab[2]["XSTR"] == b"\xde\xad\xbe\xef"

    def test_empty_input_table(self, conn: Connection) -> None:
        result = conn.call("STFC_DEEP_TABLE", IMPORT_TAB=[])
        tab = result["EXPORT_TAB"]
        assert len(tab) == 1
        assert tab[0]["I"] == 10

    def test_unicode_in_string_field(self, conn: Connection) -> None:
        rows = [{"I": 0, "C": "", "STR": "𝄞 café 你好", "XSTR": b""}]
        result = conn.call("STFC_DEEP_TABLE", IMPORT_TAB=rows)
        assert result["EXPORT_TAB"][0]["STR"] == "𝄞 café 你好"


# ── Edge cases ──────────────────────────────────────────────────────────


class TestEdgeCases:
    def test_unknown_parameter_name(self, conn: Connection) -> None:
        with pytest.raises(RfcError, match="not found") as exc_info:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCFLOAT": 0.0}, DOESNOTEXIST="x")
        assert exc_info.value.key == "RFC_INVALID_PARAMETER"
        assert exc_info.value.sdk_function == "RfcGetParameterDescByName"

    def test_set_export_only_parameter(self, conn: Connection) -> None:
        with pytest.raises(ValueError, match="EXPORT-only") as exc_info:
            conn.call("STFC_STRUCTURE", ECHOSTRUCT={"RFCFLOAT": 1.0})
        assert "ECHOSTRUCT" in str(exc_info.value)

    def test_multiple_calls_same_connection(self, conn: Connection) -> None:
        for i in range(5):
            result = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": i})
            assert result["ECHOSTRUCT"]["RFCINT4"] == i

    def test_surrogate_in_char1_raises_value_error(self, conn: Connection) -> None:
        """𝄞 (U+1D11E) = 2 UTF-16 code units → over-long for CHAR(1).
        Native mode rejects before any setter call."""
        with pytest.raises(ValueError, match="RFCCHAR1") as exc_info:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR1": "𝄞"})
        assert "2 UTF-16 units" in str(exc_info.value)
        assert "CHAR(1)" in str(exc_info.value)

    def test_surrogate_in_char1_compat_truncates(self, conn: Connection) -> None:
        """In compat mode, the SDK truncates — producing a broken surrogate → U+FFFD."""
        result = conn._call_compat("STFC_STRUCTURE", {"IMPORTSTRUCT": {"RFCCHAR1": "𝄞"}})
        assert result["ECHOSTRUCT"]["RFCCHAR1"] == "�"

    def test_char50_overflow_raises_value_error(self, conn: Connection) -> None:
        """Over-long CHAR input raises ValueError natively."""
        long_text = "A" * 100
        with pytest.raises(ValueError, match="RFCDATA1"):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATA1": long_text})

    def test_char50_overflow_compat_truncates(self, conn: Connection) -> None:
        """In compat mode, the SDK silently truncates."""
        long_text = "A" * 100
        result = conn._call_compat("STFC_STRUCTURE", {"IMPORTSTRUCT": {"RFCDATA1": long_text}})
        echoed = result["ECHOSTRUCT"]["RFCDATA1"]
        assert echoed == "A" * 50

    def test_exact_width_passes(self, conn: Connection) -> None:
        """Exact-width CHAR input is accepted without error."""
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": "ABCD"})["ECHOSTRUCT"]
        assert echo["RFCCHAR4"] == "ABCD"

    def test_overlong_error_names_field_path(self, conn: Connection) -> None:
        """Error message contains the field path including parent."""
        with pytest.raises(ValueError) as exc_info:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": "ABCDE"})
        msg = str(exc_info.value)
        assert "IMPORTSTRUCT.RFCCHAR4" in msg
        assert "5 UTF-16 units" in msg
        assert "CHAR(4)" in msg

    def test_overlong_in_table_row_names_index(self, conn: Connection) -> None:
        """Error message for table rows includes [i].FIELD."""
        rows = [
            {"I": 0, "C": "A" * 300, "STR": "", "XSTR": b""},
        ]
        with pytest.raises(ValueError) as exc_info:
            conn.call("STFC_DEEP_TABLE", IMPORT_TAB=rows)
        msg = str(exc_info.value)
        assert "IMPORT_TAB[0].C" in msg

    def test_string_input_never_length_checked(self, conn: Connection) -> None:
        """STRING fields accept any length — no length check."""
        result = conn.call("STFC_STRING", QUESTION="A" * 1000)
        assert "MYANSWER" in result

    def test_native_char_output_no_trailing_spaces(self, conn: Connection) -> None:
        """Native output CHAR has no trailing pad blanks."""
        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": "AB"})["ECHOSTRUCT"]
        assert echo["RFCCHAR4"] == "AB"
        assert not echo["RFCCHAR4"].endswith(" ")


# ── integer inputs ────────────────────────────────────────────────


class TestIntegerInputs:
    def test_str_rejected_for_int_field(self, conn: Connection) -> None:
        """Strings no longer convert to integer targets."""
        with pytest.raises(TypeError, match="INT4 field requires int input") as exc_info:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": "42"})
        assert "IMPORTSTRUCT.RFCINT4" in str(exc_info.value)

    def test_float_and_decimal_rejected_for_int_field(self, conn: Connection) -> None:
        from decimal import Decimal

        with pytest.raises(TypeError, match="INT4 field requires int input"):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": 42.0})
        with pytest.raises(TypeError, match="INT4 field requires int input"):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": Decimal("42")})

    def test_out_of_range_raises_value_error(self, conn: Connection) -> None:
        """Native range check fires before any setter call."""
        with pytest.raises(ValueError, match=r"256 out of range for INT1 \(0\.\.255\)") as ei:
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT1": 256})
        assert "IMPORTSTRUCT.RFCINT1" in str(ei.value)

        with pytest.raises(ValueError, match=r"out of range for INT1"):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT1": -1})

        with pytest.raises(ValueError, match=r"out of range for INT2"):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT2": 32768})

        for value in (2**31, -(2**31) - 1):
            with pytest.raises(ValueError, match=r"out of range for INT4"):
                conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": value})

        # The compat facade raises pyrfc's OverflowError here; native must not.
        for value in (2**63, -(2**63) - 1):
            with pytest.raises(ValueError, match=r"CV_INT8: .* out of range for INT8"):
                conn.call("SEWS_REF_DIRECT_TYPES", CV_INT8=value)

    def test_beyond_i128_raises_overflow_error(self, conn: Connection) -> None:
        """Ints beyond i128 fail at conversion with Python's natural error."""
        with pytest.raises(OverflowError):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": 10**40})

    def test_int_subclass_marshals_numerically(self, conn: Connection) -> None:
        """IntEnum extracts by value, not str() — version-independent."""
        import enum

        class Answer(enum.IntEnum):
            YES = 3

        echo = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT4": Answer.YES})["ECHOSTRUCT"]
        assert echo["RFCINT4"] == 3

    def test_int_rejected_for_char_field(self, conn: Connection) -> None:
        """Int is only valid for numeric targets."""
        with pytest.raises(TypeError, match="int input is not valid for CHAR field"):
            conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4": 42})

    def test_out_of_range_compat_delegates_to_sdk(self, conn: Connection) -> None:
        """Compat: 256 → INT1 goes through RfcSetInt (pyrfc's setter)
        and the SDK judges it — never our client-side ValueError/TypeError."""
        try:
            conn._call_compat("STFC_STRUCTURE", {"IMPORTSTRUCT": {"RFCINT1": 256}})
        except (ValueError, TypeError):
            pytest.fail("compat must delegate to the SDK, not fail client-side")
        except RfcError:
            pass  # SDK-side conversion error — the pyrfc-parity outcome


# ── decimal inputs ────────────────────────────────────────────────
# ZPYRFCX_FE_DATATYPES echoes IS_INPUT into ES_OUTPUT; ZDEC is a BCD field.


class TestDecimalInputs:
    def test_non_numeric_string_rejected_before_the_sdk(self, conn: Connection) -> None:
        """RfcSetString on a BCD target *silently garbage-converts* a
        non-numeric string ('A' → 1.00), so the native path must reject it
        before the setter — a ValueError naming the field, not corruption."""
        for bad in ("A", "x9y", "1.2.3", "", "1,5"):
            with pytest.raises(ValueError, match="BCD field requires a numeric value") as ei:
                conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZDEC": bad})
            assert "IS_INPUT.ZDEC" in str(ei.value)

    def test_valid_numbers_still_round_trip(self, conn: Connection) -> None:
        """The guard must not reject any legitimate input: str, float,
        Decimal and int all reach the backend unchanged (modulo field
        precision)."""
        from decimal import Decimal

        out = conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZDEC": "12345.67"})["ES_OUTPUT"]
        assert out["ZDEC"] == Decimal("12345.67")
        for good in (Decimal("99.50"), 1234.56, 42):
            out = conn.call("ZPYRFCX_FE_DATATYPES", IS_INPUT={"ZDEC": good})["ES_OUTPUT"]
            assert out["ZDEC"] == Decimal(str(good))


# ── Error handling ──────────────────────────────────────────────────────


class TestErrors:
    def test_nonexistent_fm(self, conn: Connection) -> None:
        with pytest.raises(AbapApplicationError) as exc_info:
            conn.call("DOES_NOT_EXIST_ZZZZZ")
        assert exc_info.value.key == "FU_NOT_FOUND"
        # The error names the SDK function that returned it.
        assert exc_info.value.sdk_function == "RfcGetFunctionDesc"

    def test_connection_alive_after_abap_exception(self, conn: Connection) -> None:
        with pytest.raises(AbapApplicationError):
            conn.call("DOES_NOT_EXIST_ZZZZZ")
        result = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCFLOAT": 0.0})
        assert "ECHOSTRUCT" in result

    def test_embedded_nul_in_a_name_or_setting_raises_before_the_sdk(
        self, conn: Connection
    ) -> None:
        """The SDK reads names and settings only up to a NUL, so pyrfcx
        rejects one. The message never shows the text (it can be a password),
        and the NUL check wins over the unknown-name error."""
        cases: list[tuple[Callable[[], object], str]] = [
            (
                lambda: Connection(dest=DEST, passwd="se\0cret"),
                "connection parameter 'passwd'",
            ),
            (lambda: Connection(**{"de\0st": DEST}), "connection parameter name"),
            (lambda: conn.call("STFC_CONNECTION\0X"), "function name"),
            (lambda: conn.describe_function("STFC_CONNECTION\0X"), "function name"),
            (lambda: conn.call("STFC_CONNECTION", **{"REQUTEXT\0X": "x"}), "parameter name"),
            (lambda: conn.call("STFC_CONNECTION", **{"NOPE\0": "x"}), "parameter name"),
            (lambda: conn.call("STFC_CONNECTION", **{"ECHOTEXT\0X": INACTIVE}), "parameter name"),
            (
                lambda: conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCCHAR4\0X": "A"}),
                "field name at IMPORTSTRUCT",
            ),
            (
                lambda: conn.call("STFC_STRUCTURE", RFCTABLE=[{"RFCCHAR4": "A"}, {"NOPE\0": "A"}]),
                "field name at RFCTABLE[1]",
            ),
        ]
        for action, what in cases:
            with pytest.raises(ValueError) as exc_info:
                action()
            assert type(exc_info.value) is ValueError
            assert str(exc_info.value) == f"embedded null character in {what}"
        # Nothing reached the SDK: this connection and a new one both work.
        assert conn.call("STFC_CONNECTION", REQUTEXT="ok")["ECHOTEXT"] == "ok"
        with Connection(dest=DEST) as fresh:
            assert fresh.call("STFC_CONNECTION", REQUTEXT="ok")["ECHOTEXT"] == "ok"

    @pytest.mark.dumps  # ST22: CONVT_OVERFLOW (SAPLSTFC)
    def test_int1_overflow_raises_runtime_error(self) -> None:
        """STFC_STRUCTURE adds 1 to RFCINT1 — 255 overflows the byte.

        Uses its own connection since AbapRuntimeError kills it.
        """
        with Connection(dest=DEST) as c:
            with pytest.raises(AbapRuntimeError, match="Overflow"):
                c.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCINT1": 255})
            assert c.closed


# ── RFC_PING ────────────────────────────────────────────────────────────


class TestPing:
    def test_ping(self, conn: Connection) -> None:
        result = conn.call("RFC_PING")
        assert isinstance(result, dict)
