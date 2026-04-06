# DuelScript Engine Integration Guide

How to integrate DuelScript into your Yu-Gi-Oh game engine.

---

## Overview

DuelScript is a **compile-time replacement for Lua card scripts**. Your engine:

1. Parses `.ds` files into an AST
2. Compiles the AST into effect metadata (u32 bitfields)
3. Implements a runtime trait so callbacks can execute game actions

```
.ds file → parse() → AST → compile_card() → CompiledCard
                                                  ↓
                                          effect_type, category,
                                          code, property, range,
                                          count_limit, callbacks
```

---

## Step 1: Add Dependency

```toml
[dependencies]
duelscript = { path = "../duelscript" }
# or from crates.io once published:
# duelscript = "0.5"
```

---

## Step 2: Parse and Compile

```rust
use duelscript::{parse, compile_card};

// Parse a .ds file
let source = std::fs::read_to_string("cards/official/c55144522.ds")?;
let file = parse(&source)?;

// Compile each card
for card in &file.cards {
    let compiled = compile_card(card);

    println!("Card: {} (ID: {})", compiled.name, compiled.card_id);
    for effect in &compiled.effects {
        println!("  effect_type: {:#x}", effect.effect_type);
        println!("  category:    {:#x}", effect.category);
        println!("  code:        {}", effect.code);
        println!("  range:       {:#x}", effect.range);
    }
}
```

---

## Step 3: Load Card Database

```rust
use duelscript::database::CardDatabase;
use duelscript::compiler::compile_card;

// Load all .ds files from a directory
let db = CardDatabase::load_from_dir(Path::new("cards/official"));

// Look up by card ID (O(1))
if let Some(card) = db.get_by_passcode(55144522) {
    let compiled = compile_card(&card);
    // Register effects in your engine...
}

// Search by name
let dragons = db.search_by_name("dragon");
```

---

## Step 4: Implement the Runtime Trait

To make callbacks actually execute game actions, implement `DuelScriptRuntime`:

```rust
use duelscript::compiler::callback_gen::DuelScriptRuntime;
use duelscript::ast::{CardFilter, Stat, Player, Zone};

struct MyRuntime {
    // Your game state references
    game_state: &mut GameState,
    player: u8,
    card_id: u32,
}

impl DuelScriptRuntime for MyRuntime {
    // === Queries ===
    fn get_lp(&self, player: u8) -> i32 {
        self.game_state.players[player as usize].lp
    }
    fn get_hand_count(&self, player: u8) -> usize {
        self.game_state.players[player as usize].hand.len()
    }
    fn get_deck_count(&self, player: u8) -> usize {
        self.game_state.players[player as usize].deck.len()
    }
    fn get_gy_count(&self, player: u8) -> usize {
        self.game_state.players[player as usize].graveyard.len()
    }
    fn get_banished_count(&self, player: u8) -> usize {
        self.game_state.players[player as usize].banished.len()
    }
    fn get_field_card_count(&self, player: u8, location: u32) -> usize {
        self.game_state.count_cards_at(player, location)
    }
    fn get_field_cards(&self, player: u8, location: u32) -> Vec<u32> {
        self.game_state.get_card_ids_at(player, location)
    }
    fn card_matches_filter(&self, card_id: u32, filter: &CardFilter) -> bool {
        // Match card against the filter using your card database
        match filter {
            CardFilter::Monster => self.game_state.is_monster(card_id),
            CardFilter::Spell => self.game_state.is_spell(card_id),
            // ... etc
        }
    }
    fn get_card_stat(&self, card_id: u32, stat: &Stat) -> i32 {
        match stat {
            Stat::Atk => self.game_state.get_atk(card_id),
            Stat::Def => self.game_state.get_def(card_id),
            Stat::Level => self.game_state.get_level(card_id) as i32,
            Stat::Rank => self.game_state.get_rank(card_id) as i32,
        }
    }
    fn effect_card_id(&self) -> u32 { self.card_id }
    fn effect_player(&self) -> u8 { self.player }
    fn event_categories(&self) -> u32 {
        self.game_state.current_chain_categories()
    }

    // === Card Movement ===
    fn draw(&mut self, player: u8, count: u32) -> u32 {
        self.game_state.draw_cards(player, count)
    }
    fn destroy(&mut self, card_ids: &[u32]) -> u32 {
        self.game_state.destroy_cards(card_ids)
    }
    fn send_to_grave(&mut self, card_ids: &[u32]) -> u32 {
        self.game_state.send_to_graveyard(card_ids)
    }
    // ... implement remaining methods

    // === Selection (UI) ===
    fn select_cards(&mut self, player: u8, candidates: &[u32],
                    min: usize, max: usize) -> Vec<u32> {
        // Present selection UI to the player
        self.game_state.prompt_card_selection(player, candidates, min, max)
    }
    fn select_option(&mut self, player: u8, options: &[String]) -> usize {
        // Present option choice to the player
        self.game_state.prompt_option_selection(player, options)
    }

    // ... see callback_gen.rs for the full trait definition
}
```

---

## Step 5: Wire into Your Script Loader

```rust
use duelscript::database::CardDatabase;
use duelscript::compiler::compile_card;

struct ScriptLoader {
    ds_db: CardDatabase,
}

impl ScriptLoader {
    fn load(&self, card_id: u32) -> Option<Vec<EffectDefinition>> {
        let card = self.ds_db.get_by_passcode(card_id)?;
        let compiled = compile_card(&card);

        Some(compiled.effects.iter().map(|eff| {
            // Convert to your engine's effect type
            YourEffectDefinition {
                effect_type: eff.effect_type,
                category: eff.category,
                code: eff.code,
                property: eff.property,
                range: eff.range,
                count_limit: eff.count_limit.as_ref().map(|cl| (cl.count, cl.code)),
                // Wrap callbacks for your engine
                condition: eff.callbacks.condition.clone(),
                cost: eff.callbacks.cost.clone(),
                target: eff.callbacks.target.clone(),
                operation: eff.callbacks.operation.clone(),
            }
        }).collect())
    }
}
```

---

## Step 6: Fallback to Lua

DuelScript supports incremental migration. Try `.ds` first, fall back to Lua:

```rust
fn load_card_script(&mut self, card_id: u32) {
    // Try DuelScript first
    if let Some(card) = self.ds_database.get_by_passcode(card_id) {
        let compiled = compile_card(&card);
        self.register_ds_effects(card_id, &compiled);
        return;
    }

    // Fall back to Lua
    self.lua_executor.load_card(card_id);
}
```

---

## Bitfield Reference

All constants match EDOPro/YGOPro `constant.lua` exactly.

### Effect Types
| Constant | Value | DuelScript Syntax |
|----------|-------|-------------------|
| `EFFECT_TYPE_SINGLE` | 0x1 | Self-targeting trigger |
| `EFFECT_TYPE_FIELD` | 0x2 | Field-watching trigger/continuous |
| `EFFECT_TYPE_EQUIP` | 0x4 | Equip effect |
| `EFFECT_TYPE_ACTIVATE` | 0x10 | Spell/Trap activation |
| `EFFECT_TYPE_IGNITION` | 0x40 | `speed: spell_speed_1` (monster, no trigger) |
| `EFFECT_TYPE_TRIGGER_O` | 0x80 | `optional: true` + trigger |
| `EFFECT_TYPE_QUICK_O` | 0x100 | `speed: spell_speed_2` + `optional: true` |
| `EFFECT_TYPE_TRIGGER_F` | 0x200 | Mandatory trigger |
| `EFFECT_TYPE_QUICK_F` | 0x400 | `speed: spell_speed_2` (mandatory) |

### Categories
| Constant | Value | DuelScript Action |
|----------|-------|-------------------|
| `CATEGORY_DESTROY` | 0x1 | `destroy` |
| `CATEGORY_SPECIAL_SUMMON` | 0x200 | `special_summon` |
| `CATEGORY_DRAW` | 0x10000 | `draw` |
| `CATEGORY_SEARCH` | 0x20000 | `search` |
| `CATEGORY_NEGATE` | 0x10000000 | `negate activation` |
| `CATEGORY_DISABLE` | 0x4000 | `negate effect` |

### Event Codes
| Constant | Value | DuelScript Trigger |
|----------|-------|-------------------|
| `EVENT_FREE_CHAIN` | 1002 | (no trigger) |
| `EVENT_CHAINING` | 1027 | `opponent_activates [...]` |
| `EVENT_SUMMON_SUCCESS` | 1100 | `when_summoned` |
| `EVENT_SPSUMMON_SUCCESS` | 1102 | `when_summoned by_special_summon` |
| `EVENT_DESTROYED` | 1029 | `when_destroyed` |
| `EVENT_ATTACK_ANNOUNCE` | 1130 | `when attack_declared` |
| `EVENT_BE_BATTLE_TARGET` | 1131 | `when_attacked` |
| `EVENT_PHASE + PHASE_END` | 0x1200 | `during_end_phase` |

---

## Validation

```rust
use duelscript::{parse, validator::validate};

let file = parse(source)?;
let errors = validate(&file);

for err in &errors {
    println!("{}", err); // [ERROR] Card Name: message
}
```

Validates: stat ranges, tribute consistency, spell speeds, link arrow counts,
material compatibility, timing declarations, and more.

---

## Migration from Lua

```rust
use duelscript::migrate::{generate_from_lua, migrate_directory, Confidence};

// Single file
let result = generate_from_lua(lua_source, 55144522, "Pot of Greed");
println!("Confidence: {}", result.confidence.label());
std::fs::write("c55144522.ds", &result.ds_content)?;

// Batch — entire directory
let results = migrate_directory(Path::new("CardScripts/official"));
let high = results.iter().filter(|r| r.confidence == Confidence::High).count();
println!("{}/{} cards migrated at HIGH confidence", high, results.len());
```

---

## Architecture

```
┌─────────────────────────────────────────┐
│              .ds source files            │
│         (c55144522.ds, c14558127.ds)     │
└────────────────┬────────────────────────┘
                 │
    ┌────────────▼────────────────────────┐
    │         duelscript crate             │
    │  ┌──────────┐  ┌──────────────────┐ │
    │  │  Parser   │  │    Validator     │ │
    │  │ (PEG→AST) │  │ (semantic checks)│ │
    │  └────┬─────┘  └─────────────────┘ │
    │       │                              │
    │  ┌────▼─────────────────────────┐   │
    │  │        Compiler               │   │
    │  │  type_mapper → bitfields      │   │
    │  │  callback_gen → closures      │   │
    │  │  expr_eval → runtime values   │   │
    │  └────┬─────────────────────────┘   │
    └───────┼──────────────────────────────┘
            │
  ┌─────────▼──────────┐
  │   Your Engine       │
  │                     │
  │  Implements:        │
  │  DuelScriptRuntime  │
  │                     │
  │  Receives:          │
  │  CompiledEffect     │
  │  (effect_type,      │
  │   category, code,   │
  │   callbacks)        │
  └─────────────────────┘
```
