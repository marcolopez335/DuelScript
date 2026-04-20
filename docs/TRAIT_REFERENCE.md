# DuelScriptRuntime Trait Reference

`DuelScriptRuntime` is the engine-facing abstraction that bridges compiled DuelScript
closures and a concrete duel engine. Every `.ds` card compiles into one or more Rust
closures that receive a `&mut dyn DuelScriptRuntime` (or `&dyn DuelScriptRuntime` for
pure queries); those closures call the methods described in this document to read game
state, move cards, manipulate life points, prompt players for choices, and fire
higher-level events. The trait lives in `src/v2/runtime.rs`. The only current
implementor used in tests is `MockRuntime` in `src/v2/mock_runtime.rs`. Production
engines (EDOPro, ygobeetle, or any other host) implement the trait and link against
the compiled closures.

---

## Conventions

**Player index.** All methods accept `player: u8` where `0` is the activating
controller and `1` is the opponent. This convention is consistent across every method.
Scripts written in DuelScript do not hard-code player indices — they use the built-in
`controller` and `opponent` keywords that the compiler maps to `effect_player()` and
its complement.

**Card IDs.** Cards are identified by a `u32` runtime ID. For real engines this is
typically the EDOPro card code (passcode). In `MockRuntime` it is a caller-assigned
integer. Where a method returns `0` as a sentinel it means "unknown" or "not
applicable", not a valid card ID. The distinction matters for methods like
`get_card_code` (which may legitimately return `0` for tokens) versus stat queries
(which return `0` to signal "card not found").

**Location bitmasks.** Zone arguments follow EDOPro's location bitmask convention:
`0x1` = DECK, `0x2` = HAND, `0x4` = MZONE (monster zone),
`0x8` = SZONE (spell/trap zone), `0x10` = GRAVE, `0x20` = REMOVED (banished).
A value of `0` for location typically defaults to the on-field union (MZONE | SZONE).
Multiple zones can be combined with bitwise OR; implementations must honor every set
bit.

**Return values on failure.** Movement methods return the count of cards actually
moved, which may be less than requested if cards were not present in the expected zone
at resolution time. Boolean mutators return `false` when the engine blocks or negates
the operation, not when a script-level precondition fails (that is enforced by the
compiled condition closure before any action method is called). Stat and counter
queries return `0` for unknown cards rather than an error.

**Must-override.** Methods marked **(Required)** below have no default implementation
or have a default that would silently do nothing useful (e.g., returning `0` when the
engine needs a real value). Any engine adapter must provide real implementations for
all Required methods or compiled card effects will behave incorrectly. Methods without
**(Required)** have a documented default behaviour that is intentionally permissive —
they are optional features that engines may skip when the feature is not relevant to
their use case.

**Phase / event bitmasks.** Several methods accept a `duration_mask` or
`survives_mask` / `resets_mask`. These are engine-defined bitmasks describing game
phases or events (e.g., "end of this turn", "start of next turn", "on resolution").
The exact bit values are defined by the host engine; the trait does not mandate
specific constants.

---

## Queries

The query methods read game state without mutating it (they take `&self`). They cover
life points, zone populations, card properties (stats, race, attribute, type, name,
archetypes), and the effect-context identifiers that tell a closure which card is
currently resolving and who activated it. Because these are reads only, they are safe
to call from both predicate (condition) closures and action (resolve) closures.

- `get_lp(player) -> i32` — Current life points of `player`. The value may be
  negative when the engine has not yet processed a lethal-damage win condition at the
  time the closure runs. **(Required)**

- `get_hand_count(player) -> usize` — Number of cards currently in `player`'s hand.
  Returns `0` if the hand is empty. **(Required)**

- `get_deck_count(player) -> usize` — Number of cards currently in `player`'s Main
  Deck. Returns `0` if the deck is empty. **(Required)**

- `get_gy_count(player) -> usize` — Number of cards in `player`'s Graveyard.
  **(Required)**

- `get_banished_count(player) -> usize` — Number of cards currently banished for
  `player`. **(Required)**

- `get_field_card_count(player, location) -> usize` — Count of cards in the zone(s)
  described by the `location` bitmask for `player`. **(Required)**

- `get_field_cards(player, location) -> Vec<u32>` — Card IDs in the zone(s) described
  by the `location` bitmask. When `location` is `0`, returns the on-field union of
  MZONE and SZONE. **(Required)**

- `card_matches_filter(card_id, filter) -> bool` — Whether `card_id` satisfies the
  given `CardFilter` predicate (type, archetype, name, or token status). Returns
  `false` for unknown cards. **(Required)**

- `get_card_stat(card_id, stat) -> i32` — Numeric stat of `card_id` for the `Stat`
  variant. `Atk`/`Def` are the current (modified) values; `BaseAtk`/`BaseDef` are
  pre-modification values; `OriginalAtk`/`OriginalDef` are the printed card values.
  Returns `0` for unknown cards. **(Required)**

- `get_card_race(card_id) -> u64` — EDOPro race bitmask for `card_id`
  (e.g., `RACE_WARRIOR = 0x1`). Returns `0` for unknown cards or non-monsters.
  Default: `0`.

- `get_card_attribute(card_id) -> u64` — EDOPro attribute bitmask for `card_id`
  (e.g., `ATTRIBUTE_DARK = 0x2`). Returns `0` for unknown cards or non-monsters.
  Default: `0`.

- `get_card_type(card_id) -> u64` — EDOPro type bitmask for `card_id`
  (e.g., `TYPE_MONSTER | TYPE_EFFECT`). Returns `0` for unknown cards. Default: `0`.

- `get_card_code(card_id) -> u32` — Canonical passcode for `card_id`. For most cards
  this equals the ID itself; may differ after `change_card_code` has been applied.
  Default: returns `card_id` unchanged.

- `get_card_name(card_id) -> String` — Display name of `card_id`. Returns an empty
  string for unknown cards. Default: `String::new()`.

- `get_card_archetypes(card_id) -> Vec<String>` — Archetype tags the card belongs to
  (e.g. `["Blue-Eyes", "Dragon"]`). Returns an empty `Vec` for unknown or untagged
  cards. Default: empty `Vec`.

- `effect_card_id() -> u32` — Card ID of the card whose effect is currently
  resolving. Used by compiled closures to self-reference without hard-coding a
  passcode. **(Required)**

- `effect_player() -> u8` — Player index (0 or 1) who activated the currently
  resolving effect. **(Required)**

- `event_categories() -> u32` — EDOPro `CATEGORY_*` bitmask for the chain link
  currently being checked. Used by hand-trap conditions such as
  `chain_link_includes`. **(Required)**

---

## Movement / Actions

These mutating methods move cards between zones. All movement methods return the count
of cards successfully moved; returning fewer than requested is normal and correct
behaviour when some cards are no longer in the expected zone at resolution time (e.g.,
the card was already destroyed in a chain). Scripts should not treat a partial move as
an error — the engine resolves effects to the greatest possible extent.

- `draw(player, count) -> u32` — Draw `count` cards from the top of `player`'s deck
  into their hand. Returns the number of cards actually drawn; may be less than
  `count` if the deck runs out. **(Required)**

- `destroy(card_ids) -> u32` — Destroy each card in `card_ids` by game effect and
  send them to the Graveyard. Cards already in the Graveyard or banished zone are not
  affected. Returns the number of cards destroyed. **(Required)**

- `send_to_grave(card_ids) -> u32` — Send cards directly to the Graveyard without
  treating the movement as destruction. Used for effects such as "send to GY" that
  bypass destruction immunity. Returns cards moved. **(Required)**

- `send_to_hand(card_ids) -> u32` — Send cards to their controller's hand. Used for
  "add to hand" effects where the origin is not the field. Returns cards moved.
  **(Required)**

- `banish(card_ids) -> u32` — Remove `card_ids` from play (banish face-up). Returns
  cards banished. **(Required)**

- `discard(card_ids) -> u32` — Discard `card_ids` from the hand to the Graveyard.
  Differs from `send_to_grave` in that the origin is always the hand; cards not in
  the hand are silently skipped. Returns cards discarded. **(Required)**

- `special_summon(card_id, player, position) -> bool` — Special Summon `card_id`
  onto `player`'s field in `position` (EDOPro `POS_*` constant, e.g.,
  `POS_FACEUP_ATTACK = 0x1`). Returns `false` if the summon was blocked or the zone
  was full. **(Required)**

- `send_to_deck(card_ids, top) -> u32` — Send `card_ids` to the deck. If `top` is
  `true`, cards go to the top; if `false`, cards go to the bottom. Returns cards
  moved. **(Required)**

- `return_to_hand(card_ids) -> u32` — Bounce cards from the field back to their
  controller's hand. Used for the "return to hand" game action distinct from "add to
  hand" (`send_to_hand`). Returns cards moved. **(Required)**

- `return_to_owner(card_ids) -> u32` — Return cards to their original owner's side
  when control has changed. Returns cards moved. Default: `0` (no-op — engines that
  do not support control-change may omit this).

- `tribute(card_ids) -> u32` — Tribute `card_ids` from the field as a cost. Returns
  cards tributed. **(Required)**

- `shuffle_deck(player)` — Shuffle `player`'s Main Deck in place. **(Required)**

- `mill(player, count) -> u32` — Send `count` cards from the top of `player`'s deck
  to the Graveyard (mill). Returns actual cards milled. Default: `0`.

- `excavate(player, count) -> Vec<u32>` — Reveal the top `count` cards of `player`'s
  deck without sending them anywhere. Returns card IDs in deck order (top card last).
  Cards remain in the deck unless the closure explicitly moves them afterward.
  Default: empty `Vec`.

---

## LP

Two symmetric methods for adjusting life points. Both return `false` when the engine
negates the operation (for example, if a continuous effect prevents LP changes). The
`amount` parameter is always a positive integer; the method name conveys the
direction.

- `damage(player, amount, damage_type) -> bool` — Inflict `amount` LP damage to
  `player`. `damage_type` is a `DamageType` enum variant (`Effect`, `Cost`, or
  `Battle`) that engines map to the corresponding `REASON_*` bit for triggers,
  negation filters, and event logging. Returns `false` if the damage was negated
  or redirected. **(Required)**

  ```rust
  pub enum DamageType { Effect, Cost, Battle }
  ```

  The compiler emits `Cost` for `PayLp` call sites (cost-phase LP payments) and
  `Effect` for direct `Action::Damage` / `Action::PayLp` effect sites. `Battle`
  is never emitted from DSL — battle damage is engine-internal — but is present
  in the enum so engine-side consumers can share one type.

- `recover(player, amount) -> bool` — Restore `amount` LP to `player`. Returns
  `false` if the recovery was negated. **(Required)**

---

## Selection

These methods present choices to a human player (or AI) and block until a selection
is made. In automated and test contexts the implementation typically chooses
deterministically — `MockRuntime` always picks the first `min` candidates, and always
returns index `0` for option selection. Real engines display UI and wait for input.

- `select_cards(player, candidates, min, max) -> Vec<u32>` — Prompt `player` to
  choose between `min` and `max` cards from `candidates`. The returned `Vec` has
  length in `[min, max]`. **(Required)**

- `select_option(player, options) -> usize` — Prompt `player` to pick one option from
  `options` (human-readable labels). Returns a zero-based index. **(Required)**

---

## Effect Metadata

These two methods are called near the top of an effect closure to register the
operation category and targets with the engine before any action methods are called.
The engine uses this information to determine which hand-trap cards may respond and to
construct accurate chain-link records.

- `set_targets(card_ids)` — Register the cards currently targeted by the effect. Also
  updates the "last selection" snapshot used by `bind_last_selection`. **(Required)**

---

## Negation

Two distinct negation levels that map to the Yu-Gi-Oh rules distinction between
negating the activation of a card (card is sent to the Graveyard, effect does not
resolve) versus negating only the effect (card remains on the field, effect is
prevented but activation stands).

- `negate_activation() -> bool` — Negate the activation of the current chain-link.
  The card and its effect are both negated; the card is typically sent to the
  Graveyard. Returns `false` if the negation itself could not be applied.
  **(Required)**

- `negate_effect() -> bool` — Negate only the effect; the card that was activated
  remains on the field in its destination zone. Returns `false` if inapplicable.
  **(Required)**

---

## Stats

Four methods for modifying a card's current ATK or DEF during resolution. The `modify_*`
variants apply a relative delta (positive or negative); the `set_*` variants assign an
absolute value. Neither affects the card's Base or Original stats — those are
read-only properties of the card definition accessible via `get_card_stat`.

- `modify_atk(card_id, delta)` — Adjust `card_id`'s current ATK by `delta`. A
  negative `delta` reduces ATK. **(Required)**

- `modify_def(card_id, delta)` — Adjust `card_id`'s current DEF by `delta`.
  **(Required)**

- `set_atk(card_id, value)` — Set `card_id`'s current ATK to an absolute `value`.
  **(Required)**

- `set_def(card_id, value)` — Set `card_id`'s current DEF to an absolute `value`.
  **(Required)**

---

## Battle

Methods that apply specifically during the Battle Phase. Both are meaningful only when
a battle-phase trigger or hand-trap effect is resolving.

- `change_position(card_id)` — Toggle `card_id` between Attack Position and Defense
  Position. **(Required)**

---

## Xyz Materials

Xyz monsters carry overlay units (materials) that can be detached as costs or
attached from other sources. Detached materials go to the Graveyard unless the card
effect says otherwise.

- `detach_material(card_id, count) -> u32` — Detach `count` Xyz Materials from
  `card_id` and send them to the Graveyard. Returns the number of materials actually
  detached, which may be less than `count` if fewer are attached. **(Required)**

- `attach_material(material_id, target_id)` — Attach `material_id` as an Xyz
  Material overlay unit to `target_id`. **(Required)**

---

## Counters

Counter names are arbitrary strings chosen by the card script (e.g.,
`"Spell Counter"`, `"Bushido Counter"`, `"Predator Counter"`). The engine stores
them as `(card_id, counter_name) -> count` pairs. Counts accumulate with
`place_counter` and are reduced (flooring at zero) by `remove_counter`.

- `place_counter(card_id, counter_name, count)` — Add `count` counters of type
  `counter_name` to `card_id`. If counters of that type already exist the count
  accumulates. **(Required)**

- `remove_counter(card_id, counter_name, count)` — Remove up to `count` counters of
  type `counter_name` from `card_id`. Saturates at zero and does not underflow.
  **(Required)**

- `has_counter(card_id, counter_name) -> bool` — Whether `card_id` has at least one
  counter of type `counter_name`. Default: `false`.

- `get_counter_count(card_id, counter_name) -> u32` — Number of counters of type
  `counter_name` on `card_id`; `0` if none or if the card is unknown. Default: `0`.

---

## State / History / Bindings

This group covers five related subsystems added incrementally across Phases 1A–1D and
Sprint 67. Together they let compiled closures express effects that depend on a card's
history, maintain cross-step state within one effect resolution, and schedule work for
a later phase.

### Flags

Flags are persistent boolean markers on a card, keyed by `(card_id, name)`. They
survive across chain links and turns according to the `survives_mask` set at
registration, and are automatically cleared by the engine when the `resets_mask` event
fires. Scripts use flags to implement "once per turn" locks, "used this chain"
trackers, and similar stateful guards.

- `register_flag(card_id, name, survives_mask, resets_mask)` — Register the named
  flag on `card_id`. `survives_mask` and `resets_mask` are engine-defined phase/event
  bitmasks. Default: no-op.

- `clear_flag(card_id, name)` — Remove the named flag from `card_id` immediately,
  regardless of its reset mask. Default: no-op.

- `has_flag(card_id, name) -> bool` — Whether the flag is currently set on `card_id`.
  Returns `false` if the flag was never registered or has been cleared. Default: `false`.

### History

History queries give closures access to where a card was before its most recent move,
what position it was in, and why it was sent to a zone. These are read-only and return
`0` when the engine has no history recorded for that card.

- `previous_location(card_id) -> u32` — Location bitmask where `card_id` was before
  its most recent move. `0` means unknown or not yet moved. Default: `0`.

- `previous_position(card_id) -> u32` — Position constant (EDOPro `POS_*`) before the
  card's most recent position change. `0` means unknown. Default: `0`.

- `sent_by_reason(card_id) -> u32` — EDOPro `REASON_*` bitmask for why the card was
  sent to the Graveyard (or other zone). `0` means unknown. Default: `0`.

### Bindings

Named bindings let compiled closures pass a card reference between steps of a
multi-step effect without re-querying the engine. For example, a Targeting step can
bind the target card under `"target"` and the Resolve step can retrieve it by name.
The binding store is scoped to the current effect resolution and is not persisted.

- `set_binding(name, card_id)` — Store `card_id` under the string key `name` for
  later retrieval. Default: no-op.

- `bind_last_selection(name)` — Bind the most recently selected or targeted group of
  cards (from the last `select_cards` or `set_targets` call) under `name`. Default:
  no-op.

- `get_binding_card(name) -> Option<u32>` — The first card ID stored under binding
  `name`. Returns `None` if the binding does not exist. Default: `None`.

- `get_binding_field(name, field) -> i32` — A numeric field of the card stored under
  binding `name`. Supported field names: `"atk"`, `"def"`, `"level"`, `"code"`.
  Returns `0` if the binding is absent or the field is not recognised. Default: `0`.

### Value Store

The value store is an integer scratchpad for persisting numeric snapshots across
effect steps — for example, recording a card's ATK at activation time so the Resolve
step can use it even if the ATK has changed by then.

- `store_value(label, value)` — Persist integer `value` under string `label`. Default:
  no-op.

- `recall_value(label) -> i32` — Retrieve the integer stored under `label`; `0` if
  absent. Default: `0`.

### Delayed Effects

- `register_delayed(phase, card_id)` — Schedule `card_id`'s effect to fire at game
  phase `phase` (engine-defined phase constant, e.g., end-of-turn). Default: no-op.

### Chain / Phase Queries

These read-only queries let closures inspect the current game phase and chain state.
All have permissive defaults so engines that do not track this information compile
without changes.

- `get_current_phase() -> u32` — Current game phase constant; `0` if unknown.
  Default: `0`.

- `chain_includes_category(category) -> bool` — Whether any effect in the current
  chain matches the given `CATEGORY_*` bitmask. Default: `false`.

- `is_face_up(card_id) -> bool` — Whether `card_id` is currently face-up.
  Default: `true`.

- `is_face_down(card_id) -> bool` — Whether `card_id` is currently face-down.
  Default: `false`.

- `is_attack_position(card_id) -> bool` — Whether `card_id` is in Attack Position.
  Default: `true`.

- `is_defense_position(card_id) -> bool` — Whether `card_id` is in Defense Position.
  Default: `false`.

- `has_attacked_this_turn(card_id) -> bool` — Whether `card_id` declared an attack
  during the current Battle Phase. Default: `false`.

- `was_summoned_this_turn(card_id) -> bool` — Whether `card_id` was summoned this
  turn. Default: `false`.

- `was_flipped_this_turn(card_id) -> bool` — Whether `card_id` was flipped face-up
  this turn. Default: `false`.

### Events

- `raise_custom_event(name, cards)` — Fire a named custom game event associated with
  `cards`. The event name and semantics are engine-defined. Default: no-op.

- `confirm_cards(owner, audience, cards)` — Reveal `cards` belonging to `owner` to
  `audience`, typically to demonstrate that a cost or condition was satisfied.
  Default: no-op.

---

## Summoning

The summoning methods cover Normal Summon, Set, and all four Extra Deck summon types.
All return `true` on success and `false` when the summon is blocked by the engine
(zone full, summon condition not met, negated by an opponent's response, etc.). The
default implementations return `true` unconditionally — real engines must enforce zone
availability, rule precedence, and legality checks in their implementations.

- `normal_summon(card_id, player) -> bool` — Perform a Normal Summon of `card_id` to
  `player`'s Monster Zone. Default: `true`.

- `set_card(card_id, player) -> bool` — Set `card_id` face-down on `player`'s field.
  Applies to both monsters (Set in lieu of Normal Summon) and spell/trap cards.
  Default: `true`.

- `ritual_summon(card_id, player, material_ids) -> bool` — Perform a Ritual Summon of
  `card_id` using `material_ids` as the tributed materials. Default: `true`.

- `fusion_summon(card_id, player, material_ids) -> bool` — Perform a Fusion Summon of
  `card_id` using `material_ids` as fusion materials. Default: `true`.

- `synchro_summon(card_id, player, material_ids) -> bool` — Perform a Synchro Summon
  of `card_id` using `material_ids` (Tuner + non-Tuner). Default: `true`.

- `xyz_summon(card_id, player, material_ids) -> bool` — Perform an Xyz Summon of
  `card_id` using `material_ids` as initial overlay units. Default: `true`.

---

## Equip / Swap / Grant

Four methods for attaching equipment, exchanging state between cards, and registering
continuous ability grants. All four default to no-ops so engines that have not
implemented these features compile and run correctly against cards that do not use them.

- `equip_card(equip_id, target_id)` — Equip `equip_id` to `target_id` as an Equip
  Spell, establishing the equip link in the engine's zone management. Default: no-op.

- `swap_control(card_a, card_b)` — Exchange control of `card_a` and `card_b` between
  players (i.e., `card_a` moves to the opponent's side and vice versa). Default:
  no-op.

- `swap_stats(card_id)` — Swap the current ATK and DEF values of `card_id` for the
  duration dictated by the card's effect. Default: no-op.

- `register_grant(card_id, grant_code, duration)` — Register a continuous ability
  grant on `card_id` identified by `grant_code` (engine-defined; e.g.,
  `GRANT_CANNOT_BE_DESTROYED`), lasting for the phase/event mask `duration`.
  Default: no-op.

---

## Card Identity

Methods for temporarily or permanently altering the type identity of a card. These
are used by effects such as "this card is also treated as a Warrior-type" or "change
the attribute of all monsters on the field to DARK". Changes applied via these methods
affect `card_matches_filter` and the race/attribute/type query results but do not
retroactively change `get_card_stat` Original variants.

- `change_level(card_id, level)` — Set `card_id`'s level to `level`. Default: no-op.

- `change_attribute(card_id, attribute)` — Set `card_id`'s attribute to the given
  EDOPro bitmask (e.g., `ATTRIBUTE_DARK = 0x2`). Default: no-op.

- `change_race(card_id, race)` — Set `card_id`'s race to the given EDOPro bitmask
  (e.g., `RACE_WARRIOR = 0x1`). Default: no-op.

- `change_name(card_id, name, duration)` — Rename `card_id` to `name` for the
  duration described by the phase/event bitmask `duration`. Default: no-op.

- `change_card_code(card_id, code, duration_mask)` — Change `card_id`'s passcode to
  `code` for `duration_mask`. Used for "treat this card as X" effects that require
  the passcode to change. Default: no-op.

- `set_scale(card_id, scale)` — Set the Pendulum Scale of `card_id` to `scale`
  (0–13). Default: no-op.

- `take_control(card_id, new_controller)` — Transfer control of `card_id` to
  `new_controller` (0 or 1). The card moves to the new controller's field zone.
  Default: no-op.

- `create_token(player, spec)` — Create `spec.count` Token monsters on
  `player`'s field. `spec` is a `TokenSpec` struct (defined in
  `src/v2/runtime.rs`) carrying the full token definition:

  ```rust
  pub struct TokenSpec {
      pub name: String,       // display name, defaults to "Token"
      pub atk: i32,
      pub def: i32,
      pub level: u32,         // defaults to 1
      pub attribute: u32,     // code from attribute_to_engine (0x10=LIGHT … 0x400=DIVINE)
      pub race: u32,          // bitmask from race_to_engine (e.g. 0x4000=Beast)
      pub position: u32,      // POS_* code (0x1=attack, 0x4=face-up defense, 0x8=face-down defense); default 0x1
      pub count: u32,
  }
  ```

  Compiler emission: the `Action::CreateToken` arm builds a `TokenSpec` from
  the parsed `ast::TokenSpec`, evaluating `StatVal` literals for atk/def,
  defaulting `name`/`level`/`position`, and mapping optional enums through
  `attribute_to_engine` / `race_to_engine` / `position_to_code` (0 when the
  DSL omits attribute or race). The `restriction` field of
  `ast::TokenSpec` is not mirrored on the runtime surface — continuous-effect
  territory, deferred (see `decisions-2.md` entry A-II).

  Signature widened in T17 (`decisions-2.md` B-II). Prior signature
  `(player, atk, def, count)` dropped name/attribute/race/level/position.
  Default: no-op.

---

## RNG

Randomness and public-information methods. In a real engine, `coin_flip` and
`dice_roll` involve a shared RNG seed and are animated for the players; in
`MockRuntime` they are deterministic (coin always heads, die always rolls 1). `reveal`
and `look_at` have no game-state side effect beyond marking cards as known — they are
grouped here because they also have no required return value.

- `coin_flip(player) -> bool` — Flip a coin for `player`. Returns `true` for heads,
  `false` for tails. Default: `true`.

- `dice_roll(player) -> u32` — Roll a six-sided die for `player`. Returns a value in
  the range 1–6 inclusive. Default: `1`.

- `reveal(card_ids)` — Publicly reveal `card_ids` to all players (e.g., from the
  hand to satisfy a spell speed condition). Default: no-op.

- `look_at(player, card_ids)` — Show `card_ids` privately to `player` only (the
  opponent does not see). Default: no-op.

---

## Announce

A two-step protocol for "declare a card name" effects such as Prohibition or mind-read
cards. The caller invokes `announce` to prompt the player and receive an opaque token,
then later invokes `get_announcement` to resolve the token into the declared value
(typically a card passcode). The indirection exists because the UI interaction
(`announce`) may complete asynchronously while the game logic (`get_announcement`) is
synchronous.

- `announce(player, kind, filter_mask) -> u32` — Prompt `player` to announce a card
  name or type. `kind` is an engine-defined category (e.g., `0` = card name, `1` =
  card type). `filter_mask` restricts what may be announced; `0` means unrestricted.
  Returns an opaque token. Default: `0`.

- `get_announcement(token) -> u32` — Retrieve the card code or type value from a
  prior `announce` call identified by `token`. Returns `0` if the token is unknown or
  if the announce has not yet resolved. Default: `0`.
