#!/bin/bash
# Oracle Cloud Always Free VM 초기 설정 스크립트
# Ubuntu 22.04+ 기준
set -e

APP_DIR="$HOME/cbnu-notice"
REPO_URL="https://github.com/pollmap/cbnu-notice"  # ← 너의 레포 주소로 수정

echo "=== 1. 시스템 업데이트 ==="
sudo apt-get update && sudo apt-get upgrade -y
sudo apt-get install -y curl git

echo "=== 2. 프로젝트 다운로드 ==="
if [ -d "$APP_DIR" ]; then
    echo "기존 디렉토리 존재, pull..."
    cd "$APP_DIR" && git pull
else
    git clone "$REPO_URL" "$APP_DIR"
    cd "$APP_DIR"
fi

echo "=== 3. 바이너리 다운로드 (GitHub Release) ==="
# GitHub Actions에서 빌드된 바이너리 다운로드
BINARY_URL="$REPO_URL/releases/download/latest/cbnu-notice-bot"
curl -sL -o "$APP_DIR/cbnu-notice-bot" "$BINARY_URL"
chmod +x "$APP_DIR/cbnu-notice-bot"

echo "=== 4. 환경변수 설정 ==="
if [ ! -f "$APP_DIR/.env" ]; then
    cat > "$APP_DIR/.env" << 'ENVEOF'
TELOXIDE_TOKEN=여기에_봇토큰_입력
CHANNEL_ID=@cbnu_notice
# LOG_CHANNEL_ID=@cbnu_notice_log
ENVEOF
    echo "⚠️  $APP_DIR/.env 파일을 수정하세요! (봇 토큰 입력)"
    echo "   nano $APP_DIR/.env"
fi

echo "=== 5. systemd 서비스 등록 ==="
sudo cp "$APP_DIR/deploy/cbnu-notice.service" /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable cbnu-notice

echo ""
echo "============================="
echo "  설정 완료!"
echo "============================="
echo ""
echo "다음 단계:"
echo "  1. 봇 토큰 입력:  nano $APP_DIR/.env"
echo "  2. 서비스 시작:    sudo systemctl start cbnu-notice"
echo "  3. 로그 확인:      sudo journalctl -u cbnu-notice -f"
echo ""
echo "유용한 명령어:"
echo "  상태 확인:    sudo systemctl status cbnu-notice"
echo "  재시작:       sudo systemctl restart cbnu-notice"
echo "  중지:         sudo systemctl stop cbnu-notice"
echo "  업데이트:     cd $APP_DIR && bash deploy/update.sh"
