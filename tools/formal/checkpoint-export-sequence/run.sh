#!/usr/bin/env bash
# run.sh — one-command rerun of the CheckpointExport TLC check.
# Sync SHA: fe2d41b3dc1a1b5f340938bf8ffbee0fd5daf403
#
# Fails closed: if the pinned TLA+ tools jar is absent or its SHA-256
# differs from the pin recorded at provisioning, this script exits 1 with
# a named error and TLC never runs.
set -euo pipefail

DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
JAR="${TLA_JAR:-$HOME/opt/tla-v1.8.0/tla2tools.jar}"
EXPECTED_SHA256="78f7e2c6bf9a8b65eda665a23e94431255587e0b43a834a4e2af1723562d9b4a"
JAVA="${JAVA:-$HOME/opt/jdk-25.0.4.1+1/bin/java}"
[ -x "$JAVA" ] || JAVA="java"

if [[ ! -f "$JAR" ]]; then
  echo "ERROR(tla-pin-missing): pinned TLA+ tools jar not found at $JAR" >&2
  echo "Provision TLA+ tools v1.8.0 (https://github.com/tlaplus/tlaplus/releases/tag/v1.8.0) there; see README.md." >&2
  exit 1
fi

ACTUAL_SHA256="$(sha256sum "$JAR" | awk '{print $1}')"
if [[ "$ACTUAL_SHA256" != "$EXPECTED_SHA256" ]]; then
  echo "ERROR(tla-pin-mismatch): $JAR sha256=$ACTUAL_SHA256, expected $EXPECTED_SHA256" >&2
  exit 1
fi

cd "$DIR"
exec "$JAVA" -cp "$JAR" tlc2.TLC -config CheckpointExport.cfg -workers 1 CheckpointExport.tla
