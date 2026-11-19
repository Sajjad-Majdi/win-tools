#!/bin/bash
# Start RustFileExplorer.exe on the REAL desktop but far off every monitor (--offscreen: x,y = -20000, no taskbar button,
# never activated), so `wfm gui shot-real <file>` can capture what only DWM draws (caption buttons, backdrop).
# Nothing appears on the screen. Same test isolation as start_hidden.sh. Stop it with `wfm gui quit`.
cd "$(dirname "$0")/.."
EXE="$(cygpath -w "$PWD/target/release/RustFileExplorer.exe")"
export WFILES_TEST=1
export WFM_PIPE="${WFM_PIPE:-wintools-files-test}"
export WFM_DATA_DIR="${WFM_DATA_DIR:-$(cygpath -w "$TEMP")\wfiles-test-data}"
mkdir -p "$(cygpath -u "$WFM_DATA_DIR")"
ARGS="--offscreen"
for a in "$@"; do ARGS="$ARGS \"$a\""; done
powershell -NoProfile -ExecutionPolicy Bypass -File tools/hidden_start.ps1 -Exe "$EXE" -ArgLine "$ARGS" -Desktop Default
for i in $(seq 1 60); do target/release/wfm.exe gui state >/dev/null 2>&1 && echo "ready" && exit 0; sleep 0.1; done
echo "window did not answer"; exit 1
