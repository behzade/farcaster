#!/bin/sh
set -eu
case_name=$1
shift

session_file="$PWD/session.jsonl"
previous=''
for argument in "$@"; do
  if [ "$previous" = '--session' ]; then session_file=$argument; fi
  previous=$argument
done
touch "$session_file"
printf '%s\n' "$session_file" >> "$PWD/fixture-sessions"
printf '%s\n' "$$" >> "$PWD/fixture-pids"
launch_count=1
if [ -f "$PWD/fixture-launch-count" ]; then
  launch_count=$(( $(cat "$PWD/fixture-launch-count") + 1 ))
fi
printf '%s' "$launch_count" > "$PWD/fixture-launch-count"
test -f "$PWD/fixture-thinking" || printf 'off' > "$PWD/fixture-thinking"
steering_queue="$PWD/fixture-steering-queue.$$"
follow_up_queue="$PWD/fixture-follow-up-queue.$$"
: > "$steering_queue"
: > "$follow_up_queue"

read_id() {
  printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'
}

read_type() {
  for candidate in get_state get_commands prompt steer follow_up abort set_model set_thinking_level set_steering_mode set_follow_up_mode get_entries; do
    case "$1" in
      *\"type\":\"$candidate\"*) printf '%s' "$candidate"; return ;;
    esac
  done
  printf 'unknown'
}

emit_queue_update() {
  steering=$(awk 'BEGIN { separator = "" } { printf "%s\"queued\"", separator; separator = "," }' "$steering_queue")
  follow_up=$(awk 'BEGIN { separator = "" } { printf "%s\"queued\"", separator; separator = "," }' "$follow_up_queue")
  printf '{"type":"queue_update","steering":[%s],"followUp":[%s]}\n' "$steering" "$follow_up"
}

while IFS= read -r line; do
  printf '%s\n' "$line" >> "$PWD/fixture-rpc-lines"
  id=$(read_id "$line")
  type=$(read_type "$line")
  case "$type" in
    get_state)
      if [ "$case_name" = 'second-readiness-fails' ] && [ "$launch_count" -eq 2 ]; then
        printf '{"type":"response","id":"%s","command":"get_state","success":false,"error":"second readiness failed"}\n' "$id"
        exit 12
      fi
      model=null
      test ! -f "$PWD/fixture-model" || model=$(cat "$PWD/fixture-model")
      thinking=$(cat "$PWD/fixture-thinking")
      if [ "$case_name" = 'replacement-defaults' ] && [ "$launch_count" -eq 2 ] && [ ! -f "$PWD/fixture-replacement-default-reported" ]; then
        model='{"id":"replacement-default","name":"Replacement default","provider":"fixture","contextWindow":8192,"reasoning":true}'
        thinking='low'
        touch "$PWD/fixture-replacement-default-reported"
      fi
      if [ "$case_name" = 'missing-locator' ]; then
        session_field=''
      else
        session_field=$(printf ',"sessionFile":"%s"' "$session_file")
      fi
      printf '{"type":"response","id":"%s","command":"get_state","success":true,"data":{"model":%s,"thinkingLevel":"%s","isStreaming":false,"isCompacting":false,"sessionId":"fixture"%s,"autoCompactionEnabled":true,"messageCount":0,"pendingMessageCount":0}}\n' "$id" "$model" "$thinking" "$session_field"
      ;;
    get_commands)
      printf '{"type":"response","id":"%s","command":"get_commands","success":true,"data":{"commands":[]}}\n' "$id"
      ;;
    prompt)
      message=$(printf '%s' "$line" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')
      printf '%s\n' "$message" >> "$PWD/fixture-requests"
      case "$message" in
        unconfirmed*) continue ;;
        exit-before-ack*) exit 21 ;;
        exit-after-ack*)
          printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
          exit 22
          ;;
        reject*)
          printf '{"type":"response","id":"%s","command":"prompt","success":false,"error":"prompt rejected"}\n' "$id"
          continue
          ;;
      esac
      if [ "$case_name" = 'history' ]; then
        printf '{"type":"message","id":"user-%s","parentId":null,"message":{"role":"user","content":"%s"}}\n' "$launch_count" "$message" >> "$session_file"
      else
        printf '%s\n' "$message" >> "$session_file"
      fi
      image=''
      case "$line" in
        *'"type":"image"'*)
          data=$(printf '%s' "$line" | sed -n 's/.*"data":"\([^"]*\)".*/\1/p')
          mime_type=$(printf '%s' "$line" | sed -n 's/.*"mimeType":"\([^"]*\)".*/\1/p')
          image=$(printf ',{"type":"image","data":"%s","mimeType":"%s"}' "$data" "$mime_type")
          ;;
      esac
      event_message=$message
      case "$message" in
        event-first-transform*)
          event_message='extension transformed'
          image=',{"type":"image","data":"dHJhbnNmb3JtZWQ=","mimeType":"image/webp"}'
          ;;
      esac
      case "$message" in
        event-first*) ;;
        *) printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id" ;;
      esac
      printf '{"type":"agent_start"}\n'
      printf '{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"%s"}%s]}}\n' "$event_message" "$image"
      printf '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"%s"}%s]}}\n' "$event_message" "$image"
      case "$message" in
        event-first*) printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id" ;;
      esac
      case "$message" in
        hold*) ;;
        *) printf '{"type":"agent_settled"}\n' ;;
      esac
      ;;
    steer)
      message=$(printf '%s' "$line" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')
      case "$message" in
        reject*)
          printf '{"type":"response","id":"%s","command":"steer","success":false,"error":"steer rejected"}\n' "$id"
          continue
          ;;
      esac
      printf '%s\n' "$message" >> "$steering_queue"
      printf 'steer:%s\n' "$message" >> "$PWD/fixture-admissions"
      emit_queue_update
      printf '{"type":"response","id":"%s","command":"steer","success":true}\n' "$id"
      ;;
    follow_up)
      message=$(printf '%s' "$line" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')
      printf '%s\n' "$message" >> "$follow_up_queue"
      printf 'follow_up:%s\n' "$message" >> "$PWD/fixture-admissions"
      emit_queue_update
      printf '{"type":"response","id":"%s","command":"follow_up","success":true}\n' "$id"
      ;;
    abort)
      if [ "$case_name" = 'slow-handoff' ]; then sleep 1; fi
      printf '{"type":"response","id":"%s","command":"abort","success":true}\n' "$id"
      while [ -s "$steering_queue" ]; do
        delivering=$(sed -n '1p' "$steering_queue")
        printf '%s\n' "$delivering" >> "$PWD/fixture-requests"
        printf '%s\n' "$delivering" >> "$session_file"
        sed '1d' "$steering_queue" > "$steering_queue.next"
        mv "$steering_queue.next" "$steering_queue"
        emit_queue_update
        printf '{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"%s"}]}}\n' "$delivering"
        printf '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"%s"}]}}\n' "$delivering"
        printf '{"type":"agent_start"}\n'
      done
      while [ -s "$follow_up_queue" ]; do
        delivering=$(sed -n '1p' "$follow_up_queue")
        printf '%s\n' "$delivering" >> "$PWD/fixture-requests"
        printf '%s\n' "$delivering" >> "$session_file"
        sed '1d' "$follow_up_queue" > "$follow_up_queue.next"
        mv "$follow_up_queue.next" "$follow_up_queue"
        emit_queue_update
        printf '{"type":"message_start","message":{"role":"user","content":[{"type":"text","text":"%s"}]}}\n' "$delivering"
        printf '{"type":"message_end","message":{"role":"user","content":[{"type":"text","text":"%s"}]}}\n' "$delivering"
        printf '{"type":"agent_start"}\n'
      done
      printf '{"type":"agent_settled"}\n'
      ;;
    set_model)
      provider=$(printf '%s' "$line" | sed -n 's/.*"provider":"\([^"]*\)".*/\1/p')
      model_id=$(printf '%s' "$line" | sed -n 's/.*"modelId":"\([^"]*\)".*/\1/p')
      if [ "$model_id" = 'rejected-model' ]; then
        printf '{"type":"response","id":"%s","command":"set_model","success":false,"error":"model rejected"}\n' "$id"
        continue
      fi
      printf '{"id":"%s","name":"Fixture","provider":"%s","contextWindow":8192,"reasoning":true}' "$model_id" "$provider" > "$PWD/fixture-model"
      printf '{"type":"response","id":"%s","command":"set_model","success":true,"data":%s}\n' "$id" "$(cat "$PWD/fixture-model")"
      ;;
    set_thinking_level)
      level=$(printf '%s' "$line" | sed -n 's/.*"level":"\([^"]*\)".*/\1/p')
      if [ "$level" = 'rejected' ]; then
        printf '{"type":"response","id":"%s","command":"set_thinking_level","success":false,"error":"reasoning rejected"}\n' "$id"
        continue
      fi
      printf '%s' "$level" > "$PWD/fixture-thinking"
      printf '{"type":"response","id":"%s","command":"set_thinking_level","success":true}\n' "$id"
      ;;
    set_steering_mode)
      printf 'steering:all\n' >> "$PWD/fixture-steering-configurations"
      printf '{"type":"response","id":"%s","command":"set_steering_mode","success":true}\n' "$id"
      ;;
    set_follow_up_mode)
      printf 'follow_up:all\n' >> "$PWD/fixture-steering-configurations"
      printf '{"type":"response","id":"%s","command":"set_follow_up_mode","success":true}\n' "$id"
      ;;
    get_entries)
      if [ "$case_name" = 'history' ]; then
        entries=$(paste -sd, "$session_file")
        printf '{"type":"response","id":"%s","command":"get_entries","success":true,"data":{"entries":[%s],"leafId":null}}\n' "$id" "$entries"
      else
        printf '{"type":"response","id":"%s","command":"get_entries","success":true,"data":{"entries":[],"leafId":null}}\n' "$id"
      fi
      ;;
    *)
      printf '{"type":"response","id":"%s","command":"%s","success":true,"data":{}}\n' "$id" "$type"
      ;;
  esac
done
