#!/bin/bash
# 봇 업데이트 스크립트 (새 버전 배포 시)
set -e

APP_DIR="$HOME/cbnu-notice"
REPO_URL="https://github.com/pollmap/cbnu-notice"  # ← 너의 레포 주소로 수정

echo "=== 업데이트 시작 ==="

cd "$APP_DIR"
git pull

# 새 바이너리 다운로드
BINARY_URL="$REPO_URL/releases/download/latest/cbnu-notice-bot"
curl -sL -o "$APP_DIR/cbnu-notice-bot" "$BINARY_URL"
chmod +x "$APP_DIR/cbnu-notice-bot"

# 서비스 재시작
sudo systemctl restart cbnu-notice

echo "=== 업데이트 완료! ==="
sudo systemctl status cbnu-notice --no-pager
