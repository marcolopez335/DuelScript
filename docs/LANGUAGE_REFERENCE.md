# DuelScript Language Reference
## Version 0.4 — Canonical Specification

---

> DuelScript is a **universal card definition format** for Yu-Gi-Oh card mechanics.
> It is engine-agnostic — any engine, written in any language, can parse and consume `.ds` files.
> The grammar and this document are the spec. The Rust crate is the reference implementation.

---

## Table of Contents

1. [Philosophy](#philosophy)
2. [File Format](#file-format)
3. [Card Structure](#card-structure)
4. [Card Fields](#card-fields)
5. [Card Types](#card-types)
6. [Attributes & Races](#attributes--races)
7. [Archetypes](#archetypes)
8. [Summon Conditions](#summon-conditions)
9. [Materials](#materials)
10. [Link Arrows](#link-arrows)
11. [Pendulum Effects](#pendulum-effects)
12. [Effect Blocks](#effect-blocks)
13. [Continuous Effects](#continuous-effects)
14. [Replacement Effects](#replacement-effects)
15. [Equip Effects](#equip-effects)
16. [Counter Systems](#counter-systems)
17. [Win Conditions](#win-conditions)
18. [Triggers](#triggers)
19. [Conditions](#conditions)
20. [Costs](#costs)
21. [Game Actions](#game-actions)
22. [Target Expressions](#target-expressions)
23. [Zones](#zones)
24. [Durations](#durations)
25. [Restrictions](#restrictions)
26. [Engine Responsibilities](#engine-responsibilities)
27. [Implementation Guide](#implementation-guide)

---

## Philosophy

DuelScript answers one question: **"What makes this card unique?"**

It declares card *identity* and *intent*. It does not simulate a duel.

**DuelScript owns:**
- Card identity — stats, types, archetypes, attributes
- Effect structure — what triggers an effect, what it costs, what it does
- Summon requirements — tribute counts, material rules
- Continuous modifiers and restrictions
- Win and lose conditions

**The engine owns:**
- When effects can legally activate (timing windows)
- Chain resolution order and SEGOC
- Damage step legality
- Spell speed conflict resolution
- All game state management

This separation means a `.ds` file is a legal card definition that any conforming engine can consume. The engine decides how to execute the declared intent.

---

## File Format

DuelScript files use the `.ds` extension. A single file may contain one or more card definitions. UTF-8 encoding. Comments use `//` (line) or `/* */` (block).

```ds
// This is a line comment

/*
  This is a block comment
*/

card "Card Name" {
  // card body
}
```

---

## Card Structure

Every card follows this structure. All sections are optional except the card body itself:

```ds
card "Card Name" {

  // ── Identity fields ──────────────
  type:      Effect Monster | Tuner
  attribute: DARK
  race:      Spellcaster
  level:     4
  atk:       1800
  def:       0
  password:  12345678

  // ── Archetype membership ─────────
  archetype: ["Spellcaster", "Chaos"]

  // ── Summon rules ─────────────────
  summon_condition { ... }
  materials        { ... }
  link_arrows:     [top, bottom_left, bottom_right]

  // ── Pendulum ─────────────────────
  scale: 4
  pendulum_effect { ... }

  // ── Effects ──────────────────────
  effect "Effect Name" { ... }
  continuous_effect   { ... }
  replacement_effect  { ... }
  equip_effect        { ... }
  counter_system      { ... }

  // ── Special ──────────────────────
  win_condition { ... }

  // ── Flavor ───────────────────────
  flavor: "A powerful monster from ancient times."
}
```

---

## Card Fields

| Field       | Type          | Required For          | Notes                              |
|-------------|---------------|-----------------------|------------------------------------|
| `type`      | CardType list | All cards             | Pipe-separated: `Effect Monster \| Tuner` |
| `attribute` | Attribute     | Monsters              |                                    |
| `race`      | Race          | Monsters              |                                    |
| `level`     | integer       | Main deck monsters    | Not for Xyz (use `rank`) or Link   |
| `rank`      | integer       | Xyz monsters          |                                    |
| `link`      | integer       | Link monsters         | Must equal number of link arrows   |
| `scale`     | integer       | Pendulum monsters     |                                    |
| `atk`       | integer or `?`| Monsters              | `?` for variable ATK               |
| `def`       | integer or `?`| Non-Link monsters     | Link monsters have no DEF          |
| `password`  | integer       | Optional              | Official card password             |
| `flavor`    | string        | Normal monsters       | Flavor text shown on card          |

---

## Card Types

Types are declared with `|` separators. A card may have multiple subtypes.

### Monster Types
| Keyword           | Meaning                        |
|-------------------|--------------------------------|
| `Normal Monster`  | No effect                      |
| `Effect Monster`  | Has one or more effects        |
| `Ritual Monster`  | Ritual summon only             |
| `Fusion Monster`  | Extra deck — Fusion            |
| `Synchro Monster` | Extra deck — Synchro           |
| `Xyz Monster`     | Extra deck — Xyz               |
| `Link Monster`    | Extra deck — Link              |
| `Pendulum Monster`| Has Pendulum scale and effect  |

### Monster Subtypes
| Keyword        | Meaning                        |
|----------------|--------------------------------|
| `Tuner`        | Can be used as Synchro tuner   |
| `Synchro Tuner`| Tuner that is also a Synchro   |
| `Gemini`       | Gemini monster                 |
| `Union`        | Union monster                  |
| `Spirit`       | Returns to hand end phase      |
| `Flip`         | Flip effect monster            |
| `Toon`         | Toon monster                   |

### Spell Types
| Keyword            | Spell Speed |
|--------------------|-------------|
| `Normal Spell`     | 1           |
| `Quick-Play Spell` | 2           |
| `Continuous Spell` | 1 (ongoing) |
| `Equip Spell`      | 1           |
| `Field Spell`      | 1           |
| `Ritual Spell`     | 1           |

### Trap Types
| Keyword          | Spell Speed |
|------------------|-------------|
| `Normal Trap`    | 2           |
| `Counter Trap`   | 3           |
| `Continuous Trap`| 2 (ongoing) |

---

## Attributes & Races

### Attributes
`LIGHT` `DARK` `FIRE` `WATER` `EARTH` `WIND` `DIVINE`

### Races
`Dragon` `Spellcaster` `Zombie` `Warrior` `Beast-Warrior` `Beast`
`Winged Beast` `Fiend` `Fairy` `Insect` `Dinosaur` `Reptile`
`Fish` `Sea Serpent` `Aqua` `Pyro` `Thunder` `Rock` `Plant`
`Machine` `Psychic` `Divine-Beast` `Wyrm` `Cyberse`

---

## Archetypes

Archetype membership is declared as a string list. Other cards can reference these names in target filters.

```ds
archetype: ["Blue-Eyes", "Dragon"]

// Another card can then target:
search (1, "Blue-Eyes" monster) from deck
```

A card may belong to multiple archetypes.

---

## Summon Conditions

The `summon_condition` block declares how a card can legally be summoned. The engine enforces these rules.

```ds
summon_condition {
  tributes_required:    2          // 1 for level 5-6, 2 for level 7+
  tribute_material:     monster    // tribute must match this filter
  cannot_normal_summon: true       // cannot be normal summoned or set
  special_summon_only:  true       // can only be special summoned
  must_be_summoned_by:  own_effect // must be summoned by card's own effect
  special_summon_from:  [hand, gy] // can only be special summoned from these zones
  summon_once_per_turn: true       // can only be summoned once per turn
}
```

### `must_be_summoned_by` Values

| Value             | Meaning                               |
|-------------------|---------------------------------------|
| `own_effect`      | Must be summoned by this card's effect|
| `ritual_spell`    | Must be ritual summoned               |
| `fusion_spell`    | Must be fusion summoned               |
| `specific_card: "Polymerization"` | Must be summoned by that card |
| `by_fusion_summon` | Any fusion summon method             |
| `by_synchro_summon`| Any synchro summon                  |
| `by_xyz_summon`    | Any xyz summon                       |
| `by_link_summon`   | Any link summon                      |

---

## Materials

Extra deck and ritual monsters declare fusion/synchro/xyz/link material requirements.

```ds
materials {
  // Named materials (exact cards required)
  require: "Blue-Eyes White Dragon" + "Blue-Eyes White Dragon"

  // Generic materials
  require: 1 tuner monster
  require: 2+ non-tuner monster

  // Restrictions on what can be used
  cannot_use:    token
  must_include:  "Stardust Dragon"
  same_attribute: false
  same_race:      true
}
```

### Material Qualifiers
`tuner` `non-tuner` `non-token` `non-special`

### `cannot_use` Values
`token` `fusion` `synchro` `xyz` `link` or a quoted card name

---

## Link Arrows

Link monsters declare their arrows. The number must equal the link rating.

```ds
link_arrows: [top, bottom_left, bottom_right]

// Available positions:
// top_left    top    top_right
// left               right
// bottom_left bottom bottom_right
```

---

## Pendulum Effects

Pendulum monsters have both a pendulum effect (active in the spell/trap zone) and a monster effect. Declare both as separate blocks.

```ds
card "Odd-Eyes Pendulum Dragon" {
  type: Pendulum Monster | Effect Monster
  scale: 4
  level: 7

  pendulum_effect {
    // Active while in spell/trap zone as a pendulum
    once_per_turn: true
    trigger: during_standby_phase of yours
    on_resolve {
      search (1, monster, with_atk <= 1500) from deck
    }
  }

  effect "Monster Effect" {
    // Normal monster effect
  }
}
```

---

## Effect Blocks

The core of DuelScript. Each `effect` block describes one activatable effect.

```ds
effect "Optional Name" {
  speed:         spell_speed_2    // spell_speed_1 | 2 | 3
  once_per_turn: true             // frequency control
  optional:      true             // false = mandatory trigger
  timing:        when             // when | if (affects "missing the timing")

  condition: in_hand              // where the card must be
  trigger:   opponent_activates [search | special_summon]

  cost {
    discard self
  }

  on_activate {
    // actions that happen when placed on chain
  }

  on_resolve {
    // actions that happen when chain resolves
    negate effect
  }

  restriction {
    cannot: be_targeted by card_effects
  }
}
```

### Frequency Keywords

| Keyword          | Meaning                                  |
|------------------|------------------------------------------|
| `once_per_turn`  | Once per turn per card (standard OPT)    |
| `twice_per_turn` | Twice per turn                           |
| `once_per_duel`  | Once per duel (e.g. Exodia pieces)       |
| `each_turn`      | Once per turn but resets (continuous)    |

### Timing: `when` vs `if`

- `timing: when` — strict. The effect can **miss the timing** if it is not the last thing to happen.
- `timing: if` — soft. The effect **cannot miss the timing** and fires regardless.

Most trigger effects use `when`. "If X, you can" effects use `if`.

---

## Continuous Effects

Passive effects that are always active while the card is in the declared zone.

```ds
continuous_effect "Boost" {
  while: on_field              // condition for effect to be active
  apply_to: (1, monster, you controls)

  modifier: atk +500           // stat modifiers
  grant:    piercing           // ability grants
}
```

### Granted Abilities

| Keyword                           | Meaning                              |
|-----------------------------------|--------------------------------------|
| `piercing`                        | Deals battle damage through defense  |
| `double_attack`                   | Can attack twice per turn            |
| `direct_attack`                   | Can attack directly                  |
| `cannot_be_destroyed_by_battle`   | Immune to battle destruction         |
| `cannot_be_destroyed_by_effect`   | Immune to effect destruction         |
| `unaffected_by_spell_effects`     | Spells don't affect this card        |
| `unaffected_by_trap_effects`      | Traps don't affect this card         |
| `unaffected_by_monster_effects`   | Monster effects don't affect this    |
| `unaffected_by_card_effects`      | No card effects affect this card     |
| `immune_to_targeting`             | Cannot be targeted                   |

---

## Replacement Effects

"Instead of X, do Y." Intercepts a game event and replaces it.

```ds
replacement_effect "Return from Void" {
  instead_of: destroyed_by_any

  do: {
    return self to extra_deck
  }
}
```

### Replaceable Events

| Keyword                | Replaces                             |
|------------------------|--------------------------------------|
| `destroyed_by_battle`  | Destruction by battle                |
| `destroyed_by_effect`  | Destruction by card effect           |
| `destroyed_by_any`     | Any destruction                      |
| `sent_to_gy`           | Being sent to the graveyard          |
| `sent_to_gy_by_effect` | Being sent to GY by effect           |
| `sent_to_gy_by_battle` | Being sent to GY by battle           |
| `banished`             | Being banished                       |
| `returned_to_hand`     | Being returned to hand               |
| `returned_to_deck`     | Being returned to deck               |

---

## Equip Effects

For Equip Spells and monsters that equip themselves.

```ds
equip_effect {
  target: (1, warrior monster, you controls)

  while_equipped {
    modifier: atk +500
    grant:    piercing
  }

  on_equipped_destroyed {
    special_summon self from spell_trap_zone
  }

  on_unequipped {
    send self to gy
  }
}
```

---

## Counter Systems

Cards that use named spell counters.

```ds
counter_system {
  name:        "spell_counter"
  placed_when: when activate_spell
  max:         none              // or a number

  effect "Remove Counter" {
    cost { remove_counter 1 "spell_counter" from self }
    on_resolve { destroy (1, card, opponent controls) }
  }
}
```

---

## Win Conditions

Declare alternate win/lose conditions.

```ds
win_condition {
  when:   all_pieces_in_hand         // Exodia
  result: win_duel
}

win_condition {
  when:   turn_count >= 20           // Final Countdown
  result: win_duel
}

win_condition {
  when:   opponent_cannot_draw
  result: win_duel
}
```

---

## Triggers

The `trigger` clause declares what event activates the effect.

```ds
trigger: when_summoned
trigger: when_summoned by_special_summon
trigger: when_destroyed by battle
trigger: when_sent_to gy
trigger: when_flipped
trigger: when_attacked
trigger: when_tribute_summoned
trigger: when_tribute_summoned using "Monarch" monster
trigger: during main_phase_1
trigger: during_standby_phase of yours
trigger: during_standby_phase of opponents
trigger: during_end_phase
trigger: on_nth_summon: 5
trigger: opponent_activates [search | special_summon | send_to_gy]
trigger: when draw
```

### Trigger Actions (for `opponent_activates`)

`search` `special_summon` `send_to_gy` `add_to_hand` `draw` `banish` `mill`
`activate_spell` `activate_trap` `activate_monster_effect`
`fusion_summon` `synchro_summon` `xyz_summon` `link_summon` `ritual_summon`
`normal_summon` `set_card` `change_battle_position` `take_damage` `gain_lp`

---

## Conditions

The `condition` clause declares where/when this card must be for the effect to be activatable.

```ds
condition: in_hand
condition: in_gy
condition: on_field
condition: you_control_no_monsters
condition: your_lp <= 1000
condition: cards_in_gy >= 5
condition: hand_size == 0
condition: in_hand and your_lp <= 2000   // composite
```

### Condition Keywords

| Keyword                       | Meaning                              |
|-------------------------------|--------------------------------------|
| `in_hand`                     | Card is in the hand                  |
| `in_gy`                       | Card is in the graveyard             |
| `in_banished`                 | Card is banished                     |
| `on_field`                    | Card is on the field                 |
| `you_control_no_monsters`     | Controller has no monsters           |
| `opponent_controls_no_monsters`| Opponent has no monsters            |
| `field_is_empty`              | No cards on either side              |
| `your_lp OP N`                | Your life points comparison          |
| `opponent_lp OP N`            | Opponent life points comparison      |
| `hand_size OP N`              | Your hand size comparison            |
| `cards_in_gy OP N`            | Cards in your graveyard              |
| `banished_count OP N`         | Your banished cards count            |

Compare operators: `>=` `<=` `>` `<` `==` `!=`

---

## Costs

Cost actions are paid on activation, before the effect resolves. If costs cannot be paid, the effect cannot be activated.

```ds
cost {
  none                                        // No cost
  pay_lp 1000                                 // Pay life points
  discard self                                // Discard this card
  discard (1, monster)                        // Discard a card
  tribute self                                // Tribute this card
  tribute (1, monster, you controls)          // Tribute a monster
  banish self                                 // Banish this card
  banish (1, monster) from gy                 // Banish from GY
  send self to gy                             // Send this card to GY
  send (1, card) to gy                        // Send a card to GY
  remove_counter 1 "spell_counter" from self  // Remove counters
  detach 1 overlay_unit from self             // Detach Xyz material
  reveal self                                 // Reveal this card
  reveal (1, monster)                         // Reveal a card
}
```

---

## Game Actions

Actions that happen in `on_activate` or `on_resolve` blocks.

### Drawing
```ds
draw 2
```

### Summoning
```ds
special_summon self from gy
special_summon self from hand in attack_position
special_summon (1, "Blue-Eyes White Dragon") from deck
fusion_summon  "Blue-Eyes Ultimate Dragon" using monster + monster
synchro_summon "Stardust Dragon" using tuner monster + non-tuner monster
xyz_summon     "Number 39: Utopia" using monster + monster
ritual_summon  "Black Luster Soldier" using monster
```

### Negation
```ds
negate effect
negate activation
negate activation and destroy
negate summon
negate attack
```

### Destruction
```ds
destroy (1, monster, opponent controls)
destroy (2, card, either_player controls)
```

### Sending / Banishing
```ds
send (1, monster) to gy
send self to gy
banish (1, card) from gy
banish self face_down
```

### Search / Add
```ds
search (1, "Blue-Eyes" monster) from deck
add_to_hand (1, "Polymerization") from deck
```

### Return
```ds
return (1, monster, opponent controls) to hand
return self to extra_deck
return (1, card) to deck shuffle
```

### ATK / DEF Modification
```ds
modifier: atk +500 on (1, monster, you controls) until_end_of_turn
modifier: atk -500 on (1, monster, opponent controls) until_end_of_turn
set_atk (1, monster) to 0 until_end_of_turn
double_atk self until_end_of_turn
halve_atk  self until_end_of_turn
```

### Battle Position
```ds
flip_face_down (1, monster, opponent controls)
change_battle_position (1, monster, opponent controls)
set (1, monster, opponent controls)
```

### Control
```ds
take_control of (1, monster, opponent controls)
take_control of (1, monster, opponent controls) until end_of_turn
```

### Counters
```ds
place_counter  2 "spell_counter" on self
remove_counter 1 "spell_counter" from self
```

### Damage / LP
```ds
deal_damage to opponent: 1000
deal_damage to opponent: self.atk
deal_damage to both_players: 500
gain_lp: 1000
gain_lp: half_opponent_lp
```

### Tokens
```ds
create_token {
  name:      "Sheep Token"
  attribute: EARTH
  race:      Beast
  atk:       0
  def:       0
  count:     2
  position:  defense_position
}
```

### Loops / Conditionals
```ds
for_each (1, "Machine" monster, you controls) in monster_zone {
  double_atk self until_end_of_turn
}

if (your_lp <= 2000) {
  draw 2
} else {
  gain_lp: 1000
}
```

### Miscellaneous
```ds
mill 3
mill 2 from opponent_deck
shuffle deck
look_at (3, card, opponent controls, top_of_deck)
reveal self
copy_effect of (1, monster, opponent controls)
equip (1, "Axe of Despair") to (1, monster, you controls)
```

---

## Target Expressions

DuelScript targets are fully typed and engine-validated.

```ds
self                                          // This card
(1, monster)                                  // 1 monster (any controller)
(1, monster, you controls)                    // 1 monster you control
(1, monster, opponent controls)               // 1 monster opponent controls
(1, monster, either_player controls)          // 1 monster either player
(2+, monster, you controls)                   // 2 or more monsters
(1, "Blue-Eyes" monster, you controls)        // archetype target
(1, card, opponent controls, face_up)         // with qualifier
(1, monster, you controls, with_atk >= 2000)  // with stat qualifier
(1, monster, opponent controls, other_than_self) // excluding self
```

### Target Qualifiers

| Qualifier                  | Meaning                            |
|----------------------------|------------------------------------|
| `face_up`                  | Must be face-up                    |
| `face_down`                | Must be face-down                  |
| `in_attack_position`       | In attack position                 |
| `in_defense_position`      | In defense position                |
| `other_than_self`          | Excludes the activating card       |
| `with_atk OP N`            | ATK comparison                     |
| `with_def OP N`            | DEF comparison                     |
| `with_level OP N`          | Level comparison                   |
| `of_attribute: FIRE`       | Specific attribute                 |
| `of_race: Dragon`          | Specific race                      |
| `of_archetype: "Monarch"`  | Archetype member                   |
| `that_was_normal_summoned` | Was normal summoned this turn      |
| `that_was_special_summoned`| Was special summoned               |
| `with_counter: "spell_counter"` | Has this counter type         |

---

## Zones

| Keyword               | Zone                        |
|-----------------------|-----------------------------|
| `hand`                | Hand                        |
| `field`               | Field (any zone)            |
| `graveyard` / `gy`    | Graveyard                   |
| `banished` / `exile`  | Banished zone               |
| `deck`                | Main deck                   |
| `extra_deck`          | Extra deck                  |
| `monster_zone`        | Monster zone specifically   |
| `spell_trap_zone`     | Spell/Trap zone             |
| `extra_monster_zone`  | Extra Monster Zone          |
| `top_of_deck`         | Top card of deck            |
| `bottom_of_deck`      | Bottom of deck              |

---

## Durations

Used with modifier actions to declare how long they last.

| Keyword                      | Duration                          |
|------------------------------|-----------------------------------|
| `until_end_of_turn`          | Until end of current turn         |
| `until_end_phase`            | Until end phase of current turn   |
| `until_end_of_damage_step`   | Until damage step ends            |
| `until_next_turn`            | Until the start of next turn      |
| `this_turn`                  | Synonym for until_end_of_turn     |
| `permanently`                | Permanent (no expiry)             |

---

## Restrictions

Declare what a card or its controller cannot do.

```ds
restriction {
  cannot: be_targeted by card_effects
  cannot: be_destroyed by battle
  cannot: be_destroyed by card_effects
  cannot: be_negated
  cannot: be_banished
  cannot: attack_directly
  must:   attack_if_able
  limit:  attacks_per_turn: 2
}
```

### Restriction Scopes

| Scope                   | Applies to                     |
|-------------------------|--------------------------------|
| `battle`                | Battle destruction only        |
| `card_effects`          | Any card effect                |
| `spell_effects`         | Spell card effects             |
| `trap_effects`          | Trap card effects              |
| `monster_effects`       | Monster effects                |
| `opponent_card_effects` | Only opponent's card effects   |
| `your_card_effects`     | Only your own card effects     |
| `any`                   | All sources                    |

---

## Engine Responsibilities

The following are **not** declared in DuelScript. The engine handles them universally:

| Responsibility               | Engine handles because...                          |
|------------------------------|----------------------------------------------------|
| Damage step activation legality | Applies universally based on card type and speed |
| SEGOC ordering               | Universal rule — controller of turn resolves first |
| Chain building legality      | Spell speed comparison is universal               |
| "Missing the timing"         | Derived from `timing: when` and game state        |
| Hand size enforcement        | Universal end phase rule                          |
| Once-per-turn tracking       | Engine tracks per card instance, not per definition|
| Spell/Trap destruction on activation | Universal rule                           |
| Battle damage calculation    | Universal formula — engine computes               |

---

## Implementation Guide

### For Rust Engines

```rust
use duelscript::{CardDatabase, DuelScriptEngine, GameEvent};

// Load all cards at startup
let db = CardDatabase::load_from_dir(Path::new("cards/"));
db.print_load_summary();

// Look up a card
let ash = db.get("Ash Blossom & Joyous Spring").unwrap();

// Your engine implements the bridge trait
impl DuelScriptEngine for MyEngine {
    type Context = MyGameState;

    fn check_trigger(&self, trigger: &TriggerExpr, event: &GameEvent, ctx: &MyGameState) -> bool {
        duelscript::engine::trigger_matches(trigger, event)
    }

    fn execute_action(&mut self, action: &GameAction, card: &Card, ctx: &mut MyGameState) {
        match action {
            GameAction::Draw { count } => ctx.draw(*count),
            GameAction::Negate { what, and_destroy } => ctx.negate_current_chain_link(*and_destroy),
            GameAction::Destroy { target } => {
                let targets = self.resolve_targets(target, card, ctx);
                for t in targets { ctx.destroy(t); }
            }
            // ... all GameAction variants
        }
    }
    // ... implement remaining methods
}
```

### For Other Languages

1. Implement a parser for `duelscript.pest` — the grammar is the canonical spec
2. Map parsed nodes to your language's type system (see `ast.rs` as reference)
3. Implement the engine bridge pattern in your language
4. Run the validator rules against parsed cards before registering them

### Conformance

A conforming DuelScript implementation must:

- Parse all constructs defined in `duelscript.pest`
- Reject cards that fail validation (errors, not warnings)
- Map all keyword semantics as defined in this document
- Not require additional per-card scripting for standard mechanics

---

## Changelog

| Version | Changes |
|---------|---------|
| v0.1 | Initial grammar — basic effects, zones, actions |
| v0.2 | Continuous, replacement, equip effects. Counter systems. Win conditions. Pendulum. Link arrows. |
| v0.3 | Summon conditions. Full tribute in all contexts. Validator. CLI tooling. |
| v0.4 | CardDatabase. Engine bridge trait. Language reference spec (this document). |

---

*DuelScript is an open specification. Yu-Gi-Oh is a trademark of Konami.*
*This is a fan project and is not affiliated with or endorsed by Konami.*
