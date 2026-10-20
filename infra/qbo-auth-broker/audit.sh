#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
RUNTIME_FILE="$(mktemp)"
trap 'rm -f "${RUNTIME_FILE}"' EXIT
uv export --locked --project "${SCRIPT_DIR}" --no-dev --no-emit-project --no-hashes --no-header --output-file "${RUNTIME_FILE}" >/dev/null
diff -u "${SCRIPT_DIR}/requirements.txt" "${RUNTIME_FILE}"
uv sync --locked --project "${SCRIPT_DIR}"
"${SCRIPT_DIR}/.venv/bin/pip-audit" --no-deps --disable-pip -r "${RUNTIME_FILE}"
