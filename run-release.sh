#!/usr/bin/env bash
set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
cd "$script_dir"

GHOSTTY_VT_LIB_DIR="$(fd -a '^libghostty-vt\.so\.0$' target/release/build | sed 's#/libghostty-vt.so.0$##' | head -n1)"
LD_LIBRARY_PATH="$GHOSTTY_VT_LIB_DIR${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" exec target/release/octty
