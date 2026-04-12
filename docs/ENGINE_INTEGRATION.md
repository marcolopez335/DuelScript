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
            Stat::Atk         => self.game_state.get_atk(card_id),
            Stat::Def         => self.game_state.get_def(card_id),
            Stat::Level       => self.game_state.get_level(card_id) as i32,
            Stat::Rank        => self.game_state.get_rank(card_id) as i32,
            Stat::BaseAtk     => self.game_state.get_base_atk(card_id),
            Stat::BaseDef     => self.game_state.get_base_def(card_id),
            Stat::OriginalAtk => self.game_state.get_original_atk(card_id),
            Stat::OriginalDef => self.game_state.get_original_def(card_id),
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

## Step 4b: Phase 1–3 Trait Methods (Advanced Features)

DuelScript has a set of advanced features — flag effects, custom events, named
bindings, change_code, history queries — that all plug in through additional
`DuelScriptRuntime` methods. Every method below has a **default no-op
implementation**, so your engine compiles without them and cards that don't use
those features run fine. Implement them as your cards need them.

### Custom Events (Phase 2)

Cards can emit and listen for named custom events. One card calls
`emit_event "summoned_this_chain_updated"` and another uses
`trigger: on_custom_event "summoned_this_chain_updated"` to react.

```rust
fn raise_custom_event(&mut self, name: &str, cards: &[u32]) {
    // Hash the name to a stable event code in your EVENT_CUSTOM range,
    // then raise it via your duel's event bus.
    let code = EVENT_CUSTOM + stable_hash(name);
    self.duel.raise_event(code, cards);
}
```

The default `trigger_to_event_code` in `type_mapper` maps
`OnCustomEvent(name)` to `EVENT_FREE_CHAIN` — override this in your engine
adapter if you want first-class custom event routing.

### Confirm Cards (Phase 3)

`confirm hand to: opponent` shows a player's hand to someone.

```rust
fn confirm_cards(&mut self, owner: u8, audience: u8, cards: &[u32]) {
    // audience: 0=you, 1=opponent, 2=both
    if cards.is_empty() {
        // empty slice = reveal the whole hand of `owner`
        let hand = self.game_state.get_hand(owner);
        self.ui.reveal(audience, &hand);
    } else {
        self.ui.reveal(audience, cards);
    }
}
```

### Announcements (Phase 3)

`announce card { filter: not extra_deck_monster } as announced` prompts the
player to name a card. The runtime returns an opaque token; later actions use
`get_announcement(token)` or the binding mechanism to read it back.

```rust
fn announce(&mut self, player: u8, kind: u8, filter_mask: u32) -> u32 {
    // kind: 0=card, 1=attribute, 2=race, 3=type, 4=level
    self.ui.prompt_announcement(player, kind, filter_mask)
}
fn get_announcement(&self, token: u32) -> u32 {
    self.announcements.get(&token).copied().unwrap_or(0)
}
```

### Flag Effects (Phase 1A)

`set_flag "flipped_once" on self { survives: [leave_field, to_gy] }` stores a
persistent flag on a card. The masks use the `RESET_*` constants in
`duelscript::compiler::type_mapper`:

```rust
use duelscript::compiler::type_mapper::{
    RESET_LEAVE_FIELD, RESET_TO_GY, RESET_CHAIN_END, // ...
};

fn register_flag(&mut self, card_id: u32, name: &str,
                 survives_mask: u32, resets_mask: u32) {
    // Translate the DuelScript mask bits to your engine's reset flags,
    // then call your equivalent of RegisterFlagEffect.
    let engine_mask = self.translate_reset_mask(survives_mask, resets_mask);
    self.duel.register_flag(card_id, stable_hash(name), engine_mask);
}
fn has_flag(&self, card_id: u32, name: &str) -> bool {
    self.duel.has_flag(card_id, stable_hash(name))
}
fn clear_flag(&mut self, card_id: u32, name: &str) {
    self.duel.clear_flag(card_id, stable_hash(name));
}
```

### History Queries (Phase 1A)

For conditions like `previous_location == field` or `previous_position == face_up`:

```rust
fn previous_location(&self, card_id: u32) -> u32 {
    self.game_state.card(card_id).previous_location
}
fn previous_position(&self, card_id: u32) -> u32 {
    self.game_state.card(card_id).previous_position
}
fn sent_by_reason(&self, card_id: u32) -> u32 {
    self.game_state.card(card_id).last_move_reason
}
```

### Named Bindings (Phase 1D)

`cost { reveal (1, fusion monster, extra_deck) as revealed }` captures a
selection that later actions reference via `revealed.name` or `revealed.atk`.
The runtime is the keeper of the binding environment for the current effect:

```rust
fn bind_last_selection(&mut self, name: &str) {
    // Called right after the inner cost runs. Snapshot whatever the
    // engine's "last selected group" was under `name`.
    let last = self.last_selection.clone();
    self.bindings.insert(name.to_string(), last);
}
fn get_binding_card(&self, name: &str) -> Option<u32> {
    self.bindings.get(name).and_then(|cards| cards.first().copied())
}
fn get_binding_field(&self, name: &str, field: &str) -> i32 {
    let Some(&card) = self.bindings.get(name).and_then(|c| c.first()) else { return 0 };
    match field {
        "atk"   => self.game_state.get_atk(card),
        "def"   => self.game_state.get_def(card),
        "level" => self.game_state.get_level(card) as i32,
        "code"  => self.game_state.get_code(card) as i32,
        "name"  => 0, // names are strings; bind via code and resolve in-engine
        _       => 0,
    }
}
```

**Important:** the binding environment must be reset between effect
resolutions. A common approach is to clear it at the start of each
`operation` callback.

### Change Name / Code (Phase 1E)

Prisma-style name copying maps to the engine's `EFFECT_CHANGE_CODE`:

```rust
fn change_card_code(&mut self, card_id: u32, code: u32, duration_mask: u32) {
    self.duel.add_effect(card_id, EFFECT_CHANGE_CODE, code, duration_mask);
}
```

`duration_mask == 0` means "until end of turn"; interpret as your engine needs.

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

DuelScript ships with a Lua-to-DSL transpiler that converts a
ProjectIgnis CardScripts directory into `.ds` files in one shot,
joining the result with BabelCdb stats. From the duelscript repo
root:

```bash
cargo run --release --bin migrate_batch \
    --features "cdb,lua_transpiler" -- \
    /path/to/CardScripts/official/ \
    /path/to/BabelCdb/cards.cdb \
    cards/official/ --all
```

The library API:

```rust
use duelscript::lua_transpiler::transpile_lua_to_ds;
use duelscript::cdb::CdbReader;

let cdb = CdbReader::open("cards.cdb")?;
let lua = std::fs::read_to_string("c55144522.lua")?;
let cdb_card = cdb.get(55144522);
let result = transpile_lua_to_ds(&lua, 55144522, "Pot of Greed", cdb_card);

println!("Tier: {:?}", result.accuracy);
std::fs::write("c55144522.ds", &result.ds_content)?;
```

The `accuracy` field tells you how completely the migrator captured
the Lua semantics:

| Tier            | Meaning                                                  |
|-----------------|----------------------------------------------------------|
| `Full`          | All Duel.X / aux.X / property-based actions mapped       |
| `High`          | >70% of actions mapped                                   |
| `Partial`       | Some actions mapped, others left as raw_effect           |
| `StructureOnly` | Effect structure preserved, no actions extracted         |
| `Failed`        | Lua source couldn't be parsed                            |

### Current corpus coverage

Against the full ProjectIgnis CardScripts/official directory
(13,298 cards) at the time of the latest sprint:

| Tier            | Cards   | %      |
|-----------------|---------|--------|
| Full            | 12,713  | 95.7%  |
| High            |     40  |  0.3%  |
| Partial         |      4  |  0.0%  |
| StructureOnly   |    529  |  4.0%  |
| Failed          |      0  |  0.0%  |

All 13,298 cards parse cleanly through the validator with **0
errors and 0 warnings**. A 2,000-card stride-sampled compile sweep
achieves a 100% success rate with an average of 2.6 effects per
card. A 10-card differential test suite verifies exact game-state
changes for canonical cards (Pot of Greed, Raigeki, Dark Hole,
etc.).

The remaining StructureOnly cards use EDOPro effect codes (chain
manipulation, LP cost change, GY redirect, dynamic SetValue
functions) whose semantics don't map to any single DSL action.

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
