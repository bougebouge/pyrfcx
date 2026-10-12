#!/usr/bin/env bash
# linux-test.sh -- Install the Linux wheel in a fresh venv and run pytest.
#
# Runs INSIDE the test image (tools/docker/Dockerfile.linux, manylinux_2_34
# base), from the repo root. `just linux-test <arch>` starts it.
#
# Usage: linux-test.sh <python-version> [pytest args...]
#   python-version: 3.10 ... 3.14 (a CPython under /opt/python)
#   pytest args default to tests/
#
# Expects the wheel in target/wheels (`just linux-wheel <arch>`) and the
# SDK mounted at vendor/nwrfcsdk. Live tests also need sapnwrfc.ini in the
# repo root and SNC_LIB / SECUDIR in the environment.

set -euo pipefail

py="$1"
shift
tag="cp${py/./}"
arch="$(uname -m)"

wheel="$(ls target/wheels/pyrfcx-*-cp310-abi3-manylinux_*_"$arch".whl | sort | tail -n 1)"
venv="/tmp/venv-$tag"

uv venv --clear -q --python "/opt/python/$tag-$tag/bin/python" "$venv"
export VIRTUAL_ENV="$venv"
uv pip install -q "$wheel" "pytest>=8"

echo "wheel: $wheel"
SAPNWRFC_HOME="$PWD/vendor/nwrfcsdk" "$venv/bin/python" -c \
    'import sys, pyrfc, pyrfcx; print(sys.version.split()[0], pyrfcx.sdk_version())'

# Keep the bind-mounted tree clean: no bytecode, no pytest cache.
export PYTHONDONTWRITEBYTECODE=1
if [ "$#" -eq 0 ]; then
    set -- tests/
fi
exec "$venv/bin/python" -m pytest -p no:cacheprovider "$@"
