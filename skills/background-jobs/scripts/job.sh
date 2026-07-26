#!/usr/bin/env bash
set -euo pipefail

socket="${PI_BACKGROUND_TMUX_SOCKET:-/tmp/pi-agent-tmux.sock}"
tmux_command=(tmux -S "$socket")

usage() {
  echo "usage: job.sh start NAME CWD COMMAND" >&2
  echo "       job.sh list" >&2
  echo "       job.sh status NAME" >&2
  echo "       job.sh read NAME [LINES]" >&2
  echo "       job.sh write NAME TEXT" >&2
  echo "       job.sh line NAME TEXT" >&2
  echo "       job.sh keys NAME KEY..." >&2
  echo "       job.sh stop NAME" >&2
  exit 2
}

valid_name() {
  [[ "$1" =~ ^pi-[A-Za-z0-9._-]{1,60}$ ]]
}

require_name() {
  valid_name "$1" || {
    echo "job name must start with pi- and use only letters, digits, dots, underscores, or hyphens" >&2
    exit 2
  }
}

has_server() {
  "${tmux_command[@]}" list-sessions >/dev/null 2>&1
}

cleanup_stale_socket() {
  if [[ -e "$socket" ]] && ! has_server; then
    rm -f -- "$socket"
  fi
}

has_session() {
  "${tmux_command[@]}" has-session -t "=$1" 2>/dev/null
}

paste_text() {
  local name="$1"
  local text="$2"
  local buffer="job-${name}"
  printf '%s' "$text" | "${tmux_command[@]}" load-buffer -b "$buffer" -
  "${tmux_command[@]}" paste-buffer -b "$buffer" -d -t "=${name}:0.0"
}

action="${1:-}"
case "$action" in
  start)
    [[ $# -eq 4 ]] || usage
    name="$2"
    cwd="$3"
    command="$4"
    require_name "$name"
    [[ -d "$cwd" ]] || {
      echo "working directory does not exist: $cwd" >&2
      exit 2
    }
    cwd="$(cd "$cwd" && pwd -P)"
    cleanup_stale_socket
    if has_session "$name"; then
      echo "job already exists: $name" >&2
      exit 1
    fi
    "${tmux_command[@]}" new-session -d -s "$name" -c "$cwd"
    "${tmux_command[@]}" set-option -g history-limit 100000
    "${tmux_command[@]}" set-option -p -t "=${name}:0.0" remain-on-exit on
    "${tmux_command[@]}" respawn-pane -k -t "=${name}:0.0" -- /bin/sh -lc "$command"
    echo "started $name"
    ;;
  list)
    [[ $# -eq 1 ]] || usage
    if ! has_server; then
      echo "no background jobs"
      exit 0
    fi
    "${tmux_command[@]}" list-sessions -F 'name=#{session_name} windows=#{session_windows} created=#{session_created_string}'
    ;;
  status)
    [[ $# -eq 2 ]] || usage
    name="$2"
    require_name "$name"
    has_session "$name" || {
      echo "unknown job: $name" >&2
      exit 1
    }
    "${tmux_command[@]}" list-panes -t "=$name" \
      -F 'name=#{session_name} dead=#{pane_dead} exit=#{pane_dead_status} pid=#{pane_pid} command=#{pane_current_command}'
    ;;
  read)
    [[ $# -eq 2 || $# -eq 3 ]] || usage
    name="$2"
    lines="${3:-200}"
    require_name "$name"
    [[ "$lines" =~ ^[0-9]+$ ]] && (( lines >= 1 && lines <= 10000 )) || {
      echo "lines must be between 1 and 10000" >&2
      exit 2
    }
    has_session "$name" || {
      echo "unknown job: $name" >&2
      exit 1
    }
    "${tmux_command[@]}" capture-pane -p -J -t "=${name}:0.0" -S "-$lines"
    ;;
  write | line)
    [[ $# -eq 3 ]] || usage
    name="$2"
    require_name "$name"
    has_session "$name" || {
      echo "unknown job: $name" >&2
      exit 1
    }
    paste_text "$name" "$3"
    if [[ "$action" == "line" ]]; then
      "${tmux_command[@]}" send-keys -t "=${name}:0.0" Enter
    fi
    echo "sent input to $name"
    ;;
  keys)
    (( $# >= 3 )) || usage
    name="$2"
    require_name "$name"
    has_session "$name" || {
      echo "unknown job: $name" >&2
      exit 1
    }
    shift 2
    "${tmux_command[@]}" send-keys -t "=${name}:0.0" -- "$@"
    echo "sent keys to $name"
    ;;
  stop)
    [[ $# -eq 2 ]] || usage
    name="$2"
    require_name "$name"
    has_session "$name" || {
      echo "unknown job: $name" >&2
      exit 1
    }
    "${tmux_command[@]}" kill-session -t "=$name"
    cleanup_stale_socket
    echo "stopped $name"
    ;;
  *)
    usage
    ;;
esac
