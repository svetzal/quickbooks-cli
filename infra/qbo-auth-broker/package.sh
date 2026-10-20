#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(mktemp -d)"
trap 'rm -rf "${PACKAGE_DIR}"' EXIT
mkdir -p "${SCRIPT_DIR}/dist"
uv export --locked --project "${SCRIPT_DIR}" --no-dev --no-emit-project --no-hashes --output-file "${PACKAGE_DIR}/runtime.txt"
uv pip install --only-binary=:all: --no-compile-bytecode \
  --target "${PACKAGE_DIR}" -r "${PACKAGE_DIR}/runtime.txt"
rm "${PACKAGE_DIR}/runtime.txt"
cp "${SCRIPT_DIR}/lambda/"{app.py,pages.json,style.css} "${PACKAGE_DIR}/"
rm -f "${SCRIPT_DIR}/dist/qbo-auth-broker.zip"
python3 - "${PACKAGE_DIR}" "${SCRIPT_DIR}/dist/qbo-auth-broker.zip" <<'PYZIP'
import pathlib
import sys
import zipfile
root = pathlib.Path(sys.argv[1])
with zipfile.ZipFile(sys.argv[2], "w", compression=zipfile.ZIP_DEFLATED) as archive:
    for path in sorted(root.rglob("*")):
        if path.is_file():
            archive.write(path, path.relative_to(root))
PYZIP
