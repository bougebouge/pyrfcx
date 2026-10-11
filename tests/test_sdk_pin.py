"""The SDK pin: prove we load the vendored library and nothing else.

This exists because we did not, for a long time, and nobody noticed. With
`SAPNWRFC_HOME` unset, the loader falls back to a bare "sapnwrfc.dll" and
Windows resolves it from the ambient DLL search order — which on the dev
machine silently supplied C:\\Windows\\System32\\sapnwrfc.dll (753 PL 1533)
instead of the vendored 750 PL 18. Every probe fact and every live test ran
against an SDK the repo does not ship, and the two versions do not behave
identically. Real pyrfc requires SAPNWRFC_HOME, so it loaded the
vendored SDK — meaning the two stacks were not even comparable.

The assertions below are deliberately self-checking rather than
version-hardcoded: they compare what our stack loaded against what the
vendored DLL reports when loaded directly by file path. Bump the SDK and
they still hold; load the wrong library and they fail immediately.
"""

from __future__ import annotations

import ctypes
import os
import sys
from pathlib import Path

import pytest

from tests.conftest import SDK_HOME

if sys.platform == "win32":
    SDK_LIB_NAME = "sapnwrfc.dll"
elif sys.platform == "darwin":
    SDK_LIB_NAME = "libsapnwrfc.dylib"
else:
    SDK_LIB_NAME = "libsapnwrfc.so"


def vendored_dll_version() -> tuple[int, int, int, str]:
    """Call RfcGetVersion on the vendored library directly, by absolute path.

    Independent of pyrfcx's loader — this is the reference value that our
    stack's reported version must match. One path for every platform:
    RfcGetVersion loads no ICU, so the library needs no search-path setup
    (measured on Windows with SAPNWRFC_HOME unset and the SDK off PATH).
    ctypes' c_wchar_p is UTF-32 on POSIX, while SAP_UC is UTF-16, so the
    description is read as 16-bit units up to the terminator.
    """
    lib = ctypes.CDLL(str(SDK_HOME / "lib" / SDK_LIB_NAME))
    lib.RfcGetVersion.restype = ctypes.POINTER(ctypes.c_uint16)
    lib.RfcGetVersion.argtypes = [ctypes.POINTER(ctypes.c_uint)] * 3
    major, minor, patch = (ctypes.c_uint() for _ in range(3))
    ptr = lib.RfcGetVersion(ctypes.byref(major), ctypes.byref(minor), ctypes.byref(patch))
    length = 0
    while ptr[length] != 0:
        length += 1
    desc = ctypes.string_at(ptr, 2 * length).decode("utf-16-le")
    return major.value, minor.value, patch.value, desc


def test_sapnwrfc_home_points_at_the_vendored_sdk() -> None:
    assert os.environ.get("SAPNWRFC_HOME") == str(SDK_HOME), (
        "conftest must pin SAPNWRFC_HOME before anything loads the SDK"
    )
    assert (SDK_HOME / "lib" / SDK_LIB_NAME).is_file()


def test_loaded_sdk_is_the_vendored_one_not_an_ambient_dll() -> None:
    """pyrfcx must report the vendored DLL's own version, not System32's."""
    from pyrfcx import _native

    ours = _native.sdk_version()
    reference = vendored_dll_version()

    assert ours == reference, (
        f"pyrfcx loaded an SDK reporting {ours!r}, but the vendored DLL "
        f"reports {reference!r} — an ambient sapnwrfc.dll (e.g. the one in "
        f"C:\\Windows\\System32) was loaded instead of vendor/nwrfcsdk"
    )


@pytest.mark.skipif(os.name != "nt", reason="DLL-search-order hazard is Windows-only")
def test_an_ambient_sdk_would_have_been_a_different_library() -> None:
    """Document the hazard: only meaningful while a rival DLL exists here.

    If System32 has no sapnwrfc.dll this is vacuous and skips — the point is
    to show the pin is load-bearing on machines where it is not.
    """
    ambient = Path(r"C:\Windows\System32\sapnwrfc.dll")
    if not ambient.is_file():
        pytest.skip("no ambient System32 sapnwrfc.dll on this machine")

    from pyrfcx import _native

    # Same base name, different file: without the pin, this is what the bare
    # "sapnwrfc.dll" fallback would resolve to.
    assert _native.sdk_version()[:3] == vendored_dll_version()[:3]


@pytest.mark.skipif(
    sys.platform not in ("darwin", "linux"), reason="Windows pins ICU through the DLL directory"
)
def test_icu_is_loaded_from_the_vendored_sdk() -> None:
    """The SDK must take ICU from the pinned SDK, not from another install.

    The SDK loads ICU itself, lazily, at the first connection attempt.
    macOS: dyld resolves that lookup through the SDK's LC_RPATH, an absolute
    path fixed when SAP built the library; pyrfcx's loader opens ICU from
    SAPNWRFC_HOME first so that lookup finds those images.
    Linux: the SDK tries the leaf name first (LD_LIBRARY_PATH, its RUNPATH,
    the system dirs) and only then its own directory, so an ICU found on
    that search path would win.
    A refused connection to the local host is enough to trigger the ICU load.
    """
    from pyrfcx import CommunicationError, Connection

    with pytest.raises(CommunicationError):
        Connection(ashost="127.0.0.1", sysnr="99", client="000", user="x", passwd="x")

    images = _loaded_images()
    # The SDK's own ICU, not the system's libicucore. The version suffix
    # differs by patch level, so match on the stem.
    icu = [p for p in images if p.name.startswith(("libicudata", "libicuuc", "libicui18n"))]
    sdk_lib = (SDK_HOME / "lib").resolve()

    assert icu, "no ICU library is loaded after a connection attempt"
    assert all(p.resolve().parent == sdk_lib for p in icu), (
        f"ICU loaded from outside {sdk_lib}: {[str(p) for p in icu]}"
    )


def _loaded_images() -> list[Path]:
    """The shared libraries mapped into this process."""
    if sys.platform == "linux":
        maps = Path("/proc/self/maps").read_text().splitlines()
        return sorted(
            {
                Path(f[5])
                for f in (line.split(maxsplit=5) for line in maps)
                if len(f) == 6 and f[5].startswith("/")
            }
        )
    dyld = ctypes.CDLL(None)
    dyld._dyld_image_count.restype = ctypes.c_uint32
    dyld._dyld_get_image_name.restype = ctypes.c_char_p
    dyld._dyld_get_image_name.argtypes = [ctypes.c_uint32]
    return [
        Path(os.fsdecode(dyld._dyld_get_image_name(i))) for i in range(dyld._dyld_image_count())
    ]
