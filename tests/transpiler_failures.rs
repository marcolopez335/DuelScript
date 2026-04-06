#![cfg(feature = "lua_transpiler")]

use duelscript::lua_transpiler::transpile_lua_to_ds;
use duelscript::parse;

#[test]
fn test_categorize_all_failures() {
    let lua_dir = std::path::Path::new("../CardScripts/official");
    if !lua_dir.exists() { return; }

    let mut total = 0u32;
    let mut parse_ok = 0u32;
    let mut error_categories: std::collections::HashMap<String, u32> = std::collections::HashMap::new();
    let mut sample_by_category: std::collections::HashMap<String, Vec<(u64, String)>> = std::collections::HashMap::new();

    for entry in std::fs::read_dir(lua_dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with('c') || !name.ends_with(".lua") { continue; }

        let source = std::fs::read_to_string(entry.path()).unwrap();
        let passcode: u64 = name[1..name.len()-4].parse().unwrap_or(0);

        let card_name = source.lines()
            .filter(|l| l.starts_with("--"))
            .nth(1).or(source.lines().find(|l| l.starts_with("--")))
            .and_then(|l| l.strip_prefix("--"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();

        let result = transpile_lua_to_ds(&source, passcode, &card_name, None);
        total += 1;

        match parse(&result.ds_content) {
            Ok(_) => parse_ok += 1,
            Err(e) => {
                let err_str = format!("{}", e);
                // Categorize by the expected token
                let category = if err_str.contains("expected") {
                    let expected = err_str.split("expected ").last().unwrap_or("unknown");
                    expected.lines().next().unwrap_or("unknown").trim().to_string()
                } else if err_str.contains("Unknown rule") {
                    let rule = err_str.split("Unknown rule: ").last().unwrap_or("unknown");
                    format!("Unknown rule: {}", rule.lines().next().unwrap_or(""))
                } else {
                    "other".to_string()
                };

                *error_categories.entry(category.clone()).or_insert(0) += 1;
                let samples = sample_by_category.entry(category).or_default();
                if samples.len() < 2 {
                    // Get the failing line from the .ds content
                    let line_hint = err_str.lines()
                        .find(|l| l.contains("-->"))
                        .map(|l| l.trim().to_string())
                        .unwrap_or_default();
                    let relevant_ds = result.ds_content.lines()
                        .enumerate()
                        .skip_while(|(i, _)| {
                            // Find the error line number
                            let line_num = line_hint.split("-->").last()
                                .and_then(|s| s.split(':').nth(0))
                                .and_then(|s| s.trim().parse::<usize>().ok())
                                .unwrap_or(0);
                            *i + 1 < line_num.saturating_sub(1)
                        })
                        .take(3)
                        .map(|(_, l)| l.to_string())
                        .collect::<Vec<_>>()
                        .join("\n");
                    samples.push((passcode, relevant_ds));
                }
            }
        }
    }

    println!("\n=== Full Transpiler Parse Rate: {}/{} ({:.1}%) ===", parse_ok, total, parse_ok as f64 / total as f64 * 100.0);
    println!("\n=== Error Categories (by count) ===");
    let mut sorted: Vec<_> = error_categories.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (cat, count) in &sorted {
        println!("  {:>5} | {}", count, cat);
        if let Some(samples) = sample_by_category.get(*cat) {
            for (id, ds_snippet) in samples {
                println!("         c{}: {}", id, ds_snippet.replace('\n', " | "));
            }
        }
    }
}
