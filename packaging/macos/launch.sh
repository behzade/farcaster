#!/bin/sh
if [ "$#" -eq 0 ]; then
  set -- "$HOME"
fi
export PI_GUI_IMPORT_SHELL_ENV=1
exec "@binary@" "$@"
