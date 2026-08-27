#!/usr/bin/env bash
# Hardwire end-to-end test driver.
#
# Boots the real server on a throwaway database (inside the gitignored
# .sqlx-test/ directory), waits for it to be healthy, runs tests/e2e/e2e.py
# against it, then shuts the server down and removes the scratch directory.
#
# Usage:
#   ./tests/e2e/run.sh
#
# Tunables (optional):
#   HARDWIRE_E2E_PORT    port to run the e2e server on (default 18093)
#   HARDWIRE_E2E_SECRET  JWT secret shared by server and test client (default: fixed test secret)
#
# The server binary must already be built: `cargo build` or `cargo build --release`
# (release is preferred when present). Python 3 (standard library only) is required.
set -uo pipefail
cd "$(dirname "$0")/../.."

PORT="${HARDWIRE_E2E_PORT:-18093}"
SECRET="${HARDWIRE_E2E_SECRET:-e2e-test-secret-0123456789-abcdef-0123456789}"

BIN=./target/release/hardwire
[ -x "$BIN" ] || BIN=./target/debug/hardwire
[ -x "$BIN" ] || { echo "server binary not found — run: cargo build (or cargo build --release)" >&2; exit 1; }
command -v python3 >/dev/null || { echo "python3 is required for tests/e2e/e2e.py" >&2; exit 1; }

mkdir -p .sqlx-test
WORK="$(mktemp -d .sqlx-test/e2e-XXXXXX)"
SERVER_PID=""
cleanup() {
    [ -n "$SERVER_PID" ] && kill "$SERVER_PID" 2>/dev/null
    [ -n "$SERVER_PID" ] && wait "$SERVER_PID" 2>/dev/null
    rm -rf "$WORK"
}
trap cleanup EXIT

echo "[e2e] server: $BIN"
echo "[e2e] port:   $PORT"
echo "[e2e] work:   $WORK"

mkdir -p "$WORK/data"
printf 'hello hardwire e2e\n' > "$WORK/data/file-a.txt"
head -c 1048576 /dev/urandom > "$WORK/data/file-b.bin"

HARDWIRE_PORT="$PORT" \
JWT_SECRET="$SECRET" \
GOOGLE_CLIENT_ID=e2e \
GOOGLE_CLIENT_SECRET=e2e \
HARDWIRE_DATA_DIR="$WORK/data" \
HARDWIRE_DB_PATH="$WORK/db.sqlite" \
HARDWIRE_FILE_INDEXER_INTERVAL=2 \
    "$BIN" -s > "$WORK/server.log" 2>&1 &
SERVER_PID=$!

# wait for the server (fresh-DB bootstrap + migrations on first start)
ok=""
for _ in $(seq 1 120); do
    if curl -sf "http://localhost:$PORT/healthcheck" >/dev/null 2>&1; then
        ok=1
        break
    fi
    kill -0 "$SERVER_PID" 2>/dev/null || break
    sleep 0.5
done
if [ -z "$ok" ]; then
    echo "[e2e] server failed to start — log follows:" >&2
    cat "$WORK/server.log" >&2
    exit 1
fi

BASE_URL="http://localhost:$PORT" \
JWT_SECRET="$SECRET" \
E2E_DATA_DIR="$WORK/data" \
    python3 tests/e2e/e2e.py
rc=$?

echo "[e2e] server log kept? no (scratch dir removed). Last lines before cleanup:"
tail -5 "$WORK/server.log" 2>/dev/null
exit "$rc"