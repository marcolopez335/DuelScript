// ============================================================
// DuelScript CardDatabase — database.rs
//
// The runtime card registry. Load .ds files at startup,
// query by name, archetype, attribute, race, or type.
// Thread-safe via Arc — clone freely across engine threads.
// ============================================================

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use crate::{
    ast::*,
    parser::{parse, ParseError},
    validator::{validate, ValidationReport},
};

// ── CardDatabase ──────────────────────────────────────────────

/// The runtime card registry. Loaded once at engine startup,
/// queried throughout the duel.
#[derive(Debug, Default)]
pub struct CardDatabase {
    /// Primary index — card name → card
    cards: HashMap<String, Arc<Card>>,

    /// Archetype index — archetype name → cards in that archetype
    by_archetype: HashMap<String, Vec<Arc<Card>>>,

    /// Attribute index
    by_attribute: HashMap<String, Vec<Arc<Card>>>,

    /// Race index
    by_race: HashMap<String, Vec<Arc<Card>>>,

    /// Type index — card type string → cards
    by_type: HashMap<String, Vec<Arc<Card>>>,

    /// Passcode index — card password/ID → card (O(1) lookup)
    by_passcode: HashMap<u32, Arc<Card>>,

    /// Load errors collected during startup (non-fatal)
    pub load_errors: Vec<LoadError>,
}

impl CardDatabase {
    /// Create an empty database.
    pub fn new() -> Self {
        Self::default()
    }

    // ── Loading ───────────────────────────────────────────────

    /// Load all `.ds` files from a directory (recursive).
    /// Returns the database even if some files fail — errors
    /// are collected in `db.load_errors` for inspection.
    pub fn load_from_dir(dir: &Path) -> Self {
        let mut db = Self::new();
        let files = collect_ds_files(dir);

        if files.is_empty() {
            db.load_errors.push(LoadError {
                path: dir.to_path_buf(),
                kind: LoadErrorKind::NoFilesFound,
            });
            return db;
        }

        for path in files {
            db.load_file(&path);
        }

        db
    }

    /// Load a single `.ds` file into the database.
    pub fn load_file(&mut self, path: &Path) {
        let source = match fs::read_to_string(path) {
            Ok(s)  => s,
            Err(e) => {
                self.load_errors.push(LoadError {
                    path: path.to_path_buf(),
                    kind: LoadErrorKind::ReadError(e.to_string()),
                });
                return;
            }
        };

        let file = match parse(&source) {
            Ok(f)  => f,
            Err(e) => {
                self.load_errors.push(LoadError {
                    path: path.to_path_buf(),
                    kind: LoadErrorKind::ParseError(e.to_string()),
                });
                return;
            }
        };

        // Validate — reject cards with errors, allow warnings through
        let all_errors = validate(&file);
        let report = ValidationReport::from(all_errors);

        if !report.errors.is_empty() {
            self.load_errors.push(LoadError {
                path: path.to_path_buf(),
                kind: LoadErrorKind::ValidationErrors(
                    report.errors.iter().map(|e| e.message.clone()).collect()
                ),
            });
            // Cards with validation errors are NOT registered
            return;
        }

        // Register all valid cards from this file
        for card in file.cards {
            self.register(card);
        }
    }

    /// Manually register a single parsed card.
    pub fn register(&mut self, card: Card) {
        let arc = Arc::new(card);

        // Primary index
        self.cards.insert(arc.name.clone(), Arc::clone(&arc));

        // Archetype index
        for archetype in &arc.archetypes {
            self.by_archetype
                .entry(archetype.clone())
                .or_default()
                .push(Arc::clone(&arc));
        }

        // Attribute index
        if let Some(attr) = &arc.attribute {
            self.by_attribute
                .entry(format!("{:?}", attr))
                .or_default()
                .push(Arc::clone(&arc));
        }

        // Race index
        if let Some(race) = &arc.race {
            self.by_race
                .entry(format!("{:?}", race))
                .or_default()
                .push(Arc::clone(&arc));
        }

        // Type index
        for card_type in &arc.card_types {
            self.by_type
                .entry(format!("{:?}", card_type))
                .or_default()
                .push(Arc::clone(&arc));
        }

        // Passcode index
        if let Some(pw) = arc.password {
            self.by_passcode.insert(pw, Arc::clone(&arc));
        }
    }

    // ── Queries ───────────────────────────────────────────────

    /// Look up a card by exact name.
    pub fn get(&self, name: &str) -> Option<Arc<Card>> {
        self.cards.get(name).cloned()
    }

    /// Look up a card by passcode — links DuelScript to BabelCdb.
    /// O(1) lookup via the passcode index.
    pub fn get_by_passcode(&self, passcode: u32) -> Option<Arc<Card>> {
        self.by_passcode.get(&passcode).cloned()
    }

    /// Check if a card exists by name.
    pub fn contains(&self, name: &str) -> bool {
        self.cards.contains_key(name)
    }

    /// Check if a card exists by passcode. O(1).
    pub fn contains_passcode(&self, passcode: u32) -> bool {
        self.by_passcode.contains_key(&passcode)
    }

    /// All cards in a given archetype.
    pub fn by_archetype(&self, archetype: &str) -> Vec<Arc<Card>> {
        self.by_archetype
            .get(archetype)
            .cloned()
            .unwrap_or_default()
    }

    /// All cards with a given attribute.
    pub fn by_attribute(&self, attribute: &Attribute) -> Vec<Arc<Card>> {
        self.by_attribute
            .get(&format!("{:?}", attribute))
            .cloned()
            .unwrap_or_default()
    }

    /// All cards with a given race.
    pub fn by_race(&self, race: &Race) -> Vec<Arc<Card>> {
        self.by_race
            .get(&format!("{:?}", race))
            .cloned()
            .unwrap_or_default()
    }

    /// All cards of a given type.
    pub fn by_type(&self, card_type: &CardType) -> Vec<Arc<Card>> {
        self.by_type
            .get(&format!("{:?}", card_type))
            .cloned()
            .unwrap_or_default()
    }

    /// All monster cards.
    pub fn all_monsters(&self) -> Vec<Arc<Card>> {
        self.cards
            .values()
            .filter(|c| c.card_types.iter().any(|t| t.is_monster()))
            .cloned()
            .collect()
    }

    /// All spell cards.
    pub fn all_spells(&self) -> Vec<Arc<Card>> {
        self.cards
            .values()
            .filter(|c| c.card_types.iter().any(|t| t.is_spell()))
            .cloned()
            .collect()
    }

    /// All trap cards.
    pub fn all_traps(&self) -> Vec<Arc<Card>> {
        self.cards
            .values()
            .filter(|c| c.card_types.iter().any(|t| t.is_trap()))
            .cloned()
            .collect()
    }

    /// Search cards by a predicate.
    pub fn search<F>(&self, predicate: F) -> Vec<Arc<Card>>
    where
        F: Fn(&Card) -> bool,
    {
        self.cards
            .values()
            .filter(|c| predicate(c))
            .cloned()
            .collect()
    }

    /// Search cards whose name contains a substring (case-insensitive).
    pub fn search_by_name(&self, query: &str) -> Vec<Arc<Card>> {
        let query = query.to_lowercase();
        self.cards
            .values()
            .filter(|c| c.name.to_lowercase().contains(&query))
            .cloned()
            .collect()
    }

    /// Filter a card against a CardFilter — used by the engine
    /// to resolve targeting expressions from DuelScript.
    pub fn matches_filter(card: &Card, filter: &CardFilter) -> bool {
        match filter {
            CardFilter::Monster         => card.card_types.iter().any(|t| t.is_monster()),
            CardFilter::Spell           => card.card_types.iter().any(|t| t.is_spell()),
            CardFilter::Trap            => card.card_types.iter().any(|t| t.is_trap()),
            CardFilter::Card            => true,
            CardFilter::Token           => false, // tokens handled separately by engine
            CardFilter::NonTokenMonster => card.card_types.iter().any(|t| t.is_monster()),
            CardFilter::TunerMonster    => card.card_types.contains(&CardType::Tuner),
            CardFilter::NonTunerMonster => {
                card.card_types.iter().any(|t| t.is_monster())
                    && !card.card_types.contains(&CardType::Tuner)
            }
            CardFilter::NormalMonster   => card.card_types.contains(&CardType::NormalMonster),
            CardFilter::EffectMonster   => card.card_types.contains(&CardType::EffectMonster),
            CardFilter::FusionMonster   => card.card_types.contains(&CardType::FusionMonster),
            CardFilter::SynchroMonster  => card.card_types.contains(&CardType::SynchroMonster),
            CardFilter::XyzMonster      => card.card_types.contains(&CardType::XyzMonster),
            CardFilter::LinkMonster     => card.card_types.contains(&CardType::LinkMonster),
            CardFilter::RitualMonster   => card.card_types.contains(&CardType::RitualMonster),
            CardFilter::ArchetypeMonster(a) => {
                card.archetypes.contains(a)
                    && card.card_types.iter().any(|t| t.is_monster())
            }
            CardFilter::ArchetypeCard(a) => card.archetypes.contains(a),
            CardFilter::NamedCard(n)     => &card.name == n,
        }
    }

    // ── Stats ─────────────────────────────────────────────────

    /// Iterate over all cards in the database.
    pub fn iter_cards(&self) -> impl Iterator<Item = (&str, &Arc<Card>)> {
        self.cards.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Total number of cards loaded.
    pub fn len(&self) -> usize { self.cards.len() }

    /// True if the database is empty.
    pub fn is_empty(&self) -> bool { self.cards.is_empty() }

    /// Print a loading summary to stdout.
    pub fn print_load_summary(&self) {
        println!(
            "CardDatabase: {} card(s) loaded, {} load error(s)",
            self.cards.len(),
            self.load_errors.len(),
        );
        for e in &self.load_errors {
            println!("  ✗ {}: {}", e.path.display(), e.kind);
        }
    }
}

// ── CardType helpers ──────────────────────────────────────────

impl CardType {
    pub fn is_monster(&self) -> bool {
        matches!(self,
            CardType::NormalMonster  | CardType::EffectMonster  | CardType::RitualMonster
          | CardType::FusionMonster  | CardType::SynchroMonster | CardType::XyzMonster
          | CardType::LinkMonster    | CardType::PendulumMonster
        )
    }

    pub fn is_spell(&self) -> bool {
        matches!(self,
            CardType::NormalSpell   | CardType::QuickPlaySpell | CardType::ContinuousSpell
          | CardType::EquipSpell    | CardType::FieldSpell     | CardType::RitualSpell
        )
    }

    pub fn is_trap(&self) -> bool {
        matches!(self,
            CardType::NormalTrap | CardType::CounterTrap | CardType::ContinuousTrap
        )
    }

    pub fn is_extra_deck(&self) -> bool {
        matches!(self,
            CardType::FusionMonster | CardType::SynchroMonster
          | CardType::XyzMonster   | CardType::LinkMonster
        )
    }
}

// ── Load Error ────────────────────────────────────────────────

#[derive(Debug)]
pub struct LoadError {
    pub path: PathBuf,
    pub kind: LoadErrorKind,
}

#[derive(Debug)]
pub enum LoadErrorKind {
    NoFilesFound,
    ReadError(String),
    ParseError(String),
    ValidationErrors(Vec<String>),
}

impl std::fmt::Display for LoadErrorKind {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        match self {
            LoadErrorKind::NoFilesFound          => write!(f, "no .ds files found"),
            LoadErrorKind::ReadError(e)          => write!(f, "read error: {}", e),
            LoadErrorKind::ParseError(e)         => write!(f, "parse error: {}", e),
            LoadErrorKind::ValidationErrors(es)  => {
                write!(f, "{} validation error(s): {}", es.len(), es.join("; "))
            }
        }
    }
}

// ── File Collection ───────────────────────────────────────────

fn collect_ds_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_recursive(dir, &mut out);
    out.sort();
    out
}

fn collect_recursive(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_recursive(&path, out);
        } else if path.extension().map_or(false, |e| e == "ds") {
            out.push(path);
        }
    }
}
