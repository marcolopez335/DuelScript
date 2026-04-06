#![cfg(feature = "lua_transpiler")]

use duelscript::lua_transpiler::transpile_lua_to_ds;
use duelscript::parse;

#[test]
fn test_transpiler_parse_rate() {
    let lua_dir = std::path::Path::new("../CardScripts/official");
    if !lua_dir.exists() { return; }

    let mut total = 0u32;
    let mut parse_ok = 0u32;
    let mut sample_failures: Vec<(String, String)> = Vec::new();

    for entry in std::fs::read_dir(lua_dir).unwrap().take(500) {
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
                if sample_failures.len() < 5 {
                    sample_failures.push((
                        name.clone(),
                        format!("ERR: {}\nDS:\n{}", e, &result.ds_content[..result.ds_content.len().min(300)]),
                    ));
                }
            }
        }
    }

    println!("\nTranspiler parse rate: {}/{} ({:.1}%)", parse_ok, total, parse_ok as f64 / total as f64 * 100.0);
    for (name, err) in &sample_failures {
        println!("\n=== {} ===\n{}", name, err);
    }
}
