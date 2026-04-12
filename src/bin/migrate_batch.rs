// ============================================================
// Batch Migration CLI — transpile Lua scripts to .ds with CDB stats
// Usage: cargo run --bin migrate_batch --features "cdb,lua_transpiler" -- <lua_dir> <cdb_path> <output_dir> [--all]
// ============================================================

use std::path::Path;
use std::fs;
use std::collections::HashSet;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 4 {
        eprintln!("Usage: migrate_batch <lua_dir> <cdb_path> <output_dir> [--all]");
        eprintln!("  lua_dir:    Path to CardScripts/official/");
        eprintln!("  cdb_path:   Path to BabelCdb/cards.cdb");
        eprintln!("  output_dir: Where to write c<ID>.ds files");
        eprintln!("  --all:      Write ALL cards (default: HIGH confidence only)");
        std::process::exit(1);
    }

    let lua_dir = Path::new(&args[1]);
    #[cfg_attr(not(feature = "cdb"), allow(unused_variables))]
    let cdb_path = Path::new(&args[2]);
    let output_dir = Path::new(&args[3]);
    let write_all = args.get(4).map(|s| s == "--all").unwrap_or(false);

    if !lua_dir.exists() {
        eprintln!("Lua directory not found: {}", lua_dir.display());
        std::process::exit(1);
    }

    fs::create_dir_all(output_dir).expect("Failed to create output directory");

    // Load CDB
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
            eprintln!("CDB not found at {}", cdb_path.display());
            None
        }
    };
    #[cfg(not(feature = "cdb"))]
    #[allow(unused_variables)]
    let cdb: Option<()> = None;

    println!("Migrating from {} to {}", lua_dir.display(), output_dir.display());
    println!("Mode: {}", if write_all { "ALL cards" } else { "parseable cards only" });

    // Protected hand-verified card IDs
    let protected: HashSet<u64> = [
        55144522, 53129443, 83764718, 14558127, 84013237,
        41420027, 10000030, 82732705, 10080320, 56747793,
        44508094, 1861629,
    ].iter().copied().collect();

    let Ok(entries) = fs::read_dir(lua_dir) else {
        eprintln!("Cannot read directory");
        return;
    };

    let mut lua_files: Vec<_> = entries.flatten()
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_string();
            name.starts_with('c') && name.ends_with(".lua")
        })
        .collect();
    lua_files.sort_by_key(|e| e.file_name());

    let total = lua_files.len();
    let mut written = 0u32;
    let mut skipped = 0u32;
    let mut parse_ok = 0u32;
    let mut parse_fail = 0u32;
    let mut protected_count = 0u32;

    #[cfg(feature = "lua_transpiler")]
    let mut by_accuracy = [0u32; 5]; // Full, High, Partial, StructureOnly, Failed

    // Sprint 50: collect unmapped calls across Partial-tier cards
    // so we know which Duel.X methods to prioritize.
    #[cfg(feature = "lua_transpiler")]
    let mut unmapped_freq: std::collections::HashMap<String, u32> = std::collections::HashMap::new();

    for entry in &lua_files {
        let name = entry.file_name().to_string_lossy().to_string();
        let id_str = name.trim_start_matches('c').trim_end_matches(".lua");
        let Ok(passcode) = id_str.parse::<u64>() else { continue };

        if protected.contains(&passcode) {
            protected_count += 1;
            continue;
        }

        let Ok(source) = fs::read_to_string(entry.path()) else { continue };

        // Extract card names from Lua comment header.
        // Line 1 is typically the Japanese name, line 2 is English.
        let comment_lines: Vec<&str> = source.lines()
            .filter(|l| l.starts_with("--") && !l.starts_with("---"))
            .take(2)
            .collect();
        let _jp_name = comment_lines.first()
            .and_then(|l| l.strip_prefix("--"))
            .map(|s| s.trim().to_string());
        let card_name = comment_lines.get(1)
            .or(comment_lines.first())
            .and_then(|l| l.strip_prefix("--"))
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|| format!("Card {}", passcode));

        // Get CDB data
        #[cfg(feature = "cdb")]
        let cdb_card = cdb.as_ref().and_then(|c| c.get(passcode));
        #[cfg(not(feature = "cdb"))]
        #[allow(unused_variables)]
        let cdb_card: Option<&()> = None;

        // Use transpiler if available, otherwise fall back to old migrator
        #[cfg(all(feature = "lua_transpiler", feature = "cdb"))]
        let (ds_content, _accuracy) = {
            let result = duelscript::lua_transpiler::transpile_lua_to_ds(
                &source, passcode, &card_name, cdb_card,
            );
            match result.accuracy {
                duelscript::lua_transpiler::TranspileAccuracy::Full          => by_accuracy[0] += 1,
                duelscript::lua_transpiler::TranspileAccuracy::High          => by_accuracy[1] += 1,
                duelscript::lua_transpiler::TranspileAccuracy::Partial       => {
                    by_accuracy[2] += 1;
                    for call in &result.unmapped_calls {
                        *unmapped_freq.entry(call.clone()).or_insert(0) += 1;
                    }
                }
                duelscript::lua_transpiler::TranspileAccuracy::StructureOnly => by_accuracy[3] += 1,
                duelscript::lua_transpiler::TranspileAccuracy::Failed        => by_accuracy[4] += 1,
            };
            (result.ds_content, result.accuracy)
        };

        #[cfg(not(all(feature = "lua_transpiler", feature = "cdb")))]
        let (ds_content, _accuracy) = {
            #[cfg(feature = "cdb")]
            let result = duelscript::migrate::generate_from_lua_with_cdb(
                &source, passcode, &card_name, cdb_card,
            );
            #[cfg(not(feature = "cdb"))]
            let result = duelscript::migrate::generate_from_lua(
                &source, passcode, &card_name,
            );
            (result.ds_content, result.confidence)
        };

        // Try to parse the generated content
        match duelscript::parse(&ds_content) {
            Ok(_) => {
                parse_ok += 1;
                let filename = format!("c{}.ds", passcode);
                let path = output_dir.join(&filename);
                if write_all || true { // Write all parseable cards
                    fs::write(&path, &ds_content).unwrap_or_else(|e| {
                        eprintln!("  Failed to write {}: {}", filename, e);
                    });
                    written += 1;
                }
            }
            Err(e) => {
                parse_fail += 1;
                skipped += 1;
                if parse_fail <= 5 {
                    eprintln!("Parse fail c{}: {:?}", passcode, e);
                    let failed_path = output_dir.join(format!("c{}.failed.ds", passcode));
                    let _ = fs::write(&failed_path, &ds_content);
                }
            }
        }
    }

    println!("\n=== Migration Complete ===");
    println!("Total Lua scripts: {}", total);
    println!("Protected:    {} (hand-verified, not overwritten)", protected_count);
    println!("Parse OK:     {} ({:.1}%)", parse_ok, parse_ok as f64 / total as f64 * 100.0);
    println!("Parse fail:   {}", parse_fail);
    println!("Written:      {}", written);
    println!("Skipped:      {}", skipped);

    #[cfg(feature = "lua_transpiler")]
    {
        println!("\nTranspiler accuracy:");
        println!("  Full:          {}", by_accuracy[0]);
        println!("  High:          {}", by_accuracy[1]);
        println!("  Partial:       {}", by_accuracy[2]);
        println!("  StructureOnly: {}", by_accuracy[3]);
        println!("  Failed:        {}", by_accuracy[4]);

        // Sprint 50: top unmapped calls in Partial-tier cards
        if !unmapped_freq.is_empty() {
            let mut sorted: Vec<_> = unmapped_freq.into_iter().collect();
            sorted.sort_by(|a, b| b.1.cmp(&a.1));
            println!("\nTop unmapped calls (Partial tier):");
            for (call, count) in sorted.iter().take(30) {
                println!("  {:6}  {}", count, call);
            }
        }
    }

    println!("\nOutput: {}", output_dir.display());
}
