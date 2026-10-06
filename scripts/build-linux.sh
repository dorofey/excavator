#!/bin/sh
set -eu
cd "$(dirname "$0")/.."
profile="${EXCAVATOR_BUILD_PROFILE:-release}"
case "$profile" in
    release) cargo build --locked --release ;;
    debug) cargo build --locked ;;
    *) printf 'Invalid build profile: %s\n' "$profile" >&2; exit 1 ;;
esac
printf 'Built target/%s/excavator\n' "$profile"
