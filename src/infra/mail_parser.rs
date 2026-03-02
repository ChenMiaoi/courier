use std::collections::HashSet;

#[derive(Debug, Clone)]
pub struct ParsedMailHeaders {
    pub message_id: String,
    pub subject: String,
    pub from_addr: String,
    pub date: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub list_id: Option<String>,
}

pub fn parse_headers(raw: &[u8], fallback_message_id: String) -> ParsedMailHeaders {
    let headers = parse_header_block(raw);

    let message_id = header_value(&headers, "message-id")
        .and_then(|value| parse_message_ids(&value).into_iter().next())
        .unwrap_or(fallback_message_id);

    let in_reply_to = header_value(&headers, "in-reply-to")
        .and_then(|value| parse_message_ids(&value).into_iter().next());

    let mut references = header_value(&headers, "references")
        .map(|value| parse_message_ids(&value))
        .unwrap_or_default();

    if references.is_empty()
        && let Some(reply_to) = in_reply_to.as_ref()
    {
        references.push(reply_to.clone());
    }

    let mut dedup = HashSet::new();
    references.retain(|id| dedup.insert(id.clone()));

    ParsedMailHeaders {
        message_id,
        subject: header_value(&headers, "subject").unwrap_or_default(),
        from_addr: header_value(&headers, "from").unwrap_or_default(),
        date: header_value(&headers, "date").filter(|value| !value.is_empty()),
        in_reply_to,
        references,
        list_id: header_value(&headers, "list-id").filter(|value| !value.is_empty()),
    }
}

pub fn normalize_subject(subject: &str) -> String {
    let mut normalized = subject.trim().to_ascii_lowercase();

    loop {
        let trimmed = normalized.trim_start();
        if let Some(rest) = trimmed.strip_prefix("re:") {
            normalized = rest.trim_start().to_string();
            continue;
        }
        if let Some(rest) = trimmed.strip_prefix("fwd:") {
            normalized = rest.trim_start().to_string();
            continue;
        }
        break;
    }

    loop {
        let trimmed = normalized.trim_start();
        if !trimmed.starts_with('[') {
            normalized = trimmed.to_string();
            break;
        }

        if let Some(index) = trimmed.find(']') {
            normalized = trimmed[index + 1..].trim_start().to_string();
            continue;
        }

        normalized = trimmed.to_string();
        break;
    }

    normalized.trim().to_string()
}

fn parse_header_block(raw: &[u8]) -> Vec<(String, String)> {
    let text = String::from_utf8_lossy(raw);
    let mut headers = Vec::new();

    let mut current_name: Option<String> = None;
    let mut current_value = String::new();

    for raw_line in text.lines() {
        let line = raw_line.trim_end_matches('\r');
        if line.is_empty() {
            break;
        }

        if line.starts_with(' ') || line.starts_with('\t') {
            if current_name.is_some() {
                let fragment = line.trim();
                if !fragment.is_empty() {
                    if !current_value.is_empty() {
                        current_value.push(' ');
                    }
                    current_value.push_str(fragment);
                }
            }
            continue;
        }

        if let Some(name) = current_name.take() {
            headers.push((name, current_value.trim().to_string()));
            current_value.clear();
        }

        if let Some((name, value)) = line.split_once(':') {
            current_name = Some(name.trim().to_ascii_lowercase());
            current_value.push_str(value.trim());
        }
    }

    if let Some(name) = current_name.take() {
        headers.push((name, current_value.trim().to_string()));
    }

    headers
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .map(|(_, value)| value.trim().to_string())
}

fn parse_message_ids(raw: &str) -> Vec<String> {
    let mut ids = Vec::new();

    let mut capture = false;
    let mut current = String::new();
    for ch in raw.chars() {
        if ch == '<' {
            capture = true;
            current.clear();
            continue;
        }

        if ch == '>' {
            if capture {
                let normalized = normalize_message_id(&current);
                if !normalized.is_empty() {
                    ids.push(normalized);
                }
            }
            capture = false;
            current.clear();
            continue;
        }

        if capture {
            current.push(ch);
        }
    }

    if ids.is_empty() {
        for part in raw.split_whitespace() {
            let normalized = normalize_message_id(part);
            if !normalized.is_empty() {
                ids.push(normalized);
            }
        }
    }

    ids
}

fn normalize_message_id(value: &str) -> String {
    value
        .trim()
        .trim_matches('<')
        .trim_matches('>')
        .trim_matches('"')
        .trim_matches(',')
        .trim()
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::{normalize_subject, parse_headers};

    #[test]
    fn parses_basic_headers_and_reference_chain() {
        let raw = b"Message-ID: <root@example.com>\r\nSubject: [PATCH] demo\r\nFrom: Alice <alice@example.com>\r\nReferences: <a@example.com> <b@example.com>\r\nIn-Reply-To: <b@example.com>\r\n\r\nbody\r\n";

        let parsed = parse_headers(raw, "fallback@example.com".to_string());
        assert_eq!(parsed.message_id, "root@example.com");
        assert_eq!(parsed.in_reply_to.as_deref(), Some("b@example.com"));
        assert_eq!(parsed.references, vec!["a@example.com", "b@example.com"]);
    }

    #[test]
    fn falls_back_to_generated_message_id() {
        let raw = b"Subject: no id\r\n\r\nbody\r\n";
        let parsed = parse_headers(raw, "synthetic@example.com".to_string());
        assert_eq!(parsed.message_id, "synthetic@example.com");
    }

    #[test]
    fn folds_continuation_lines() {
        let raw = b"Message-ID: <fold@example.com>\r\nReferences: <a@example.com>\r\n <b@example.com>\r\n\r\n";
        let parsed = parse_headers(raw, "fallback@example.com".to_string());
        assert_eq!(parsed.references, vec!["a@example.com", "b@example.com"]);
    }

    #[test]
    fn normalizes_common_subject_prefixes() {
        assert_eq!(normalize_subject("Re: [PATCH v2 0/3] Demo"), "demo");
        assert_eq!(normalize_subject("fwd:  Re: status"), "status");
    }
}
