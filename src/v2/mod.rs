// DuelScript v2 — clean language redesign
pub mod ast;
pub mod parser;
pub mod fmt;
pub mod validator;
pub mod compiler;
pub mod constants;
pub mod runtime;
pub mod mock_runtime;
pub mod segoc;
#[cfg(feature = "lsp")]
pub mod lsp;
