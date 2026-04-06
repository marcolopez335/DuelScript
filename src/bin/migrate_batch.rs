// ============================================================
// Batch Migration CLI — writes .ds files from Lua scripts
// Usage: cargo run --bin migrate_batch -- <lua_dir> <output_dir>
// ============================================================

use duelscript::migrate::{migrate_directory, Confidence};
use std::path::Path;
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: migrate_batch <lua_dir> <output_dir> [--high-only]");
        eprintln!("  lua_dir:    Path to CardScripts/official/");
        eprintln!("  output_dir: Where to write c<ID>.ds files");
        eprintln!("  --high-only: Only write HIGH confidence cards");
        std::process::exit(1);
    }

    let lua_dir = Path::new(&args[1]);
    let output_dir = Path::new(&args[2]);
    let high_only = args.get(3).map(|s| s == "--high-only").unwrap_or(false);

    if !lua_dir.exists() {
        eprintln!("Lua directory not found: {}", lua_dir.display());
        std::process::exit(1);
    }

    fs::create_dir_all(output_dir).expect("Failed to create output directory");

    println!("Migrating from {} to {}", lua_dir.display(), output_dir.display());
    if high_only { println!("  (HIGH confidence only)"); }

    let results = migrate_directory(lua_dir);

    let mut written = 0u32;
    let mut skipped = 0u32;
    let mut by_confidence = [0u32; 4];

    for result in &results {
        match result.confidence {
            Confidence::Full   => by_confidence[0] += 1,
            Confidence::High   => by_confidence[1] += 1,
            Confidence::Medium => by_confidence[2] += 1,
            Confidence::Low    => by_confidence[3] += 1,
        }

        if high_only && !matches!(result.confidence, Confidence::Full | Confidence::High) {
            skipped += 1;
            continue;
        }

        // Try to parse the generated content to validate it
        match duelscript::parse(&result.ds_content) {
            Ok(_) => {
                let filename = format!("c{}.ds", result.passcode);
                let path = output_dir.join(&filename);
                fs::write(&path, &result.ds_content).unwrap_or_else(|e| {
                    eprintln!("  Failed to write {}: {}", filename, e);
                });
                written += 1;
            }
            Err(_) => {
                skipped += 1;
            }
        }
    }

    println!("\n=== Migration Complete ===");
    println!("Total Lua scripts: {}", results.len());
    println!("Confidence: FULL={} HIGH={} MEDIUM={} LOW={}",
        by_confidence[0], by_confidence[1], by_confidence[2], by_confidence[3]);
    println!("Written:  {}", written);
    println!("Skipped:  {}", skipped);
    println!("Output:   {}", output_dir.display());
}
