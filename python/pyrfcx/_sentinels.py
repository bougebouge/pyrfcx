"""Sentinel values for the native pyrfcx API.

Pure Python so the extension module can recognize the objects by identity
(the same pattern as ``_exceptions``: the PyO3 layer imports this module).
"""

from __future__ import annotations

from typing import final


@final
class InactiveType:
    """The type of :data:`INACTIVE`. A singleton — ``InactiveType()`` always
    returns the same object, so identity checks in the extension module hold
    even for unpickled/copied instances."""

    _singleton: InactiveType | None = None

    def __new__(cls) -> InactiveType:
        if cls._singleton is None:
            cls._singleton = super().__new__(cls)
        return cls._singleton

    def __repr__(self) -> str:
        return "pyrfcx.INACTIVE"


INACTIVE = InactiveType()
"""Pass as a parameter value in :meth:`pyrfcx.Connection.call` to deactivate
that parameter for the call (``RfcSetParameterActive``).

An inactive input parameter is not sent, so a backend-side default value
(defined in SE37) applies instead. An inactive output parameter is not
returned by the backend — saving network traffic and, if the ABAP code
checks ``IS SUPPLIED``, backend work — and is omitted from the result dict.
"""
