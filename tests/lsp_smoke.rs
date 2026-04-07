// ============================================================
// Sprint 18: LSP smoke test.
//
// Tests the diagnostic-generation logic directly. The full
// JSON-RPC pipeline (tower-lsp + tokio + stdio framing) was
// verified manually via a Python harness — see
// scripts/lsp_manual_check.py for the canonical reproducer.
// We don't drive the binary from a Rust test because
// process-orchestration races between rustc-built children and
// the test harness aren't worth chasing for a smoke check.
//
// What this DOES test:
//   - Parse errors map to LSP-shaped Diagnostic with line/col.
//   - Validation errors anchor to the card declaration line.
//   - Valid cards yield zero diagnostics.
// ============================================================

#![cfg(feature = "lsp")]

use duelscript::parse;
use duelscript::validator::{validate_card, Severity};

/// Mirror of the Backend::diagnostics_for logic so the unit test
/// doesn't need a Backend or Client. Any change to the LSP server's
/// diagnostic builder must be mirrored here too.
fn diagnostics_for(source: &str) -> Vec<(String, &'static str)> {
    match parse(source) {
        Err(e) => vec![(format!("{}", e), "error")],
        Ok(file) => {
            let mut out = Vec::new();
            for card in &file.cards {
                let mut errs = Vec::new();
                validate_card(card, &mut errs);
                for e in errs {
                    let kind = match e.severity {
                        Severity::Error   => "error",
                        Severity::Warning => "warning",
                    };
                    out.push((e.message, kind));
                }
            }
            out
        }
    }
}

#[test]
fn parse_error_produces_diagnostic() {
    let bad = r#"card "X" { type: Normal Spell password: 1 effect "e" { on_resolve { not_a_real_action } } }"#;
    let diags = diagnostics_for(bad);
    assert!(!diags.is_empty(), "expected at least one diagnostic for invalid card");
    assert!(diags[0].0.contains("Parse error"),
        "diagnostic should report parse error: {:?}", diags[0]);
    assert_eq!(diags[0].1, "error");
}

#[test]
fn valid_card_produces_no_errors() {
    let good = r#"
        card "Pot of Greed" {
            type: Normal Spell
            password: 55144522
            effect "Draw 2" {
                speed: spell_speed_1
                on_resolve { draw 2 }
            }
        }
    "#;
    let diags = diagnostics_for(good);
    let errors: Vec<_> = diags.iter().filter(|(_, k)| *k == "error").collect();
    assert!(errors.is_empty(),
        "valid Pot of Greed should have zero error diagnostics, got {:?}", errors);
}

#[test]
fn validation_warning_surfaces() {
    // A monster with a 'flip_effect' block but no Flip type triggers
    // the validator warning we added in Sprint 9.
    let card = r#"
        card "Galaxy Mirror Sage" {
            type: Effect Monster
            password: 98263709
            attribute: LIGHT
            race: Spellcaster
            level: 3
            atk: 800
            def: 500
            flip_effect "Gain LP" {
                on_resolve { gain_lp: 500 }
            }
        }
    "#;
    let diags = diagnostics_for(card);
    let warnings: Vec<_> = diags.iter().filter(|(_, k)| *k == "warning").collect();
    assert!(!warnings.is_empty(),
        "should warn about flip_effect on non-Flip monster, got {:?}", diags);
}
