#!/bin/sh
while IFS= read -r request; do
    id=$(printf '%s\n' "$request" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
    [ -n "$id" ] || continue
    case "$request" in
        *'"method":"initialize"'*)
            result=$(printf '{"userAgent":"fixture","codexHome":"%s","platformFamily":"unix","platformOs":"macos"}' "$CODEX_HOME") ;;
        *'"method":"thread/loaded/list"'*) result='{"data":[]}' ;;
        *'"method":"config/read"'*) result='{"config":{}}' ;;
        *'"method":"thread/turns/list"'*) result='{"data":[{"items":[{"type":"agentMessage","text":"Recent answer"}]}]}' ;;
        *'"method":"thread/read"'*)
            attempts=0
            while [ ! -f "$CODEX_HOME/release-full-read" ] && [ "$attempts" -lt 300 ]; do
                sleep 0.05
                attempts=$((attempts + 1))
            done
            result='{"thread":{"turns":[{"items":[{"type":"userMessage","content":[{"type":"text","text":"Original question"}]},{"type":"agentMessage","text":"Recent answer"}]}]}}' ;;
        *) result='{}' ;;
    esac
    printf '{"id":%s,"result":%s}\n' "$id" "$result"
done
