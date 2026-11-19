#!/bin/bash
# Keys go to the focused text field. Types "a b" through real WM_KEYDOWN / WM_CHAR messages into
#   1. the search box   -> query is "a b", nothing selected, no overlay (Space must not preview / select)
#   2. the address bar  -> text is "a b" (Space, letters never reach the list)
#   3. inline rename    -> name field is "a b.txt"; Esc cancels, the file keeps its name
# Runs on the hidden desktop with the test pipe, in a scratch folder in %TEMP%; exits 0 on success.
cd "$(dirname "$0")/.."
export WFM_PIPE="${WFM_PIPE:-wintools-files-test}"
W=target/release/wfm.exe
D="$(cygpath -u "$TEMP")/rfe-keys-test"
rm -rf "$D"; mkdir -p "$D"
for n in alpha beta gamma; do echo x > "$D/$n.txt"; done
tools/start_hidden.sh "$(cygpath -w "$D")" >/dev/null || { echo "FAIL: window did not start"; exit 1; }
fail() { echo "FAIL: $1"; $W gui quit >/dev/null; rm -rf "$D"; exit 1; }
# string fields only ("text": 1.0 in "surfaces" is a number)
field() { echo "$1" | grep -o "\"$2\": \"[^\"]*\"\|\"$2\": null" | head -1; }

# 1. search box
$W gui key ctrl+f >/dev/null
$W gui keys "a b" >/dev/null
sleep 0.6
ST="$($W gui state)"
echo "search: $(field "$ST" text) | $(field "$ST" overlay) | $(echo "$ST" | grep -o '"selected": \[[^]]*\]')"
[ "$(field "$ST" text)" = '"text": "a b"' ] || fail "query is not \"a b\""
[ "$(field "$ST" overlay)" = '"overlay": ""' ] || fail "an overlay opened"
echo "$ST" | grep -q '"selected": \[\]' || fail "the keys selected something"
$W gui key esc >/dev/null

# 2. address bar
$W gui key ctrl+l >/dev/null
$W gui key ctrl+a >/dev/null
$W gui keys "a b" >/dev/null
ST="$($W gui state)"
echo "address bar: $(field "$ST" path_edit)"
[ "$(field "$ST" path_edit)" = '"path_edit": "a b"' ] || fail "address bar text is not \"a b\""
echo "$ST" | grep -q '"selected": \[\]' || fail "the keys selected something"
$W gui key esc >/dev/null

# 3. inline rename (Esc: nothing is renamed)
$W gui select "beta.txt" >/dev/null
$W gui key f2 >/dev/null
$W gui keys "a b" >/dev/null
ST="$($W gui state)"
echo "rename: $(field "$ST" inline_rename)"
[ "$(field "$ST" inline_rename)" = '"inline_rename": "a b.txt"' ] || fail "rename text is not \"a b.txt\""
$W gui key esc >/dev/null
[ -f "$D/beta.txt" ] || fail "the file was renamed"

$W gui quit >/dev/null
rm -rf "$D"
echo "PASS"
