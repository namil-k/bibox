# bibox

PDF 기반 bibliography 관리자. 논문 PDF를 추가하면 DOI로 메타데이터를 자동 조회하고, BibTeX 키를 생성해서 로컬에 정리한다. TUI와 CLI 모두 지원.

---

## 설치

```bash
cargo install --path .
```

빌드 의존성: `libssl-dev`, `pkg-config` (Ubuntu: `sudo apt install libssl-dev pkg-config`)

---

## 빠른 시작

```bash
# PDF 추가 (DOI 자동 조회)
bibox add paper.pdf

# DOI만으로 추가 (PDF 없이)
bibox add --doi 10.1145/3290605.3300907

# TUI 열기 (인수 없이 실행)
bibox
```

---

## 저장 위치

| 항목 | 경로 |
|------|------|
| PDF 파일 | `~/Documents/bibox/` |
| 메타데이터 DB | `~/.local/share/bibox/db.json` |
| 설정 파일 | `~/.config/bibox/config.toml` |

---

## CLI 커맨드

### `add` — 항목 추가

```bash
bibox add <file.pdf>
bibox add --doi 10.xxxx/xxxxx
bibox add paper.pdf --to ml --key kim2024
bibox add paper.pdf --title "Override Title" --author "Kim, N; Lee, S" --year 2024
```

- PDF에서 DOI 자동 추출 → Crossref로 메타데이터 조회
- DOI 없으면 arXiv 제목 검색 (인터랙티브 선택)
- `--to`: 컬렉션 지정 (생략 시 `default_collection` config 값 사용)

### `list` — 목록 보기

```bash
bibox list                        # 컬렉션 요약
bibox list ml                     # 특정 컬렉션
bibox list --type article         # 타입 필터
bibox list --tag transformer      # 태그 필터
bibox list --year 2024            # 연도 필터
bibox list ml --limit 10
```

### `search` — 인터랙티브 검색

```bash
bibox search "attention mechanism"
bibox search "transformer" --collection ml
bibox search "kim" --field author
```

Enter로 선택하면 citekey를 클립보드에 복사.

### `show` — 상세 보기

```bash
bibox show kim2024attention
```

### `edit` — 메타데이터 편집

```bash
bibox edit kim2024 --title "New Title"
bibox edit kim2024 --tags-add "survey,transformer" --tags-remove "old"
bibox edit kim2024 --year 2025 --journal "Nature"
```

메타데이터 변경 시 PDF 파일명도 자동으로 변경됨.

### `meta` — DOI로 메타데이터 재조회

```bash
bibox meta kim2024 --doi 10.1145/3290605.3300907
bibox meta kim2024 --title "Corrected Title"   # 수동 수정
```

Crossref에서 가져온 메타데이터로 덮어씀. 파일명도 자동 변경.

### `open` — PDF 열기

```bash
bibox open kim2024attention
```

`pdf_viewer` config가 설정되어 있으면 해당 앱으로, 없으면 `xdg-open` (Linux) / `open` (macOS).

### `collect` / `uncollect` — 컬렉션 관리

```bash
bibox collect kim2024 ml systems
bibox uncollect kim2024 systems
```

### `import` — .bib 파일 가져오기

```bash
bibox import references.bib
bibox import references.bib --to imported
```

DOI 기준으로 중복 체크. 중복 항목은 빈 필드만 병합.

### `out` — BibTeX / PDF 내보내기

```bash
bibox out --collection ml                      # ml 컬렉션 → .bib 파일
bibox out --key kim2024 --clipboard            # 클립보드로 복사
bibox out --collection ml --as-pdf             # PDF 파일들 복사
bibox out --collection ml --as-pdf --zip       # ZIP 압축
bibox out --output refs.bib                    # 출력 경로 지정
```

### `sync` — 파일 시스템 동기화

```bash
bibox sync
```

- DB에 등록됐지만 파일이 없는 항목: 확인 후 DB에서 삭제
- 파일이 있지만 DB에 없는 항목: 확인 후 추가
- 메타데이터와 파일명이 다른 항목: 자동으로 파일명 수정

---

## TUI

`bibox`를 인수 없이 실행하면 TUI가 열림.

```
┌─ bibox ──────────────────────────────────────────────┐
│  All  ml  rust  systems                              │
├──────────────────────────────────────────────────────┤
│▶ [kim2024attention] Attention Is All You Need  [pdf] │
│  article | Vaswani et al. | 2017 | transformer, ml   │
│                                                      │
│  [park2023rl] Deep RL Survey                  [pdf]  │
│  article | Park | 2023 | rl, survey                  │
├──────────────────────────────────────────────────────┤
│ j/k ↑↓  Tab: tab  /: search  Enter: detail  q: quit │
└──────────────────────────────────────────────────────┘
```

### 키바인딩

| 키 | 동작 |
|----|------|
| `j` / `k` / ↑↓ | 목록 이동 |
| `Tab` / `l` / `h` | 컬렉션 탭 전환 |
| `/` | 인라인 검색 (title/author/key/tag) |
| `Esc` | 검색 초기화 |
| `Enter` | 상세 팝업 |
| `y` | citekey 클립보드 복사 |
| `p` | PDF 열기 |
| `o` | 현재 컬렉션 .bib 내보내기 |
| `d` | 삭제 (y/n 확인) |
| `q` / `Ctrl+C` | 종료 |

---

## 설정

`~/.config/bibox/config.toml`:

```toml
# PDF 저장 디렉터리
bibox_dir = "/home/user/Documents/bibox"

# PDF 뷰어 (없으면 xdg-open / open)
pdf_viewer = "zathura"

# add --to 생략 시 기본 컬렉션
default_collection = "inbox"

# 검색 대소문자 구분
search_case_sensitive = false

# 기본 페이지 크기
default_page_size = 20

# 언어: "en" 또는 "ko"
language = "en"
```

---

## 파일명 규칙

PDF는 `<firstauthor><year><firstword>.pdf` 형식으로 자동 저장됨.

예) `kim2024attention.pdf`

`edit` / `meta` 로 메타데이터 변경 시 파일명도 자동으로 맞춰짐.

---

## 의존 크레이트

| 크레이트 | 용도 |
|----------|------|
| clap | CLI 파싱 |
| ratatui + crossterm | TUI |
| reqwest + tokio | Crossref / Unpaywall / arXiv API |
| lopdf | PDF에서 DOI 추출 |
| arboard | 클립보드 |
| serde_json | DB 직렬화 |
| toml | 설정 파일 |
| zip | PDF ZIP 내보내기 |
