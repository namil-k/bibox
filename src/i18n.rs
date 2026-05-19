#[derive(Debug, Clone)]
pub enum Lang {
    En,
    Ko,
}

#[derive(Debug, Clone)]
pub struct Msgs {
    pub lang: Lang,
}

impl Default for Msgs {
    fn default() -> Self {
        Self { lang: Lang::En }
    }
}

impl Msgs {
    pub fn new(lang: &str) -> Self {
        Self {
            lang: match lang {
                "ko" => Lang::Ko,
                _ => Lang::En,
            },
        }
    }

    // ── static strings ────────────────────────────────────────────────────────

    pub fn no_entries(&self) -> &'static str {
        match self.lang {
            Lang::En => "No entries.",
            Lang::Ko => "항목이 없습니다.",
        }
    }

    pub fn collections_header(&self) -> &'static str {
        match self.lang {
            Lang::En => "# collections:",
            Lang::Ko => "# 컬렉션:",
        }
    }

    pub fn entry_count(&self, n: usize) -> String {
        match self.lang {
            Lang::En => format!("{} {}", n, if n == 1 { "entry" } else { "entries" }),
            Lang::Ko => format!("{}개", n),
        }
    }

    pub fn no_title(&self) -> &'static str {
        match self.lang {
            Lang::En => "(no title)",
            Lang::Ko => "(제목 없음)",
        }
    }

    pub fn no_key(&self) -> &'static str {
        match self.lang {
            Lang::En => "(no key)",
            Lang::Ko => "키 없음",
        }
    }

    pub fn cancelled(&self) -> &'static str {
        match self.lang {
            Lang::En => "Cancelled.",
            Lang::Ko => "취소됨.",
        }
    }

    pub fn sync_complete(&self) -> &'static str {
        match self.lang {
            Lang::En => "Sync complete.",
            Lang::Ko => "sync 완료.",
        }
    }

    pub fn fetching_crossref(&self) -> &'static str {
        match self.lang {
            Lang::En => "Fetching metadata from Crossref...",
            Lang::Ko => "Crossref에서 메타데이터를 가져오는 중...",
        }
    }

    pub fn searching_unpaywall(&self) -> &'static str {
        match self.lang {
            Lang::En => "Searching Unpaywall for open access PDF...",
            Lang::Ko => "Unpaywall에서 오픈 액세스 PDF를 검색하는 중...",
        }
    }

    pub fn no_oa_pdf(&self) -> &'static str {
        match self.lang {
            Lang::En => "  No open access PDF found. Saving metadata only.",
            Lang::Ko => "  오픈 액세스 PDF를 찾을 수 없습니다. 메타데이터만 저장합니다.",
        }
    }

    pub fn download_prompt(&self) -> &'static str {
        match self.lang {
            Lang::En => "Download?",
            Lang::Ko => "다운로드하시겠습니까?",
        }
    }

    pub fn downloading(&self) -> &'static str {
        match self.lang {
            Lang::En => "  Downloading...",
            Lang::Ko => "  다운로드 중...",
        }
    }

    pub fn done(&self) -> &'static str {
        match self.lang {
            Lang::En => " done",
            Lang::Ko => " 완료",
        }
    }

    pub fn doi_not_found(&self) -> &'static str {
        match self.lang {
            Lang::En => " DOI not found.",
            Lang::Ko => " DOI를 찾을 수 없습니다.",
        }
    }

    pub fn already_exists(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!("Already in library: [{}]. Skipping.", key),
            Lang::Ko => format!("이미 라이브러리에 있습니다: [{}]. 건너뜀.", key),
        }
    }

    pub fn already_exists_with_hint(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!(
                "Already in library: [{}]. Use 'bibox edit {} --doi <DOI>' to update metadata.",
                key, key
            ),
            Lang::Ko => format!(
                "이미 라이브러리에 있습니다: [{}]. 'bibox edit {} --doi <DOI>'로 메타데이터를 업데이트하세요.",
                key, key
            ),
        }
    }

    pub fn merged_fields(&self, key: &str, n: usize) -> String {
        match self.lang {
            Lang::En => format!("Merged [{}]: {} field(s) updated.", key, n),
            Lang::Ko => format!("병합됨 [{}]: {}개 필드 업데이트.", key, n),
        }
    }

    pub fn extracting_doi(&self) -> &'static str {
        match self.lang {
            Lang::En => "Extracting DOI from PDF...",
            Lang::Ko => "PDF에서 DOI를 추출하는 중...",
        }
    }

    pub fn no_file_or_doi(&self) -> &'static str {
        match self.lang {
            Lang::En => "File path, --doi, --url, or --search required.",
            Lang::Ko => "파일 경로, --doi, --url, 또는 --search가 필요합니다.",
        }
    }

    pub fn add_to_db_prompt(&self) -> &'static str {
        match self.lang {
            Lang::En => "Add to DB? (will attempt DOI extraction)",
            Lang::Ko => "DB에 추가하시겠습니까? (DOI 추출 시도)",
        }
    }

    pub fn sync_added_note(&self) -> &'static str {
        match self.lang {
            Lang::En => "Added via sync. Metadata required.",
            Lang::Ko => "bibox sync로 추가됨. 메타데이터 입력 필요.",
        }
    }

    pub fn label_key(&self) -> &'static str {
        match self.lang {
            Lang::En => "Key",
            Lang::Ko => "키",
        }
    }

    pub fn label_type(&self) -> &'static str {
        match self.lang {
            Lang::En => "Type",
            Lang::Ko => "유형",
        }
    }

    pub fn label_author(&self) -> &'static str {
        match self.lang {
            Lang::En => "Author",
            Lang::Ko => "저자",
        }
    }

    pub fn label_year(&self) -> &'static str {
        match self.lang {
            Lang::En => "Year",
            Lang::Ko => "연도",
        }
    }

    pub fn label_title(&self) -> &'static str {
        match self.lang {
            Lang::En => "Title",
            Lang::Ko => "제목",
        }
    }

    pub fn label_journal(&self) -> &'static str {
        match self.lang {
            Lang::En => "Journal",
            Lang::Ko => "저널",
        }
    }

    pub fn label_publisher(&self) -> &'static str {
        match self.lang {
            Lang::En => "Publisher",
            Lang::Ko => "출판사",
        }
    }

    pub fn label_booktitle(&self) -> &'static str {
        match self.lang {
            Lang::En => "Booktitle",
            Lang::Ko => "학회",
        }
    }

    pub fn label_volume(&self) -> &'static str {
        match self.lang {
            Lang::En => "Volume",
            Lang::Ko => "권",
        }
    }

    pub fn label_number(&self) -> &'static str {
        match self.lang {
            Lang::En => "Number",
            Lang::Ko => "호",
        }
    }

    pub fn label_pages(&self) -> &'static str {
        match self.lang {
            Lang::En => "Pages",
            Lang::Ko => "페이지",
        }
    }

    pub fn label_doi(&self) -> &'static str {
        "DOI"
    }

    pub fn label_tags(&self) -> &'static str {
        match self.lang {
            Lang::En => "Tags",
            Lang::Ko => "태그",
        }
    }

    pub fn label_collections(&self) -> &'static str {
        match self.lang {
            Lang::En => "Collections",
            Lang::Ko => "컬렉션",
        }
    }

    pub fn label_file(&self) -> &'static str {
        match self.lang {
            Lang::En => "File",
            Lang::Ko => "파일",
        }
    }

    pub fn label_howpublished(&self) -> &'static str {
        match self.lang {
            Lang::En => "How published",
            Lang::Ko => "출판 방법",
        }
    }

    pub fn label_month(&self) -> &'static str {
        match self.lang {
            Lang::En => "Month",
            Lang::Ko => "월",
        }
    }

    pub fn label_note(&self) -> &'static str {
        match self.lang {
            Lang::En => "Note",
            Lang::Ko => "노트",
        }
    }

    pub fn label_created(&self) -> &'static str {
        match self.lang {
            Lang::En => "Created",
            Lang::Ko => "생성",
        }
    }

    pub fn label_id(&self) -> &'static str {
        "ID"
    }

    pub fn no_required_fields(&self) -> &'static str {
        match self.lang {
            Lang::En => "(missing required fields)",
            Lang::Ko => "(필수 필드 없음)",
        }
    }

    // ── parameterized strings ─────────────────────────────────────────────────

    pub fn clipboard_init_failed(&self) -> &'static str {
        match self.lang {
            Lang::En => "Clipboard init failed",
            Lang::Ko => "클립보드 초기화 실패",
        }
    }

    pub fn clipboard_copy_failed(&self) -> &'static str {
        match self.lang {
            Lang::En => "Clipboard copy failed",
            Lang::Ko => "클립보드 복사 실패",
        }
    }

    pub fn found_title(&self, title: &str) -> String {
        match self.lang {
            Lang::En => format!("  Found: {}", title),
            Lang::Ko => format!("  찾음: {}", title),
        }
    }

    pub fn doi_lookup_failed(&self, e: &str) -> String {
        match self.lang {
            Lang::En => format!("DOI lookup failed: {}", e),
            Lang::Ko => format!("DOI 조회 실패: {}", e),
        }
    }

    pub fn oa_found(&self, source: &str) -> String {
        match self.lang {
            Lang::En => format!("  Open access PDF found. (source: {})", source),
            Lang::Ko => format!("  오픈 액세스 PDF를 찾았습니다. (출처: {})", source),
        }
    }

    pub fn unpaywall_failed(&self, e: &str) -> String {
        match self.lang {
            Lang::En => format!("  Unpaywall lookup failed: {}", e),
            Lang::Ko => format!("  Unpaywall 조회 실패: {}", e),
        }
    }

    pub fn doi_found(&self, doi: &str) -> String {
        match self.lang {
            Lang::En => format!(" found: {}", doi),
            Lang::Ko => format!(" 찾음: {}", doi),
        }
    }

    pub fn doi_extract_failed(&self, e: &str) -> String {
        match self.lang {
            Lang::En => format!(" extraction failed: {}", e),
            Lang::Ko => format!(" 추출 실패: {}", e),
        }
    }

    pub fn meta_lookup_failed(&self, e: &str) -> String {
        match self.lang {
            Lang::En => format!("  Metadata lookup failed: {}. Manual entry required.", e),
            Lang::Ko => format!("  메타데이터 조회 실패: {}. 수동 입력이 필요합니다.", e),
        }
    }

    pub fn file_moved(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("  File moved: {}", name),
            Lang::Ko => format!("  파일 이동: {}", name),
        }
    }

    pub fn file_copy_failed(&self, src: &str, dst: &str) -> String {
        match self.lang {
            Lang::En => format!("File copy failed: {} -> {}", src, dst),
            Lang::Ko => format!("파일 복사 실패: {} -> {}", src, dst),
        }
    }

    pub fn added(&self, key: &str, title: &str) -> String {
        match self.lang {
            Lang::En => format!("Added: [{}] {}", key, title),
            Lang::Ko => format!("추가됨: [{}] {}", key, title),
        }
    }

    pub fn showing_of(&self, shown: usize, total: usize) -> String {
        match self.lang {
            Lang::En => format!(
                "  ... showing {} of {}. Use --limit to see more.",
                shown, total
            ),
            Lang::Ko => format!(
                "  ... {} 개 중 {} 개 표시. --limit 으로 더 볼 수 있습니다.",
                total, shown
            ),
        }
    }

    pub fn total(&self, n: usize) -> String {
        match self.lang {
            Lang::En => format!("Total: {}", n),
            Lang::Ko => format!("총 {} 개", n),
        }
    }

    pub fn no_results(&self, query: &str) -> String {
        match self.lang {
            Lang::En => format!("No results for: \"{}\"", query),
            Lang::Ko => format!("검색 결과가 없습니다: \"{}\"", query),
        }
    }

    pub fn copied_to_clipboard(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!("Copied to clipboard: {}", key),
            Lang::Ko => format!("클립보드에 복사됨: {}", key),
        }
    }

    pub fn entry_not_found(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!("Entry not found: {}", key),
            Lang::Ko => format!("항목을 찾을 수 없습니다: {}", key),
        }
    }

    pub fn file_rename_failed(&self, path: &str) -> String {
        match self.lang {
            Lang::En => format!("File rename failed: {}", path),
            Lang::Ko => format!("파일 이름 변경 실패: {}", path),
        }
    }

    pub fn file_renamed(&self, old: &str, new: &str) -> String {
        match self.lang {
            Lang::En => format!("File renamed: {} -> {}", old, new),
            Lang::Ko => format!("파일 이름 변경: {} -> {}", old, new),
        }
    }

    pub fn updated(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!("Updated: [{}]", key),
            Lang::Ko => format!("수정됨: [{}]", key),
        }
    }

    pub fn delete_prompt(&self, key: &str, title: &str) -> String {
        match self.lang {
            Lang::En => format!("Delete [{}] {}?", key, title),
            Lang::Ko => format!("[{}] {} 를 삭제하시겠습니까?", key, title),
        }
    }

    pub fn file_deleted(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("File deleted: {}", name),
            Lang::Ko => format!("파일 삭제: {}", name),
        }
    }

    pub fn deleted(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!("Deleted: [{}]", key),
            Lang::Ko => format!("삭제됨: [{}]", key),
        }
    }

    pub fn collect_added(&self, key: &str, cols: &str) -> String {
        match self.lang {
            Lang::En => format!("[{}] added to collections: {}", key, cols),
            Lang::Ko => format!("[{}] 컬렉션 추가: {}", key, cols),
        }
    }

    pub fn collect_skipped(&self, key: &str, cols: &str) -> String {
        match self.lang {
            Lang::En => format!("[{}] already in collections (skipped): {}", key, cols),
            Lang::Ko => format!("[{}] 이미 소속됨 (스킵): {}", key, cols),
        }
    }

    pub fn not_in_collection(&self, key: &str, col: &str) -> String {
        match self.lang {
            Lang::En => format!("[{}] is not in collection '{}'", key, col),
            Lang::Ko => format!("[{}] 는 '{}' 컬렉션에 속해 있지 않습니다.", key, col),
        }
    }

    pub fn uncollected(&self, key: &str, col: &str) -> String {
        match self.lang {
            Lang::En => format!("[{}] removed from collection: {}", key, col),
            Lang::Ko => format!("[{}] 컬렉션 제거: {}", key, col),
        }
    }

    pub fn file_read_failed(&self, path: &str) -> String {
        match self.lang {
            Lang::En => format!("Cannot read file: {}", path),
            Lang::Ko => format!("파일을 읽을 수 없습니다: {}", path),
        }
    }

    pub fn import_complete(&self, n: usize) -> String {
        match self.lang {
            Lang::En => format!("Import complete: {} added", n),
            Lang::Ko => format!("임포트 완료: {} 개 추가됨", n),
        }
    }

    pub fn skipped_header(&self, n: usize) -> String {
        match self.lang {
            Lang::En => format!("Skipped ({}):", n),
            Lang::Ko => format!("스킵됨 ({} 개):", n),
        }
    }

    pub fn zip_created(&self, path: &str, n: usize) -> String {
        match self.lang {
            Lang::En => format!("ZIP created: {} ({} files)", path, n),
            Lang::Ko => format!("ZIP 생성: {} ({} 개 파일)", path, n),
        }
    }

    pub fn folder_created(&self, path: &str, n: usize) -> String {
        match self.lang {
            Lang::En => format!("Folder created: {} ({} files)", path, n),
            Lang::Ko => format!("폴더 생성: {} ({} 개 파일)", path, n),
        }
    }

    pub fn clipboard_copied_entries(&self, n: usize) -> String {
        match self.lang {
            Lang::En => format!("Copied to clipboard ({} entries)", n),
            Lang::Ko => format!("클립보드에 복사됨 ({} 개 항목)", n),
        }
    }

    pub fn bibtex_saved(&self, path: &str, n: usize) -> String {
        match self.lang {
            Lang::En => format!("BibTeX saved: {} ({} entries)", path, n),
            Lang::Ko => format!("BibTeX 저장: {} ({} 개 항목)", path, n),
        }
    }

    pub fn sync_file_missing(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("'{}' not found on disk. Remove from DB?", name),
            Lang::Ko => format!("'{}' 파일이 없습니다. DB에서 삭제하시겠습니까?", name),
        }
    }

    pub fn sync_removed(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("Removed: {}", name),
            Lang::Ko => format!("삭제됨: {}", name),
        }
    }

    pub fn sync_new_file(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("New file found: {}", name),
            Lang::Ko => format!("새 파일 발견: {}", name),
        }
    }

    pub fn sync_entry_added(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!(
                "  Added: [{}] (metadata needed: bibox edit {} --doi <DOI>)",
                key, key
            ),
            Lang::Ko => format!(
                "  추가됨: [{}] (메타데이터 입력 필요: bibox edit {} --doi <DOI>)",
                key, key
            ),
        }
    }

    pub fn entry_block_meta(
        &self,
        entry_type: &str,
        author: &str,
        year: &str,
        tags: &str,
        collections: &str,
    ) -> String {
        match self.lang {
            Lang::En => format!(
                "  type: {} | author: {} | year: {}{}{}",
                entry_type, author, year, tags, collections
            ),
            Lang::Ko => format!(
                "  유형: {} | 저자: {} | 연도: {}{}{}",
                entry_type, author, year, tags, collections
            ),
        }
    }

    pub fn tag_inline(&self, tags: &str) -> String {
        match self.lang {
            Lang::En => format!(" | tags: {}", tags),
            Lang::Ko => format!(" | 태그: {}", tags),
        }
    }

    pub fn collection_inline(&self, cols: &str) -> String {
        match self.lang {
            Lang::En => format!(" | collections: {}", cols),
            Lang::Ko => format!(" | 컬렉션: {}", cols),
        }
    }

    pub fn searching_arxiv(&self) -> &'static str {
        match self.lang {
            Lang::En => "Searching arXiv by title...",
            Lang::Ko => "arXiv에서 제목으로 검색 중...",
        }
    }

    pub fn no_arxiv_results(&self) -> &'static str {
        match self.lang {
            Lang::En => "  No results found on arXiv.",
            Lang::Ko => "  arXiv에서 결과를 찾을 수 없습니다.",
        }
    }

    pub fn arxiv_failed(&self, e: &str) -> String {
        match self.lang {
            Lang::En => format!("  arXiv search failed: {}", e),
            Lang::Ko => format!("  arXiv 검색 실패: {}", e),
        }
    }

    pub fn arxiv_found(&self, n: usize) -> String {
        match self.lang {
            Lang::En => format!("  Found {} result(s) on arXiv. Select to download:", n),
            Lang::Ko => format!("  arXiv에서 {} 개 검색됨. 선택하여 다운로드:", n),
        }
    }

    pub fn searching_crossref_query(&self, query: &str) -> String {
        match self.lang {
            Lang::En => format!("Searching Crossref for \"{}\"...", query),
            Lang::Ko => format!("Crossref에서 \"{}\" 검색 중...", query),
        }
    }

    pub fn no_search_results(&self, query: &str) -> String {
        match self.lang {
            Lang::En => format!("No results found for '{}'.", query),
            Lang::Ko => format!("'{}'에 대한 검색 결과가 없습니다.", query),
        }
    }

    pub fn url_resolve_failed(&self) -> &'static str {
        match self.lang {
            Lang::En => "Could not extract metadata from URL. Try --doi or --search instead.",
            Lang::Ko => "URL에서 메타데이터를 추출할 수 없습니다. --doi 또는 --search를 사용하세요.",
        }
    }

    pub fn url_fetch_failed(&self, reason: &str) -> String {
        match self.lang {
            Lang::En => format!("Failed to fetch URL: {}", reason),
            Lang::Ko => format!("URL 가져오기 실패: {}", reason),
        }
    }

    pub fn note_saved(&self, path: &str) -> String {
        match self.lang {
            Lang::En => format!("Note saved: {}", path),
            Lang::Ko => format!("노트 저장됨: {}", path),
        }
    }

    pub fn note_not_found(&self, key: &str) -> String {
        match self.lang {
            Lang::En => format!("No note found for '{}'.", key),
            Lang::Ko => format!("'{}'에 대한 노트가 없습니다.", key),
        }
    }

    pub fn note_already_exists(&self) -> &'static str {
        match self.lang {
            Lang::En => "Note already exists. Use --force to overwrite with template.",
            Lang::Ko => "노트가 이미 존재합니다. --force를 사용하여 템플릿으로 덮어쓰세요.",
        }
    }

    pub fn section_requires_source(&self) -> &'static str {
        match self.lang {
            Lang::En => "--section requires --stdin or --from.",
            Lang::Ko => "--section은 --stdin 또는 --from이 필요합니다.",
        }
    }

    pub fn note_written_section(&self, section: &str, path: &str) -> String {
        match self.lang {
            Lang::En => format!("Section '{}' written to {}", section, path),
            Lang::Ko => format!("섹션 '{}'이(가) {}에 작성됨", section, path),
        }
    }

    pub fn note_appended(&self, path: &str) -> String {
        match self.lang {
            Lang::En => format!("Content appended to {}", path),
            Lang::Ko => format!("내용이 {}에 추가됨", path),
        }
    }

    pub fn note_template_applied(&self, template: &str, path: &str) -> String {
        match self.lang {
            Lang::En => format!("Template '{}' applied to {}", template, path),
            Lang::Ko => format!("템플릿 '{}'이(가) {}에 적용됨", template, path),
        }
    }
}
