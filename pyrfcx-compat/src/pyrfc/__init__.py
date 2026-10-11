"""pyrfc — compatibility facade over pyrfcx.

Reproduces the observable behavior of pyrfc 3.4 (https://github.com/SAP/PyRFC),
checked against real pyrfc 3.4.
"""

from __future__ import annotations

import sys
from enum import Enum
from os.path import isfile, join
from threading import Thread, Timer
from typing import Any, NoReturn

import pyrfcx
from pyrfc._exception import (
    ABAPApplicationError,
    ABAPRuntimeError,
    CommunicationError,
    ExternalApplicationError,
    ExternalAuthorizationError,
    ExternalRuntimeError,
    LogonError,
    RFCError,
    RFCLibError,
    RFCTypeError,
)

__version__ = "3.4"
__version_info__ = ("3", "4")

__all__ = [
    "ABAPApplicationError",
    "ABAPRuntimeError",
    "CommunicationError",
    "Connection",
    "ExternalApplicationError",
    "ExternalAuthorizationError",
    "ExternalRuntimeError",
    "FunctionDescription",
    "LogonError",
    "RFCError",
    "RFCLibError",
    "RFCTypeError",
    "RfcFieldType",
    "RfcParameterDirection",
    "TypeDescription",
    "__version__",
    "__version_info__",
    "cancel_connection",
    "get_nwrfclib_version",
    "reload_ini_file",
    "set_cryptolib_path",
    "set_ini_file_directory",
]

# pyrfc 3.4's reconnect triggers (group ints are RFC_ERROR_GROUP in sapnwrfc.h).
# pyrfc tests them only on the error of RfcInvoke — an error from an earlier
# step (descriptor fetch, not_requested, input fill) never reconnects.
# codes: RFC_COMMUNICATION_FAILURE, RFC_ABAP_RUNTIME_FAILURE,
#        RFC_ABAP_MESSAGE, RFC_EXTERNAL_FAILURE
_RECONNECT_CODES = frozenset({1, 3, 4, 15})
# groups: ABAP_RUNTIME_FAILURE, LOGON_FAILURE, COMMUNICATION_FAILURE,
#         EXTERNAL_RUNTIME_FAILURE
_RECONNECT_GROUPS = frozenset({2, 3, 4, 5})
# RFC_CANCELED: pyrfc rewrites its message after a successful reconnect.
_RFC_CANCELED = 7

# pyrfc 3.4's config keys and their defaults. Unknown keys get pyrfc's exact
# "not supported" RFCError — see Connection.__init__.
_PYRFC_CONFIG_DEFAULTS: dict[str, Any] = {
    "rstrip": True,
    "return_import_params": False,
    "dtime": False,
    "check_date": True,
    "check_time": True,
    "timeout": None,
}

# pyrfc 3.4's get_connection_attributes() keys, in its order, mapped to the
# native ConnectionAttributes field that holds the same RFC_ATTRIBUTES member.
# pyrfc adds `reserved` and `active_unit` after these (see the method).
_ATTRIBUTE_KEYS: dict[str, str] = {
    "dest": "dest",
    "host": "host",
    "partnerHost": "partner_host",
    "sysNumber": "sys_number",
    "sysId": "sys_id",
    "client": "client",
    "user": "user",
    "language": "language",
    "trace": "trace",
    "isoLanguage": "iso_language",
    "codepage": "codepage",
    "partnerCodepage": "partner_codepage",
    "rfcRole": "rfc_role",
    "type": "type",
    "partnerType": "partner_type",
    "rel": "rel",
    "partnerRel": "partner_rel",
    "kernelRel": "kernel_rel",
    "cpicConvId": "cpic_conv_id",
    "progName": "prog_name",
    "partnerBytesPerChar": "partner_bytes_per_char",
    "partnerSystemCodepage": "partner_system_codepage",
    "partnerIP": "partner_ip",
    "partnerIPv6": "partner_ipv6",
}

# wrapError's group → exception class map (pyrfc _cyrfc.pyx; RFC_ERROR_GROUP
# ints as in sapnwrfc.h: 1=ABAP_APPLICATION_FAILURE .. 7=EXTERNAL_AUTHORIZATION_FAILURE).
_GROUP_TO_ERROR: dict[int, type[RFCLibError]] = {
    1: ABAPApplicationError,
    2: ABAPRuntimeError,
    3: LogonError,
    4: CommunicationError,
    5: ExternalRuntimeError,
    6: ExternalApplicationError,
    7: ExternalAuthorizationError,
}


def _raise_compat(exc: pyrfcx.RfcError, message: str | None = None) -> NoReturn:
    """Re-raise a native pyrfcx.RfcError as its pyrfc-compatible twin.

    Classification mirrors pyrfc's wrapError: the exception class comes from
    the RFC_ERROR_INFO group. Field names are renamed (abap_msg_* → msg_*);
    empty fields stay "" (pyrfc's wrapString never yields None for set fields).
    `message` replaces the SDK message, as pyrfc does for RFC_CANCELED.

    COMPAT-ONLY, not from the SDK: pyrfc's wrapError passes every text field
    through wrapString, which right-strips ASCII whitespace, so the SDK's
    trailing newline of a communication error is lost there. Native pyrfcx
    keeps the SDK text unchanged.
    """
    cls = _GROUP_TO_ERROR.get(exc.group, RFCLibError)
    raise cls(
        _wrap_string(exc.message if message is None else message),
        exc.code,
        _wrap_string(exc.key),
        _wrap_string(exc.abap_msg_class),
        _wrap_string(exc.abap_msg_type),
        _wrap_string(exc.abap_msg_number),
        _wrap_string(exc.abap_msg_v1),
        _wrap_string(exc.abap_msg_v2),
        _wrap_string(exc.abap_msg_v3),
        _wrap_string(exc.abap_msg_v4),
    ) from exc


# pyrfc's own enums: plain Enum (not IntEnum), valued with the SDK codes.
# RfcFieldType lacks the SDK's BOX (41) and GENERIC_BOX (42), as in pyrfc.
class RfcParameterDirection(Enum):
    RFC_IMPORT = 1
    RFC_EXPORT = 2
    RFC_CHANGING = 3
    RFC_TABLES = 7


class RfcFieldType(Enum):
    RFCTYPE_CHAR = 0
    RFCTYPE_DATE = 1
    RFCTYPE_BCD = 2
    RFCTYPE_TIME = 3
    RFCTYPE_BYTE = 4
    RFCTYPE_TABLE = 5
    RFCTYPE_NUM = 6
    RFCTYPE_FLOAT = 7
    RFCTYPE_INT = 8
    RFCTYPE_INT2 = 9
    RFCTYPE_INT1 = 10
    RFCTYPE_NULL = 14
    RFCTYPE_ABAPOBJECT = 16
    RFCTYPE_STRUCTURE = 17
    RFCTYPE_DECF16 = 23
    RFCTYPE_DECF34 = 24
    RFCTYPE_XMLDATA = 28
    RFCTYPE_STRING = 29
    RFCTYPE_XSTRING = 30
    RFCTYPE_INT8 = 31
    RFCTYPE_UTCLONG = 32
    RFCTYPE_UTCSECOND = 33
    RFCTYPE_UTCMINUTE = 34
    RFCTYPE_DTDAY = 35
    RFCTYPE_DTWEEK = 36
    RFCTYPE_DTMONTH = 37
    RFCTYPE_TSECOND = 38
    RFCTYPE_TMINUTE = 39
    RFCTYPE_CDAY = 40


def _enum_names(enum_obj: type[Enum]) -> set[str]:
    # A set: its order in pyrfc's error messages changes from run to run.
    return {en.name for en in enum_obj}


class TypeDescription:
    """A structure or table type description, as pyrfc 3.4's class.

    The checks and messages are pyrfc's. pyrfc's source tests lengths
    against ``long``; a pyrfc built with Cython 3.0 reads that as ``int``,
    which is what this class does.
    """

    def __init__(self, name: Any, nuc_length: Any, uc_length: Any) -> None:
        self.fields: list[dict[str, Any]] = []
        if len(name) < 1 or len(name) > 30:
            raise TypeError(f"field 'name' (string) '{name}' should be from 1-30 chars.")
        for int_field in [nuc_length, uc_length]:
            # Exact type: True is rejected here, but accepted by add_field.
            if type(int_field) is not int:
                raise TypeError(f"field '{name}' length '{int_field}' must be of type integer")
        self.name = name
        self.nuc_length = nuc_length
        self.uc_length = uc_length

    def add_field(
        self,
        name: Any,
        field_type: Any,
        nuc_length: Any,
        uc_length: Any,
        nuc_offset: Any,
        uc_offset: Any,
        decimals: Any = 0,
        type_description: TypeDescription | None = None,
    ) -> None:
        """Add a field. A field with an empty name is skipped silently."""
        if len(name) < 1:
            return None
        if len(name) > 30:
            raise TypeError(f"field 'name' (string) '{name}' should be from 1-30 chars.")
        if field_type not in _enum_names(RfcFieldType):
            raise TypeError(
                f"'field_type' (string) '{field_type}' must be in {_enum_names(RfcFieldType)}"
            )
        for int_field in [nuc_length, nuc_offset, uc_length, uc_offset]:
            if not isinstance(int_field, int):
                raise TypeError(f"field '{name}' length '{int_field}' must be of type integer")
        self.fields.append(
            {
                "name": name,
                "field_type": field_type,
                "nuc_length": nuc_length,
                "nuc_offset": nuc_offset,
                "uc_length": uc_length,
                "uc_offset": uc_offset,
                "decimals": decimals,
                "type_description": type_description,
            }
        )
        return None

    def __repr__(self) -> str:
        return (
            f"<TypeDescription '{self.name}' with {len(self.fields)} "
            f"fields (n/uclength={self.nuc_length}/{self.uc_length})>"
        )


class FunctionDescription:
    """A function module description, as pyrfc 3.4's class.

    The checks and messages are pyrfc's, including the ``parameter_text``
    message that prints its placeholder instead of the value.
    """

    def __init__(self, name: Any) -> None:
        self.name = name
        self.parameters: list[dict[str, Any]] = []

    def add_parameter(
        self,
        name: Any,
        parameter_type: Any,
        direction: Any,
        nuc_length: Any,
        uc_length: Any,
        decimals: Any = 0,
        default_value: Any = "",
        parameter_text: Any = "",
        optional: Any = False,
        type_description: TypeDescription | None = None,
    ) -> None:
        """Add a parameter. Lengths are not checked, as in pyrfc."""
        if len(name) < 1 or len(name) > 30:
            raise TypeError(f"field 'name' (string) {name} should be from 1-30 chars.")
        if parameter_type not in _enum_names(RfcFieldType):
            raise TypeError(
                f"'parameter_type' (string) '{parameter_type}' must be in "
                f"{_enum_names(RfcFieldType)}"
            )
        if direction not in _enum_names(RfcParameterDirection):
            raise TypeError(
                f"'direction' (string) '{direction}' must be in "
                f"'{_enum_names(RfcParameterDirection)}'"
            )
        if len(default_value) > 30:
            raise TypeError(
                f"'default_value' (string) '{default_value}' must not exceed 30 chars."
            )
        if len(parameter_text) > 79:
            # Not an f-string in pyrfc either: the message shows the placeholder.
            raise TypeError(
                "'parameter_text' (string) '{parameter_text}' must not exceed 79 chars."
            )
        self.parameters.append(
            {
                "name": name,
                "parameter_type": parameter_type,
                "direction": direction,
                "nuc_length": nuc_length,
                "uc_length": uc_length,
                "decimals": decimals,
                "default_value": default_value,
                "parameter_text": parameter_text,
                "optional": optional,
                "type_description": type_description,
            }
        )

    def __repr__(self) -> str:
        return f"<FunctionDescription '{self.name}' with {len(self.parameters)} params>"


def _wrap_string(value: str) -> str:
    # COMPAT-ONLY: pyrfc's wrapString strips the UTF-8 bytes with
    # bytes.rstrip(): ASCII whitespace only.
    return value.rstrip(" \t\n\r\x0b\x0c")


def _wrap_type_description(td: pyrfcx.TypeDescription) -> TypeDescription:
    """pyrfc's wrapTypeDescription over a native type description."""
    type_desc = TypeDescription(_wrap_string(td.name), td.nuc_length, td.uc_length)
    for field in td.fields:
        type_desc.add_field(
            name=_wrap_string(field.name),
            field_type=RfcFieldType(int(field.type)).name,
            nuc_length=field.nuc_length,
            nuc_offset=field.nuc_offset,
            uc_length=field.uc_length,
            uc_offset=field.uc_offset,
            decimals=field.decimals,
            type_description=None
            if field.type_description is None
            else _wrap_type_description(field.type_description),
        )
    return type_desc


def _wrap_function_description(fd: pyrfcx.FunctionDescription) -> FunctionDescription:
    """pyrfc's wrapFunctionDescription over a native function description."""
    func_desc = FunctionDescription(_wrap_string(fd.name))
    for param in fd.parameters:
        func_desc.add_parameter(
            name=_wrap_string(param.name),
            parameter_type=RfcFieldType(int(param.type)).name,
            direction=RfcParameterDirection(int(param.direction)).name,
            nuc_length=param.nuc_length,
            uc_length=param.uc_length,
            decimals=param.decimals,
            default_value=_wrap_string(param.default_value),
            parameter_text=_wrap_string(param.text),
            optional=param.optional,
            type_description=None
            if param.type_description is None
            else _wrap_type_description(param.type_description),
        )
    return func_desc


def _invalid_handle_error() -> ExternalRuntimeError:
    """The SDK's answer when pyrfc passes its NULL handle of a closed
    connection to an SDK call."""
    return ExternalRuntimeError(
        "An invalid handle 'RFC_CONNECTION_HANDLE' was passed to the API call",
        13,
        "RFC_INVALID_HANDLE",
        "",
        "",
        "",
        "",
        "",
        "",
        "",
    )


def set_ini_file_directory(path_name: Any) -> None:
    """Set the directory in which the SDK searches for sapnwrfc.ini, as
    pyrfc 3.4 does: the directory must hold a sapnwrfc.ini file."""
    if type(path_name) is not str:
        raise TypeError("sapnwrfc.ini path is not a string:", path_name)
    if not isfile(join(path_name, "sapnwrfc.ini")):
        raise TypeError("sapnwrfc.ini not found in:", path_name)
    try:
        pyrfcx.set_ini_path(path_name)
    except pyrfcx.RfcError as exc:
        _raise_compat(exc)


def reload_ini_file() -> None:
    """Read sapnwrfc.ini again, as pyrfc 3.4 does. A missing file is not an
    error."""
    try:
        pyrfcx.reload_ini_file()
    except pyrfcx.RfcError as exc:
        _raise_compat(exc)


def set_cryptolib_path(path_name: Any) -> None:
    """Set the SAP crypto library path, as pyrfc 3.4 does: the path must be
    an existing file. The SDK keeps the first path, even when that library
    fails to load. pyrfc's TypeError for a non-string names sapnwrfc.ini."""
    if type(path_name) is not str:
        raise TypeError("sapnwrfc.ini path is not a string:", path_name)
    if not isfile(path_name):
        raise TypeError("Crypto library not found:", path_name)
    try:
        pyrfcx.load_crypto_library(path_name)
    except pyrfcx.RfcError as exc:
        _raise_compat(exc)


def get_nwrfclib_version() -> dict[str, Any]:
    """SDK version, as pyrfc 3.4 reports it: RfcGetVersion plus sys.platform."""
    v = pyrfcx.sdk_version()
    return {
        "major": v.major,
        "minor": v.minor,
        "patchLevel": v.patch_level,
        "platform": sys.platform,
    }


def _cancel_connection(conn: pyrfcx.Connection) -> None:
    """Cancel the call in flight on `conn`. Runs in its own thread.

    As in pyrfc 3.4, a closed connection is skipped silently, and an SDK
    error (RFC_ILLEGAL_STATE when no call is in flight) is raised in this
    thread, where threading.excepthook prints it and the caller never sees
    it. The function name matches pyrfc's, so the printed thread name does.
    """
    if conn.closed:
        return
    try:
        conn.cancel()
    except ValueError:
        # Closed between the check and the cancel: pyrfc's handle is None.
        return
    except pyrfcx.RfcError as exc:
        _raise_compat(exc)


def _start_cancel(conn: pyrfcx.Connection) -> None:
    # pyrfc's cancel_connection: the cancel runs in a new, non-daemon thread.
    Thread(target=_cancel_connection, args=(conn,)).start()


def cancel_connection(client_connection: Connection) -> None:
    """Cancel the RFC call in flight on `client_connection`, as pyrfc 3.4 does.

    The cancelled call raises CommunicationError RFC_CANCELED and reconnects.
    """
    _start_cancel(client_connection._conn)


def _pyrfc_sdk_text(text: str) -> str:
    """COMPAT-ONLY, not from the SDK: the text the SDK reads under pyrfc 3.4.

    pyrfc hands the SDK a NUL-terminated copy of every connection parameter
    and function name, so the SDK reads it only up to an embedded NUL
    (measured on real pyrfc: ``dest="X\\0junk"`` opens ``X``). Native pyrfcx
    raises ``ValueError`` for such a text, so the facade cuts it first. The
    binding cuts parameter, field and ``not_requested`` names the same way
    (``compat_input::pyrfc_sdk_name``).
    """
    return text.split("\0", 1)[0]


class Connection:
    """A pyrfc-compatible RFC connection (facade over pyrfcx).

    Keyword arguments are string connection parameters, passed to pyrfcx
    (and thence the SDK) verbatim. All six pyrfc 3.4 `config` options
    are supported.
    """

    def __init__(self, config: dict[str, Any] | None = None, **params: str) -> None:
        # pyrfc validates config before the connection parameters.
        config = config or {}
        for key in config:
            if key not in _PYRFC_CONFIG_DEFAULTS:
                raise RFCError(f"Connection configuration option '{key}' is not supported")
        self._config: dict[str, Any] = {
            key: config.get(key, default) for key, default in _PYRFC_CONFIG_DEFAULTS.items()
        }
        # pyrfc copies these flags into a bit mask here, so a later change to
        # the `options` dict does not affect them. It reads `timeout` from
        # that dict at each call, so a change to it does.
        self._check_date = bool(self._config["check_date"])
        self._check_time = bool(self._config["check_time"])
        self._rstrip = bool(self._config["rstrip"])
        self._dtime = bool(self._config["dtime"])
        self._return_import_params = bool(self._config["return_import_params"])
        if not params:
            raise RFCError("Connection parameters missing")
        for name, value in params.items():
            if not isinstance(value, str):
                raise RFCError(f"Connection parameter '{name}' value is not string")
        self._params: dict[str, str] = {
            _pyrfc_sdk_text(name): _pyrfc_sdk_text(value) for name, value in params.items()
        }
        try:
            self._conn = pyrfcx.Connection(**self._params)
        except pyrfcx.RfcError as exc:
            _raise_compat(exc)

    # ── lifecycle surface ───────────────────────────────────────────────

    def close(self) -> None:
        """Close the connection. Idempotent."""
        self._conn.close()

    def open(self) -> None:
        """Open the connection again with the same parameters.

        pyrfc 3.4 opens a new connection even when one is open, and drops
        the old handle without closing it. The facade closes the old one:
        the caller cannot see the difference, and the server session is
        freed.
        """
        self._conn.close()
        try:
            self._conn = pyrfcx.Connection(**self._params)
        except pyrfcx.RfcError as exc:
            _raise_compat(exc)

    def reopen(self) -> None:
        """Close the connection (if open) and reopen it with the same parameters.

        Probably not needed anymore: a stale connection is reopened
        automatically inside call() when the failure is connection-killing
        (the reconnect). Exists only for pyrfc retrocompatibility — there
        is no SDK call behind it (close + open composed), so the native
        pyrfcx.Connection deliberately does not have it.
        """
        self.open()

    def cancel(self) -> None:
        """Cancel the RFC call in flight, as pyrfc 3.4's cancel_connection(self)."""
        cancel_connection(self)

    @property
    def version(self) -> dict[str, Any]:
        """SDK version, as pyrfc 3.4 reports it: RfcGetVersion plus sys.platform."""
        return get_nwrfclib_version()

    @property
    def options(self) -> dict[str, Any]:
        """The connection configuration: all six pyrfc config keys.

        The dict itself, not a copy, as in pyrfc 3.4.
        """
        return self._config

    @property
    def alive(self) -> bool:
        """Whether the connection is open."""
        return not self._conn.closed

    def __bool__(self) -> bool:
        return self.alive

    def __enter__(self) -> Connection:
        return self

    def __exit__(self, *exc: object) -> None:
        self._conn.close()

    def __repr__(self) -> str:
        state = "open" if self.alive else "closed"
        return f"<pyrfc.Connection [{state}]>"

    # ── connection management and information ──────────────────────────

    def ping(self) -> None:
        """Send an RFC ping (RfcPing). pyrfc 3.4 never reconnects here."""
        if not self.alive:
            raise _invalid_handle_error()
        try:
            self._conn.ping()
        except pyrfcx.RfcError as exc:
            _raise_compat(exc)

    def is_valid(self) -> bool:
        """Whether the SDK still holds the connection (RfcIsConnectionHandleValid).

        No network round-trip; False once the connection is closed.
        """
        return self._conn.is_valid()

    def get_function_description(self, func_name: str) -> FunctionDescription:
        """The description of a function module, as pyrfc 3.4 builds it.

        The name is upper-cased first. pyrfc 3.4 never reconnects here.
        """
        func_name = _pyrfc_sdk_text(func_name.upper())
        if not self.alive:
            raise _invalid_handle_error()
        try:
            desc = self._conn.describe_function(func_name)
        except pyrfcx.RfcError as exc:
            _raise_compat(exc)
        return _wrap_function_description(desc)

    def get_connection_attributes(self) -> dict[str, Any]:
        """Connection attributes as pyrfc 3.4's 26-key dict; {} when closed."""
        if not self.alive:
            return {}
        try:
            attrs = self._conn.attributes()
        except pyrfcx.RfcError as exc:
            _raise_compat(exc)
        result: dict[str, Any] = {
            key: getattr(attrs, field) for key, field in _ATTRIBUTE_KEYS.items()
        }
        # pyrfc passes the SDK's `dest` through, and the SDK reports it empty;
        # pyrfcx fills it from the connection parameters instead.
        result["dest"] = ""
        # pyrfc also returns the SDK's reserved buffer, which pyrfcx does not
        # expose. It is empty in the 7.50 SDK.
        result["reserved"] = ""
        # The facade has no transactional or unit RFC, so no unit is active.
        result["active_unit"] = False
        return result

    # ── call ────────────────────────────────────────────────────────────

    def call(
        self,
        func_name: str,
        options: dict[str, Any] | None = None,
        **params: Any,
    ) -> dict[str, Any]:
        """Call a function module; output in the pyrfc compat profile."""
        if not isinstance(func_name, str):
            raise RFCError(
                "Remote function module name must be unicode string, received:",
                func_name,
                type(func_name),
            )
        if not self.alive:
            raise RFCError(
                f"Remote function module '{func_name}' invocation rejected "
                "because the connection is closed"
            )
        # pyrfc reads only `not_requested` and `timeout`; any other option
        # key is silently ignored.
        options = options or {}
        not_requested = options.get("not_requested", [])
        if type(not_requested) is not list:
            not_requested = [not_requested]
        for name in not_requested:
            # pyrfc's fillString calls name.encode(): a non-str name raises
            # that AttributeError, e.g. for a tuple passed instead of a list.
            name.encode()
        # pyrfc starts the timer after the deactivation and before the input
        # fill. An explicit per-call None disables the config timeout. The
        # timer holds the connection the call runs on, not self._conn at
        # fire time, which a reconnect may have replaced.
        conn = self._conn
        timeout = options.get("timeout", self._config["timeout"])
        cancel_timer = None
        if timeout is not None:
            cancel_timer = Timer(timeout, _start_cancel, (conn,))
            cancel_timer.start()
        try:
            return conn._call_compat(
                func_name,
                params,
                check_date=self._check_date,
                check_time=self._check_time,
                not_requested=not_requested,
                rstrip=self._rstrip,
                dtime=self._dtime,
                return_import_params=self._return_import_params,
            )
        except pyrfcx.RfcError as exc:
            # pyrfc stops the timer right after RfcInvoke, before a reconnect.
            if cancel_timer is not None:
                cancel_timer.cancel()
            # Reconnect-on-error: pyrfc reopens inside call() when
            # RfcInvoke fails with a connection-killing error, so `alive` stays
            # True afterwards.
            if exc.sdk_function == "RfcInvoke" and (
                exc.code in _RECONNECT_CODES or exc.group in _RECONNECT_GROUPS
            ):
                # pyrfc drops the old handle without closing it. Closing it is
                # not visible to the caller and frees the server session.
                conn.close()
                try:
                    self._conn = pyrfcx.Connection(**self._params)
                except pyrfcx.RfcError as reconnect_exc:
                    _raise_compat(reconnect_exc)
                if exc.code == _RFC_CANCELED:
                    # pyrfc prints the two SDK handle values here. The facade
                    # holds no handle, so it prints the ids of the two native
                    # connection objects: the same shape, and like the
                    # handles, the values change from run to run.
                    _raise_compat(
                        exc,
                        f"Connection was canceled: {id(conn)}. New handle: {id(self._conn)}",
                    )
            _raise_compat(exc)
        finally:
            if cancel_timer is not None:
                cancel_timer.cancel()
