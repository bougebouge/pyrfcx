# regen-bindings.ps1 -- Regenerate committed FFI bindings from sapnwrfc.h
#
# By default, generates for the primary target only (x86_64-pc-windows-msvc).
# Pass -CrossCheck to run bindgen for all 4 targets and verify byte-identity.
# Cross-check requires sysroots for all targets (CI/release machines).
#
# Prerequisites: bindgen-cli (cargo install bindgen-cli), LLVM/libclang
# Usage: just regen-bindings  (or: powershell tools/regen-bindings.ps1)

param(
    [switch]$CrossCheck
)

$ErrorActionPreference = 'Stop'

# Auto-detect LIBCLANG_PATH if not set
if (-not $env:LIBCLANG_PATH) {
    $llvmBin = 'C:\Program Files\LLVM\bin'
    if (Test-Path "$llvmBin\libclang.dll") {
        $env:LIBCLANG_PATH = $llvmBin
        Write-Host "Auto-detected LIBCLANG_PATH: $llvmBin"
    }
}

$WrapperH = 'crates/pyrfcx-core/src/ffi/wrapper.h'
$OutputFile = 'crates/pyrfcx-core/src/ffi/bindings.rs'
$IncludeDir = 'vendor/nwrfcsdk/include'

$PrimaryTarget = 'x86_64-pc-windows-msvc'

# All four 64-bit targets we support
$AllTargets = @(
    'x86_64-pc-windows-msvc',
    'x86_64-unknown-linux-gnu',
    'aarch64-unknown-linux-gnu',
    'aarch64-apple-darwin'
)

if ($CrossCheck) {
    $Targets = $AllTargets
} else {
    $Targets = @($PrimaryTarget)
}

# Function allowlist (incl. the repository functions)
$Functions = @(
    # Connection lifecycle
    'RfcOpenConnection', 'RfcCloseConnection',
    # Descriptor lookup
    'RfcGetFunctionDesc', 'RfcGetCachedFunctionDesc',
    # Container create/destroy
    'RfcCreateFunction', 'RfcDestroyFunction',
    'RfcCreateStructure', 'RfcDestroyStructure',
    'RfcCreateTable', 'RfcDestroyTable',
    # Invoke
    'RfcInvoke',
    # Typed getters (by name)
    'RfcGetChars', 'RfcGetNum', 'RfcGetString', 'RfcGetStringLength',
    'RfcGetDate', 'RfcGetTime', 'RfcGetBytes', 'RfcGetXString',
    'RfcGetInt', 'RfcGetInt1', 'RfcGetInt2', 'RfcGetInt8',
    'RfcGetFloat', 'RfcGetDecF16', 'RfcGetDecF34',
    'RfcGetStructure', 'RfcGetTable',
    # Typed setters (by name)
    'RfcSetChars', 'RfcSetNum', 'RfcSetString',
    'RfcSetDate', 'RfcSetTime', 'RfcSetBytes', 'RfcSetXString',
    'RfcSetInt', 'RfcSetInt1', 'RfcSetInt2', 'RfcSetInt8',
    'RfcSetFloat', 'RfcSetDecF16', 'RfcSetDecF34',
    # Typed getters (by index)
    'RfcGetCharsByIndex', 'RfcGetNumByIndex',
    'RfcGetStringByIndex', 'RfcGetStringLengthByIndex',
    'RfcGetDateByIndex', 'RfcGetTimeByIndex',
    'RfcGetBytesByIndex', 'RfcGetXStringByIndex',
    'RfcGetIntByIndex', 'RfcGetInt1ByIndex', 'RfcGetInt2ByIndex', 'RfcGetInt8ByIndex',
    'RfcGetFloatByIndex', 'RfcGetDecF16ByIndex', 'RfcGetDecF34ByIndex',
    'RfcGetStructureByIndex', 'RfcGetTableByIndex',
    # Typed setters (by index)
    'RfcSetCharsByIndex', 'RfcSetNumByIndex', 'RfcSetStringByIndex',
    'RfcSetDateByIndex', 'RfcSetTimeByIndex',
    'RfcSetBytesByIndex', 'RfcSetXStringByIndex',
    'RfcSetIntByIndex', 'RfcSetInt1ByIndex', 'RfcSetInt2ByIndex', 'RfcSetInt8ByIndex',
    'RfcSetFloatByIndex', 'RfcSetDecF16ByIndex', 'RfcSetDecF34ByIndex',
    # Table cursor
    'RfcGetRowCount', 'RfcMoveToFirstRow', 'RfcMoveToNextRow',
    'RfcMoveTo', 'RfcMoveToLastRow', 'RfcMoveToPreviousRow',
    'RfcGetCurrentRow', 'RfcAppendNewRow', 'RfcAppendRow',
    'RfcDeleteCurrentRow', 'RfcDeleteAllRows', 'RfcAppendNewRows',
    'RfcInsertNewRow', 'RfcInsertRow', 'RfcReserveCapacity', 'RfcGetRowType',
    # Metadata introspection
    'RfcGetFunctionName', 'RfcGetParameterCount',
    'RfcGetParameterDescByIndex', 'RfcGetParameterDescByName',
    'RfcGetExceptionCount', 'RfcGetExceptionDescByIndex',
    'RfcGetTypeName', 'RfcGetFieldCount',
    'RfcGetFieldDescByIndex', 'RfcGetFieldDescByName', 'RfcGetTypeLength',
    # Parameter activation
    'RfcSetParameterActive', 'RfcIsParameterActive',
    # Repository
    'RfcSaveRepository', 'RfcLoadRepository', 'RfcMetadataBatchQuery',
    # Error/utility
    'RfcGetVersion', 'RfcGetRcAsString', 'RfcGetTypeAsString',
    'RfcSetTraceDir', 'RfcSetTraceLevel', 'RfcSetIniPath', 'RfcReloadIniFile',
    'RfcLoadCryptoLibrary',
    # Connection utilities
    'RfcPing', 'RfcResetServerContext', 'RfcIsConnectionHandleValid',
    'RfcGetConnectionAttributes', 'RfcCancel'
)

# Type allowlist
$Types = @(
    'RFC_ERROR_INFO', 'RFC_PARAMETER_DESC', 'RFC_FIELD_DESC',
    'RFC_ATTRIBUTES', 'RFC_EXCEPTION_DESC', 'RFC_CONNECTION_PARAMETER',
    'RFCTYPE', 'RFC_RC', 'RFC_DIRECTION', 'RFC_ERROR_GROUP',
    'DecFloat16', 'DecFloat34',
    'RFC_CONNECTION_HANDLE', 'RFC_FUNCTION_DESC_HANDLE',
    'RFC_TYPE_DESC_HANDLE', 'DATA_CONTAINER_HANDLE',
    'RFC_STRUCTURE_HANDLE', 'RFC_FUNCTION_HANDLE', 'RFC_TABLE_HANDLE',
    'RFC_ABAP_NAME', 'RFC_PARAMETER_DEFVALUE', 'RFC_PARAMETER_TEXT'
)

# Build bindgen base args
$BaseArgs = @(
    $WrapperH,
    '--dynamic-loading', 'SapNwRfc',
    '--dynamic-link-require-all',
    '--default-enum-style', 'newtype',
    # The header's comments are SAP's text; our license cannot cover them.
    '--no-doc-comments'
)

foreach ($f in $Functions) {
    $BaseArgs += '--allowlist-function'
    $BaseArgs += $f
}
foreach ($t in $Types) {
    $BaseArgs += '--allowlist-type'
    $BaseArgs += $t
}

# Clang args (after --)
$ClangArgs = @(
    '--', '-DSAPwithUNICODE', '-DSAP_API='
)

$TempDir = [System.IO.Path]::GetTempPath() + 'pyrfcx-bindgen-' + [guid]::NewGuid().ToString('N').Substring(0, 8)
New-Item -ItemType Directory -Force $TempDir | Out-Null

Write-Host "Generating bindings for $($Targets.Count) target(s)..."

$BindgenVersion = (bindgen --version) -replace 'bindgen ', ''

foreach ($target in $Targets) {
    $outFile = "$TempDir/$target.rs"
    $targetArgs = $BaseArgs + @('-o', $outFile) + $ClangArgs + @("-I$IncludeDir", "--target=$target")

    Write-Host "  -> $target"
    & bindgen @targetArgs
    if ($LASTEXITCODE -ne 0) {
        Write-Error "bindgen failed for target $target"
        Remove-Item -Recurse -Force $TempDir
        exit 1
    }
}

# Cross-target diff check -- only when -CrossCheck is set
if ($CrossCheck -and $Targets.Count -gt 1) {
    Write-Host "Verifying cross-target identity..."
    $referenceFile = "$TempDir/$($Targets[0]).rs"
    $referenceHash = (Get-FileHash $referenceFile -Algorithm SHA256).Hash

    for ($i = 1; $i -lt $Targets.Count; $i++) {
        $target = $Targets[$i]
        $checkFile = "$TempDir/$target.rs"
        $checkHash = (Get-FileHash $checkFile -Algorithm SHA256).Hash
        if ($checkHash -ne $referenceHash) {
            Write-Error "MISMATCH: $($Targets[0]) vs $target -- bindings differ across targets. Aborting."
            Write-Host "Temp files preserved at $TempDir for inspection."
            exit 1
        }
    }
    Write-Host "All targets produce identical output."
    $verifiedTargets = $AllTargets -join ', '
} else {
    $verifiedTargets = $PrimaryTarget
}

# Write final file with header comment
$referenceFile = "$TempDir/$($Targets[0]).rs"
$date = Get-Date -Format 'yyyy-MM-dd'
$header = @"
// Auto-generated by bindgen $BindgenVersion -- do not edit.
// Regenerate with: just regen-bindings
// SDK headers: SAP NW RFC SDK 7.50 (vendor/nwrfcsdk/include/sapnwrfc.h)
// Defines: -DSAPwithUNICODE -DSAP_API=
// Date: $date
// Targets verified: $verifiedTargets

"@

$content = Get-Content $referenceFile -Raw
$header + $content | Set-Content $OutputFile -Encoding utf8 -NoNewline

# Cleanup
Remove-Item -Recurse -Force $TempDir

Write-Host "Written: $OutputFile"
Write-Host "Done."
