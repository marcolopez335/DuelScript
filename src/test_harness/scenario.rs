// ============================================================
// DuelScenario — fluent builder for setting up MockRuntime state.
//
// Usage:
//   let mut rt = DuelScenario::new()
//       .player(0).hand([55144522])              // Pot of Greed
//       .player(0).deck([46986414, 46986414])    // 2 Dark Magicians
//       .build();
//
// This is purely a convenience over poking at MockState directly.
// Tests can mix-and-match: use the builder for setup and then
// manipulate `rt.state` for edge cases.
// ============================================================

use super::mock_runtime::{CardSnapshot, MockRuntime, PlayerState};

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
