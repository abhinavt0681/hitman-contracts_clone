#!/usr/bin/env bash
# Decompiles your own copy of HitmanContracts.exe with headless Ghidra into data/re/ghidra/
# (gitignored): decompiled C per 64 KB of code, functions.tsv and strings_xrefs.tsv.
# Takes about 3 minutes. Usage: GHIDRA=/path/to/ghidra_12.1.4_PUBLIC tools/ghidra/decompile.sh
set -euo pipefail
cd "$(dirname "$0")/../.."
: "${GHIDRA:?set GHIDRA to your Ghidra install directory (needs JDK 21 on PATH or JAVA_HOME)}"
exe=data/HitmanContracts.exe
[ -f "$exe" ] || { echo "copy HitmanContracts.exe from your install to $exe first" >&2; exit 1; }
echo "$(sha256sum "$exe" | cut -d' ' -f1)  (addresses in docs/ match 189f8ce2…b6cd702, the Steam build)"
mkdir -p data/re/ghidra data/re/ghidra-project
"$GHIDRA/support/analyzeHeadless" data/re/ghidra-project HitmanContracts -overwrite \
    -import "$exe" -scriptPath tools/ghidra -postScript ExportAll.java data/re/ghidra \
    > data/re/ghidra/analyze.log 2>&1
echo "done: data/re/ghidra/ (log in analyze.log)"
