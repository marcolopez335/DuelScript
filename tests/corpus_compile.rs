// ============================================================
// M6 — corpus-wide compile regression suite
// Decision II-I, 2026-04-19
//
// Two tests exercise every .ds file in the card corpus through
// the parse + compile + validate pipeline.
//
// corpus_compiles_goat     — runs on `cargo test` (default)
// corpus_compiles_official — gated behind #[ignore]; run with
//                            `cargo test -- --ignored`
// ============================================================

use std::{
    fs,
    panic,
    path::{Path, PathBuf},
};
use duelscript::{parse_v2, compile_card_v2, validate_v2};

// ── KPI types ────────────────────────────────────────────────

/// Per-variant closure-presence counts for a corpus run.
#[derive(Debug, Default)]
struct ClosureCoverageCounts {
    passive_with_op:       usize,
    passive_total:         usize,
    restriction_with_op:   usize,
    restriction_total:     usize,
    replacement_with_op:   usize,
    replacement_total:     usize,
    summon_proc_with_op:   usize,
    summon_proc_total:     usize,
}

// ── Shared helpers ──────────────────────────────────────────

fn collect_ds_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .expect("could not open corpus directory")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "ds"))
        .collect();
    files.sort();
    files
}

/// Run parse + compile + (non-fatal) validate over every `.ds` file in `dir`.
///
/// Returns `(parse_compile_failures, validation_warning_count, validation_file_count)`.
/// Parse errors and compile panics accumulate into `failures`.
/// Validation warnings are counted but do NOT contribute to `failures`.
fn run_corpus(dir: &Path) -> (Vec<(PathBuf, String)>, usize, usize) {
    let files = collect_ds_files(dir);
    let total = files.len();

    let mut failures: Vec<(PathBuf, String)> = Vec::new();
    let mut total_validation_warnings: usize = 0;
    let mut files_with_validation_warnings: usize = 0;

    for path in &files {
        // ── Read ──────────────────────────────────────────
        let source = match fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) => {
                failures.push((path.clone(), format!("read error: {e}")));
                continue;
            }
        };

        // ── Parse ─────────────────────────────────────────
        let parsed_file = match parse_v2(&source) {
            Ok(f) => f,
            Err(e) => {
                failures.push((path.clone(), format!("parse error: {e}")));
                continue;
            }
        };

        // ── Compile (guarded against panics per HH-I) ────
        for card in &parsed_file.cards {
            // Clone the card reference so catch_unwind can own it.
            // compile_card_v2 takes &Card, which is Send; wrap in
            // AssertUnwindSafe so the closure is UnwindSafe.
            let card_ref = panic::AssertUnwindSafe(card);
            let result = panic::catch_unwind(|| compile_card_v2(*card_ref));
            if let Err(payload) = result {
                let msg = payload
                    .downcast_ref::<String>()
                    .map(|s| s.clone())
                    .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
                    .unwrap_or_else(|| "<unknown panic>".to_string());
                failures.push((path.clone(), format!("compile panic in '{}': {msg}", card.name)));
            }
        }

        // ── Validate (non-fatal) ──────────────────────────
        let report = validate_v2(&parsed_file);
        let w = report.warning_count();
        if w > 0 {
            total_validation_warnings += w;
            files_with_validation_warnings += 1;
        }
        // Validation errors are also non-fatal for M6 — count them too.
        let e = report.error_count();
        if e > 0 {
            total_validation_warnings += e;
            files_with_validation_warnings += 1;
        }
    }

    println!(
        "corpus: {total} file(s) processed, {} parse/compile failure(s)",
        failures.len()
    );
    if total_validation_warnings > 0 {
        println!(
            "validation: {total_validation_warnings} warning(s)/error(s) across \
             {files_with_validation_warnings} file(s) (not fatal)"
        );
    } else {
        println!("validation: clean (0 warnings/errors)");
    }

    (failures, total_validation_warnings, files_with_validation_warnings)
}

fn report_and_assert(failures: Vec<(PathBuf, String)>, label: &str) {
    if failures.is_empty() {
        return;
    }

    let n = failures.len();
    // The total is not available here directly; just print what we have.
    eprintln!(
        "\n{label}: {n} file(s) failed to compile cleanly\nfirst 10 failures:"
    );
    for (path, msg) in failures.iter().take(10) {
        eprintln!("  {}: {}", path.display(), msg);
    }

    assert!(
        failures.is_empty(),
        "{label}: {n} file(s) failed to parse or compile — see stderr for details"
    );
}

// ── Test 1: goat corpus (default, ~154 files) ───────────────

#[test]
fn corpus_compiles_goat() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("cards/goat");
    let (failures, _, _) = run_corpus(&dir);
    report_and_assert(failures, "corpus_compiles_goat");
}

// ── Test 2: official corpus (ignored, ~13 298 files) ────────

#[test]
#[ignore]
fn corpus_compiles_official() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("cards/official");
    let (failures, _, _) = run_corpus(&dir);
    report_and_assert(failures, "corpus_compiles_official");
}

// ── Test 3: closure-coverage KPI (ignored) ───────────────────

/// Diagnostic-only KPI test. Iterates both the goat and official corpora,
/// compiles every card, and tallies per-variant closure presence.
///
/// No assertions — the numbers are recorded for the NN-I close-out.
/// Run with: `cargo test --test corpus_compile corpus_closure_coverage -- --ignored`
#[test]
#[ignore]
fn corpus_closure_coverage() {
    let goat_dir     = Path::new(env!("CARGO_MANIFEST_DIR")).join("cards/goat");
    let official_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("cards/official");

    let goat_counts     = measure_closure_coverage(&goat_dir, "goat");
    let official_counts = measure_closure_coverage(&official_dir, "official");

    // Print both KPI reports (stdout is visible with -- --nocapture, or in CI logs).
    print_kpi("goat (154)", &goat_counts);
    print_kpi("official (13,298)", &official_counts);
}

fn measure_closure_coverage(dir: &Path, label: &str) -> ClosureCoverageCounts {
    let files = collect_ds_files(dir);
    let mut counts = ClosureCoverageCounts::default();

    for path in &files {
        let source = match fs::read_to_string(path) {
            Ok(s)  => s,
            Err(_) => continue,
        };
        let parsed_file = match parse_v2(&source) {
            Ok(f)  => f,
            Err(_) => continue,
        };

        for card in &parsed_file.cards {
            // Compile, catching any panics so we don't abort the KPI run.
            let card_ref = panic::AssertUnwindSafe(card);
            let compiled = match panic::catch_unwind(|| compile_card_v2(*card_ref)) {
                Ok(c)  => c,
                Err(_) => continue,
            };

            // ── Passive ──────────────────────────────────────
            for passive in &card.passives {
                counts.passive_total += 1;
                // Match by label (passive.name is used as the compiled effect label).
                if compiled.effects.iter()
                    .any(|e| e.label == passive.name && e.operation.is_some())
                {
                    counts.passive_with_op += 1;
                }
            }

            // ── Restriction ──────────────────────────────────
            for restr in &card.restrictions {
                counts.restriction_total += 1;
                let label = restr.name.as_deref().unwrap_or("restriction");
                if compiled.effects.iter()
                    .any(|e| e.label == label && e.operation.is_some())
                {
                    counts.restriction_with_op += 1;
                }
            }

            // ── Replacement ──────────────────────────────────
            for repl in &card.replacements {
                counts.replacement_total += 1;
                let label = repl.name.as_deref().unwrap_or("replacement");
                if compiled.effects.iter()
                    .any(|e| e.label == label && e.operation.is_some())
                {
                    counts.replacement_with_op += 1;
                }
            }

            // ── Summon procedure ──────────────────────────────
            // Count each sub-site: materials-based ("Summon Procedure") and
            // special summon procedure ("Special Summon Procedure").
            // Xyz Check (code 946) and Cannot Normal Summon (code 42) are
            // intentionally excluded — they are pure metadata tags (M2c scope).
            if let Some(ref summon) = card.summon {
                let has_materials = summon.fusion_materials.is_some()
                    || summon.synchro_materials.is_some()
                    || summon.xyz_materials.is_some()
                    || summon.link_materials.is_some();

                if has_materials {
                    counts.summon_proc_total += 1;
                    if compiled.effects.iter()
                        .any(|e| e.label == "Summon Procedure" && e.operation.is_some())
                    {
                        counts.summon_proc_with_op += 1;
                    }
                }

                if summon.special_summon_procedure.is_some() && !has_materials {
                    counts.summon_proc_total += 1;
                    if compiled.effects.iter()
                        .any(|e| e.label == "Special Summon Procedure" && e.operation.is_some())
                    {
                        counts.summon_proc_with_op += 1;
                    }
                }
            }
        }
    }

    eprintln!("[closure_coverage] {} scanned {} files", label, files.len());
    counts
}

fn print_kpi(label: &str, c: &ClosureCoverageCounts) {
    println!(
        "closure_coverage {}: passive {}/{}, restriction {}/{}, \
         replacement {}/{}, summon_proc {}/{}",
        label,
        c.passive_with_op,     c.passive_total,
        c.restriction_with_op, c.restriction_total,
        c.replacement_with_op, c.replacement_total,
        c.summon_proc_with_op, c.summon_proc_total,
    );
}
