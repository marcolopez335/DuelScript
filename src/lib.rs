// ============================================================
// DuelScript — lib.rs v0.5
// Universal card definition format for Yu-Gi-Oh card mechanics.
// ============================================================

pub mod ast;
pub mod parser;
pub mod validator;
pub mod database;
pub mod engine;
pub mod compiler;
pub mod cdb;
pub mod migrate;
pub mod fmt;
pub mod test_harness;
#[cfg(feature = "python")]
pub mod python;
#[cfg(feature = "lua_transpiler")]
pub mod lua_transpiler;

pub use parser::{parse, ParseError};
pub use validator::{validate, validate_card, ValidationError, ValidationReport, Severity};
pub use database::{CardDatabase, LoadError, LoadErrorKind};
pub use cdb::{CdbReader, CdbCard, CdbError, CdbRegion, MergedCard, merge_databases};
pub use migrate::{generate_from_lua, migrate_directory, MigrationResult, Confidence};
pub use compiler::compile_card;
pub use fmt::format_source;
pub use engine::bridge::{
    DuelScriptEngine, GameContext, GameEvent, GameEventKind,
    CardHandle, EffectActivation, ChainLink,
    evaluate_condition_default, frequency_allows, trigger_matches,
};
pub use ast::*;
