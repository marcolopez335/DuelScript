// ============================================================
// Sprint 59: LLM-assisted card generation CLI.
//
// Usage:
//   export ANTHROPIC_API_KEY=sk-ant-...
//   cargo run --bin llm_generate --features "llm,cdb" -- \
//       <cdb_path> <output_dir> [--card <passcode>] [--batch <count>]
//
// Reads card text from BabelCdb, generates .ds via Claude API,
// validates the output, and writes passing cards to the output dir.
// ============================================================

use std::path::Path;
use std::fs;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 3 {
        eprintln!("Usage: llm_generate <cdb_path> <output_dir> [--card <passcode>] [--batch <count>]");
        eprintln!("  ANTHROPIC_API_KEY env var must be set");
        std::process::exit(1);
    }

    let cdb_path = Path::new(&args[1]);
    let output_dir = Path::new(&args[2]);
    fs::create_dir_all(output_dir).expect("create output dir");

    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .expect("ANTHROPIC_API_KEY not set");

    // Parse optional flags
    let mut target_card: Option<u64> = None;
    let mut batch_count: usize = 10;
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--card" => {
                i += 1;
                target_card = Some(args[i].parse().expect("invalid passcode"));
            }
            "--batch" => {
                i += 1;
                batch_count = args[i].parse().expect("invalid count");
            }
            _ => {}
        }
        i += 1;
    }

    // Load CDB
    #[cfg(feature = "cdb")]
    let cdb = {
        match duelscript::CdbReader::open(cdb_path) {
            Ok(reader) => {
                println!("Loaded CDB: {} cards", reader.len());
                reader
            }
            Err(e) => {
                eprintln!("Failed to open CDB: {:?}", e);
                std::process::exit(1);
            }
        }
    };

    let examples_dir = Path::new("cards/test");

    // Build card list
    let cards: Vec<_> = if let Some(id) = target_card {
        match cdb.get(id) {
            Some(c) => vec![c],
            None => {
                eprintln!("Card {} not found in CDB", id);
                std::process::exit(1);
            }
        }
    } else {
        let mut all = cdb.all_cards();
        // Pick a stride-sample
        let stride = (all.len() / batch_count).max(1);
        all.into_iter().step_by(stride).take(batch_count).collect()
    };

    println!("Generating {} cards via Claude API...\n", cards.len());

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let mut success = 0u32;
    let mut fail = 0u32;
    let mut parse_fail = 0u32;

    for card in &cards {
        let type_line = card.ds_type_line();
        let stats = if card.is_monster() {
            format!("ATK/{} DEF/{} Level/{}", card.atk_str(), card.def_str(), card.actual_level())
        } else {
            String::new()
        };

        let prompt = duelscript::llm::build_prompt(
            &card.name,
            &card.desc,
            &type_line,
            &stats,
            examples_dir,
        );

        print!("  c{} ({})... ", card.id, card.name);

        // Call Claude
        let result = rt.block_on(duelscript::llm::call_claude(&prompt, &api_key));
        match result {
            Ok(ds_content) => {
                // Validate
                match duelscript::llm::validate_generated(&ds_content) {
                    Ok(()) => {
                        let path = output_dir.join(format!("c{}.ds", card.id));
                        fs::write(&path, &ds_content).expect("write file");
                        println!("OK");
                        success += 1;
                    }
                    Err(e) => {
                        println!("VALIDATE FAIL: {}", e);
                        // Write the failed output for debugging
                        let path = output_dir.join(format!("c{}.failed.ds", card.id));
                        fs::write(&path, &ds_content).ok();
                        parse_fail += 1;
                    }
                }
            }
            Err(e) => {
                println!("API FAIL: {}", e);
                fail += 1;
            }
        }
    }

    println!("\n=== LLM Generation Complete ===");
    println!("  Success:        {}", success);
    println!("  Validate fail:  {}", parse_fail);
    println!("  API fail:       {}", fail);
    println!("  Total:          {}", cards.len());
    println!("  Success rate:   {:.1}%", success as f64 / cards.len() as f64 * 100.0);
}
