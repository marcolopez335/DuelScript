# DuelScript v2 Language Reference

DuelScript is a standalone scripting language for Yu-Gi-Oh card mechanics. Each `.ds` file describes what a card does — how it's summoned, what it costs, what it does, what it restricts. The engine handles everything else: game rules, UI, networking, AI.

## Quick Example

```
card "Pot of Greed" {
    id: 55144522
    type: Normal Spell

    effect "Draw 2" {
        speed: 1
        resolve {
            draw 2
        }
    }
}
```

## Card Structure

```
card "Name" {
    // Fields
    id: 55144522
    type: Normal Spell
    attribute: DARK
    race: Spellcaster
    level: 7
    atk: 2500
    def: 2100

    // Blocks (all optional)
    summon { ... }
    passive "Name" { ... }
    effect "Name" { ... }
    restriction "Name" { ... }
    replacement "Name" { ... }
}
```

## Fields

| Field | Values | Required |
|-------|--------|----------|
| `id` | Card passcode (unsigned integer) | Recommended |
| `type` | Card types separated by `\|` | Yes |
| `attribute` | LIGHT, DARK, FIRE, WATER, EARTH, WIND, DIVINE | Monsters |
| `race` | Dragon, Spellcaster, Warrior, etc. | Monsters |
| `level` | 1-12 | Non-Xyz/Link monsters |
| `rank` | 1-13 | Xyz monsters |
| `link` | 1-8 | Link monsters |
| `scale` | 0-13 | Pendulum monsters |
| `atk` | Number or `?` | Monsters |
| `def` | Number or `?` | Non-Link monsters |
| `link_arrows` | `[top, bottom_left, right]` | Link monsters |
| `archetype` | `["Gravekeeper's", "HERO"]` | Optional |

### Card Types

**Monsters:** Normal Monster, Effect Monster, Ritual Monster, Fusion Monster, Synchro Monster, Xyz Monster, Link Monster, Pendulum Monster

**Subtypes:** Tuner, Synchro Tuner, Flip, Gemini, Union, Spirit, Toon

**Spells:** Normal Spell, Quick-Play Spell, Continuous Spell, Equip Spell, Field Spell, Ritual Spell

**Traps:** Normal Trap, Counter Trap, Continuous Trap

## Summon Block

Describes how the card enters the field.

```
summon {
    tributes: 1                    // Tribute count for Normal Summon
    cannot_normal_summon           // Special Summon only
    cannot_special_summon          // Cannot be Special Summoned

    // Fusion materials
    fusion materials: "Dark Magician" + "Buster Blader"

    // Synchro materials
    synchro materials {
        tuner: (1, tuner monster)
        non_tuner: (1, non-tuner monster)
    }

    // Xyz materials
    xyz materials: (2, monster, where level == 4)

    // Link materials
    link materials: (2+, monster)

    // Special summon procedure (e.g., Lava Golem)
    special_summon_procedure {
        from: hand
        to: opponent_field
        cost {
            tribute (2, monster, opponent controls)
        }
    }
}
```

## Effect Block

Activated and trigger effects.

```
effect "Effect Name" {
    speed: 1                       // 1 = Spell Speed 1, 2 = Quick, 3 = Counter
    mandatory                      // Mandatory trigger (omit for optional)
    once_per_turn: hard            // hard = card-specific, soft = can retry if negated
    timing: when                   // when = can miss timing, if = cannot
    trigger: destroyed_by_battle   // What activates this effect
    who: controller                // Who this effect applies to
    damage_step: true              // Can activate during damage step

    condition: lp >= 1000          // Activation condition
    activate_from: [hand, gy]      // Override default activation location

    target (1, monster, opponent controls)   // Targeting declaration

    cost {                         // Activation cost
        pay_lp 1000
        discard (1, card)
        tribute (1, monster, you control)
    }

    resolve {                      // What happens when the effect resolves
        destroy (all, monster, opponent controls)
        draw 2
        damage opponent 1000
    }
}
```

### Speed

| Speed | Meaning | Used By |
|-------|---------|---------|
| 1 | Spell Speed 1 | Normal Spells, Ignition effects, Trigger effects |
| 2 | Spell Speed 2 | Quick-Play Spells, Traps, Quick effects |
| 3 | Spell Speed 3 | Counter Traps only |

### Frequency

- `once_per_turn: hard` — Hard OPT (tied to card name, even if negated)
- `once_per_turn: soft` — Soft OPT (can retry if negated)
- `once_per_duel` — Once per duel
- `twice_per_turn` — Twice per turn

### Triggers

**Summon:** `summoned`, `summoned by fusion`, `normal_summoned`, `tribute_summoned`, `flip_summoned`, `flipped`, `special_summoned`

**Destruction:** `destroyed`, `destroyed_by_battle`, `destroyed_by_effect`, `destroys_by_battle`

**Movement:** `sent_to gy`, `sent_to gy from field`, `leaves_field`, `banished`, `returned_to hand`

**Battle:** `attack_declared`, `opponent_attack_declared`, `attacked`, `battle_damage to opponent`, `direct_attack_damage`, `damage_calculation`

**Phase:** `standby_phase`, `end_phase`, `draw_phase`, `main_phase`, `battle_phase`

**Chain:** `summon_attempt`, `opponent_activates [activate_spell, negate]`, `spell_trap_activated`, `chain_link`

**Status:** `targeted`, `position_changed`, `equipped`, `used_as_material for fusion`

## Passive Block

Always-on continuous effects.

```
passive "ATK Boost" {
    scope: self                    // self = this card, field = all cards
    target: (all, monster, you control, where race == Dragon)
    modifier: atk + 500
    grant: piercing
    negate_effects                 // Negate targeted cards' effects
    set_atk: 0                    // Set ATK to fixed value
}
```

## Restriction Block

Grant or deny abilities.

```
restriction "No Normal Summon" {
    apply_to: summoner
    cannot_normal_summon
    duration: this_turn
    trigger: summoned              // When this restriction activates
    condition: on_field            // When this restriction applies
}
```

## Replacement Block

Replace events with different outcomes.

```
replacement "Banish Instead" {
    instead_of: sent_to_gy
    do {
        banish self
    }
}
```

## Selectors

Selectors describe which cards to target/affect.

```
// Keywords
self                              // This card
target                            // Previously selected target
searched                          // Result of last search
equipped_card                     // Card equipped to this

// Counted selectors
(quantity, filter, controller, zone, position, where_clause)

// Examples
(1, monster, opponent controls)
(all, spell, either controls)
(2, "Gravekeeper's" monster, you control, in gy)
(1, monster, where atk >= 2000 and race == Dragon)
(3, card, opponent controls, face_down)
(1, fusion monster, where level <= 6)
```

### Quantities
- `1`, `2`, `3` — exact count
- `1+`, `2+` — at least N
- `all` — all matching cards

### Filters
monster, spell, trap, card, effect monster, normal monster, fusion monster, synchro monster, xyz monster, link monster, ritual monster, pendulum monster, tuner monster, non-tuner monster, non-token monster

### Controllers
`you control`, `opponent controls`, `either controls`

### Zones
`in hand`, `in field`, `in deck`, `in gy`, `in banished`, `in extra_deck`, `from deck`, `from gy`, `on your field`

### Positions
`face_up`, `face_down`, `in attack_position`, `in defense_position`, `except self`

### Where Clauses
```
where atk >= 1500
where level <= 4 and race == Warrior
where attribute == DARK
where is_face_up and is_effect
```

## Actions

### Card Movement
| Action | Syntax |
|--------|--------|
| Draw | `draw 2` |
| Discard | `discard (1, card, opponent controls)` |
| Destroy | `destroy (all, monster, opponent controls)` |
| Banish | `banish (1, monster) from gy` |
| Send to GY | `send self to gy` |
| Return | `return (1, card) to hand` / `to deck shuffle` |
| Search | `search (1, monster, where level <= 4) from deck` |
| Add to hand | `add_to_hand searched` |
| Special Summon | `special_summon (1, monster) from gy in defense_position` |
| Set | `set (1, card) from hand` |
| Flip down | `flip_down target` |
| Mill | `mill 3` |
| Excavate | `excavate 5 from your_deck` |

### Combat & Control
| Action | Syntax |
|--------|--------|
| Negate | `negate` / `negate and destroy` |
| Negate effects | `negate_effects target until end_of_turn` |
| Take control | `take_control target until end_phase` |
| Change position | `change_position target to defense_position` |
| Equip | `equip target to self` |
| Grant ability | `grant self piercing until end_of_turn` |

### Life Points
| Action | Syntax |
|--------|--------|
| Damage | `damage opponent 1000` |
| Gain LP | `gain_lp 500` |
| Pay LP | `pay_lp 800` |

### Stat Modification
| Action | Syntax |
|--------|--------|
| Modify ATK | `modify_atk self + 500 until end_of_turn` |
| Set ATK | `set_atk target 0` |
| Change level | `change_level target to 1` |
| Change attribute | `change_attribute target to DARK` |

### Tokens
```
create_token {
    name: "Sheep Token"
    attribute: EARTH
    race: Beast
    atk: 0
    def: 0
    count: 4
    position: defense_position
}
```

### Control Flow
```
if (lp >= 2000) { draw 1 } else { damage you 500 }
for_each (all, monster) in gy { banish target }
delayed until end_phase { destroy self }
then { draw 1 }
also { gain_lp 500 }
and_if_you_do { destroy target }
```

## Expressions

Used for dynamic values (damage amounts, stat modifications, etc.).

```
2                                  // Literal
half                               // Half (context-dependent, usually LP)
self.atk                           // This card's ATK
target.level                       // Target's level
your_lp                            // Your life points
opponent_lp                        // Opponent's life points
count((all, monster, in gy))       // Count matching cards
self.atk + 500                     // Arithmetic
count((all, "Dragon" monster)) * 500  // Complex expression
```

## Costs

```
cost {
    pay_lp 1000
    pay_lp half                    // Half your LP
    discard (1, card)
    discard (1, monster, where attribute == WATER)
    tribute (1, monster, you control)
    tribute self
    banish (1, card) from gy
    send self to gy
    detach 2 from self             // Xyz materials
    remove_counter "Spell Counter" 1 from self
    reveal (1, card)
    announce type
    none                           // No cost
}
```

## Conditions

```
condition: lp >= 1000
condition: opponent_lp <= 4000
condition: hand_size >= 1
condition: cards_in_gy >= 5
condition: you controls (1, monster)
condition: no monster on opponent field
condition: on_field
condition: in_gy
condition: phase == battle
condition: chain_includes [search, special_summon]
condition: self face_up
condition: lp >= 1000 and cards_in_gy >= 3
```

## Duration

Used in grants, stat modifications, and take control effects.

`this_turn`, `end_of_turn`, `end_phase`, `end_of_damage_step`, `next_standby_phase`, `while_on_field`, `while_face_up`, `permanently`, `2_turns`

## Grant Abilities

`cannot_attack`, `cannot_attack_directly`, `cannot_change_position`, `cannot_be_destroyed`, `cannot_be_destroyed by battle`, `cannot_be_destroyed by effect`, `cannot_be_targeted`, `cannot_be_targeted by effects`, `cannot_be_tributed`, `cannot_be_used_as_material`, `cannot_activate`, `cannot_activate effects`, `cannot_normal_summon`, `cannot_special_summon`, `unaffected_by spells`, `unaffected_by traps`, `unaffected_by effects`, `piercing`, `direct_attack`, `double_attack`, `triple_attack`, `attack_all_monsters`, `must_attack`, `immune_to_targeting`

## Engine Integration

To use DuelScript in your engine:

```rust
use duelscript::v2::parser::parse_v2;
use duelscript::v2::validator::validate_v2;
use duelscript::v2::compiler::compile_card_v2;

// Parse a .ds file
let source = std::fs::read_to_string("card.ds")?;
let file = parse_v2(&source)?;

// Validate
let report = validate_v2(&file);
if report.has_errors() { /* handle */ }

// Compile to engine metadata
let compiled = compile_card_v2(&file.cards[0]);
for effect in &compiled.effects {
    // effect.effect_type  — EDOPro-compatible bitflags
    // effect.category     — CATEGORY_DRAW, CATEGORY_DESTROY, etc.
    // effect.code         — Event code (EVENT_FREE_CHAIN, etc.)
    // effect.property     — EFFECT_FLAG_CARD_TARGET, etc.
    // effect.range        — LOCATION_MZONE, LOCATION_SZONE, etc.
    // effect.count_limit  — OPT/OPD limits
    // effect.condition    — Rust closure: fn(&dyn DuelScriptRuntime) -> bool
    // effect.cost         — Rust closure: fn(&mut dyn DuelScriptRuntime, bool) -> bool
    // effect.target       — Rust closure: fn(&mut dyn DuelScriptRuntime, bool) -> bool
    // effect.operation    — Rust closure: fn(&mut dyn DuelScriptRuntime)
}
```

### DuelScriptRuntime Trait

Your engine implements `DuelScriptRuntime` to provide game state access:

```rust
use duelscript::compiler::callback_gen::DuelScriptRuntime;

impl DuelScriptRuntime for MyEngine {
    fn get_lp(&self, player: u8) -> i32 { ... }
    fn draw(&mut self, player: u8, count: u32) -> u32 { ... }
    fn destroy(&mut self, card_ids: &[u32]) -> u32 { ... }
    fn damage(&mut self, player: u8, amount: i32) -> bool { ... }
    fn negate_activation(&mut self) -> bool { ... }
    fn special_summon(&mut self, card_id: u32, player: u8, pos: u32) -> bool { ... }
    // ... see callback_gen.rs for full trait
}
```

### Batch Loading

```rust
// Load all .ds files from a directory
let dir = std::fs::read_dir("cards/")?;
for entry in dir {
    let source = std::fs::read_to_string(entry.path())?;
    if let Ok(file) = parse_v2(&source) {
        for card in &file.cards {
            let compiled = compile_card_v2(card);
            engine.register_card(compiled);
        }
    }
}
```

## File Naming Convention

Official cards use `c{passcode}.ds` (e.g., `c55144522.ds` for Pot of Greed). This matches the Lua script naming convention used by EDOPro/YGOPro engines.
