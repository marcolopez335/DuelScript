// ============================================================
// DuelScript v2 Runtime Trait
// Engine abstraction — no v1 imports.
// ============================================================

// ── Types used by the runtime trait ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardFilter {
    Monster, Spell, Trap, Card, Token,
    NonTokenMonster, TunerMonster, NonTunerMonster,
    NormalMonster, EffectMonster, FusionMonster,
    SynchroMonster, XyzMonster, LinkMonster, RitualMonster,
    ArchetypeMonster(String), ArchetypeCard(String), NamedCard(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stat {
    Atk, Def, Level, Rank, BaseAtk, BaseDef, OriginalAtk, OriginalDef,
}

/// Reason bit for `damage` dispatch. Engines that distinguish damage sources
/// (for reason flags, triggers, negation, etc.) consume this to route the
/// right `REASON_*` bit. Compiler emits `Cost` for PayLp call sites and
/// `Effect` for direct effect-damage call sites. `Battle` is included for
/// completeness though the compiler never emits it — battle damage is
/// engine-internal and never routed from DSL.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum DamageType {
    Effect,
    Cost,
    Battle,
}

// ── Runtime Abstraction ───────────────────────────────────────

/// Trait that engines implement to expose game state and operations
/// to compiled DuelScript closures.
///
/// Intentionally not `Send + Sync` — implementations often use `Rc<RefCell>`.
/// The closures themselves are `Send + Sync` because they receive the runtime
/// as a parameter rather than capturing it.
pub trait DuelScriptRuntime {
    // ── Queries ──────────────────────────────────────────────

    /// Current life points of `player`.
    ///
    /// # Args
    /// - `player`: 0 = controller, 1 = opponent.
    ///
    /// # Returns
    /// Life-point total as a signed integer (can be negative when a player has
    /// taken lethal damage before the engine processes the win condition).
    ///
    /// **Required to override.**
    fn get_lp(&self, player: u8) -> i32;

    /// Number of cards currently in `player`'s hand.
    ///
    /// # Args
    /// - `player`: 0 or 1.
    ///
    /// # Returns
    /// Card count; 0 if hand is empty.
    ///
    /// **Required to override.**
    fn get_hand_count(&self, player: u8) -> usize;

    /// Number of cards currently in `player`'s Main Deck.
    ///
    /// # Returns
    /// Card count; 0 if deck is empty.
    ///
    /// **Required to override.**
    fn get_deck_count(&self, player: u8) -> usize;

    /// Number of cards in `player`'s Graveyard.
    ///
    /// **Required to override.**
    fn get_gy_count(&self, player: u8) -> usize;

    /// Number of cards currently banished for `player`.
    ///
    /// **Required to override.**
    fn get_banished_count(&self, player: u8) -> usize;

    /// Number of cards in `player`'s field zone(s) matched by `location`.
    ///
    /// # Args
    /// - `location`: EDOPro-style location bitmask (e.g. `0x4` = MZONE,
    ///   `0x8` = SZONE). Implementations may combine bits.
    ///
    /// # Returns
    /// Count of cards present in the specified location(s).
    ///
    /// **Required to override.**
    fn get_field_card_count(&self, player: u8, location: u32) -> usize;

    /// Card IDs of every card in `player`'s field zone(s) matched by `location`.
    ///
    /// # Args
    /// - `location`: EDOPro-style location bitmask. Common values:
    ///   `0x1` = DECK, `0x2` = HAND, `0x4` = MZONE, `0x8` = SZONE,
    ///   `0x10` = GRAVE, `0x20` = REMOVED (banished). A value of `0`
    ///   defaults to the on-field union (MZONE | SZONE).
    ///
    /// # Returns
    /// Vec of card IDs; empty if no matching cards.
    ///
    /// **Required to override.**
    fn get_field_cards(&self, player: u8, location: u32) -> Vec<u32>;

    /// Whether `card_id` satisfies `filter`.
    ///
    /// # Args
    /// - `card_id`: Runtime card identifier.
    /// - `filter`: Type/name/archetype predicate to test against.
    ///
    /// # Returns
    /// `true` if the card matches; `false` if it does not or if the card is unknown.
    ///
    /// **Required to override.**
    fn card_matches_filter(&self, card_id: u32, filter: &CardFilter) -> bool;

    /// Numeric stat of `card_id` for the given `stat` variant.
    ///
    /// # Args
    /// - `stat`: Which stat to read. `BaseAtk`/`BaseDef` are pre-modification
    ///   values; `OriginalAtk`/`OriginalDef` are the printed card values.
    ///
    /// # Returns
    /// Stat value; `0` if the card is unknown or the stat is not applicable.
    ///
    /// **Required to override.**
    fn get_card_stat(&self, card_id: u32, stat: &Stat) -> i32;

    /// Sprint 25: card-attribute queries used by predicate filters.
    /// Returns 0 / sentinel for unknown cards. The race/attribute/type
    /// values are EDOPro-style bitfields (e.g., RACE_WARRIOR = 0x1).

    /// Race bitmask of `card_id` in EDOPro format (e.g., `RACE_WARRIOR = 0x1`).
    ///
    /// # Returns
    /// Bitmask; `0` for unknown cards or non-monsters.
    ///
    /// # Default
    /// Returns `0` (no race).
    fn get_card_race(&self, _card_id: u32) -> u64 { 0 }

    /// Attribute bitmask of `card_id` in EDOPro format (e.g., `ATTRIBUTE_DARK = 0x2`).
    ///
    /// # Returns
    /// Bitmask; `0` for unknown cards or non-monsters.
    ///
    /// # Default
    /// Returns `0` (no attribute).
    fn get_card_attribute(&self, _card_id: u32) -> u64 { 0 }

    /// Type bitmask of `card_id` in EDOPro format (e.g., `TYPE_MONSTER | TYPE_EFFECT`).
    ///
    /// # Returns
    /// Bitmask; `0` for unknown cards.
    ///
    /// # Default
    /// Returns `0` (unknown type).
    fn get_card_type(&self, _card_id: u32) -> u64 { 0 }

    /// Canonical card code (passcode) for `card_id`.
    ///
    /// # Returns
    /// The card's passcode. For most cards this equals the card ID itself.
    /// May differ when `change_card_code` has been applied.
    ///
    /// # Default
    /// Returns `card_id` unchanged.
    fn get_card_code(&self, card_id: u32) -> u32 { card_id }

    /// Display name of `card_id`.
    ///
    /// # Returns
    /// The card's name string; empty string for unknown cards.
    ///
    /// # Default
    /// Returns `String::new()`.
    fn get_card_name(&self, _card_id: u32) -> String { String::new() }

    /// Archetype tags that `card_id` belongs to.
    ///
    /// # Returns
    /// A list of archetype name strings (e.g. `["Blue-Eyes", "Dragon"]`);
    /// empty for unknown or untagged cards.
    ///
    /// # Default
    /// Returns an empty `Vec`.
    fn get_card_archetypes(&self, _card_id: u32) -> Vec<String> { Vec::new() }

    /// Card ID of the card whose effect is currently resolving.
    ///
    /// # Returns
    /// The owner card's runtime ID. Used by compiled closures to self-reference
    /// without hard-coding a passcode.
    ///
    /// **Required to override.**
    fn effect_card_id(&self) -> u32;

    /// Player index (0 or 1) who activated the currently-resolving effect.
    ///
    /// **Required to override.**
    fn effect_player(&self) -> u8;

    /// Event-category bitmask for the chain link currently being checked.
    ///
    /// # Returns
    /// EDOPro-style category bits (e.g., `CATEGORY_DRAW`, `CATEGORY_SEARCH`).
    /// Used by hand-trap conditions such as `chain_link_includes`.
    ///
    /// **Required to override.**
    fn event_categories(&self) -> u32;

    // ── Card Movement / Actions ──────────────────────────────

    /// Draw `count` cards from the top of `player`'s deck into their hand.
    ///
    /// # Returns
    /// Number of cards actually drawn (may be less than `count` if the deck runs out).
    ///
    /// **Required to override.**
    fn draw(&mut self, player: u8, count: u32) -> u32;

    /// Destroy each card in `card_ids` by game effect and send it to the Graveyard.
    ///
    /// # Returns
    /// Number of cards successfully destroyed.
    ///
    /// **Required to override.**
    fn destroy(&mut self, card_ids: &[u32]) -> u32;

    /// Send each card in `card_ids` directly to the Graveyard (not destruction).
    ///
    /// # Returns
    /// Number of cards successfully sent.
    ///
    /// **Required to override.**
    fn send_to_grave(&mut self, card_ids: &[u32]) -> u32;

    /// Send each card in `card_ids` to the hand of its controller.
    ///
    /// # Returns
    /// Number of cards successfully moved.
    ///
    /// **Required to override.**
    fn send_to_hand(&mut self, card_ids: &[u32]) -> u32;

    /// Banish each card in `card_ids` (remove from play).
    ///
    /// # Returns
    /// Number of cards successfully banished.
    ///
    /// **Required to override.**
    fn banish(&mut self, card_ids: &[u32]) -> u32;

    /// Discard each card in `card_ids` from the hand to the Graveyard.
    ///
    /// Differs from `send_to_grave` in that the discard origin is always the hand.
    ///
    /// # Returns
    /// Number of cards successfully discarded.
    ///
    /// **Required to override.**
    fn discard(&mut self, card_ids: &[u32]) -> u32;

    /// Special Summon `card_id` onto `player`'s field in `position`.
    ///
    /// # Args
    /// - `position`: EDOPro position constant (e.g., `POS_FACEUP_ATTACK = 0x1`,
    ///   `POS_FACEUP_DEFENSE = 0x4`, `POS_FACEDOWN_DEFENSE = 0x8`).
    ///
    /// # Returns
    /// `true` if the summon succeeded; `false` if it was blocked or the zone was full.
    ///
    /// **Required to override.**
    fn special_summon(&mut self, card_id: u32, player: u8, position: u32) -> bool;

    // ── Life Points ──────────────────────────────────────────

    /// Inflict `amount` points of damage to `player`'s life points.
    ///
    /// # Args
    /// - `damage_type`: reason classification (`Effect`, `Cost`, or `Battle`).
    ///   Engines map this to the corresponding `REASON_*` bit for triggers,
    ///   negation filters, and event logging. Compiler emits `Cost` for
    ///   `PayLp` call sites and `Effect` for direct damage sites; `Battle`
    ///   is never emitted from DSL (battle damage is engine-internal).
    ///
    /// # Returns
    /// `true` if the damage was applied; `false` if it was negated or
    /// redirected by the engine.
    ///
    /// **Required to override.**
    fn damage(&mut self, player: u8, amount: i32, damage_type: DamageType) -> bool;

    /// Restore `amount` life points to `player`.
    ///
    /// # Returns
    /// `true` if the recovery was applied; `false` if the effect was negated.
    ///
    /// **Required to override.**
    fn recover(&mut self, player: u8, amount: i32) -> bool;

    // ── Selection (UI) ───────────────────────────────────────

    /// Prompt `player` to choose between `min` and `max` cards from `candidates`.
    ///
    /// # Args
    /// - `candidates`: Eligible card IDs the player may select from.
    /// - `min`: Minimum number of cards that must be selected.
    /// - `max`: Maximum number of cards that may be selected.
    ///
    /// # Returns
    /// The chosen card IDs. Length is between `min` and `max` inclusive.
    ///
    /// **Required to override.**
    fn select_cards(&mut self, player: u8, candidates: &[u32], min: usize, max: usize) -> Vec<u32>;

    /// Prompt `player` to choose one option from a list of labeled strings.
    ///
    /// # Args
    /// - `options`: Human-readable option labels shown to the player.
    ///
    /// # Returns
    /// Zero-based index of the chosen option.
    ///
    /// **Required to override.**
    fn select_option(&mut self, player: u8, options: &[String]) -> usize;

    // ── Effect Metadata ──────────────────────────────────────

    /// Register the cards currently targeted by the resolving effect.
    ///
    /// # Args
    /// - `card_ids`: IDs of cards that have been targeted. Also updates the
    ///   "last selection" used by `bind_last_selection`.
    ///
    /// **Required to override.**
    fn set_targets(&mut self, card_ids: &[u32]);

    // ── Negation ─────────────────────────────────────────────

    /// Negate the activation of the current chain-link (card + effect are negated).
    ///
    /// # Returns
    /// `true` if the negation was applied; `false` if it failed.
    ///
    /// **Required to override.**
    fn negate_activation(&mut self) -> bool;

    /// Negate only the effect of the current chain-link (card remains on field).
    ///
    /// # Returns
    /// `true` if the negation was applied; `false` if it failed.
    ///
    /// **Required to override.**
    fn negate_effect(&mut self) -> bool;

    // ── Additional Card Movement ──────────────────────────────

    /// Send each card in `card_ids` to the deck.
    ///
    /// # Args
    /// - `top`: If `true`, cards are placed on top of the deck; otherwise bottom.
    ///
    /// # Returns
    /// Number of cards successfully moved.
    ///
    /// **Required to override.**
    fn send_to_deck(&mut self, card_ids: &[u32], top: bool) -> u32;

    /// Return each card in `card_ids` to its controller's hand.
    ///
    /// Differs from `send_to_hand` in that "return to hand" is used for bouncing
    /// cards already on the field, whereas `send_to_hand` is for cards moving from
    /// other locations.
    ///
    /// # Returns
    /// Number of cards successfully returned.
    ///
    /// **Required to override.**
    fn return_to_hand(&mut self, card_ids: &[u32]) -> u32;

    /// Return each card in `card_ids` to its original owner's hand or deck.
    ///
    /// Used when control has changed and cards must go back to the player who
    /// owns them, not the current controller.
    ///
    /// # Returns
    /// Number of cards successfully returned.
    ///
    /// # Default
    /// Returns `0` (no-op).
    fn return_to_owner(&mut self, _card_ids: &[u32]) -> u32 { 0 }

    /// Remove `card_ids` from the field as a tribute cost.
    ///
    /// # Returns
    /// Number of cards successfully tributed.
    ///
    /// **Required to override.**
    fn tribute(&mut self, card_ids: &[u32]) -> u32;

    /// Shuffle `player`'s Main Deck.
    ///
    /// **Required to override.**
    fn shuffle_deck(&mut self, player: u8);

    // ── Stat Modification ────────────────────────────────────

    /// Increase or decrease `card_id`'s current ATK by `delta`.
    ///
    /// Negative `delta` reduces ATK. The modification is relative to the
    /// card's current ATK. See `set_atk` for absolute assignment.
    ///
    /// **Required to override.**
    fn modify_atk(&mut self, card_id: u32, delta: i32);

    /// Increase or decrease `card_id`'s current DEF by `delta`.
    ///
    /// **Required to override.**
    fn modify_def(&mut self, card_id: u32, delta: i32);

    /// Set `card_id`'s current ATK to an absolute `value`.
    ///
    /// **Required to override.**
    fn set_atk(&mut self, card_id: u32, value: i32);

    /// Set `card_id`'s current DEF to an absolute `value`.
    ///
    /// **Required to override.**
    fn set_def(&mut self, card_id: u32, value: i32);

    // ── Battle ───────────────────────────────────────────────

    /// Toggle `card_id` between Attack Position and Defense Position.
    ///
    /// **Required to override.**
    fn change_position(&mut self, card_id: u32);

    // ── Xyz Materials ────────────────────────────────────────

    /// Detach `count` Xyz Materials from `card_id` and send them to the Graveyard.
    ///
    /// # Returns
    /// Number of materials successfully detached (may be less than `count` if
    /// fewer materials are attached).
    ///
    /// **Required to override.**
    fn detach_material(&mut self, card_id: u32, count: u32) -> u32;

    /// Attach `material_id` as an Xyz Material to `target_id`.
    ///
    /// **Required to override.**
    fn attach_material(&mut self, material_id: u32, target_id: u32);

    // ── Counters ─────────────────────────────────────────────

    /// Place `count` counters of type `counter_name` onto `card_id`.
    ///
    /// If counters of that type already exist, the count accumulates.
    ///
    /// **Required to override.**
    fn place_counter(&mut self, card_id: u32, counter_name: &str, count: u32);

    /// Remove `count` counters of type `counter_name` from `card_id`.
    ///
    /// Saturates at zero — does not underflow.
    ///
    /// **Required to override.**
    fn remove_counter(&mut self, card_id: u32, counter_name: &str, count: u32);

    // ── Deck operations ──────────────────────────────────────

    /// Send `count` cards from the top of `player`'s deck to the Graveyard.
    ///
    /// # Returns
    /// Number of cards successfully milled (may be less than `count` if the deck
    /// runs out).
    ///
    /// # Default
    /// Returns `0` (no-op).
    fn mill(&mut self, _player: u8, _count: u32) -> u32 { 0 }

    /// Reveal the top `count` cards of `player`'s deck without sending them.
    ///
    /// # Returns
    /// Card IDs of the excavated cards in deck order (top card last).
    /// The cards remain in the deck unless the effect explicitly moves them.
    ///
    /// # Default
    /// Returns an empty `Vec` (no-op).
    fn excavate(&mut self, _player: u8, _count: u32) -> Vec<u32> { Vec::new() }

    // ── Phase 2: Custom events ───────────────────────────────

    /// Fire a named custom game event, associating it with `cards`.
    ///
    /// # Args
    /// - `name`: Arbitrary event tag (engine-defined; e.g., `"on_tribute"`).
    /// - `cards`: Cards involved in or caused by this event.
    ///
    /// # Default
    /// No-op.
    fn raise_custom_event(&mut self, _name: &str, _cards: &[u32]) {}

    // ── Phase 3: Confirm cards ───────────────────────────────

    /// Reveal `cards` to `audience` (typically to demonstrate a cost or condition).
    ///
    /// # Args
    /// - `owner`: Player who owns the cards being confirmed.
    /// - `audience`: Player to whom the cards are shown.
    /// - `cards`: Card IDs to confirm/reveal.
    ///
    /// # Default
    /// No-op.
    fn confirm_cards(&mut self, _owner: u8, _audience: u8, _cards: &[u32]) {}

    // ── Phase 3: Announce ────────────────────────────────────

    /// Prompt `player` to announce a card name or type, returning an opaque token.
    ///
    /// # Args
    /// - `kind`: Announcement category (engine-defined; `0` = card name).
    /// - `filter_mask`: Restricts what may be announced (engine-defined bitmask;
    ///   `0` = unrestricted).
    ///
    /// # Returns
    /// An opaque token that can later be passed to `get_announcement` to retrieve
    /// the announced value.
    ///
    /// # Default
    /// Returns `0`.
    fn announce(&mut self, _player: u8, _kind: u8, _filter_mask: u32) -> u32 { 0 }

    /// Retrieve the value associated with a prior `announce` call.
    ///
    /// # Args
    /// - `token`: The token returned by `announce`.
    ///
    /// # Returns
    /// The announced card code or type value; `0` if the token is unknown.
    ///
    /// # Default
    /// Returns `0`.
    fn get_announcement(&self, _token: u32) -> u32 { 0 }

    // ── Phase 1A: Flag effects ───────────────────────────────

    /// Register a persistent flag on `card_id` that tracks a continuous state.
    ///
    /// # Args
    /// - `name`: Flag identifier string (e.g., `"used_once"`).
    /// - `survives_mask`: Phase/event bitmask describing when the flag persists.
    /// - `resets_mask`: Phase/event bitmask describing when the flag is cleared.
    ///
    /// # Default
    /// No-op.
    fn register_flag(&mut self, _card_id: u32, _name: &str, _survives_mask: u32, _resets_mask: u32) {}

    /// Remove the flag named `name` from `card_id`.
    ///
    /// # Default
    /// No-op.
    fn clear_flag(&mut self, _card_id: u32, _name: &str) {}

    /// Whether `card_id` currently has the flag named `name`.
    ///
    /// # Returns
    /// `true` if the flag is set; `false` if absent or if the card is unknown.
    ///
    /// # Default
    /// Returns `false`.
    fn has_flag(&self, _card_id: u32, _name: &str) -> bool { false }

    // ── Phase 1B: History queries ────────────────────────────

    /// Location bitmask where `card_id` was before its most recent move.
    ///
    /// # Returns
    /// EDOPro location bitmask; `0` if unknown or not yet moved.
    ///
    /// # Default
    /// Returns `0`.
    fn previous_location(&self, _card_id: u32) -> u32 { 0 }

    /// Position (face-up/face-down/attack/defense) of `card_id` before its
    /// most recent position change.
    ///
    /// # Returns
    /// EDOPro position constant; `0` if unknown.
    ///
    /// # Default
    /// Returns `0`.
    fn previous_position(&self, _card_id: u32) -> u32 { 0 }

    /// Reason bitmask describing why `card_id` was sent to the Graveyard (or other zone).
    ///
    /// # Returns
    /// EDOPro `REASON_*` bitmask (e.g., `REASON_DESTROY`, `REASON_COST`); `0` if unknown.
    ///
    /// # Default
    /// Returns `0`.
    fn sent_by_reason(&self, _card_id: u32) -> u32 { 0 }

    // ── Phase 1D: Named bindings ─────────────────────────────

    /// Store `card_id` under the named binding `name` for later retrieval.
    ///
    /// Used by compiled closures to pass a card reference between steps of a
    /// multi-step effect without re-selecting it.
    ///
    /// # Default
    /// No-op.
    fn set_binding(&mut self, _name: &str, _card_id: u32) {}

    /// Bind the most recently selected group of cards under `name`.
    ///
    /// Equivalent to calling `set_binding` for each card returned by the last
    /// `select_cards` or `set_targets` call.
    ///
    /// # Default
    /// No-op.
    fn bind_last_selection(&mut self, _name: &str) {}

    /// Retrieve the primary card associated with the binding named `name`.
    ///
    /// # Returns
    /// The first card ID in the binding, or `None` if the binding does not exist.
    ///
    /// # Default
    /// Returns `None`.
    fn get_binding_card(&self, _name: &str) -> Option<u32> { None }

    /// Read a numeric field from the card stored under binding `name`.
    ///
    /// # Args
    /// - `field`: Stat field name (`"atk"`, `"def"`, `"level"`, `"code"`).
    ///
    /// # Returns
    /// The field value, or `0` if the binding does not exist or the field is unknown.
    ///
    /// # Default
    /// Returns `0`.
    fn get_binding_field(&self, _name: &str, _field: &str) -> i32 { 0 }

    // ── Phase 1E: Change name / code ─────────────────────────

    /// Temporarily change `card_id`'s card code (passcode) to `code`.
    ///
    /// # Args
    /// - `code`: The new passcode to assign.
    /// - `duration_mask`: Phase/event bitmask describing when the change expires.
    ///
    /// # Default
    /// No-op.
    fn change_card_code(&mut self, _card_id: u32, _code: u32, _duration_mask: u32) {}

    // ── Sprint 67: Control change ────────────────────────────

    /// Transfer control of `card_id` to `new_controller`.
    ///
    /// # Args
    /// - `new_controller`: 0 or 1.
    ///
    /// # Default
    /// No-op.
    fn take_control(&mut self, _card_id: u32, _new_controller: u8) {}

    // ── Sprint 67: Token creation ────────────────────────────

    /// Create `count` Token monsters on `player`'s field with the given stats.
    ///
    /// # Args
    /// - `atk`: ATK value of each created token.
    /// - `def`: DEF value of each created token.
    /// - `count`: Number of tokens to create.
    ///
    /// # Default
    /// No-op.
    fn create_token(&mut self, _player: u8, _atk: i32, _def: i32, _count: u32) {}

    // ── Sprint 67: Cross-phase state ─────────────────────────

    /// Persist an integer value across effect steps under string key `label`.
    ///
    /// Useful for recording a stat snapshot (e.g., ATK at activation time) that
    /// must survive until resolution. See `recall_value` to read it back.
    ///
    /// # Default
    /// No-op.
    fn store_value(&mut self, _label: &str, _value: i32) {}

    /// Retrieve a previously stored integer by `label`.
    ///
    /// # Returns
    /// The stored value, or `0` if no value exists for `label`.
    ///
    /// # Default
    /// Returns `0`.
    fn recall_value(&self, _label: &str) -> i32 { 0 }

    // ── Sprint 67: Delayed effect registration ───────────────

    /// Schedule `card_id`'s effect to fire during a future game phase.
    ///
    /// # Args
    /// - `phase`: Phase constant at which the delayed effect should trigger
    ///   (engine-defined; e.g., end-of-turn phase code).
    /// - `card_id`: Card whose delayed effect is being registered.
    ///
    /// # Default
    /// No-op.
    fn register_delayed(&mut self, _phase: u32, _card_id: u32) {}

    // ── Phase / State Queries ────────────────────────────────

    /// Current game phase as an engine phase constant.
    ///
    /// # Returns
    /// Phase code; `0` if unknown or not applicable.
    ///
    /// # Default
    /// Returns `0`.
    fn get_current_phase(&self) -> u32 { 0 }

    /// Whether `card_id` is currently face-up.
    ///
    /// # Default
    /// Returns `true`.
    fn is_face_up(&self, _card_id: u32) -> bool { true }

    /// Whether `card_id` is currently face-down.
    ///
    /// # Default
    /// Returns `false`.
    fn is_face_down(&self, _card_id: u32) -> bool { false }

    /// Whether `card_id` is in Attack Position.
    ///
    /// # Default
    /// Returns `true`.
    fn is_attack_position(&self, _card_id: u32) -> bool { true }

    /// Whether `card_id` is in Defense Position.
    ///
    /// # Default
    /// Returns `false`.
    fn is_defense_position(&self, _card_id: u32) -> bool { false }

    /// Whether `card_id` has declared an attack during the current Battle Phase.
    ///
    /// # Default
    /// Returns `false`.
    fn has_attacked_this_turn(&self, _card_id: u32) -> bool { false }

    /// Whether `card_id` was summoned during the current turn.
    ///
    /// # Default
    /// Returns `false`.
    fn was_summoned_this_turn(&self, _card_id: u32) -> bool { false }

    /// Whether `card_id` was flipped face-up during the current turn.
    ///
    /// # Default
    /// Returns `false`.
    fn was_flipped_this_turn(&self, _card_id: u32) -> bool { false }

    /// Whether `card_id` has at least one counter of type `counter_name`.
    ///
    /// # Default
    /// Returns `false`.
    fn has_counter(&self, _card_id: u32, _counter_name: &str) -> bool { false }

    /// Number of counters of type `counter_name` currently on `card_id`.
    ///
    /// # Returns
    /// Counter count; `0` if none or if the card is unknown.
    ///
    /// # Default
    /// Returns `0`.
    fn get_counter_count(&self, _card_id: u32, _counter_name: &str) -> u32 { 0 }

    /// Whether any effect in the current chain belongs to the given category.
    ///
    /// # Args
    /// - `category`: EDOPro `CATEGORY_*` bitmask to test.
    ///
    /// # Returns
    /// `true` if the chain contains at least one link matching `category`.
    ///
    /// # Default
    /// Returns `false`.
    fn chain_includes_category(&self, _category: u32) -> bool { false }

    // ── Card Identity Changes ────────────────────────────────

    /// Set `card_id`'s level to `level`.
    ///
    /// # Default
    /// No-op.
    fn change_level(&mut self, _card_id: u32, _level: u32) {}

    /// Set `card_id`'s attribute to the given EDOPro bitmask.
    ///
    /// # Default
    /// No-op.
    fn change_attribute(&mut self, _card_id: u32, _attribute: u32) {}

    /// Set `card_id`'s race (type) to the given EDOPro bitmask.
    ///
    /// # Default
    /// No-op.
    fn change_race(&mut self, _card_id: u32, _race: u32) {}

    /// Temporarily rename `card_id` to `name` for the duration described by `duration`.
    ///
    /// # Args
    /// - `duration`: Phase/event bitmask describing when the name change expires
    ///   (same convention as `change_card_code`).
    ///
    /// # Default
    /// No-op.
    fn change_name(&mut self, _card_id: u32, _name: &str, _duration: u32) {}

    /// Set the Pendulum Scale of `card_id` to `scale`.
    ///
    /// # Default
    /// No-op.
    fn set_scale(&mut self, _card_id: u32, _scale: u32) {}

    // ── Info / RNG ───────────────────────────────────────────────────

    /// Publicly reveal `card_ids` to all players (e.g., from the hand).
    ///
    /// # Default
    /// No-op.
    fn reveal(&mut self, _card_ids: &[u32]) {}

    /// Show `card_ids` to `player` only (private knowledge).
    ///
    /// # Args
    /// - `player`: The player who sees the cards.
    ///
    /// # Default
    /// No-op.
    fn look_at(&mut self, _player: u8, _card_ids: &[u32]) {}

    /// Perform a coin flip for `player`.
    ///
    /// # Returns
    /// `true` for heads, `false` for tails.
    ///
    /// # Default
    /// Returns `true` (deterministic heads).
    fn coin_flip(&mut self, _player: u8) -> bool { true }

    /// Roll a six-sided die for `player`.
    ///
    /// # Returns
    /// Result in the range 1–6 inclusive.
    ///
    /// # Default
    /// Returns `1` (deterministic).
    fn dice_roll(&mut self, _player: u8) -> u32 { 1 }

    // ── Summon / Set ─────────────────────────────────────────────────

    /// Perform a Normal Summon of `card_id` to `player`'s field.
    ///
    /// # Returns
    /// `true` if the summon succeeded.
    ///
    /// # Default
    /// Returns `true`.
    fn normal_summon(&mut self, _card_id: u32, _player: u8) -> bool { true }

    /// Set `card_id` face-down on `player`'s field.
    ///
    /// Applies to both monsters (Set instead of Summon) and spells/traps.
    ///
    /// # Returns
    /// `true` if the card was set successfully.
    ///
    /// # Default
    /// Returns `true`.
    fn set_card(&mut self, _card_id: u32, _player: u8) -> bool { true }

    // ── Extra Deck Summons ───────────────────────────────────────────

    /// Perform a Ritual Summon of `card_id` using `material_ids` as tributes.
    ///
    /// # Returns
    /// `true` if the summon succeeded.
    ///
    /// # Default
    /// Returns `true`.
    fn ritual_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }

    /// Perform a Fusion Summon of `card_id` using `material_ids`.
    ///
    /// # Returns
    /// `true` if the summon succeeded.
    ///
    /// # Default
    /// Returns `true`.
    fn fusion_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }

    /// Perform a Synchro Summon of `card_id` using `material_ids`.
    ///
    /// # Returns
    /// `true` if the summon succeeded.
    ///
    /// # Default
    /// Returns `true`.
    fn synchro_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }

    /// Perform an Xyz Summon of `card_id` using `material_ids` as overlay units.
    ///
    /// # Returns
    /// `true` if the summon succeeded.
    ///
    /// # Default
    /// Returns `true`.
    fn xyz_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }

    // ── Equip ────────────────────────────────────────────────────────

    /// Equip `equip_id` to `target_id` as an Equip Spell.
    ///
    /// # Default
    /// No-op.
    fn equip_card(&mut self, _equip_id: u32, _target_id: u32) {}

    // ── Swap ─────────────────────────────────────────────────────────

    /// Swap control of `card_a` and `card_b` between players.
    ///
    /// # Default
    /// No-op.
    fn swap_control(&mut self, _card_a: u32, _card_b: u32) {}

    /// Swap the ATK and DEF values of `card_id`.
    ///
    /// # Default
    /// No-op.
    fn swap_stats(&mut self, _card_id: u32) {}

    // ── Grant (continuous effect registration) ───────────────────────

    /// Register a continuous grant effect on `card_id`.
    ///
    /// # Args
    /// - `grant_code`: Engine-defined code identifying which ability or immunity
    ///   is granted (e.g., `GRANT_CANNOT_BE_DESTROYED`).
    /// - `duration`: Phase/event bitmask describing how long the grant lasts.
    ///
    /// # Default
    /// No-op.
    fn register_grant(&mut self, _card_id: u32, _grant_code: u32, _duration: u32) {}
}
