use chrono::{Local, NaiveDate};
use regex::Regex;

/// 공지 제목에서 마감일을 추출한다.
/// "~까지", "마감" 키워드 근처의 날짜를 우선, 없으면 제목 내 마지막 날짜를 반환.
pub fn extract_deadline(title: &str) -> Option<NaiveDate> {
    let year = Local::now().format("%Y").to_string().parse::<i32>().unwrap_or(2026);

    // 패턴 1: YYYY.MM.DD / YYYY-MM-DD / YYYY/MM/DD
    let re_full = Regex::new(r"(\d{4})[.\-/](\d{1,2})[.\-/](\d{1,2})").unwrap();
    // 패턴 2: M.D / M월D일 / M월 D일
    let re_md = Regex::new(r"(\d{1,2})[.\uc6d4]\s?(\d{1,2})[.\uc77c]?").unwrap();

    // "까지", "마감" 근처 날짜 우선 탐색
    let deadline_keywords = ["까지", "마감", "이내"];
    for kw in &deadline_keywords {
        if let Some(pos) = title.find(kw) {
            // 키워드 앞 40자 범위에서 날짜 검색
            let start = pos.saturating_sub(40);
            let region = &title[start..pos];

            if let Some(caps) = re_full.captures(region) {
                if let Some(d) = parse_ymd(&caps[1], &caps[2], &caps[3]) {
                    return Some(d);
                }
            }
            if let Some(caps) = re_md.captures(region) {
                if let Some(d) = parse_md(year, &caps[1], &caps[2]) {
                    return Some(d);
                }
            }
        }
    }

    // fallback: 제목 전체에서 마지막으로 등장하는 날짜
    let mut last: Option<NaiveDate> = None;
    for caps in re_full.captures_iter(title) {
        if let Some(d) = parse_ymd(&caps[1], &caps[2], &caps[3]) {
            last = Some(d);
        }
    }
    if last.is_some() {
        return last;
    }
    for caps in re_md.captures_iter(title) {
        if let Some(d) = parse_md(year, &caps[1], &caps[2]) {
            last = Some(d);
        }
    }
    last
}

fn parse_ymd(y: &str, m: &str, d: &str) -> Option<NaiveDate> {
    let y: i32 = y.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    let d: u32 = d.parse().ok()?;
    NaiveDate::from_ymd_opt(y, m, d)
}

fn parse_md(year: i32, m: &str, d: &str) -> Option<NaiveDate> {
    let m: u32 = m.parse().ok()?;
    let d: u32 = d.parse().ok()?;
    NaiveDate::from_ymd_opt(year, m, d)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Datelike;

    #[test]
    fn test_full_date_with_deadline_keyword() {
        let d = extract_deadline("장학금 신청 (~2026.02.14까지)");
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 2, 14));
    }

    #[test]
    fn test_short_date_with_keyword() {
        let d = extract_deadline("(재)하림장학재단 추천 안내(2.10.(화)까지 신청서 제출)");
        assert!(d.is_some());
        assert_eq!(d.unwrap().month(), 2);
        assert_eq!(d.unwrap().day(), 10);
    }

    #[test]
    fn test_range_picks_last() {
        let d = extract_deadline("2.6(금)~2.8(일) 등록금 납부");
        assert!(d.is_some());
        assert_eq!(d.unwrap().day(), 8);
    }

    #[test]
    fn test_no_date_returns_none() {
        assert!(extract_deadline("장학금 신청 안내").is_none());
    }

    #[test]
    fn test_full_iso_date() {
        let d = extract_deadline("2026-03-01 마감 공지");
        assert_eq!(d, NaiveDate::from_ymd_opt(2026, 3, 1));
    }
}
