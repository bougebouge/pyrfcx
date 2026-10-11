#!/usr/bin/env bash
# linux-run.sh -- Run a command in the pyrfcx Linux image, repo mounted.
#
# Usage: linux-run.sh <wheel|test> <aarch64|x86_64> <command...>
#
# The images hold toolchains only (tools/docker/Dockerfile.linux). Every SAP
# file comes from the environment of the caller and is mounted at run time:
#   PYRFCX_LINUX_SDK      the unpacked Linux SDK for <arch> (the folder
#                         holding lib/ and include/). Required.
#   PYRFCX_LINUX_CRYPTO   folder holding libsapcrypto.so for <arch>.
#                         Optional; sets SNC_LIB for live tests.
#   PYRFCX_LINUX_SECUDIR  folder holding the PSE. Optional; mounted
#                         read-only and set as SECUDIR for live tests.
#   PYRFCX_LINUX_DOCKER_ARGS  extra `docker run` arguments. Optional.
# Live tests also read sapnwrfc.ini from the repo root, as on the host.
#
# The SDK is mounted at vendor/nwrfcsdk, the path the cargo and pytest pins
# use. When vendor/nwrfcsdk is a symlink, Docker resolves it inside the
# container, so the host link is never touched.
#
# Build output and the cargo registry live in named volumes per arch
# (pyrfcx-linux-target-<arch>, pyrfcx-linux-cargo-registry-<arch>), never in
# the host's target/ or venvs.

set -euo pipefail

kind="$1"
arch="$2"
shift 2

case "$arch" in
    aarch64) platform="linux/arm64" ;;
    x86_64) platform="linux/amd64" ;;
    *)
        echo "linux-run.sh: arch must be aarch64 or x86_64, not '$arch'" >&2
        exit 2
        ;;
esac

: "${PYRFCX_LINUX_SDK:?set PYRFCX_LINUX_SDK to the unpacked Linux SDK for $arch}"

repo="$(cd "$(dirname "$0")/../.." && pwd)"

args=(
    --rm --platform "$platform"
    -v "$repo:/work" -w /work
    -v "$PYRFCX_LINUX_SDK:/work/vendor/nwrfcsdk:ro"
    -v "pyrfcx-linux-target-$arch:/build/target"
    -v "pyrfcx-linux-cargo-registry-$arch:/opt/cargo/registry"
    -e CARGO_TARGET_DIR=/build/target
)
if [ -n "${PYRFCX_LINUX_CRYPTO:-}" ]; then
    args+=(-v "$PYRFCX_LINUX_CRYPTO:/opt/sapcrypto:ro" -e SNC_LIB=/opt/sapcrypto/libsapcrypto.so)
fi
if [ -n "${PYRFCX_LINUX_SECUDIR:-}" ]; then
    args+=(-v "$PYRFCX_LINUX_SECUDIR:/opt/sec:ro" -e SECUDIR=/opt/sec)
fi
# shellcheck disable=SC2206 # word splitting is the point
args+=(${PYRFCX_LINUX_DOCKER_ARGS:-})

exec docker run "${args[@]}" "pyrfcx-linux-$kind:$arch" bash -c "$*"
