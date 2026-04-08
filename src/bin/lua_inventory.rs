// ============================================================
// lua_inventory — measure Lua API usage in the ProjectIgnis corpus
//
// Walks a directory of c<id>.lua files, counts every reference
// to Duel.X, Card.X, c:X(), Group.X, g:X(), aux.X, Auxiliary.X,
// and Effect.X. Outputs a sorted CSV so we can see exactly which
// API methods drive the most cards — and which are vestigial.
//
// This is the data that drives DuelScript coverage decisions.
// Without it, "what should we implement next" is a guess. With
// it, the priority list writes itself.
//
// Usage:
//
//     cargo run --release --bin lua_inventory -- /Users/marco/git/CardScripts/official > inventory.csv
//
// Output columns:
//
//     namespace, method, total_calls, distinct_files, coverage_pct
//
// Sort: by total_calls descending. The top 50 rows cover most
// real card behavior; the bottom thousands are vestigial / cosmetic.
// ============================================================

use std::collections::HashMap;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let args: Vec<String> = env::args().collect();
    if args.len() < 2 {
        eprintln!("usage: {} <path-to-lua-scripts>", args[0]);
        std::process::exit(2);
    }
    let dir = Path::new(&args[1]);
    if !dir.is_dir() {
        eprintln!("not a directory: {}", dir.display());
        std::process::exit(2);
    }

    // (namespace, method) → (total_calls, set-of-file-ids)
    let mut counts: HashMap<(String, String), (u64, std::collections::HashSet<String>)> =
        HashMap::new();

    let mut total_files = 0u64;
    for entry in fs::read_dir(dir).expect("read scripts dir") {
        let Ok(entry) = entry else { continue };
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("lua") {
            continue;
        }
        // Skip non-card helpers (constant.lua, utility.lua, proc_*.lua, etc.)
        let stem = path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
        if !stem.starts_with('c') {
            continue;
        }
        // Skip the prerelease/test files we don't care about; only true c<id>.lua
        let id_part = &stem[1..];
        if id_part.is_empty() || !id_part.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        total_files += 1;

        let Ok(source) = fs::read_to_string(&path) else { continue };
        for (ns, method) in scan(&source) {
            let entry = counts.entry((ns, method)).or_insert((0, Default::default()));
            entry.0 += 1;
            entry.1.insert(stem.to_string());
        }
    }

    eprintln!("Scanned {} card files", total_files);

    // Sort: by total_calls desc, then by distinct_files desc.
    let mut rows: Vec<(String, String, u64, usize, f64)> = counts.into_iter()
        .map(|((ns, method), (total, files))| {
            let distinct = files.len();
            let coverage = (distinct as f64) / (total_files as f64) * 100.0;
            (ns, method, total, distinct, coverage)
        })
        .collect();
    rows.sort_by(|a, b| b.2.cmp(&a.2).then_with(|| b.3.cmp(&a.3)));

    // CSV output
    println!("namespace,method,total_calls,distinct_files,coverage_pct");
    for (ns, method, total, distinct, pct) in &rows {
        println!("{},{},{},{},{:.2}", ns, method, total, distinct, pct);
    }

    eprintln!("Wrote {} rows", rows.len());
}

/// Scan a Lua source and yield every (namespace, method) symbol we
/// recognize. Recognized patterns:
///
///   Duel.X(           → ("Duel", "X")
///   Card.X(           → ("Card", "X")
///   c:X(   or  c2:X(  → ("Card", "X")    [method receiver]
///   Group.X(          → ("Group", "X")
///   g:X(   or  sg:X(  → ("Group", "X")
///   aux.X(            → ("aux", "X")
///   Auxiliary.X(      → ("aux", "X")
///   Effect.X(         → ("Effect", "X")
///   e:X(   or  e1:X(  → ("Effect", "X")
///
/// We use raw byte scanning rather than regex to keep the binary
/// fast and dependency-free.
fn scan(src: &str) -> Vec<(String, String)> {
    let bytes = src.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        // Skip line comments.
        if i + 1 < bytes.len() && bytes[i] == b'-' && bytes[i + 1] == b'-' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Skip string literals.
        if bytes[i] == b'"' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' { i += 1; }
                i += 1;
            }
            if i < bytes.len() { i += 1; }
            continue;
        }
        if bytes[i] == b'\'' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' { i += 1; }
                i += 1;
            }
            if i < bytes.len() { i += 1; }
            continue;
        }

        // Look for an identifier followed by '.' or ':' followed by another identifier
        // followed by '('.
        if is_ident_start(bytes[i]) {
            let id_start = i;
            while i < bytes.len() && is_ident_continue(bytes[i]) {
                i += 1;
            }
            let id1 = &src[id_start..i];

            if i < bytes.len() && (bytes[i] == b'.' || bytes[i] == b':') {
                let sep = bytes[i];
                let after_sep = i + 1;
                if after_sep < bytes.len() && is_ident_start(bytes[after_sep]) {
                    let m_start = after_sep;
                    let mut m_end = m_start;
                    while m_end < bytes.len() && is_ident_continue(bytes[m_end]) {
                        m_end += 1;
                    }
                    let id2 = &src[m_start..m_end];
                    if m_end < bytes.len() && bytes[m_end] == b'(' {
                        // Classify the namespace.
                        let ns = classify(id1, sep);
                        if let Some(ns) = ns {
                            out.push((ns.to_string(), id2.to_string()));
                        }
                    }
                    i = m_end;
                    continue;
                }
            }
            continue;
        }
        i += 1;
    }
    out
}

fn is_ident_start(b: u8) -> bool {
    (b'a'..=b'z').contains(&b) || (b'A'..=b'Z').contains(&b) || b == b'_'
}
fn is_ident_continue(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

/// Map an identifier + separator to a normalized namespace.
/// `Duel`, `Card`, `Group`, `Effect`, `aux`, `Auxiliary` are recognized
/// directly. Method-call receivers (`c`, `c2`, `g`, `sg`, `e`, `e1`,
/// `tc`, etc.) are recognized by single-letter prefix conventions.
fn classify(name: &str, sep: u8) -> Option<&'static str> {
    // Direct namespaces use `.`
    if sep == b'.' {
        return match name {
            "Duel"      => Some("Duel"),
            "Card"      => Some("Card"),
            "Group"     => Some("Group"),
            "Effect"    => Some("Effect"),
            "aux"       => Some("aux"),
            "Auxiliary" => Some("aux"),
            "Debug"     => Some("Debug"),
            _ => None,
        };
    }
    // Method receivers use `:`. Apply the conventions used by ProjectIgnis:
    //   c, c1, c2, c3, tc, tc1, tc2, sc, fc, mc — Card receiver
    //   e, e1, e2, e3, re — Effect receiver
    //   g, g1, g2, sg, mg, dg, eg, og — Group receiver
    let lower = name.to_ascii_lowercase();
    let stripped: String = lower.chars()
        .take_while(|c| c.is_ascii_alphabetic())
        .collect();
    match stripped.as_str() {
        "c" | "tc" | "sc" | "fc" | "mc" | "ec" | "tg" | "rc" => Some("Card"),
        "e" | "re" => Some("Effect"),
        "g" | "sg" | "mg" | "dg" | "eg" | "og" => Some("Group"),
        _ => None,
    }
}
