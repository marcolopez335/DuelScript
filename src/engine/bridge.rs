// ============================================================
// DuelScript Engine Bridge — engine/bridge.rs
//
// The contract between DuelScript and any consuming engine.
// Implement DuelScriptEngine in your Rust engine to get
// typed, validated card data instead of Lua scripts.
//
// DuelScript declares WHAT a card does.
// Your engine decides HOW to execute it.
// ============================================================

use std::sync::Arc;
use crate::ast::*;

// ── Core Engine Trait ─────────────────────────────────────────

/// Implement this trait in your engine to consume DuelScript cards.
///
/// DuelScript guarantees that all data passed through this interface
/// has been parsed and validated — your engine never receives an
/// illegal card definition.
///
/// # Example
/// ```rust,no_run,ignore
/// struct MyEngine { /* ... */ }
///
/// impl DuelScriptEngine for MyEngine {
///     type Context = MyGameContext;
///
///     fn check_trigger(&self, trigger: &TriggerExpr, event: &GameEvent, ctx: &Self::Context) -> bool {
///         match trigger {
///             TriggerExpr::OpponentActivates(actions) => {
///                 actions.iter().any(|a| ctx.current_event_matches(a))
///             }
///             TriggerExpr::WhenSummoned(_) => ctx.just_summoned(),
///             _ => false,
///         }
///     }
///     // ... implement remaining methods
/// }
/// ```
pub trait DuelScriptEngine {
    /// The engine's game context type — passed to every call so the
    /// engine has access to full game state when resolving effects.
    type Context;

    // ── Trigger Resolution ────────────────────────────────────

    /// Check whether a trigger condition is currently satisfied.
    /// Called by the engine whenever a game event occurs that might
    /// activate a card effect.
    fn check_trigger(
        &self,
        trigger: &TriggerExpr,
        event:   &GameEvent,
        ctx:     &Self::Context,
    ) -> bool;

    /// Check whether an activation condition is met.
    /// Called before an effect can be placed on the chain.
    fn evaluate_condition(
        &self,
        condition: &ConditionExpr,
        card:      &Card,
        ctx:       &Self::Context,
    ) -> bool;

    // ── Cost Execution ────────────────────────────────────────

    /// Check whether a cost is currently payable without executing it.
    /// Used to determine if an effect can legally be activated.
    fn can_pay_cost(
        &self,
        cost: &CostAction,
        card: &Card,
        ctx:  &Self::Context,
    ) -> bool;

    /// Execute a cost action. Returns true if the cost was paid
    /// successfully, false if it could not be paid (e.g. no valid
    /// target existed when resolution happened).
    ///
    /// If this returns false, the effect should be considered invalid
    /// and removed from the chain without resolving.
    fn execute_cost(
        &mut self,
        cost: &CostAction,
        card: &Card,
        ctx:  &mut Self::Context,
    ) -> bool;

    // ── Effect Resolution ─────────────────────────────────────

    /// Execute a single game action during effect resolution.
    /// The engine translates DuelScript AST nodes into game state changes.
    fn execute_action(
        &mut self,
        action: &GameAction,
        card:   &Card,
        ctx:    &mut Self::Context,
    );

    /// Execute an entire effect's resolution clause.
    /// Default implementation iterates actions in order —
    /// override if your engine needs custom sequencing.
    fn execute_resolve(
        &mut self,
        effect: &EffectBody,
        card:   &Card,
        ctx:    &mut Self::Context,
    ) {
        for action in &effect.on_resolve {
            self.execute_action(action, card, ctx);
        }
    }

    // ── Target Selection ──────────────────────────────────────

    /// Resolve a target expression into concrete card handles.
    /// The engine controls what "targeting" means — zone state,
    /// face-up/face-down, controller, valid targets etc.
    ///
    /// Returns the IDs (or handles) of selected targets.
    /// Returns an empty Vec if targeting failed (no valid targets).
    fn resolve_targets(
        &self,
        target: &TargetExpr,
        card:   &Card,
        ctx:    &Self::Context,
    ) -> Vec<CardHandle>;

    // ── Continuous Effect Application ─────────────────────────

    /// Apply a continuous effect to the game state.
    /// Called whenever a card with continuous effects enters a zone.
    fn apply_continuous_effect(
        &mut self,
        effect: &ContinuousEffect,
        card:   &Card,
        ctx:    &mut Self::Context,
    );

    /// Remove a continuous effect from the game state.
    /// Called whenever a card with continuous effects leaves a zone.
    fn remove_continuous_effect(
        &mut self,
        effect: &ContinuousEffect,
        card:   &Card,
        ctx:    &mut Self::Context,
    );

    // ── Replacement Effects ───────────────────────────────────

    /// Check if a replacement effect intercepts a game event.
    /// Returns true if the replacement fires (and the original event
    /// should be suppressed).
    fn check_replacement(
        &self,
        effect: &ReplacementEffect,
        event:  &GameEvent,
        card:   &Card,
        ctx:    &Self::Context,
    ) -> bool;

    // ── Summon Validation ─────────────────────────────────────

    /// Check whether a card can legally be summoned given current
    /// game state. The engine enforces summon conditions declared
    /// in the card's `summon_conditions` list.
    fn can_summon(
        &self,
        card:      &Card,
        method:    &SummonMethod,
        materials: &[CardHandle],
        ctx:       &Self::Context,
    ) -> bool;

    // ── Win Condition ─────────────────────────────────────────

    /// Check whether a win condition has been met.
    /// Called at appropriate intervals by the engine.
    fn check_win_condition(
        &self,
        condition: &WinCondition,
        card:      &Card,
        ctx:       &Self::Context,
    ) -> Option<WinResult>;
}

// ── Game Event ────────────────────────────────────────────────

/// A game event passed to trigger checks.
/// Your engine constructs these as game state changes occur.
/// DuelScript trigger expressions are matched against these.
#[derive(Debug, Clone)]
pub struct GameEvent {
    pub kind:      GameEventKind,
    pub source:    Option<CardHandle>,   // card that caused the event
    pub targets:   Vec<CardHandle>,      // cards affected
    pub controller: Player,             // whose turn / who caused it
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameEventKind {
    // Summon events
    NormalSummon,
    TributeSummon       { tributes: Vec<CardHandle> },
    SpecialSummon       { method: SummonMethod },
    FlipSummon,

    // Card movement
    SentToGraveyard     { cause: DestructionCause },
    Banished            { cause: DestructionCause },
    ReturnedToHand,
    ReturnedToDeck,
    AddedToHand,
    Drawn,

    // Battle
    Attacked            { attacker: CardHandle, defender: CardHandle },
    Destroyed           { cause: DestructionCause },

    // Effect events
    EffectActivated     { actions: Vec<TriggerAction> },
    SpellActivated,
    TrapActivated,
    MonsterEffectActivated,

    // Phase events
    PhaseStarted        { phase: Phase },
    PhaseEnded          { phase: Phase },

    // LP events
    DamageTaken         { amount: u32, player: Player },
    LpGained            { amount: u32, player: Player },

    // Counter events
    CounterPlaced       { name: String, count: u32 },
    CounterRemoved      { name: String, count: u32 },

    // Nth summon tracking
    NthSummon           { n: u32 },
}

// ── Card Handle ───────────────────────────────────────────────

/// A lightweight reference to a card instance in the engine.
/// The engine defines what this means internally — could be a
/// UUID, an index, a pointer, whatever makes sense for your engine.
///
/// DuelScript only passes handles around — it never owns card instances.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CardHandle(pub u64);

impl CardHandle {
    pub fn new(id: u64) -> Self { Self(id) }
    pub fn id(&self) -> u64 { self.0 }
}

// ── Effect Activation Request ─────────────────────────────────

/// A fully resolved request to activate an effect.
/// The engine builds this when a player declares activation,
/// then passes it through the chain system.
#[derive(Debug)]
pub struct EffectActivation {
    /// The card activating the effect
    pub card:        Arc<Card>,
    /// Which specific effect on the card
    pub effect_index: usize,
    /// Pre-selected targets (if targeting happens on activation)
    pub targets:     Vec<CardHandle>,
    /// The player activating
    pub controller:  Player,
}

// ── Effect Chain Entry ────────────────────────────────────────

/// One link in the effect chain.
/// Your engine maintains a Vec<ChainLink> during chain building.
#[derive(Debug)]
pub struct ChainLink {
    pub activation:  EffectActivation,
    pub chain_index: usize,         // 1-indexed position in chain
    pub resolved:    bool,
}

// ── Engine Context Helpers ────────────────────────────────────

/// Helper trait for your game context type.
/// Implement this alongside DuelScriptEngine to give DuelScript
/// access to the game state it needs for condition evaluation.
pub trait GameContext {
    /// The current phase of the game.
    fn current_phase(&self) -> &Phase;

    /// Whose turn it currently is.
    fn turn_player(&self) -> Player;

    /// Life points for a player.
    fn life_points(&self, player: Player) -> u32;

    /// Number of cards in a player's hand.
    fn hand_size(&self, player: Player) -> usize;

    /// Number of cards in a player's graveyard.
    fn gy_count(&self, player: Player) -> usize;

    /// Number of cards a player has banished.
    fn banished_count(&self, player: Player) -> usize;

    /// Whether a player controls any monsters.
    fn controls_monsters(&self, player: Player) -> bool;

    /// Whether a specific card is currently in a zone.
    fn card_is_in_zone(&self, card: &CardHandle, zone: &Zone) -> bool;

    /// Current ATK of a card instance (may differ from base ATK due to effects).
    fn current_atk(&self, card: &CardHandle) -> i32;

    /// Current DEF of a card instance.
    fn current_def(&self, card: &CardHandle) -> i32;

    /// The card definition for a handle.
    fn card_def(&self, handle: &CardHandle) -> Option<Arc<Card>>;

    /// How many times an effect has been used this turn (for once_per_turn).
    fn effect_uses_this_turn(&self, card: &CardHandle, effect_index: usize) -> u32;

    /// How many times an effect has been used this duel (for once_per_duel).
    fn effect_uses_this_duel(&self, card: &CardHandle, effect_index: usize) -> u32;
}

// ── Default Condition Evaluator ───────────────────────────────

/// A standalone helper that evaluates ConditionExpr nodes
/// against a GameContext. Engines can use this directly or
/// override individual cases in their DuelScriptEngine impl.
pub fn evaluate_condition_default<Ctx: GameContext>(
    condition: &ConditionExpr,
    card_handle: &CardHandle,
    card: &Card,
    ctx: &Ctx,
) -> bool {
    match condition {
        ConditionExpr::Simple(s) => eval_simple(s, card_handle, card, ctx),
        ConditionExpr::And(cs)   => cs.iter().all(|c| eval_simple(c, card_handle, card, ctx)),
        ConditionExpr::Or(cs)    => cs.iter().any(|c| eval_simple(c, card_handle, card, ctx)),
    }
}

fn eval_simple<Ctx: GameContext>(
    cond: &SimpleCondition,
    card_handle: &CardHandle,
    _card: &Card,
    ctx: &Ctx,
) -> bool {
    match cond {
        SimpleCondition::InZone(zone) => {
            ctx.card_is_in_zone(card_handle, zone)
        }
        SimpleCondition::OnField => {
            ctx.card_is_in_zone(card_handle, &Zone::Field)
                || ctx.card_is_in_zone(card_handle, &Zone::MonsterZone)
                || ctx.card_is_in_zone(card_handle, &Zone::SpellTrapZone)
        }
        SimpleCondition::YouControlNoMonsters => {
            // "you" = the controller of this card
            !ctx.controls_monsters(ctx.turn_player()) // simplified
        }
        SimpleCondition::LpCondition { player, op, value } => {
            let lp = ctx.life_points(player.clone());
            compare(lp, *op, *value)
        }
        SimpleCondition::HandSize { op, value } => {
            let size = ctx.hand_size(ctx.turn_player()) as u32;
            compare(size, *op, *value)
        }
        SimpleCondition::CardsInGy { op, value } => {
            let count = ctx.gy_count(ctx.turn_player()) as u32;
            compare(count, *op, *value)
        }
        SimpleCondition::BanishedCount { op, value } => {
            let count = ctx.banished_count(ctx.turn_player()) as u32;
            compare(count, *op, *value)
        }
        _ => true, // engine handles complex board conditions
    }
}

fn compare(lhs: u32, op: CompareOp, rhs: u32) -> bool {
    match op {
        CompareOp::Gte => lhs >= rhs,
        CompareOp::Lte => lhs <= rhs,
        CompareOp::Gt  => lhs >  rhs,
        CompareOp::Lt  => lhs <  rhs,
        CompareOp::Eq  => lhs == rhs,
        CompareOp::Neq => lhs != rhs,
    }
}

// ── Frequency Guard ───────────────────────────────────────────

/// Helper to check once_per_turn / once_per_duel guards.
/// Call this before allowing an effect activation.
pub fn frequency_allows<Ctx: GameContext>(
    effect:       &EffectBody,
    card_handle:  &CardHandle,
    effect_index: usize,
    ctx:          &Ctx,
) -> bool {
    match &effect.frequency {
        Frequency::Unlimited    => true,
        Frequency::OncePerTurn(_) => ctx.effect_uses_this_turn(card_handle, effect_index) == 0,
        Frequency::TwicePerTurn => ctx.effect_uses_this_turn(card_handle, effect_index) < 2,
        Frequency::OncePerDuel  => ctx.effect_uses_this_duel(card_handle, effect_index) == 0,
        Frequency::EachTurn     => ctx.effect_uses_this_turn(card_handle, effect_index) == 0,
    }
}

// ── Trigger Matcher ───────────────────────────────────────────

/// Default trigger matching logic.
/// Engines can use this directly or extend it for custom triggers.
pub fn trigger_matches(trigger: &TriggerExpr, event: &GameEvent) -> bool {
    match (trigger, &event.kind) {
        (TriggerExpr::OpponentActivates(actions), GameEventKind::EffectActivated { actions: ev_actions }) => {
            // Any of the declared trigger actions match the event
            actions.iter().any(|a| ev_actions.contains(a))
        }
        (TriggerExpr::WhenSummoned(method), GameEventKind::NormalSummon) => {
            method.as_ref().map_or(true, |m| *m == SummonMethod::ByNormalSummon)
        }
        (TriggerExpr::WhenSummoned(method), GameEventKind::SpecialSummon { method: ev_method }) => {
            method.as_ref().map_or(true, |m| m == ev_method)
        }
        (TriggerExpr::WhenTributeSummoned(_), GameEventKind::TributeSummon { .. }) => true,
        (TriggerExpr::WhenDestroyed(cause), GameEventKind::Destroyed { cause: ev_cause }) => {
            cause.as_ref().map_or(true, |c| c == ev_cause)
        }
        (TriggerExpr::WhenSentTo { zone, cause }, GameEventKind::SentToGraveyard { cause: ev_cause }) => {
            *zone == Zone::Graveyard
                && cause.as_ref().map_or(true, |c| c == ev_cause)
        }
        (TriggerExpr::WhenFlipped, GameEventKind::FlipSummon) => true,
        (TriggerExpr::WhenAttacked, GameEventKind::Attacked { .. }) => true,
        (TriggerExpr::DuringPhase(phase), GameEventKind::PhaseStarted { phase: ev_phase }) => {
            phase == ev_phase
        }
        (TriggerExpr::OnNthSummon(n), GameEventKind::NthSummon { n: ev_n }) => n == ev_n,
        _ => false,
    }
}
