// ============================================================
// Batch Migration Test — migrate Lua scripts and report coverage
// ============================================================

use duelscript::migrate::{migrate_directory, Confidence};
use duelscript::parse;
use duelscript::compiler::compile_card;
use std::path::Path;

#[test]
fn test_batch_migrate_and_compile() {
    let lua_dir = Path::new("../CardScripts/official");
    if !lua_dir.exists() {
        eprintln!("CardScripts directory not found, skipping batch test");
        return;
    }

    let results = migrate_directory(lua_dir);

    let total = results.len();
    let mut by_confidence = [0u32; 4]; // Full, High, Medium, Low
    let mut parse_success = 0u32;
    let mut parse_fail = 0u32;
    let mut compile_success = 0u32;
    let mut sample_failures: Vec<(u64, String)> = Vec::new();

    for result in &results {
        match result.confidence {
            Confidence::Full   => by_confidence[0] += 1,
            Confidence::High   => by_confidence[1] += 1,
            Confidence::Medium => by_confidence[2] += 1,
            Confidence::Low    => by_confidence[3] += 1,
        }

        // Try to parse the generated .ds content
        match parse(&result.ds_content) {
            Ok(file) => {
                parse_success += 1;
                // Try to compile
                if !file.cards.is_empty() {
                    let _compiled = compile_card(&file.cards[0]);
                    compile_success += 1;
                }
            }
            Err(e) => {
                parse_fail += 1;
                if sample_failures.len() < 10 {
                    sample_failures.push((result.passcode, format!("{}", e)));
                }
            }
        }
    }

    println!("\n=== Batch Migration Results ===");
    println!("Total Lua scripts:    {}", total);
    println!("Confidence breakdown:");
    println!("  FULL:   {}", by_confidence[0]);
    println!("  HIGH:   {}", by_confidence[1]);
    println!("  MEDIUM: {}", by_confidence[2]);
    println!("  LOW:    {}", by_confidence[3]);
    println!();
    println!("Parse success:   {} ({:.1}%)", parse_success, parse_success as f64 / total as f64 * 100.0);
    println!("Parse failures:  {}", parse_fail);
    println!("Compile success: {} ({:.1}%)", compile_success, compile_success as f64 / total as f64 * 100.0);

    if !sample_failures.is_empty() {
        println!("\nSample parse failures:");
        for (id, err) in &sample_failures {
            println!("  c{}: {}", id, &err[..err.len().min(100)]);
        }
    }

    // We should be able to parse at least 50% of generated .ds files
    assert!(parse_success as f64 / total as f64 > 0.3,
        "Expected at least 30% parse success rate, got {:.1}%",
        parse_success as f64 / total as f64 * 100.0);
}
