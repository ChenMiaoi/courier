use std::fs;

use crate::infra::mail_store::ThreadRow;

use super::{PREVIEW_RECIPIENT_PREVIEW_LIMIT, PREVIEW_TAB_SPACES};

pub(super) fn load_mail_preview(thread: &ThreadRow) -> String {
    let subject = normalized_subject_or_default(&thread.subject);
    let fallback_from = normalized_header_or_default(&thread.from_addr, "<unknown sender>");
    let fallback_sent = thread.date.as_deref().and_then(non_empty_normalized_header);

    let Some(path) = thread.raw_path.as_ref() else {
        return format_preview_with_headers(
            &fallback_from,
            fallback_sent.as_deref().unwrap_or("<unknown sent time>"),
            "<none>",
            "<none>",
            &subject,
            "<raw mail file unavailable>",
        );
    };

    let content = match fs::read(path) {
        Ok(value) => value,
        Err(error) => {
            return format_preview_with_headers(
                &fallback_from,
                fallback_sent.as_deref().unwrap_or("<unknown sent time>"),
                "<none>",
                "<none>",
                &subject,
                &format!("<failed to read {}: {}>", path.display(), error),
            );
        }
    };

    extract_mail_preview(&content, &subject, &fallback_from, fallback_sent.as_deref())
}

pub(super) fn extract_mail_preview(
    raw: &[u8],
    fallback_subject: &str,
    fallback_from: &str,
    fallback_sent: Option<&str>,
) -> String {
    let headers = parse_preview_header_block(raw);

    let from = preview_header_value(&headers, "from")
        .or_else(|| non_empty_normalized_header(fallback_from))
        .unwrap_or_else(|| "<unknown sender>".to_string());
    let sent = preview_header_value(&headers, "date")
        .or_else(|| fallback_sent.and_then(non_empty_normalized_header))
        .unwrap_or_else(|| "<unknown sent time>".to_string());
    let to = preview_recipient_line(&headers, "to");
    let cc = preview_recipient_line(&headers, "cc");
    let subject = preview_header_value(&headers, "subject")
        .or_else(|| non_empty_normalized_header(fallback_subject))
        .unwrap_or_else(|| "(no subject)".to_string());
    let body = extract_mail_body_preview(raw);

    format_preview_with_headers(&from, &sent, &to, &cc, &subject, &body)
}

fn format_preview_with_headers(
    from: &str,
    sent: &str,
    to: &str,
    cc: &str,
    subject: &str,
    body: &str,
) -> String {
    format!("From: {from}\nSent: {sent}\nTo: {to}\nCc: {cc}\nSubject: {subject}\n\n{body}")
}

fn normalized_subject_or_default(subject: &str) -> String {
    non_empty_normalized_header(subject).unwrap_or_else(|| "(no subject)".to_string())
}

fn non_empty_normalized_header(value: &str) -> Option<String> {
    let normalized = normalize_preview_header_whitespace(value);
    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn normalized_header_or_default(value: &str, default: &str) -> String {
    non_empty_normalized_header(value).unwrap_or_else(|| default.to_string())
}

fn parse_preview_header_block(raw: &[u8]) -> Vec<(String, String)> {
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
            headers.push((name, normalize_preview_header_whitespace(&current_value)));
            current_value.clear();
        }

        if let Some((name, value)) = line.split_once(':') {
            current_name = Some(name.trim().to_ascii_lowercase());
            current_value.push_str(value.trim());
        }
    }

    if let Some(name) = current_name.take() {
        headers.push((name, normalize_preview_header_whitespace(&current_value)));
    }

    headers
}

fn preview_header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(header_name, _)| header_name == name)
        .and_then(|(_, value)| non_empty_normalized_header(value))
}

fn preview_header_values(headers: &[(String, String)], name: &str) -> Vec<String> {
    headers
        .iter()
        .filter(|(header_name, _)| header_name == name)
        .filter_map(|(_, value)| non_empty_normalized_header(value))
        .collect()
}

fn preview_recipient_line(headers: &[(String, String)], name: &str) -> String {
    let mut recipients = Vec::new();
    for value in preview_header_values(headers, name) {
        recipients.extend(split_recipient_list(&value));
    }

    if recipients.is_empty() {
        return "<none>".to_string();
    }

    if recipients.len() <= PREVIEW_RECIPIENT_PREVIEW_LIMIT {
        return recipients.join("; ");
    }

    format!(
        "{}; ...",
        recipients[..PREVIEW_RECIPIENT_PREVIEW_LIMIT].join("; ")
    )
}

fn split_recipient_list(value: &str) -> Vec<String> {
    let mut recipients = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut angle_depth = 0usize;

    for character in value.chars() {
        if in_quotes {
            current.push(character);
            if escaped {
                escaped = false;
                continue;
            }

            match character {
                '\\' => escaped = true,
                '"' => in_quotes = false,
                _ => {}
            }
            continue;
        }

        match character {
            '"' => {
                in_quotes = true;
                current.push(character);
            }
            '<' => {
                angle_depth += 1;
                current.push(character);
            }
            '>' => {
                angle_depth = angle_depth.saturating_sub(1);
                current.push(character);
            }
            ',' | ';' if angle_depth == 0 => {
                if let Some(recipient) = non_empty_normalized_header(&current) {
                    recipients.push(recipient);
                }
                current.clear();
            }
            _ => current.push(character),
        }
    }

    if let Some(recipient) = non_empty_normalized_header(&current) {
        recipients.push(recipient);
    }

    recipients
}

fn normalize_preview_header_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

pub(super) fn extract_mail_body_preview(raw: &[u8]) -> String {
    let body_start = find_subslice(raw, b"\r\n\r\n")
        .map(|index| index + 4)
        .or_else(|| find_subslice(raw, b"\n\n").map(|index| index + 2))
        .unwrap_or(0);

    let body = &raw[body_start..];
    let text = String::from_utf8_lossy(body).replace("\r\n", "\n");
    let stripped = strip_first_mime_part_headers(&text);

    let sanitized = sanitize_preview_text(&stripped);

    let lines: Vec<&str> = sanitized
        .lines()
        .map(str::trim_end)
        .skip_while(|line| line.trim().is_empty())
        .take(80)
        .collect();

    let snippet = lines.join("\n");
    if snippet.trim().is_empty() {
        "<empty mail body>".to_string()
    } else {
        snippet
    }
}

fn sanitize_preview_text(input: &str) -> String {
    let mut sanitized = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\n' => sanitized.push('\n'),
            '\t' => sanitized.push_str(PREVIEW_TAB_SPACES),
            _ if character.is_control() => {}
            _ => sanitized.push(character),
        }
    }
    sanitized
}

fn strip_first_mime_part_headers(body: &str) -> String {
    let lines: Vec<&str> = body.lines().collect();
    let Some(first_non_empty_index) = lines.iter().position(|line| !line.trim().is_empty()) else {
        return String::new();
    };

    let boundary = lines[first_non_empty_index].trim();
    if !boundary.starts_with("--") {
        return body.to_string();
    }

    let mut cursor = first_non_empty_index + 1;
    while cursor < lines.len() && !lines[cursor].trim().is_empty() {
        cursor += 1;
    }

    if cursor >= lines.len() {
        return body.to_string();
    }

    let content_start = cursor + 1;
    let mut content = Vec::new();
    let closing_boundary = format!("{boundary}--");
    for line in &lines[content_start..] {
        let trimmed = line.trim();
        if trimmed == boundary || trimmed == closing_boundary {
            break;
        }
        content.push(line.trim_end());
    }

    if content.is_empty() {
        body.to_string()
    } else {
        content.join("\n")
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || needle.len() > haystack.len() {
        return None;
    }

    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
