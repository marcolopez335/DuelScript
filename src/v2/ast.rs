// ============================================================
// DuelScript v2 AST — clean type definitions
//
// Every type maps 1:1 to a grammar rule. No bitfields, no
// engine-specific constants. Just game mechanics.
// ============================================================

// ── Top Level ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct File {
    pub cards: Vec<Card>,
}

#[derive(Debug, Clone)]
pub struct Card {
    pub name: String,
    pub fields: CardFields,
    pub summon: Option<SummonBlock>,
    pub effects: Vec<Effect>,
    pub passives: Vec<Passive>,
    pub restrictions: Vec<Restriction>,
    pub replacements: Vec<Replacement>,
}

#[derive(Debug, Clone, Default)]
pub struct CardFields {
    pub id: Option<u64>,
    pub card_types: Vec<CardType>,
    pub attribute: Option<Attribute>,
    pub race: Option<Race>,
    pub level: Option<u32>,
    pub rank: Option<u32>,
    pub link: Option<u32>,
    pub scale: Option<u32>,
    pub atk: Option<StatVal>,
    pub def: Option<StatVal>,
    pub link_arrows: Vec<Arrow>,
    pub archetypes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatVal {
    Number(i32),
    Unknown, // ?
}

// ── Card Types ───────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardType {
    NormalMonster, EffectMonster, RitualMonster,
    FusionMonster, SynchroMonster, XyzMonster,
    LinkMonster, PendulumMonster,
    Tuner, SynchroTuner, Flip, Gemini, Union, Spirit, Toon,
    NormalSpell, QuickPlaySpell, ContinuousSpell,
    EquipSpell, FieldSpell, RitualSpell,
    NormalTrap, CounterTrap, ContinuousTrap,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Attribute { Light, Dark, Fire, Water, Earth, Wind, Divine }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Race {
    Dragon, Spellcaster, Zombie, Warrior, BeastWarrior,
    Beast, WingedBeast, Fiend, Fairy, Insect,
    Dinosaur, Reptile, Fish, SeaSerpent, Aqua,
    Pyro, Thunder, Rock, Plant, Machine,
    Psychic, DivineBeast, Wyrm, Cyberse, Illusion,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arrow {
    TopLeft, Top, TopRight, Left, Right,
    BottomLeft, Bottom, BottomRight,
}

// ── Summon Block ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SummonBlock {
    pub cannot_normal_summon: bool,
    pub cannot_special_summon: bool,
    pub tributes: Option<u32>,
    pub special_summon_procedure: Option<SpecialSummonProcedure>,
    pub fusion_materials: Option<MaterialList>,
    pub synchro_materials: Option<SynchroMaterials>,
    pub xyz_materials: Option<Selector>,
    pub link_materials: Option<Selector>,
    pub ritual_materials: Option<RitualMaterials>,
    pub pendulum_from: Vec<Zone>,
}

#[derive(Debug, Clone)]
pub struct SpecialSummonProcedure {
    pub from: Option<Zone>,
    pub to: Option<FieldTarget>,
    pub cost: Vec<CostAction>,
    pub condition: Option<Condition>,
    pub restriction: Option<Restriction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldTarget { YourField, OpponentField, EitherField }

#[derive(Debug, Clone)]
pub struct MaterialList {
    pub items: Vec<MaterialItem>,
}

#[derive(Debug, Clone)]
pub enum MaterialItem {
    Named(String),
    Generic(Selector),
}

#[derive(Debug, Clone)]
pub struct SynchroMaterials {
    pub tuner: Selector,
    pub non_tuner: Selector,
}

#[derive(Debug, Clone)]
pub struct RitualMaterials {
    pub materials: Selector,
    pub level_constraint: Option<LevelConstraint>,
}

#[derive(Debug, Clone)]
pub struct LevelConstraint {
    pub kind: LevelConstraintKind,
    pub op: CompareOp,
    pub value: Expr,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LevelConstraintKind { TotalLevel, ExactLevel }

// ── Effect Block ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Effect {
    pub name: String,
    pub speed: Option<u8>,          // 1, 2, or 3
    pub frequency: Option<Frequency>,
    pub mandatory: bool,
    pub timing: Option<Timing>,
    pub trigger: Option<Trigger>,
    pub who: Option<PlayerWho>,
    pub condition: Option<Condition>,
    pub activate_from: Vec<Zone>,
    pub damage_step: Option<bool>,
    pub target: Option<TargetDecl>,
    pub cost: Vec<CostAction>,
    pub resolve: Vec<Action>,
    pub choose: Option<ChooseBlock>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frequency {
    OncePerTurn(OptKind),
    TwicePerTurn,
    OncePerDuel,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptKind { Soft, Hard }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Timing { When, If }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlayerWho { You, Opponent, Controller, Owner, Summoner, Both }

// ── Triggers ─────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Trigger {
    Summoned(Option<SummonMethod>),
    SpecialSummoned(Option<SummonMethod>),
    NormalSummoned,
    TributeSummoned,
    FlipSummoned,
    Flipped,
    Destroyed(Option<DestroyBy>),
    DestroyedByBattle,
    DestroyedByEffect,
    DestroysByBattle,
    SentTo(Zone, Option<Zone>),     // sent_to zone [from zone]
    LeavesField,
    Banished,
    ReturnedTo(Zone),
    AttackDeclared,
    OpponentAttackDeclared,
    Attacked,
    BattleDamage(Option<PlayerWho>),
    DirectAttackDamage,
    DamageCalculation,
    StandbyPhase(Option<PhaseOwner>),
    EndPhase,
    DrawPhase,
    MainPhase,
    BattlePhase,
    SummonAttempt,
    SpellTrapActivated,
    OpponentActivates(Vec<Category>),
    ChainLink,
    Targeted,
    PositionChanged,
    ControlChanged,
    Equipped,
    Unequipped,
    UsedAsMaterial(Option<SummonMethod>),
    Custom(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SummonMethod {
    Normal, Special, Flip, Tribute,
    Fusion, Synchro, Xyz, Link, Ritual, Pendulum,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DestroyBy { Battle, Effect, CardEffect }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseOwner { Yours, Opponents, Either }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Category {
    Search, SpecialSummon, SendToGy, AddToHand,
    Draw, Banish, Destroy, Negate, Mill,
    ActivateSpell, ActivateTrap, ActivateMonsterEffect,
    NormalSummon, FusionSummon, SynchroSummon,
    XyzSummon, LinkSummon, RitualSummon,
    AttackDeclared,
}

// ── Conditions ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Condition {
    And(Vec<ConditionAtom>),
    Or(Vec<ConditionAtom>),
    Single(ConditionAtom),
}

#[derive(Debug, Clone)]
pub enum ConditionAtom {
    Not(Box<ConditionAtom>),
    SelfState(CardState),
    Controls(PlayerWho, Selector),
    NoCardsOnField(CardFilterKind, FieldOwner),
    LpCompare(CompareOp, Expr),
    OpponentLpCompare(CompareOp, Expr),
    HandSize(CompareOp, Expr),
    CardsInGy(CompareOp, Expr),
    CardsInBanished(CompareOp, Expr),
    OnField,
    InGy,
    InHand,
    InBanished,
    PhaseIs(PhaseName),
    ChainIncludes(Vec<Category>),
    HasCounter(String, Option<CompareOp>, Option<Expr>, CounterTarget),
    HasFlag(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardState {
    SummonedThisTurn, AttackedThisTurn, FlippedThisTurn,
    ActivatedThisTurn, FaceUp, FaceDown,
    InAttackPosition, InDefensePosition,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldOwner { Your, Opponent, Either }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CounterTarget { OnSelf, OnSelector }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PhaseName {
    Draw, Standby, Main1, Battle, Main2, End,
    Damage, DamageCalculation,
}

// ── Selectors ────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Selector {
    SelfCard,
    Target,             // previously selected
    EquippedCard,
    NegatedCard,
    Searched,
    LinkedCard,
    Binding(String),    // named reference
    Counted {
        quantity: Quantity,
        filter: CardFilter,
        controller: Option<Controller>,
        zone: Option<ZoneFilter>,
        position: Option<PositionFilter>,
        where_clause: Option<Predicate>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Quantity { All, Exact(u32), AtLeast(u32) }

#[derive(Debug, Clone)]
pub struct CardFilter {
    pub name: Option<String>,       // "Dark Magician"
    pub kind: CardFilterKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CardFilterKind {
    Monster, Spell, Trap, Card,
    EffectMonster, NormalMonster,
    FusionMonster, SynchroMonster, XyzMonster, LinkMonster,
    RitualMonster, PendulumMonster,
    TunerMonster, NonTunerMonster, NonTokenMonster,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Controller { You, Opponent, Either }

#[derive(Debug, Clone)]
pub enum ZoneFilter { In(Vec<Zone>), From(Vec<Zone>), OnField(FieldOwner) }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PositionFilter {
    FaceUp, FaceDown, AttackPosition, DefensePosition, ExceptSelf,
}

// ── Predicates (where clauses) ───────────────────────────────

#[derive(Debug, Clone)]
pub enum Predicate {
    And(Vec<PredicateAtom>),
    Or(Vec<PredicateAtom>),
    Single(PredicateAtom),
}

#[derive(Debug, Clone)]
pub enum PredicateAtom {
    Not(Box<PredicateAtom>),
    StatCompare(StatField, CompareOp, Expr),
    AttributeIs(Attribute),
    RaceIs(Race),
    TypeIs(CardType),
    NameIs(String),
    ArchetypeIs(String),
    IsFaceUp, IsFaceDown,
    IsMonster, IsSpell, IsTrap,
    IsEffect, IsNormal,
    IsTuner, IsFusion, IsSynchro, IsXyz, IsLink,
    IsRitual, IsPendulum, IsToken, IsFlip,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatField {
    Atk, Def, Level, Rank, Link, Scale,
    BaseAtk, BaseDef, OriginalAtk, OriginalDef,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompareOp { Gte, Lte, Eq, Neq, Gt, Lt }

// ── Target Declaration ───────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TargetDecl {
    pub selector: Selector,
    pub binding: Option<String>,
}

// ── Costs ────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum CostAction {
    PayLp(Expr),
    Discard(Selector, Option<String>),
    Tribute(Selector, Option<String>),
    Banish(Selector, Option<Zone>, Option<String>),
    Send(Selector, Zone, Option<String>),
    Detach(u32, Selector),
    RemoveCounter(String, u32, Selector),
    Reveal(Selector),
    Announce(AnnounceWhat, Option<String>),
    None,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AnnounceWhat { Type, Attribute, Race, Level, Card }

// ── Actions ──────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Action {
    Draw(Expr),
    Discard(Selector),
    Destroy(Selector),
    Banish(Selector, Option<Zone>, bool),  // face_down flag
    Send(Selector, Zone),
    Return(Selector, ReturnDest),
    Search(Selector, Option<Zone>),
    AddToHand(Selector, Option<Zone>),
    SpecialSummon(Selector, Option<Zone>, Option<BattlePosition>),
    RitualSummon {
        target: Selector,
        materials: Option<Selector>,
        level_op: Option<CompareOp>,
        level_expr: Option<Expr>,
    },
    FusionSummon { target: Selector, materials: Option<Selector> },
    SynchroSummon { target: Selector, materials: Option<Selector> },
    XyzSummon { target: Selector, materials: Option<Selector> },
    NormalSummon(Selector),
    Set(Selector, Option<Zone>),
    FlipDown(Selector),
    ChangePosition(Selector, Option<BattlePosition>),
    TakeControl(Selector, Option<Duration>),
    Equip(Selector, Selector),
    Negate(bool),                         // and_destroy flag
    NegateEffects(Selector, Option<Duration>),
    Damage(PlayerWho, Expr),
    GainLp(Expr),
    PayLp(Expr),
    ModifyStat(StatName, Selector, bool, Expr, Option<Duration>), // is_negative
    SetStat(StatName, Selector, Expr, Option<Duration>),
    ChangeLevel(Selector, Expr),
    ChangeAttribute(Selector, Attribute),
    ChangeRace(Selector, Race),
    ChangeName(Selector, String, Option<Duration>),
    SetScale(Selector, Expr),
    CreateToken(TokenSpec),
    Attach(Selector, Selector),
    Detach(u32, Selector),
    PlaceCounter(String, u32, Selector),
    RemoveCounter(String, u32, Selector),
    Mill(Expr, Option<DeckOwner>),
    Excavate(Expr, DeckOwner),
    Reveal(Selector),
    LookAt(Selector, Option<Zone>),
    ShuffleDeck(Option<DeckOwner>),
    Announce(AnnounceWhat, Option<String>),
    LinkTo(Selector, Selector),
    CoinFlip { heads: Vec<Action>, tails: Vec<Action> },
    DiceRoll(Vec<Action>),
    Grant(Selector, GrantAbility, Option<Duration>),
    If { condition: Condition, then: Vec<Action>, otherwise: Vec<Action> },
    ForEach { selector: Selector, zone: Zone, body: Vec<Action> },
    Choose(ChooseBlock),
    Delayed { until: PhaseName, body: Vec<Action> },
    AndIfYouDo(Vec<Action>),
    Then(Vec<Action>),
    Also(Vec<Action>),
    InstallWatcher { name: String, event: Trigger, duration: Duration, check: Vec<Action> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReturnDest { Hand, Deck(Option<DeckPosition>), ExtraDeck }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeckPosition { Top, Bottom, Shuffle }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StatName { Atk, Def }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeckOwner { Yours, Opponents }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BattlePosition { Attack, Defense, FaceDownDefense }

// ── Grant Abilities ──────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantAbility {
    CannotAttack,
    CannotAttackDirectly,
    CannotChangePosition,
    CannotBeDestroyed(Option<DestroyBy>),
    CannotBeTargeted(Option<TargetedBy>),
    CannotBeTributed,
    CannotBeUsedAsMaterial,
    CannotActivate(Option<ActivateWhat>),
    CannotNormalSummon,
    CannotSpecialSummon,
    UnaffectedBy(UnaffectedSource),
    Piercing,
    DirectAttack,
    DoubleAttack,
    TripleAttack,
    AttackAllMonsters,
    MustAttack,
    ImmuneToTargeting,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetedBy { Spells, Traps, Monsters, Effects, Opponent }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActivateWhat { Effects, Spells, Traps }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnaffectedSource { Spells, Traps, Monsters, Effects, OpponentEffects }

// ── Duration ─────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Duration {
    ThisTurn,
    EndOfTurn,
    EndPhase,
    EndOfDamageStep,
    NextStandbyPhase,
    WhileOnField,
    WhileFaceUp,
    Permanently,
    NTurns(u32),
}

// ── Passive Block ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Passive {
    pub name: String,
    pub scope: Option<Scope>,
    pub target: Option<Selector>,
    pub condition: Option<Condition>,
    pub modifiers: Vec<Modifier>,
    pub grants: Vec<GrantAbility>,
    pub negate_effects: bool,
    pub set_atk: Option<Expr>,
    pub set_def: Option<Expr>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Scope { Self_, Field }

#[derive(Debug, Clone)]
pub struct Modifier {
    pub stat: StatName,
    pub positive: bool,
    pub value: Expr,
}

// ── Restriction Block ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Restriction {
    pub name: Option<String>,
    pub apply_to: Option<PlayerWho>,
    pub target: Option<Selector>,
    pub abilities: Vec<GrantAbility>,
    pub duration: Option<Duration>,
    pub trigger: Option<Trigger>,
    pub condition: Option<Condition>,
}

// ── Replacement Block ────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Replacement {
    pub name: Option<String>,
    pub instead_of: ReplaceableEvent,
    pub actions: Vec<Action>,
    pub condition: Option<Condition>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceableEvent {
    DestroyedByBattle, DestroyedByEffect, Destroyed,
    SentToGy, Banished,
    ReturnedToHand, ReturnedToDeck, LeavesField,
}

// ── Choose Block ─────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct ChooseBlock {
    pub options: Vec<ChooseOption>,
}

#[derive(Debug, Clone)]
pub struct ChooseOption {
    pub label: String,
    pub target: Option<TargetDecl>,
    pub cost: Vec<CostAction>,
    pub trigger: Option<Trigger>,
    pub resolve: Vec<Action>,
}

// ── Token Spec ───────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct TokenSpec {
    pub name: Option<String>,
    pub attribute: Option<Attribute>,
    pub race: Option<Race>,
    pub level: Option<u32>,
    pub atk: StatVal,
    pub def: StatVal,
    pub count: u32,
    pub position: Option<BattlePosition>,
    pub restriction: Option<Restriction>,
}

// ── Expressions ──────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum Expr {
    Literal(i32),
    Half,
    StatRef(String, StatField),     // "self.atk", "target.level"
    BindingRef(String, StatField),  // "tributed.level"
    PlayerLp(LpOwner),
    Count(Box<Selector>),
    BinOp { left: Box<Expr>, op: BinOp, right: Box<Expr> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LpOwner { Your, Opponent, Controller }

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinOp { Add, Sub, Mul, Div }

// ── Zones ────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Zone {
    Hand, Field, Deck, ExtraDeck, ExtraDeckFaceUp,
    Gy, Banished,
    MonsterZone, SpellTrapZone, FieldZone,
    PendulumZone, ExtraMonsterZone,
    Overlay, Equipped,
    TopOfDeck, BottomOfDeck,
}
