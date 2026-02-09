# cbnu-notice-bot

충북대학교 공지사항을 자동 크롤링하여 텔레그램 채널로 알림을 보내주는 Rust 기반 봇

## 텔레그램 채널

**[@cbnu_notice](https://t.me/cbnu_notice)** - 구독하면 충북대 공지사항을 실시간으로 받아볼 수 있습니다.

## 지원하는 공지 소스

### 대학 본부 (eGovFramework)
- 충북대 대학공지 (`chungbuk.ac.kr`)
- 충북대 서비스공지 (`cbnu.ac.kr`)

### 학과 (PHP master.php)
- 경영학부, 천문우주학과, 물리학과, 수학과
- 원예과학과, 식품생명공학과, 생화학과, 간호학과
- `config.toml`에 서브도메인과 `pg_idx`만 추가하면 새 학과 지원 가능

## 동작 방식

1. GitHub Actions가 15분마다 크롤링 실행
2. 각 소스의 공지 목록 1페이지를 파싱
3. SQLite DB로 중복 감지 (신규 공지만 필터)
4. 텔레그램 채널로 자동 발송
5. DB를 GitHub Artifact로 보존

## 새 학과 추가 방법

`config.toml`에 다음 블록을 추가하고 PR을 보내주세요:

```toml
[[source]]
key = "your_dept"
display_name = "🏫 학과이름"
parser = "php_master"
url = "https://subdomain.chungbuk.ac.kr"
enabled = true

[source.params]
pg_idx = "학과페이지의pg_idx값"
```

`pg_idx`는 학과 홈페이지의 공지사항 메뉴 링크에서 확인할 수 있습니다.

## 로컬 개발

```bash
# 빌드
cargo build

# 테스트
cargo test

# 크롤링 실행 (dry-run, 텔레그램 발송 없음)
cargo run -- crawl

# 텔레그램 발송 포함 실행
TELOXIDE_TOKEN=your_bot_token CHANNEL_ID=@your_channel cargo run -- crawl
```

## 환경변수

| 변수 | 설명 | 필수 |
|------|------|------|
| `TELOXIDE_TOKEN` | 텔레그램 봇 API 토큰 | 예 (없으면 dry-run) |
| `CHANNEL_ID` | 메인 채널 (`@cbnu_notice`) | 예 |
| `LOG_CHANNEL_ID` | 에러 알림 채널 (비공개) | 아니오 |

## 기술 스택

- **Rust** + tokio (비동기 런타임)
- **reqwest** (HTTP 크롤링)
- **scraper** (HTML 파싱)
- **teloxide** (텔레그램 봇 API)
- **rusqlite** (SQLite 중복 감지)
- **GitHub Actions** (15분 cron 무료 배포)

## 라이선스

MIT
