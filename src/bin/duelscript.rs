// ============================================================
// DuelScript CLI — main.rs
// Usage:
//   duelscript check cards/           validate all .ds files
//   duelscript check ash_blossom.ds   validate a single file
//   duelscript inspect ash_blossom.ds pretty-print the AST
// ============================================================

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};
use duelscript::{parse_v2, validate_v2};

fn main() {
    let args: Vec<String> = env::args().collect();

    if args.len() < 3 {
        print_usage();
        process::exit(1);
    }

    let command = &args[1];
    let target  = Path::new(&args[2]);

    match command.as_str() {
        "check"   => cmd_check(target),
        "inspect" => cmd_inspect(target),
        other => {
            eprintln!("Unknown command: '{}'\n", other);
            print_usage();
            process::exit(1);
        }
    }
}

// ── Commands ──────────────────────────────────────────────────

fn cmd_check(target: &Path) {
    let files = collect_ds_files(target);
    if files.is_empty() {
        eprintln!("No .ds files found at: {}", target.display());
        process::exit(1);
    }

    let mut total_errors   = 0;
    let mut total_warnings = 0;
    let mut files_checked  = 0;
    let mut all_clean      = true;

    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s)  => s,
            Err(e) => {
                eprintln!("Could not read {}: {}", path.display(), e);
                continue;
            }
        };

        match parse_v2(&source) {
            Err(e) => {
                println!("✗ {} — PARSE ERROR: {}", path.display(), e);
                total_errors += 1;
                all_clean = false;
            }
            Ok(file) => {
                let report = validate_v2(&file);
                files_checked += 1;

                let errors: Vec<_> = report.errors.iter()
                    .filter(|e| e.severity == duelscript::v2::validator::Severity::Error)
                    .collect();
                let warnings: Vec<_> = report.errors.iter()
                    .filter(|e| e.severity == duelscript::v2::validator::Severity::Warning)
                    .collect();

                if errors.is_empty() && warnings.is_empty() {
                    println!("✓ {}", path.display());
                } else {
                    all_clean = false;
                    println!("\n── {} ──", path.display());
                    for e in &errors {
                        println!("  [ERROR] {}: {}", e.card, e.message);
                    }
                    for w in &warnings {
                        println!("  [WARN ] {}: {}", w.card, w.message);
                    }
                    total_errors   += errors.len();
                    total_warnings += warnings.len();
                }
            }
        }
    }

    println!(
        "\n── Summary: {} file(s) checked, {} error(s), {} warning(s) ──",
        files_checked, total_errors, total_warnings
    );

    if !all_clean {
        process::exit(1);
    }
}

fn cmd_inspect(target: &Path) {
    if target.is_dir() {
        eprintln!("'inspect' works on a single file, not a directory");
        process::exit(1);
    }

    let source = fs::read_to_string(target).unwrap_or_else(|e| {
        eprintln!("Could not read {}: {}", target.display(), e);
        process::exit(1);
    });

    let file = parse_v2(&source).unwrap_or_else(|e| {
        eprintln!("Parse error in {}: {}", target.display(), e);
        process::exit(1);
    });

    for card in &file.cards {
        println!("╔══════════════════════════════════════");
        println!("║  Card: {}", card.name);
        println!("╠══════════════════════════════════════");
        println!("║  Types:      {:?}", card.fields.card_types);
        if let Some(attr)  = &card.fields.attribute  { println!("║  Attribute:  {:?}", attr); }
        if let Some(race)  = &card.fields.race        { println!("║  Race:       {:?}", race); }
        if let Some(level) = card.fields.level        { println!("║  Level:      {}", level); }
        if let Some(rank)  = card.fields.rank         { println!("║  Rank:       {}", rank); }
        if let Some(link)  = card.fields.link         { println!("║  Link:       {}", link); }
        if let Some(scale) = card.fields.scale        { println!("║  Scale:      {}", scale); }
        if let Some(atk)   = &card.fields.atk         { println!("║  ATK:        {:?}", atk); }
        if let Some(def)   = &card.fields.def         { println!("║  DEF:        {:?}", def); }

        println!("║  Effects: {}", card.effects.len());
        for (i, effect) in card.effects.iter().enumerate() {
            println!("║  ┌─ Effect {} \"{}\"", i + 1, effect.name);
            println!("║  │  Speed:     {:?}", effect.speed);
            println!("║  │  Mandatory: {}", effect.mandatory);
            println!("║  │  Timing:    {:?}", effect.timing);
            if let Some(c) = &effect.condition { println!("║  │  Condition: {:?}", c); }
            if let Some(t) = &effect.trigger   { println!("║  │  Trigger:   {:?}", t); }
            if !effect.cost.is_empty() {
                println!("║  │  Cost:      {:?}", effect.cost);
            }
            println!("║  │  Resolve actions: {}", effect.resolve.len());
            for action in &effect.resolve {
                println!("║  │    {:?}", action);
            }
        }

        println!("╚══════════════════════════════════════\n");
    }
}

// ── File Collection ───────────────────────────────────────────

fn collect_ds_files(target: &Path) -> Vec<PathBuf> {
    if target.is_file() {
        if target.extension().map_or(false, |e| e == "ds") {
            return vec![target.to_path_buf()];
        }
        return vec![];
    }

    if target.is_dir() {
        let mut results = Vec::new();
        collect_ds_recursive(target, &mut results);
        results.sort();
        return results;
    }

    vec![]
}

fn collect_ds_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_ds_recursive(&path, out);
        } else if path.extension().map_or(false, |e| e == "ds") {
            out.push(path);
        }
    }
}

// ── Usage ─────────────────────────────────────────────────────

fn print_usage() {
    println!("DuelScript CLI v{}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("USAGE:");
    println!("  duelscript <command> <target>");
    println!();
    println!("COMMANDS:");
    println!("  check   <file.ds | directory>   Validate .ds files for errors and warnings");
    println!("  inspect <file.ds>               Pretty-print the parsed AST of a card");
    println!();
    println!("EXAMPLES:");
    println!("  duelscript check cards/");
    println!("  duelscript check cards/v2_test/pot_of_greed.ds");
    println!("  duelscript inspect cards/v2_test/stardust_dragon.ds");
}
