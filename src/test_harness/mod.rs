// ============================================================
// DuelScript Test Harness
//
// Standalone, engine-agnostic test infrastructure for verifying
// that the parse → AST → compile → callback pipeline produces
// correct runtime behavior.
//
// This module provides:
//   - MockRuntime: a DuelScriptRuntime implementation that records
//     every method call so tests can assert on what happened.
//   - DuelScenario: a builder for setting up an initial game state
//     before running an effect.
//   - run_card_effect: a one-shot helper that loads a .ds file,
//     compiles it, and runs the first effect's operation callback.
//
// The harness is *not* a real duel simulator. It does the bare
// minimum to exercise the callback closures so we can prove that:
//   1. .ds files parse and compile without crashing
//   2. The compiler emits closures that call the right runtime methods
//   3. Expressions evaluate against the runtime correctly
//   4. Phase 1-3 features (flags, bindings, events, etc.) are wired up
//
// Tests using this harness should be deterministic and cheap so they
// can run on every commit via `cargo test`.
// ============================================================

pub mod mock_runtime;
pub mod scenario;

pub use mock_runtime::{MockRuntime, RuntimeCall, MockState, CardSnapshot};
pub use scenario::DuelScenario;

use crate::ast::Card;
use crate::compiler::{compile_card, CompiledCard};
use crate::parser::{parse, ParseError};

/// Compile a .ds source string into a CompiledCard. Returns an error if
/// parsing fails or no card is found in the file.
pub fn compile_source(source: &str) -> Result<CompiledCard, ParseError> {
    let file = parse(source)?;
    let card = file.cards.into_iter().next()
        .ok_or_else(|| ParseError::MissingField("no card in file"))?;
    Ok(compile_card(&card))
}

/// Compile the first card in a .ds file at the given path.
pub fn compile_file(path: &std::path::Path) -> Result<CompiledCard, String> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| format!("read {}: {}", path.display(), e))?;
    compile_source(&source).map_err(|e| format!("parse {}: {:?}", path.display(), e))
}

/// Parse-only version that returns the raw AST without compiling.
pub fn parse_first_card(source: &str) -> Result<Card, ParseError> {
    let file = parse(source)?;
    file.cards.into_iter().next()
        .ok_or(ParseError::MissingField("no card in file"))
}
