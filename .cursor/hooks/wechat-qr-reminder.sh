#!/usr/bin/env bash
set -euo pipefail

ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
QR="docs/assets/wechat-group-qr.jpg"

t="$(git -C "$ROOT" log -1 --format=%ct -- "$QR" 2>/dev/null || true)"
if [ -z "$t" ]; then
  echo '{}'
  exit 0
fi

d="$(( ( $(date +%s) - t ) / 86400 ))"
if [ "$d" -lt 6 ]; then
  echo '{}'
  exit 0
fi

export DAYS="$d"
python3 - <<'PY'
import json
import os

days = os.environ["DAYS"]
msg = (
    f"[wechat-qr-reminder] docs/assets/wechat-group-qr.jpg 距上次更新已 {days} 天，"
    "微信群二维码 7 天失效。请在本次回答开头主动提醒用户重新导出二维码并更新。"
    "处理流程见 AGENTS.md「WeChat group QR maintenance」。"
)
print(json.dumps({"additional_context": msg}))
PY
