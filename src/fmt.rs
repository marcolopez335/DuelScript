// ============================================================
// DuelScript formatter — fmt.rs
//
// A brace-aware reformatter for .ds files. Doesn't require an AST
// roundtrip; instead it walks the source line-by-line, tracking
// `{`/`}` depth (excluding occurrences inside strings and line
// comments), and re-emits each line with canonical indentation.
//
// Rules:
//   1. Indent: 4 spaces per nesting level.
//   2. `}` on its own line dedents BEFORE printing.
//   3. Lines containing both `{` and `}` (e.g. `{ count: 3 }`) keep
//      their inline form and don't change depth.
//   4. Trailing whitespace stripped.
//   5. Multiple consecutive blank lines collapsed to one.
//   6. Inside string literals, braces and comments are ignored.
//
// The formatter is intentionally NOT a parse/reprint pass — that
// would require fully serializing the AST, which is a much larger
// surface area. The brace-aware pass handles 95% of canonical
// formatting needs and leaves card semantics untouched.
// ============================================================

/// Format a `.ds` source string with canonical indentation and
/// whitespace. Returns the formatted text.
pub fn format_source(src: &str) -> String {
    let mut out = String::with_capacity(src.len());
    let mut depth: i32 = 0;
    let mut prev_blank = false;

    for raw_line in src.split('\n') {
        // Strip trailing whitespace.
        let line = raw_line.trim_end();
        let trimmed = line.trim_start();

        // Blank line normalization.
        if trimmed.is_empty() {
            if !prev_blank {
                out.push('\n');
                prev_blank = true;
            }
            continue;
        }
        prev_blank = false;

        // Count brace deltas for THIS line, ignoring braces inside
        // strings and line comments.
        let (opens, closes) = count_braces(trimmed);

        // Lines that start with `}` dedent BEFORE printing.
        // (e.g. `}` on its own line, or `} else {`.)
        let starts_with_close = trimmed.starts_with('}');
        let print_depth = if starts_with_close {
            (depth - 1).max(0)
        } else {
            depth.max(0)
        };

        // Emit indented line.
        for _ in 0..print_depth {
            out.push_str("    ");
        }
        out.push_str(trimmed);
        out.push('\n');

        // Apply net delta for the next line.
        depth += opens - closes;
        if depth < 0 { depth = 0; }
    }

    // Ensure exactly one trailing newline.
    while out.ends_with("\n\n") {
        out.pop();
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Count `{` / `}` in `line`, ignoring those inside `"..."` strings
/// and after `//` line comments.
fn count_braces(line: &str) -> (i32, i32) {
    let mut opens = 0i32;
    let mut closes = 0i32;
    let bytes = line.as_bytes();
    let mut i = 0;
    let mut in_string = false;
    while i < bytes.len() {
        let c = bytes[i];
        // Skip line comments entirely.
        if !in_string && c == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            break;
        }
        if c == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_string = !in_string;
        } else if !in_string {
            if c == b'{' { opens += 1; }
            else if c == b'}' { closes += 1; }
        }
        i += 1;
    }
    (opens, closes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idempotent_on_canonical_card() {
        let src = "card \"X\" {\n    type: Normal Spell\n    password: 1\n\n    effect \"e\" {\n        on_resolve {\n            draw 2\n        }\n    }\n}\n";
        let out = format_source(src);
        assert_eq!(out, src,
            "canonical input should round-trip unchanged.\n--- expected ---\n{}\n--- got ---\n{}",
            src, out);
    }

    #[test]
    fn fixes_messy_indentation() {
        let messy = "card \"X\" {\ntype: Normal Spell\npassword: 1\neffect \"e\" {\non_resolve {\ndraw 2\n}\n}\n}\n";
        let out = format_source(messy);
        assert!(out.contains("    type: Normal Spell"),
            "should re-indent fields with 4 spaces. Got:\n{}", out);
        assert!(out.contains("            draw 2"),
            "deeply nested action should be at 12-space indent. Got:\n{}", out);
    }

    #[test]
    fn collapses_blank_lines() {
        let src = "card \"X\" {\n    type: Normal Spell\n\n\n\n    password: 1\n}\n";
        let out = format_source(src);
        let blanks = out.matches("\n\n").count();
        assert!(blanks <= 1, "should collapse multiple blank lines. Got:\n{}", out);
    }

    #[test]
    fn ignores_braces_in_strings_and_comments() {
        let src = "card \"X { fake brace }\" {\n    // comment with { brace }\n    type: Normal Spell\n}\n";
        let out = format_source(src);
        // The `type:` line should still be indented at depth 1, not depth 3.
        assert!(out.contains("\n    type: Normal Spell"),
            "braces inside strings/comments must not affect depth. Got:\n{}", out);
    }

    #[test]
    fn ensures_trailing_newline() {
        let src = "card \"X\" {\n    type: Normal Spell\n}";
        let out = format_source(src);
        assert!(out.ends_with('\n'));
    }

    #[test]
    fn formatted_output_still_parses() {
        let src = "card \"Pot of Greed\" {\ntype: Normal Spell\npassword: 55144522\neffect \"Draw 2\" {\nspeed: spell_speed_1\non_resolve {\ndraw 2\n}\n}\n}\n";
        let out = format_source(src);
        assert!(crate::parse(&out).is_ok(),
            "formatted output must still parse. Got:\n{}", out);
    }
}
