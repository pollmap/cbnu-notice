use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Academic,
    Scholarship,
    Recruit,
    Contest,
    Event,
    General,
}

impl Category {
    /// Classify a notice by title keywords. Priority order matters.
    pub fn classify(title: &str) -> Self {
        let t = title.to_lowercase();

        let rules: &[(&[&str], Category)] = &[
            (
                &[
                    "수강", "학점", "성적", "졸업", "휴학", "복학", "전과", "재입학", "수업",
                    "학사일정", "교육과정", "이수", "학기", "편입", "등록금 납부", "학위",
                ],
                Category::Academic,
            ),
            (
                &[
                    "장학", "학자금", "등록금 감면", "국가장학", "교내장학", "근로장학",
                ],
                Category::Scholarship,
            ),
            (
                &[
                    "채용", "인사", "공무직", "계약직", "교원", "조교", "강사 채용", "직원",
                    "합격자", "경쟁채용",
                ],
                Category::Recruit,
            ),
            (
                &[
                    "모집", "공모", "선발", "신청 안내", "접수", "지원자", "참가자", "대회",
                    "공모전",
                ],
                Category::Contest,
            ),
            (
                &[
                    "특강", "세미나", "워크숍", "설명회", "포럼", "행사", "축제", "공연",
                    "전시", "초청",
                ],
                Category::Event,
            ),
        ];

        for (keywords, category) in rules {
            if keywords.iter().any(|k| t.contains(k)) {
                return category.clone();
            }
        }
        Category::General
    }

    pub fn emoji(&self) -> &str {
        match self {
            Self::Academic => "\u{1f4da}",     // 📚
            Self::Scholarship => "\u{1f4b0}",  // 💰
            Self::Recruit => "\u{1f4bc}",      // 💼
            Self::Contest => "\u{1f4cb}",      // 📋
            Self::Event => "\u{1f3a4}",        // 🎤
            Self::General => "\u{1f4e2}",      // 📢
        }
    }

    pub fn label(&self) -> &str {
        match self {
            Self::Academic => "학사",
            Self::Scholarship => "장학",
            Self::Recruit => "채용",
            Self::Contest => "모집",
            Self::Event => "행사",
            Self::General => "일반",
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Academic => "academic",
            Self::Scholarship => "scholarship",
            Self::Recruit => "recruit",
            Self::Contest => "contest",
            Self::Event => "event",
            Self::General => "general",
        }
    }

    pub fn from_str_tag(s: &str) -> Self {
        match s {
            "academic" => Self::Academic,
            "scholarship" => Self::Scholarship,
            "recruit" => Self::Recruit,
            "contest" => Self::Contest,
            "event" => Self::Event,
            _ => Self::General,
        }
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.emoji(), self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_classify() {
        assert_eq!(
            Category::classify("2026학년도 1학기 수강신청 일정 안내"),
            Category::Academic
        );
        assert_eq!(
            Category::classify("2026학년도 국가장학금 신청 안내"),
            Category::Scholarship
        );
        assert_eq!(
            Category::classify("2026년도 제1차 직원(공무직) 채용 공고"),
            Category::Recruit
        );
        assert_eq!(
            Category::classify("해외 어학연수 참가자 모집"),
            Category::Contest
        );
        assert_eq!(
            Category::classify("AI 특강 및 세미나 안내"),
            Category::Event
        );
        assert_eq!(
            Category::classify("캠퍼스 도로 보수공사 안내"),
            Category::General
        );
        // Priority test: "장학금 모집" should be Scholarship (higher priority)
        assert_eq!(
            Category::classify("교내장학금 신청 모집"),
            Category::Scholarship
        );
    }
}
