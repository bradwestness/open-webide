use crate::text::truncate_chars;

pub fn push_history(history: &mut Vec<String>, entry: String) {
    let entry = truncate_chars(&entry, 2000);
    if history.last() == Some(&entry) {
        return;
    }
    history.push(entry);
    if history.len() > 200 {
        history.drain(..history.len() - 200);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consecutive_duplicates_are_skipped() {
        let mut history = Vec::new();
        for entry in ["a", "a", "b", "a"] {
            push_history(&mut history, entry.into());
        }
        assert_eq!(history, ["a", "b", "a"]);
    }

    #[test]
    fn keeps_the_newest_200_entries() {
        let mut history = Vec::new();
        for i in 0..250 {
            push_history(&mut history, i.to_string());
        }
        assert_eq!(history.len(), 200);
        assert_eq!(history.first().unwrap(), "50");
        assert_eq!(history.last().unwrap(), "249");
    }

    #[test]
    fn truncation_is_character_safe() {
        let mut history = Vec::new();
        push_history(&mut history, "é".repeat(3000));
        assert_eq!(history[0], "é".repeat(2000));
        assert_eq!(history[0].chars().count(), 2000);
    }
}
