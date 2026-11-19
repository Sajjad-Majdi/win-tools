#!/bin/bash
# Save a PNG of the open (hidden) Win Tools window: tools/shot.sh <name>  -> design/build/<name>.png
cd "$(dirname "$0")/.."
D="$(cygpath -w "$PWD/design/build")"; mkdir -p design/build
app/target/release/wtm.exe gui shot "$D/$1.bmp" >/dev/null || exit 1
python -c "import sys; from PIL import Image; Image.open(sys.argv[1]).convert('RGB').save(sys.argv[2])" "$D/$1.bmp" "$D/$1.png" && rm -f "design/build/$1.bmp"
echo "design/build/$1.png"
