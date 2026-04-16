// ============================================================
// DuelScript CLI — main.rs
// Usage:
//   duelscript check cards/           validate all .ds files
//   duelscript check ash_blossom.ds   validate a single file
//   duelscript inspect ash_blossom.ds pretty-print the AST
//   duelscript fmt cards/             auto-format .ds files
//   duelscript test card.ds           compile + run effects
// ============================================================

use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};
use duelscript::{parse_v2, validate_v2};
use duelscript::v2::fmt::format_file;
use duelscript::compile_card_v2;
use duelscript::v2::mock_runtime::MockRuntime;

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
        "fmt"     => cmd_fmt(target, &args),
        "test"    => cmd_test(target, &args),
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

fn cmd_fmt(target: &Path, args: &[String]) {
    let check_only = args.iter().any(|a| a == "--check");
    let files = collect_ds_files(target);
    if files.is_empty() {
        eprintln!("No .ds files found at: {}", target.display());
        process::exit(1);
    }
    let mut changed = 0;
    let mut clean = 0;
    let mut errors = 0;
    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => { eprintln!("  ✗ {} — {}", path.display(), e); errors += 1; continue; }
        };
        let file = match parse_v2(&source) {
            Ok(f) => f,
            Err(e) => { eprintln!("  ✗ {} — parse: {}", path.display(), e); errors += 1; continue; }
        };
        let formatted = format_file(&file);
        if parse_v2(&formatted).is_err() {
            eprintln!("  ✗ {} — fmt produced invalid output", path.display());
            errors += 1;
            continue;
        }
        if formatted == source { clean += 1; continue; }
        changed += 1;
        if check_only {
            println!("  Δ {}", path.display());
        } else {
            match fs::write(path, &formatted) {
                Ok(_) => println!("  ✓ {}", path.display()),
                Err(e) => { eprintln!("  ✗ {} — write: {}", path.display(), e); errors += 1; }
            }
        }
    }
    println!("\n{} formatted, {} clean, {} errors",
        if check_only { 0 } else { changed }, clean, errors);
    if check_only && changed > 0 { process::exit(1); }
    if errors > 0 { process::exit(1); }
}

fn cmd_test(target: &Path, args: &[String]) {
    if !target.is_file() {
        eprintln!("'test' requires a single .ds file");
        process::exit(1);
    }
    // Parse flags
    let mut lp = 8000i32;
    let verbose = args.iter().any(|a| a == "--verbose");
    let mut i = 3;
    while i < args.len() {
        match args[i].as_str() {
            "--lp" if i + 1 < args.len() => { lp = args[i+1].parse().unwrap_or(8000); i += 2; }
            _ => i += 1,
        }
    }

    let source = fs::read_to_string(target).unwrap_or_else(|e| {
        eprintln!("Read error: {}", e); process::exit(1);
    });
    let file = parse_v2(&source).unwrap_or_else(|e| {
        eprintln!("Parse error: {}", e); process::exit(1);
    });
    let card = &file.cards[0];
    let compiled = compile_card_v2(card);
    let card_id = card.fields.id.unwrap_or(0) as u32;

    println!("{} ({})", card.name, card_id);

    for (idx, eff) in compiled.effects.iter().enumerate() {
        // Create fresh MockRuntime per effect
        let mut rt = MockRuntime::new();
        rt.effect_card_id = card_id;
        rt.effect_player = 0;
        rt.state.players[0].lp = lp;
        rt.state.players[1].lp = lp;

        println!("  Effect {} \"{}\":", idx + 1, eff.label);

        // Condition
        let cond_ok = match &eff.condition {
            Some(c) => { let r = c(&rt); println!("    condition:  {}", if r {"PASS"} else {"FAIL"}); r }
            None => { println!("    condition:  none"); true }
        };
        if !cond_ok { println!(); continue; }

        // Cost
        if let Some(cost) = &eff.cost {
            let can = cost(&mut rt, true);
            println!("    cost:       {}", if can {"CAN PAY"} else {"CANNOT PAY"});
            if can { cost(&mut rt, false); }
        } else {
            println!("    cost:       none");
        }

        // Target
        if let Some(tgt) = &eff.target {
            let has = tgt(&mut rt, true);
            println!("    targets:    {}", if has {"found"} else {"none"});
            if has { tgt(&mut rt, false); }
        } else {
            println!("    targets:    none");
        }

        // Operation
        if let Some(op) = &eff.operation {
            op(&mut rt);
            let log = rt.dump_calls();
            let lines: Vec<&str> = log.lines().collect();
            println!("    operation:  {} calls", lines.len());
            if verbose {
                for line in &lines { println!("      {}", line); }
            } else {
                for line in lines.iter().take(3) { println!("      {}", line); }
                if lines.len() > 3 { println!("      ... +{} more (--verbose)", lines.len() - 3); }
            }
        }
        println!();
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
    println!("  duelscript <command> <target> [flags]");
    println!();
    println!("COMMANDS:");
    println!("  check   <file.ds | directory>   Validate .ds files");
    println!("  inspect <file.ds>               Pretty-print the AST");
    println!("  fmt     <file.ds | directory>   Auto-format .ds files");
    println!("  test    <file.ds>               Compile + execute against MockRuntime");
    println!();
    println!("FLAGS:");
    println!("  --check      (fmt) dry-run, exit 1 if files need formatting");
    println!("  --lp N       (test) set starting LP (default 8000)");
    println!("  --verbose    (test) show full call log");
    println!();
    println!("EXAMPLES:");
    println!("  duelscript check cards/goat/");
    println!("  duelscript fmt cards/goat/ --check");
    println!("  duelscript test cards/goat/pot_of_greed.ds");
    println!("  duelscript test cards/goat/solemn_judgment.ds --lp 4000 --verbose");
}
