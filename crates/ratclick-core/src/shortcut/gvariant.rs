//! Just enough GVariant text-format parsing to read what `gsettings` prints.
//!
//! We shell out to `gsettings` rather than linking GIO into the CLI, so the
//! values come back as GVariant *text*: `['<Alt>F7', '<Super>x']`, `@as []`,
//! `'a string'`, `@ms nothing`. Only the string and string-array forms matter
//! for keybindings.

/// Parse a GVariant text value into the list of strings it represents.
///
/// Accepts a bare string (`'<Alt>F7'`), an array (`['a', 'b']`), the empty
/// array in either spelling (`[]`, `@as []`), and `nothing`. Anything else
/// yields an empty list rather than an error — an unparseable value simply
/// holds no shortcut we could conflict with.
pub fn parse_string_list(raw: &str) -> Vec<String> {
    let mut s = raw.trim();

    // Strip a leading type annotation such as `@as ` or `@ms `.
    if let Some(rest) = s.strip_prefix('@') {
        match rest.find(char::is_whitespace) {
            Some(i) => s = rest[i..].trim_start(),
            None => return Vec::new(),
        }
    }

    if s == "nothing" || s.is_empty() {
        return Vec::new();
    }

    if let Some(inner) = s.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        return split_quoted(inner);
    }

    // A single quoted string.
    split_quoted(s)
}

/// Pull every quoted run out of `s`, honouring backslash escapes.
///
/// Splitting on `,` would break on accelerators that contain a comma, so we
/// scan for quote pairs instead.
fn split_quoted(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        // GVariant text uses single quotes, but double quotes are also valid.
        if c != '\'' && c != '"' {
            continue;
        }
        let quote = c;
        let mut buf = String::new();
        loop {
            match chars.next() {
                None => break,
                Some('\\') => {
                    // Preserve the escaped character verbatim; accelerators
                    // never contain escapes we need to interpret specially.
                    if let Some(esc) = chars.next() {
                        match esc {
                            'n' => buf.push('\n'),
                            't' => buf.push('\t'),
                            other => buf.push(other),
                        }
                    }
                }
                Some(ch) if ch == quote => break,
                Some(ch) => buf.push(ch),
            }
        }
        out.push(buf);
    }
    out
}

/// Render a list of strings as a GVariant array literal for `gsettings set`.
pub fn format_string_list(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    let body = items
        .iter()
        .map(|s| format!("'{}'", escape(s)))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

/// Quote a single string as a GVariant string literal.
pub fn format_string(s: &str) -> String {
    format!("'{}'", escape(s))
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\'', "\\'")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_arrays() {
        assert_eq!(
            parse_string_list("['<Alt>F7', '<Super>x']"),
            vec!["<Alt>F7", "<Super>x"]
        );
    }

    #[test]
    fn parses_empty_arrays_in_both_spellings() {
        assert!(parse_string_list("[]").is_empty());
        assert!(parse_string_list("@as []").is_empty());
        assert!(parse_string_list("  @as []  ").is_empty());
    }

    #[test]
    fn parses_bare_strings() {
        assert_eq!(parse_string_list("'<Primary>q'"), vec!["<Primary>q"]);
    }

    #[test]
    fn parses_nothing() {
        assert!(parse_string_list("nothing").is_empty());
        assert!(parse_string_list("@ms nothing").is_empty());
    }

    #[test]
    fn does_not_split_on_a_comma_inside_an_accelerator() {
        assert_eq!(parse_string_list("['<Super>comma']"), vec!["<Super>comma"]);
        assert_eq!(parse_string_list("['a,b', 'c']"), vec!["a,b", "c"]);
    }

    #[test]
    fn handles_escaped_quotes() {
        assert_eq!(parse_string_list(r"['it\'s']"), vec!["it's"]);
    }

    #[test]
    fn formatting_roundtrips() {
        let items = vec!["<Super>c".to_string(), "it's".to_string()];
        assert_eq!(parse_string_list(&format_string_list(&items)), items);
    }

    #[test]
    fn empty_list_formats_with_a_type_annotation() {
        // A bare `[]` is ambiguous to gsettings; `@as []` is not.
        assert_eq!(format_string_list(&[]), "@as []");
    }
}
