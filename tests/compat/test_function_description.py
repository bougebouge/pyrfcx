"""pyrfc 3.4's function descriptions: get_function_description, the
FunctionDescription / TypeDescription classes and the two enums.

Every expected value here was measured on real pyrfc 3.4 first
(anti-circularity), on TEST_NW750 (release 750) and TEST_NW757 (release 757).
pyrfc's own suite has no client test for these members.

pyrfc's source tests lengths against ``long``, which only Cython 3.0 defines
(as ``int``). Released pyrfc 3.4 was built with Cython 3.0, so the oracle must
be too (the justfile pins it).
"""

from __future__ import annotations

import ast

import pytest

from pyrfc import (
    ABAPApplicationError,
    ExternalApplicationError,
    ExternalRuntimeError,
    FunctionDescription,
    RfcFieldType,
    RfcParameterDirection,
    TypeDescription,
)
from tests.compat.config import BACKEND

pytestmark = [pytest.mark.live, pytest.mark.compat]


FIELD_TYPES = {
    "RFCTYPE_CHAR": 0,
    "RFCTYPE_DATE": 1,
    "RFCTYPE_BCD": 2,
    "RFCTYPE_TIME": 3,
    "RFCTYPE_BYTE": 4,
    "RFCTYPE_TABLE": 5,
    "RFCTYPE_NUM": 6,
    "RFCTYPE_FLOAT": 7,
    "RFCTYPE_INT": 8,
    "RFCTYPE_INT2": 9,
    "RFCTYPE_INT1": 10,
    "RFCTYPE_NULL": 14,
    "RFCTYPE_ABAPOBJECT": 16,
    "RFCTYPE_STRUCTURE": 17,
    "RFCTYPE_DECF16": 23,
    "RFCTYPE_DECF34": 24,
    "RFCTYPE_XMLDATA": 28,
    "RFCTYPE_STRING": 29,
    "RFCTYPE_XSTRING": 30,
    "RFCTYPE_INT8": 31,
    "RFCTYPE_UTCLONG": 32,
    "RFCTYPE_UTCSECOND": 33,
    "RFCTYPE_UTCMINUTE": 34,
    "RFCTYPE_DTDAY": 35,
    "RFCTYPE_DTWEEK": 36,
    "RFCTYPE_DTMONTH": 37,
    "RFCTYPE_TSECOND": 38,
    "RFCTYPE_TMINUTE": 39,
    "RFCTYPE_CDAY": 40,
}

# STFC_STRUCTURE's line type RFCTEST:
# (name, field_type, nuc_length, nuc_offset, uc_length, uc_offset, decimals)
RFCTEST = [
    ("RFCFLOAT", "RFCTYPE_FLOAT", 8, 0, 8, 0, 16),
    ("RFCCHAR1", "RFCTYPE_CHAR", 1, 8, 2, 8, 0),
    ("RFCINT2", "RFCTYPE_INT2", 2, 10, 2, 10, 0),
    ("RFCINT1", "RFCTYPE_INT1", 1, 12, 1, 12, 0),
    ("RFCCHAR4", "RFCTYPE_CHAR", 4, 13, 8, 14, 0),
    ("RFCINT4", "RFCTYPE_INT", 4, 20, 4, 24, 0),
    ("RFCHEX3", "RFCTYPE_BYTE", 3, 24, 3, 28, 0),
    ("RFCCHAR2", "RFCTYPE_CHAR", 2, 27, 4, 32, 0),
    ("RFCTIME", "RFCTYPE_TIME", 6, 29, 12, 36, 0),
    ("RFCDATE", "RFCTYPE_DATE", 8, 35, 16, 48, 0),
    ("RFCDATA1", "RFCTYPE_CHAR", 50, 43, 100, 64, 0),
    ("RFCDATA2", "RFCTYPE_CHAR", 50, 93, 100, 164, 0),
]

# Our echo FM's structure ZPYRFCX_FE_INPUT. A 754+ backend adds ZUTCLONG.
ZPYRFCX_FE_INPUT = [
    ("ZFLTP", "RFCTYPE_FLOAT", 8, 0, 8, 0, 16),
    ("ZFLTP_MIN", "RFCTYPE_FLOAT", 8, 8, 8, 8, 16),
    ("ZFLTP_MAX", "RFCTYPE_FLOAT", 8, 16, 8, 16, 16),
    ("ZDEC", "RFCTYPE_BCD", 7, 24, 7, 24, 2),
    ("ZCURR", "RFCTYPE_BCD", 7, 31, 7, 31, 2),
    ("ZQUAN", "RFCTYPE_BCD", 7, 38, 7, 38, 4),
    ("ZQUAN_SIGN", "RFCTYPE_BCD", 7, 45, 7, 45, 3),
    ("ZDECF16_MIN", "RFCTYPE_DECF16", 8, 56, 8, 56, 0),
    ("ZDECF16_MAX", "RFCTYPE_DECF16", 8, 64, 8, 64, 0),
    ("ZDECF34_MIN", "RFCTYPE_DECF34", 16, 80, 16, 80, 0),
    ("ZDECF34_MAX", "RFCTYPE_DECF34", 16, 96, 16, 96, 0),
    ("ZRAW", "RFCTYPE_BYTE", 17, 112, 17, 112, 0),
    ("ZRAWSTRING", "RFCTYPE_XSTRING", 8, 136, 8, 136, 0),
    ("ZSHLP_MAT1", "RFCTYPE_CHAR", 40, 144, 80, 144, 0),
    ("ZNUMC", "RFCTYPE_NUM", 10, 184, 20, 224, 0),
]
if BACKEND.utclong:
    ZPYRFCX_FE_INPUT.append(("ZUTCLONG", "RFCTYPE_UTCLONG", 8, 200, 8, 248, 0))

STFCCPLXT_T = [
    ("I", "RFCTYPE_INT", 4, 0, 4, 0, 0),
    ("C", "RFCTYPE_CHAR", 10, 4, 20, 4, 0),
    ("STR", "RFCTYPE_STRING", 8, 16, 8, 24, 0),
    ("XSTR", "RFCTYPE_XSTRING", 8, 24, 8, 32, 0),
]


def assert_type_description(td, name, nuc_length, uc_length, fields):
    assert type(td) is TypeDescription
    assert vars(td) == {
        "fields": td.fields,
        "name": name,
        "nuc_length": nuc_length,
        "uc_length": uc_length,
    }
    assert list(vars(td)) == ["fields", "name", "nuc_length", "uc_length"]
    assert repr(td) == (
        f"<TypeDescription '{name}' with {len(fields)} fields "
        f"(n/uclength={nuc_length}/{uc_length})>"
    )
    expected = [
        {
            "name": f[0],
            "field_type": f[1],
            "nuc_length": f[2],
            "nuc_offset": f[3],
            "uc_length": f[4],
            "uc_offset": f[5],
            "decimals": f[6],
            "type_description": None,
        }
        for f in fields
    ]
    assert td.fields == expected
    assert [list(f) for f in td.fields] == [list(f) for f in expected]


def param(name, parameter_type, direction, nuc_length, uc_length, text, **extra):
    p = {
        "name": name,
        "parameter_type": parameter_type,
        "direction": direction,
        "nuc_length": nuc_length,
        "uc_length": uc_length,
        "decimals": 0,
        "default_value": "",
        "parameter_text": text,
        "optional": False,
        "type_description": None,
    }
    p.update(extra)
    return p


def without_types(params):
    return [{k: v for k, v in p.items() if k != "type_description"} for p in params]


def test_stfc_structure(conn):
    fd = conn.get_function_description("STFC_STRUCTURE")
    assert type(fd) is FunctionDescription
    assert repr(fd) == "<FunctionDescription 'STFC_STRUCTURE' with 4 params>"
    assert list(vars(fd)) == ["name", "parameters"]
    assert fd.name == "STFC_STRUCTURE"
    assert without_types(fd.parameters) == without_types(
        [
            param(
                "ECHOSTRUCT", "RFCTYPE_STRUCTURE", "RFC_EXPORT", 144, 264, "Exporting structure"
            ),
            param(
                "RESPTEXT", "RFCTYPE_CHAR", "RFC_EXPORT", 255, 510, "Exporting response message"
            ),
            param(
                "IMPORTSTRUCT", "RFCTYPE_STRUCTURE", "RFC_IMPORT", 144, 264, "Importing structure"
            ),
            param(
                "RFCTABLE", "RFCTYPE_TABLE", "RFC_TABLES", 144, 264, "Importing/exporting table"
            ),
        ]
    )
    assert [list(p) for p in fd.parameters] == [list(param("P", "", "", 0, 0, ""))] * 4
    assert fd.parameters[1]["type_description"] is None
    for i in (0, 2, 3):
        assert_type_description(fd.parameters[i]["type_description"], "RFCTEST", 144, 264, RFCTEST)
    # Each parameter gets its own TypeDescription object.
    assert fd.parameters[0]["type_description"] is not fd.parameters[2]["type_description"]


def test_defaults_optional_and_deep_tables(conn):
    fd = conn.get_function_description("ZPYRFCX_FE_DATATYPES")
    nuc, uc = BACKEND.fe_input_length
    assert repr(fd) == "<FunctionDescription 'ZPYRFCX_FE_DATATYPES' with 3 params>"
    assert without_types(fd.parameters) == without_types(
        [
            param("ES_OUTPUT", "RFCTYPE_STRUCTURE", "RFC_EXPORT", nuc, uc, ""),
            param("IS_INPUT", "RFCTYPE_STRUCTURE", "RFC_IMPORT", nuc, uc, "", optional=True),
            param(
                "IV_COUNT", "RFCTYPE_INT", "RFC_IMPORT", 4, 4, "", default_value="0", optional=True
            ),
        ]
    )
    for i in (0, 1):
        assert_type_description(
            fd.parameters[i]["type_description"], "ZPYRFCX_FE_INPUT", nuc, uc, ZPYRFCX_FE_INPUT
        )
    assert fd.parameters[2]["type_description"] is None

    fd = conn.get_function_description("STFC_DEEP_TABLE")
    text = "Complex Test Structure for Veris"
    assert without_types(fd.parameters) == without_types(
        [
            param("EXPORT_TAB", "RFCTYPE_TABLE", "RFC_EXPORT", 32, 40, text),
            param(
                "RESPTEXT",
                "RFCTYPE_CHAR",
                "RFC_EXPORT",
                255,
                510,
                "List processing, contents of selected line",
            ),
            param("IMPORT_TAB", "RFCTYPE_TABLE", "RFC_IMPORT", 32, 40, text),
        ]
    )
    for i in (0, 2):
        assert_type_description(
            fd.parameters[i]["type_description"], "STFCCPLXT_T", 32, 40, STFCCPLXT_T
        )


def test_scalar_only_functions_and_name_handling(conn):
    fd = conn.get_function_description("RFC_PING")
    assert repr(fd) == "<FunctionDescription 'RFC_PING' with 0 params>"
    assert fd.parameters == []
    # The name is upper-cased before the SDK sees it.
    fd = conn.get_function_description("stfc_connection")
    assert fd.name == "STFC_CONNECTION"
    assert repr(fd) == "<FunctionDescription 'STFC_CONNECTION' with 3 params>"


def test_errors(conn):
    with pytest.raises(ABAPApplicationError) as ex:
        conn.get_function_description("NO_SUCH_FM_XYZ")
    assert ex.value.args == ("ID:FL Type:E Number:046 NO_SUCH_FM_XYZ",)
    assert str(ex.value) == (
        "5 (rc=5): key=FU_NOT_FOUND, message=ID:FL Type:E Number:046 NO_SUCH_FM_XYZ "
        "[MSG: class=FL, type=E, number=046, v1-4:=NO_SUCH_FM_XYZ;;;]"
    )
    with pytest.raises(ExternalApplicationError) as ex:
        conn.get_function_description("")
    assert str(ex.value) == (
        '20 (rc=20): key=RFC_INVALID_PARAMETER, message=Parameter "Function name" is empty '
        "[MSG: class=, type=, number=, v1-4:=;;;]"
    )
    with pytest.raises(AttributeError) as ex:
        conn.get_function_description(123)
    assert ex.value.args == ("'int' object has no attribute 'upper'",)
    # No reconnect: the same connection is still open.
    assert conn.alive
    conn.close()
    with pytest.raises(ExternalRuntimeError) as ex:
        conn.get_function_description("STFC_CONNECTION")
    assert str(ex.value) == (
        "13 (rc=13): key=RFC_INVALID_HANDLE, message=An invalid handle "
        "'RFC_CONNECTION_HANDLE' was passed to the API call "
        "[MSG: class=, type=, number=, v1-4:=;;;]"
    )
    # The name is upper-cased before the closed handle is noticed.
    with pytest.raises(AttributeError):
        conn.get_function_description(123)


def test_enums():
    assert {m.name: m.value for m in RfcFieldType} == FIELD_TYPES
    assert list(RfcFieldType.__members__) == list(FIELD_TYPES)
    assert [(m.name, m.value) for m in RfcParameterDirection] == [
        ("RFC_IMPORT", 1),
        ("RFC_EXPORT", 2),
        ("RFC_CHANGING", 3),
        ("RFC_TABLES", 7),
    ]
    # Plain Enum, not IntEnum.
    assert RfcFieldType.RFCTYPE_CHAR != 0
    assert repr(RfcParameterDirection.RFC_TABLES) == "<RfcParameterDirection.RFC_TABLES: 7>"


def must_be_in(message: str, prefix: str) -> set[str]:
    """The set printed at the end of pyrfc's message; its order varies."""
    assert message.startswith(prefix)
    return ast.literal_eval(message[len(prefix) :].strip("'"))


def test_function_description_class():
    fd = FunctionDescription("Z")
    assert fd.add_parameter("P", "RFCTYPE_CHAR", "RFC_IMPORT", 1, 2) is None
    assert fd.parameters == [param("P", "RFCTYPE_CHAR", "RFC_IMPORT", 1, 2, "")]
    assert repr(fd) == "<FunctionDescription 'Z' with 1 params>"
    # Lengths are not checked.
    fd.add_parameter("Q", "RFCTYPE_CHAR", "RFC_IMPORT", "x", None)
    assert fd.parameters[1]["nuc_length"] == "x"
    assert repr(FunctionDescription(None)) == "<FunctionDescription 'None' with 0 params>"

    def raised(*args, **kwargs):
        with pytest.raises(TypeError) as ex:
            fd.add_parameter(*args, **kwargs)
        (message,) = ex.value.args
        return message

    assert raised("", "RFCTYPE_CHAR", "RFC_IMPORT", 1, 2) == (
        "field 'name' (string)  should be from 1-30 chars."
    )
    assert raised("P" * 31, "RFCTYPE_CHAR", "RFC_IMPORT", 1, 2) == (
        f"field 'name' (string) {'P' * 31} should be from 1-30 chars."
    )
    message = raised("P", "CHAR", "RFC_IMPORT", 1, 2)
    assert must_be_in(message, "'parameter_type' (string) 'CHAR' must be in ") == set(FIELD_TYPES)
    # An enum member is not accepted, only its name.
    message = raised("P", RfcFieldType.RFCTYPE_CHAR, "RFC_IMPORT", 1, 2)
    assert message.startswith("'parameter_type' (string) 'RfcFieldType.RFCTYPE_CHAR' must be in ")
    message = raised("P", "RFCTYPE_CHAR", "IMPORT", 1, 2)
    assert must_be_in(message, "'direction' (string) 'IMPORT' must be in ") == {
        "RFC_IMPORT",
        "RFC_EXPORT",
        "RFC_CHANGING",
        "RFC_TABLES",
    }
    assert raised("P", "RFCTYPE_CHAR", "RFC_IMPORT", 1, 2, default_value="d" * 31) == (
        f"'default_value' (string) '{'d' * 31}' must not exceed 30 chars."
    )
    # pyrfc's message is not an f-string: it shows the placeholder.
    assert raised("P", "RFCTYPE_CHAR", "RFC_IMPORT", 1, 2, parameter_text="t" * 80) == (
        "'parameter_text' (string) '{parameter_text}' must not exceed 79 chars."
    )
    assert len(fd.parameters) == 2


def test_type_description_class():
    td = TypeDescription("T", 1, 2)
    assert td.add_field("F", "RFCTYPE_CHAR", 1, 2, 0, 0) is None
    # isinstance check: True passes, and decimals is not checked.
    td.add_field("G", "RFCTYPE_CHAR", True, 2, 0, 0, decimals="x")
    assert td.fields[1]["nuc_length"] is True
    assert td.fields[1]["decimals"] == "x"
    assert repr(td) == "<TypeDescription 'T' with 2 fields (n/uclength=1/2)>"

    def raised(fn, *args):
        with pytest.raises(TypeError) as ex:
            fn(*args)
        (message,) = ex.value.args
        return message

    assert raised(td.add_field, "F" * 31, "RFCTYPE_CHAR", 1, 2, 0, 0) == (
        f"field 'name' (string) '{'F' * 31}' should be from 1-30 chars."
    )
    message = raised(td.add_field, "F", "X", 1, 2, 0, 0)
    assert must_be_in(message, "'field_type' (string) 'X' must be in ") == set(FIELD_TYPES)
    assert raised(td.add_field, "F", "RFCTYPE_CHAR", 1, 2.0, 0, 0) == (
        "field 'F' length '2.0' must be of type integer"
    )
    # The constructor checks the exact type: True fails there.
    assert raised(TypeDescription, "X", 1.5, 2) == "field 'X' length '1.5' must be of type integer"
    assert raised(TypeDescription, "X", True, 2) == (
        "field 'X' length 'True' must be of type integer"
    )
    assert len(td.fields) == 2


def test_type_description_names():
    td = TypeDescription("T", 1, 2)
    # A field without a name is skipped before any other check.
    assert td.add_field("", "X", None, None, None, None) is None
    assert td.fields == []
    with pytest.raises(TypeError) as ex:
        TypeDescription("", 1, 2)
    assert ex.value.args == ("field 'name' (string) '' should be from 1-30 chars.",)
    with pytest.raises(TypeError) as ex:
        TypeDescription("X" * 31, 1, 2)
    assert ex.value.args == (f"field 'name' (string) '{'X' * 31}' should be from 1-30 chars.",)
    with pytest.raises(TypeError) as ex:
        TypeDescription(5, 1, 2)
    assert ex.value.args == ("object of type 'int' has no len()",)
