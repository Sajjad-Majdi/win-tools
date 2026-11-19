#!/bin/bash
# A long session of about 40 actions on the hidden desktop (test pipe, test data dir), then the window's working set
# and private bytes. Read-only on the owner's files: it only opens folders, menus, Settings, search and previews.
# Usage: tools/long_session.sh            prints "start ws/private" and "after ws/private" in MB
cd "$(dirname "$0")/.."
export WFM_PIPE="${WFM_PIPE:-wintools-files-test}"
W=target/release/wfm.exe
U="$USERPROFILE"
# LONG_REAL=1: on the real desktop, off every monitor (true memory; the hidden desktop adds WARP)
if [ -n "$LONG_REAL" ]; then S=tools/start_offscreen.sh; else S=tools/start_hidden.sh; fi
$S "$U\\Downloads" >/dev/null || { echo "window did not start"; exit 1; }
PID=$(powershell -NoProfile -Command "(Get-CimInstance Win32_Process -Filter \"Name='RustFileExplorer.exe'\" | Where-Object { \$_.CommandLine -like '*Downloads*' } | Sort-Object CreationDate -Descending | Select-Object -First 1).ProcessId")
mem() { powershell -NoProfile -Command "\$p = Get-Process -Id $PID; '{0:N1} {1:N1}' -f (\$p.WorkingSet64/1MB), (\$p.PrivateMemorySize64/1MB)"; }
sleep 3
echo "start (ws private MB): $(mem)"
act() { $W gui "$@" >/dev/null 2>&1; sleep 0.25; }
for d in "$U" "$U\\Documents" "$U\\Desktop" "C:\\Windows\\System32" "$U\\Downloads"; do act open "$d"; sleep 1; done
act view grid; sleep 1; act view details
act select 0; act menu; sleep 1.5; act close-popups
act settings; act wheel 700 400 -10; act wheel 700 400 10
act open "$U\\Downloads"; act search "png"; sleep 1; act clear
act palette "set"; act key esc
for i in 1 2 3; do act tab new "$U\\Documents"; done
for i in 1 2 3; do act tab close; done
act select 2; act preview; sleep 0.5; act preview next; act preview next; act key esc
act open "C:\\Windows"; sleep 1; act wheel 600 400 -20; act wheel 600 400 20
act open "C:\\Program Files"; sleep 1
act select 1; act menu; sleep 1.5; act close-popups
act hover 102 64; sleep 0.7; act hover 600 400
act split on; act split off
act open "$U\\Downloads"; act view list; act view details
sleep 4
echo "after ~40 actions (ws private MB): $(mem)"
$W gui quit >/dev/null 2>&1
