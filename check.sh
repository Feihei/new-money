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
# 表现层的单元测试以前不在门里：`check --all-targets` 只做类型检查、不出目标码，于是一条
# panic 在测试里的用例（`MinimalPlugins` 不含 `StatesPlugin`）能让整张门报「全部通过」。
# ARCHITECTURE.md §八 早就写着质量门要 `cargo test --workspace`，这一步是补上那句的落地。
# `game_app` 是纯二进制 crate（没有 lib target，`--lib` 会直接 error 101），所以整包跑
step app    cargo test -p game_app
step board  python tools/validate_board.py assets/data/boards/prototype.json
echo "== 结果 =="
[ "$fail" -eq 0 ] && echo "全部通过" || echo "有步骤失败"
exit "$fail"
