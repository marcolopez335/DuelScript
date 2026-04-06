// ============================================================
// DuelScript CLI — main.rs
// Usage:
//   duelscript check cards/           validate all .ds files
//   duelscript check ash_blossom.ds   validate a single file
//   duelscript inspect ash_blossom.ds pretty-print the AST
//   duelscript fmt ash_blossom.ds     auto-format a .ds file
//   duelscript fmt cards/             auto-format all .ds files
// ============================================================

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};
use duelscript::{parse, validator::{validate, ValidationReport, Severity}};

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
        "fmt"     => cmd_fmt(target),
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

        match parse(&source) {
            Err(e) => {
                println!("✗ {} — PARSE ERROR: {}", path.display(), e);
                total_errors += 1;
                all_clean = false;
            }
            Ok(file) => {
                let all_errors = validate(&file);
                let report = ValidationReport::from(all_errors);
                files_checked += 1;

                if report.is_clean() && report.warnings.is_empty() {
                    println!("✓ {}", path.display());
                } else {
                    all_clean = false;
                    println!("\n── {} ──", path.display());
                    for e in &report.errors {
                        println!("  [ERROR] {}: {}", e.card_name, e.message);
                    }
                    for w in &report.warnings {
                        println!("  [WARN ] {}: {}", w.card_name, w.message);
                    }
                    total_errors   += report.errors.len();
                    total_warnings += report.warnings.len();
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

    let file = parse(&source).unwrap_or_else(|e| {
        eprintln!("Parse error in {}: {}", target.display(), e);
        process::exit(1);
    });

    for card in &file.cards {
        println!("╔══════════════════════════════════════");
        println!("║  Card: {}", card.name);
        println!("╠══════════════════════════════════════");
        println!("║  Types:      {:?}", card.card_types);
        if let Some(attr)  = &card.attribute { println!("║  Attribute:  {:?}", attr); }
        if let Some(race)  = &card.race      { println!("║  Race:       {:?}", race); }
        if let Some(level) = card.level      { println!("║  Level:      {}", level); }
        if let Some(rank)  = card.rank       { println!("║  Rank:       {}", rank); }
        if let Some(link)  = card.link       { println!("║  Link:       {}", link); }
        if let Some(scale) = card.scale      { println!("║  Scale:      {}", scale); }

        if let Some(atk) = &card.stats.atk { println!("║  ATK:        {:?}", atk); }
        if let Some(def) = &card.stats.def { println!("║  DEF:        {:?}", def); }

        if !card.archetypes.is_empty() {
            println!("║  Archetypes: {:?}", card.archetypes);
        }
        if !card.link_arrows.is_empty() {
            println!("║  Arrows:     {:?}", card.link_arrows);
        }
        if !card.summon_conditions.is_empty() {
            println!("║  Summon Conditions:");
            for rule in &card.summon_conditions {
                println!("║    {:?}", rule);
            }
        }

        println!("║  Effects: {}", card.effects.len());
        for (i, effect) in card.effects.iter().enumerate() {
            println!("║  ┌─ Effect {} {:?}", i + 1, effect.name);
            println!("║  │  Speed:     {:?}", effect.body.speed);
            println!("║  │  Frequency: {:?}", effect.body.frequency);
            println!("║  │  Optional:  {}", effect.body.optional);
            println!("║  │  Timing:    {:?}", effect.body.timing);
            if let Some(c) = &effect.body.condition { println!("║  │  Condition: {:?}", c); }
            if let Some(t) = &effect.body.trigger   { println!("║  │  Trigger:   {:?}", t); }
            if !effect.body.cost.is_empty() {
                println!("║  │  Cost:      {:?}", effect.body.cost);
            }
            println!("║  │  Resolve actions: {}", effect.body.on_resolve.len());
            for action in &effect.body.on_resolve {
                println!("║  │    {:?}", action);
            }
        }

        if !card.continuous_effects.is_empty() {
            println!("║  Continuous Effects: {}", card.continuous_effects.len());
        }
        if !card.replacement_effects.is_empty() {
            println!("║  Replacement Effects: {}", card.replacement_effects.len());
            for re in &card.replacement_effects {
                println!("║    Instead of: {:?}", re.instead_of);
            }
        }
        if let Some(wc) = &card.win_condition {
            println!("║  Win Condition: {:?} → {:?}", wc.trigger, wc.result);
        }
        println!("╚══════════════════════════════════════\n");
    }
}

fn cmd_fmt(target: &Path) {
    // Formatter stub — in v0.4 this will rewrite .ds files with canonical style
    // For now it validates and reports what would change
    let files = collect_ds_files(target);
    if files.is_empty() {
        eprintln!("No .ds files found at: {}", target.display());
        process::exit(1);
    }

    println!("duelscript fmt — formatter (v0.3 stub)");
    println!("Full formatting pass coming in v0.4.");
    println!("Checking {} file(s) for parse validity:\n", files.len());

    let mut ok = 0;
    let mut fail = 0;
    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s)  => s,
            Err(e) => { eprintln!("  ✗ {} — read error: {}", path.display(), e); fail += 1; continue; }
        };
        match parse(&source) {
            Ok(_)  => { println!("  ✓ {}", path.display()); ok += 1; }
            Err(e) => { println!("  ✗ {} — {}", path.display(), e); fail += 1; }
        }
    }

    println!("\n{} ok, {} failed", ok, fail);
    if fail > 0 { process::exit(1); }
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
    println!("DuelScript CLI v0.3");
    println!();
    println!("USAGE:");
    println!("  duelscript <command> <target>");
    println!();
    println!("COMMANDS:");
    println!("  check   <file.ds | directory>   Validate .ds files for errors and warnings");
    println!("  inspect <file.ds>               Pretty-print the parsed AST of a card");
    println!("  fmt     <file.ds | directory>   Auto-format .ds files (v0.4 — stub in v0.3)");
    println!();
    println!("EXAMPLES:");
    println!("  duelscript check cards/");
    println!("  duelscript check cards/ash_blossom.ds");
    println!("  duelscript inspect cards/stardust_dragon.ds");
    println!("  duelscript fmt cards/");
}
