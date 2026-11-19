#!/bin/bash
# Start RustTaskManager.exe on the hidden desktop and wait until `wtm gui` answers.
cd "$(dirname "$0")/.."
EXE="${1:-$PWD/app/target/release/RustTaskManager.exe}"; shift
powershell -NoProfile -ExecutionPolicy Bypass -File tools/hidden_start.ps1 -Exe "$(cygpath -w "$EXE")" -ArgLine "$*" >/dev/null
for i in $(seq 1 50); do app/target/release/wtm.exe gui state 2>/dev/null && exit 0; sleep 0.1; done
echo "window did not answer"; exit 1
