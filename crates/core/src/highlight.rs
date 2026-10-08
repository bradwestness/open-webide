//! A small, dependency-free syntax highlighter.
//!
//! It tokenizes source text into colored [`Token`]s for a handful of common
//! languages so the in-browser editor can render a highlighted overlay behind a
//! transparent textarea. The tokenizer is pure and natively unit-testable; the
//! frontend turns the tokens into HTML spans.
//!
//! The guarantee that makes the overlay work: for each line, concatenating the
//! token texts reproduces that line exactly. No character is dropped or
//! altered, so the colored text stays pixel-aligned with the raw text.

/// The syntactic category of a token, used to pick a display color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TokenKind {
    /// Whitespace, identifiers, and anything unclassified.
    Plain,
    Keyword,
    Type,
    String,
    Char,
    Comment,
    Number,
    Function,
    Operator,
    Punct,
    Attribute,
    Macro,
    Lifetime,
    Boolean,
}

/// One token: a run of source text plus its category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    pub kind: TokenKind,
    pub text: String,
}

/// Immutable tokens for one source row.
pub type TokenRow = std::sync::Arc<[Token]>;
/// Token rows shared by lexical updates and editor paint consumers.
pub type TokenRows = Vec<TokenRow>;

/// Transfer owned tokens into compact immutable rows without cloning their text.
pub fn share_token_rows(rows: Vec<Vec<Token>>) -> TokenRows {
    rows.into_iter().map(std::sync::Arc::from).collect()
}

/// The language a file is written in, chosen from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    Jsx,
    TypeScript,
    Tsx,
    Java,
    CSharp,
    Php,
    Json,
    Html,
    Css,
    Markdown,
    Shell,
    Toml,
    Yaml,
    Sql,
    C,
    Cpp,
    Go,
    Plain,
}

/// Pick a language from a file path's extension.
pub fn language_from_path(path: &str) -> Language {
    let ext = crate::file_type::extension(path).unwrap_or_default();
    match ext.as_str() {
        "rs" => Language::Rust,
        "py" => Language::Python,
        "js" | "mjs" | "cjs" => Language::JavaScript,
        "jsx" => Language::Jsx,
        "ts" | "mts" | "cts" => Language::TypeScript,
        "tsx" => Language::Tsx,
        "java" => Language::Java,
        "cs" => Language::CSharp,
        "php" | "phtml" => Language::Php,
        "json" => Language::Json,
        "html" | "htm" | "xml" | "svg" => Language::Html,
        "css" | "scss" => Language::Css,
        "md" | "markdown" => Language::Markdown,
        "sh" | "bash" | "zsh" => Language::Shell,
        "toml" => Language::Toml,
        "yaml" | "yml" => Language::Yaml,
        "sql" => Language::Sql,
        "c" | "h" => Language::C,
        "cpp" | "cc" | "cxx" | "hpp" => Language::Cpp,
        "go" => Language::Go,
        _ => Language::Plain,
    }
}

/// Lines longer than this are rendered as a single unhighlighted [`Token`]
/// rather than tokenized, so minified files can't spend unbounded time in the
/// per-character tokenizer loops.
pub const MAX_HIGHLIGHT_LINE_BYTES: usize = 10_000;

/// Tokenize `source` line by line into colored tokens.
///
/// The returned lines line up with `source.split('\n')`, and concatenating a
/// line's token texts reproduces that line exactly.
pub fn highlight_lines(source: &str, language: Language) -> Vec<Vec<Token>> {
    highlight_lines_while(source, language, || true)
        .expect("unconditional highlighting cannot cancel")
}

/// Preserve multiline lexical state, publishing no partial paint after cancellation.
pub fn highlight_lines_while(
    source: &str,
    language: Language,
    mut should_continue: impl FnMut() -> bool,
) -> Option<Vec<Vec<Token>>> {
    let mut lines = Vec::new();
    let mut state = State::Normal;
    for line in source.split('\n') {
        if !should_continue() {
            return None;
        }
        let (tokens, next_state) = highlight_line(line, language, state);
        state = next_state;
        lines.push(tokens);
    }
    Some(lines)
}

fn highlight_line(line: &str, language: Language, state: State) -> (Vec<Token>, State) {
    if line.len() > MAX_HIGHLIGHT_LINE_BYTES {
        return (
            vec![Token {
                kind: TokenKind::Plain,
                text: line.to_string(),
            }],
            state,
        );
    }
    match language {
        Language::Rust => highlight_rust_line(line, state),
        _ => highlight_generic_line(line, language, state),
    }
}

pub const LEXICAL_BATCH_ROWS: usize = 128;
pub const LEXICAL_BATCH_BYTES: usize = 64 * 1024;
pub const LEXICAL_BATCHES_PER_FRAME: usize = 8;

/// Yield rendering after a short preparation interval, rather than imposing a
/// frame wait on every few cheap retained-row batches. Invalid clocks retain the
/// conservative fixed-batch schedule; the hard cap also bounds fast-clock work.
pub fn lexical_frame_due(batches: usize, elapsed_ms: f64) -> bool {
    if !elapsed_ms.is_finite() || elapsed_ms < 0.0 {
        return batches >= LEXICAL_BATCHES_PER_FRAME;
    }
    elapsed_ms >= 4.0 || batches >= 64
}

/// Complete contextual lexical paint and the row states needed for exact reuse.
#[derive(Debug)]
pub struct LexicalSnapshot {
    source: std::sync::Arc<String>,
    language: Language,
    normalize_crlf: bool,
    rows: std::sync::Arc<Vec<LexicalRow>>,
    tokens: std::sync::Arc<TokenRows>,
    retokenized_rows: usize,
}
impl LexicalSnapshot {
    pub fn tokens(&self) -> &std::sync::Arc<TokenRows> {
        &self.tokens
    }
    pub const fn retokenized_rows(&self) -> usize {
        self.retokenized_rows
    }
}

#[derive(Debug)]
struct LexicalRow {
    start: usize,
    end: usize,
    before: State,
    after: State,
}

/// A source-owned lexical job that preserves multiline state across cooperative
/// batches. Callers can discard it on cancellation; unfinished paint is never
/// returned by finish. Budgets count whole rows, allowing one oversized row.
pub struct LexicalPreparation {
    source: std::sync::Arc<String>,
    language: Language,
    normalize_crlf: bool,
    next: usize,
    state: State,
    rows: TokenRows,
    contexts: Vec<LexicalRow>,
    previous: Option<std::sync::Arc<LexicalSnapshot>>,
    retokenized_rows: usize,
    complete: bool,
    unchanged: bool,
    source_change: Option<crate::editor::TextChange>,
    previous_row: usize,
    #[cfg(test)]
    indexed_searches: std::cell::Cell<usize>,
    #[cfg(test)]
    boundary_scans: usize,
}
impl LexicalPreparation {
    pub fn new(source: std::sync::Arc<String>, language: Language) -> Self {
        Self {
            source,
            language,
            normalize_crlf: false,
            next: 0,
            state: State::Normal,
            rows: Vec::new(),
            contexts: Vec::new(),
            previous: None,
            retokenized_rows: 0,
            complete: false,
            unchanged: false,
            source_change: None,
            previous_row: 0,
            #[cfg(test)]
            indexed_searches: std::cell::Cell::new(0),
            #[cfg(test)]
            boundary_scans: 0,
        }
    }
    /// Match native textarea newline normalization without copying full source.
    pub fn for_textarea(source: std::sync::Arc<String>, language: Language) -> Self {
        Self {
            normalize_crlf: true,
            ..Self::new(source, language)
        }
    }
    /// Reuse only exact raw rows with the same incoming lexical state. Check both
    /// unchanged offsets and the total byte shift, validating every candidate;
    /// insertions, deletions and disjoint edits cannot reuse mismatching context.
    pub fn reuse(mut self, previous: std::sync::Arc<LexicalSnapshot>) -> Self {
        if previous.language == self.language && previous.normalize_crlf == self.normalize_crlf {
            self.next = 0;
            self.previous_row = 0;
            self.state = State::Normal;
            self.rows.clear();
            self.contexts.clear();
            self.retokenized_rows = 0;
            self.complete = false;
            self.unchanged = false;
            self.source_change = if std::sync::Arc::ptr_eq(&self.source, &previous.source) {
                None
            } else {
                crate::editor::text_change(&previous.source, &self.source)
            };
            if self.source_change.is_none() {
                self.unchanged = true;
                self.complete = true;
            }
            self.previous = Some(previous);
        }
        self
    }
    pub const fn is_complete(&self) -> bool {
        self.complete
    }
    /// Whole rows outside the validated replacement retain their raw boundaries.
    /// Terminal rows may only be reused when they still terminate the new source.
    fn indexed_row(&self) -> Option<(usize, usize, bool)> {
        let previous = self.previous.as_ref()?;
        let change = self.source_change.as_ref()?;
        let prefix = self.next < change.range.start;
        let old_start = if prefix {
            self.next
        } else if self.next >= change.new_end {
            self.next
                .checked_sub(change.new_end)?
                .checked_add(change.range.end)?
        } else {
            return None;
        };
        let index = if previous
            .rows
            .get(self.previous_row)
            .is_some_and(|row| row.start == old_start)
        {
            self.previous_row
        } else {
            #[cfg(test)]
            self.indexed_searches.set(self.indexed_searches.get() + 1);
            previous
                .rows
                .binary_search_by_key(&old_start, |row| row.start)
                .ok()?
        };
        let row = &previous.rows[index];
        let end = if prefix {
            if row.end > change.range.start {
                return None;
            }
            row.end
        } else {
            row.end
                .checked_sub(change.range.end)?
                .checked_add(change.new_end)?
        };
        let newline = row.end > row.start && previous.source.as_bytes()[row.end - 1] == b'\n';
        if !newline && end != self.source.len() {
            return None;
        }
        Some((index, end, newline))
    }

    fn reusable_row(&self, end: usize) -> Option<usize> {
        let previous = self.previous.as_ref()?;
        let shifted = if self.source.len() >= previous.source.len() {
            self.next
                .checked_sub(self.source.len() - previous.source.len())
        } else {
            self.next
                .checked_add(previous.source.len() - self.source.len())
        };
        [Some(self.next), shifted]
            .into_iter()
            .flatten()
            .find_map(|start| {
                let index = previous
                    .rows
                    .binary_search_by_key(&start, |row| row.start)
                    .ok()?;
                let row = &previous.rows[index];
                (row.before == self.state
                    && previous.source[row.start..row.end] == self.source[self.next..end])
                    .then_some(index)
            })
    }
    pub fn advance(&mut self, max_rows: usize, max_bytes: usize) -> usize {
        let mut count = 0;
        let mut bytes = 0;
        if max_bytes == 0 {
            return 0;
        }
        while !self.complete && count < max_rows {
            let indexed = self.indexed_row();
            let (next, newline) = indexed.map_or_else(
                || {
                    #[cfg(test)]
                    {
                        self.boundary_scans += 1;
                    }
                    let tail = &self.source[self.next..];
                    let newline = tail.find('\n');
                    (
                        self.next + newline.map_or(tail.len(), |end| end + 1),
                        newline.is_some(),
                    )
                },
                |(_, next, newline)| (next, newline),
            );
            let end = next - usize::from(newline);
            let raw = &self.source[self.next..end];
            let cost = next - self.next;
            if count > 0 && cost > max_bytes.saturating_sub(bytes) {
                break;
            }
            let line = if self.normalize_crlf && newline {
                raw.strip_suffix('\r').unwrap_or(raw)
            } else {
                raw
            };
            let reusable = indexed.map_or_else(
                || self.reusable_row(next),
                |(index, _, _)| {
                    self.previous
                        .as_ref()
                        .filter(|previous| previous.rows[index].before == self.state)
                        .map(|_| index)
                },
            );
            let (tokens, state) = if let Some(index) = reusable {
                let previous = self.previous.as_ref().expect("matched previous row");
                (previous.tokens[index].clone(), previous.rows[index].after)
            } else {
                self.retokenized_rows += 1;
                let (tokens, state) = highlight_line(line, self.language, self.state);
                (std::sync::Arc::from(tokens), state)
            };
            // Exact source boundaries remain valid even when multiline state
            // requires fresh tokens. Changed/new rows recover via indexed lookup.
            if let Some(index) = indexed.map(|(index, _, _)| index).or(reusable) {
                self.previous_row = index + 1;
            }
            self.contexts.push(LexicalRow {
                start: self.next,
                end: next,
                before: self.state,
                after: state,
            });
            self.rows.push(tokens);
            self.state = state;
            self.complete = !newline;
            self.next = next;
            count += 1;
            bytes += cost;
        }
        count
    }
    pub fn finish(self) -> Option<TokenRows> {
        if !self.complete {
            return None;
        }
        Some(if self.unchanged {
            self.previous
                .expect("validated unchanged source")
                .tokens
                .as_ref()
                .clone()
        } else {
            self.rows
        })
    }
    pub fn finish_snapshot(self) -> Option<LexicalSnapshot> {
        if self.unchanged {
            let previous = self.previous.expect("validated unchanged source");
            return Some(LexicalSnapshot {
                source: self.source,
                language: self.language,
                normalize_crlf: self.normalize_crlf,
                rows: previous.rows.clone(),
                tokens: previous.tokens.clone(),
                retokenized_rows: 0,
            });
        }
        self.complete.then_some(LexicalSnapshot {
            source: self.source,
            language: self.language,
            normalize_crlf: self.normalize_crlf,
            rows: std::sync::Arc::new(self.contexts),
            tokens: std::sync::Arc::new(self.rows),
            retokenized_rows: self.retokenized_rows,
        })
    }
}

/// Tokenizer state carried across lines (a block comment may span lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum State {
    Normal,
    BlockComment,
}

fn is_operator(c: char) -> bool {
    matches!(
        c,
        '+' | '-' | '*' | '/' | '=' | '<' | '>' | '!' | '&' | '|' | '^' | '~' | '?'
    )
}

fn is_punct(c: char) -> bool {
    matches!(c, '.' | ',' | ';' | ':' | '(' | ')' | '{' | '}' | '[' | ']')
}

/// A char that can join a plain (uncolored) run: whitespace, or anything that
/// does not start a recognized token.
fn is_plain_char(c: char) -> bool {
    c.is_whitespace()
        || !(c.is_alphanumeric()
            || c == '_'
            || c == '"'
            || c == '\''
            || c == '`'
            || c == '#'
            || is_operator(c)
            || is_punct(c))
}

/// Read a string literal starting at `rest` (which begins with `quote`),
/// honoring backslash escapes. Returns the literal text and the byte length
/// consumed. An unterminated string runs to the end of the line.
fn read_string(rest: &str, quote: char) -> (String, usize) {
    let mut indices = rest.char_indices();
    let mut end = match indices.next() {
        Some((_, c)) => c.len_utf8(),
        None => 0,
    };
    let mut escaped = false;
    for (idx, c) in indices {
        end = idx + c.len_utf8();
        if escaped {
            escaped = false;
            continue;
        }
        if c == '\\' {
            escaped = true;
        } else if c == quote {
            break;
        }
    }
    (rest[..end].to_string(), end)
}

/// Read an identifier (letters, digits, underscores) starting at `rest`.
fn read_ident(rest: &str) -> (String, usize) {
    let mut text = String::new();
    let mut consumed = 0usize;
    for c in rest.chars() {
        if c.is_alphanumeric() || c == '_' {
            text.push(c);
            consumed += c.len_utf8();
        } else {
            break;
        }
    }
    (text, consumed)
}

/// Read a number literal starting at `rest` (which begins with a digit):
/// optional `0x`/`0o`/`0b` prefix, integer part, optional fraction, optional
/// exponent. Returns the literal text and the byte length consumed.
fn read_number(rest: &str) -> (String, usize) {
    let bytes = rest.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    // `0x` / `0o` / `0b` prefix.
    if i < len && bytes[i] == b'0' {
        i += 1;
        if i < len && matches!(bytes[i], b'x' | b'X' | b'o' | b'O' | b'b' | b'B') {
            i += 1;
            while i < len && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                i += 1;
            }
            return (rest[..i].to_string(), i);
        }
    }

    // Integer part.
    while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
        i += 1;
    }
    // Fractional part (only if a digit follows the dot, so `1..2` stays a range).
    if i + 1 < len && bytes[i] == b'.' && bytes[i + 1].is_ascii_digit() {
        i += 1;
        while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }
    // Exponent.
    if i < len && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < len && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        while i < len && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
            i += 1;
        }
    }

    (rest[..i].to_string(), i)
}

/// Classify a Rust identifier using the text that follows it.
fn classify_rust_ident(word: &str, after: &str) -> TokenKind {
    if word == "true" || word == "false" {
        return TokenKind::Boolean;
    }
    if RUST_KEYWORDS.contains(&word) {
        return TokenKind::Keyword;
    }
    if after.starts_with('!') {
        return TokenKind::Macro;
    }
    if after.starts_with('(') {
        return TokenKind::Function;
    }
    if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return TokenKind::Type;
    }
    TokenKind::Plain
}

/// Read a Rust char literal (`'a'`, `'\n'`) or lifetime (`'a`) starting at
/// `rest` (which begins with `'`). Returns `None` when the quote does not begin
/// either (a stray quote), in which case the caller treats it as plain text.
fn read_char_or_lifetime(rest: &str) -> Option<(Token, usize)> {
    let first = rest.chars().next()?;
    if first != '\'' {
        return None;
    }

    let sample: Vec<char> = rest.chars().take(4).collect();
    if sample.len() < 2 {
        return Some((
            Token {
                kind: TokenKind::Lifetime,
                text: "'".to_string(),
            },
            first.len_utf8(),
        ));
    }

    let second = sample[1];
    if second == '\\' {
        // Escaped char literal, e.g. `'\n'` or `'\''`.
        if sample.len() >= 4 && sample[3] == '\'' {
            let consumed: usize = sample.iter().take(4).map(|c| c.len_utf8()).sum();
            return Some((
                Token {
                    kind: TokenKind::Char,
                    text: rest[..consumed].to_string(),
                },
                consumed,
            ));
        }
        return None;
    }
    if second == '\'' {
        // `''` is not valid Rust; treat the lone quote as a lifetime.
        return Some((
            Token {
                kind: TokenKind::Lifetime,
                text: "'".to_string(),
            },
            first.len_utf8(),
        ));
    }
    // A char literal when the third char is a closing quote.
    if sample.len() >= 3 && sample[2] == '\'' {
        let consumed: usize = sample.iter().take(3).map(|c| c.len_utf8()).sum();
        return Some((
            Token {
                kind: TokenKind::Char,
                text: rest[..consumed].to_string(),
            },
            consumed,
        ));
    }
    // Otherwise a lifetime: `'ident`. Iterate lazily; identifiers are short,
    // so this never collects the rest of the line.
    let mut text = String::from("'");
    let mut consumed = first.len_utf8();
    for c in rest[first.len_utf8()..].chars() {
        if c.is_alphanumeric() || c == '_' {
            text.push(c);
            consumed += c.len_utf8();
        } else {
            break;
        }
    }
    Some((
        Token {
            kind: TokenKind::Lifetime,
            text,
        },
        consumed,
    ))
}

fn highlight_rust_line(line: &str, state: State) -> (Vec<Token>, State) {
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let len = line.len();
    let mut state = state;

    while i < len {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();

        if state == State::BlockComment {
            match rest.find("*/") {
                Some(end) => {
                    let seg = &rest[..end + "*/".len()];
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: seg.to_string(),
                    });
                    i += seg.len();
                    state = State::Normal;
                }
                None => {
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: rest.to_string(),
                    });
                    i = len;
                }
            }
            continue;
        }

        if rest.starts_with("//") {
            tokens.push(Token {
                kind: TokenKind::Comment,
                text: rest.to_string(),
            });
            i = len;
            continue;
        }

        if let Some(inner) = rest.strip_prefix("/*") {
            match inner.find("*/") {
                Some(end) => {
                    let seg = &rest[..end + "/*".len() + "*/".len()];
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: seg.to_string(),
                    });
                    i += seg.len();
                }
                None => {
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: rest.to_string(),
                    });
                    i = len;
                    state = State::BlockComment;
                }
            }
            continue;
        }

        if c == '"' {
            let (text, consumed) = read_string(rest, '"');
            tokens.push(Token {
                kind: TokenKind::String,
                text,
            });
            i += consumed;
            continue;
        }

        if c == '\''
            && let Some((tok, consumed)) = read_char_or_lifetime(rest)
        {
            tokens.push(tok);
            i += consumed;
            continue;
        }

        if c.is_ascii_digit() {
            let (text, consumed) = read_number(rest);
            tokens.push(Token {
                kind: TokenKind::Number,
                text,
            });
            i += consumed;
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let (word, consumed) = read_ident(rest);
            let after = &rest[consumed..];
            let kind = classify_rust_ident(&word, after);
            tokens.push(Token { kind, text: word });
            i += consumed;
            continue;
        }

        if c == '#' {
            tokens.push(Token {
                kind: TokenKind::Attribute,
                text: "#".to_string(),
            });
            i += 1;
            continue;
        }

        if is_operator(c) {
            let mut j = i + c.len_utf8();
            for ch in line[j..].chars() {
                if !is_operator(ch) {
                    break;
                }
                j += ch.len_utf8();
            }
            tokens.push(Token {
                kind: TokenKind::Operator,
                text: line[i..j].to_string(),
            });
            i = j;
            continue;
        }

        if is_punct(c) {
            tokens.push(Token {
                kind: TokenKind::Punct,
                text: c.to_string(),
            });
            i += 1;
            continue;
        }

        let mut j = i + c.len_utf8();
        for ch in line[j..].chars() {
            if !is_plain_char(ch) {
                break;
            }
            j += ch.len_utf8();
        }
        tokens.push(Token {
            kind: TokenKind::Plain,
            text: line[i..j].to_string(),
        });
        i = j;
    }

    (tokens, state)
}

/// Per-language parameters for the generic (non-Rust) highlighter: the line
/// comment prefix, the block comment delimiters, and the keyword/boolean sets.
type LangParams = (
    Option<&'static str>,
    Option<(&'static str, &'static str)>,
    &'static [&'static str],
    &'static [&'static str],
);

fn lang_params(language: Language) -> LangParams {
    match language {
        Language::JavaScript | Language::Jsx => {
            (Some("//"), Some(("/*", "*/")), JS_KEYWORDS, JS_BOOLS)
        }
        Language::TypeScript | Language::Tsx => {
            (Some("//"), Some(("/*", "*/")), TS_KEYWORDS, TS_BOOLS)
        }
        Language::Java => (Some("//"), Some(("/*", "*/")), JAVA_KEYWORDS, C_BOOLS),
        Language::CSharp => (Some("//"), Some(("/*", "*/")), CSHARP_KEYWORDS, C_BOOLS),
        Language::Php => (Some("//"), Some(("/*", "*/")), PHP_KEYWORDS, C_BOOLS),
        Language::Python => (Some("#"), None, PY_KEYWORDS, PY_BOOLS),
        Language::Json => (None, None, &[], JSON_BOOLS),
        Language::C => (Some("//"), Some(("/*", "*/")), C_KEYWORDS, C_BOOLS),
        Language::Cpp => (Some("//"), Some(("/*", "*/")), CPP_KEYWORDS, CPP_BOOLS),
        Language::Go => (Some("//"), Some(("/*", "*/")), GO_KEYWORDS, GO_BOOLS),
        Language::Shell => (Some("#"), None, SH_KEYWORDS, SH_BOOLS),
        Language::Toml => (Some("#"), None, &[], TOML_BOOLS),
        Language::Yaml => (Some("#"), None, &[], YAML_BOOLS),
        Language::Sql => (Some("--"), Some(("/*", "*/")), SQL_KEYWORDS, SQL_BOOLS),
        Language::Html => (None, Some(("<!--", "-->")), HTML_KEYWORDS, &[]),
        Language::Css => (None, Some(("/*", "*/")), CSS_KEYWORDS, &[]),
        Language::Markdown => (None, Some(("<!--", "-->")), &[], &[]),
        _ => (None, None, &[], &[]),
    }
}

/// Classify a generic-language identifier using the text that follows it.
fn classify_generic_ident(
    word: &str,
    after: &str,
    keywords: &[&str],
    booleans: &[&str],
    language: Language,
) -> TokenKind {
    if booleans.contains(&word) {
        return TokenKind::Boolean;
    }
    if keywords.contains(&word) {
        return TokenKind::Keyword;
    }
    if language == Language::Sql {
        let upper = word.to_ascii_uppercase();
        if booleans.contains(&upper.as_str()) {
            return TokenKind::Boolean;
        }
        if keywords.contains(&upper.as_str()) {
            return TokenKind::Keyword;
        }
    }
    if after.starts_with('(') {
        return TokenKind::Function;
    }
    if word.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
        return TokenKind::Type;
    }
    TokenKind::Plain
}

fn highlight_generic_line(line: &str, language: Language, state: State) -> (Vec<Token>, State) {
    let (line_comment, block_comment, keywords, booleans) = lang_params(language);
    let mut tokens = Vec::new();
    let mut i = 0usize;
    let len = line.len();
    let mut state = state;

    while i < len {
        let rest = &line[i..];
        let c = rest.chars().next().unwrap();

        if state == State::BlockComment {
            let close = block_comment.map(|(_, b)| b).unwrap_or("*/");
            match rest.find(close) {
                Some(end) => {
                    let seg = &rest[..end + close.len()];
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: seg.to_string(),
                    });
                    i += seg.len();
                    state = State::Normal;
                }
                None => {
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: rest.to_string(),
                    });
                    i = len;
                }
            }
            continue;
        }

        if let Some(lc) = line_comment
            && rest.starts_with(lc)
        {
            tokens.push(Token {
                kind: TokenKind::Comment,
                text: rest.to_string(),
            });
            i = len;
            continue;
        }

        if let Some((open, close)) = block_comment
            && rest.starts_with(open)
        {
            match rest[open.len()..].find(close) {
                Some(end) => {
                    let seg = &rest[..end + open.len() + close.len()];
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: seg.to_string(),
                    });
                    i += seg.len();
                }
                None => {
                    tokens.push(Token {
                        kind: TokenKind::Comment,
                        text: rest.to_string(),
                    });
                    i = len;
                    state = State::BlockComment;
                }
            }
            continue;
        }

        if c == '"' || c == '\'' || c == '`' {
            let (text, consumed) = read_string(rest, c);
            tokens.push(Token {
                kind: TokenKind::String,
                text,
            });
            i += consumed;
            continue;
        }

        if c.is_ascii_digit() {
            let (text, consumed) = read_number(rest);
            tokens.push(Token {
                kind: TokenKind::Number,
                text,
            });
            i += consumed;
            continue;
        }

        if c.is_alphabetic() || c == '_' {
            let (word, consumed) = read_ident(rest);
            let after = &rest[consumed..];
            let kind = classify_generic_ident(&word, after, keywords, booleans, language);
            tokens.push(Token { kind, text: word });
            i += consumed;
            continue;
        }

        if is_operator(c) {
            let mut j = i + c.len_utf8();
            for ch in line[j..].chars() {
                if !is_operator(ch) {
                    break;
                }
                j += ch.len_utf8();
            }
            tokens.push(Token {
                kind: TokenKind::Operator,
                text: line[i..j].to_string(),
            });
            i = j;
            continue;
        }

        if is_punct(c) {
            tokens.push(Token {
                kind: TokenKind::Punct,
                text: c.to_string(),
            });
            i += 1;
            continue;
        }

        let mut j = i + c.len_utf8();
        for ch in line[j..].chars() {
            if !is_plain_char(ch) {
                break;
            }
            j += ch.len_utf8();
        }
        tokens.push(Token {
            kind: TokenKind::Plain,
            text: line[i..j].to_string(),
        });
        i = j;
    }

    (tokens, state)
}

const RUST_KEYWORDS: &[&str] = &[
    "as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern",
    "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "type", "unsafe", "use", "where",
    "while",
];

const JAVA_KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "final",
    "finally",
    "float",
    "for",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "package",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];
const CSHARP_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "async",
    "await",
    "base",
    "bool",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "checked",
    "class",
    "const",
    "continue",
    "decimal",
    "default",
    "delegate",
    "do",
    "double",
    "else",
    "enum",
    "event",
    "explicit",
    "extern",
    "finally",
    "fixed",
    "float",
    "for",
    "foreach",
    "if",
    "implicit",
    "in",
    "int",
    "interface",
    "internal",
    "is",
    "lock",
    "long",
    "namespace",
    "new",
    "object",
    "operator",
    "out",
    "override",
    "params",
    "private",
    "protected",
    "public",
    "readonly",
    "record",
    "ref",
    "return",
    "sbyte",
    "sealed",
    "short",
    "sizeof",
    "stackalloc",
    "static",
    "string",
    "struct",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "uint",
    "ulong",
    "unchecked",
    "unsafe",
    "ushort",
    "using",
    "var",
    "virtual",
    "void",
    "volatile",
    "while",
    "yield",
];
const PHP_KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "die",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "endfor",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "enum",
    "eval",
    "exit",
    "extends",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

const JS_KEYWORDS: &[&str] = &[
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "export",
    "extends",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "of",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];
const JS_BOOLS: &[&str] = &["true", "false", "null", "undefined"];

const TS_KEYWORDS: &[&str] = &[
    "abstract",
    "as",
    "asserts",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "declare",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "finally",
    "for",
    "from",
    "function",
    "get",
    "if",
    "implements",
    "import",
    "in",
    "infer",
    "instanceof",
    "interface",
    "is",
    "keyof",
    "let",
    "namespace",
    "new",
    "of",
    "private",
    "protected",
    "public",
    "readonly",
    "return",
    "set",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "try",
    "type",
    "typeof",
    "var",
    "void",
    "while",
];
const TS_BOOLS: &[&str] = &["true", "false", "null", "undefined"];

const PY_KEYWORDS: &[&str] = &[
    "and", "as", "assert", "async", "await", "break", "case", "class", "continue", "def", "del",
    "elif", "else", "except", "finally", "for", "from", "global", "if", "import", "in", "is",
    "lambda", "match", "nonlocal", "not", "or", "pass", "raise", "return", "try", "while", "with",
    "yield",
];
const PY_BOOLS: &[&str] = &["True", "False", "None"];

const JSON_BOOLS: &[&str] = &["true", "false", "null"];

const C_KEYWORDS: &[&str] = &[
    "auto", "break", "case", "char", "const", "continue", "default", "do", "double", "else",
    "enum", "extern", "float", "for", "goto", "if", "inline", "int", "long", "register",
    "restrict", "return", "short", "signed", "sizeof", "static", "struct", "switch", "typedef",
    "union", "unsigned", "void", "volatile", "while",
];
const C_BOOLS: &[&str] = &["NULL", "true", "false", "bool"];

const CPP_KEYWORDS: &[&str] = &[
    "alignas",
    "alignof",
    "and",
    "and_eq",
    "asm",
    "auto",
    "bitand",
    "bitor",
    "bool",
    "break",
    "case",
    "catch",
    "char",
    "class",
    "concept",
    "const",
    "consteval",
    "constexpr",
    "constinit",
    "const_cast",
    "continue",
    "co_await",
    "co_return",
    "co_yield",
    "decltype",
    "default",
    "delete",
    "do",
    "double",
    "dynamic_cast",
    "else",
    "enum",
    "explicit",
    "export",
    "extern",
    "float",
    "for",
    "friend",
    "goto",
    "if",
    "inline",
    "int",
    "long",
    "mutable",
    "namespace",
    "new",
    "noexcept",
    "not",
    "not_eq",
    "operator",
    "or",
    "or_eq",
    "private",
    "protected",
    "public",
    "register",
    "reinterpret_cast",
    "requires",
    "return",
    "short",
    "signed",
    "sizeof",
    "static",
    "static_assert",
    "static_cast",
    "struct",
    "switch",
    "template",
    "this",
    "thread_local",
    "throw",
    "try",
    "typedef",
    "typeid",
    "typename",
    "union",
    "unsigned",
    "using",
    "virtual",
    "void",
    "volatile",
    "while",
];
const CPP_BOOLS: &[&str] = &["nullptr", "true", "false", "NULL"];

const GO_KEYWORDS: &[&str] = &[
    "break",
    "case",
    "chan",
    "const",
    "continue",
    "default",
    "defer",
    "else",
    "fallthrough",
    "for",
    "func",
    "go",
    "goto",
    "if",
    "import",
    "interface",
    "map",
    "package",
    "range",
    "return",
    "select",
    "struct",
    "switch",
    "type",
    "var",
];
const GO_BOOLS: &[&str] = &["true", "false", "iota", "nil"];

const SH_KEYWORDS: &[&str] = &[
    "if", "then", "else", "elif", "fi", "case", "esac", "for", "select", "while", "until", "do",
    "done", "in", "function", "time", "return", "exit", "export", "local", "readonly", "set",
    "unset", "shift", "source",
];
const SH_BOOLS: &[&str] = &["true", "false"];

const TOML_BOOLS: &[&str] = &["true", "false", "inf", "nan"];

const YAML_BOOLS: &[&str] = &[
    "true", "false", "yes", "no", "null", "on", "off", "True", "False", "None",
];

const SQL_KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "INSERT",
    "INTO",
    "UPDATE",
    "DELETE",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "ON",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "CREATE",
    "TABLE",
    "INDEX",
    "DROP",
    "ALTER",
    "ADD",
    "COLUMN",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "NOT",
    "NULL",
    "AND",
    "OR",
    "IN",
    "AS",
    "DISTINCT",
    "UNION",
    "ALL",
    "EXISTS",
    "BETWEEN",
    "LIKE",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "CAST",
    "VALUES",
    "SET",
    "DEFAULT",
    "CHECK",
    "UNIQUE",
];
const SQL_BOOLS: &[&str] = &["TRUE", "FALSE", "NULL", "true", "false", "null"];

const HTML_KEYWORDS: &[&str] = &[
    "html", "head", "body", "title", "meta", "link", "script", "style", "div", "span", "p", "a",
    "button", "input", "form", "textarea", "select", "option", "h1", "h2", "h3", "h4", "h5", "h6",
    "ul", "ol", "li", "table", "tr", "th", "td", "img", "svg", "path", "header", "footer", "main",
    "nav", "section", "article", "aside",
];

const CSS_KEYWORDS: &[&str] = &[
    "display",
    "flex",
    "grid",
    "position",
    "absolute",
    "relative",
    "fixed",
    "sticky",
    "width",
    "height",
    "min-width",
    "max-width",
    "margin",
    "padding",
    "border",
    "background",
    "color",
    "font-family",
    "font-size",
    "font-weight",
    "line-height",
    "text-align",
    "overflow",
    "cursor",
    "transition",
    "transform",
    "opacity",
    "z-index",
    "inherit",
    "initial",
    "none",
    "auto",
    "important",
];

#[cfg(test)]
mod tests {
    #[test]
    fn lexical_rendering_budget_bounds_work_without_frame_waits_for_each_small_batch() {
        for cost in [0.01, 1.5] {
            let mut elapsed = 0.0;
            let mut batches = 0;
            let mut frames = 0;
            for _ in 0..100_000_usize.div_ceil(super::LEXICAL_BATCH_ROWS) {
                elapsed += cost;
                batches += 1;
                if super::lexical_frame_due(batches, elapsed) {
                    assert!(elapsed <= 4.0 + cost);
                    assert!(batches <= 64);
                    elapsed = 0.0;
                    batches = 0;
                    frames += 1;
                }
            }
            if cost < 0.1 {
                assert!(
                    frames < 20,
                    "cheap retained rows must not wait nearly 100 frames"
                );
            } else {
                assert!(
                    frames > 200,
                    "costly batches must regularly yield rendering"
                );
            }
        }
        for elapsed in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0] {
            assert!(!super::lexical_frame_due(
                super::LEXICAL_BATCHES_PER_FRAME - 1,
                elapsed
            ));
            assert!(super::lexical_frame_due(
                super::LEXICAL_BATCHES_PER_FRAME,
                elapsed
            ));
        }
    }

    use super::*;

    #[test]
    fn cooperative_lexical_rows_preserve_context_and_newline_contracts() {
        let source = format!(
            "/* start\r\ninside 文😀\r\n*/ let value = 1;\n{}\r\ntail\r",
            "x".repeat(MAX_HIGHLIGHT_LINE_BYTES + 1)
        );
        for language in [
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::JavaScript,
            Language::Jsx,
            Language::Python,
            Language::Java,
            Language::CSharp,
            Language::Cpp,
            Language::Php,
            Language::Shell,
            Language::C,
            Language::Go,
            Language::Html,
            Language::Css,
            Language::Plain,
        ] {
            for normalize in [false, true] {
                let source = std::sync::Arc::new(source.clone());
                let mut job = if normalize {
                    LexicalPreparation::for_textarea(source.clone(), language)
                } else {
                    LexicalPreparation::new(source.clone(), language)
                };
                let mut steps = 0;
                while !job.is_complete() {
                    assert_eq!(job.advance(1, 16), 1);
                    steps += 1;
                }
                assert!(steps > 3, "fixture must cross lexical batch boundaries");
                let expected = if normalize {
                    source.replace("\r\n", "\n")
                } else {
                    source.to_string()
                };
                let snapshot = job.finish_snapshot().unwrap();
                assert!(std::sync::Arc::ptr_eq(&source, &snapshot.source));
                assert_eq!(
                    snapshot.tokens().as_ref(),
                    &share_token_rows(highlight_lines(&expected, language))
                );
            }
        }
        for source in ["", "\n", "\r\n", "\r", "a\n\n"] {
            let mut job =
                LexicalPreparation::new(std::sync::Arc::new(source.to_owned()), Language::Plain);
            while !job.is_complete() {
                job.advance(2, 8);
            }
            assert_eq!(
                job.finish().unwrap(),
                share_token_rows(highlight_lines(source, Language::Plain))
            );
        }
        let mut cancelled = LexicalPreparation::new(
            std::sync::Arc::new("/*\nunfinished\n*/".to_owned()),
            Language::Rust,
        );
        assert_eq!(cancelled.advance(1, 8), 1);
        assert!(
            cancelled.finish().is_none(),
            "partial lexical jobs cannot publish"
        );
    }

    #[test]
    fn indexed_lexical_boundaries_limit_scans_and_keep_context_propagation() {
        use std::sync::Arc;
        for ending in ["\n", "\r\n"] {
            let source =
                Arc::new(format!("head{ending}body 文😀{ending}tail{ending}").repeat(1000));
            let mut old = LexicalPreparation::for_textarea(source.clone(), Language::Rust);
            while !old.is_complete() {
                old.advance(128, usize::MAX);
            }
            let old = Arc::new(old.finish_snapshot().unwrap());
            let changed = Arc::new(source.replacen("body", "updated", 1));
            let mut job = LexicalPreparation::for_textarea(changed.clone(), Language::Rust)
                .reuse(old.clone());
            while !job.is_complete() {
                job.advance(128, usize::MAX);
            }
            assert_eq!(job.boundary_scans, 1);
            assert_eq!(job.indexed_searches.get(), 1);
            let next = job.finish_snapshot().unwrap();
            assert_eq!(next.retokenized_rows(), 1);
            assert_eq!(
                next.tokens().as_ref(),
                &share_token_rows(highlight_lines(
                    &changed.replace("\r\n", "\n"),
                    Language::Rust
                ))
            );
            assert!(Arc::ptr_eq(&next.tokens()[2999], &old.tokens()[2999]));
        }
        let source = Arc::new("/*\ninside\n*/\ntail\n/*\ninside\n*/".to_owned());
        let mut old = LexicalPreparation::new(source.clone(), Language::Rust);
        old.advance(128, usize::MAX);
        let old = Arc::new(old.finish_snapshot().unwrap());
        let changed = Arc::new(source.replacen("*/", "--", 1));
        let mut job = LexicalPreparation::new(changed.clone(), Language::Rust).reuse(old);
        job.advance(128, usize::MAX);
        assert_eq!(job.boundary_scans, 1);
        assert_eq!(job.indexed_searches.get(), 1);
        let next = job.finish_snapshot().unwrap();
        assert!(next.retokenized_rows() > 1);
        assert_eq!(
            next.tokens().as_ref(),
            &share_token_rows(highlight_lines(&changed, Language::Rust))
        );
    }

    #[test]
    fn indexed_lexical_boundaries_match_fresh_rows_through_unicode_crlf_and_eof_edits() {
        use std::sync::Arc;
        for source in [
            "",
            "\n",
            "\r\n",
            "a",
            "a\nb",
            "a\nb\n",
            "文😀\r\nx",
            "/*\nx\n*/",
        ] {
            let mut old =
                LexicalPreparation::for_textarea(Arc::new(source.to_owned()), Language::Rust);
            old.advance(128, usize::MAX);
            let old = Arc::new(old.finish_snapshot().unwrap());
            let positions = source
                .char_indices()
                .map(|(offset, _)| offset)
                .chain([source.len()])
                .collect::<Vec<_>>();
            for &start in &positions {
                for &end in positions.iter().filter(|&&end| end >= start) {
                    for replacement in ["", "x", "\n", "\r\n", "/*", "*/", "文😀"] {
                        let changed = Arc::new(format!(
                            "{}{replacement}{}",
                            &source[..start],
                            &source[end..]
                        ));
                        let mut job =
                            LexicalPreparation::for_textarea(changed.clone(), Language::Rust)
                                .reuse(old.clone());
                        while !job.is_complete() {
                            job.advance(2, 8);
                        }
                        assert_eq!(
                            job.finish_snapshot().unwrap().tokens().as_ref(),
                            &share_token_rows(highlight_lines(
                                &changed.replace("\r\n", "\n"),
                                Language::Rust
                            )),
                            "{source:?} {start}..{end} {replacement:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn unchanged_lexical_tables_preserve_source_settings_and_repeated_reuse() {
        use std::sync::Arc;
        let source = Arc::new("/*\r\n文😀\r\n*/\r\n".repeat(1000));
        let prepare = |source: Arc<String>| {
            let mut job = LexicalPreparation::for_textarea(source, Language::Rust);
            while !job.is_complete() {
                job.advance(128, 64 * 1024);
            }
            Arc::new(job.finish_snapshot().unwrap())
        };
        let old = prepare(source.clone());
        for current in [source.clone(), Arc::new(source.as_ref().clone())] {
            let mut job = LexicalPreparation::for_textarea(current.clone(), Language::Rust)
                .reuse(old.clone());
            assert!(job.is_complete());
            assert_eq!(job.advance(128, 64 * 1024), 0);
            let snapshot = job.finish_snapshot().unwrap();
            assert!(Arc::ptr_eq(&snapshot.source, &current));
            assert!(Arc::ptr_eq(&snapshot.rows, &old.rows));
            assert!(Arc::ptr_eq(snapshot.tokens(), old.tokens()));
            assert_eq!(snapshot.retokenized_rows(), 0);
        }
        assert_eq!(
            &LexicalPreparation::for_textarea(source.clone(), Language::Rust)
                .reuse(old.clone())
                .finish()
                .unwrap(),
            old.tokens().as_ref()
        );
        let changed = prepare(Arc::new(source.replace("文😀", "changed")));
        let mut job = LexicalPreparation::for_textarea(source.clone(), Language::Rust)
            .reuse(old.clone())
            .reuse(changed);
        assert!(!job.is_complete());
        while !job.is_complete() {
            job.advance(128, 64 * 1024);
        }
        assert_eq!(job.finish_snapshot().unwrap().tokens(), old.tokens());
        assert!(
            !LexicalPreparation::new(source.clone(), Language::Rust)
                .reuse(old.clone())
                .is_complete()
        );
        assert!(
            !LexicalPreparation::for_textarea(source, Language::Plain)
                .reuse(old)
                .is_complete()
        );
    }

    #[test]
    fn incremental_lexical_rows_reuse_only_matching_source_and_context() {
        fn prepare(
            source: &str,
            previous: Option<std::sync::Arc<LexicalSnapshot>>,
            language: Language,
        ) -> LexicalSnapshot {
            let mut job =
                LexicalPreparation::for_textarea(std::sync::Arc::new(source.to_owned()), language);
            if let Some(previous) = previous {
                job = job.reuse(previous);
            }
            while !job.is_complete() {
                job.advance(3, 40);
            }
            let result = job.finish_snapshot().unwrap();
            assert_eq!(
                result.tokens().as_ref(),
                &share_token_rows(highlight_lines(&source.replace("\r\n", "\n"), language))
            );
            result
        }
        let source = "head\r\n/*\r\ninside 文😀\r\n*/\r\ntail\r\n";
        for language in [
            Language::Rust,
            Language::TypeScript,
            Language::Tsx,
            Language::Python,
            Language::JavaScript,
            Language::Jsx,
            Language::Java,
            Language::CSharp,
            Language::Cpp,
            Language::Php,
            Language::Shell,
            Language::C,
            Language::Go,
            Language::Html,
            Language::Css,
            Language::Plain,
            Language::Sql,
            Language::Json,
            Language::Markdown,
            Language::Toml,
            Language::Yaml,
        ] {
            let old = std::sync::Arc::new(prepare(source, None, language));
            let changed = source.replace("inside 文😀", "inside revised 文😀");
            let next = prepare(&changed, Some(old.clone()), language);
            assert_eq!(next.retokenized_rows(), 1, "{language:?}");
            for row in [0, 1, 3, 4, 5] {
                assert!(
                    std::sync::Arc::ptr_eq(&old.tokens()[row], &next.tokens()[row]),
                    "unchanged row must share its token allocation: {language:?} row={row}"
                );
            }
            assert!(!std::sync::Arc::ptr_eq(&old.tokens()[2], &next.tokens()[2]));
            for changed in [
                format!("new\r\n{source}"),
                source.replace("head\r\n", ""),
                source.replace("/*", "  "),
                source.replace("*/", "  "),
                source.replace("head", "longer head").replace("tail", "x"),
                source.replace("\r\n", "\n"),
                "".into(),
                "\n".into(),
            ] {
                prepare(&changed, Some(old.clone()), language);
            }
            let unchanged = prepare(source, Some(old.clone()), language);
            assert_eq!(unchanged.retokenized_rows(), 0);
            assert!(std::sync::Arc::ptr_eq(&old.rows, &unchanged.rows));
            assert!(std::sync::Arc::ptr_eq(old.tokens(), unchanged.tokens()));
            assert!(
                old.tokens()
                    .iter()
                    .zip(unchanged.tokens().iter())
                    .all(|(old, new)| std::sync::Arc::ptr_eq(old, new))
            );
        }
        let old = std::sync::Arc::new(prepare(source, None, Language::Rust));
        let changed_context = prepare(
            &source.replace("/*", "  "),
            Some(old.clone()),
            Language::Rust,
        );
        assert!(
            changed_context.retokenized_rows() >= 3,
            "comment state propagates until convergence"
        );
        let different_language = prepare(source, Some(old), Language::Plain);
        assert_eq!(
            different_language.retokenized_rows(),
            source.split('\n').count()
        );
    }

    /// The concatenation of a line's token texts must equal the original line.
    fn rejoins(source: &str, language: Language) {
        let lines = highlight_lines(source, language);
        assert_eq!(lines.len(), source.split('\n').count());
        for (line, tokens) in source.split('\n').zip(&lines) {
            let joined: String = tokens.iter().map(|t| t.text.as_str()).collect();
            assert_eq!(joined, line, "tokens must rejoin to the source line");
        }
    }

    fn kinds(source: &str, language: Language) -> Vec<TokenKind> {
        highlight_lines(source, language)
            .into_iter()
            .flatten()
            .map(|t| t.kind)
            .collect()
    }

    #[test]
    fn language_from_path_uses_extension() {
        assert_eq!(language_from_path("src/main.rs"), Language::Rust);
        assert_eq!(language_from_path("a/b/app.py"), Language::Python);
        assert_eq!(language_from_path("index.js"), Language::JavaScript);
        assert_eq!(language_from_path("index.tsx"), Language::Tsx);
        assert_eq!(language_from_path("index.jsx"), Language::Jsx);
        assert_eq!(language_from_path("Main.java"), Language::Java);
        assert_eq!(language_from_path("Main.cs"), Language::CSharp);
        assert_eq!(language_from_path("index.php"), Language::Php);
        assert_eq!(language_from_path("package.json"), Language::Json);
        assert_eq!(language_from_path("index.html"), Language::Html);
        assert_eq!(language_from_path("styles.css"), Language::Css);
        assert_eq!(language_from_path("README.md"), Language::Markdown);
        assert_eq!(language_from_path("deploy.sh"), Language::Shell);
        assert_eq!(language_from_path("Cargo.toml"), Language::Toml);
        assert_eq!(language_from_path("docker-compose.yml"), Language::Yaml);
        assert_eq!(language_from_path("query.sql"), Language::Sql);
        assert_eq!(language_from_path("main.c"), Language::C);
        assert_eq!(language_from_path("main.cpp"), Language::Cpp);
        assert_eq!(language_from_path("server.go"), Language::Go);
        assert_eq!(language_from_path("Makefile"), Language::Plain);
        assert_eq!(language_from_path("src/lib.rs.bak"), Language::Plain);
    }

    #[test]
    fn rust_keywords_and_types() {
        let toks = highlight_lines("fn main() { let x = 5; }", Language::Rust);
        let flat: Vec<TokenKind> = toks.into_iter().flatten().map(|t| t.kind).collect();
        assert!(flat.contains(&TokenKind::Keyword)); // fn, let
        assert!(flat.contains(&TokenKind::Function)); // main
        assert!(flat.contains(&TokenKind::Number)); // 5
    }

    #[test]
    fn rust_string_and_escape() {
        let toks = highlight_lines(r#"let s = "a \"b\" c";"#, Language::Rust);
        let flat: Vec<(TokenKind, String)> = toks
            .into_iter()
            .flatten()
            .map(|t| (t.kind, t.text))
            .collect();
        assert!(flat.contains(&(TokenKind::String, r#""a \"b\" c""#.to_string())));
    }

    #[test]
    fn rust_line_comment_to_eol() {
        let toks = highlight_lines("let a = 1; // note", Language::Rust);
        let line = &toks[0];
        assert_eq!(line.last().unwrap().kind, TokenKind::Comment);
        assert_eq!(line.last().unwrap().text, "// note");
    }

    #[test]
    fn rust_block_comment_spans_lines() {
        let source = "let a = 1; /* start\nstill comment */ let b = 2;";
        let toks = highlight_lines(source, Language::Rust);
        // Line 1 ends inside the block comment; line 2 finishes it.
        assert_eq!(toks[0].last().unwrap().kind, TokenKind::Comment);
        assert_eq!(toks[0].last().unwrap().text, "/* start");
        assert_eq!(toks[1].first().unwrap().kind, TokenKind::Comment);
        assert_eq!(toks[1].first().unwrap().text, "still comment */");
        rejoins(source, Language::Rust);
    }

    #[test]
    fn rust_char_and_lifetime() {
        let toks = highlight_lines("fn f<'a>(c: char) -> &'a str { 'x' }", Language::Rust);
        let flat: Vec<(TokenKind, String)> = toks
            .into_iter()
            .flatten()
            .map(|t| (t.kind, t.text))
            .collect();
        assert!(flat.contains(&(TokenKind::Lifetime, "'a".to_string())));
        assert!(flat.contains(&(TokenKind::Char, "'x'".to_string())));
    }

    #[test]
    fn rust_macro_and_attribute() {
        let toks = highlight_lines("#[derive(Debug)]\nprintln!(\"hi\");", Language::Rust);
        let flat: Vec<(TokenKind, String)> = toks
            .into_iter()
            .flatten()
            .map(|t| (t.kind, t.text))
            .collect();
        assert!(flat.contains(&(TokenKind::Attribute, "#".to_string())));
        assert!(flat.contains(&(TokenKind::Macro, "println".to_string())));
    }

    #[test]
    fn rust_numbers() {
        let toks = highlight_lines("let a = 0x1F; let b = 3.14; let c = 1e3;", Language::Rust);
        let nums: Vec<String> = toks
            .into_iter()
            .flatten()
            .filter(|t| t.kind == TokenKind::Number)
            .map(|t| t.text)
            .collect();
        assert_eq!(nums, vec!["0x1F", "3.14", "1e3"]);
    }

    #[test]
    fn rust_range_is_not_a_float() {
        let toks = highlight_lines("for i in 1..5 {}", Language::Rust);
        let flat: Vec<(TokenKind, String)> = toks
            .into_iter()
            .flatten()
            .map(|t| (t.kind, t.text))
            .collect();
        assert!(flat.contains(&(TokenKind::Number, "1".to_string())));
        assert!(flat.contains(&(TokenKind::Number, "5".to_string())));
        // The `..` must not be swallowed into a number.
        assert!(
            !flat
                .iter()
                .any(|(k, t)| *k == TokenKind::Number && t.contains('.'))
        );
    }

    #[test]
    fn all_languages_rejoin_to_source() {
        for (src, lang) in [
            ("fn main() {}\nlet x = 'a';\n// c\n/* d */", Language::Rust),
            ("def f(x):\n    return x  # comment\n", Language::Python),
            ("const a = \"s\"; // js\n/* b */", Language::JavaScript),
            (
                "type T = { a: number };\nlet b = true;",
                Language::TypeScript,
            ),
            ("{\n  \"key\": 1, \"ok\": true\n}\n", Language::Json),
            ("plain text, no rules\n", Language::Plain),
            ("let s = \"café\"; // €\n", Language::Rust),
            ("s = \"héllo\"  # ✓\n", Language::Python),
        ] {
            rejoins(src, lang);
        }
    }

    #[test]
    fn python_comment_and_keywords() {
        let k = kinds("def f():\n    return None  # done", Language::Python);
        assert!(k.contains(&TokenKind::Keyword));
        assert!(k.contains(&TokenKind::Boolean));
        assert!(k.contains(&TokenKind::Comment));
    }

    #[test]
    fn json_strings_numbers_booleans() {
        let k = kinds(r#"{"a": 1, "b": true, "c": null}"#, Language::Json);
        assert!(k.contains(&TokenKind::String));
        assert!(k.contains(&TokenKind::Number));
        assert!(k.contains(&TokenKind::Boolean));
    }

    #[test]
    fn non_ascii_symbols_never_panic_and_rejoin() {
        let languages = [
            Language::Rust,
            Language::Python,
            Language::JavaScript,
            Language::TypeScript,
            Language::Json,
            Language::Html,
            Language::Css,
            Language::Markdown,
            Language::Shell,
            Language::Toml,
            Language::Yaml,
            Language::Sql,
            Language::C,
            Language::Cpp,
            Language::Go,
            Language::Plain,
        ];
        let samples = [
            "a — b",
            "let x = 1; €",
            "→",
            "°",
            "٣",
            "😀 x",
            "#é",
            "'é'",
            "\"é\"",
            "a→=b",
        ];
        let fixture = "fn main() { let x = 1; } // comment\n";
        for &language in &languages {
            for &sample in &samples {
                rejoins(sample, language);
                rejoins(&format!("{fixture}{sample}{fixture}"), language);
            }
        }
    }

    #[test]
    fn overlong_line_is_single_plain_token() {
        let long_line = "x".repeat(MAX_HIGHLIGHT_LINE_BYTES + 1);
        let toks = highlight_lines(&long_line, Language::Rust);
        assert_eq!(toks.len(), 1);
        assert_eq!(toks[0].len(), 1);
        assert_eq!(toks[0][0].kind, TokenKind::Plain);
        assert_eq!(toks[0][0].text, long_line);

        // A block comment spanning a capped line keeps its state across it.
        let source = format!("/* start\n{long_line}\nstill comment */ let a = 1;");
        let toks = highlight_lines(&source, Language::Rust);
        assert_eq!(toks[1].len(), 1);
        assert_eq!(toks[1][0].kind, TokenKind::Plain);
        assert_eq!(toks[1][0].text, long_line);
        assert_eq!(toks[2].first().unwrap().kind, TokenKind::Comment);
        assert_eq!(toks[2].first().unwrap().text, "still comment */");
        rejoins(&source, Language::Rust);
    }

    use proptest::prelude::*;

    /// A strategy over all 16 `Language` variants (`Language` has no
    /// `Arbitrary` impl, so enumerate them explicitly).
    fn any_language() -> impl Strategy<Value = Language> {
        prop_oneof![
            Just(Language::Rust),
            Just(Language::Python),
            Just(Language::JavaScript),
            Just(Language::TypeScript),
            Just(Language::Json),
            Just(Language::Html),
            Just(Language::Css),
            Just(Language::Markdown),
            Just(Language::Shell),
            Just(Language::Toml),
            Just(Language::Yaml),
            Just(Language::Sql),
            Just(Language::C),
            Just(Language::Cpp),
            Just(Language::Go),
            Just(Language::Plain),
        ]
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]
        // Generalises the fixed-table `all_languages_rejoin_to_source` to
        // arbitrary Unicode input across every language: concatenating a
        // line's token texts must reproduce that line exactly.
        #[test]
        fn highlight_rejoins_arbitrary(source in any::<String>(), language in any_language()) {
            rejoins(&source, language);
        }
    }
}
