#!/usr/bin/env bash
set -euo pipefail

files=("$1")
whim="$GITHUB_WORKSPACE/target/aarch64-unknown-freebsd/debug/whim"
if [[ -x "$whim" && "$whim" != "$1" ]]; then
  files+=("$whim")
fi

rsync -aR --rsync-path=/usr/local/bin/rsync "${files[@]}" freebsd:/
printf -v freebsd_command " '%s'" "${@//\'/\'\\\'\'}"
printf -v freebsd_directory "'%s'" "${PWD//\'/\'\\\'\'}"
printf 'cd %s && CARGO_MANIFEST_DIR=%s CARGO_PROFILE_DEV_DEBUG=0 exec%s\n' \
  "$freebsd_directory" "$freebsd_directory" "$freebsd_command" | ssh freebsd sh
