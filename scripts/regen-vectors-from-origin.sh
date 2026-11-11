#!/usr/bin/env bash
#
# Regenerate vectors/price-vectors.jsonl, vectors/v2-vectors.jsonl and vectors/manifest.json from the
# pinned origin checkout. The run is local and chain-free: forge executes the generator scripts
# against its own EVM simulation. The generated files are committed; nothing in the build or test
# path invokes this script.
#
# Requires git, forge, solc and either python3 or node on PATH.

set -euo pipefail

ORIGIN_REPO=https://github.com/vibecodermaxi/websitekit
ORIGIN_COMMIT=82ff6db8d7239475aafbc5aac23175907583fa80
CONTRACTS_SUBDIR=packages/websitekit-contracts

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
VECTORS_DIR="${REPO_ROOT}/vectors"
WORK_DIR="$(mktemp -d)"

cleanup() {
  rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

require() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "error: $1 is required on PATH" >&2
    exit 1
  fi
}

require git
require forge
# forge downloads the pinned solc itself; a standalone solc on PATH is optional.

if ! command -v python3 >/dev/null 2>&1 && ! command -v node >/dev/null 2>&1; then
  echo "error: manifest generation needs python3 or node on PATH" >&2
  exit 1
fi

echo "cloning ${ORIGIN_REPO} at ${ORIGIN_COMMIT}"
git clone --quiet "${ORIGIN_REPO}" "${WORK_DIR}/origin"
git -C "${WORK_DIR}/origin" checkout --quiet "${ORIGIN_COMMIT}"

CHECKED_OUT="$(git -C "${WORK_DIR}/origin" rev-parse HEAD)"
if [ "${CHECKED_OUT}" != "${ORIGIN_COMMIT}" ]; then
  echo "error: checkout is at ${CHECKED_OUT}, expected ${ORIGIN_COMMIT}" >&2
  exit 1
fi

ORIGIN_CONTRACTS="${WORK_DIR}/origin/${CONTRACTS_SUBDIR}"
if [ ! -d "${ORIGIN_CONTRACTS}" ]; then
  echo "error: ${CONTRACTS_SUBDIR} is absent from the origin checkout" >&2
  exit 1
fi

for generator in GenPriceVectors GenV2Vectors; do
  echo "running script/${generator}.s.sol"
  ( cd "${ORIGIN_CONTRACTS}" && forge script "script/${generator}.s.sol" )
done

count_rows() {
  awk 'NF > 0' "$1" | wc -l | tr -d ' '
}

# The generators write through vm.writeLine to ./out (see fs_permissions in foundry.toml).
copy_vector() {
  local name="$1"
  local dest="$2"
  local src=""
  if [ -s "${ORIGIN_CONTRACTS}/out/${name}.jsonl" ]; then
    src="${ORIGIN_CONTRACTS}/out/${name}.jsonl"
  elif [ -s "${ORIGIN_CONTRACTS}/${name}.jsonl" ]; then
    src="${ORIGIN_CONTRACTS}/${name}.jsonl"
  else
    echo "error: ${name}.jsonl was not produced by the generator run" >&2
    exit 1
  fi
  cp "${src}" "${dest}"
}

mkdir -p "${VECTORS_DIR}"
PRICE_FILE="${VECTORS_DIR}/price-vectors.jsonl"
V2_FILE="${VECTORS_DIR}/v2-vectors.jsonl"

copy_vector price-vectors "${PRICE_FILE}"
copy_vector v2-vectors "${V2_FILE}"

PRICE_ROWS="$(count_rows "${PRICE_FILE}")"
V2_ROWS="$(count_rows "${V2_FILE}")"

if [ "${PRICE_ROWS}" -eq 0 ]; then
  echo "error: price-vectors.jsonl has no rows" >&2
  exit 1
fi
if [ "${V2_ROWS}" -eq 0 ]; then
  echo "error: v2-vectors.jsonl has no rows" >&2
  exit 1
fi

FORGE_VERSION="$(forge --version | head -n 1)"
if command -v solc >/dev/null 2>&1; then
  SOLC_VERSION="$(solc --version | tr -s '[:space:]' ' ' | sed 's/^ //; s/ $//')"
else
  # foundry.toml pins the compiler; report the pin forge will fetch.
  SOLC_VERSION="$(forge config 2>/dev/null | sed -n 's/^solc = //p' | head -n 1)"
  SOLC_VERSION="${SOLC_VERSION:-unknown (forge-managed)}"
fi

export STELE_VECTORS_DIR="${VECTORS_DIR}"
export STELE_PRICE_FILE="${PRICE_FILE}"
export STELE_V2_FILE="${V2_FILE}"
export STELE_PRICE_ROWS="${PRICE_ROWS}"
export STELE_V2_ROWS="${V2_ROWS}"
export STELE_FORGE_VERSION="${FORGE_VERSION}"
export STELE_SOLC_VERSION="${SOLC_VERSION}"

if command -v python3 >/dev/null 2>&1; then
  python3 - <<'PY'
import hashlib
import json
import os
import pathlib


def entry(path, rows):
    data = pathlib.Path(path).read_bytes()
    return {
        "rows": int(rows),
        "sha256": hashlib.sha256(data).hexdigest(),
        "forge": os.environ["STELE_FORGE_VERSION"],
        "solc": os.environ["STELE_SOLC_VERSION"],
    }


manifest = {
    "price": entry(os.environ["STELE_PRICE_FILE"], os.environ["STELE_PRICE_ROWS"]),
    "v2": entry(os.environ["STELE_V2_FILE"], os.environ["STELE_V2_ROWS"]),
}

out = pathlib.Path(os.environ["STELE_VECTORS_DIR"]) / "manifest.json"
out.write_text(json.dumps(manifest, indent=2) + "\n", encoding="ascii")
PY
else
  node - <<'JS'
const crypto = require("node:crypto");
const fs = require("node:fs");
const path = require("node:path");

function entry(file, rows) {
  const data = fs.readFileSync(file);
  return {
    rows: Number(rows),
    sha256: crypto.createHash("sha256").update(data).digest("hex"),
    forge: process.env.STELE_FORGE_VERSION,
    solc: process.env.STELE_SOLC_VERSION,
  };
}

const manifest = {
  price: entry(process.env.STELE_PRICE_FILE, process.env.STELE_PRICE_ROWS),
  v2: entry(process.env.STELE_V2_FILE, process.env.STELE_V2_ROWS),
};

const out = path.join(process.env.STELE_VECTORS_DIR, "manifest.json");
fs.writeFileSync(out, JSON.stringify(manifest, null, 2) + "\n");
JS
fi

if [ ! -s "${VECTORS_DIR}/manifest.json" ]; then
  echo "error: manifest.json was not written" >&2
  exit 1
fi

echo "price-vectors.jsonl rows: ${PRICE_ROWS}"
echo "v2-vectors.jsonl rows: ${V2_ROWS}"
echo "wrote ${VECTORS_DIR}/manifest.json"
