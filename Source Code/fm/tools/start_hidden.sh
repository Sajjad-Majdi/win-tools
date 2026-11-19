#!/bin/bash
# Start RustFileExplorer.exe on the never-shown desktop "wfiles_hidden" and wait until `wfm gui state` answers.
# Test mode: its own pipe (WFM_PIPE), its own settings + undo journal (WFM_DATA_DIR in %TEMP%\wfiles-test-data), no hand-off to another window.
# Usage: tools/start_hidden.sh [args...]   e.g. tools/start_hidden.sh "C:\Users\Sajjad\Downloads" --theme light --size 1180x760
cd "$(dirname "$0")/.."
EXE="$(cygpath -w "$PWD/target/release/RustFileExplorer.exe")"
export WFILES_TEST=1
export WFM_PIPE="${WFM_PIPE:-wintools-files-test}"
export WFM_DATA_DIR="${WFM_DATA_DIR:-$(cygpath -w "$TEMP")\wfiles-test-data}"
mkdir -p "$(cygpath -u "$WFM_DATA_DIR")"
ARGS=""
for a in "$@"; do ARGS="$ARGS \"$a\""; done
powershell -NoProfile -ExecutionPolicy Bypass -File tools/hidden_start.ps1 -Exe "$EXE" -ArgLine "$ARGS"
for i in $(seq 1 60); do target/release/wfm.exe gui state >/dev/null 2>&1 && echo "ready" && exit 0; sleep 0.1; done
echo "window did not answer"; exit 1
