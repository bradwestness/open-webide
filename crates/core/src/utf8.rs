pub struct Utf8Decoder {
    pending: Vec<u8>,
}

impl Utf8Decoder {
    pub fn new() -> Self {
        Self {
            pending: Vec::new(),
        }
    }

    pub fn push(&mut self, bytes: &[u8]) -> String {
        let mut out = String::new();
        self.pending.extend_from_slice(bytes);

        let mut offset = 0;
        while offset < self.pending.len() {
            match std::str::from_utf8(&self.pending[offset..]) {
                Ok(s) => {
                    out.push_str(s);
                    offset = self.pending.len();
                    break;
                }
                Err(e) => {
                    let valid_len = e.valid_up_to();
                    if valid_len > 0 {
                        // We have some valid text before the error
                        let valid_str = unsafe { std::str::from_utf8_unchecked(&self.pending[offset..offset + valid_len]) };
                        out.push_str(valid_str);
                        offset += valid_len;
                        continue;
                    }

                    // No valid prefix. Is it an incomplete sequence at the end, or an actual error?
                    match e.error_len() {
                        Some(err_len) => {
                            // Real error of length err_len
                            out.push('\u{FFFD}');
                            offset += err_len;
                        }
                        None => {
                            // Incomplete sequence at the end of the slice (up to 3 bytes long).
                            break;
                        }
                    }
                }
            }
        }

        self.pending.drain(..offset);
        out
    }

    pub fn finish(&mut self) -> String {
        let mut out = String::new();
        if !self.pending.is_empty() {
            // Whatever is left is incomplete and thus invalid. We just emit FFFD for each invalid chunk if we want to strictly follow lossy rules,
            // but `String::from_utf8_lossy` does exactly what we need for a trailing sequence.
            out.push_str(&String::from_utf8_lossy(&self.pending));
            self.pending.clear();
        }
        out
    }
}

impl Default for Utf8Decoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_utf8_decoder() {
        // Euro sign is 3 bytes: E2 82 AC
        // Emoji 🎉 is 4 bytes: F0 9F 8E 89
        let text = "café 日本 🎉";
        for i in 0..text.len() {
            let mut d2 = Utf8Decoder::new();
            let mut res = String::new();
            res.push_str(&d2.push(&text.as_bytes()[..i]));
            res.push_str(&d2.push(&text.as_bytes()[i..]));
            res.push_str(&d2.finish());
            assert_eq!(res, text);
        }

        // 0xFF then text -> one U+FFFD then the text
        let mut d = Utf8Decoder::new();
        let mut res = String::new();
        res.push_str(&d.push(&[0xFF]));
        res.push_str(&d.push(b"hello"));
        res.push_str(&d.finish());
        assert_eq!(res, "\u{FFFD}hello");

        // finish on a dangling lead byte
        let mut d = Utf8Decoder::new();
        let mut res = String::new();
        res.push_str(&d.push(&[0xE2, 0x82]));
        res.push_str(&d.finish());
        assert_eq!(res, "\u{FFFD}"); // from_utf8_lossy on [0xE2, 0x82] replaces with one FFFD
    }
}
