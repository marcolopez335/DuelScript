// ============================================================
// DuelScript Python Bindings — python.rs
//
// PyO3 module exposing DuelScript's core functionality to Python:
//   - parse .ds source files
//   - validate card definitions
//   - compile to engine-level effect metadata
//   - load and query card databases
//
// Usage from Python:
//   import duelscript
//   card = duelscript.parse_card(source)
//   errors = duelscript.validate(source)
//   compiled = duelscript.compile(source)
//   db = duelscript.CardDB("cards/official")
// ============================================================

use pyo3::prelude::*;
use pyo3::types::PyDict;
use pyo3::exceptions::{PyRuntimeError, PyValueError};

use crate::ast;
use crate::parser;
use crate::validator;
use crate::compiler;
use crate::database::CardDatabase;

// ── Parse Functions ───────────────────────────────────────────

/// Parse a .ds source string and return a list of card dicts.
#[pyfunction]
fn parse_source(source: &str) -> PyResult<Vec<PyCard>> {
    let file = parser::parse(source)
        .map_err(|e| PyRuntimeError::new_err(format!("Parse error: {}", e)))?;

    Ok(file.cards.into_iter().map(PyCard::from).collect())
}

/// Parse a .ds file from disk and return a list of card dicts.
#[pyfunction]
fn parse_file(path: &str) -> PyResult<Vec<PyCard>> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| PyRuntimeError::new_err(format!("Failed to read {}: {}", path, e)))?;
    parse_source(&source)
}

/// Validate a .ds source string. Returns a list of error/warning messages.
#[pyfunction]
fn validate(source: &str) -> PyResult<Vec<PyValidationError>> {
    let file = parser::parse(source)
        .map_err(|e| PyRuntimeError::new_err(format!("Parse error: {}", e)))?;

    let errors = validator::validate(&file);
    Ok(errors.into_iter().map(PyValidationError::from).collect())
}

/// Parse and compile a .ds source string. Returns compiled effect metadata.
#[pyfunction]
fn compile(source: &str) -> PyResult<Vec<PyCompiledCard>> {
    let file = parser::parse(source)
        .map_err(|e| PyRuntimeError::new_err(format!("Parse error: {}", e)))?;

    Ok(file.cards.iter().map(|card| {
        let compiled = compiler::compile_card(card);
        PyCompiledCard::from(compiled)
    }).collect())
}

// ── Card Database ─────────────────────────────────────────────

/// A card database loaded from .ds files on disk.
#[pyclass]
#[pyo3(name = "CardDB")]
struct PyCardDB {
    db: CardDatabase,
}

#[pymethods]
impl PyCardDB {
    /// Load all .ds files from a directory (recursive).
    #[new]
    fn new(dir: &str) -> PyResult<Self> {
        let path = std::path::Path::new(dir);
        if !path.exists() {
            return Err(PyValueError::new_err(format!("Directory not found: {}", dir)));
        }
        let db = CardDatabase::load_from_dir(path);
        Ok(PyCardDB { db })
    }

    /// Number of cards loaded.
    fn __len__(&self) -> usize {
        self.db.len()
    }

    /// Get a card by name.
    fn get(&self, name: &str) -> Option<PyCard> {
        self.db.get(name).map(|c| PyCard::from_ref(&c))
    }

    /// Get a card by passcode (card ID).
    fn get_by_id(&self, passcode: u32) -> Option<PyCard> {
        self.db.get_by_passcode(passcode).map(|c| PyCard::from_ref(&c))
    }

    /// Check if a card exists by name.
    fn contains(&self, name: &str) -> bool {
        self.db.contains(name)
    }

    /// Search cards by name substring (case-insensitive).
    fn search(&self, query: &str) -> Vec<PyCard> {
        self.db.search_by_name(query).iter().map(|c| PyCard::from_ref(c)).collect()
    }

    /// Get all card names.
    fn card_names(&self) -> Vec<String> {
        self.db.iter_cards().map(|(name, _)| name.to_string()).collect()
    }

    /// Get load errors (if any files failed to parse).
    fn load_errors(&self) -> Vec<String> {
        self.db.load_errors.iter()
            .map(|e| format!("{}: {}", e.path.display(), e.kind))
            .collect()
    }

    /// Compile all cards and return compiled metadata.
    fn compile_all(&self) -> Vec<PyCompiledCard> {
        self.db.iter_cards().map(|(_, card)| {
            let compiled = compiler::compile_card(card);
            PyCompiledCard::from(compiled)
        }).collect()
    }
}

// ── Python Types ──────────────────────────────────────────────

/// A parsed card exposed to Python.
#[pyclass]
#[pyo3(name = "Card")]
#[derive(Clone)]
struct PyCard {
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    password: Option<u32>,
    #[pyo3(get)]
    card_types: Vec<String>,
    #[pyo3(get)]
    attribute: Option<String>,
    #[pyo3(get)]
    race: Option<String>,
    #[pyo3(get)]
    level: Option<u32>,
    #[pyo3(get)]
    rank: Option<u32>,
    #[pyo3(get)]
    link: Option<u32>,
    #[pyo3(get)]
    scale: Option<u32>,
    #[pyo3(get)]
    atk: Option<String>,
    #[pyo3(get)]
    def: Option<String>,
    #[pyo3(get)]
    archetypes: Vec<String>,
    #[pyo3(get)]
    num_effects: usize,
    #[pyo3(get)]
    num_continuous_effects: usize,
    #[pyo3(get)]
    has_materials: bool,
    #[pyo3(get)]
    link_arrows: Vec<String>,
}

#[pymethods]
impl PyCard {
    fn __repr__(&self) -> String {
        format!("Card('{}', id={:?}, types={:?})", self.name, self.password, self.card_types)
    }

    fn __str__(&self) -> String {
        self.name.clone()
    }
}

impl PyCard {
    fn from(card: ast::Card) -> Self {
        Self::from_ref(&card)
    }

    fn from_ref(card: &ast::Card) -> Self {
        PyCard {
            name: card.name.clone(),
            password: card.password,
            card_types: card.card_types.iter().map(|t| format!("{:?}", t)).collect(),
            attribute: card.attribute.as_ref().map(|a| format!("{:?}", a)),
            race: card.race.as_ref().map(|r| format!("{:?}", r)),
            level: card.level,
            rank: card.rank,
            link: card.link,
            scale: card.scale,
            atk: card.stats.atk.as_ref().map(|v| match v {
                ast::StatValue::Number(n) => n.to_string(),
                ast::StatValue::Variable => "?".to_string(),
            }),
            def: card.stats.def.as_ref().map(|v| match v {
                ast::StatValue::Number(n) => n.to_string(),
                ast::StatValue::Variable => "?".to_string(),
            }),
            archetypes: card.archetypes.clone(),
            num_effects: card.effects.len(),
            num_continuous_effects: card.continuous_effects.len(),
            has_materials: card.materials.is_some(),
            link_arrows: card.link_arrows.iter().map(|a| format!("{:?}", a)).collect(),
        }
    }
}

/// A validation error/warning exposed to Python.
#[pyclass]
#[pyo3(name = "ValidationError")]
#[derive(Clone)]
struct PyValidationError {
    #[pyo3(get)]
    card_name: String,
    #[pyo3(get)]
    message: String,
    #[pyo3(get)]
    severity: String,
}

#[pymethods]
impl PyValidationError {
    fn __repr__(&self) -> String {
        format!("[{}] {}: {}", self.severity, self.card_name, self.message)
    }
}

impl From<validator::ValidationError> for PyValidationError {
    fn from(e: validator::ValidationError) -> Self {
        PyValidationError {
            card_name: e.card_name.clone(),
            message: e.message.clone(),
            severity: format!("{:?}", e.severity),
        }
    }
}

/// A compiled card with engine-level effect metadata.
#[pyclass]
#[pyo3(name = "CompiledCard")]
#[derive(Clone)]
struct PyCompiledCard {
    #[pyo3(get)]
    card_id: u32,
    #[pyo3(get)]
    name: String,
    #[pyo3(get)]
    effects: Vec<PyCompiledEffect>,
}

#[pymethods]
impl PyCompiledCard {
    fn __repr__(&self) -> String {
        format!("CompiledCard('{}', id={}, effects={})", self.name, self.card_id, self.effects.len())
    }
}

impl From<compiler::CompiledCard> for PyCompiledCard {
    fn from(c: compiler::CompiledCard) -> Self {
        PyCompiledCard {
            card_id: c.card_id,
            name: c.name,
            effects: c.effects.iter().map(PyCompiledEffect::from_ref).collect(),
        }
    }
}

/// A compiled effect with engine bitfields.
#[pyclass]
#[pyo3(name = "CompiledEffect")]
#[derive(Clone)]
struct PyCompiledEffect {
    #[pyo3(get)]
    effect_type: u32,
    #[pyo3(get)]
    category: u32,
    #[pyo3(get)]
    code: u32,
    #[pyo3(get)]
    property: u32,
    #[pyo3(get)]
    range: u32,
    #[pyo3(get)]
    count_limit_count: Option<u32>,
    #[pyo3(get)]
    count_limit_code: Option<u32>,
    #[pyo3(get)]
    has_condition: bool,
    #[pyo3(get)]
    has_cost: bool,
    #[pyo3(get)]
    has_target: bool,
    #[pyo3(get)]
    has_operation: bool,
}

#[pymethods]
impl PyCompiledEffect {
    fn __repr__(&self) -> String {
        format!(
            "CompiledEffect(type={:#x}, cat={:#x}, code={}, range={:#x})",
            self.effect_type, self.category, self.code, self.range
        )
    }

    /// Get effect type as human-readable string.
    fn type_name(&self) -> &str {
        match self.effect_type {
            0x10   => "ACTIVATE",
            0x40   => "IGNITION",
            0x80   => "TRIGGER_O",
            0x100  => "QUICK_O",
            0x200  => "TRIGGER_F",
            0x400  => "QUICK_F",
            0x800  => "CONTINUOUS",
            0x1000 => "FIELD",
            0x2000 => "EQUIP",
            _      => "UNKNOWN",
        }
    }
}

impl PyCompiledEffect {
    fn from_ref(e: &compiler::CompiledEffect) -> Self {
        PyCompiledEffect {
            effect_type: e.effect_type,
            category: e.category,
            code: e.code,
            property: e.property,
            range: e.range,
            count_limit_count: e.count_limit.as_ref().map(|cl| cl.count),
            count_limit_code: e.count_limit.as_ref().map(|cl| cl.code),
            has_condition: e.callbacks.condition.is_some(),
            has_cost: e.callbacks.cost.is_some(),
            has_target: e.callbacks.target.is_some(),
            has_operation: e.callbacks.operation.is_some(),
        }
    }
}

// ── Module Registration ───────────────────────────────────────

#[pymodule]
fn duelscript(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(parse_source, m)?)?;
    m.add_function(wrap_pyfunction!(parse_file, m)?)?;
    m.add_function(wrap_pyfunction!(validate, m)?)?;
    m.add_function(wrap_pyfunction!(compile, m)?)?;
    m.add_class::<PyCardDB>()?;
    m.add_class::<PyCard>()?;
    m.add_class::<PyCompiledCard>()?;
    m.add_class::<PyCompiledEffect>()?;
    m.add_class::<PyValidationError>()?;

    // Constants for effect types
    m.add("EFFECT_TYPE_ACTIVATE", 0x10u32)?;
    m.add("EFFECT_TYPE_IGNITION", 0x40u32)?;
    m.add("EFFECT_TYPE_TRIGGER_O", 0x80u32)?;
    m.add("EFFECT_TYPE_QUICK_O", 0x100u32)?;
    m.add("EFFECT_TYPE_TRIGGER_F", 0x200u32)?;
    m.add("EFFECT_TYPE_QUICK_F", 0x400u32)?;
    m.add("EFFECT_TYPE_CONTINUOUS", 0x800u32)?;
    m.add("EFFECT_TYPE_FIELD", 0x1000u32)?;
    m.add("EFFECT_TYPE_EQUIP", 0x2000u32)?;

    // Constants for categories (EDOPro-compatible)
    m.add("CATEGORY_DESTROY", 0x1u32)?;
    m.add("CATEGORY_SPECIAL_SUMMON", 0x200u32)?;
    m.add("CATEGORY_DRAW", 0x10000u32)?;
    m.add("CATEGORY_SEARCH", 0x20000u32)?;
    m.add("CATEGORY_NEGATE", 0x10000000u32)?;
    m.add("CATEGORY_DAMAGE", 0x80000u32)?;
    m.add("CATEGORY_RECOVER", 0x100000u32)?;
    m.add("CATEGORY_CONTROL", 0x2000u32)?;

    Ok(())
}
