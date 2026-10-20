#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_FILE="$(mktemp)"
trap 'rm -f "${RUNTIME_FILE}"' EXIT
uv export --locked --project "${SCRIPT_DIR}" --no-dev --no-emit-project --no-hashes --output-file "${RUNTIME_FILE}" >/dev/null
python3 -m pip_audit --no-deps --disable-pip -r "${RUNTIME_FILE}"
