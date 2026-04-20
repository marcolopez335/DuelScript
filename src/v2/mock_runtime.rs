// ============================================================
// MockRuntime (v2-local copy)
//
// Records every DuelScriptRuntime method call into a log so tests
// can assert on what the compiled callbacks actually did. Holds a
// minimal mutable game state — LP, hand/deck/gy/field per player,
// bindings, flags — that callbacks can read and write through the
// trait methods.
//
// Design goals:
//   - Deterministic: no randomness; selection picks the first N
//     cards from the candidate list.
//   - Observable: every call records both the method name and a
//     debug string of arguments so failures are diagnosable.
//   - Mutable: actions like draw, destroy, damage actually update
//     the state, so multi-step effects work end-to-end.
//   - Permissive: features that aren't relevant to the test (e.g.
//     UI selection, animation, network) are no-ops.
// ============================================================

use std::collections::HashMap;
use super::runtime::{CardFilter, DamageType, Stat, DuelScriptRuntime};

// ── Recorded call types ──────────────────────────────────────

/// One method call recorded by the MockRuntime.
#[derive(Debug, Clone, PartialEq)]
pub struct RuntimeCall {
    pub method: String,
    pub args: String,
}

impl RuntimeCall {
    fn new(method: &str, args: impl Into<String>) -> Self {
        Self { method: method.to_string(), args: args.into() }
    }
}

// ── Mock card snapshot ───────────────────────────────────────

/// A simple stand-in for a real card. Just enough fields to satisfy
/// the runtime's stat queries, filter checks, and Sprint 25 predicate
/// queries (race / attribute / type bitmasks).
#[derive(Debug, Clone)]
pub struct CardSnapshot {
    pub id: u32,
    pub name: String,
    pub atk: i32,
    pub def: i32,
    pub level: u32,
    pub is_monster: bool,
    pub is_spell: bool,
    pub is_trap: bool,
    /// EDOPro RACE_X bitmask. 0 if unknown.
    pub race: u64,
    /// EDOPro ATTRIBUTE_X bitmask. 0 if unknown.
    pub attribute: u64,
    /// EDOPro TYPE_X bitmask (TYPE_MONSTER | TYPE_EFFECT | etc.).
    pub type_bits: u64,
    /// Archetype names this card belongs to (for is_archetype predicate).
    pub archetypes: Vec<String>,
}

impl CardSnapshot {
    pub fn monster(id: u32, name: &str, atk: i32, def: i32, level: u32) -> Self {
        Self {
            id, name: name.to_string(), atk, def, level,
            is_monster: true, is_spell: false, is_trap: false,
            race: 0, attribute: 0,
            type_bits: 0x1 | 0x20, // TYPE_MONSTER + TYPE_EFFECT
            archetypes: Vec::new(),
        }
    }

    pub fn spell(id: u32, name: &str) -> Self {
        Self {
            id, name: name.to_string(), atk: 0, def: 0, level: 0,
            is_monster: false, is_spell: true, is_trap: false,
            race: 0, attribute: 0,
            type_bits: 0x2, // TYPE_SPELL
            archetypes: Vec::new(),
        }
    }

    pub fn trap(id: u32, name: &str) -> Self {
        Self {
            id, name: name.to_string(), atk: 0, def: 0, level: 0,
            is_monster: false, is_spell: false, is_trap: true,
            race: 0, attribute: 0,
            type_bits: 0x4, // TYPE_TRAP
            archetypes: Vec::new(),
        }
    }

    /// Sprint 25 builder: attach race/attribute bitmasks.
    pub fn with_race(mut self, race_bits: u64) -> Self {
        self.race = race_bits;
        self
    }
    pub fn with_attribute(mut self, attr_bits: u64) -> Self {
        self.attribute = attr_bits;
        self
    }
    pub fn with_archetype(mut self, name: &str) -> Self {
        self.archetypes.push(name.to_string());
        self
    }
    /// T7 (exotic predicate atoms) builder: overwrite the type bitmask
    /// entirely. Use when testing `IsTuner` / `IsFusion` / `IsSynchro` /
    /// etc. where the default `monster(...)` type_bits (TYPE_MONSTER |
    /// TYPE_EFFECT = 0x1 | 0x20) must be replaced with a specific
    /// sub-type mask.
    pub fn with_type(mut self, type_bits: u64) -> Self {
        self.type_bits = type_bits;
        self
    }
}

// ── Player state ─────────────────────────────────────────────

#[derive(Debug, Clone, Default)]
pub struct PlayerState {
    pub lp: i32,
    pub hand: Vec<u32>,
    pub deck: Vec<u32>,
    pub graveyard: Vec<u32>,
    pub banished: Vec<u32>,
    pub field_monsters: Vec<u32>,
    pub field_spells: Vec<u32>,
}

impl PlayerState {
    pub fn fresh() -> Self {
        Self { lp: 8000, ..Default::default() }
    }
}

// ── Mock game state ──────────────────────────────────────────

/// The complete mutable state of a mock duel.
#[derive(Debug, Clone)]
pub struct MockState {
    pub players: [PlayerState; 2],
    pub cards: HashMap<u32, CardSnapshot>,
    /// Persistent flags: (card_id, name) → present
    pub flags: HashMap<(u32, String), ()>,
    /// Counters: (card_id, counter_name) → count
    pub counters: HashMap<(u32, String), u32>,
    /// Named bindings for the current effect resolution
    pub bindings: HashMap<String, Vec<u32>>,
    /// The "last selected" group, used by bind_last_selection
    pub last_selection: Vec<u32>,
}

impl Default for MockState {
    fn default() -> Self {
        Self {
            players: [PlayerState::fresh(), PlayerState::fresh()],
            cards: HashMap::new(),
            flags: HashMap::new(),
            counters: HashMap::new(),
            bindings: HashMap::new(),
            last_selection: Vec::new(),
        }
    }
}

impl MockState {
    pub fn add_card(&mut self, snap: CardSnapshot) {
        self.cards.insert(snap.id, snap);
    }
}

// ── MockRuntime ──────────────────────────────────────────────

/// Records all DuelScriptRuntime calls and updates a MockState.
pub struct MockRuntime {
    pub state: MockState,
    pub calls: Vec<RuntimeCall>,
    /// Effect context: which player activated, which card the effect belongs to.
    pub effect_player: u8,
    pub effect_card_id: u32,
    /// Categories present on the chain link being checked (for hand traps).
    pub event_categories: u32,
}

impl MockRuntime {
    pub fn new() -> Self {
        Self {
            state: MockState::default(),
            calls: Vec::new(),
            effect_player: 0,
            effect_card_id: 0,
            event_categories: 0,
        }
    }

    /// Convenient builder: 8000 LP, both players empty.
    pub fn fresh() -> Self {
        Self::new()
    }

    fn record(&mut self, method: &str, args: impl Into<String>) {
        self.calls.push(RuntimeCall::new(method, args));
    }

    /// Number of times a method was called.
    pub fn call_count(&self, method: &str) -> usize {
        self.calls.iter().filter(|c| c.method == method).count()
    }

    /// True if any recorded call matches both method and substring of args.
    pub fn was_called_with(&self, method: &str, args_contains: &str) -> bool {
        self.calls.iter().any(|c| c.method == method && c.args.contains(args_contains))
    }

    /// Pretty-print the call log (for debugging/inspection).
    pub fn dump_calls(&self) -> String {
        let mut out = String::new();
        for (i, c) in self.calls.iter().enumerate() {
            out.push_str(&format!("  {:>3}. {}({})\n", i + 1, c.method, c.args));
        }
        out
    }
}

impl Default for MockRuntime {
    fn default() -> Self { Self::new() }
}

// ── DuelScriptRuntime impl ───────────────────────────────────

impl DuelScriptRuntime for MockRuntime {
    // ── Queries ──────────────────────────────────────────────
    fn get_lp(&self, player: u8) -> i32 {
        self.state.players[player as usize].lp
    }
    fn get_hand_count(&self, player: u8) -> usize {
        self.state.players[player as usize].hand.len()
    }
    fn get_deck_count(&self, player: u8) -> usize {
        self.state.players[player as usize].deck.len()
    }
    fn get_gy_count(&self, player: u8) -> usize {
        self.state.players[player as usize].graveyard.len()
    }
    fn get_banished_count(&self, player: u8) -> usize {
        self.state.players[player as usize].banished.len()
    }
    fn get_field_card_count(&self, player: u8, location: u32) -> usize {
        self.get_field_cards(player, location).len()
    }
    fn get_field_cards(&self, player: u8, location: u32) -> Vec<u32> {
        // Honor LOCATION_* bitmask so cards that target GY/hand/deck work.
        // Constants from constants: HAND=0x2, MZONE=0x4, SZONE=0x8,
        // GRAVE=0x10, REMOVED=0x20, DECK=0x1.
        let p = &self.state.players[player as usize];
        let mut out = Vec::new();
        if location & 0x4 != 0 { out.extend_from_slice(&p.field_monsters); }
        if location & 0x8 != 0 { out.extend_from_slice(&p.field_spells); }
        if location & 0x10 != 0 { out.extend_from_slice(&p.graveyard); }
        if location & 0x20 != 0 { out.extend_from_slice(&p.banished); }
        if location & 0x2 != 0 { out.extend_from_slice(&p.hand); }
        if location & 0x1 != 0 { out.extend_from_slice(&p.deck); }
        // If no specific bits given, default to on-field union.
        // (0xC is already covered by the MZONE+SZONE bits above.)
        if location == 0 {
            out.extend_from_slice(&p.field_monsters);
            out.extend_from_slice(&p.field_spells);
        }
        out
    }
    fn card_matches_filter(&self, card_id: u32, filter: &CardFilter) -> bool {
        let Some(card) = self.state.cards.get(&card_id) else { return false };
        match filter {
            CardFilter::Monster        => card.is_monster,
            CardFilter::Spell          => card.is_spell,
            CardFilter::Trap           => card.is_trap,
            CardFilter::Card           => true,
            CardFilter::EffectMonster  => card.is_monster,
            _                          => true,
        }
    }
    fn get_card_stat(&self, card_id: u32, stat: &Stat) -> i32 {
        let Some(card) = self.state.cards.get(&card_id) else { return 0 };
        match stat {
            Stat::Atk | Stat::BaseAtk | Stat::OriginalAtk => card.atk,
            Stat::Def | Stat::BaseDef | Stat::OriginalDef => card.def,
            Stat::Level => card.level as i32,
            // Stat::Rank intentionally maps to `card.level` — Xyz rank is
            // stored in the `level` slot by engine convention (see the
            // YgobeetleRuntimeAdapter mirror at ds_runtime_adapter.rs).
            // Backlog item 2 originally flagged this as a naming concern;
            // T9 (LLL-I) resolves as "working-as-intended; convention doc-locked."
            Stat::Rank  => card.level as i32,
        }
    }
    // Sprint 25: card-attribute queries.
    fn get_card_race(&self, card_id: u32) -> u64 {
        self.state.cards.get(&card_id).map(|c| c.race).unwrap_or(0)
    }
    fn get_card_attribute(&self, card_id: u32) -> u64 {
        self.state.cards.get(&card_id).map(|c| c.attribute).unwrap_or(0)
    }
    fn get_card_type(&self, card_id: u32) -> u64 {
        self.state.cards.get(&card_id).map(|c| c.type_bits).unwrap_or(0)
    }
    fn get_card_code(&self, card_id: u32) -> u32 { card_id }
    fn get_card_name(&self, card_id: u32) -> String {
        self.state.cards.get(&card_id).map(|c| c.name.clone()).unwrap_or_default()
    }
    fn get_card_archetypes(&self, card_id: u32) -> Vec<String> {
        self.state.cards.get(&card_id).map(|c| c.archetypes.clone()).unwrap_or_default()
    }
    fn effect_card_id(&self) -> u32 { self.effect_card_id }
    fn effect_player(&self) -> u8 { self.effect_player }
    fn event_categories(&self) -> u32 { self.event_categories }

    // ── Card movement ────────────────────────────────────────
    fn draw(&mut self, player: u8, count: u32) -> u32 {
        self.record("draw", format!("player={} count={}", player, count));
        let p = &mut self.state.players[player as usize];
        let mut drawn = 0u32;
        for _ in 0..count {
            if let Some(card) = p.deck.pop() {
                p.hand.push(card);
                drawn += 1;
            }
        }
        drawn
    }
    fn destroy(&mut self, card_ids: &[u32]) -> u32 {
        self.record("destroy", format!("ids={:?}", card_ids));
        let mut destroyed = 0u32;
        for id in card_ids {
            for player in 0..2 {
                let p = &mut self.state.players[player];
                let before = p.field_monsters.len() + p.field_spells.len();
                p.field_monsters.retain(|x| x != id);
                p.field_spells.retain(|x| x != id);
                let after = p.field_monsters.len() + p.field_spells.len();
                if after < before {
                    p.graveyard.push(*id);
                    destroyed += 1;
                }
            }
        }
        destroyed
    }
    fn send_to_grave(&mut self, card_ids: &[u32]) -> u32 {
        self.record("send_to_grave", format!("ids={:?}", card_ids));
        let mut moved = 0u32;
        for id in card_ids {
            for player in 0..2 {
                let p = &mut self.state.players[player];
                if let Some(pos) = p.hand.iter().position(|x| x == id) {
                    p.hand.remove(pos);
                    p.graveyard.push(*id);
                    moved += 1;
                    break;
                }
            }
        }
        moved
    }
    fn send_to_hand(&mut self, card_ids: &[u32]) -> u32 {
        self.record("send_to_hand", format!("ids={:?}", card_ids));
        card_ids.len() as u32
    }
    fn banish(&mut self, card_ids: &[u32]) -> u32 {
        self.record("banish", format!("ids={:?}", card_ids));
        card_ids.len() as u32
    }
    fn discard(&mut self, card_ids: &[u32]) -> u32 {
        self.record("discard", format!("ids={:?}", card_ids));
        let mut moved = 0u32;
        for id in card_ids {
            for player in 0..2 {
                let p = &mut self.state.players[player];
                if let Some(pos) = p.hand.iter().position(|x| x == id) {
                    p.hand.remove(pos);
                    p.graveyard.push(*id);
                    moved += 1;
                    break;
                }
            }
        }
        moved
    }
    fn special_summon(&mut self, card_id: u32, player: u8, position: u32) -> bool {
        self.record("special_summon",
            format!("card={} player={} position={}", card_id, player, position));
        self.state.players[player as usize].field_monsters.push(card_id);
        self.state.last_selection = vec![card_id];
        true
    }

    // ── LP ───────────────────────────────────────────────────
    fn damage(&mut self, player: u8, amount: i32, damage_type: DamageType) -> bool {
        self.record("damage",
            format!("player={} amount={} type={:?}", player, amount, damage_type));
        self.state.players[player as usize].lp -= amount;
        true
    }
    fn recover(&mut self, player: u8, amount: i32) -> bool {
        self.record("recover", format!("player={} amount={}", player, amount));
        self.state.players[player as usize].lp += amount;
        true
    }

    // ── Selection ────────────────────────────────────────────
    fn select_cards(&mut self, player: u8, candidates: &[u32], min: usize, max: usize) -> Vec<u32> {
        self.record("select_cards",
            format!("player={} candidates={:?} min={} max={}", player, candidates, min, max));
        // Deterministic: pick the first `min` candidates (or `max` if smaller).
        let n = max.min(candidates.len()).max(min.min(candidates.len()));
        let picked: Vec<u32> = candidates.iter().take(n).copied().collect();
        self.state.last_selection = picked.clone();
        picked
    }
    fn select_option(&mut self, player: u8, options: &[String]) -> usize {
        self.record("select_option",
            format!("player={} options={:?}", player, options));
        0
    }

    // ── Metadata ─────────────────────────────────────────────
    fn set_targets(&mut self, card_ids: &[u32]) {
        self.record("set_targets", format!("ids={:?}", card_ids));
        self.state.last_selection = card_ids.to_vec();
    }

    // ── Negation ─────────────────────────────────────────────
    fn negate_activation(&mut self) -> bool {
        self.record("negate_activation", "");
        true
    }
    fn negate_effect(&mut self) -> bool {
        self.record("negate_effect", "");
        true
    }

    // ── More movement ────────────────────────────────────────
    fn send_to_deck(&mut self, card_ids: &[u32], top: bool) -> u32 {
        self.record("send_to_deck", format!("ids={:?} top={}", card_ids, top));
        card_ids.len() as u32
    }
    fn return_to_hand(&mut self, card_ids: &[u32]) -> u32 {
        self.record("return_to_hand", format!("ids={:?}", card_ids));
        card_ids.len() as u32
    }
    fn return_to_owner(&mut self, card_ids: &[u32]) -> u32 {
        self.record("return_to_owner", format!("ids={:?}", card_ids));
        card_ids.len() as u32
    }
    fn tribute(&mut self, card_ids: &[u32]) -> u32 {
        self.record("tribute", format!("ids={:?}", card_ids));
        card_ids.len() as u32
    }
    fn shuffle_deck(&mut self, player: u8) {
        self.record("shuffle_deck", format!("player={}", player));
    }

    // ── Stat mods ────────────────────────────────────────────
    fn modify_atk(&mut self, card_id: u32, delta: i32) {
        self.record("modify_atk", format!("card={} delta={}", card_id, delta));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.atk += delta; }
    }
    fn modify_def(&mut self, card_id: u32, delta: i32) {
        self.record("modify_def", format!("card={} delta={}", card_id, delta));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.def += delta; }
    }
    fn set_atk(&mut self, card_id: u32, value: i32) {
        self.record("set_atk", format!("card={} value={}", card_id, value));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.atk = value; }
    }
    fn set_def(&mut self, card_id: u32, value: i32) {
        self.record("set_def", format!("card={} value={}", card_id, value));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.def = value; }
    }

    // ── Battle ───────────────────────────────────────────────
    fn change_position(&mut self, card_id: u32) {
        self.record("change_position", format!("card={}", card_id));
    }

    // ── Xyz materials ────────────────────────────────────────
    fn detach_material(&mut self, card_id: u32, count: u32) -> u32 {
        self.record("detach_material", format!("card={} count={}", card_id, count));
        count
    }
    fn attach_material(&mut self, material_id: u32, target_id: u32) {
        self.record("attach_material", format!("material={} target={}", material_id, target_id));
    }

    // ── Counters ─────────────────────────────────────────────
    fn place_counter(&mut self, card_id: u32, name: &str, count: u32) {
        self.record("place_counter", format!("card={} name={} count={}", card_id, name, count));
        let entry = self.state.counters.entry((card_id, name.to_string())).or_insert(0);
        *entry += count;
    }
    fn remove_counter(&mut self, card_id: u32, name: &str, count: u32) {
        self.record("remove_counter", format!("card={} name={} count={}", card_id, name, count));
        let entry = self.state.counters.entry((card_id, name.to_string())).or_insert(0);
        *entry = entry.saturating_sub(count);
    }
    fn get_counter_count(&self, card_id: u32, counter_name: &str) -> u32 {
        self.state.counters.get(&(card_id, counter_name.to_string())).copied().unwrap_or(0)
    }

    // ── Deck operations ──────────────────────────────────────
    fn mill(&mut self, player: u8, count: u32) -> u32 {
        self.record("mill", format!("player={} count={}", player, count));
        let p = &mut self.state.players[player as usize];
        let mut milled = 0u32;
        for _ in 0..count {
            if let Some(card) = p.deck.pop() {
                p.graveyard.push(card);
                milled += 1;
            }
        }
        milled
    }
    fn excavate(&mut self, player: u8, count: u32) -> Vec<u32> {
        self.record("excavate", format!("player={} count={}", player, count));
        let deck = &self.state.players[player as usize].deck;
        // Return the top N cards (end of vec) without removing them.
        let n = (count as usize).min(deck.len());
        deck[deck.len() - n..].to_vec()
    }

    // ── Phase 2: custom events ───────────────────────────────
    fn raise_custom_event(&mut self, name: &str, cards: &[u32]) {
        self.record("raise_custom_event", format!("name={:?} cards={:?}", name, cards));
    }

    // ── Phase 3: confirm ─────────────────────────────────────
    fn confirm_cards(&mut self, owner: u8, audience: u8, cards: &[u32]) {
        self.record("confirm_cards",
            format!("owner={} audience={} cards={:?}", owner, audience, cards));
    }

    // ── Phase 3: announce ────────────────────────────────────
    fn announce(&mut self, player: u8, kind: u8, filter_mask: u32) -> u32 {
        self.record("announce",
            format!("player={} kind={} mask=0x{:x}", player, kind, filter_mask));
        // Return a fixed token for tests to assert against.
        0xA55E_2700
    }
    fn get_announcement(&self, _token: u32) -> u32 { 0 }

    // ── Phase 1A: flags ──────────────────────────────────────
    fn register_flag(&mut self, card_id: u32, name: &str, survives: u32, resets: u32) {
        self.record("register_flag",
            format!("card={} name={:?} survives=0x{:x} resets=0x{:x}",
                    card_id, name, survives, resets));
        self.state.flags.insert((card_id, name.to_string()), ());
    }
    fn clear_flag(&mut self, card_id: u32, name: &str) {
        self.record("clear_flag", format!("card={} name={:?}", card_id, name));
        self.state.flags.remove(&(card_id, name.to_string()));
    }
    fn has_flag(&self, card_id: u32, name: &str) -> bool {
        self.state.flags.contains_key(&(card_id, name.to_string()))
    }

    // ── Phase 1A: history queries (stubs) ────────────────────
    fn previous_location(&self, _card_id: u32) -> u32 { 0 }
    fn previous_position(&self, _card_id: u32) -> u32 { 0 }
    fn sent_by_reason(&self, _card_id: u32) -> u32 { 0 }

    // ── Phase 1D: bindings ───────────────────────────────────
    fn set_binding(&mut self, name: &str, card_id: u32) {
        self.record("set_binding", format!("name={:?} card={}", name, card_id));
        self.state.bindings.insert(name.to_string(), vec![card_id]);
    }
    fn bind_last_selection(&mut self, name: &str) {
        self.record("bind_last_selection", format!("name={:?}", name));
        let last = self.state.last_selection.clone();
        self.state.bindings.insert(name.to_string(), last);
    }
    fn get_binding_card(&self, name: &str) -> Option<u32> {
        self.state.bindings.get(name).and_then(|cs| cs.first().copied())
    }
    fn get_binding_field(&self, name: &str, field: &str) -> i32 {
        let Some(card_id) = self.get_binding_card(name) else { return 0 };
        let Some(card) = self.state.cards.get(&card_id) else { return 0 };
        match field {
            "atk"   => card.atk,
            "def"   => card.def,
            "level" => card.level as i32,
            "code"  => card.id as i32,
            _ => 0,
        }
    }

    // ── Phase 1E: change code ────────────────────────────────
    fn change_card_code(&mut self, card_id: u32, code: u32, duration_mask: u32) {
        self.record("change_card_code",
            format!("card={} code={} duration=0x{:x}", card_id, code, duration_mask));
    }

    // ── Info / RNG ───────────────────────────────────────────
    fn reveal(&mut self, card_ids: &[u32]) {
        self.record("reveal", format!("ids={:?}", card_ids));
    }
    fn look_at(&mut self, player: u8, card_ids: &[u32]) {
        self.record("look_at", format!("player={} ids={:?}", player, card_ids));
    }
    fn coin_flip(&mut self, player: u8) -> bool {
        self.record("coin_flip", format!("player={}", player));
        true // deterministic: always heads
    }
    fn dice_roll(&mut self, player: u8) -> u32 {
        self.record("dice_roll", format!("player={}", player));
        1 // deterministic: always roll 1
    }

    // ── Summon / Set ─────────────────────────────────────────
    fn normal_summon(&mut self, card_id: u32, player: u8) -> bool {
        self.record("normal_summon", format!("card={} player={}", card_id, player));
        self.state.players[player as usize].field_monsters.push(card_id);
        true
    }
    fn set_card(&mut self, card_id: u32, player: u8) -> bool {
        self.record("set_card", format!("card={} player={}", card_id, player));
        self.state.players[player as usize].field_spells.push(card_id);
        true
    }

    // ── Extra Deck Summons ────────────────────────────────────
    fn ritual_summon(&mut self, card_id: u32, player: u8, material_ids: &[u32]) -> bool {
        self.record("ritual_summon", format!("card={} player={} materials={:?}", card_id, player, material_ids));
        self.state.players[player as usize].field_monsters.push(card_id);
        true
    }
    fn fusion_summon(&mut self, card_id: u32, player: u8, material_ids: &[u32]) -> bool {
        self.record("fusion_summon", format!("card={} player={} materials={:?}", card_id, player, material_ids));
        self.state.players[player as usize].field_monsters.push(card_id);
        true
    }
    fn synchro_summon(&mut self, card_id: u32, player: u8, material_ids: &[u32]) -> bool {
        self.record("synchro_summon", format!("card={} player={} materials={:?}", card_id, player, material_ids));
        self.state.players[player as usize].field_monsters.push(card_id);
        true
    }
    fn xyz_summon(&mut self, card_id: u32, player: u8, material_ids: &[u32]) -> bool {
        self.record("xyz_summon", format!("card={} player={} materials={:?}", card_id, player, material_ids));
        self.state.players[player as usize].field_monsters.push(card_id);
        true
    }

    // ── Equip ─────────────────────────────────────────────────
    fn equip_card(&mut self, equip_id: u32, target_id: u32) {
        self.record("equip_card", format!("equip={} target={}", equip_id, target_id));
    }

    // ── Swap ──────────────────────────────────────────────────
    fn swap_control(&mut self, card_a: u32, card_b: u32) {
        self.record("swap_control", format!("a={} b={}", card_a, card_b));
    }
    fn swap_stats(&mut self, card_id: u32) {
        self.record("swap_stats", format!("card={}", card_id));
    }

    // ── Grant ─────────────────────────────────────────────────
    fn register_grant(&mut self, card_id: u32, grant_code: u32, duration: u32) {
        self.record("register_grant",
            format!("card={} grant=0x{:x} duration={}", card_id, grant_code, duration));
    }

    // ── Card Identity Changes ────────────────────────────────
    fn change_level(&mut self, card_id: u32, level: u32) {
        self.record("change_level", format!("card={} level={}", card_id, level));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.level = level; }
    }
    fn change_attribute(&mut self, card_id: u32, attribute: u32) {
        self.record("change_attribute", format!("card={} attribute=0x{:x}", card_id, attribute));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.attribute = attribute as u64; }
    }
    fn change_race(&mut self, card_id: u32, race: u32) {
        self.record("change_race", format!("card={} race=0x{:x}", card_id, race));
        if let Some(c) = self.state.cards.get_mut(&card_id) { c.race = race as u64; }
    }
    fn change_name(&mut self, card_id: u32, name: &str, duration: u32) {
        self.record("change_name", format!("card={} name={:?} duration={}", card_id, name, duration));
    }
    fn set_scale(&mut self, card_id: u32, scale: u32) {
        self.record("set_scale", format!("card={} scale={}", card_id, scale));
    }
}

// ── DuelScenario ─────────────────────────────────────────────

/// Fluent builder for setting up MockRuntime state.
///
/// Usage:
///   let mut rt = DuelScenario::new()
///       .player(0).hand([55144522])              // Pot of Greed
///       .player(0).deck([46986414, 46986414])    // 2 Dark Magicians
///       .build();
pub struct DuelScenario {
    rt: MockRuntime,
    /// Currently selected player for chained `.hand()` / `.deck()` calls.
    current_player: u8,
}

impl DuelScenario {
    pub fn new() -> Self {
        Self { rt: MockRuntime::new(), current_player: 0 }
    }

    /// Switch the "current" player so subsequent zone setters apply to them.
    pub fn player(mut self, p: u8) -> Self {
        self.current_player = p;
        self
    }

    /// Set the LP of the currently selected player.
    pub fn lp(mut self, amount: i32) -> Self {
        self.rt.state.players[self.current_player as usize].lp = amount;
        self
    }

    /// Set the current player's hand to the given card IDs.
    pub fn hand<I: IntoIterator<Item = u32>>(mut self, ids: I) -> Self {
        self.rt.state.players[self.current_player as usize].hand = ids.into_iter().collect();
        self
    }

    /// Set the current player's deck (top of deck = LAST element).
    pub fn deck<I: IntoIterator<Item = u32>>(mut self, ids: I) -> Self {
        self.rt.state.players[self.current_player as usize].deck = ids.into_iter().collect();
        self
    }

    /// Set the current player's graveyard.
    pub fn graveyard<I: IntoIterator<Item = u32>>(mut self, ids: I) -> Self {
        self.rt.state.players[self.current_player as usize].graveyard = ids.into_iter().collect();
        self
    }

    /// Place monsters on the current player's field.
    pub fn monsters<I: IntoIterator<Item = u32>>(mut self, ids: I) -> Self {
        self.rt.state.players[self.current_player as usize].field_monsters = ids.into_iter().collect();
        self
    }

    /// Reset a player's state to a clean PlayerState (8000 LP, empty zones).
    pub fn reset(mut self, p: u8) -> Self {
        self.rt.state.players[p as usize] = PlayerState::fresh();
        self
    }

    /// Register a card snapshot so MockRuntime can answer stat queries.
    pub fn card(mut self, snap: CardSnapshot) -> Self {
        self.rt.state.add_card(snap);
        self
    }

    /// Register multiple cards in bulk.
    pub fn cards<I: IntoIterator<Item = CardSnapshot>>(mut self, snaps: I) -> Self {
        for snap in snaps {
            self.rt.state.add_card(snap);
        }
        self
    }

    /// Set the effect activator/owner. Defaults to player 0 with card 0.
    pub fn activated_by(mut self, player: u8, card_id: u32) -> Self {
        self.rt.effect_player = player;
        self.rt.effect_card_id = card_id;
        self
    }

    /// Set the chain-link event categories (used by hand traps).
    pub fn event_categories(mut self, mask: u32) -> Self {
        self.rt.event_categories = mask;
        self
    }

    /// Finish building and return the MockRuntime.
    pub fn build(self) -> MockRuntime {
        self.rt
    }
}

impl Default for DuelScenario {
    fn default() -> Self { Self::new() }
}

// ── Tests for the mock itself ────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fresh_runtime_has_8000_lp() {
        let rt = MockRuntime::fresh();
        assert_eq!(rt.get_lp(0), 8000);
        assert_eq!(rt.get_lp(1), 8000);
    }

    #[test]
    fn draw_records_and_moves_cards() {
        let mut rt = MockRuntime::fresh();
        rt.state.players[0].deck = vec![1, 2, 3];
        let drawn = rt.draw(0, 2);
        assert_eq!(drawn, 2);
        assert_eq!(rt.get_hand_count(0), 2);
        assert_eq!(rt.get_deck_count(0), 1);
        assert_eq!(rt.call_count("draw"), 1);
        assert!(rt.was_called_with("draw", "count=2"));
    }

    #[test]
    fn damage_reduces_lp_and_records() {
        let mut rt = MockRuntime::fresh();
        rt.damage(1, 1500, DamageType::Effect);
        assert_eq!(rt.get_lp(1), 6500);
        assert_eq!(rt.call_count("damage"), 1);
    }

    #[test]
    fn flag_set_clear_query() {
        let mut rt = MockRuntime::fresh();
        assert!(!rt.has_flag(42, "marked"));
        rt.register_flag(42, "marked", 0, 0);
        assert!(rt.has_flag(42, "marked"));
        rt.clear_flag(42, "marked");
        assert!(!rt.has_flag(42, "marked"));
    }
}
