#!/usr/bin/env bash
# ──────────────────────────────────────────────────────────────────
# RustRelay Load Test Script
#
# Simulates concurrent WebSocket connections and messaging.
# Requires: websocat (cargo install websocat)
# Usage: ./scripts/load_test.sh [num_clients] [messages_per_client]
# ──────────────────────────────────────────────────────────────────

set -euo pipefail

NUM_CLIENTS=${1:-100}
MSGS_PER_CLIENT=${2:-10}
WS_URL="ws://127.0.0.1:8080/ws"
CHANNEL_ID="20000000-0000-0000-0000-000000000001"
TOKENS=("token_alice" "token_bob" "token_charlie" "token_dave")

echo "═══════════════════════════════════════════════════"
echo "  RustRelay Load Test"
echo "  Clients: $NUM_CLIENTS"
echo "  Messages/client: $MSGS_PER_CLIENT"
echo "═══════════════════════════════════════════════════"

START=$(date +%s%N)

for i in $(seq 1 $NUM_CLIENTS); do
    TOKEN=${TOKENS[$((i % ${#TOKENS[@]}))]}
    (
        {
            echo "$TOKEN"
            sleep 1  # Wait for READY
            for j in $(seq 1 $MSGS_PER_CLIENT); do
                echo "{\"op\":\"send_message\",\"d\":{\"channel_id\":\"$CHANNEL_ID\",\"content\":\"Load test msg $i-$j\"}}"
                sleep 0.05
            done
            sleep 2
        } | websocat "$WS_URL" > /dev/null 2>&1
    ) &

    # Stagger connections slightly
    if (( i % 20 == 0 )); then
        sleep 0.1
    fi
done

wait

END=$(date +%s%N)
ELAPSED=$(( (END - START) / 1000000 ))
TOTAL_MSGS=$((NUM_CLIENTS * MSGS_PER_CLIENT))

echo ""
echo "═══════════════════════════════════════════════════"
echo "  Results"
echo "  Total messages: $TOTAL_MSGS"
echo "  Elapsed: ${ELAPSED}ms"
echo "  Throughput: $(( TOTAL_MSGS * 1000 / ELAPSED )) msg/sec"
echo "═══════════════════════════════════════════════════"

# Check stats endpoint
echo ""
echo "Server stats:"
curl -s http://127.0.0.1:8080/api/stats | python3 -m json.tool 2>/dev/null || \
curl -s http://127.0.0.1:8080/api/stats
echo ""
