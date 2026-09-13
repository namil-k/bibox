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

    pub fn url_resolve_failed(&self, url: &str, reason: Option<&str>) -> String {
        match self.lang {
            Lang::En => {
                let detail = reason.map(|r| format!("\nReason: {}", r)).unwrap_or_default();
                format!("Could not get a DOI from {}{}\nMany publishers block automated requests, so the page cannot be read directly. Supply the DOI with --doi, look the paper up with --search, or record it manually with --title.", url, detail)
            }
            Lang::Ko => {
                let detail = reason.map(|r| format!("\n이유: {}", r)).unwrap_or_default();
                format!("{}에서 DOI를 얻지 못했습니다.{}\n상당수 출판사가 자동 요청을 차단해 페이지를 직접 읽을 수 없습니다. --doi로 DOI를 지정하거나, --search로 논문을 검색하거나, --title로 수동 기록하세요.", url, detail)
            }
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

    pub fn unmapped_fields_warning(&self, n: usize, names: &str) -> String {
        match self.lang {
            Lang::En => format!("⚠ {} unmapped field(s) ignored: {}", n, names),
            Lang::Ko => format!("⚠ 매핑 안 된 필드 {}개 무시됨: {}", n, names),
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

    pub fn exported_to(&self, n: usize, path: &str) -> String {
        match self.lang {
            Lang::En => format!("Exported {} entries to {}", n, path),
            Lang::Ko => format!("{} 개 항목을 {}에 내보냈습니다", n, path),
        }
    }

    pub fn reveal_question(&self) -> &'static str {
        match self.lang {
            Lang::En => "Reveal in file manager?",
            Lang::Ko => "파일 관리자에서 열까요?",
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

    // ── 인용 복사 ──────────────────────────────────────────────────────────────

    pub fn citation_copied(&self, style: &str, n: usize) -> String {
        match (&self.lang, n) {
            (Lang::En, 1) => format!("Copied {} citation", style),
            (Lang::En, n) => format!("Copied {} {} citations", n, style),
            (Lang::Ko, 1) => format!("{} 인용을 복사했습니다", style),
            (Lang::Ko, n) => format!("{} 인용 {}개를 복사했습니다", style, n),
        }
    }

    // ── keymap.toml 로드 진단 ──────────────────────────────────────────────────

    pub fn keymap_fallback_header(&self) -> &'static str {
        match self.lang {
            Lang::En => "keymap.toml has problems, starting with the default keymap.",
            Lang::Ko => "keymap.toml에 문제가 있어 기본 키맵으로 시작합니다.",
        }
    }

    pub fn keymap_warning_header(&self) -> &'static str {
        match self.lang {
            Lang::En => "keymap.toml warnings:",
            Lang::Ko => "keymap.toml 경고:",
        }
    }

    pub fn keymap_press_enter(&self) -> &'static str {
        match self.lang {
            Lang::En => "Press Enter to continue...",
            Lang::Ko => "계속하려면 Enter를 누르세요...",
        }
    }



    pub fn keymap_problem(&self, p: &crate::keymap::KeymapProblem) -> String {
        use crate::keymap::KeymapProblem::*;
        match (&self.lang, p) {
            (Lang::En, Syntax { detail }) => format!("  TOML syntax: {}", detail),
            (Lang::Ko, Syntax { detail }) => format!("  TOML 문법: {}", detail),
            (Lang::En, UnknownLayer { detail }) => format!("  unknown layer: {}", detail),
            (Lang::Ko, UnknownLayer { detail }) => format!("  알 수 없는 레이어: {}", detail),
            (Lang::En, UnknownAction { detail }) => format!("  unknown action: {}", detail),
            (Lang::Ko, UnknownAction { detail }) => format!("  알 수 없는 액션: {}", detail),
            (Lang::En, BadKey { layer, token }) => {
                format!("  [{}] unknown key notation \"{}\"", layer, token)
            }
            (Lang::Ko, BadKey { layer, token }) => {
                format!("  [{}] 알 수 없는 키 표기 \"{}\"", layer, token)
            }
            (Lang::En, PrefixConflict { layer, shorter, longer }) => format!(
                "  [{}] \"{}\" is a prefix of \"{}\", so it would never fire",
                layer, shorter, longer
            ),
            (Lang::Ko, PrefixConflict { layer, shorter, longer }) => format!(
                "  [{}] \"{}\"가 \"{}\"의 접두사라 영원히 걸리지 않습니다",
                layer, shorter, longer
            ),
            (Lang::En, DuplicateBinding { layer, keys }) => {
                format!("  [{}] \"{}\" is bound twice; the first one wins", layer, keys)
            }
            (Lang::Ko, DuplicateBinding { layer, keys }) => {
                format!("  [{}] \"{}\"가 두 번 바인딩되어 앞의 것을 씁니다", layer, keys)
            }
            (Lang::En, LayerNotWired { layer }) => {
                format!("  [{}] this layer is not wired up yet and was ignored", layer)
            }
            (Lang::Ko, LayerNotWired { layer }) => {
                format!("  [{}] 아직 구현되지 않은 레이어라 무시했습니다", layer)
            }
            (Lang::En, UnknownPluginCommand { layer, name }) => {
                format!("  [{}] plugin command \"{}\" is not installed or is disabled; that binding was skipped", layer, name)
            }
            (Lang::Ko, UnknownPluginCommand { layer, name }) => {
                format!("  [{}] 플러그인 명령 \"{}\"가 설치되지 않았거나 꺼져 있어 그 바인딩을 건너뜁니다", layer, name)
            }
            (Lang::En, PluginKeyShadowed { plugin, command, layer, key, by }) => format!(
                "  [{}] {}.{}: default key \"{}\" is shadowed by {}; bind it in keymap.toml",
                layer, plugin, command, key, by
            ),
            (Lang::Ko, PluginKeyShadowed { plugin, command, layer, key, by }) => format!(
                "  [{}] {}.{}: 기본 키 \"{}\"가 {}에 가려집니다. keymap.toml에서 바인딩하세요",
                layer, plugin, command, key, by
            ),
            (Lang::En, PluginKeyTaken { plugin, command, layer, key, by_plugin }) => format!(
                "  [{}] {}.{}: default key \"{}\" is already used by plugin {}; bind it in keymap.toml",
                layer, plugin, command, key, by_plugin
            ),
            (Lang::Ko, PluginKeyTaken { plugin, command, layer, key, by_plugin }) => format!(
                "  [{}] {}.{}: 기본 키 \"{}\"를 플러그인 {}가 먼저 씁니다. keymap.toml에서 바인딩하세요",
                layer, plugin, command, key, by_plugin
            ),
        }
    }

    // ── 플러그인 로드 진단 ──────────────────────────────────────────────────

    pub fn plugin_problem_header(&self) -> &'static str {
        match self.lang {
            Lang::En => "plugin problems:",
            Lang::Ko => "플러그인 문제:",
        }
    }

    pub fn plugin_problem(&self, p: &crate::plugin::PluginProblem) -> String {
        use crate::plugin::PluginProblem::*;
        match (&self.lang, p) {
            (Lang::En, Manifest { plugin, detail }) => format!("  {}: plugin.toml: {} (plugin not loaded)", plugin, detail),
            (Lang::Ko, Manifest { plugin, detail }) => format!("  {}: plugin.toml: {} (플러그인을 로드하지 않음)", plugin, detail),
            (Lang::En, BadKey { plugin, command, token }) => format!("  {}.{}: unknown key notation \"{}\"; the default key was dropped", plugin, command, token),
            (Lang::Ko, BadKey { plugin, command, token }) => format!("  {}.{}: 알 수 없는 키 표기 \"{}\". 기본 키를 버렸습니다", plugin, command, token),
            (Lang::En, UnknownLayer { plugin, command, layer }) => format!("  {}.{}: unknown layer \"{}\" ignored", plugin, command, layer),
            (Lang::Ko, UnknownLayer { plugin, command, layer }) => format!("  {}.{}: 알 수 없는 레이어 \"{}\"를 무시했습니다", plugin, command, layer),
            (Lang::En, BadHook { plugin, detail }) => format!("  {}: {} (hook ignored)", plugin, detail),
            (Lang::Ko, BadHook { plugin, detail }) => format!("  {}: {} (훅을 무시했습니다)", plugin, detail),
            (Lang::En, NoManifest { dir }) => format!("  {}: no plugin.toml in this directory", dir),
            (Lang::Ko, NoManifest { dir }) => format!("  {}: 이 디렉토리에 plugin.toml이 없습니다", dir),
            (Lang::En, ConfigWithoutPlugin { name }) => format!("  [plugins.{}] in config.toml, but no such plugin is installed", name),
            (Lang::Ko, ConfigWithoutPlugin { name }) => format!("  config.toml에 [plugins.{}]가 있지만 그 플러그인이 없습니다", name),
            (Lang::En, ExecutableMissing { plugin, program }) => format!("  {}: \"{}\" is not on PATH; the plugin will fail to start", plugin, program),
            (Lang::Ko, ExecutableMissing { plugin, program }) => format!("  {}: \"{}\"가 PATH에 없어 플러그인이 시작되지 않습니다", plugin, program),
            (Lang::En, NameCollidesWithSubcommand { plugin }) => format!("  {}: name collides with a built-in subcommand, so `bibox {}` never reaches the plugin", plugin, plugin),
            (Lang::Ko, NameCollidesWithSubcommand { plugin }) => format!("  {}: 내장 서브커맨드와 이름이 같아 `bibox {}`가 플러그인에 닿지 않습니다", plugin, plugin),
            (Lang::En, ObsoleteGitSetting) => "  config.toml: `git = true` is obsolete; the git-sync plugin now commits on every write when the portable home is a git repository. Remove the line".to_string(),
            (Lang::Ko, ObsoleteGitSetting) => "  config.toml: `git = true`는 더 이상 쓰이지 않습니다. 포터블 홈이 git 저장소이면 git-sync 플러그인이 저장할 때마다 커밋합니다. 그 줄을 지우세요".to_string(),
            (Lang::En, ObsoleteEnabledFlag { name }) => format!("  config.toml: [plugins.{}] enabled is no longer supported; remove the plugin with `bibox plugin remove {}` instead", name, name),
            (Lang::Ko, ObsoleteEnabledFlag { name }) => format!("  config.toml: [plugins.{}]의 enabled는 더 이상 지원하지 않습니다. 대신 `bibox plugin remove {}`로 지우세요", name, name),
            (Lang::En, SettingTypeMismatch { plugin, key, expected, found }) => format!("  config.toml: [plugins.{}] {}: expected {}, found {}", plugin, key, expected, found),
            (Lang::Ko, SettingTypeMismatch { plugin, key, expected, found }) => format!("  config.toml: [plugins.{}] {}: {}이어야 하는데 {}입니다", plugin, key, expected, found),
            (Lang::En, UndeclaredSetting { plugin, key, suggestion }) => match suggestion {
                Some(s) => format!("  config.toml: [plugins.{}] {} is not a setting of {} (did you mean {}?)", plugin, key, plugin, s),
                None => format!("  config.toml: [plugins.{}] {} is not a setting of {}", plugin, key, plugin),
            },
            (Lang::Ko, UndeclaredSetting { plugin, key, suggestion }) => match suggestion {
                Some(s) => format!("  config.toml: [plugins.{}]의 {}는 {}의 설정이 아닙니다 ({}를 말한 건가요?)", plugin, key, plugin, s),
                None => format!("  config.toml: [plugins.{}]의 {}는 {}의 설정이 아닙니다", plugin, key, plugin),
            },
        }
    }

    // ── bibox plugin install / remove ──────────────────────────────────────

    pub fn plugin_install_header(&self, name: &str, source: &str) -> String {
        match self.lang {
            Lang::En => format!("Install \"{}\" from {}?", name, source),
            Lang::Ko => format!("{}에서 \"{}\"를 설치할까요?", source, name),
        }
    }

    pub fn plugin_not_reviewed(&self) -> &'static str {
        match self.lang {
            Lang::En => "  Not in any registry. Nobody has reviewed this code.",
            Lang::Ko => "  어느 레지스트리에도 없습니다. 아무도 이 코드를 검토하지 않았습니다.",
        }
    }

    pub fn plugin_runs(&self, run: &str) -> String {
        match self.lang {
            Lang::En => format!("  Runs: {}", run),
            Lang::Ko => format!("  실행: {}", run),
        }
    }

    pub fn plugin_runs_as_you(&self) -> &'static str {
        match self.lang {
            Lang::En => "  This plugin will run on your machine with your permissions.",
            Lang::Ko => "  이 플러그인은 이 기기에서 사용자의 권한으로 실행됩니다.",
        }
    }

    pub fn plugin_install_question(&self) -> &'static str {
        match self.lang {
            Lang::En => "Install?",
            Lang::Ko => "설치할까요?",
        }
    }

    pub fn plugin_remove_question(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("Remove plugin \"{}\" and its directory?", name),
            Lang::Ko => format!("플러그인 \"{}\"와 그 디렉토리를 지울까요?", name),
        }
    }

    pub fn plugin_installed(&self, name: &str, dir: &str) -> String {
        match self.lang {
            Lang::En => format!("Installed {} at {}", name, dir),
            Lang::Ko => format!("{}를 {}에 설치했습니다", name, dir),
        }
    }

    pub fn plugin_removed(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("Removed {}", name),
            Lang::Ko => format!("{}를 지웠습니다", name),
        }
    }

    pub fn tab_rendering(&self, page: u32) -> String {
        match self.lang {
            Lang::En => format!("rendering page {}...", page),
            Lang::Ko => format!("{}쪽 만드는 중...", page),
        }
    }

    pub fn tab_no_pdf(&self) -> &'static str {
        match self.lang {
            Lang::En => "No PDF attached.\nPress o to fetch or open.",
            Lang::Ko => "PDF가 없습니다.\no로 받거나 엽니다.",
        }
    }

    pub fn tab_no_entry(&self) -> &'static str {
        match self.lang {
            Lang::En => "No entry selected.",
            Lang::Ko => "선택된 항목이 없습니다.",
        }
    }

    pub fn takes_effect_next_start(&self) -> &'static str {
        match self.lang {
            Lang::En => "Saved. Takes effect on next start",
            Lang::Ko => "저장했습니다. 다음 실행부터 적용됩니다",
        }
    }

    pub fn plugin_install_from_title(&self) -> &'static str {
        match self.lang {
            Lang::En => "Install plugin from (owner/repo, git URL, or local path)",
            Lang::Ko => "플러그인 설치 (owner/repo, git URL, 로컬 경로)",
        }
    }

    pub fn plugin_install_failed(&self, detail: &str) -> String {
        match self.lang {
            Lang::En => format!("Install failed: {}", detail),
            Lang::Ko => format!("설치 실패: {}", detail),
        }
    }

    pub fn plugin_cloning(&self, source: &str) -> String {
        match self.lang {
            Lang::En => format!("Cloning {}...", source),
            Lang::Ko => format!("{} 받는 중...", source),
        }
    }

    pub fn plugin_not_found(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("No plugin named \"{}\" (see `bibox plugin list`)", name),
            Lang::Ko => format!("\"{}\"라는 플러그인이 없습니다 (`bibox plugin list` 참조)", name),
        }
    }

    pub fn plugin_no_cli(&self, name: &str) -> String {
        match self.lang {
            Lang::En => format!("Plugin \"{}\" has no [cli] section", name),
            Lang::Ko => format!("플러그인 \"{}\"에 [cli] 절이 없습니다", name),
        }
    }
}
