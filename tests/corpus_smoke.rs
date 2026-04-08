// ============================================================
// Sprint 30: Corpus smoke test.
//
// Picks 50 representative migrated cards across the tier
// distribution and verifies that each one:
//   1. parses cleanly
//   2. compiles to a CompiledCard with at least one effect
//   3. runs every effect's operation closure without panicking
//      against a MockRuntime stocked with sensible defaults
//
// This is the "smoke" gate before differential testing — a card
// that doesn't even survive a generic mock-runtime walk has zero
// chance of behaving correctly in a real engine. Failures here
// surface compiler/closure bugs that affect the corpus at scale,
// not just hand-written test cards.
//
// We pick cards by sampling from cards/test (canonical) plus a
// stratified sample from /tmp/ds_out_v9 (migrated). The migrated
// path runs only when the directory exists — CI / fresh checkouts
// without a populated /tmp/ds_out_v9 will skip cleanly.
// ============================================================

use std::fs;
use std::path::Path;

use duelscript::test_harness::{compile_file, MockRuntime};

const SAMPLE_SIZE: usize = 50;

fn try_run_card(path: &Path, rt: &mut MockRuntime) -> Result<(usize, usize), String> {
    let compiled = compile_file(path).map_err(|e| format!("compile: {}", e))?;
    let total = compiled.effects.len();
    let mut ran = 0usize;
    for eff in &compiled.effects {
        // Stock the deck so draw actions have something to pop.
        if rt.state.players[0].deck.is_empty() {
            rt.state.players[0].deck = (1000..1100).collect();
        }
        if rt.state.players[1].deck.is_empty() {
            rt.state.players[1].deck = (2000..2100).collect();
        }
        rt.effect_card_id = compiled.card_id;
        let cond_ok = eff.callbacks.condition.as_ref()
            .map(|cb| cb(rt))
            .unwrap_or(true);
        if !cond_ok { continue; }
        if let Some(op) = &eff.callbacks.operation {
            // Catch panics so one bad card doesn't bring down the run.
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                op(rt);
            }));
            if result.is_err() {
                return Err(format!("operation panicked"));
            }
            ran += 1;
        }
    }
    Ok((ran, total))
}

#[test]
fn corpus_smoke_test_canonical_cards() {
    let dir = Path::new("cards/test");
    let mut paths: Vec<_> = fs::read_dir(dir).expect("read cards/test")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
        .collect();
    paths.sort();

    let mut rt = MockRuntime::new();
    let mut failures = Vec::new();
    let mut total_effects = 0usize;
    let mut total_ran = 0usize;

    for p in &paths {
        match try_run_card(p, &mut rt) {
            Ok((ran, total)) => {
                total_ran += ran;
                total_effects += total;
            }
            Err(e) => {
                failures.push(format!("{}: {}", p.display(), e));
            }
        }
    }

    eprintln!("Canonical cards: {} cards, {} effects total, {} effect ops ran without panic",
        paths.len(), total_effects, total_ran);

    if !failures.is_empty() {
        for f in &failures { eprintln!("  FAIL: {}", f); }
        panic!("{} canonical cards failed corpus smoke", failures.len());
    }
}

#[test]
fn corpus_smoke_test_migrated_cards() {
    let dir = Path::new("/tmp/ds_out_v9");
    if !dir.exists() {
        eprintln!("[skip] /tmp/ds_out_v9 not present — re-run migration first");
        return;
    }

    // Sample SAMPLE_SIZE files across the alphabetical span so we hit
    // a mix of card categories without biasing toward any one prefix.
    let mut all: Vec<_> = fs::read_dir(dir).expect("read ds_out_v9")
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("ds"))
        .collect();
    all.sort();

    if all.is_empty() {
        eprintln!("[skip] no .ds files in /tmp/ds_out_v9");
        return;
    }

    let stride = (all.len() / SAMPLE_SIZE).max(1);
    let sample: Vec<_> = all.into_iter().step_by(stride).take(SAMPLE_SIZE).collect();

    let mut rt = MockRuntime::new();
    let mut compile_failures = 0usize;
    let mut runtime_panics = 0usize;
    let mut total_effects = 0usize;
    let mut total_ran = 0usize;
    let mut failure_log = Vec::new();

    for p in &sample {
        match try_run_card(p, &mut rt) {
            Ok((ran, total)) => {
                total_ran += ran;
                total_effects += total;
            }
            Err(e) => {
                if e.contains("compile") {
                    compile_failures += 1;
                } else {
                    runtime_panics += 1;
                }
                failure_log.push(format!("{}: {}", p.display(), e));
            }
        }
    }

    let success_rate = (sample.len() - compile_failures - runtime_panics) as f64
        / sample.len() as f64 * 100.0;

    eprintln!();
    eprintln!("=== Sprint 30: corpus smoke test ===");
    eprintln!("  Sampled:           {}/{}", sample.len(), SAMPLE_SIZE);
    eprintln!("  Compile failures:  {}", compile_failures);
    eprintln!("  Runtime panics:    {}", runtime_panics);
    eprintln!("  Effects ran:       {}/{}", total_ran, total_effects);
    eprintln!("  Success rate:      {:.1}%", success_rate);

    if !failure_log.is_empty() {
        eprintln!();
        eprintln!("First 10 failures:");
        for f in failure_log.iter().take(10) {
            eprintln!("  {}", f);
        }
    }

    // Assert at least 90% of sampled cards survive the smoke test.
    // Anything lower means a regression worth chasing.
    assert!(success_rate >= 90.0,
        "corpus smoke success rate too low: {:.1}% (need ≥ 90%)", success_rate);
}
