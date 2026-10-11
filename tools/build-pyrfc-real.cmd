@echo off
REM Build REAL pyrfc 3.4 from its source checkout into the oracle venv.
REM
REM Invoked by `just pyrfc-real-setup`; not part of the shipped product.
REM pyrfc is the comparison oracle for tests/compat: the suite
REM is adapted from pyrfc's own tests, so running it against genuine pyrfc is
REM what proves our expectations are pyrfc's and not our own.
REM
REM Why a .cmd and not a line in the justfile: the MSVC environment must be
REM initialised before setuptools runs, and quoting `call "...vcvars.bat"`
REM through Git Bash into cmd mangles beyond repair.
REM
REM Args:  %1 = python.exe of the oracle venv   %2 = repo root
REM        %3 = pyrfc 3.4 source folder (the justfile clones it when needed)
setlocal

set "VENV_PY=%~1"
set "REPO=%~2"
set "PYRFC_SRC=%~3"

REM ARM64 host, x64 target: the SDK is x64-only and SAP ships no ARM64
REM Windows build. vcvarsarm64_amd64 = native ARM64 compiler emitting
REM x64, so no emulation. Without this, setuptools guesses the host wrong and
REM reaches for vcvarsall x86_amd64, which fails here.
REM NB: expanding %VCVARS% inside an if(...) block would break cmd parsing —
REM the path contains "(x86)" and its ")" closes the block early. Hence goto.
set "VCVARS=%ProgramFiles(x86)%\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvarsarm64_amd64.bat"
if not exist "%VCVARS%" goto :no_vcvars
call "%VCVARS%" >nul || exit /b 1
goto :have_vcvars

:no_vcvars
echo ERROR: MSVC ARM64-to-x64 cross-toolchain not found at:
echo "%VCVARS%"
echo Install VS 2022 Build Tools with the ARM64/x64 cross compiler.
exit /b 1

:have_vcvars

REM Tell setuptools the compiler environment is already configured, so it
REM does not try to locate and re-run vcvarsall itself.
set DISTUTILS_USE_SDK=1
set MSSdk=1

REM pyrfc's setup.py exits without SAPNWRFC_HOME, and only cythonises when
REM PYRFC_BUILD_CYTHON is set on Windows (the checkout has no generated
REM _cyrfc.c). PYTHONSOURCE is pyrfc's own knob, written for a CPython source
REM tree; setuptools supplies the real include/lib dirs, so the interpreter
REM root is enough and its bogus -I/-LIBPATH entries are harmless.
set "SAPNWRFC_HOME=%REPO%\vendor\nwrfcsdk"
set PYRFC_BUILD_CYTHON=1
for /f "delims=" %%P in ('"%VENV_PY%" -c "import sys; print(sys.base_prefix)"') do set "PYTHONSOURCE=%%P"

uv pip install --python "%VENV_PY%" --no-build-isolation "%PYRFC_SRC%" || exit /b 1

REM Prove it links and reaches the SDK: a bare import is not enough, because
REM pyrfc's __init__ swallows a missing SAPNWRFC_HOME with suppress(Exception)
REM and the failure only resurfaces later as an opaque ICU error.
REM The SDK loads ICU itself and searches PATH for it; without this line the
REM check stops on a modal "could not open ICU library" dialog.
set "PATH=%SAPNWRFC_HOME%\lib;%PATH%"
"%VENV_PY%" -c "import pyrfc; print('built pyrfc', pyrfc.__version__, pyrfc.get_nwrfclib_version())" || exit /b 1
