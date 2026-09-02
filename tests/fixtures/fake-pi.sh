#!/bin/sh
set -eu
case_name=$1

if [ "$case_name" = "project-directory" ]; then
  printf '%s' "$PWD" > "$PWD/process-project"
  previous=''
  for argument in "$@"; do
    if [ "$previous" = '--mcp-config' ]; then
      cat "$argument" > "$PWD/process-mcp-config"
      break
    fi
    previous=$argument
  done
fi

if [ "$case_name" = "ignore-term" ]; then
  trap '' TERM
fi
if [ "$case_name" = "peer-delivery" ]; then
  : > "$PWD/fake-session.jsonl"
  : > "$PWD/peer-delivery.log"
fi
if [ "$case_name" = "term-marker" ]; then
  marker=$2
  on_term() {
    printf 'terminated\n' > "$marker"
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
if [ "$case_name" = "peer-delivery" ] || [ "$case_name" = "deferred-session" ]; then
  printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","sessionFile":"%s/fake-session.jsonl","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}\n' "$id" "$PWD"
else
  printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}\n' "$id"
fi
model_changed=0
entries_loaded=0

while IFS= read -r line; do
  if [ "$case_name" = "peer-delivery" ]; then
    printf '%s\n' "$line" >> "$PWD/peer-delivery.log"
  fi
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
  case "$type" in
    get_messages)
      data='{"messages":[]}'
      ;;
    get_entries)
      if [ "$case_name" = "history-control" ]; then
        entries_loaded=1
        data='{"entries":[{"type":"message","id":"one","parentId":null,"message":{"role":"user","content":"preserved history"}}],"leafId":"one"}'
      else
        data='{"entries":[],"leafId":null}'
      fi
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
      if [ "$case_name" = "history-control" ] && [ "$model_changed" -eq 1 ]; then
        data='{"model":{"id":"new-model","name":"New Model","provider":"new-provider","reasoning":true},"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":1,"pendingMessageCount":0}'
      else
        data='{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}'
      fi
      ;;
    set_model)
      if [ "$case_name" = "history-control" ]; then
        [ "$entries_loaded" -eq 1 ] || exit 9
        case "$line" in
          *'"provider":"new-provider"'*'"modelId":"new-model"'*) ;;
          *) exit 10 ;;
        esac
      fi
      model_changed=1
      data='{"id":"new-model","name":"New Model","provider":"new-provider","reasoning":true}'
      ;;
    prompt)
      if [ "$case_name" = "peer-delivery" ]; then
        printf '{"type":"agent_start"}\n'
      fi
      data='{}'
      ;;
    *)
      data='{}'
      ;;
  esac
  printf '{"type":"response","id":"%s","command":"%s","success":true,"data":%s}\n' "$id" "$type" "$data"
done
