# pyrfcx

`pyrfcx` is an SAP NW RFC client for Python with a Rust core which includes **`pyrfcx-compat`** (compatibility layer), a drop-in replacement for the client part of SAP's pyrfc 3.4.

The compatibility layer reproduces the quirks of pyrfc 3.4, each one extensively measured on real pyrfc first.

## Install

```powershell
pip install pyrfcx-compat
```

Remove real `pyrfc` from the environment first: both packages install the
import name `pyrfc`.

## Use

```python
from pyrfc import Connection

with Connection(dest="MY_DEST") as conn:
    result = conn.call("STFC_CONNECTION", REQUTEXT="hi")
    print(result["ECHOTEXT"])
```

`dest` names a destination in a `sapnwrfc.ini` file. By default the SDK
reads that file from the current directory. You can also pass the connection parameters
directly (`ashost`, `sysnr`, `client`, `user`, `passwd`, and so on).

### What is supported

| Area | Members |
|---|---|
| Connection | `Connection(config=..., **params)`, `open()`, `close()`, `reopen()`, `is_valid()`, `alive`, context manager |
| Calls | `call(name, options=..., **params)`, `ping()`, `cancel()` |
| `config` keys | `rstrip`, `return_import_params`, `dtime`, `check_date`, `check_time`, `timeout` |
| `call` options | `not_requested`, `timeout` |
| Information | `version`, `options`, `get_connection_attributes()` |
| Descriptions | `get_function_description()`, with `FunctionDescription`, `TypeDescription`, `RfcFieldType` and `RfcParameterDirection` |
| Module functions | `get_nwrfclib_version()`, `cancel_connection()`, `set_ini_file_directory()`, `reload_ini_file()`, `set_cryptolib_path()` |
| Exceptions | The pyrfc classes, with the same names and the same attributes: `RFCError`, `RFCLibError`, `CommunicationError`, `LogonError`, `ABAPApplicationError`, `ABAPRuntimeError`, `ExternalAuthorizationError`, `ExternalApplicationError`, `ExternalRuntimeError`, `RFCTypeError` |

### What is not supported

- `Server`, `Throughput`, tRFC, qRFC, bgRFC and WebSocket RFC. This is a
  client library, so these are out of scope.
- Not built yet: `free()`, `reset_server_context()`, `type_desc_get()`,
  `type_desc_remove()`, `func_desc_remove()`, `set_locale_radix()`, the
  language converters and `ConnectionParameters`.
- `Connection.handle` is not exposed. The Rust core owns the SDK handle.

### Tests

Every live test runs against a real ABAP system. The compat suite runs
twice on the same SDK: once on `pyrfcx-compat` and once on real pyrfc 3.4.
Both runs must give the same results.

| Suite | pyrfc 3.4 | `pyrfcx-compat` | `pyrfcx` | C SDK |
|---|---|---|---|---|
| pyrfc 3.4's own tests, adapted (`tests/compat/`) | 78 | 78 | – | – |
| Added compat tests, each expectation measured on real pyrfc first | 56 | 56 | – | – |
| Behaviour facts (unpublished) | 168 | 168 | 168 | 168 |

### Performance against real pyrfc

Time was
measured on macOS arm64 with SDK 7.50 patch level 15; memory on Windows x64
with patch level 18.

**Time.** On large tables `pyrfcx-compat` is faster than pyrfc 3.4 and uses
less CPU. On small calls the two are the same: the backend sets the pace.
Seconds per call, median of 10 runs in fresh processes:

| Case | Measure | pyrfc | `pyrfcx-compat` | `pyrfcx-compat` is |
|---|---|---|---|---|
| `RFC_READ_TABLE`, 100 000 rows of DD03L | Wall time<br>CPU time | 5.79<br>1.13 | 5.62<br>0.99 | 3 % better<br>13 % better |
| 100 000 rows of 40 × CHAR(15) | Wall time<br>CPU time | 10.98<br>6.30 | 9.97<br>5.27 | 9 % better<br>16 % better |

**Memory.** `pyrfcx-compat` never uses more memory than pyrfc, and its
results are smaller: all row dicts share one string object per key. Bytes
per table row:

| Table shape | Measure | pyrfc | `pyrfcx-compat` | `pyrfcx-compat` is |
|---|---|---|---|---|
| 40 × CHAR(15) | Peak memory<br>Size of the result | 5 508<br>5 429 | 3 606<br>3 468 | 35 % better<br>36 % better |
| 1 × CHAR(512) | Peak memory<br>Size of the result | 1 201<br>614 | 1 199<br>561 | same<br>9 % better |
| Audit log, 30 fields incl. STRING | Peak memory<br>Size of the result | 4 993<br>4 007 | 4 989<br>2 333 | same<br>42 % better |

## Status

- **Version 0.1.** The `pyrfc` API is complete for the client members listed below, and stable.
- **Platforms.** Wheels exist for Windows x64, macOS arm64
  (`macosx_11_0_arm64`), and Linux x86_64 and aarch64 (`manylinux_2_28`).
  All are `cp310-abi3`.
- **Client only.** Server RFC, tRFC, qRFC and bgRFC are not supported.
- **The native `pyrfcx` package is not a supported API yet.** It ships in
  the same install because `pyrfc` runs on it. Its interface may change in
  any 0.x release and it is not documented here. See "The engine" below.

## Requirements

1. The **SAP NW RFC SDK 7.50, patch level 15 or later**, for your platform.
   Patch levels from 5 should also work, but they are untested.
   The SDK belongs to SAP and is not part of this distribution. Download
   it from the SAP Support Portal (`support.sap.com`). You need an S-user.
2. The environment variable `SAPNWRFC_HOME`. Set it to the SDK directory
   (the directory that contains `lib/`).
3. CPython 3.10 or later, built for the same architecture as the SDK.
   - **Windows:** SAP ships the SDK for x64 only. Use an x64 Python, even on
     an ARM64 machine. An ARM64 Python cannot load the x64 SDK.
   - **Linux:** the wheel needs glibc 2.28, but the SDK needs more:
     `GLIBCXX_3.4.26` (the libstdc++ of GCC 9) and `libuuid.so.1`. The wheel
     runs only where the SDK runs.

With `SAPNWRFC_HOME` set, nothing else is needed: no `PATH` or
`LD_LIBRARY_PATH` entry, on any platform. Without it, the SDK is searched by
name, and on Windows a stray copy of `sapnwrfc.dll` can be found whose ICU
libraries are not: the first connect then hangs on a hidden "could not open
ICU library" dialog instead of failing. Set `SAPNWRFC_HOME`.

## pyrfc 3.4 quirks that the drop-in keeps

A program can depend on a quirk, so `pyrfc` from `pyrfcx-compat` reproduces
the quirks below.

| Case | pyrfc 3.4 and the drop-in |
|---|---|
| A CHAR value ends with a tab or a newline that belongs to the data | The character is removed. `"AB\t"` comes back as `"AB"`. |
| A STRING value ends with blanks | The blanks are removed. `"AB  "` comes back as `"AB"`. |
| A DATE field is initial | Comes back as `""`. You cannot tell it from an empty value. |
| A CHAR input is longer than the field | The SDK truncates the value. No error. |
| `256` for an INT1 field, `32768` for an INT2 field | The value wraps. No error. For INT1 the backend gets `0`. |
| An `IntEnum` value for an integer field | Raises `TypeError`. |
| An output parameter is deactivated with `not_requested` | The result contains the parameter with an initial value that the backend never sent. |
| A scalar parameter is in `not_requested` and also has a value | The value is sent. |
| `str()` of an exception | The text should start with the name of the return code, such as `RFC_INVALID_HANDLE (rc=5): ...`. It shows the number twice instead: `5 (rc=5): ...`. |
| `cancel()` fails, for example because no call is running | `cancel()` runs the SDK call in a new thread and returns at once. The error is raised in that thread and printed to stderr; the caller cannot catch it. |
| `get_connection_attributes()["dest"]` | The value is always empty. |

Each row is measured on real pyrfc 3.4.

## The engine

pyrfcx shares no code with pyrfc. `pyrfcx` is a new binding of the SAP NW RFC SDK, written in Rust. `pyrfcx-compat` is a thin Python layer on top of it that reproduces the pyrfc 3.4 API.

You need no Rust and no compiler to use it. Pre-compiled wheels exist for Windows x64, macOS arm64 and Linux x86_64 and aarch64, for CPython 3.10 and later.

The Rust core loads the SDK at runtime and marshals the character, number,
date and time, byte, structure and table types without loss. A call holds
the GIL only to build the Python result. The core never closes a handle
that the SDK has already freed. The `pyrfc` package is a facade over the
core that reproduces pyrfc's conversions and its quirks.

The native `pyrfcx` Python API is installed with the facade, but it is not
a supported interface yet. It may change in any 0.x release. Use
`import pyrfc`. The native API gets its own documentation when it is ready.

## Build from source

You need Rust (stable, 1.95 or later), CPython 3.10 or later and
[uv](https://docs.astral.sh/uv/). On Windows the Python must be x64. The
build does not need the SDK: the package loads it at runtime.

```sh
git clone https://github.com/bougebouge/pyrfcx
cd pyrfcx
uv venv --python <path-to-python>
uv pip install "maturin>=1.5,<2"    # or `just sync`, which installs every dev tool

# Build the wheels into target/wheels (on Windows add --target x86_64-pc-windows-msvc)
uv run maturin build --release --out target/wheels
uv build pyrfcx-compat/ --wheel --out-dir target/wheels

# Install them
uv pip install --find-links target/wheels pyrfcx-compat
```

`just wheels` builds the same pyrfcx wheel with the source paths remapped, so
the binary holds no path of the build machine; prefer it for a wheel you
give to others.

For a development install, use an editable build:

```sh
uv run maturin develop --uv     # on Windows add --target x86_64-pc-windows-msvc
uv pip install -e pyrfcx-compat/
```

The Linux wheels are built in Docker (`just linux-wheel <arch>`). The tests
need a live ABAP system. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

## More

- Development: [`CONTRIBUTING.md`](CONTRIBUTING.md)

## License

pyrfcx is licensed under the [Apache License, Version 2.0](LICENSE). The
SAP NW RFC SDK is not covered by this license. See [`NOTICE`](NOTICE).

SAP, SAP NetWeaver and the SAP NW RFC SDK are trademarks or registered
trademarks of SAP SE in Germany and other countries. pyrfc is SAP's Python
RFC connector. This project is not affiliated with, endorsed by or
sponsored by SAP SE.
