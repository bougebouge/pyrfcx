"""pyrfc exception hierarchy — compat facade over pyrfcx.

Reproduces the exception surface of pyrfc 3.4 (https://github.com/SAP/PyRFC).
"""

from __future__ import annotations

from enum import Enum


class RcCodeText(Enum):
    """RFC_RC return codes by name (values: the SDK's RFC_RC enum)."""

    RFC_OK = 0
    RFC_COMMUNICATION_FAILURE = 1
    RFC_LOGON_FAILURE = 2
    RFC_ABAP_RUNTIME_FAILURE = 3
    RFC_ABAP_MESSAGE = 4
    RFC_ABAP_EXCEPTION = 5
    RFC_CLOSED = 6
    RFC_CANCELED = 7
    RFC_TIMEOUT = 8
    RFC_MEMORY_INSUFFICIENT = 9
    RFC_VERSION_MISMATCH = 10
    RFC_INVALID_PROTOCOL = 11
    RFC_SERIALIZATION_FAILURE = 12
    RFC_INVALID_HANDLE = 13
    RFC_RETRY = 14
    RFC_EXTERNAL_FAILURE = 15
    RFC_EXECUTED = 16
    RFC_NOT_FOUND = 17
    RFC_NOT_SUPPORTED = 18
    RFC_ILLEGAL_STATE = 19
    RFC_INVALID_PARAMETER = 20
    RFC_CODEPAGE_CONVERSION_FAILURE = 21
    RFC_CONVERSION_FAILURE = 22
    RFC_BUFFER_TOO_SMALL = 23
    RFC_TABLE_MOVE_BOF = 24
    RFC_TABLE_MOVE_EOF = 25
    RFC_START_SAPLOGON_FAILURE = 26
    RFC_ABAP_CLASS_EXCEPTION = 27
    RFC_UNKNOWN_ERROR = 28
    RFC_AUTHORIZATION_FAILURE = 29
    RFC_AUTHENTICATION_FAILURE = 30
    RFC_CRYPTOLIB_FAILURE = 31
    RFC_IO_FAILURE = 32
    RFC_LOCKING_FAILURE = 33


class RFCError(Exception):
    """Base class of all pyrfc errors."""


class RFCLibError(RFCError):
    """An error reported by the SAP NW RFC library."""

    def __init__(
        self,
        message: str | None = None,
        code: int | None = None,
        key: str | None = None,
        msg_class: str | None = None,
        msg_type: str | None = None,
        msg_number: str | None = None,
        msg_v1: str | None = None,
        msg_v2: str | None = None,
        msg_v3: str | None = None,
        msg_v4: str | None = None,
    ) -> None:
        super().__init__(message)
        self.message = message
        self.code = code
        self.key = key
        self.msg_class = msg_class
        self.msg_type = msg_type
        self.msg_number = msg_number
        self.msg_v1 = msg_v1
        self.msg_v2 = msg_v2
        self.msg_v3 = msg_v3
        self.msg_v4 = msg_v4

    def __str__(self) -> str:
        # Replicates pyrfc 3.4's __str__ bug-for-bug: its RcCodeText enum uses
        # auto() (values 1..34) and prints `.value`, so rc_text is the *int*
        # code itself for 1..34 and "??" otherwise (note: code 0 → "??").
        # pyrfc's own test pins `str(e)` == "5 (rc=5): key=..." (byte-exact).
        code = 28 if self.code is None else self.code  # 28 = RFC_UNKNOWN_ERROR
        rc_text = code if isinstance(code, int) and 1 <= code <= 34 else "??"
        return (
            f"{rc_text} (rc={self.code}): key={self.key}"
            f", message={self.message}"
            f" [MSG: class={self.msg_class}, type={self.msg_type}"
            f", number={self.msg_number}"
            f", v1-4:={self.msg_v1};{self.msg_v2};{self.msg_v3};{self.msg_v4}]"
        )


class ABAPApplicationError(RFCLibError): ...


class ABAPRuntimeError(RFCLibError): ...


class CommunicationError(RFCLibError): ...


class ExternalApplicationError(RFCLibError): ...


class ExternalAuthorizationError(RFCLibError): ...


class ExternalRuntimeError(RFCLibError): ...


class RFCTypeError(RFCLibError):
    """Type conversion error — dead code in pyrfc 3.4 (defined, never raised).

    Defined here solely so `from pyrfc import RFCTypeError` keeps working.
    """


class LogonError(RFCLibError):
    def __init__(
        self,
        message: str | None = None,
        code: int = 2,
        key: str = "RFC_LOGON_FAILURE",
        msg_class: str | None = None,
        msg_type: str | None = None,
        msg_number: str | None = None,
        msg_v1: str | None = None,
        msg_v2: str | None = None,
        msg_v3: str | None = None,
        msg_v4: str | None = None,
    ) -> None:
        super().__init__(
            message,
            code,
            key,
            msg_class,
            msg_type,
            msg_number,
            msg_v1,
            msg_v2,
            msg_v3,
            msg_v4,
        )
