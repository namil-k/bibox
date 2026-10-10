use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    Article,
    Book,
    InProceedings,
    /// 모르는 타입은 전부 여기로. 가져오기 기본값이기도 하다.
    #[default]
    Misc,
}

impl std::fmt::Display for EntryType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EntryType::Article => write!(f, "article"),
            EntryType::Book => write!(f, "book"),
            EntryType::InProceedings => write!(f, "inproceedings"),
            EntryType::Misc => write!(f, "misc"),
        }
    }
}

impl std::str::FromStr for EntryType {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "article" => Ok(EntryType::Article),
            "book" | "inbook" | "booklet" => Ok(EntryType::Book),
            "inproceedings" | "incollection" | "conference" => Ok(EntryType::InProceedings),
            "misc" | "online" | "electronic" | "www"
            | "phdthesis" | "mastersthesis" | "thesis"
            | "techreport" | "report" | "manual"
            | "unpublished" | "proceedings" | "patent"
            | "standard" | "dataset" | "software" => Ok(EntryType::Misc),
            _ => Err(anyhow::anyhow!("Unknown entry type: {}", s)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub bibtex_key: String,
    pub entry_type: EntryType,
    pub title: Option<String>,
    pub author: Vec<String>,
    pub year: Option<u32>,

    // Article fields
    pub journal: Option<String>,
    pub volume: Option<String>,
    pub number: Option<String>,
    pub pages: Option<String>,

    // Book fields
    pub publisher: Option<String>,
    pub editor: Option<String>,
    pub edition: Option<String>,
    pub isbn: Option<String>,

    // InProceedings fields
    pub booktitle: Option<String>,

    // Common optional fields
    pub doi: Option<String>,
    pub url: Option<String>,
    #[serde(default, rename = "abstract")]
    pub abstract_text: Option<String>,
    pub tags: Vec<String>,
    // Misc fields
    pub howpublished: Option<String>,
    pub month: Option<String>,
    pub note: Option<String>,
    pub collections: Vec<String>,
    pub file_path: Option<String>,
    pub created_at: String,
    #[serde(default)]
    pub updated_at: Option<String>,
}

impl Entry {
    pub fn author_display(&self) -> String {
        match self.author.len() {
            0 => String::from("Unknown"),
            1 => self.author[0]
                .split(',')
                .next()
                .unwrap_or(&self.author[0])
                .trim()
                .to_string(),
            _ => {
                let last = self.author[0]
                    .split(',')
                    .next()
                    .unwrap_or(&self.author[0])
                    .trim()
                    .to_string();
                format!("{} et al.", last)
            }
        }
    }
}

/// 새로 찍는 수정·생성 시각. 시간대를 붙여야 다른 기계(서버는 UTC)의 시각과 비교된다.
pub fn now_stamp() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false)
}

/// 저장된 시각 하나. 옛 데이터는 시간대 없는 `%Y-%m-%d %H:%M:%S`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stamp {
    Zoned(chrono::DateTime<chrono::FixedOffset>),
    Local(chrono::NaiveDateTime),
    Unknown,
}

pub fn parse_stamp(s: &str) -> Stamp {
    if let Ok(t) = chrono::DateTime::parse_from_rfc3339(s) {
        return Stamp::Zoned(t);
    }
    match chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        Ok(t) => Stamp::Local(t),
        Err(_) => Stamp::Unknown,
    }
}

/// 한 기계 안의 정렬용 시각(UTC). 시간대 없는 값은 이 기계의 현지 시각으로 본다.
fn sort_instant(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    match parse_stamp(s) {
        Stamp::Zoned(t) => Some(t.with_timezone(&chrono::Utc)),
        Stamp::Local(n) => n.and_local_timezone(chrono::Local).earliest().map(|t| t.with_timezone(&chrono::Utc)),
        Stamp::Unknown => None,
    }
}

/// 정렬 비교. 읽을 수 없는 값은 앞으로, 같으면 문자열로.
pub fn cmp_stamps(a: &str, b: &str) -> std::cmp::Ordering {
    sort_instant(a).cmp(&sort_instant(b)).then_with(|| a.cmp(b))
}

/// 화면용 `YYYY-MM-DD HH:MM`(이 기계의 현지 시각). 읽을 수 없으면 그대로.
pub fn display_stamp(s: &str) -> String {
    match parse_stamp(s) {
        Stamp::Zoned(t) => t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string(),
        Stamp::Local(n) => n.format("%Y-%m-%d %H:%M").to_string(),
        Stamp::Unknown => s.to_string(),
    }
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Database {
    pub entries: Vec<Entry>,
}

#[cfg(test)]
mod stamp_tests {
    use super::*;

    #[test]
    fn now_stamp_carries_its_offset_and_parses_back() {
        let s = now_stamp();
        assert_eq!(s.len(), 25, "{}", s); // 2026-10-10T10:05:00+02:00
        assert!(matches!(parse_stamp(&s), Stamp::Zoned(_)), "{}", s);
    }

    #[test]
    fn old_stamps_are_local_and_garbage_is_unknown() {
        assert!(matches!(parse_stamp("2026-04-10 13:17:02"), Stamp::Local(_)));
        assert!(matches!(parse_stamp(""), Stamp::Unknown));
        assert!(matches!(parse_stamp("yesterday"), Stamp::Unknown));
    }

    #[test]
    fn zoned_stamps_compare_as_instants() {
        // 08:05 UTC는 10:00 +02:00(= 08:00 UTC)보다 늦다
        assert_eq!(cmp_stamps("2026-10-10T08:05:00+00:00", "2026-10-10T10:00:00+02:00"), std::cmp::Ordering::Greater);
        assert_eq!(cmp_stamps("2026-10-10 09:00:00", "2026-10-10 10:00:00"), std::cmp::Ordering::Less);
    }

    #[test]
    fn display_drops_seconds_and_offset() {
        assert_eq!(display_stamp("2026-04-10 13:17:02"), "2026-04-10 13:17");
        assert_eq!(display_stamp("not a time"), "not a time");
        assert_eq!(display_stamp(&now_stamp()).len(), 16);
    }
}
