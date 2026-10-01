/// URL encode query parameter components.
pub fn url_encode(input: &str) -> String {
    use std::fmt::Write;
    let mut encoded = String::with_capacity(input.len() * 2);
    for byte in input.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(byte as char);
            }
            b' ' => encoded.push('+'),
            _ => {
                write!(encoded, "%{byte:02X}").expect("writing to a string cannot fail");
            }
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodes_query_components() {
        assert_eq!(url_encode("a b/☃%"), "a+b%2F%E2%98%83%25");
        assert_eq!(url_encode("Az-._~09"), "Az-._~09");
    }
}
