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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// The language a file is written in, chosen from its extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    Python,
    JavaScript,
    TypeScript,
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
        "ts" | "tsx" | "jsx" => Language::TypeScript,
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
    let mut lines = Vec::new();
    let mut state = State::Normal;
    for line in source.split('\n') {
        if line.len() > MAX_HIGHLIGHT_LINE_BYTES {
            lines.push(vec![Token {
                kind: TokenKind::Plain,
                text: line.to_string(),
            }]);
            continue;
        }
        let (tokens, next_state) = match language {
            Language::Rust => highlight_rust_line(line, state),
            _ => highlight_generic_line(line, language, state),
        };
        state = next_state;
        lines.push(tokens);
    }
    lines
}

/// Tokenizer state carried across lines (a block comment may span lines).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
        Language::JavaScript => (Some("//"), Some(("/*", "*/")), JS_KEYWORDS, JS_BOOLS),
        Language::TypeScript => (Some("//"), Some(("/*", "*/")), TS_KEYWORDS, TS_BOOLS),
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
    use super::*;

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
        assert_eq!(language_from_path("index.tsx"), Language::TypeScript);
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
