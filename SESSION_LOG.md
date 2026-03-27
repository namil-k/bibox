## 세션 로그

### 2026-03-27

**상태:** CLI 리팩토링 + 노트 시스템 구현 완료

**완료:**

- [x] PLAN.md + PLAN_1.md 합병 → 단일 PLAN.md로 최신화
- [x] 노트 시스템 디자인 스펙 작성 (`docs/superpowers/specs/2026-03-25-ai-agent-notes-design.md`)
- [x] 노트 시스템 구현 플랜 작성 (`docs/superpowers/plans/2026-03-25-ai-agent-notes.md`)
- [x] CLI 리팩토링 디자인 스펙 작성 (`docs/superpowers/specs/2026-03-27-cli-refactor-design.md`)
- [x] CLI 리팩토링 구현 플랜 작성 (`docs/superpowers/plans/2026-03-27-cli-refactor.md`)
- [x] CLI 리팩토링 구현 (7커밋)
  - [x] `scraper` + `urlencoding` 의존성 추가
  - [x] `url_resolver.rs` 모듈 생성 (URL→DOI 패턴매칭 + HTML 메타태그 파싱, 9개 단위 테스트)
  - [x] `crossref.rs`에 `search_by_title` 함수 추가 (Crossref bibliographic search)
  - [x] i18n 메시지 업데이트 (`meta` 참조 제거, `--url`/`--search` 메시지 추가)
  - [x] `out` → `export` 리네임 (`out`은 alias로 유지)
  - [x] `meta` → `edit` 흡수 (`cmd_edit` async화, preserve-on-None 시멘틱)
  - [x] `cmd_add`에 `--url`, `--search` 플래그 추가

- [x] 노트 시스템 구현 (6커밋)
  - [x] `notes.rs` 모듈 생성 (섹션 파서 + 템플릿 엔진, 8개 단위 테스트)
  - [x] `Commands::Note` CLI 플래그 확장 (--stdin, --from, --section, --template, --show, --path, --force)
  - [x] `templates_dir` config 필드 + 노트 관련 i18n 메시지 7개 추가
  - [x] `cmd_note` 전면 재작성 (템플릿→콘텐츠→에디터 순서 보장)
  - [x] 12개 스모크 테스트 전체 통과

- [x] TUI Phase 1 전체 구현 (5커밋)
  - [x] 키바인딩 리맵: `e`=export, `o`=open PDF, `p` freed
  - [x] Help 화면 (`?`) — 전체 키바인딩 오버레이
  - [x] Note 보기/편집 (`n`=읽기 팝업, `N`=$EDITOR 실행, TUI suspend/resume)
  - [x] Sort 메뉴 (`s`) — Year/Author/Title/Created, None은 항상 마지막, Esc=revert
  - [x] Collection 관리 (`c`) + Tag 편집 (`t`) — ChecklistPicker 공유 컴포넌트, 체크박스 토글 + 새 항목 생성

- [x] TUI 대규모 리팩토링 (1커밋, +1352/-778 lines)
  - [x] 3-panel yazi 스타일 레이아웃 (좌: 컬렉션, 중: 항목, 우: 프리뷰)
  - [x] 프리뷰 패널 Tab 순환: Info / Note / PDF (pdftotext fallback)
  - [x] h/l 패널 네비게이션, j/k 패널 내 이동
  - [x] vim 이동: gg, G, H/M/L, {n}j/k, Ctrl+d/u
  - [x] 줄번호 (absolute/relative/none, config 연동)
  - [x] 멀티셀렉트: Space 토글, V 전체선택, Esc 해제, 녹색 배경 하이라이트
  - [x] Export 팝업 (`e`): scope(selected/collection/all) + format(bib/yaml/ris) + include PDFs
  - [x] Settings 팝업 (`,`): line_numbers, panel_ratio, bib_export_dir, export_dir
  - [x] PDF fetch 스피너 (백그라운드 스레드, TUI 내 braille 애니메이션)
  - [x] CLI export 개선: positional keys, --include-pdf, --key 제거
  - [x] config 확장: line_numbers, panel_ratio, bib_export_dir, export_dir
  - [x] User-Agent 헤더 추가 (Unpaywall 403 대응)
  - [x] Help 화면 전체 키바인딩 반영
  - [x] 상태바에 `, settings` 힌트 추가

**다음:**

- [ ] `bibox init <path>` — portable home 디렉토리 (db.json + notes + pdfs 한 폴더)
- [ ] Settings에 home 경로 읽기전용 표시
- [ ] TUI Phase 2: Fuzzy search (nucleo + ratatui)
- [ ] TUI Phase 3: Add entry from TUI (DOI/URL 입력 폼)
- [ ] PDF 프리뷰 Kitty graphics protocol 지원
- [ ] Shell completion (zsh, 동적 citekey 완성)
- [ ] TUI 디자인 개선 (bubbletea 스타일)

**결정 사항:**

- **`--url` 해석 범위: Tier 1 + Tier 2** (대안: Tier 1만, Tier 3까지)
  - 이유: Tier 1(패턴매칭)으로 arxiv/doi.org/ACM/Springer/Nature 커버, Tier 2(HTML 메타태그)로 나머지 학술 사이트 대부분 커버. 비용 대비 효과 최대
  - Tier 1만: IEEE, OpenReview 등 커버 못함
  - Tier 3(Semantic Scholar API 등): 별도 API 연동 필요, ROI 낮음. 나중에 추가 가능
  - 참고: 대부분의 학술 사이트가 citation_doi HTML 메타태그를 제공

- **`--search`에 `query.bibliographic` 사용** (대안: `query.title`)
  - 이유: free-text 검색에 더 robust (제목+저자+키워드 매칭)
  - `query.title`은 정확한 제목 매칭에만 효과적

- **`out` → `export` (alias 유지)** (대안: `out` 즉시 제거)
  - 이유: 기존 스크립트/습관 호환성. clap `#[command(alias = "out")]` 한 줄로 해결

- **`meta` 즉시 제거 (deprecation 없음)** (대안: deprecation 경고 후 유지)
  - 이유: 초기 단계 프로젝트, 외부 사용자 없음. `edit --doi`가 완전한 상위호환

- **`collect`/`uncollect` 이름 유지** (대안: `collect --remove`, `drop`, `file/unfile`, `remove`)
  - 이유: 대안들을 검토했으나 어느 것도 기존보다 명확하게 나은 게 없음
  - `collect --remove`: collect가 그룹 커맨드가 되면 구조가 복잡해짐
  - `drop`: delete와 혼동 가능성
  - `remove`: delete와 혼동 가능성 (두 번째 인자로 구분되지만 직관적이지 않음)

- **preserve-on-None 채택** (대안: wipe-on-None)
  - 이유: Crossref가 일부 필드를 반환하지 않을 때 기존 DB 값을 보존해야 데이터 손실 방지
  - 패턴: `entry.field = cli_flag.or(crossref_value).or(entry.field.take())`

- **LLM 통합 방향 B 채택 (bibox는 저장소, AI는 외부)** (대안: A-내장, C-얇은 래퍼)
  - 이유: Unix 철학 (한 가지를 잘 함), 어떤 AI 모델이든 파이프로 연결 가능
  - A(내장): API 키 관리, 모델 종속성, 복잡도 급증
  - C(래퍼): 향후 검토 가능하지만 현재 불필요

- **노트를 md 파일로 저장 (DB가 아닌 파일시스템)** (대안: DB에 note 필드로 저장)
  - 이유: AI 에이전트가 `--show`로 읽고 `--stdin --section`으로 쓰기 용이, 사람은 에디터로 직접 편집 가능
  - DB 저장: 검색은 편하지만 외부 도구와의 연동이 불편

- **3-panel yazi 스타일 TUI 채택** (대안: 기존 탭 + 리스트 레이아웃)
  - 이유: 컬렉션/항목/프리뷰를 동시에 볼 수 있어 정보 밀도 높음
  - panel_ratio를 config로 조절 가능 (yazi와 동일한 비율 방식)

- **Export를 팝업 메뉴로 통합** (대안: `e`로 바로 export)
  - 이유: scope(selected/collection/all) + format + include PDFs 선택이 한 화면에서 가능
  - Zotero의 Export 워크플로우 참고
  - CLI도 맞춰서 positional keys + --include-pdf 지원

- **bibox init으로 portable home 지원 예정** (대안: DB 경로만 설정)
  - 이유: GitHub 동기화를 위해 db.json + notes + pdfs가 한 폴더에 있어야 함
  - config.toml에 `home = "~/bibox"` 기록, 어디서든 해당 home 사용
  - Settings에서는 읽기전용 표시, 변경은 `bibox init <새경로>`로만

- **줄번호 기본값 absolute** (대안: relative, none)
  - 이유: 일반 사용자에게 가장 익숙. vim 사용자는 Settings에서 relative로 변경 가능

- **PDF 프리뷰: pdftotext fallback, Kitty protocol은 추후** (대안: chafa, 이미지 없이 텍스트만)
  - 이유: pdftotext가 가장 가볍고 의존성 최소. Ghostty/kitty 지원은 다음 단계
  - Kitty graphics protocol로 실제 이미지 렌더링 가능 (Ghostty 지원 확인됨)

---
