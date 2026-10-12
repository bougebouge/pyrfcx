# Contributing to pyrfcx

This file tells you how to build pyrfcx, how to run its tests, and which
rules a change must follow.

## Toolchain

| Tool | Version | Note |
|---|---|---|
| Rust | stable, 1.95 or later | `rust-toolchain.toml` selects stable. |
| CPython | 3.10 or later | Same architecture as the SDK. On Windows use an **x64 build**: an ARM64 Python cannot load the x64 SDK. |
| [uv](https://docs.astral.sh/uv/) | current | Use uv for all package management, not pip. |
| [maturin](https://www.maturin.rs/) | 1.5 or later, below 2 | The build backend. `just sync` installs it into the venv. |
| [just](https://just.systems/) | current | Runs the tasks in `justfile`. |
| SAP NW RFC SDK | 7.50, patch level 11 or later, for the host platform | Put it in `vendor/nwrfcsdk/` (see below). |

Create the venv with your Python, then install the development dependencies:

```sh
uv venv --python <path-to-python>
just sync
```

The dev dependencies are the `dev` dependency group in `pyproject.toml`.
A bare `uv sync` removes the editable installs; `just sync` runs
`uv sync --group dev` and restores them.

### Build target

`.cargo/config.toml` sets no build target: cargo builds for the host.

**On an ARM64 Windows host you must set the build target to Windows x64.**
The SAP SDK for Windows is x64-only, so an ARM64 build cannot load it. Set
the user environment variable once, then open a new terminal:

```
setx CARGO_BUILD_TARGET x86_64-pc-windows-msvc
```

The host then cross-compiles to x64 and runs the result under x64 emulation.
The `justfile` exports the same target and passes it to maturin, so the
`just` recipes work without the variable. A bare `cargo build` or
`cargo test` does not.

## just targets

| Target | What it does |
|---|---|
| `just` | Lists the targets. |
| `cargo build --workspace` | Builds the Rust crates alone; no Python needed. |
| `just develop` | Builds the extension module into the venv (`maturin develop`). |
| `just sync` | Syncs the venv with the dev group, then builds pyrfcx into it as an editable install (`pyrfcx` and `pyrfc`). |
| `just test` | Runs `develop`, then the Rust tests and the Python tests. |
| `just test-rust` | Runs `cargo test --workspace`. |
| `just test-python` | Runs `pytest tests/`. Run `just develop` first. Extra arguments go to pytest: `just test-python -rs`. A good run has 0 failed; skips come only from a missing `PYRFCX_TEST_USER`, an SNC-only destination or a release below 754; `dumps`, `netcut` and `soak` tests are deselected by default. |
| `just test-all-py` | Runs the Python suite on every supported CPython with one abi3 wheel (tox, `tox.toml`). |
| `just test-dumps` | Runs only the tests that leave ST22 dumps (see below). |
| `just pyrfc-real-setup` | Builds real pyrfc 3.4 into `.venv-pyrfc-real/` (the oracle, see below). |
| `just test-pyrfc-real` | Runs `tests/compat/` against real pyrfc 3.4. Extra arguments go to pytest. |
| `just test-compat-both` | Runs `tests/compat/` against the facade, then against real pyrfc. Extra arguments go to both. |
| `just lint` | Runs `cargo fmt --check`, clippy with `-D warnings`, ruff check, ruff format check, mypy. Needs the dev group: run `just sync` first. |
| `just fmt` | Formats Rust and Python and applies ruff fixes. |
| `just wheels` | Builds a release wheel for the host platform, for local checks. Release wheels come from CI (below). |
| `just third-party-licenses` | Regenerates `THIRD-PARTY-LICENSES` with cargo-about. Run it after a dependency change. |
| `just regen-bindings` | Regenerates the FFI bindings of the host platform from the SDK headers (needs libclang). |
| `just linux-image <arch>` and the other `linux-*` targets | Build and test on Linux in Docker (see below). |

## The SDK pin

The SDK is not in the repository. Copy the SAP NW RFC SDK 7.50 for the host
platform to `vendor/nwrfcsdk/`, so that `vendor/nwrfcsdk/lib/` holds the SDK
library (`sapnwrfc.dll`, `libsapnwrfc.so` or `libsapnwrfc.dylib`) and its ICU
libraries. A symbolic link works too. `vendor/` is gitignored. The minimum
SDK is 7.50 patch level 11. A function newer than patch level 11 must not be
added to the generated bindings.

The build and the tests pin `SAPNWRFC_HOME` to that directory:

- `.cargo/config.toml` sets it for every cargo process (`cargo test`,
  `cargo run`), with `force = true`.
- `tests/conftest.py` sets it, and adds `<SDK>/lib` to the DLL search and to
  `PATH`, before any test imports `pyrfcx` or `pyrfc`.

**Do not set `SAPNWRFC_HOME` by hand for the tests.** When the variable is
unset, the loader falls back to the bare library name, and Windows can find
a different SDK in a system directory. This happened: for months the tests
ran against an SDK of another version, which behaves differently.
`tests/test_sdk_pin.py` now fails if the loaded SDK is not the pinned one.

Put a probe in `tests/` as a test, where the pin applies. A standalone
script must set `SAPNWRFC_HOME` before it imports `pyrfcx` or `pyrfc`, as
`tests/conftest.py` does. An incomplete setup does not fail with an error: the SDK shows a modal "could not open ICU library"
dialog and the run hangs. On macOS the SDK exits the process instead.

## Live tests

Most tests need a live ABAP backend. You need the items below.

### sapnwrfc.ini

Create `sapnwrfc.ini` at the repository root. The SDK reads it from the
current directory. The file is gitignored. The test suites use two
destinations: `TEST_NW750` (release 750) and `TEST_NW757` (release 757).
The template uses SNC. Replace every `<...>`:

```ini
DEST=TEST_NW750
ASHOST=<host>
SYSNR=<nr>
CLIENT=<client>
SNC_MODE=1
SNC_PARTNERNAME=p:CN=<...>
SNC_MYNAME=p:CN=<...>

DEST=TEST_NW757
ASHOST=<host>
SYSNR=<nr>
CLIENT=<client>
SNC_MODE=1
SNC_PARTNERNAME=p:CN=<...>
SNC_MYNAME=p:CN=<...>
```

Without SNC, use `USER=` and `PASSWD=` instead of the `SNC_*` keys. Two
error tests need user-and-password logon; they skip on an SNC-only system.

### Environment variables

| Variable | Default | What it selects |
|---|---|---|
| `PYRFCX_TEST_DEST` | `TEST_NW750` | The destination for the native suite and the compat suite (`pytest tests/`). Use `TEST_NW757` for the tests that need release 754 or later (UTCLONG). The values that differ between backends are in `BACKENDS` in `tests/compat/config.py`; another destination needs an entry there. The Rust live tests do not read this variable: they always use `TEST_NW750`. |
| `PYRFCX_TEST_USER` | none | The user that `BAPI_USER_GET_DETAIL` reads in the compat tests. The user must exist on the backend. If the variable is unset, those tests skip. |
| `PYRFCX_TEST_SID` | none | The system ID that the attribute test expects. If the variable is unset, the test does not check the system ID. |

SNC logon needs two more variables, which the SDK reads, not the tests:
`SECUDIR`, the folder that holds the PSE, and the crypto library:
`SNC_LIB` on macOS and Linux, `SNC_LIB_64` on Windows (there `SNC_LIB`
names the 32-bit library). Export them in the shell before every live run;
the test code does not set them. Without them every live test fails in
`SncPDLInit`.

### Running the Rust live tests

The Rust live tests are `#[ignore]`, so `just test-rust` and `cargo test`
skip them. Run them with:

```sh
cargo test --workspace -- --include-ignored
```

The 27 repository-fixture tests (`repository_roundtrip`) are ignored too;
they need the SDK but no backend. A good run has 0 failed; `live_memory`
holds 0 tests outside Windows. Two traps, which apply to the offline run as
well:

- Do not activate the venv for `cargo test`. The pyrfcx-py unit tests embed
  a CPython, and PyO3 takes the first `python3` on `PATH`. A uv-managed
  interpreter cannot be embedded (`No module named 'encodings'`); a system
  or Homebrew Python works. `uv run pytest` is unaffected.
- `just` exports `CARGO_BUILD_TARGET`, so its recipes build into
  `target/<triple>/`, while a plain `cargo` command builds into
  `target/debug/`. The first switch between the two rebuilds everything.

The live tests write `dev_rfc.log` next to the crate manifest; it is
gitignored.

### ABAP objects on the backend

The objects below are in [`docs/abap/`](docs/abap/) in abapGit format, ready to import into a package of your choice; its `README.md` says which files belong to release 757. The table and the source listings here describe the same objects.

The suites call the function modules below. Prepare both backends,
`TEST_NW750` and `TEST_NW757`.

In the "Used by" columns, "native" is `tests/*.py`, "compat" is
`tests/compat/`, and "Rust live" is `crates/pyrfcx-core/tests/`.

#### Custom objects

Create these objects in package `$TMP` on each backend. A test that calls a
missing function module fails with `FU_NOT_FOUND`.

| Object | Type | Definition | Backend | Used by |
|---|---|---|---|---|
| `ZPYRFCX_TEST` | Function group | Holds the two function modules. | both | — |
| `ZPYRFCX_FE_INPUT` | Structure | The fields below. | both; `ZUTCLONG` on release 757 only | `ZPYRFCX_FE_DATATYPES` |
| `ZPYRFCX_FE_DATATYPES` | Function module, **remote-enabled** | `IMPORTING VALUE(IS_INPUT) TYPE ZPYRFCX_FE_INPUT OPTIONAL`, `VALUE(IV_COUNT) TYPE I DEFAULT 0`; `EXPORTING VALUE(ES_OUTPUT) TYPE ZPYRFCX_FE_INPUT`. Body: `es_output = is_input.` | both | native, compat, Rust live |
| `ZPYRFCX_MEM_ROW` | Structure | Fields `F01` to `F40`, each `abap.char(15)`. | both | `ZPYRFCX_MEM_ROWS` |
| `ZPYRFCX_MEM_ROWS` | Function module, **remote-enabled** | `IMPORTING VALUE(IV_ROWS) TYPE I`, `VALUE(IV_FILL) TYPE I DEFAULT 15`; `TABLES ET_ROWS LIKE ZPYRFCX_MEM_ROW`. Returns `IV_ROWS` rows. Source below. | both | native, compat (large tables), Rust live (thread test; `live_memory.rs` with `MEM_FUNC`) |

Fields of `ZPYRFCX_FE_INPUT`:

| Field | Type |
|---|---|
| `ZFLTP`, `ZFLTP_MIN`, `ZFLTP_MAX` | `abap.fltp` |
| `ZDEC`, `ZCURR` | `abap.dec(13,2)` |
| `ZQUAN` | `abap.dec(13,4)` |
| `ZQUAN_SIGN` | `abap.dec(13,3)` |
| `ZDECF16_MIN`, `ZDECF16_MAX` | `abap.df16_raw` |
| `ZDECF34_MIN`, `ZDECF34_MAX` | `abap.df34_raw` |
| `ZRAW` | `abap.raw(17)` |
| `ZRAWSTRING` | `abap.rawstring(0)` |
| `ZSHLP_MAT1` | `abap.char(40)` |
| `ZNUMC` | `abap.numc(10)` |
| `ZUTCLONG` | `abap.utclong`. **Release 757 copy only**: release 750 has no UTCLONG. |

In the structure source, release 750 uses `define type` and release 757 uses
`define structure`.

`ZPYRFCX_MEM_ROWS` returns a table, so it uses a `TABLES` parameter. Every
field value is unique per row and per field: `R<row, 7 digits>F<field, 2
digits>XXXX`, cut to the first `IV_FILL` characters. It reads no database
table.

```abap
FUNCTION ZPYRFCX_MEM_ROWS
  IMPORTING
    VALUE(IV_ROWS) TYPE I
    VALUE(IV_FILL) TYPE I DEFAULT 15
  TABLES
    ET_ROWS LIKE ZPYRFCX_MEM_ROW.

  DATA lv_fill  TYPE i.
  DATA lv_row   TYPE i.
  DATA lv_field TYPE i.
  DATA lv_value TYPE c LENGTH 15.
  DATA ls_row   TYPE zpyrfcx_mem_row.
  FIELD-SYMBOLS <lv_comp> TYPE any.

  CLEAR et_rows[].

  lv_fill = iv_fill.
  IF lv_fill < 0.
    lv_fill = 0.
  ELSEIF lv_fill > 15.
    lv_fill = 15.
  ENDIF.

  DO iv_rows TIMES.
    lv_row = sy-index.
    CLEAR ls_row.
    IF lv_fill > 0.
      DO 40 TIMES.
        lv_field = sy-index.
        ASSIGN COMPONENT lv_field OF STRUCTURE ls_row TO <lv_comp>.
        lv_value = |R{ lv_row WIDTH = 7 ALIGN = RIGHT PAD = '0' }F{ lv_field WIDTH = 2 ALIGN = RIGHT PAD = '0' }XXXX|.
        <lv_comp> = lv_value(lv_fill).
      ENDDO.
    ENDIF.
    APPEND ls_row TO et_rows.
  ENDDO.
ENDFUNCTION.
```

#### SAP standard function modules

These modules ship with every backend. Do not change them.

| Function module | Used by | Dumps and notes |
|---|---|---|
| `STFC_CONNECTION` | native, compat, Rust live | — |
| `STFC_STRUCTURE` | native, compat, Rust live, repository fixture | `RFCINT1=255` leaves `CONVT_OVERFLOW`: the Python test marked `dumps` and the Rust live test `int1_overflow_kills_connection` (runs with `--include-ignored`). |
| `STFC_STRING` | native, Rust live, repository fixture | — |
| `STFC_XSTRING` | native, Rust live, repository fixture | Its constant answer differs by release (`BACKENDS` in `tests/compat/config.py`). |
| `STFC_DEEP_TABLE` | native, compat, Rust live, repository fixture | — |
| `STFC_DEEP_STRUCTURE` | compat | — |
| `STFC_CHANGING` | compat, Rust live | — |
| `STFC_SAPGUI` | compat | Always leaves `DYNPRO_SEND_IN_BACKGROUND`. Its test is marked `dumps`. |
| `RFC_PING` | native, compat, Rust live | — |
| `RFC_PING_AND_WAIT` | native, compat, Rust live | The cancel and timeout tests cancel it while it waits. |
| `RFC_SYSTEM_INFO` | native, Rust live (thread tests) | — |
| `RFC_RAISE_ERROR` | compat | The `dumps` tests leave `COMPUTE_INT_ZERODIVIDE`, `MESSAGE_TYPE_X` and `BLOCKED_COMMIT`. |
| `RFC_READ_TABLE` | native, compat, Rust live | Reads tables `T000`, `T008X` and `DD03L`. |
| `SEWS_REF_DIRECT_TYPES` | native, compat | — |
| `BAPI_USER_GET_DETAIL` | compat, Rust live (thread test) | The compat tests read the user in `PYRFCX_TEST_USER` and skip when it is unset. |

### Tests that leave ST22 dumps

Tests marked `dumps` cause ABAP short dumps on purpose. In ABAP a runtime
error is a short dump, so a test of the `AbapRuntimeError` mapping must cause
one. `pytest` deselects these tests by default. Run them with
`just test-dumps` before you ask for a review.

## The pyrfc oracle

`tests/compat/` is adapted from pyrfc 3.4's own test suite
(https://github.com/SAP/PyRFC). The facade must pass it, and real pyrfc 3.4
must pass it too.

1. Run `just pyrfc-real-setup`. It clones SAP's PyRFC repository into
   `.pyrfc-real-src/` (gitignored) at the pyrfc 3.4 revision that the
   `justfile` names, unless the environment variable `PYRFC_REAL_SRC` names a
   local checkout. It then builds pyrfc into `.venv-pyrfc-real/` with
   CPython 3.12 and Cython 3.0 (`Cython>=3.0,<3.1`; pyrfc 3.4 uses the
   Python 2 name `long`, which Cython 3.1 and later leave undefined). On
   Windows it needs Visual Studio 2022 Build Tools with the ARM64-to-x64
   cross compiler (see `tools/build-pyrfc-real.cmd`).
2. Run `just test-compat-both`. Both halves must print the same counts
   and 0 failed; skips come only from a missing `PYRFCX_TEST_USER`, an
   SNC-only destination or a release below 754, and the deselected tests
   carry the `dumps` marker. Add `-rs` to see the skip reasons. The setup
   step prints a harmless uv warning, `virtualenv's --clear has no effect`.

**Anti-circularity rule.** Show a compat expectation on real pyrfc first. Do
not take an expected value from the facade's own output. If you cannot show
that pyrfc behaves that way, the expectation is a guess.

Read the pyrfc source only for work on the compat facade. The native API and
the Rust core are designed from the SDK, not from pyrfc.

## Release wheels (CI)

`.github/workflows/wheels.yml` builds the wheels for Linux x86_64 and
aarch64 (manylinux_2_28), Windows x64 and macOS arm64, one wheel per
platform. It runs on every tag and on manual start, never on a push or a
pull request. It needs no SAP file: the FFI bindings are committed and the
SDK is loaded at run time. Each job installs its wheel on CPython 3.10 and
imports `pyrfcx` and `pyrfc` from it; the live suites stay local. The
`collect` job offers one download, `wheels-<tag>`, for the upload to
TestPyPI and PyPI. The wheels on PyPI come from this workflow, not from a
workstation.

## Linux in Docker

The Linux wheels are built and tested in Docker. The images hold toolchains
only. The SDK, the crypto library and the PSE are mounted at run time from
the environment variables `PYRFCX_LINUX_SDK` (required),
`PYRFCX_LINUX_CRYPTO` and `PYRFCX_LINUX_SECUDIR`: the unpacked Linux SDK
folder for `<arch>` (the one that holds `lib/` and `include/`), the folder
that holds `libsapcrypto.so`, and the folder that holds the PSE. The same
PSE works from Linux and from the host. `<arch>` is `aarch64` or `x86_64`.
The images hold only toolchains; rebuild them with `just linux-image` when
`tools/docker/Dockerfile.linux` changes.

| Target | What it does |
|---|---|
| `just linux-image <arch>` | Builds the `wheel` image (manylinux_2_28) and the `test` image (manylinux_2_34). |
| `just linux-regen-bindings <arch>` | Regenerates the Linux FFI bindings. |
| `just linux-check <arch>` | Builds, runs clippy and the offline tests. |
| `just linux-cargo <arch> <args>` | Runs cargo in the test image. |
| `just linux-wheel <arch>` | Builds the abi3 manylinux wheel into `target/wheels`. |
| `just linux-test <arch> [py] [pytest args]` | Installs the wheel on one CPython and runs pytest. Arguments replace the default `tests/`, so give the path too: `just linux-test aarch64 3.12 tests/ -rs`. |

The wheel is built on manylinux_2_28 and needs only glibc 2.28. The SDK
needs `GLIBCXX_3.4.26` and `libuuid.so.1`, so the tests run on
manylinux_2_34. A Linux wheel is stale after any Rust change: rebuild it
before `just linux-test`.

## Bindings

The FFI bindings are generated by bindgen and committed, one file per
target. The files differ between targets, so never copy one to another
target. A new SDK function goes into the allowlist in
`tools/regen-bindings.ps1` (`tools/regen-bindings.sh` reads it), and every
bindings file is regenerated: `just regen-bindings` on Windows and on macOS,
`just linux-regen-bindings aarch64` and `x86_64` in Docker.

## Rules for a change

- Rust: clippy clean (`just lint`). No `unsafe` outside the FFI module. Every
  `unsafe` block has a safety comment.
- Python: full type hints, ruff clean, mypy strict clean.
- Compat-only machinery has "compat" in its name (`CompatOptions`,
  `compat=`, `compat_input.rs`). It has a comment that says it is not from
  the SDK and exists only for pyrfc compatibility. Compat validation lives in
  `pyrfcx-py` or in the facade, never in `pyrfcx-core`.
- The core is lossless. A lossy projection lives only in compat machinery.
- Never invent SDK behaviour. Read the SDK header and SAP's programming
  guide (from the SAP Support Portal) first. If they are unclear, ask, then
  probe.
- Probe before you assert, also when the answer looks obvious. Record which
  SDK DLL produced a probe result (`pyrfcx.sdk_version()`), not only a
  version label.
- A native-path bug that corrupts data silently is a defect to fix now. When
  you add a path from a Python value to a field type, check whether the SDK
  setter accepts bad input without an error.
- Keep the test count small: merge related assertions into one test. Assert
  the exact error type and message.
- Commit messages: conventional commits, one line (`feat: ...`, `fix: ...`,
  `docs: ...`).

## Differences from pyrfc

A facade behaviour that differs from pyrfc 3.4 is a bug unless it is
deliberate and documented with its reason. The README lists the pyrfc bugs
the facade keeps; report any other difference as an issue.
