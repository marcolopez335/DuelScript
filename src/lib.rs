// DuelScript — a standalone scripting language for Yu-Gi-Oh card mechanics.

pub mod v2;
pub mod cdb;

#[cfg(feature = "lua_ast")]
pub mod lua_ast;

// Re-export the v2 public API at crate root
pub use v2::parser::{parse_v2, V2ParseError};
pub use v2::validator::{validate_v2, ValidationReport};
pub use v2::compiler::{compile_card_v2, CompiledCardV2, CompiledEffectV2};
pub use v2::ast;
pub use v2::constants;
pub use v2::runtime::{DuelScriptRuntime, CardFilter, Stat};
