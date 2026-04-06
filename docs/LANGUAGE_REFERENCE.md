# DuelScript Language Reference
## Version 0.5 — Canonical Specification

---

> **DuelScript** is a domain-specific language for defining Yu-Gi-Oh card mechanics.
> It replaces Lua card scripting with a readable, declarative format that compiles
> to engine-compatible bitfields (EDOPro/YGOPro compatible).
>
> **Files use the `c<ID>.ds` naming convention** (e.g., `c55144522.ds` for Pot of Greed).

---

## Table of Contents

1. [Quick Start](#quick-start)
2. [Card Structure](#card-structure)
3. [Card Fields](#card-fields)
4. [Types, Attributes & Races](#types-attributes--races)
5. [Archetypes](#archetypes)
6. [Summon Conditions](#summon-conditions)
7. [Materials](#materials)
8. [Link Arrows](#link-arrows)
9. [Effect Blocks](#effect-blocks)
10. [Speed & Frequency](#speed--frequency)
11. [Timing (When vs If)](#timing-when-vs-if)
12. [Conditions](#conditions)
13. [Triggers](#triggers)
14. [Costs](#costs)
15. [Game Actions](#game-actions)
16. [Expressions](#expressions)
17. [Target Expressions](#target-expressions)
18. [Continuous Effects](#continuous-effects)
19. [Replacement Effects](#replacement-effects)
20. [Equip Effects](#equip-effects)
21. [Advanced Actions](#advanced-actions)
22. [Restrictions](#restrictions)
23. [Counter Systems](#counter-systems)
24. [Win Conditions](#win-conditions)
25. [Zones](#zones)
26. [Python Module](#python-module)
27. [Engine Integration](#engine-integration)

---

## Quick Start

```
// c55144522.ds — Pot of Greed
card "Pot of Greed" {
    type: Normal Spell
    password: 55144522

    effect "Draw 2" {
        speed: spell_speed_1
        on_resolve {
            draw 2
        }
    }
}
```

```
// c14558127.ds — Ash Blossom & Joyous Spring
card "Ash Blossom & Joyous Spring" {
    type: Effect Monster | Tuner
    attribute: FIRE
    race: Zombie
    level: 3
    atk: 0
    def: 1800
    password: 14558127

    effect "Negate Deck Interaction" {
        speed: spell_speed_2
        once_per_turn: hard
        optional: true
        condition: chain_link_includes [add_to_hand, special_summon, send_to_gy, draw]
        trigger: opponent_activates [search | special_summon | send_to_gy | draw]
        cost {
            discard self
        }
        on_resolve {
            negate effect
        }
    }
}
```

---

## Card Structure

Cards are defined with the `card` keyword. All blocks can appear in any order.

```
card "Card Name" {
    // Card fields (type, attribute, stats, etc.)
    // Archetype declaration
    // Summon conditions
    // Materials block
    // Link arrows
    // Effect blocks
    // Continuous effect blocks
    // Replacement effect blocks
    // Equip effect blocks
    // Counter system
    // Win condition
}
```

---

## Card Fields

| Field | Syntax | Required |
|-------|--------|----------|
| Type | `type: Normal Spell` | Yes |
| Attribute | `attribute: DARK` | Monsters only |
| ATK | `atk: 2500` or `atk: ?` | Monsters only |
| DEF | `def: 2000` or `def: ?` | Non-Link monsters |
| Race | `race: Dragon` | Monsters only |
| Level | `level: 4` | Non-Xyz, non-Link monsters |
| Rank | `rank: 4` | Xyz monsters |
| Link | `link: 3` | Link monsters |
| Scale | `scale: 4` | Pendulum monsters |
| Password | `password: 55144522` | Recommended (card ID) |
| Flavor | `flavor: "Description text"` | Optional |

---

## Types, Attributes & Races

### Card Types
```
Normal Monster | Effect Monster | Ritual Monster | Fusion Monster
Synchro Monster | Xyz Monster | Link Monster | Pendulum Monster
Tuner | Synchro Tuner | Gemini | Union | Spirit | Flip | Toon
Normal Spell | Quick-Play Spell | Continuous Spell | Equip Spell
Field Spell | Ritual Spell
Normal Trap | Counter Trap | Continuous Trap
```

Multiple types: `type: Effect Monster | Tuner`

### Attributes
`LIGHT | DARK | FIRE | WATER | EARTH | WIND | DIVINE`

### Races
```
Dragon | Spellcaster | Zombie | Warrior | Beast-Warrior | Beast
Winged Beast | Fiend | Fairy | Insect | Dinosaur | Reptile
Fish | Sea Serpent | Aqua | Pyro | Thunder | Rock | Plant
Machine | Psychic | Divine-Beast | Wyrm | Cyberse
```

---

## Archetypes

```
archetype: ["Blue-Eyes", "Eyes of Blue"]
```

---

## Summon Conditions

```
summon_condition {
    cannot_normal_summon: true
    must_be_summoned_by: own_effect
    tributes_required: 2
    summon_once_per_turn: true
    special_summon_from: [hand, gy]
}
```

---

## Materials

```
// Xyz: 2 Level 4 monsters
materials {
    require: 2 monster level 4
    same_level: true
    method: xyz
}

// Synchro: 1 Tuner + 1+ non-Tuners
materials {
    require: 1 tuner monster
    require: 1+ non-tuner monster
    method: synchro
}

// Link: 2+ Effect Monsters
materials {
    require: 2+ effect monster
    method: link
}

// Named materials
materials {
    require: "Blue-Eyes White Dragon" + "Blue-Eyes White Dragon" + "Blue-Eyes White Dragon"
    method: fusion
}
```

### Material Qualifiers
`tuner | non-tuner | non-token | non-special | non-fusion | non-synchro | non-xyz | non-link`

### Material Constraints
```
same_level: true
same_attribute: true
same_race: true
must_include: "Specific Card Name"
cannot_use: token
method: xyz | synchro | link | fusion | ritual
```

### Alternative Materials
```
alternative {
    require: 3+ non-tuner synchro monster
}
```

---

## Link Arrows

```
link_arrows: [top, bottom_left, bottom_right]
```

Options: `top_left | top | top_right | left | right | bottom_left | bottom | bottom_right`

---

## Effect Blocks

```
effect "Effect Name" {
    speed: spell_speed_2
    once_per_turn: hard
    optional: true
    timing: if

    condition: chain_link_includes [search, draw]
    trigger: opponent_activates [search | draw]

    cost {
        discard self
    }

    on_activate {
        search (1, monster) from deck
    }

    on_resolve {
        negate effect
    }

    restriction {
        cannot: special_summon
    }
}
```

All clauses are optional and can appear in any order.

---

## Speed & Frequency

### Spell Speed
```
speed: spell_speed_1    // Spells, Ignition, Trigger effects
speed: spell_speed_2    // Quick Effects, Traps, Quick-Play Spells
speed: spell_speed_3    // Counter Traps
```

### Frequency
```
once_per_turn: hard     // Cannot re-activate even if negated
once_per_turn: soft     // Can re-activate if negated
once_per_turn           // Defaults to hard
twice_per_turn
once_per_duel
each_turn
```

---

## Timing (When vs If)

**Only applies to optional trigger effects (Spell Speed 1).**

```
timing: when    // Can miss the timing (strict)
timing: if      // Cannot miss the timing (lenient)
```

- `when` — If anything happens between the trigger event and chain building, this effect misses its activation window.
- `if` — The engine always offers this effect during SEGOC, regardless of intervening events.
- **Quick Effects (Speed 2/3) do NOT use when/if timing** — they have their own activation windows managed by the engine's priority system.

The validator warns if an optional trigger effect doesn't explicitly declare timing.

---

## Conditions

```
condition: on_field
condition: in_hand
condition: in_gy
condition: in_banished

condition: you_control_no_monsters
condition: opponent_controls_no_monsters
condition: field_is_empty
condition: you_control (1+, "Blue-Eyes" monster)
condition: opponent_controls (1, spell)

condition: your_lp >= 1000
condition: opponent_lp < 5000
condition: hand_size >= 5
condition: cards_in_gy < 3
condition: banished_count >= 2

// Chain-aware (for hand traps like Ash Blossom)
condition: chain_link_includes [search, special_summon, send_to_gy, draw]

// Composite
condition: on_field and your_lp >= 2000
condition: in_hand or in_gy
```

---

## Triggers

```
trigger: when_summoned
trigger: when_summoned by_special_summon
trigger: when_destroyed
trigger: when_destroyed by battle
trigger: when_destroyed by card_effect
trigger: when_sent_to gy
trigger: when_sent_to gy by card_effect
trigger: when_flipped
trigger: when_attacked                // This card is attacked (SINGLE)
trigger: when attack_declared          // Any monster declares an attack (FIELD)
trigger: when_tributed
trigger: when_tribute_summoned
trigger: on_nth_summon: 5

trigger: during_standby_phase
trigger: during_standby_phase of yours
trigger: during_end_phase
trigger: during battle_phase

trigger: opponent_activates [search | special_summon | send_to_gy | draw]
```

---

## Costs

```
cost {
    pay_lp 1000
    pay_lp your_lp / 2              // Dynamic expression
    discard self
    discard (1, card, you controls, hand)
    tribute self
    tribute (1, monster, you controls)
    banish self
    banish self from gy
    send self to gy
    detach 1 overlay_unit from self
    remove_counter 3 "Spell Counter" from self
    reveal self
    none
}
```

---

## Game Actions

### Card Movement
```
draw 2
draw count((1+, monster, you controls, gy)) // Dynamic
special_summon self from gy in attack_position
special_summon (1, monster) from gy
destroy (1+, monster, either_player controls)
send self to gy
banish (1, card, opponent controls) face_down
return self to hand
return (1, card) to deck shuffle
search (1, "Blue-Eyes" monster) from deck
add_to_hand (1, card) from gy
mill 3
mill 2 from opponent_deck
discard (1, card)
tribute self
shuffle deck
```

### Stat Modification
```
modifier: atk +500 on (1, monster, you controls) until_end_of_turn
modifier: atk + count((1+, monster, you controls, gy)) * 300
modifier: def -200
set_atk (1, monster) to 0
double_atk self
halve_atk (1, monster, opponent controls)
```

### Negation
```
negate effect                  // CATEGORY_DISABLE (Ash Blossom)
negate activation              // CATEGORY_NEGATE (Solemn Warning)
negate activation and destroy  // NEGATE + DESTROY (Solemn Judgment)
negate summon                  // CATEGORY_DISABLE_SUMMON
negate summon and destroy      // DISABLE_SUMMON + DESTROY
negate attack                  // No category (Utopia)
```

### Control & Position
```
take_control of (1, monster, opponent controls)
take_control of (1, monster) until end_phase
change_battle_position (1, monster)
set (1, card)
flip_face_down (1, monster)
```

### Xyz Operations
```
detach 1 overlay_unit from self
attach (1, monster) to self as_material
```

### Counters
```
place_counter 3 "Spell Counter" on self
remove_counter 2 "Spell Counter" from self
```

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

### Damage & LP
```
deal_damage to opponent: 1000
deal_damage to opponent: self.atk
deal_damage to both_players: 500
gain_lp: 1000
gain_lp: count((1+, monster, you controls, gy)) * 500
```

### Summoning
```
fusion_summon (1, "Card Name") using monster + monster
synchro_summon (1, synchro monster) using (1, tuner) + (1, non-tuner)
xyz_summon (1, xyz monster) using monster + monster
ritual_summon (1, ritual monster) using monster
pendulum_summon (1+, monster) from [hand, extra_deck_face_up]
```

---

## Expressions

Dynamic values can be used anywhere a number is expected.

```
// Literals
draw 2
pay_lp 1000

// Card stats
deal_damage to opponent: self.atk
modifier: atk + target.def

// Player LP
pay_lp your_lp / 2

// Counting
draw count((1+, monster, you controls, gy))
modifier: atk + count((1+, "Dragon" monster, you controls, monster_zone)) * 300

// Arithmetic
deal_damage to opponent: self.level * 200
modifier: atk + (count((1+, card, you controls, banished)) + 1) * 100
```

---

## Target Expressions

```
self                                          // This card
(1, monster)                                  // 1 monster (any)
(1+, card, opponent controls)                 // 1 or more cards opponent controls
(2, monster, you controls, monster_zone)      // 2 of your monsters in monster zone
(1, "Blue-Eyes" monster, you controls, gy)    // 1 Blue-Eyes in your GY

// Qualifiers
(1, monster, you controls, face_up)
(1, monster, opponent controls, with_atk >= 2000)
(1, monster, you controls, of_attribute: DARK)
(1, monster, either_player controls, of_race: Dragon)
(1, card, you controls, other_than_self)
```

---

## Continuous Effects

```
continuous_effect "ATK Boost" {
    while: on_field
    apply_to: (1+, "Dragon" monster, you controls, monster_zone)
    modifier: atk +300
    modifier: def +300
}

// Dynamic modifier with expression
continuous_effect "Linked Boost" {
    while: on_field
    modifier: atk + count((1+, monster, either_player controls, monster_zone)) * 500
}

// Granting abilities
continuous_effect "Protection" {
    while: on_field
    apply_to: self
    grant: cannot_be_destroyed_by_battle
    grant: immune_to_targeting
}
```

### Granted Abilities
```
piercing | double_attack | direct_attack
cannot_be_destroyed_by_battle | cannot_be_destroyed_by_effect
unaffected_by_spell_effects | unaffected_by_trap_effects
unaffected_by_monster_effects | unaffected_by_card_effects
immune_to_targeting | cannot_activate_effects
```

---

## Replacement Effects

```
replacement_effect "Indestructible" {
    instead_of: destroyed_by_battle
    do: {
        detach 1 overlay_unit from self
    }
}
```

### Replaceable Events
```
destroyed_by_battle | destroyed_by_effect | destroyed_by_any
sent_to_gy | sent_to_gy_by_effect | sent_to_gy_by_battle
banished | returned_to_hand | returned_to_deck
```

---

## Equip Effects

```
equip_effect {
    target: (1, monster, you controls, monster_zone, face_up)

    while_equipped {
        modifier: atk + count((1+, monster, you controls, monster_zone, face_up)) * 800
        modifier: def + count((1+, monster, you controls, monster_zone, face_up)) * 800
    }

    on_equipped_destroyed {
        destroy self
    }

    on_unequipped {
        send self to gy
    }
}
```

---

## Advanced Actions

### Player Choice
```
on_resolve {
    choose {
        option "Take control" {
            take_control of (1, monster, opponent controls)
        }
        option "Revive from GY" {
            special_summon (1, monster) from gy
        }
    }
}
```

### Delayed Effects
```
on_resolve {
    delayed until end_phase {
        destroy self
    }
}
```

### Dynamic Effect Registration
```
on_resolve {
    register_effect on (1, monster, opponent controls) {
        grant: cannot_activate_effects
        duration: until_end_of_turn
    }
}
```

### State Persistence
```
on_activate {
    store "chosen" = selected_targets
}
on_resolve {
    recall "chosen"
}
```

### Conditional Logic
```
on_resolve {
    if (your_lp <= 2000) {
        draw 2
    } else {
        draw 1
    }
}
```

### Iteration
```
on_resolve {
    for_each (1+, monster, you controls) in monster_zone {
        modifier: atk +500
    }
}
```

---

## Restrictions

```
restriction {
    cannot: be_targeted by card_effects
    cannot: be_destroyed by battle
    cannot: attack_directly
    cannot: special_summon
    must: attack_if_able
    limit: attacks_per_turn: 2
    limit: special_summons_per_turn: 1
}
```

---

## Counter Systems

```
counter_system {
    name: "Spell Counter"
    placed_when: when activate_spell
    max: 6

    effect "Remove counters" {
        speed: spell_speed_1
        cost {
            remove_counter 3 "Spell Counter" from self
        }
        on_resolve {
            destroy (1, card, opponent controls)
        }
    }
}
```

---

## Win Conditions

```
win_condition {
    when: all_pieces_in_hand
    result: win_duel
}

win_condition {
    when: turn_count >= 20
    result: win_duel
}
```

---

## Zones

```
hand | field | deck | extra_deck | extra_deck_face_up
graveyard | gy | banished | exile
monster_zone | spell_trap_zone | extra_monster_zone
field_zone | pendulum_zone
top_of_deck | bottom_of_deck
```

---

## Python Module

Install: `maturin develop --features python`

```python
import duelscript

# Parse
cards = duelscript.parse_file("c55144522.ds")
card = cards[0]
print(card.name, card.card_types, card.atk)

# Validate
errors = duelscript.validate(source_string)
for e in errors:
    print(e)  # [ERROR] Card Name: message

# Compile to engine bitfields
compiled = duelscript.compile(source_string)
for cc in compiled:
    for eff in cc.effects:
        print(eff.type_name(), hex(eff.category))

# Load card database
db = duelscript.CardDB("cards/official")
card = db.get_by_id(55144522)
results = db.search("dragon")

# Constants
duelscript.CATEGORY_DRAW        # 0x10000
duelscript.EFFECT_TYPE_QUICK_O  # 0x100
```

---

## Engine Integration

DuelScript compiles to the same u32 bitfields used by EDOPro/YGOPro:

| DuelScript Field | Engine Equivalent |
|------------------|-------------------|
| `effect_type` | `EFFECT_TYPE_*` flags |
| `category` | `CATEGORY_*` flags |
| `code` | `EVENT_*` codes |
| `property` | `EFFECT_FLAG_*` flags |
| `range` | `LOCATION_*` flags |
| `count_limit` | `(count, code)` — 0=soft OPT, card_id=hard OPT |

See [ENGINE_INTEGRATION.md](ENGINE_INTEGRATION.md) for the full integration guide.
