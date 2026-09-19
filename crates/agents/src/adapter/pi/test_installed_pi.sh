#!/bin/sh
set -eu
export PI_CODING_AGENT_DIR="$PWD/pi-agent"
export FARCASTER_PI_FIXTURE_LOG="$PWD/fixture-requests"
exec "$@"
