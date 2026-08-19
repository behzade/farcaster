#!/bin/sh
set -eu
case_name=$1

if [ "$case_name" = "direnv" ]; then
  test "${PI_GUI_DIRENV_MARKER:-}" = "loaded"
fi

if [ "$case_name" = "ignore-term" ]; then
  trap '' TERM
fi
if [ "$case_name" = "term-marker" ]; then
  marker=$2
  on_term() {
    printf 'terminated\n' >> "$marker"
    exit 0
  }
  trap on_term TERM
fi

read_id() {
  printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'
}
read_type() {
  printf '%s' "$1" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p'
}

IFS= read -r line || exit 2
id=$(read_id "$line")
if [ "$case_name" = "bad-handshake" ]; then
  printf '{"type":"response","id":"%s","command":"get_state","success":false,"error":"not ready"}\n' "$id"
  exit 3
fi
if [ "$case_name" = "mismatch-handshake" ]; then
  printf '{"type":"response","id":"%s","command":"get_messages","success":true,"data":{}}\n' "$id"
  exit 4
fi
if [ "$case_name" = "normal" ]; then
  printf '{"type":"agent_start"}\n'
fi
printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}\n' "$id"

while IFS= read -r line; do
  id=$(read_id "$line")
  type=$(read_type "$line")
  if [ "$case_name" = "eof" ]; then
    printf 'fake stderr before exit\n' >&2
    exit 7
  fi
  if [ "$case_name" = "delayed-stderr" ]; then
    (sleep 0.1; printf 'delayed final stderr\n' >&2) &
    exec 1>&-
    wait
    exit 8
  fi
  if [ "$case_name" = "mismatch-response" ]; then
    printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{}}\n' "$id"
    continue
  fi
  if [ "$case_name" = "term-marker" ] && [ "$type" = "abort" ]; then
    printf 'aborted\n' >> "$marker"
  fi
  case "$type" in
    get_messages)
      data='{"messages":[]}'
      ;;
    get_available_models)
      data='{"models":[]}'
      ;;
    get_available_thinking_levels)
      data='{"levels":["off"]}'
      ;;
    get_session_stats)
      data='{"contextUsage":{"tokens":4096,"contextWindow":8192,"percent":50}}'
      ;;
    get_state)
      data='{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}'
      ;;
    *)
      data='{}'
      ;;
  esac
  printf '{"type":"response","id":"%s","command":"%s","success":true,"data":%s}\n' "$id" "$type" "$data"
done
