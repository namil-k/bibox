use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum EntryType {
    Article,
    Book,
    InProceedings,
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
            "book" => Ok(EntryType::Book),
            "inproceedings" => Ok(EntryType::InProceedings),
            "misc" => Ok(EntryType::Misc),
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
    pub tags: Vec<String>,
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

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Database {
    pub entries: Vec<Entry>,
}
