"""Smoke tests — verifies the skeleton is importable and wired correctly."""

from __future__ import annotations


def test_import_pyrfcx() -> None:
    import pyrfcx

    assert hasattr(pyrfcx, "__version__")
    assert isinstance(pyrfcx.__version__, str)


def test_package_version() -> None:
    import importlib.metadata

    import pyrfcx

    assert pyrfcx.__version__ == "0.2.0"
    assert pyrfcx.__version__ == importlib.metadata.version("pyrfcx")


def test_import_pyrfc_exceptions() -> None:
    from pyrfc._exception import (
        ABAPApplicationError,
        ABAPRuntimeError,
        CommunicationError,
        LogonError,
        RFCError,
        RFCLibError,
    )

    assert issubclass(RFCLibError, RFCError)
    assert issubclass(ABAPApplicationError, RFCLibError)
    assert issubclass(ABAPRuntimeError, RFCLibError)
    assert issubclass(CommunicationError, RFCLibError)
    assert issubclass(LogonError, RFCLibError)


def test_pyrfc_version() -> None:
    import pyrfc

    assert pyrfc.__version__ == "3.4"
    assert pyrfc.__version_info__ == ("3", "4")


def test_pyrfc_ships_in_the_pyrfcx_distribution() -> None:
    # One distribution, two import packages: the facade has no wheel of its own.
    import importlib.metadata
    from pathlib import Path

    import pyrfc
    import pyrfcx

    # Both packages sit side by side: in site-packages, or in python/ for an
    # editable install.
    assert Path(pyrfc.__file__).parent.parent == Path(pyrfcx.__file__).parent.parent
    # An editable install records no package files, so the mapping is
    # checked only where the wheel itself is installed.
    owners = importlib.metadata.packages_distributions()
    if "pyrfcx" in owners:
        assert owners["pyrfc"] == ["pyrfcx"]


def test_no_raw_handle() -> None:
    # Owner ruling: the raw SDK handle is never exposed. The SDK frees a
    # handle on a fatal error and reuses the address, so a copy outside the
    # core's locked cell can reach another live connection. Real pyrfc has
    # `handle`; the facade deliberately does not.
    import pyrfc
    import pyrfcx

    assert not hasattr(pyrfc.Connection, "handle")
    assert not hasattr(pyrfcx.Connection, "handle")
    assert not hasattr(pyrfcx._native.Connection, "handle")
