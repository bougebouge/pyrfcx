# pyrfcx development tasks — UV-based workflow
#
# The SAP NW RFC SDK for Windows is x64-only, so every Windows host builds
# x86_64-pc-windows-msvc (also on ARM64, with an x64 Python in the venv).
# A macOS or Linux host builds its own triple. The recipes export the choice
# as CARGO_BUILD_TARGET; .cargo/config.toml sets no target.

maturin_target := if os() == "macos" { arch() + "-apple-darwin" } else if os() == "linux" { arch() + "-unknown-linux-gnu" } else { "x86_64-pc-windows-msvc" }

export CARGO_BUILD_TARGET := maturin_target

# Read by tox.toml; its defaults are the Windows values.
# sysconfig.get_platform() says arm64 on macOS but aarch64 on Linux.
export PYRFCX_TOX_ARCH := if os() == "windows" { "x86_64" } else { arch() }
export PYRFCX_TOX_PLATFORM := if os() == "macos" { "-" + replace(arch(), "aarch64", "arm64") } else if os() == "linux" { "linux-" + arch() } else { "win-amd64" }
export PYRFCX_TOX_TARGET := maturin_target

# Venv holding REAL pyrfc 3.4, the comparison oracle (see `pyrfc-real-setup`
# / `test-pyrfc-real`). Dev-only; gitignored.
pyrfc_real_venv := ".venv-pyrfc-real"
pyrfc_real_python := if os_family() == "windows" { pyrfc_real_venv + "/Scripts/python.exe" } else { pyrfc_real_venv + "/bin/python" }

# pyrfc 3.4 source for the oracle. A local checkout named by the environment
# variable PYRFC_REAL_SRC wins when that folder exists; otherwise
# `pyrfc-real-setup` clones the archived repository into .pyrfc-real-src
# (gitignored). GitHub has no tag for 3.4: the archived `main` branch IS
# 3.4 (pyproject.toml `version = "3.4"`), frozen at the commit below.
pyrfc_real_repo := "https://github.com/SAP/PyRFC"
pyrfc_real_ref := "main"
pyrfc_real_commit := "5d4a20a5aee37accf32fc44ed8d668bd69332169"
pyrfc_real_clone := ".pyrfc-real-src"

# Default: list available targets
default:
    @just --list

# Build the extension module in dev mode into the active venv
develop:
    uv run maturin develop --uv --target {{ maturin_target }}

# Run all tests: Rust workspace + Python pytest
test: develop
    cargo test --workspace
    uv run pytest tests/

# Run Rust tests only (no SDK required)
test-rust:
    cargo test --workspace

# Run Python tests only (assumes `just develop` already ran)
test-python *args:
    uv run pytest tests/ {{ args }}

# Every x64 CPython 3.10-3.14 against one shared abi3 wheel (tox.toml);
# extra args go to pytest, e.g. `just test-all-py -e py312 -- tests/compat`
# Run the Python suite on every supported CPython (x64)
test-all-py *args:
    uv run tox {{ args }}

# Not a defect: in ABAP a runtime error IS a short dump, so asserting the
# AbapRuntimeError mapping requires causing one, and the stimulus is a
# SAP-standard FM (RFC_RAISE_ERROR / STFC_SAPGUI). Expect ~6 fresh dumps.

# Run the ST22-dumping tests (deselected by default; run before a commit)
test-dumps:
    uv run pytest tests/ -m dumps

# ── Real pyrfc 3.4 — the comparison oracle ──────────────────────────────
# tests/compat is adapted from pyrfc's own suite, so running it
# against REAL pyrfc proves the expectations are pyrfc's and not ours (the
# anti-circularity rule). Both stacks load the SAME vendored SDK: the pin
# lives in tests/conftest.py, so neither target needs SAPNWRFC_HOME set in
# the environment — that ambient dependency is exactly the bug this avoids.
#
# pyrfc is NOT on PyPI (SAP ships wheels via GitHub releases only), so it is
# built from source; it caps at Python 3.12, hence its own venv. The source
# comes from PYRFC_REAL_SRC or a shallow clone (see pyrfc_real_ref above).
# Windows also needs the x64 MSVC toolchain — that setup lives in
# tools/build-pyrfc-real.cmd, see the comments there.
# Cython is pinned below 3.1 like the released pyrfc 3.4 wheels: pyrfc's
# source uses `long`, which Cython 3.1+ leaves undefined.

# Build the real pyrfc 3.4 oracle venv (dev-only, one-off)
[windows]
pyrfc-real-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    src="${PYRFC_REAL_SRC:-}"
    if [ -z "$src" ] || [ ! -d "$src" ]; then
        src="{{ pyrfc_real_clone }}"
        [ -d "$src" ] || git clone --depth 1 --branch {{ pyrfc_real_ref }} {{ pyrfc_real_repo }} "$src"
        head="$(git -C "$src" rev-parse HEAD)"
        [ "$head" = "{{ pyrfc_real_commit }}" ] || { echo "pyrfc clone is at $head, expected {{ pyrfc_real_commit }} (3.4)"; exit 1; }
    fi
    echo "pyrfc source: $src"
    uv venv --clear --python cpython-3.12-windows-x86_64 {{ pyrfc_real_venv }}
    uv pip install --python {{ pyrfc_real_venv }}/Scripts/python.exe "Cython>=3.0,<3.1" "setuptools>=67" wheel pytest
    cmd //c tools\\build-pyrfc-real.cmd "{{ justfile_directory() }}\\{{ pyrfc_real_venv }}\\Scripts\\python.exe" "{{ justfile_directory() }}" "$(cd "$src" && pwd -W)"

# pyrfc's setup.py cythonises only on Linux unless PYRFC_BUILD_CYTHON is set,
# and links with an rpath to $SAPNWRFC_HOME/lib. That path is the
# vendor/nwrfcsdk symlink, so repointing the symlink also switches the SDK
# that real pyrfc loads. The final call enters the SDK: a bare import proves
# nothing (pyrfc's __init__ swallows SDK setup errors).

# Build the real pyrfc 3.4 oracle venv (dev-only, one-off)
[unix]
pyrfc-real-setup:
    #!/usr/bin/env bash
    set -euo pipefail
    src="${PYRFC_REAL_SRC:-}"
    if [ -z "$src" ] || [ ! -d "$src" ]; then
        src="{{ pyrfc_real_clone }}"
        [ -d "$src" ] || git clone --depth 1 --branch {{ pyrfc_real_ref }} {{ pyrfc_real_repo }} "$src"
        head="$(git -C "$src" rev-parse HEAD)"
        [ "$head" = "{{ pyrfc_real_commit }}" ] || { echo "pyrfc clone is at $head, expected {{ pyrfc_real_commit }} (3.4)"; exit 1; }
    fi
    echo "pyrfc source: $src"
    uv venv --clear --python cpython-3.12 {{ pyrfc_real_venv }}
    uv pip install --python {{ pyrfc_real_python }} "Cython>=3.0,<3.1" "setuptools>=67" wheel pytest
    SAPNWRFC_HOME="{{ justfile_directory() }}/vendor/nwrfcsdk" PYRFC_BUILD_CYTHON=1 \
        uv pip install --python {{ pyrfc_real_python }} --no-build-isolation --no-cache --reinstall-package pyrfc "$(cd "$src" && pwd)"
    SAPNWRFC_HOME="{{ justfile_directory() }}/vendor/nwrfcsdk" {{ pyrfc_real_python }} -c "import pyrfc; print('built pyrfc', pyrfc.__version__, pyrfc.get_nwrfclib_version())"

# Expect the same passes as our facade; the xfails we carry should XPASS
# here — those are our unbuilt surface, not pyrfc quirks.

# Run tests/compat against REAL pyrfc 3.4 (needs `just pyrfc-real-setup`)
test-pyrfc-real *args:
    {{ pyrfc_real_python }} -m pytest tests/compat/ -q {{ args }}

# Our facade and real pyrfc, back to back, on the same vendored SDK
test-compat-both *args: develop
    @echo "── pyrfcx compat facade ─────────────────────────────"
    uv run pytest tests/compat/ -q {{ args }}
    @echo "── real pyrfc 3.4 ───────────────────────────────────"
    just test-pyrfc-real {{ args }}

# Lint everything: cargo fmt/clippy + ruff + mypy
lint:
    cargo fmt --check
    cargo clippy --workspace --all-targets -- -D warnings
    uv run ruff check .
    uv run ruff format --check .
    uv run mypy python/ pyrfcx-compat/src/ tests/

# Format everything
fmt:
    cargo fmt
    uv run ruff format .
    uv run ruff check --fix .

# Rust writes the source path of every panic site into the binary. For a
# dependency that path is under CARGO_HOME, so a wheel built here would name
# the build user's home folder. The release build remaps both prefixes to
# neutral names; the later, longer prefix wins. RUSTFLAGS changes the build fingerprint, so this recipe
# builds into its own target directory and leaves `cargo test` builds alone.
home_dir := if os_family() == "windows" { env("USERPROFILE") } else { env("HOME") }
cargo_home := env("CARGO_HOME", home_dir / ".cargo")
release_rustflags := "--remap-path-prefix=" + home_dir + "=/home --remap-path-prefix=" + cargo_home + "=/cargo"

# Build a release wheel for the host platform (paths remapped, no SBOM).
# Release wheels come from the GitHub workflow (.github/workflows/wheels.yml).
wheels:
    RUSTFLAGS="{{ release_rustflags }}" CARGO_TARGET_DIR=target/wheels-build uv run maturin build --release --out target/wheels --target {{ maturin_target }}

# The wheel ships this file (pyproject.toml license-files). Run it after a
# dependency change. Targets and accepted licenses are in about.toml.
# Needs cargo-about: `cargo install cargo-about --locked --features cli`
# (on ARM64 Windows add `--target x86_64-pc-windows-msvc`).

# Regenerate THIRD-PARTY-LICENSES for the crates linked into the wheel
third-party-licenses:
    cargo about generate --manifest-path crates/pyrfcx-py/Cargo.toml --locked --fail --output-file THIRD-PARTY-LICENSES about.hbs

# Regenerate FFI bindings (maintainer only — needs LLVM + vendored headers)
[windows]
regen-bindings:
    powershell -NoProfile -ExecutionPolicy Bypass -File tools/regen-bindings.ps1

# Regenerate this host's FFI bindings (maintainer only — needs libclang + SDK headers)
[unix]
regen-bindings:
    bash tools/regen-bindings.sh

# ── Linux in Docker ─────────────────────────────────────────────────────
# arch is aarch64 or x86_64. The images hold toolchains only; the SDK,
# crypto library and PSE come from PYRFCX_LINUX_SDK, PYRFCX_LINUX_CRYPTO
# and PYRFCX_LINUX_SECUDIR (see tools/docker/linux-run.sh). Two images per
# arch: `wheel` (manylinux_2_28 base, widest tag) and `test` (manylinux_2_34
# base: the SDK needs libstdc++ GLIBCXX_3.4.26, which 2_28 lacks).

# Build the Linux wheel and test images for one architecture
[unix]
linux-image arch:
    docker build -f tools/docker/Dockerfile.linux --platform linux/{{ if arch == "x86_64" { "amd64" } else { "arm64" } }} --build-arg BASE=quay.io/pypa/manylinux_2_28_{{ arch }} -t pyrfcx-linux-wheel:{{ arch }} tools/docker
    docker build -f tools/docker/Dockerfile.linux --platform linux/{{ if arch == "x86_64" { "amd64" } else { "arm64" } }} --build-arg BASE=quay.io/pypa/manylinux_2_34_{{ arch }} -t pyrfcx-linux-test:{{ arch }} tools/docker

# Regenerate the Linux FFI bindings for one architecture
[unix]
linux-regen-bindings arch:
    bash tools/docker/linux-run.sh test {{ arch }} bash tools/regen-bindings.sh

# Build, clippy and offline tests on Linux (incl. the repository-fixture tier)
[unix]
linux-check arch:
    bash tools/docker/linux-run.sh test {{ arch }} 'cargo build --workspace && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && cargo test -p pyrfcx-core --test repository_roundtrip -- --ignored'

# Run cargo in the Linux test image, e.g. `just linux-cargo aarch64 test --workspace -- --include-ignored`
[unix]
linux-cargo arch *args:
    bash tools/docker/linux-run.sh test {{ arch }} cargo {{ args }}

# Build the abi3 manylinux wheel into target/wheels
[unix]
linux-wheel arch:
    bash tools/docker/linux-run.sh wheel {{ arch }} 'CARGO_TARGET_DIR=/build/target/wheel maturin build --release -i /opt/python/cp310-cp310/bin/python -o target/wheels'

# Install the Linux wheel on one CPython and run pytest (args default to tests/)
[unix]
linux-test arch py="3.10" *args:
    bash tools/docker/linux-run.sh test {{ arch }} bash tools/docker/linux-test.sh {{ py }} {{ args }}

# NB: `uv sync --dev` (dependency-groups) would PRUNE the venv — our dev deps
# are extras, so it removes pytest/ruff/mypy/maturin/pyrfcx-compat and leaves
# you reinstalling by hand. `--extra dev` is the correct flag; the editable
# installs below are not in the lock and must be restored after any sync.

# Sync/create the project venv with all dev dependencies (+ editable installs)
sync:
    uv sync --extra dev
    uv run maturin develop --uv --target {{ maturin_target }}
    uv pip install -e pyrfcx-compat/

# Internal-only recipes; the file is absent in the public tree.
import? 'justfile.internal'
