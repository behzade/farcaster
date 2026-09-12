#!/bin/sh
set -eu

printf '%s\n' "$@" > "$PWD/worker-resume-arguments"

session_file=''
previous=''
for argument in "$@"; do
  if [ "$previous" = '--session' ]; then
    session_file=$argument
    break
  fi
  previous=$argument
done

if [ -z "$session_file" ]; then
  printf 'resume did not supply --session\n' >&2
  exit 2
fi

cat "$session_file" > "$PWD/worker-resume-history"
escaped_session=$(printf '%s' "$session_file" | sed 's/\\/\\\\/g; s/"/\\"/g')

read_id() {
  printf '%s' "$1" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p'
}

read_type() {
  printf '%s' "$1" | sed -n 's/.*"type":"\([^"]*\)".*/\1/p'
}

while IFS= read -r line; do
  printf '%s\n' "$line" >> "$PWD/worker-resume-requests"
  id=$(read_id "$line")
  type=$(read_type "$line")
  case "$type" in
    get_state)
      data=$(printf '{"model":null,"thinkingLevel":"off","isStreaming":false,"isCompacting":false,"sessionId":"resumed-worker","sessionFile":"%s","autoCompactionEnabled":true,"messageCount":1,"pendingMessageCount":0}' "$escaped_session")
      ;;
    get_commands)
      data='{"commands":[]}'
      ;;
    set_steering_mode)
      data='{}'
      ;;
    *)
      data='{}'
      ;;
  esac
  printf '{"type":"response","id":"%s","command":"%s","success":true,"data":%s}\n' "$id" "$type" "$data"
done
