"""pyrfcx exception hierarchy.

Programming errors (empty params, closed connection, wrong types) raise
stdlib TypeError / ValueError. SDK/SAP failures raise from PyrfcxError.
"""

from __future__ import annotations


class PyrfcxError(Exception):
    """Base for all pyrfcx errors — anything that went wrong talking to SAP."""


class SdkNotFoundError(PyrfcxError):
    """The SAP NW RFC SDK shared library could not be loaded."""


class RfcError(PyrfcxError):
    """An error reported by the SAP NW RFC SDK. Carries RFC_ERROR_INFO fields.

    `sdk_function` is plain metadata, not an RFC_ERROR_INFO field: the name
    of the SDK function that returned the error, e.g. "RfcInvoke" or
    "RfcGetFunctionDesc". It is "" when pyrfcx raised the error itself
    before any SDK call (for example on a closed connection).
    """

    code: int
    group: int
    key: str
    message: str
    abap_msg_class: str
    abap_msg_type: str
    abap_msg_number: str
    abap_msg_v1: str
    abap_msg_v2: str
    abap_msg_v3: str
    abap_msg_v4: str
    sdk_function: str

    def __init__(
        self,
        message: str,
        *,
        code: int = 0,
        group: int = 0,
        key: str = "",
        abap_msg_class: str = "",
        abap_msg_type: str = "",
        abap_msg_number: str = "",
        abap_msg_v1: str = "",
        abap_msg_v2: str = "",
        abap_msg_v3: str = "",
        abap_msg_v4: str = "",
        sdk_function: str = "",
    ) -> None:
        super().__init__(message)
        self.code = code
        self.group = group
        self.key = key
        self.message = message
        self.abap_msg_class = abap_msg_class
        self.abap_msg_type = abap_msg_type
        self.abap_msg_number = abap_msg_number
        self.abap_msg_v1 = abap_msg_v1
        self.abap_msg_v2 = abap_msg_v2
        self.abap_msg_v3 = abap_msg_v3
        self.abap_msg_v4 = abap_msg_v4
        self.sdk_function = sdk_function

    def __str__(self) -> str:
        return self.message


class CommunicationError(RfcError):
    """Network or transport failure — connection is dead."""


class LogonError(RfcError):
    """Authentication or SNC failure."""


class AbapRuntimeError(RfcError):
    """ABAP SYSTEM_FAILURE or ABAP MESSAGE (E/A/X) — connection is dead."""


class AbapApplicationError(RfcError):
    """ABAP exception raised by the function module — connection is alive."""
