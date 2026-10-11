"""Type stubs for the pyrfcx._native extension module."""

from typing import Any

def sdk_version() -> tuple[int, int, int, str]:
    """The loaded SDK's `RfcGetVersion`: (major, minor, patch_level, description).

    Reports the library actually loaded, which is not always the one intended
    — see tests/test_sdk_pin.py.
    """

def set_ini_path(directory: str) -> None:
    """`RfcSetIniPath` — the directory where the SDK looks for `sapnwrfc.ini`.

    Process-wide. Destinations already read stay known after a switch; the
    directory the SDK already has is not read again; a directory that does
    not exist is accepted. An embedded NUL raises ValueError before any SDK
    call.
    """

def reload_ini_file() -> None:
    """`RfcReloadIniFile` — read `sapnwrfc.ini` again.

    Process-wide. Destinations read earlier stay known.
    """

def load_crypto_library(path: str) -> None:
    """`RfcLoadCryptoLibrary` — the SAP crypto library path for TLS.

    Process-wide and final after the first call, even when that call fails;
    a later call with another path raises. SNC takes its library from
    `SNC_LIB`. An embedded NUL raises ValueError before any SDK call.
    """

class Connection:
    """SDK failures raise `pyrfcx._exceptions.RfcError` subclasses. Each one
    carries `sdk_function`, the name of the SDK function that returned the
    error (e.g. "RfcInvoke"), or "" for an error raised before any SDK call.
    """

    def __init__(self, params: dict[str, str]) -> None: ...
    def close(self) -> None: ...
    def ping(self) -> None:
        """`RfcPing` — a network round-trip; raises if the connection is broken."""
    def cancel(self) -> None:
        """`RfcCancel` — cancel the call in flight, from a different thread.

        A successful cancel also closes the connection (SDK behavior).
        Raises the SDK's RFC_ILLEGAL_STATE failure if no call is in flight.
        """
    def attributes(self) -> dict[str, str]:
        """`RfcGetConnectionAttributes`, keyed by the core's snake_case names.

        `dest` is always empty — the SDK does not report it. `pyrfcx.Connection`
        supplies it from the parameters the connection was opened with.
        """

    def describe_function(self, function_name: str) -> dict[str, Any]:
        """`RfcGetFunctionDesc` plus the descriptor walk, as nested dicts.

        Keys: `name`, `parameters` (list of dicts with `name`, `type`,
        `direction`, `nuc_length`, `uc_length`, `decimals`, `default_value`,
        `text`, `optional`, `type_description`). A type description is a dict
        with `name`, `nuc_length`, `uc_length`, `fields` (dicts with `name`,
        `type`, `nuc_length`, `nuc_offset`, `uc_length`, `uc_offset`,
        `decimals`, `type_description`), or None. `type` and `direction` are
        the SDK's int codes.

        The name goes to the SDK unchanged; the SDK lookup is case-sensitive
        (a lower-case name gives FU_NOT_FOUND).
        """
    def is_valid(self) -> bool:
        """`RfcIsConnectionHandleValid`; False once closed.

        Asks the SDK about the handle, locally, without a network round trip.
        """

    def call(
        self,
        function_name: str,
        params: dict[str, Any],
        compat: bool = False,
        check_date: bool = True,
        check_time: bool = True,
        not_requested: list[str] = ...,
        rstrip: bool = True,
        dtime: bool = False,
        return_import_params: bool = False,
    ) -> dict[str, Any]: ...
