#!/usr/bin/env bash
set -euo pipefail

function build_image() {
  local tag="$1"
  docker build --tag "$tag" .
}

deploy() {
  local environment="$1"
  build_image "app:${environment}"
  printf 'deploying %s\n' "$environment"
}

outer() {
  helper() {
    printf 'nested helper\n'
  }
  helper
}
