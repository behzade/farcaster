#!/bin/sh
set -eu
case_name=$1
case "$case_name" in
  sandbox-*|history-control|worker-launch-config)
    mkdir -p "$PWD/sandbox-extension"
    printf '{"name":"pi-nono"}\n' > "$PWD/sandbox-extension/package.json"
    : > "$PWD/sandbox-extension/index.js"
    if [ "$case_name" = "sandbox-unrelated" ]; then
      printf '{"name":"another-sandbox"}\n' > "$PWD/sandbox-extension/package.json"
    fi
    ;;
esac
if [ "$case_name" = "worker-launch-config" ]; then
  printf '%s\n%s\n' "${http_proxy-}" "${https_proxy-}" > "$PWD/worker-launch-proxy"
fi
session_file=''
previous=''
for argument in "$@"; do
  if [ "$previous" = '--session' ]; then
    session_file=$(printf '%s' "$argument" | sed 's/\\/\\\\/g; s/"/\\"/g')
    break
  fi
  previous=$argument
done

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
# Deliberately ignore --session so readiness tests can detect a wrong resume.
if [ "$case_name" = "fixed-session" ]; then
  session_file="$PWD/fake-session.jsonl"
fi
if [ -z "$session_file" ]; then
  case "$case_name" in
    peer-delivery|deferred-session|project-directory) session_file="$PWD/fake-session.jsonl" ;;
  esac
fi
if [ -n "$session_file" ]; then
  printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","sessionFile":"%s","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}\n' "$id" "$session_file"
else
  printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}\n' "$id"
fi
model_changed=0
entries_loaded=0

while IFS= read -r line; do
  if [ "$case_name" = "cancelled-fork" ]; then
    printf '%s\n' "$line" >> "$PWD/fork-requests"
  fi
  if [ "$case_name" = "peer-delivery" ]; then
    printf '%s\n' "$line" >> "$PWD/peer-delivery.log"
  fi
  id=$(read_id "$line")
  type=$(read_type "$line")
  if [ "$case_name" = "eof" ] && [ "$type" != "get_commands" ]; then
    printf 'fake stderr before exit\n' >&2
    exit 7
  fi
  if [ "$case_name" = "delayed-stderr" ] && [ "$type" != "get_commands" ]; then
    (sleep 0.1; printf 'delayed final stderr\n' >&2) &
    exec 1>&-
    wait
    exit 8
  fi
  if [ "$case_name" = "mismatch-response" ] && [ "$type" != "get_commands" ]; then
    printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{}}\n' "$id"
    continue
  fi
  case "$type" in
    fork)
      if [ "$case_name" = "cancelled-fork" ]; then
        data='{"text":null,"cancelled":true}'
      else
        data='{"text":null,"cancelled":false}'
      fi
      ;;
    get_commands)
      case "$case_name" in
        sandbox-missing) data='{"commands":[]}' ;;
        sandbox-template) data='{"commands":[{"name":"sandbox-mode","source":"prompt"}]}' ;;
        sandbox-no-source) data='{"commands":[{"name":"sandbox-mode","source":"extension"}]}' ;;
        sandbox-*|history-control|worker-launch-config)
          control_name=sandbox-mode
          if [ "$case_name" = "sandbox-collision" ]; then control_name=sandbox-mode:2; fi
          source_path=$(printf '%s' "$PWD/sandbox-extension/index.js" | sed 's/\\/\\\\/g; s/"/\\"/g')
          data=$(printf '{"commands":[{"name":"%s","source":"extension","sourceInfo":{"path":"%s"}}]}' "$control_name" "$source_path")
          ;;
        *) data='{"commands":[]}' ;;
      esac
      ;;
    prompt)
      if [ "$case_name" = "peer-delivery" ]; then
        printf '{"type":"agent_start"}\n'
      fi
      case "$line" in
        *'/sandbox-mode '*|*'/sandbox-mode:2 '*)
          printf '%s\n' "$line" >> "$PWD/sandbox-controls"
          request_id=$(printf '%s' "$line" | sed -n 's/.*\\"requestId\\":\\"\([^\\]*\)\\".*/\1/p')
          mode=$(printf '%s' "$line" | sed -n 's/.*\\"files\\":\\"\([^\\]*\)\\".*/\1/p')
          success=true
          case "$case_name" in
            sandbox-failed) success=false ;;
            sandbox-stale) request_id=old ;;
            sandbox-wrong-mode) mode=full ;;
            sandbox-rejected)
              printf '{"type":"response","id":"%s","command":"prompt","success":false,"error":"control rejected"}\n' "$id"
              continue ;;
          esac
          printf '{"type":"extension_ui_request","method":"setStatus","id":"sandbox","statusKey":"\\u001fpi-gpui-sandbox-mode\\u001f","statusText":"{\\"version\\":1,\\"requestId\\":\\"%s\\",\\"files\\":\\"%s\\",\\"network\\":\\"%s\\",\\"success\\":%s}"}\n' "$request_id" "$mode" "$mode" "$success"
          ;;
        *) printf '%s\n' "$line" >> "$PWD/agent-prompts" ;;
      esac
      data='{}'
      ;;
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
      if [ "$case_name" = "malformed-catalog" ]; then
        data='{"models":[{"id":"valid","name":"Valid","provider":"test"},{"id":"incomplete"}]}'
      else
        data='{"models":[]}'
      fi
      ;;
    get_available_thinking_levels)
      data='{"levels":["off"]}'
      ;;
    get_session_stats)
      data='{"tokens":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0},"contextUsage":{"tokens":4096,"contextWindow":8192,"percent":50}}'
      ;;
    get_state)
      if [ "$case_name" = "history-control" ] && [ "$model_changed" -eq 1 ]; then
        data='{"model":{"id":"new-model","name":"New Model","provider":"new-provider","reasoning":true},"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":1,"pendingMessageCount":0}'
      else
        data='{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"fake","autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}'
      fi
      if [ -n "$session_file" ]; then
        data=$(printf '{"sessionFile":"%s",%s' "$session_file" "${data#\{}")
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
    *)
      data='{}'
      ;;
  esac
  printf '{"type":"response","id":"%s","command":"%s","success":true,"data":%s}\n' "$id" "$type" "$data"
done
