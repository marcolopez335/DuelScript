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

// ── Runtime Abstraction ───────────────────────────────────────

/// Trait that engines implement to expose game state and operations
/// to compiled DuelScript closures.
///
/// Intentionally not `Send + Sync` — implementations often use `Rc<RefCell>`.
/// The closures themselves are `Send + Sync` because they receive the runtime
/// as a parameter rather than capturing it.
pub trait DuelScriptRuntime {
    // ── Queries ──────────────────────────────────────────────
    fn get_lp(&self, player: u8) -> i32;
    fn get_hand_count(&self, player: u8) -> usize;
    fn get_deck_count(&self, player: u8) -> usize;
    fn get_gy_count(&self, player: u8) -> usize;
    fn get_banished_count(&self, player: u8) -> usize;
    fn get_field_card_count(&self, player: u8, location: u32) -> usize;
    fn get_field_cards(&self, player: u8, location: u32) -> Vec<u32>;
    fn card_matches_filter(&self, card_id: u32, filter: &CardFilter) -> bool;
    fn get_card_stat(&self, card_id: u32, stat: &Stat) -> i32;

    /// Sprint 25: card-attribute queries used by predicate filters.
    /// Returns 0 / sentinel for unknown cards. The race/attribute/type
    /// values are EDOPro-style bitfields (e.g., RACE_WARRIOR = 0x1).
    fn get_card_race(&self, _card_id: u32) -> u64 { 0 }
    fn get_card_attribute(&self, _card_id: u32) -> u64 { 0 }
    fn get_card_type(&self, _card_id: u32) -> u64 { 0 }
    fn get_card_code(&self, card_id: u32) -> u32 { card_id }
    fn get_card_name(&self, _card_id: u32) -> String { String::new() }
    fn get_card_archetypes(&self, _card_id: u32) -> Vec<String> { Vec::new() }

    /// Get the card_id of the effect's owner
    fn effect_card_id(&self) -> u32;
    /// Get the activating player (0 or 1)
    fn effect_player(&self) -> u8;
    /// Get event categories (for chain-link checking)
    fn event_categories(&self) -> u32;

    // ── Card Movement / Actions ──────────────────────────────
    fn draw(&mut self, player: u8, count: u32) -> u32;
    fn destroy(&mut self, card_ids: &[u32]) -> u32;
    fn send_to_grave(&mut self, card_ids: &[u32]) -> u32;
    fn send_to_hand(&mut self, card_ids: &[u32]) -> u32;
    fn banish(&mut self, card_ids: &[u32]) -> u32;
    fn discard(&mut self, card_ids: &[u32]) -> u32;
    fn special_summon(&mut self, card_id: u32, player: u8, position: u32) -> bool;

    // ── Life Points ──────────────────────────────────────────
    fn damage(&mut self, player: u8, amount: i32) -> bool;
    fn recover(&mut self, player: u8, amount: i32) -> bool;

    // ── Selection (UI) ───────────────────────────────────────
    fn select_cards(&mut self, player: u8, candidates: &[u32], min: usize, max: usize) -> Vec<u32>;
    fn select_option(&mut self, player: u8, options: &[String]) -> usize;

    // ── Effect Metadata ──────────────────────────────────────
    fn set_operation_info(&mut self, category: u32, count: u32);
    fn set_targets(&mut self, card_ids: &[u32]);

    // ── Negation ─────────────────────────────────────────────
    fn negate_activation(&mut self) -> bool;
    fn negate_effect(&mut self) -> bool;

    // ── Additional Card Movement ──────────────────────────────
    fn send_to_deck(&mut self, card_ids: &[u32], top: bool) -> u32;
    fn return_to_hand(&mut self, card_ids: &[u32]) -> u32;
    fn tribute(&mut self, card_ids: &[u32]) -> u32;
    fn shuffle_deck(&mut self, player: u8);

    // ── Stat Modification ────────────────────────────────────
    fn modify_atk(&mut self, card_id: u32, delta: i32);
    fn modify_def(&mut self, card_id: u32, delta: i32);
    fn set_atk(&mut self, card_id: u32, value: i32);
    fn set_def(&mut self, card_id: u32, value: i32);

    // ── Battle ───────────────────────────────────────────────
    fn negate_attack(&mut self) -> bool;
    fn change_position(&mut self, card_id: u32);

    // ── Xyz Materials ────────────────────────────────────────
    fn detach_material(&mut self, card_id: u32, count: u32) -> u32;
    fn attach_material(&mut self, material_id: u32, target_id: u32);

    // ── Counters ─────────────────────────────────────────────
    fn place_counter(&mut self, card_id: u32, counter_name: &str, count: u32);
    fn remove_counter(&mut self, card_id: u32, counter_name: &str, count: u32);

    // ── Deck operations ──────────────────────────────────────
    fn mill(&mut self, _player: u8, _count: u32) -> u32 { 0 }
    fn excavate(&mut self, _player: u8, _count: u32) -> Vec<u32> { Vec::new() }

    // ── Count matching cards ─────────────────────────────────
    fn count_matching(&self, player: u8, location: u32, filter: &CardFilter) -> usize;

    // ── Phase 2: Custom events ───────────────────────────────
    fn raise_custom_event(&mut self, _name: &str, _cards: &[u32]) {}

    // ── Phase 3: Confirm cards ───────────────────────────────
    fn confirm_cards(&mut self, _owner: u8, _audience: u8, _cards: &[u32]) {}

    // ── Phase 3: Announce ────────────────────────────────────
    fn announce(&mut self, _player: u8, _kind: u8, _filter_mask: u32) -> u32 { 0 }
    fn get_announcement(&self, _token: u32) -> u32 { 0 }

    // ── Phase 1A: Flag effects ───────────────────────────────
    fn register_flag(&mut self, _card_id: u32, _name: &str, _survives_mask: u32, _resets_mask: u32) {}
    fn clear_flag(&mut self, _card_id: u32, _name: &str) {}
    fn has_flag(&self, _card_id: u32, _name: &str) -> bool { false }

    // ── Phase 1B: History queries ────────────────────────────
    fn previous_location(&self, _card_id: u32) -> u32 { 0 }
    fn previous_position(&self, _card_id: u32) -> u32 { 0 }
    fn sent_by_reason(&self, _card_id: u32) -> u32 { 0 }

    // ── Phase 1D: Named bindings ─────────────────────────────
    fn set_binding(&mut self, _name: &str, _card_id: u32) {}
    fn bind_last_selection(&mut self, _name: &str) {}
    fn get_binding_card(&self, _name: &str) -> Option<u32> { None }
    fn get_binding_field(&self, _name: &str, _field: &str) -> i32 { 0 }

    // ── Phase 1E: Change name / code ─────────────────────────
    fn change_card_code(&mut self, _card_id: u32, _code: u32, _duration_mask: u32) {}

    // ── Sprint 67: Control change ────────────────────────────
    fn take_control(&mut self, _card_id: u32, _new_controller: u8) {}

    // ── Sprint 67: Token creation ────────────────────────────
    fn create_token(&mut self, _player: u8, _atk: i32, _def: i32, _count: u32) {}

    // ── Sprint 67: Cross-phase state ─────────────────────────
    fn store_value(&mut self, _label: &str, _value: i32) {}
    fn recall_value(&self, _label: &str) -> i32 { 0 }

    // ── Sprint 67: Delayed effect registration ───────────────
    fn register_delayed(&mut self, _phase: u32, _card_id: u32) {}

    // ── Phase / State Queries ────────────────────────────────
    fn get_current_phase(&self) -> u32 { 0 }
    fn is_face_up(&self, _card_id: u32) -> bool { true }
    fn is_face_down(&self, _card_id: u32) -> bool { false }
    fn is_attack_position(&self, _card_id: u32) -> bool { true }
    fn is_defense_position(&self, _card_id: u32) -> bool { false }
    fn has_attacked_this_turn(&self, _card_id: u32) -> bool { false }
    fn was_summoned_this_turn(&self, _card_id: u32) -> bool { false }
    fn was_flipped_this_turn(&self, _card_id: u32) -> bool { false }
    fn has_counter(&self, _card_id: u32, _counter_name: &str) -> bool { false }
    fn get_counter_count(&self, _card_id: u32, _counter_name: &str) -> u32 { 0 }
    fn chain_includes_category(&self, _category: u32) -> bool { false }

    // ── Card Identity Changes ────────────────────────────────
    fn change_level(&mut self, _card_id: u32, _level: u32) {}
    fn change_attribute(&mut self, _card_id: u32, _attribute: u32) {}
    fn change_race(&mut self, _card_id: u32, _race: u32) {}
    fn change_name(&mut self, _card_id: u32, _name: &str, _duration: u32) {}
    fn set_scale(&mut self, _card_id: u32, _scale: u32) {}

    // ── Info / RNG ───────────────────────────────────────────────────
    fn reveal(&mut self, _card_ids: &[u32]) {}
    fn look_at(&mut self, _player: u8, _card_ids: &[u32]) {}
    fn coin_flip(&mut self, _player: u8) -> bool { true }
    fn dice_roll(&mut self, _player: u8) -> u32 { 1 }

    // ── Summon / Set ─────────────────────────────────────────────────
    fn normal_summon(&mut self, _card_id: u32, _player: u8) -> bool { true }
    fn set_card(&mut self, _card_id: u32, _player: u8) -> bool { true }

    // ── Extra Deck Summons ───────────────────────────────────────────
    fn ritual_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }
    fn fusion_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }
    fn synchro_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }
    fn xyz_summon(&mut self, _card_id: u32, _player: u8, _material_ids: &[u32]) -> bool { true }

    // ── Equip ────────────────────────────────────────────────────────
    fn equip_card(&mut self, _equip_id: u32, _target_id: u32) {}

    // ── Grant (continuous effect registration) ───────────────────────
    fn register_grant(&mut self, _card_id: u32, _grant_code: u32, _duration: u32) {}
}
