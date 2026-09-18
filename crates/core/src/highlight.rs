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
    Plain,
}

/// Pick a language from a file path's extension.
pub fn language_from_path(path: &str) -> Language {
    let ext = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "rs" => Language::Rust,
        "py" => Language::Python,
        "js" | "mjs" | "cjs" => Language::JavaScript,
        "ts" | "tsx" | "jsx" => Language::TypeScript,
        "json" => Language::Json,
        _ => Language::Plain,
    }
}

/// Tokenize `source` line by line into colored tokens.
///
/// The returned lines line up with `source.split('\n')`, and concatenating a
/// line's token texts reproduces that line exactly.
pub fn highlight_lines(source: &str, language: Language) -> Vec<Vec<Token>> {
    let mut lines = Vec::new();
    let mut state = State::Normal;
    for line in source.split('\n') {
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
    let chars: Vec<char> = rest.chars().collect();
    let mut text = String::new();
    let mut consumed = 0usize;
    let mut i = 0usize;
    if i < chars.len() {
        text.push(chars[i]);
        consumed += chars[i].len_utf8();
        i += 1;
    }
    while i < chars.len() {
        let c = chars[i];
        text.push(c);
        consumed += c.len_utf8();
        i += 1;
        if c == '\\' && i < chars.len() {
            let esc = chars[i];
            text.push(esc);
            consumed += esc.len_utf8();
            i += 1;
        } else if c == quote {
            break;
        }
    }
    (text, consumed)
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
    let chars: Vec<char> = rest.chars().collect();
    let mut i = 0usize;
    let mut consumed = 0usize;
    let mut text = String::new();

    // `0x` / `0o` / `0b` prefix.
    if i < chars.len() && chars[i] == '0' {
        text.push(chars[i]);
        consumed += chars[i].len_utf8();
        i += 1;
        if i < chars.len() && "xXoObB".contains(chars[i]) {
            text.push(chars[i]);
            consumed += chars[i].len_utf8();
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_hexdigit() || chars[i] == '_') {
                text.push(chars[i]);
                consumed += chars[i].len_utf8();
                i += 1;
            }
            return (text, consumed);
        }
    }

    // Integer part.
    while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
        text.push(chars[i]);
        consumed += chars[i].len_utf8();
        i += 1;
    }
    // Fractional part (only if a digit follows the dot, so `1..2` stays a range).
    if i + 1 < chars.len() && chars[i] == '.' && chars[i + 1].is_ascii_digit() {
        text.push(chars[i]);
        consumed += chars[i].len_utf8();
        i += 1;
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
            text.push(chars[i]);
            consumed += chars[i].len_utf8();
            i += 1;
        }
    }
    // Exponent.
    if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
        text.push(chars[i]);
        consumed += chars[i].len_utf8();
        i += 1;
        if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
            text.push(chars[i]);
            consumed += chars[i].len_utf8();
            i += 1;
        }
        while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
            text.push(chars[i]);
            consumed += chars[i].len_utf8();
            i += 1;
        }
    }

    (text, consumed)
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
    let chars: Vec<char> = rest.chars().collect();
    if chars.is_empty() || chars[0] != '\'' {
        return None;
    }
    let mut consumed = chars[0].len_utf8();
    let mut text = String::new();
    text.push(chars[0]);

    if chars.len() < 2 {
        return Some((
            Token {
                kind: TokenKind::Lifetime,
                text,
            },
            consumed,
        ));
    }

    let second = chars[1];
    if second == '\\' {
        // Escaped char literal, e.g. `'\n'` or `'\''`.
        if chars.len() >= 4 && chars[3] == '\'' {
            consumed += second.len_utf8() + chars[2].len_utf8() + chars[3].len_utf8();
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
                text,
            },
            consumed,
        ));
    }
    // A char literal when the third char is a closing quote.
    if chars.len() >= 3 && chars[2] == '\'' {
        consumed += second.len_utf8() + chars[2].len_utf8();
        return Some((
            Token {
                kind: TokenKind::Char,
                text: rest[..consumed].to_string(),
            },
            consumed,
        ));
    }
    // Otherwise a lifetime: `'ident`.
    for &c in &chars[1..] {
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
            let mut j = i + 1;
            while j < len && is_operator(line[j..].chars().next().unwrap()) {
                j += 1;
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

        let mut j = i + 1;
        while j < len && is_plain_char(line[j..].chars().next().unwrap()) {
            j += 1;
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
        _ => (None, None, &[], &[]),
    }
}

/// Classify a generic-language identifier using the text that follows it.
fn classify_generic_ident(
    word: &str,
    after: &str,
    keywords: &[&str],
    booleans: &[&str],
) -> TokenKind {
    if booleans.contains(&word) {
        return TokenKind::Boolean;
    }
    if keywords.contains(&word) {
        return TokenKind::Keyword;
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
            let kind = classify_generic_ident(&word, after, keywords, booleans);
            tokens.push(Token { kind, text: word });
            i += consumed;
            continue;
        }

        if is_operator(c) {
            let mut j = i + 1;
            while j < len && is_operator(line[j..].chars().next().unwrap()) {
                j += 1;
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

        let mut j = i + 1;
        while j < len && is_plain_char(line[j..].chars().next().unwrap()) {
            j += 1;
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
        assert_eq!(language_from_path("README.md"), Language::Plain);
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
}
