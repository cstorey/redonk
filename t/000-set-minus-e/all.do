rm -f log
(redo fatal  || true) > /dev/null 2>&1

[ "$(cat log)" = "ok" ] || exit 5
