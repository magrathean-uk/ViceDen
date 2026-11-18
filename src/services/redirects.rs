pub fn safe_return_to(raw: Option<String>, fallback: String) -> String {
    let Some(raw) = raw else {
        return fallback;
    };

    let decoded = percent_decode_once(raw.trim());
    if is_safe_local_path(&decoded) {
        decoded
    } else {
        fallback
    }
}

fn is_safe_local_path(value: &str) -> bool {
    if !value.starts_with('/') || value.starts_with("//") {
        return false;
    }
    if value.chars().any(|ch| ch == '\\' || ch.is_control() || ch.is_whitespace()) {
        return false;
    }
    true
}

fn percent_decode_once(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            if let (Some(high), Some(low)) =
                (from_hex(bytes[index + 1]), from_hex(bytes[index + 2]))
            {
                output.push((high << 4) | low);
                index += 3;
                continue;
            }
        }
        output.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&output).to_string()
}

fn from_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::safe_return_to;

    #[test]
    fn rejects_redirect_tricks() {
        let cases = [
            "//evil.test",
            "%2f%2fevil.test",
            "/%5cevil",
            "\\\\evil",
            "https://evil.test",
            "http://evil.test",
            "javascript:alert(1)",
            "/has space",
            "/\ncontrol",
        ];
        for input in cases {
            assert_eq!(safe_return_to(Some(input.to_string()), "/profile".to_string()), "/profile");
        }
    }

    #[test]
    fn accepts_single_slash_local_paths() {
        assert_eq!(
            safe_return_to(Some("/b/general?page=2".to_string()), "/profile".to_string()),
            "/b/general?page=2",
        );
    }
}
