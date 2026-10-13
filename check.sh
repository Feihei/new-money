#!/usr/bin/env bash
# 质量门：每一步都单独取退出码，不用管道尾吞掉失败。
set -u
cd "$(dirname "$0")"

fail=0
step() {
  local name="$1"; shift
  if "$@" > "/tmp/nm_$name.log" 2>&1; then
    echo "  ok    $name"
  else
    echo "  FAIL  $name (exit=$?)  -> /tmp/nm_$name.log"
    tail -20 "/tmp/nm_$name.log"
    fail=1
  fi
}

echo "== 质量门 =="
step fmt    cargo fmt --all --check
step core   cargo test -p game_core
step check  cargo check --workspace --all-targets
step clippy cargo clippy --workspace --all-targets -- -D warnings
step board  python tools/validate_board.py assets/data/boards/prototype.json
echo "== 结果 =="
[ "$fail" -eq 0 ] && echo "全部通过" || echo "有步骤失败"
exit "$fail"
