// ============================================================
// Batch Migration CLI — writes .ds files from Lua scripts + CDB
// Usage: cargo run --bin migrate_batch --features cdb -- <lua_dir> <cdb_path> <output_dir> [--high-only]
// ============================================================

use std::path::Path;
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: migrate_batch <lua_dir> <cdb_path> <output_dir> [--high-only]");
        eprintln!("  lua_dir:    Path to CardScripts/official/");
        eprintln!("  cdb_path:   Path to BabelCdb/cards.cdb");
        eprintln!("  output_dir: Where to write c<ID>.ds files");
        eprintln!("  --high-only: Only write HIGH confidence cards");
        std::process::exit(1);
    }

    let lua_dir = Path::new(&args[1]);
    let cdb_path = Path::new(&args[2]);
    let output_dir = Path::new(&args[3]);
    let high_only = args.get(4).map(|s| s == "--high-only").unwrap_or(false);

    if !lua_dir.exists() {
        eprintln!("Lua directory not found: {}", lua_dir.display());
        std::process::exit(1);
    }

    fs::create_dir_all(output_dir).expect("Failed to create output directory");

    // Load CDB if available
    #[cfg(feature = "cdb")]
    let cdb = {
        if cdb_path.exists() {
            match duelscript::CdbReader::open(cdb_path) {
                Ok(reader) => {
                    println!("Loaded CDB: {} cards", reader.len());
                    Some(reader)
                }
                Err(e) => {
                    eprintln!("Failed to open CDB: {:?}", e);
                    None
                }
            }
        } else {
            eprintln!("CDB not found at {}, generating without stats", cdb_path.display());
            None
        }
    };
    #[cfg(not(feature = "cdb"))]
    let cdb: Option<()> = {
        eprintln!("CDB support not enabled. Build with: --features cdb");
        None
    };

    println!("Migrating from {} to {}", lua_dir.display(), output_dir.display());

    let mut total = 0u32;
    let mut written = 0u32;
    let mut skipped = 0u32;
    let mut parse_fail = 0u32;
    let mut by_confidence = [0u32; 4];

    // Protected hand-verified card IDs — don't overwrite these
    let protected: std::collections::HashSet<u64> = [
        55144522, 53129443, 83764718, 14558127, 84013237,
        41420027, 10000030, 82732705, 10080320, 56747793,
        44508094, 1861629,
    ].iter().copied().collect();

    let Ok(entries) = fs::read_dir(lua_dir) else {
        eprintln!("Cannot read lua directory");
        return;
    };

    let mut lua_files: Vec<_> = entries.flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with('c') && name.ends_with(".lua")
        })
        .collect();
    lua_files.sort_by_key(|e| e.file_name());

    for entry in &lua_files {
        let name = entry.file_name().to_string_lossy().to_string();
        let id_str = name.trim_start_matches('c').trim_end_matches(".lua");
        let Ok(passcode) = id_str.parse::<u64>() else { continue };

        // Don't overwrite hand-verified cards
        if protected.contains(&passcode) {
            continue;
        }

        let Ok(source) = fs::read_to_string(entry.path()) else { continue };

        // Extract card name from comments (prefer English name)
        let card_name = source.lines()
            .filter(|l| l.starts_with("--") && !l.starts_with("---"))
            .nth(1)
            .or_else(|| source.lines().find(|l| l.starts_with("--")))
            .and_then(|l| l.strip_prefix("--"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("Card {}", passcode));

        // Get CDB data if available
        #[cfg(feature = "cdb")]
        let cdb_card = cdb.as_ref().and_then(|c| c.get(passcode));
        #[cfg(not(feature = "cdb"))]
        let cdb_card: Option<&()> = None;

        #[cfg(feature = "cdb")]
        let result = duelscript::migrate::generate_from_lua_with_cdb(
            &source, passcode, &card_name, cdb_card,
        );
        #[cfg(not(feature = "cdb"))]
        let result = duelscript::migrate::generate_from_lua(&source, passcode, &card_name);

        total += 1;

        match result.confidence {
            duelscript::Confidence::Full   => by_confidence[0] += 1,
            duelscript::Confidence::High   => by_confidence[1] += 1,
            duelscript::Confidence::Medium => by_confidence[2] += 1,
            duelscript::Confidence::Low    => by_confidence[3] += 1,
        }

        if high_only && !matches!(result.confidence, duelscript::Confidence::Full | duelscript::Confidence::High) {
            skipped += 1;
            continue;
        }

        match duelscript::parse(&result.ds_content) {
            Ok(_) => {
                let filename = format!("c{}.ds", passcode);
                let path = output_dir.join(&filename);
                fs::write(&path, &result.ds_content).unwrap_or_else(|e| {
                    eprintln!("  Failed to write {}: {}", filename, e);
                });
                written += 1;
            }
            Err(_) => {
                parse_fail += 1;
                skipped += 1;
            }
        }
    }

    println!("\n=== Migration Complete ===");
    println!("Total Lua scripts: {}", total);
    println!("Confidence: FULL={} HIGH={} MEDIUM={} LOW={}",
        by_confidence[0], by_confidence[1], by_confidence[2], by_confidence[3]);
    println!("Written:     {}", written);
    println!("Parse fail:  {}", parse_fail);
    println!("Skipped:     {}", skipped);
    println!("Protected:   {} (hand-verified, not overwritten)", protected.len());
    println!("Output:      {}", output_dir.display());
}
