"""Call options and the check_date/check_time config, as pyrfc 3.4 does them.

pyrfc's own test_options.py uses EAM_TASKLIST_GET_DETAIL, which this system
does not have, so these cases use STFC_CONNECTION/STFC_STRUCTURE. Every
expectation was measured on real pyrfc 3.4 first (`just test-pyrfc-real`),
which is what makes them pyrfc's and not ours (anti-circularity).

pyrfc behaviour pinned here (_cyrfc.pyx Connection.call):
- it deactivates the `not_requested` names BEFORE it fills the inputs, so a
  value for a deactivated scalar re-activates it (the SDK's rule) while a
  deactivated structure stays unsent;
- it never asks RfcIsParameterActive, so a deactivated output comes back
  with its initial value instead of being omitted.
"""

from __future__ import annotations

import pytest

from pyrfc import Connection, ExternalRuntimeError
from tests.compat.config import CONNECTION_PARAMS, skip_if_sdk_accepts_unknown_inactive_name

pytestmark = [pytest.mark.live, pytest.mark.compat]


class TestNotRequested:
    def test_deactivated_output_is_returned_initial(self, conn):
        # list form and plain str form behave the same.
        for option in (["ECHOTEXT"], "ECHOTEXT"):
            res = conn.call("STFC_CONNECTION", {"not_requested": option}, REQUTEXT="hi")
            assert res["ECHOTEXT"] == ""
            assert res["RESPTEXT"].startswith("SAP R/3")

        res = conn.call(
            "STFC_STRUCTURE",
            {"not_requested": ["ECHOSTRUCT", "RESPTEXT"]},
            IMPORTSTRUCT={"RFCINT4": 5},
        )
        assert res["RESPTEXT"] == ""
        assert res["ECHOSTRUCT"]["RFCINT4"] == 0
        assert res["ECHOSTRUCT"]["RFCTIME"] == "000000"
        assert res["ECHOSTRUCT"]["RFCDATE"] == ""
        # RFCTABLE was not deactivated: the backend appended its row.
        assert res["RFCTABLE"][0]["RFCCHAR1"] == "X"

    def test_deactivated_input_with_value(self, conn):
        # Scalar: the setter re-activates it, so the value is sent.
        res = conn.call("STFC_CONNECTION", {"not_requested": ["REQUTEXT"]}, REQUTEXT="hello")
        assert res["ECHOTEXT"] == "hello"
        # Structure: filled through its own handle, it stays inactive.
        res = conn.call(
            "STFC_STRUCTURE", {"not_requested": ["IMPORTSTRUCT"]}, IMPORTSTRUCT={"RFCINT4": 5}
        )
        assert res["ECHOSTRUCT"]["RFCINT4"] == 0
        # Table: not sent, and the result holds the caller's own local row.
        res = conn.call(
            "STFC_STRUCTURE", {"not_requested": ["RFCTABLE"]}, RFCTABLE=[{"RFCINT4": 7}]
        )
        assert len(res["RFCTABLE"]) == 1
        assert res["RFCTABLE"][0]["RFCINT4"] == 7
        assert res["RFCTABLE"][0]["RFCCHAR1"] == ""

    def test_unknown_parameter_name(self, conn):
        skip_if_sdk_accepts_unknown_inactive_name()
        # The name reaches the SDK verbatim: case matters.
        for name in ("NOPE", "echotext"):
            with pytest.raises(ExternalRuntimeError) as ex:
                conn.call("STFC_CONNECTION", {"not_requested": [name]}, REQUTEXT="hi")
            assert ex.value.args == (f"field '{name}' not found",)
            assert str(ex.value) == (
                f"20 (rc=20): key=RFC_INVALID_PARAMETER, message=field '{name}' not found "
                "[MSG: class=, type=, number=, v1-4:=;;;]"
            )
        assert conn.alive
        # Deactivation runs before the fill, so it wins over a bad input.
        with pytest.raises(ExternalRuntimeError) as ex:
            conn.call("STFC_STRUCTURE", {"not_requested": ["NOPE"]}, IMPORTSTRUCT=[1])
        assert ex.value.args == ("field 'NOPE' not found",)

    def test_tuple_is_not_a_list(self, conn):
        # Only a list is iterated; a tuple is taken as ONE name.
        with pytest.raises(AttributeError) as ex:
            conn.call("STFC_CONNECTION", {"not_requested": ("ECHOTEXT",)}, REQUTEXT="hi")
        assert ex.value.args == ("'tuple' object has no attribute 'encode'",)


def test_unknown_option_key_is_ignored(conn):
    for options in ({"xyz": 1}, {}, {"timeout": None}):
        res = conn.call("STFC_CONNECTION", options, REQUTEXT="hi")
        assert res["ECHOTEXT"] == "hi"


def test_check_date_time_false_passes_input_to_sdk():
    # With the check off, pyrfc sets DATE/TIME with the fixed-width
    # RfcSetDate/RfcSetTime: 8 (6) characters are read, whatever they are.
    cases = {
        ("check_date", "RFCDATE"): [
            ("20161232", "20161232"),  # out of range: accepted
            ("abcdefgh", "abcdefgh"),  # not a date: accepted
            ("201612311", "20161231"),  # too long: cut
            ("2016123", "2016123\x00"),  # one short: the NUL terminator is read
        ],
        ("check_time", "RFCTIME"): [
            ("123466", "123466"),
            ("abcdef", "abcdef"),
            ("12345", "12345\x00"),
        ],
    }
    for (key, field), values in cases.items():
        conn = Connection(config={key: False}, **CONNECTION_PARAMS)
        for value, expected in values:
            res = conn.call("STFC_STRUCTURE", IMPORTSTRUCT={field: value})
            assert res["ECHOSTRUCT"][field] == expected
        conn.close()


def test_config_all_defaults():
    config = {
        "dtime": False,
        "check_date": True,
        "check_time": True,
        "rstrip": True,
        "return_import_params": False,
        "timeout": None,
    }
    conn = Connection(config=config, **CONNECTION_PARAMS)
    assert conn.alive
    # check_date=True still rejects a bad date client-side.
    with pytest.raises(TypeError) as ex:
        conn.call("STFC_STRUCTURE", IMPORTSTRUCT={"RFCDATE": "20161232"})
    assert ex.value.args[0] == "date value required, received"
    conn.close()
