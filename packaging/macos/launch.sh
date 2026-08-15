#!/bin/sh
if [ "$#" -eq 0 ]; then
  set -- "$HOME"
fi
exec "@binary@" "$@"
