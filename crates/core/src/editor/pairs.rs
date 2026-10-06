//! Paired typing commands, with language-aware lexical boundaries.
use super::{
    Document, Edit, EditError, Selection, Structure,
    indent::line_start,
    structure::{closing, supports_brackets, supports_quote},
};
use crate::highlight::Language;

impl Document {
    pub fn type_character(&mut self, ch: char, language: Language) -> Result<bool, EditError> {
        let syntax = Structure::new(&self.text, language);
        self.type_character_in(ch, &syntax)
    }

    pub fn type_character_with_context(
        &mut self,
        ch: char,
        syntax: &Structure,
    ) -> Result<bool, EditError> {
        if !syntax.matches_source(&self.text) {
            return Err(EditError::StaleContext);
        }
        self.type_character_in(ch, syntax)
    }

    fn type_character_in(&mut self, ch: char, syntax: &Structure) -> Result<bool, EditError> {
        let mut changes = Vec::new();
        for selection in &self.selections {
            let range = selection.range();
            let language = syntax.language_at(range.start);
            let interpolation = syntax.opens_interpolation(range.start, ch);
            let after = syntax.next_character(range.end);
            let quote = language != Language::Html && supports_quote(language, ch);
            let close = supports_brackets(language)
                .then(|| closing(ch))
                .flatten()
                .or(quote.then_some(ch));
            let start = line_start(&self.text, range.start);
            let prefix = &self.text[start..range.start];
            if range.is_empty()
                && matches!(ch, ')' | ']' | '}')
                && supports_brackets(language)
                && syntax.is_code(range.start)
                && prefix.chars().all(|ch| matches!(ch, ' ' | '\t'))
                && let Some(&(open, _, _)) = syntax
                    .brackets
                    .iter()
                    .rev()
                    .find(|&&(position, bracket, pair)| {
                        position < range.start
                            && closing(bracket).is_some()
                            && pair.is_none_or(|close| close >= range.start)
                    })
                    .filter(|(_, bracket, _)| closing(*bracket) == Some(ch))
            {
                let open_start = line_start(&self.text, open);
                let base: String = self.text[open_start..open]
                    .chars()
                    .take_while(|ch| matches!(ch, ' ' | '\t'))
                    .collect();
                let text = format!("{base}{ch}");
                changes.push((
                    Edit::replace(start..range.end + usize::from(after == Some(ch)), &text),
                    Selection::caret(text.len()),
                ));
                continue;
            }
            if range.is_empty()
                && after == Some(ch)
                && (matches!(ch, ')' | ']' | '}')
                    && syntax.brackets.iter().any(|&(position, bracket, pair)| {
                        position == range.start && bracket == ch && pair.is_some()
                    })
                    || quote && syntax.quote_closes_at(range.start))
            {
                // Replace with the same character to advance the caret without a history entry.
                changes.push((
                    Edit::replace(range.start..range.start + ch.len_utf8(), ch.to_string()),
                    Selection::caret(ch.len_utf8()),
                ));
                continue;
            }
            let safe_after = after.is_none_or(|ch| {
                ch.is_whitespace() || matches!(ch, ')' | ']' | '}' | ',' | ';' | ':')
            });
            let previous = self.text[..range.start].chars().next_back();
            let word_quote = range.is_empty()
                && quote
                && ch == '\''
                && previous.is_some_and(|ch| ch.is_alphanumeric() || matches!(ch, '_' | '&' | '<'));
            if let Some(close) = close
                && (syntax.is_code(range.start) || interpolation)
                && !word_quote
                && !(language == Language::Rust && ch == '\'' && range.is_empty())
                && (!range.is_empty() || safe_after || interpolation)
            {
                let selected = &self.text[range.clone()];
                let text = format!("{ch}{selected}{close}");
                let anchor = if selection.anchor <= selection.head {
                    ch.len_utf8()
                } else {
                    ch.len_utf8() + selected.len()
                };
                let head = if selection.anchor <= selection.head {
                    ch.len_utf8() + selected.len()
                } else {
                    ch.len_utf8()
                };
                changes.push((Edit::replace(range, text), Selection { anchor, head }));
            } else {
                changes.push((
                    Edit::replace(range, ch.to_string()),
                    Selection::caret(ch.len_utf8()),
                ));
            }
        }
        self.apply_caret_edits(changes)
    }

    /// Return false when Backspace should retain the browser's native behavior.
    pub fn delete_empty_pairs(&mut self, language: Language) -> Result<bool, EditError> {
        let syntax = Structure::new(&self.text, language);
        self.delete_pairs_in(&syntax)
    }

    pub fn delete_pairs_with_context(&mut self, syntax: &Structure) -> Result<bool, EditError> {
        if !syntax.matches_source(&self.text) {
            return Err(EditError::StaleContext);
        }
        self.delete_pairs_in(syntax)
    }

    fn delete_pairs_in(&mut self, syntax: &Structure) -> Result<bool, EditError> {
        let mut changes = Vec::new();
        for selection in &self.selections {
            if selection.anchor != selection.head {
                return Ok(false);
            }
            let position = selection.head;
            let before = self.text[..position].chars().next_back();
            let after = self.text[position..].chars().next();
            let matched = before.zip(after).is_some_and(|(open, close)| {
                closing(open) == Some(close)
                    && syntax
                        .brackets
                        .iter()
                        .any(|&(at, _, pair)| at + 1 == position && pair == Some(position))
                    || open == close
                        && matches!(open, '\'' | '"' | '`')
                        && syntax.empty_quote_pair(position)
            });
            if !matched {
                return Ok(false);
            }
            changes.push((
                Edit::replace(position - 1..position + 1, ""),
                Selection::caret(0),
            ));
        }
        self.apply_caret_edits(changes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pairs_surround_directional_unicode_selection_skip_and_delete() {
        let mut doc = Document::new("😀");
        doc.set_selections(vec![Selection { anchor: 4, head: 0 }])
            .unwrap();
        doc.type_character('(', Language::Rust).unwrap();
        assert_eq!(doc.text(), "(😀)");
        assert_eq!(doc.selections(), &[Selection { anchor: 5, head: 1 }]);
        doc.set_selections(vec![Selection::caret(5)]).unwrap();
        assert!(!doc.type_character(')', Language::Rust).unwrap());
        assert_eq!(doc.selections()[0].head, 6);
        doc.undo();
        assert_eq!(doc.text(), "😀");
        let mut doc = Document::new("");
        doc.type_character('[', Language::Json).unwrap();
        assert_eq!(doc.text(), "[]");
        assert!(doc.delete_empty_pairs(Language::Json).unwrap());
        assert_eq!(doc.text(), "");
        doc.undo();
        assert_eq!(doc.text(), "[]");
    }
    #[test]
    fn quote_surround_keeps_a_selection_inside_an_identifier() {
        let mut doc = Document::new("foobar");
        doc.set_selections(vec![Selection { anchor: 6, head: 3 }])
            .unwrap();
        doc.type_character('\'', Language::Rust).unwrap();
        assert_eq!(doc.text(), "foo'bar'");
        assert_eq!(doc.selections(), &[Selection { anchor: 7, head: 4 }]);
        doc.undo();
        assert_eq!(doc.text(), "foobar");
    }

    #[test]
    fn opening_delimiters_never_skip_and_existing_closers_also_outdent() {
        let mut doc = Document::new("({})");
        doc.type_character('(', Language::Rust).unwrap();
        assert_eq!(doc.text(), "(({})");
        let mut doc = Document::new("if x {\n    }");
        doc.set_selections(vec![Selection::caret(11)]).unwrap();
        doc.type_character('}', Language::Rust).unwrap();
        assert_eq!(doc.text(), "if x {\n}");
        doc.undo();
        assert_eq!(doc.text(), "if x {\n    }");
    }

    #[test]
    fn typed_closers_outdent_to_the_opener_without_changing_its_indentation() {
        let mut doc = Document::new("\tif x {\r\n\t\t");
        doc.set_selections(vec![Selection::caret(doc.text().len())])
            .unwrap();
        doc.type_character('}', Language::Rust).unwrap();
        assert_eq!(doc.text(), "\tif x {\r\n\t}");
        doc.undo();
        assert_eq!(doc.text(), "\tif x {\r\n\t\t");
    }
    #[test]
    fn comments_strings_lifetimes_and_plain_text_do_not_auto_pair() {
        for prefix in ["&", "impl ", "x: "] {
            let mut lifetime = Document::new(prefix);
            lifetime
                .set_selections(vec![Selection::caret(prefix.len())])
                .unwrap();
            lifetime.type_character('\'', Language::Rust).unwrap();
            assert_eq!(lifetime.text(), format!("{prefix}'"));
        }
        for (text, language) in [
            ("// comment ", Language::Rust),
            ("\"text ", Language::Rust),
            ("plain", Language::Plain),
        ] {
            let mut doc = Document::new(text);
            doc.set_selections(vec![Selection::caret(text.len())])
                .unwrap();
            doc.type_character('(', language).unwrap();
            assert_eq!(doc.text(), format!("{text}("));
        }
    }
}
