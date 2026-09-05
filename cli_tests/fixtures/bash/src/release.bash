#!/usr/bin/env bash

function publish {
  local channel="$1"
  local artifacts=(cli lib docs)

  case "$channel" in
    stable)
      target="production"
      ;;
    *)
      target="staging"
      ;;
  esac

  for artifact in "${artifacts[@]}"; do
    printf '%s -> %s\\n' "$artifact" "${target}"
  done

  cat <<EOF
release ${channel} to ${target}
EOF
}
