#!/usr/bin/env bash
set -euo pipefail
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PACKAGE_DIR="$(mktemp -d)"
trap 'rm -rf "${PACKAGE_DIR}"' EXIT
mkdir -p "${SCRIPT_DIR}/dist"
python3 -m pip install --disable-pip-version-check --only-binary=:all: --no-compile \
  --target "${PACKAGE_DIR}" -r "${SCRIPT_DIR}/requirements.txt"
cp "${SCRIPT_DIR}/lambda/app.py" "${PACKAGE_DIR}/app.py"
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
