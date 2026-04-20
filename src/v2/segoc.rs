// ============================================================
// DuelScript v2 SEGOC — Simultaneous Effects Go On Chain
//
// When multiple trigger effects meet their activation condition
// from the same game event, the game collects them all and
// builds a chain in the correct turn-player-first order.
//
// Rule (Yu-Gi-Oh TCG):
//   1. Turn player's optional triggers are offered first; the
//      turn player chooses the order among their own effects.
//   2. Non-turn player's optional triggers are offered next;
//      the non-turn player chooses the order among their own.
//   3. Mandatory triggers follow the same split but are placed
//      before optional triggers of the same player.
//
// This module is self-contained inside duelscript/. It holds no
// cross-folder imports. The engine feeds it a list of pending
// triggers and receives an ordered chain back.
// ============================================================

// ── Pending trigger entry ─────────────────────────────────────

/// A single trigger effect that is waiting to enter a SEGOC chain.
///
/// `controller` is the player index (0 or 1) who controls the card
/// that owns this trigger. `is_mandatory` indicates whether the
/// effect must be activated (the player has no choice) or is optional.
#[derive(Debug, Clone)]
pub struct PendingTrigger {
    /// The card that owns this effect (engine card id).
    pub card_id: u32,
    /// Which player controls the card (0 = turn player, 1 = opponent,
    /// or vice versa — caller passes the raw controller index; the
    /// queue resolves ordering relative to `turn_player`).
    pub controller: u8,
    /// Whether the effect is mandatory (TRIGGER_F) or optional (TRIGGER_O).
    pub is_mandatory: bool,
    /// Display label for the effect (used in tests and UI).
    pub label: String,
    /// The event code this trigger responds to (same as `CompiledEffectV2::code`).
    pub event_code: u32,
}

impl PendingTrigger {
    /// Convenience constructor.
    pub fn new(
        card_id: u32,
        controller: u8,
        is_mandatory: bool,
        label: impl Into<String>,
        event_code: u32,
    ) -> Self {
        Self {
            card_id,
            controller,
            is_mandatory,
            label: label.into(),
            event_code,
        }
    }
}

// ── SEGOC Queue ───────────────────────────────────────────────

/// Collects pending trigger effects triggered by the same game event
/// and orders them into a chain according to SEGOC rules.
///
/// Usage:
/// ```no_run
/// # use duelscript::v2::segoc::{SegocQueue, PendingTrigger};
/// let mut queue = SegocQueue::new(0 /* turn player */);
/// queue.push(PendingTrigger::new(101, 0, true,  "Card A mandatory", 1102));
/// queue.push(PendingTrigger::new(202, 1, false, "Card B optional",  1102));
/// queue.push(PendingTrigger::new(303, 0, false, "Card C optional",  1102));
/// let chain = queue.build_chain();
/// // chain[0] → turn-player's mandatory trigger (Card A)
/// // chain[1] → turn-player's optional trigger (Card C)
/// // chain[2] → opponent's optional trigger (Card B)
/// ```
pub struct SegocQueue {
    /// The player index (0 or 1) whose turn it currently is.
    turn_player: u8,
    /// All triggers added to this queue.
    pending: Vec<PendingTrigger>,
}

impl SegocQueue {
    /// Create a new queue. `turn_player` is 0 or 1.
    pub fn new(turn_player: u8) -> Self {
        Self { turn_player, pending: Vec::new() }
    }

    /// Add a trigger to the queue. Multiple triggers from the same
    /// game event should all be pushed before calling `build_chain`.
    pub fn push(&mut self, trigger: PendingTrigger) {
        self.pending.push(trigger);
    }

    /// Returns `true` if there are no pending triggers.
    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    /// Returns the number of pending triggers.
    pub fn len(&self) -> usize {
        self.pending.len()
    }

    /// Build the SEGOC chain from all pending triggers.
    ///
    /// Ordering rules (applied in priority order):
    ///   1. Turn-player mandatory triggers  (tp_mandatory)
    ///   2. Turn-player optional triggers   (tp_optional)
    ///   3. Non-turn-player mandatory triggers (ntp_mandatory)
    ///   4. Non-turn-player optional triggers  (ntp_optional)
    ///
    /// Within each group the insertion order is preserved. In a real
    /// game the active player chooses the order within their own
    /// group; here we preserve declaration order as a deterministic
    /// default. The engine can override ordering within a group by
    /// re-sorting the returned sub-slices before consuming them.
    ///
    /// The returned `Vec` is ordered lowest chain-link to highest:
    /// index 0 resolves last (LIFO chain), index last resolves first.
    /// Equivalently: the last item in the returned Vec is chain link 1
    /// (the first to resolve).
    ///
    /// Consumes the queue.
    pub fn build_chain(self) -> Vec<PendingTrigger> {
        let tp = self.turn_player;

        // Partition into four buckets maintaining stable insertion order.
        let mut tp_mandatory: Vec<PendingTrigger>  = Vec::new();
        let mut tp_optional:  Vec<PendingTrigger>  = Vec::new();
        let mut ntp_mandatory: Vec<PendingTrigger> = Vec::new();
        let mut ntp_optional:  Vec<PendingTrigger> = Vec::new();

        for trigger in self.pending {
            let is_tp = trigger.controller == tp;
            match (is_tp, trigger.is_mandatory) {
                (true,  true)  => tp_mandatory.push(trigger),
                (true,  false) => tp_optional.push(trigger),
                (false, true)  => ntp_mandatory.push(trigger),
                (false, false) => ntp_optional.push(trigger),
            }
        }

        // Concatenate in SEGOC order: tp_mandatory, tp_optional,
        // ntp_mandatory, ntp_optional.
        let mut chain = Vec::with_capacity(
            tp_mandatory.len() + tp_optional.len()
            + ntp_mandatory.len() + ntp_optional.len()
        );
        chain.extend(tp_mandatory);
        chain.extend(tp_optional);
        chain.extend(ntp_mandatory);
        chain.extend(ntp_optional);
        chain
    }

    /// Like `build_chain` but does not consume the queue. The pending
    /// list is cloned. Useful for inspection in tests.
    pub fn peek_chain(&self) -> Vec<PendingTrigger> {
        let clone = Self {
            turn_player: self.turn_player,
            pending: self.pending.clone(),
        };
        clone.build_chain()
    }
}

// ── Helper: collect triggers from compiled effects ────────────

use crate::v2::compiler::CompiledEffectV2;
use crate::v2::constants as tm;

/// Given a slice of compiled effects and the current game event code,
/// returns a `SegocQueue` pre-populated with every effect whose event
/// code matches the triggering event.
///
/// `controller_for` is a closure that maps a card_id to its
/// controlling player (0 or 1). The engine provides this.
///
/// Only effects with `EFFECT_TYPE_TRIGGER_O` or `EFFECT_TYPE_TRIGGER_F`
/// bits set are considered; ignition and activated effects are ignored.
pub fn collect_simultaneous_triggers<F>(
    effects: &[(&CompiledEffectV2, u32)], // (effect, card_id)
    event_code: u32,
    turn_player: u8,
    controller_for: F,
) -> SegocQueue
where
    F: Fn(u32) -> u8,
{
    let mut queue = SegocQueue::new(turn_player);

    for (effect, card_id) in effects {
        let is_trigger = effect.effect_type
            & (tm::EFFECT_TYPE_TRIGGER_O | tm::EFFECT_TYPE_TRIGGER_F) != 0;
        if !is_trigger {
            continue;
        }
        if effect.code != event_code {
            continue;
        }
        let is_mandatory = effect.effect_type & tm::EFFECT_TYPE_TRIGGER_F != 0;
        let controller = controller_for(*card_id);
        queue.push(PendingTrigger::new(
            *card_id,
            controller,
            is_mandatory,
            &effect.label,
            event_code,
        ));
    }

    queue
}

// ── Tests ─────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper: build a PendingTrigger quickly ─────────────────
    fn pt(card_id: u32, controller: u8, mandatory: bool, label: &str) -> PendingTrigger {
        PendingTrigger::new(card_id, controller, mandatory, label, 1102)
    }

    // ── Basic ordering ─────────────────────────────────────────

    #[test]
    fn empty_queue_builds_empty_chain() {
        let queue = SegocQueue::new(0);
        assert!(queue.build_chain().is_empty());
    }

    #[test]
    fn single_trigger_is_unchanged() {
        let mut queue = SegocQueue::new(0);
        queue.push(pt(1, 0, false, "Solo"));
        let chain = queue.build_chain();
        assert_eq!(chain.len(), 1);
        assert_eq!(chain[0].label, "Solo");
    }

    /// Core SEGOC scenario: two triggers from the same event, one for
    /// each player. Turn player's effect must come first in the chain.
    #[test]
    fn simultaneous_triggers_turn_player_first() {
        // Turn player = 0. Two optional triggers: one from player 0, one from player 1.
        let mut queue = SegocQueue::new(0);
        // Pushed in reverse order to verify sorting, not insertion order, governs output.
        queue.push(pt(200, 1, false, "Opponent Trigger"));
        queue.push(pt(100, 0, false, "Turn Player Trigger"));

        let chain = queue.build_chain();
        assert_eq!(chain.len(), 2);
        // Turn-player's trigger must be index 0.
        assert_eq!(chain[0].card_id, 100, "expected turn-player trigger first");
        assert_eq!(chain[0].label, "Turn Player Trigger");
        assert_eq!(chain[1].card_id, 200, "expected opponent trigger second");
        assert_eq!(chain[1].label, "Opponent Trigger");
    }

    #[test]
    fn mandatory_before_optional_same_player() {
        let mut queue = SegocQueue::new(0);
        // Turn-player optional first, then mandatory (insertion order).
        queue.push(pt(10, 0, false, "TP Optional"));
        queue.push(pt(20, 0, true,  "TP Mandatory"));
        let chain = queue.build_chain();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].label, "TP Mandatory");
        assert_eq!(chain[1].label, "TP Optional");
    }

    #[test]
    fn full_four_bucket_ordering() {
        // Verify the canonical order: tp_mandatory, tp_optional, ntp_mandatory, ntp_optional.
        let mut queue = SegocQueue::new(0);
        queue.push(pt(4, 1, false, "NTP Optional"));
        queue.push(pt(3, 1, true,  "NTP Mandatory"));
        queue.push(pt(2, 0, false, "TP Optional"));
        queue.push(pt(1, 0, true,  "TP Mandatory"));

        let chain = queue.build_chain();
        assert_eq!(chain.len(), 4);
        assert_eq!(chain[0].label, "TP Mandatory",  "slot 0 = tp mandatory");
        assert_eq!(chain[1].label, "TP Optional",   "slot 1 = tp optional");
        assert_eq!(chain[2].label, "NTP Mandatory", "slot 2 = ntp mandatory");
        assert_eq!(chain[3].label, "NTP Optional",  "slot 3 = ntp optional");
    }

    #[test]
    fn insertion_order_preserved_within_group() {
        // Both triggers belong to the turn player and are optional.
        // Insertion order must be preserved within the group.
        let mut queue = SegocQueue::new(0);
        queue.push(pt(1, 0, false, "First"));
        queue.push(pt(2, 0, false, "Second"));
        queue.push(pt(3, 0, false, "Third"));
        let chain = queue.build_chain();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].label, "First");
        assert_eq!(chain[1].label, "Second");
        assert_eq!(chain[2].label, "Third");
    }

    #[test]
    fn peek_chain_does_not_consume() {
        let mut queue = SegocQueue::new(0);
        queue.push(pt(1, 0, false, "A"));
        queue.push(pt(2, 1, false, "B"));
        let _ = queue.peek_chain();
        // Queue should still be intact.
        assert_eq!(queue.len(), 2);
    }

    // ── Integration: collect_simultaneous_triggers ─────────────

    /// Shows two simultaneously triggered effects from the same event being
    /// collected and ordered turn-player-first. This is the representative
    /// test demonstrating SEGOC support (acceptance criterion 4).
    #[test]
    fn segoc_two_triggers_same_event_correct_order() {
        use crate::v2::parser::parse_v2;
        use crate::v2::compiler::compile_card_v2;

        // Card A: turn player (player 0) — optional trigger on special summon.
        let card_a_src = r#"
card "Segoc Card Alpha" {
    id: 99101
    type: Effect Monster
    attribute: LIGHT
    race: Warrior
    level: 4
    atk: 1800
    def: 1200

    effect "Alpha Trigger" {
        speed: 1
        trigger: special_summoned
        timing: if
        resolve {
            draw 1
        }
    }
}
"#;
        // Card B: opponent (player 1) — optional trigger on special summon.
        let card_b_src = r#"
card "Segoc Card Beta" {
    id: 99102
    type: Effect Monster
    attribute: DARK
    race: Spellcaster
    level: 4
    atk: 1600
    def: 1400

    effect "Beta Trigger" {
        speed: 1
        trigger: special_summoned
        timing: if
        resolve {
            gain_lp 500
        }
    }
}
"#;
        let file_a = parse_v2(card_a_src).unwrap();
        let compiled_a = compile_card_v2(&file_a.cards[0]);
        let file_b = parse_v2(card_b_src).unwrap();
        let compiled_b = compile_card_v2(&file_b.cards[0]);

        // Both cards have exactly one trigger effect.
        let effect_a = &compiled_a.effects[0];
        let effect_b = &compiled_b.effects[0];
        assert_eq!(effect_a.label, "Alpha Trigger");
        assert_eq!(effect_b.label, "Beta Trigger");

        // The event both respond to is EVENT_SPSUMMON_SUCCESS (1102).
        let event = crate::v2::constants::EVENT_SPSUMMON_SUCCESS;

        // Build SEGOC queue. Player 0 is the turn player.
        // Alpha belongs to player 0, Beta to player 1.
        let effects: Vec<(&CompiledEffectV2, u32)> = vec![
            (effect_a, 99101u32),
            (effect_b, 99102u32),
        ];
        let controller_for = |card_id: u32| -> u8 {
            if card_id == 99101 { 0 } else { 1 }
        };

        let queue = collect_simultaneous_triggers(&effects, event, 0, controller_for);
        assert_eq!(queue.len(), 2, "both triggers must be collected");

        let chain = queue.build_chain();
        assert_eq!(chain.len(), 2);

        // Turn player's trigger must come first.
        assert_eq!(chain[0].card_id, 99101, "Alpha (tp) must be chain[0]");
        assert_eq!(chain[0].label, "Alpha Trigger");
        assert_eq!(chain[1].card_id, 99102, "Beta (opponent) must be chain[1]");
        assert_eq!(chain[1].label, "Beta Trigger");

        // Neither is mandatory — both should be optional triggers.
        assert!(!chain[0].is_mandatory);
        assert!(!chain[1].is_mandatory);
    }

    /// Verify that a mandatory trigger from the opponent still goes before
    /// the opponent's own optional trigger but after all turn-player effects.
    #[test]
    fn segoc_mandatory_ntp_before_optional_ntp() {
        let mut queue = SegocQueue::new(0);
        queue.push(pt(10, 0, false, "TP Optional A"));
        queue.push(pt(20, 1, false, "NTP Optional"));
        queue.push(pt(30, 1, true,  "NTP Mandatory"));
        let chain = queue.build_chain();
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].label, "TP Optional A");
        assert_eq!(chain[1].label, "NTP Mandatory");
        assert_eq!(chain[2].label, "NTP Optional");
    }

    /// Verify that when turn_player = 1, player 1's effects go first.
    #[test]
    fn segoc_turn_player_1_goes_first() {
        let mut queue = SegocQueue::new(1); // player 1 is the turn player
        queue.push(pt(10, 0, false, "Player 0 Effect"));
        queue.push(pt(20, 1, false, "Player 1 Effect"));
        let chain = queue.build_chain();
        assert_eq!(chain.len(), 2);
        assert_eq!(chain[0].card_id, 20, "player 1 (turn player) must go first");
        assert_eq!(chain[1].card_id, 10);
    }
}
