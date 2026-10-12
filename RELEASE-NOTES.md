# Release notes

## 0.2.0

Released 2026-10-12. First release.

- `pip install pyrfcx` installs one wheel with two import packages: `pyrfc`
  and `pyrfcx`. The native `pyrfcx` API is not a supported interface yet.
- `pyrfc` from pyrfcx covers the client part of pyrfc 3.4:
  - `Connection` with `call`, `ping`, `open`, `close`, `reopen`, `cancel`,
    `is_valid`, `alive`, `version`, `options`, `get_connection_attributes`
    and `get_function_description`, and use as a context manager.
  - The `config` keys `rstrip`, `dtime`, `check_date`, `check_time`,
    `return_import_params` and `timeout`.
  - The `options` of `call`: `not_requested` and `timeout`.
  - `FunctionDescription`, `TypeDescription`, `RfcFieldType` and
    `RfcParameterDirection`.
  - The module functions `get_nwrfclib_version`, `cancel_connection`,
    `set_ini_file_directory`, `reload_ini_file` and `set_cryptolib_path`.
  - The pyrfc exception classes, with the same names and attributes.
- Not supported: server RFC, tRFC, qRFC and bgRFC. The drop-in is a client
  only.
- Wheels for Windows x64, Linux x86_64 and aarch64 (`manylinux_2_28`) and
  macOS arm64. One abi3 wheel per platform serves CPython 3.10 and later.
- Requires the SDK 7.50, patch level 11 or later. Set `SAPNWRFC_HOME` to the
  SDK directory. Patch levels 11, 14, 15, 18, 19 and 20 are tested.
- The behaviour differences between patch levels are listed in
  [docs/sdk-patch-levels.md](docs/sdk-patch-levels.md).
- Licence: Apache-2.0.

0.1.0 was published with a two-package layout and withdrawn. There is no 0.1.1.
