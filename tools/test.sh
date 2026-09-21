#!/bin/sh
# Validate the checkout's engine, independent of the caller's installed binary.
set -eu

for selector in CCC_MRD_BIN MERIDIAN_MRD_BIN MERIDIAN_DAEMON_BIN MRD_BIN; do
    if printenv "$selector" >/dev/null; then
        printf 'test environment: clearing inherited %s\n' "$selector" >&2
    fi
    unset "$selector"
done

exec cargo test "$@"
