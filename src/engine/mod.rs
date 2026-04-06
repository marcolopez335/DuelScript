// ============================================================
// DuelScript Engine Module — engine/mod.rs
// ============================================================

pub mod bridge;

pub use bridge::{
    DuelScriptEngine,
    GameContext,
    GameEvent,
    GameEventKind,
    CardHandle,
    EffectActivation,
    ChainLink,
    evaluate_condition_default,
    frequency_allows,
    trigger_matches,
};
