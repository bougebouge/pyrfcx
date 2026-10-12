# SDK patch levels

This page lists the changes between patch levels of the SAP NW RFC SDK 7.50
that a program can see when it uses `pyrfc`. Read it before you update the
SDK on a server.

The SDK causes each change, not the Python library. So each change applies
to pyrfc 3.4 and to `pyrfc` from pyrfcx in the same way.

## Tested patch levels

- Tested: 11, 14, 15, 18, 19 and 20.
- Not tested: 12, 13, 16 and 17.

The patch level that introduced each change comes from the SDK's release
notes, the file `doc/release_notes.txt` in the SDK.

## Changes

### Patch level 12: new text for a missing host parameter

- SDK change: the error message for a missing host parameter names the
  WebSocket ports.
- Patch level 11 gives: `Parameter ASHOST, GWHOST, MSHOST or PORT is missing.`
- Patch level 12 and later give:
  `Parameter ASHOST, GWHOST, MSHOST, WSPORT or TCPPORT is missing.`

### Patch level 15: unknown name in `not_requested` raises an error

- SDK change: `RfcSetParameterActive` rejects a parameter name that the
  function module does not have.
- Below patch level 15: an unknown name in
  `options={"not_requested": [...]}` is ignored.
- Patch level 15 and later: the call raises `ExternalRuntimeError` with the
  key `RFC_INVALID_PARAMETER`.

### Patch levels 15 to 17: empty BCD field cannot be read

- SDK change: `RfcGetString` on an empty BCD field without decimals returns
  an empty string. This is a regression. Patch level 18 fixes it.
- Patch levels 15 to 17: reading such a field raises
  `decimal.InvalidOperation`.
- Patch level 18 and later: the field reads as `Decimal('0')`.
- Patch levels 11 and 14 are not measured for this case.

### Patch level 18: empty value sets the initial value

- SDK change: `RfcSetString` with an empty value sets the field to its
  initial value.
- Below patch level 18: the SDK rejects `""` for DATE, TIME, INT, BCD,
  DECFLOAT, BYTE and UTCLONG fields with `RFC_CONVERSION_FAILURE`.
- A program sees no difference. pyrfc does not call the setter for an empty
  DATE or TIME value, and it rejects `""` for the other types itself.

### Patch level 19: ICU libraries have new file names

- SDK change: the ICU library files have new names. Linux: `libicuuc57.so`
  instead of `libicuuc.so.50`. macOS: `libicuuc57.dylib` instead of
  `libicuuc.50.dylib`.
- A program sees no difference.

### Patch level 19: trace files are written with no trace setting

- SDK change: a part of the RFC trace is always on. This is a regression.
  Patch level 20 fixes it.
- Patch level 19: the SDK writes RFC trace files (`dev_rfc*.trc`) in the
  working directory. No trace setting in `sapnwrfc.ini` or in the
  environment is needed for this.
- Patch level 20 and later: the SDK writes trace files only when a trace is
  switched on.
