#!/bin/bash
# Save a PNG of the hidden window: tools/shot.sh <name>  -> ui-shots/<name>.png (rendered offscreen, no screen needed)
cd "$(dirname "$0")/.."
mkdir -p ui-shots
target/release/wfm.exe gui shot "$(cygpath -w "$PWD/ui-shots/$1.png")" >/dev/null || exit 1
echo "ui-shots/$1.png"
