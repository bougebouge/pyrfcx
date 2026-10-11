"""pyrfcx — Modern SAP NW RFC client for Python."""

from __future__ import annotations

import decimal
import enum
import importlib.metadata as _metadata
import os
from dataclasses import dataclass
from typing import Any

# Import _exceptions before _native so the extension module's import of
# pyrfcx._exceptions is always a sys.modules hit. (_sentinels has no such
# constraint — the extension imports it lazily inside call().)
from pyrfcx import _native
from pyrfcx._exceptions import (
    AbapApplicationError,
    AbapRuntimeError,
    CommunicationError,
    LogonError,
    PyrfcxError,
    RfcError,
    SdkNotFoundError,
)
from pyrfcx._sentinels import INACTIVE, InactiveType

# PEP 440 form from the installed distribution metadata (maturin derives it
# from Cargo's version, e.g. "0.1.0-alpha.1" → "0.1.0a1"), so __version__ equals
# importlib.metadata.version("pyrfcx").
__version__: str = _metadata.version("pyrfcx")

InputValue = (
    str | int | float | bytes | decimal.Decimal | dict[str, Any] | list[Any] | InactiveType
)
OutputValue = Any


@dataclass(frozen=True, slots=True)
class SdkVersion:
    """Version of the loaded SAP NW RFC SDK library, from ``RfcGetVersion``.

    Library-level, not connection-level — ``RfcGetVersion`` takes no
    connection handle, which is why this is a module function and not a
    ``Connection`` method. Numbers follow the SDK's own convention:
    ``major`` is the release (e.g. ``7500``), not a single digit.
    """

    major: int
    """SAP release, e.g. ``7500`` for SDK 7.50."""
    minor: int
    """Minor version (``0`` for every shipped 7.50 SDK)."""
    patch_level: int
    """Patch level, e.g. ``18``."""
    description: str
    """The SDK's own version string (the ``RfcGetVersion`` return value)."""


def sdk_version() -> SdkVersion:
    """Version of the loaded SAP NW RFC SDK library (``RfcGetVersion``).

    Reports the library actually loaded — which is not always the one
    intended, since an unset ``SAPNWRFC_HOME`` falls back to the ambient
    DLL search order. Loads the SDK if it is not loaded yet;
    raises :class:`SdkNotFoundError` if it cannot be found.
    """
    major, minor, patch_level, description = _native.sdk_version()
    return SdkVersion(major=major, minor=minor, patch_level=patch_level, description=description)


def set_ini_path(directory: str | os.PathLike[str]) -> None:
    """Set the directory where the SDK looks for ``sapnwrfc.ini``
    (``RfcSetIniPath``).

    Process-wide: it applies to every connection opened afterwards. The SDK
    reads the file from the new directory at once. By default the SDK looks
    in the current working directory.

    Traps:

    - Destinations the SDK has already read stay known after a switch to
      another directory.
    - Setting the directory the SDK already has does not read the file
      again. Use :func:`reload_ini_file` after an edit.
    - The SDK accepts a directory that does not exist.
    - A path with an embedded NUL character raises :class:`ValueError`
      before any SDK call.
    """
    _native.set_ini_path(os.fspath(directory))


def reload_ini_file() -> None:
    """Read ``sapnwrfc.ini`` again (``RfcReloadIniFile``), from the
    directory set by :func:`set_ini_path` or the current working directory.

    Process-wide. Needed only after the file was edited:
    :func:`set_ini_path` with the directory the SDK already has does not
    read the file again. Destinations the SDK read earlier stay known, even
    when the new file no longer defines them. A missing file is not an
    error.
    """
    _native.reload_ini_file()


def load_crypto_library(path: str | os.PathLike[str]) -> None:
    """Set the SAP crypto library for TLS over WebSocket RFC
    (``RfcLoadCryptoLibrary``; same effect as ``TLS_SAPCRYPTOLIB`` in
    ``sapnwrfc.ini``). ``path`` is the library file, not its directory.

    Process-wide and final after the first call: the SDK keeps the first
    path, even when that library fails to load (:class:`CommunicationError`).
    A later call with the same path succeeds; another path raises
    :class:`RfcError`. An empty path is rejected and sets nothing. A path
    with an embedded NUL character raises :class:`ValueError` before any SDK
    call, so it does not fix the setting.

    This setting is for TLS only. SNC takes its library from ``SNC_LIB``.
    """
    _native.load_crypto_library(os.fspath(path))


class _SdkCode(enum.IntEnum):
    """Base of the SDK code enums: an unknown code becomes a pseudo-member."""

    @classmethod
    def _missing_(cls, value: object) -> Any:
        # A code the 7.50 header does not define. The pseudo-member keeps the
        # code, is cached (one object per code), and is not listed in
        # iteration or by name lookup.
        if not isinstance(value, int) or isinstance(value, bool):
            return None
        member = int.__new__(cls, value)
        member._name_ = f"UNKNOWN_{value}"
        member._value_ = value
        return cls._value2member_map_.setdefault(value, member)


class RfcType(_SdkCode):
    """The SDK's ``RFCTYPE`` codes (7.50 header): the type of a parameter
    or a structure field. Names follow ABAP where it differs from the SDK
    (``NUMC`` is the SDK's ``RFCTYPE_NUM``).

    A code the header does not define gives a pseudo-member named
    ``UNKNOWN_<code>`` that keeps the code: ``RfcType(99) == 99``.
    """

    CHAR = 0
    DATE = 1
    BCD = 2
    TIME = 3
    BYTE = 4
    TABLE = 5
    NUMC = 6
    FLOAT = 7
    INT = 8
    INT2 = 9
    INT1 = 10
    NULL = 14
    ABAPOBJECT = 16
    STRUCTURE = 17
    DECF16 = 23
    DECF34 = 24
    XMLDATA = 28
    STRING = 29
    XSTRING = 30
    INT8 = 31
    UTCLONG = 32
    UTCSECOND = 33
    UTCMINUTE = 34
    DTDAY = 35
    DTWEEK = 36
    DTMONTH = 37
    TSECOND = 38
    TMINUTE = 39
    CDAY = 40
    BOX = 41
    GENERIC_BOX = 42


class RfcDirection(_SdkCode):
    """The SDK's ``RFC_DIRECTION`` codes: how a parameter is passed.

    A code the header does not define gives a pseudo-member named
    ``UNKNOWN_<code>`` that keeps the code.
    """

    IMPORT = 1
    EXPORT = 2
    CHANGING = 3
    TABLES = 7


@dataclass(frozen=True, slots=True)
class TypeDescription:
    """A structure or table line type (``RFC_TYPE_DESC_HANDLE``)."""

    name: str
    """DDIC name of the type."""
    nuc_length: int
    """Length in bytes in a 1-byte-per-character system."""
    uc_length: int
    """Length in bytes in a 2-byte-per-character (Unicode) system."""
    fields: tuple[FieldDescription, ...]
    """The fields, in the SDK's order."""


@dataclass(frozen=True, slots=True)
class FieldDescription:
    """One field of a structure or table line type (``RFC_FIELD_DESC``)."""

    name: str
    type: RfcType
    """The field type."""
    nuc_length: int
    """Length in bytes in a 1-byte-per-character system."""
    nuc_offset: int
    """Offset in bytes in a 1-byte-per-character system."""
    uc_length: int
    """Length in bytes in a 2-byte-per-character (Unicode) system."""
    uc_offset: int
    """Offset in bytes in a 2-byte-per-character (Unicode) system."""
    decimals: int
    """Number of decimals, as the SDK reports it."""
    type_description: TypeDescription | None
    """The nested type of a STRUCTURE or TABLE field; ``None`` otherwise."""


@dataclass(frozen=True, slots=True)
class ParameterDescription:
    """One parameter of a function module (``RFC_PARAMETER_DESC``)."""

    name: str
    type: RfcType
    """The parameter type."""
    direction: RfcDirection
    """How the parameter is passed."""
    nuc_length: int
    """Length in bytes in a 1-byte-per-character system."""
    uc_length: int
    """Length in bytes in a 2-byte-per-character (Unicode) system."""
    decimals: int
    """Number of decimals, as the SDK reports it."""
    default_value: str
    """Default value as defined in SE37, verbatim."""
    text: str
    """Description text as defined in SE37, verbatim."""
    optional: bool
    """Whether SE37 defines the parameter as optional."""
    type_description: TypeDescription | None
    """The type of a STRUCTURE or TABLE parameter; ``None`` otherwise."""


@dataclass(frozen=True, slots=True)
class FunctionDescription:
    """The signature of a function module (``RFC_FUNCTION_DESC_HANDLE``)."""

    name: str
    """The name the SDK's descriptor reports."""
    parameters: tuple[ParameterDescription, ...]
    """The parameters, in the SDK's order."""


def _type_description(raw: dict[str, Any] | None) -> TypeDescription | None:
    if raw is None:
        return None
    return TypeDescription(
        name=raw["name"],
        nuc_length=raw["nuc_length"],
        uc_length=raw["uc_length"],
        fields=tuple(
            FieldDescription(
                name=f["name"],
                type=RfcType(f["type"]),
                nuc_length=f["nuc_length"],
                nuc_offset=f["nuc_offset"],
                uc_length=f["uc_length"],
                uc_offset=f["uc_offset"],
                decimals=f["decimals"],
                type_description=_type_description(f["type_description"]),
            )
            for f in raw["fields"]
        ),
    )


def _function_description(raw: dict[str, Any]) -> FunctionDescription:
    return FunctionDescription(
        name=raw["name"],
        parameters=tuple(
            ParameterDescription(
                name=p["name"],
                type=RfcType(p["type"]),
                direction=RfcDirection(p["direction"]),
                nuc_length=p["nuc_length"],
                uc_length=p["uc_length"],
                decimals=p["decimals"],
                default_value=p["default_value"],
                text=p["text"],
                optional=p["optional"],
                type_description=_type_description(p["type_description"]),
            )
            for p in raw["parameters"]
        ),
    )


@dataclass(frozen=True, slots=True)
class ConnectionAttributes:
    """Attributes of an open connection.

    One field per member of the SDK's ``RFC_ATTRIBUTES``, except ``reserved``
    ("reserved for later use" in the header — no defined meaning to expose).
    The SDK's naming convention is that an unprefixed field describes our side
    and a ``partner_*`` field the other side. Every value is a string because
    every member of ``RFC_ATTRIBUTES`` is a character buffer — the SDK reports
    even ``trace`` and ``partner_bytes_per_char`` as text.

    Trailing blanks are stripped: these are fixed-width buffers holding
    blank-padded ABAP values, and the SDK pads them inconsistently.
    """

    dest: str
    """RFC destination. The SDK does not report this, so it is supplied from
    the connection's own parameters — and is empty for a connection opened
    without a ``dest`` (by ``ashost``/``sysnr``/``client``), which genuinely
    has no destination name."""
    host: str
    """Own host name."""
    partner_host: str
    """Partner host name."""
    sys_number: str
    """R/3 system number."""
    sys_id: str
    """R/3 system ID."""
    client: str
    """Client ("Mandant")."""
    user: str
    """User."""
    language: str
    """Language."""
    trace: str
    """Trace level (0-3)."""
    iso_language: str
    """2-byte ISO-Language."""
    codepage: str
    """Own code page."""
    partner_codepage: str
    """Partner code page."""
    rfc_role: str
    """C/S: RFC Client / RFC Server."""
    type: str
    """2/3/E/R: R/2, R/3, Ext, Reg.Ext."""
    partner_type: str
    """2/3/E/R: R/2, R/3, Ext, Reg.Ext."""
    rel: str
    """Own system release."""
    partner_rel: str
    """Partner system release."""
    kernel_rel: str
    """Partner kernel release."""
    cpic_conv_id: str
    """CPI-C conversation ID."""
    prog_name: str
    """Name of the calling ABAP program (report, module pool)."""
    partner_bytes_per_char: str
    """Bytes per character in the backend's current codepage. Note this is
    different from the semantics of the PCS parameter."""
    partner_system_codepage: str
    """Partner system code page."""
    partner_ip: str
    """Partner IP."""
    partner_ipv6: str
    """Partner IPv6. The SDK reports the IPv4 address here when the partner
    has no IPv6 address."""


class Connection:
    """A connection to a SAP system.

    Usage::

        with Connection(ashost="sap01", sysnr="00", client="100",
                        user="RFC_USER", passwd="secret") as conn:
            result = conn.call("RFC_PING")

    A parameter name or value with an embedded NUL character raises
    ``ValueError`` before any SDK call: the SDK reads it only up to the
    NUL. The message names the parameter, never its value.
    """

    def __init__(self, **params: str) -> None:
        if not params:
            raise ValueError("at least one connection parameter is required")
        self._native = _native.Connection(params)
        # Kept for later lookup — the SDK cannot report every parameter back
        # (`dest` most notably). Private on purpose: these may hold `passwd`
        # and other secrets, so they must not reach a public attribute, a
        # __repr__, or a traceback.
        self._params: dict[str, str] = dict(params)
        self._closed = False

    def _param(self, name: str) -> str:
        """Look up a connection parameter we were opened with, case-insensitively
        (the SDK treats parameter names that way). Empty string if absent."""
        lowered = name.lower()
        for key, value in self._params.items():
            if key.lower() == lowered:
                return value
        return ""

    def attributes(self) -> ConnectionAttributes:
        """Read the connection's attributes from the SDK."""
        if self._closed:
            raise ValueError("connection is closed")
        attrs = self._native.attributes()
        # The SDK reports `dest` as empty even when opened by destination, so
        # answer from the parameters we were given rather than passing the
        # SDK's blank through.
        attrs["dest"] = self._param("dest")
        return ConnectionAttributes(**attrs)

    def ping(self) -> None:
        """Ping the communication partner (``RfcPing``).

        A network round-trip that verifies the connection is genuinely
        alive (a firewall may have dropped it without notifying either
        end). Returns ``None`` on success; raises
        :class:`CommunicationError` if the connection is broken — and
        marks it closed, the same seam as :meth:`call`.
        """
        if self._closed:
            raise ValueError("connection is closed")
        try:
            self._native.ping()
        except CommunicationError:
            self._closed = True
            raise

    def describe_function(self, function_name: str, /) -> FunctionDescription:
        """Describe a function module's signature (``RfcGetFunctionDesc``).

        The name goes to the SDK unchanged, and the SDK lookup is
        case-sensitive: a lower-case name raises
        :class:`AbapApplicationError` with key ``FU_NOT_FOUND`` ("function
        not found"). pyrfcx does not upper-case the name. A name with an
        embedded NUL character raises ``ValueError`` before the SDK lookup.

        The first description of a function is a network round-trip; the SDK
        caches it afterwards.
        """
        if self._closed:
            raise ValueError("connection is closed")
        try:
            raw = self._native.describe_function(function_name)
        except (CommunicationError, AbapRuntimeError):
            self._closed = True
            raise
        return _function_description(raw)

    def is_valid(self) -> bool:
        """Whether the SDK still holds this connection
        (``RfcIsConnectionHandleValid``).

        :attr:`closed` is what pyrfcx knows about the connection.
        ``is_valid()`` asks the SDK about the handle, locally, without a
        network round-trip: a connection a firewall silently dropped still
        reports ``True`` — :meth:`ping` is the check for that. ``False`` once
        the connection is closed.
        """
        if self._closed:
            return False
        return self._native.is_valid()

    def cancel(self) -> None:
        """Cancel the RFC call currently executing on this connection
        (``RfcCancel``).

        Must be called from a *different thread* than the one blocked in
        :meth:`call` — the SDK specifies that, and the blocked thread cannot
        call anything anyway. The cancelled :meth:`call` raises in its own
        thread.

        A successful cancel also **closes the connection** (SDK behavior),
        so this connection is unusable afterwards. If no call is in flight,
        the SDK fails with ``RFC_ILLEGAL_STATE`` (raised as
        :class:`RfcError`) and the connection stays open.
        """
        if self._closed:
            raise ValueError("connection is closed")
        self._native.cancel()
        self._closed = True

    def close(self) -> None:
        """Close the connection. Idempotent."""
        if self._closed:
            return
        self._native.close()
        self._closed = True

    @property
    def closed(self) -> bool:
        """Whether the connection has been closed, as pyrfcx knows it.

        No SDK call. :meth:`is_valid` asks the SDK about the handle.
        """
        return self._closed

    def call(self, function_name: str, /, **params: InputValue) -> dict[str, OutputValue]:
        """Invoke an RFC function module and return its output parameters.

        Pass :data:`pyrfcx.INACTIVE` as a parameter's value to deactivate it
        for this call (``RfcSetParameterActive``): an inactive input is not
        sent (the backend's SE37 default value applies), and an inactive
        output is not returned by the backend and is omitted from the
        result. Unknown parameter names raise, as always.

        The SDK reads every name as a NUL-terminated string. A function,
        parameter or field name with an embedded NUL character raises
        ``ValueError`` before the SDK call that takes it, so before the
        unknown-name error. A text value is sent with its length and is not
        checked: a NUL in a value reaches the SDK.
        """
        if self._closed:
            raise ValueError("connection is closed")
        try:
            return self._native.call(function_name, params)
        except (CommunicationError, AbapRuntimeError):
            self._closed = True
            raise

    def _call_compat(
        self,
        function_name: str,
        params: dict[str, Any],
        *,
        check_date: bool = True,
        check_time: bool = True,
        not_requested: list[str] | None = None,
        rstrip: bool = True,
        dtime: bool = False,
        return_import_params: bool = False,
    ) -> dict[str, Any]:
        """Documented-private seam for the pyrfc compat facade.

        Passes compat=True to the native layer, enabling pyrfc-compatible
        output projections and the compat empty DATE/TIME skip. The keyword
        arguments are pyrfc's `check_date`/`check_time`/`rstrip`/`dtime`/
        `return_import_params` config flags and its `not_requested` call
        option — compat-only, not SDK concepts.
        """
        if self._closed:
            raise ValueError("connection is closed")
        try:
            return self._native.call(
                function_name,
                params,
                True,
                check_date,
                check_time,
                not_requested or [],
                rstrip,
                dtime,
                return_import_params,
            )
        except (CommunicationError, AbapRuntimeError):
            self._closed = True
            raise

    def __enter__(self) -> Connection:
        return self

    def __exit__(self, *exc: object) -> None:
        self.close()

    def __repr__(self) -> str:
        state = "closed" if self._closed else "open"
        return f"<pyrfcx.Connection [{state}]>"


__all__ = [
    "INACTIVE",
    "AbapApplicationError",
    "AbapRuntimeError",
    "CommunicationError",
    "Connection",
    "ConnectionAttributes",
    "FieldDescription",
    "FunctionDescription",
    "InactiveType",
    "LogonError",
    "ParameterDescription",
    "PyrfcxError",
    "RfcDirection",
    "RfcError",
    "RfcType",
    "SdkNotFoundError",
    "SdkVersion",
    "TypeDescription",
    "__version__",
    "load_crypto_library",
    "reload_ini_file",
    "sdk_version",
    "set_ini_path",
]
