//! Converts a subset of Markdown (this app's own entry body format) to Typst markup.
//!
//! Handles: headings (`#`/`##`/`###` -> `=`/`==`/`===`), bold (`**x**`/`__x__` -> `*x*`),
//! italic (`*x*`/`_x_` -> `_x_`), unordered lists (`-`/`*` -> `-`), ordered lists
//! (`1.` -> `+`), inline code (`` `x` `` -> `` `x` ``, same syntax), links
//! (`[text](url)` -> `#link("url")[text]`), and paragraph breaks.
//!
//! Does NOT handle image references -- those are extracted and rendered separately
//! by `export::pdf`, not inline-converted here (the app's own editor always emits
//! image references as `![alt](attachments/..)`, which this converter treats like
//! any other paragraph text -- image extraction happens on the *original* `body_md`
//! before conversion, see `commands::export::export_pdf`).
//!
//! Anything unrecognized passes through as literal text, with Typst's own special
//! characters (`# * _ \` < > @ $ \`) escaped, so stray Markdown syntax this app
//! doesn't generate itself (or a raw `#`/`$` in someone's note text) can't break
//! compilation of the generated Typst source.

use std::sync::OnceLock;

use regex::Regex;

/// Converts a subset of Markdown (this app's own entry body format) to Typst markup.
pub fn convert(body_md: &str) -> String {
    let mut out = String::new();
    for line in body_md.lines() {
        if line.trim().is_empty() {
            // Blank line -> paragraph break. A single extra "\n" here combines with
            // the "\n" already pushed after the previous line to form "\n\n", which
            // Typst treats as a paragraph break (a lone "\n" is just whitespace).
            out.push('\n');
            continue;
        }

        if let Some(caps) = heading_pattern().captures(line) {
            let level = caps[1].len();
            out.push_str(&"=".repeat(level));
            out.push(' ');
            out.push_str(&convert_inline(&caps[2]));
        } else if let Some(caps) = unordered_list_pattern().captures(line) {
            out.push_str("- ");
            out.push_str(&convert_inline(&caps[1]));
        } else if let Some(caps) = ordered_list_pattern().captures(line) {
            out.push_str("+ ");
            out.push_str(&convert_inline(&caps[1]));
        } else {
            out.push_str(&convert_inline(line));
        }
        out.push('\n');
    }
    out.trim().to_string()
}

fn convert_inline(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;
    for caps in inline_pattern().captures_iter(text) {
        let whole = caps.get(0).expect("group 0 always matches");
        result.push_str(&escape_literal(&text[last_end..whole.start()]));

        if let Some(code) = caps.name("code") {
            let s = code.as_str();
            let inner = &s[1..s.len() - 1];
            result.push('`');
            result.push_str(inner);
            result.push('`');
        } else if let Some(bold) = caps.name("bold") {
            let s = bold.as_str();
            let inner = &s[2..s.len() - 2];
            result.push('*');
            result.push_str(&escape_literal(inner));
            result.push('*');
        } else if let Some(italic) = caps.name("italic") {
            let s = italic.as_str();
            let inner = &s[1..s.len() - 1];
            result.push('_');
            result.push_str(&escape_literal(inner));
            result.push('_');
        } else if let Some(link) = caps.name("link") {
            if let Some(link_caps) = link_inner_pattern().captures(link.as_str()) {
                let link_text = &link_caps[1];
                let url = &link_caps[2];
                result.push_str("#link(\"");
                result.push_str(&escape_typst_string(url));
                result.push_str("\")[");
                result.push_str(&escape_literal(link_text));
                result.push(']');
            } else {
                // Should not happen given the outer pattern already matched this
                // shape, but fall back to literal text rather than panicking.
                result.push_str(&escape_literal(link.as_str()));
            }
        }

        last_end = whole.end();
    }
    result.push_str(&escape_literal(&text[last_end..]));
    result
}

/// Escapes Typst's special markup characters so literal user text can never be
/// interpreted as Typst syntax. Backslash is escaped first so we never
/// double-escape the backslashes this function itself introduces.
fn escape_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' | '#' | '*' | '_' | '`' | '<' | '>' | '@' | '$' => {
                out.push('\\');
                out.push(c);
            }
            _ => out.push(c),
        }
    }
    out
}

/// Escapes text destined for inside a Typst string literal (`"..."`).
fn escape_typst_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            _ => out.push(c),
        }
    }
    out
}

fn heading_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^(#{1,3})\s+(.*)$").unwrap())
}

fn unordered_list_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^[-*]\s+(.*)$").unwrap())
}

fn ordered_list_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^\d+\.\s+(.*)$").unwrap())
}

fn link_inner_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| Regex::new(r"^\[([^\]]*)\]\(([^)]*)\)$").unwrap())
}

fn inline_pattern() -> &'static Regex {
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    PATTERN.get_or_init(|| {
        Regex::new(
            r"(?P<code>`[^`]*`)|(?P<bold>\*\*[^*]+\*\*|__[^_]+__)|(?P<italic>\*[^*\n]+\*|_[^_\n]+_)|(?P<link>\[[^\]]*\]\([^)]*\))",
        )
        .unwrap()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_headings_by_level() {
        assert_eq!(convert("# Titel"), "= Titel");
        assert_eq!(convert("## Titel"), "== Titel");
        assert_eq!(convert("### Titel"), "=== Titel");
    }

    #[test]
    fn converts_bold_with_both_syntaxes() {
        assert_eq!(convert("**fett**"), "*fett*");
        assert_eq!(convert("__fett__"), "*fett*");
        assert_eq!(
            convert("Text mit **fett** dazwischen"),
            "Text mit *fett* dazwischen"
        );
    }

    #[test]
    fn converts_italic_with_both_syntaxes() {
        assert_eq!(convert("*kursiv*"), "_kursiv_");
        assert_eq!(convert("_kursiv_"), "_kursiv_");
    }

    #[test]
    fn converts_unordered_list_items() {
        let input = "- Erstens\n* Zweitens";
        assert_eq!(convert(input), "- Erstens\n- Zweitens");
    }

    #[test]
    fn converts_ordered_list_items_to_plus_syntax() {
        let input = "1. Erstens\n2. Zweitens";
        assert_eq!(convert(input), "+ Erstens\n+ Zweitens");
    }

    #[test]
    fn converts_inline_code_unchanged() {
        assert_eq!(
            convert("Befehl: `systemctl restart nginx`"),
            "Befehl: `systemctl restart nginx`"
        );
    }

    #[test]
    fn converts_link_to_typst_link_function() {
        assert_eq!(
            convert("Siehe [Doku](https://example.com/docs)"),
            "Siehe #link(\"https://example.com/docs\")[Doku]"
        );
    }

    #[test]
    fn preserves_blank_line_paragraph_breaks() {
        let input = "Erster Absatz\n\nZweiter Absatz";
        let result = convert(input);
        assert!(result.contains("Erster Absatz\n\nZweiter Absatz"));
    }

    #[test]
    fn escapes_literal_hash_and_dollar_so_they_are_not_interpreted_as_typst_syntax() {
        let result = convert("Preis: 5$ und #wichtig als reiner Text");
        assert_eq!(result, "Preis: 5\\$ und \\#wichtig als reiner Text");
        // The raw, unescaped characters must not appear on their own -- only
        // preceded by the backslash that makes them literal in Typst.
        assert!(result.contains("\\$"));
        assert!(result.contains("\\#"));
    }

    #[test]
    fn escapes_backslash_itself_before_other_escaping() {
        let result = convert(r"C:\Pfad\zur\Datei");
        assert_eq!(result, r"C:\\Pfad\\zur\\Datei");
    }

    #[test]
    fn escapes_special_chars_inside_bold_and_link_text() {
        let bold = convert("**# nicht wichtig**");
        assert_eq!(bold, "*\\# nicht wichtig*");

        let link = convert("[a # b](http://x)");
        assert_eq!(link, "#link(\"http://x\")[a \\# b]");
    }

    #[test]
    fn plain_text_without_markdown_syntax_passes_through() {
        assert_eq!(convert("Ganz normaler Text."), "Ganz normaler Text.");
    }
}
