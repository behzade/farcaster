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
queued_steering=''
queued_follow_up=''

read_id() {
  printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'
}

read_type() {
  printf '%s' "$1" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p'
}

while IFS= read -r line; do
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
      printf '%s\n' "$message" >> "$session_file"
      printf '{"type":"response","id":"%s","command":"prompt","success":true}\n' "$id"
      printf '{"type":"agent_start"}\n'
      case "$message" in
        hold*) ;;
        *) printf '{"type":"agent_settled"}\n' ;;
      esac
      ;;
    steer)
      queued_steering=$(printf '%s' "$line" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')
      printf 'steer:%s\n' "$queued_steering" >> "$PWD/fixture-admissions"
      printf '{"type":"response","id":"%s","command":"steer","success":true}\n' "$id"
      ;;
    follow_up)
      queued_follow_up=$(printf '%s' "$line" | sed -n 's/.*"message":"\([^"]*\)".*/\1/p')
      printf 'follow_up:%s\n' "$queued_follow_up" >> "$PWD/fixture-admissions"
      printf '{"type":"response","id":"%s","command":"follow_up","success":true}\n' "$id"
      ;;
    abort)
      printf '{"type":"response","id":"%s","command":"abort","success":true}\n' "$id"
      if [ -n "$queued_steering" ]; then
        printf '%s\n' "$queued_steering" >> "$PWD/fixture-requests"
        queued_steering=''
        printf '{"type":"agent_start"}\n'
      fi
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
      printf 'all\n' >> "$PWD/fixture-steering-configurations"
      printf '{"type":"response","id":"%s","command":"set_steering_mode","success":true}\n' "$id"
      ;;
    get_entries)
      printf '{"type":"response","id":"%s","command":"get_entries","success":true,"data":{"entries":[],"leafId":null}}\n' "$id"
      ;;
    *)
      printf '{"type":"response","id":"%s","command":"%s","success":true,"data":{}}\n' "$id" "$type"
      ;;
  esac
done
