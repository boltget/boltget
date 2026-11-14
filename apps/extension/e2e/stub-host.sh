#!/bin/sh
exec node "$(dirname "$0")/stub-host.mjs" "$@"
