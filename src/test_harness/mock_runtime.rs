// ============================================================
// MockRuntime
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

use crate::ast::{CardFilter, Stat};
use crate::compiler::callback_gen::DuelScriptRuntime;

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
/// the runtime's stat queries and filter checks.
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
}

impl CardSnapshot {
    pub fn monster(id: u32, name: &str, atk: i32, def: i32, level: u32) -> Self {
        Self {
            id, name: name.to_string(), atk, def, level,
            is_monster: true, is_spell: false, is_trap: false,
        }
    }

    pub fn spell(id: u32, name: &str) -> Self {
        Self {
            id, name: name.to_string(), atk: 0, def: 0, level: 0,
            is_monster: false, is_spell: true, is_trap: false,
        }
    }

    pub fn trap(id: u32, name: &str) -> Self {
        Self {
            id, name: name.to_string(), atk: 0, def: 0, level: 0,
            is_monster: false, is_spell: false, is_trap: true,
        }
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
        // Constants from type_mapper: HAND=0x2, MZONE=0x4, SZONE=0x8,
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
            Stat::Rank  => card.level as i32,
        }
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
    fn damage(&mut self, player: u8, amount: i32) -> bool {
        self.record("damage", format!("player={} amount={}", player, amount));
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
    fn set_operation_info(&mut self, category: u32, count: u32) {
        self.record("set_operation_info", format!("category=0x{:x} count={}", category, count));
    }
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
    fn negate_attack(&mut self) -> bool {
        self.record("negate_attack", "");
        true
    }
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
    }
    fn remove_counter(&mut self, card_id: u32, name: &str, count: u32) {
        self.record("remove_counter", format!("card={} name={} count={}", card_id, name, count));
    }

    // ── Counting ─────────────────────────────────────────────
    fn count_matching(&self, player: u8, _location: u32, _filter: &CardFilter) -> usize {
        self.get_field_cards(player, 0).len()
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
        rt.damage(1, 1500);
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
