//! Parallel computer-player evaluation.

#[cfg(feature = "native-worker")]
use flume::{Receiver, Sender};
use ludo_domain::{GameCommand, GameEvent, GameState, PlayerId, TokenId, TokenPosition, TurnPhase};
use rand::{RngExt, SeedableRng, rngs::SmallRng};
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// Difficulty controls the amount of parallel look-ahead work.
pub use ludo_domain::BotDifficulty as Difficulty;

/// Revision-tagged immutable AI request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BotRequest {
    /// State snapshot.
    pub state: GameState,
    /// Difficulty.
    pub difficulty: Difficulty,
    /// Approximate Monte Carlo work budget in milliseconds.
    pub thinking_time_ms: u64,
}

impl BotRequest {
    /// Creates a request with a difficulty-appropriate work budget.
    #[must_use]
    pub const fn new(state: GameState, difficulty: Difficulty) -> Self {
        let thinking_time_ms = match difficulty {
            Difficulty::Easy => 0,
            Difficulty::Medium => 20,
            Difficulty::Hard => 120,
        };
        Self {
            state,
            difficulty,
            thinking_time_ms,
        }
    }

    /// Overrides the rollout work budget.
    #[must_use]
    pub const fn with_thinking_time_ms(mut self, thinking_time_ms: u64) -> Self {
        self.thinking_time_ms = thinking_time_ms;
        self
    }
}

/// Worker result, safe to discard when its revision is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BotDecision {
    /// Source state revision.
    pub revision: u64,
    /// Current player.
    pub player: PlayerId,
    /// Selected token, or none if the request was not in move phase.
    pub token: Option<TokenId>,
    /// Parallel rollout count used for the decision.
    pub simulations: u32,
}

/// Chooses moves using Rayon for independent candidate evaluation.
#[derive(Debug, Default, Clone, Copy)]
pub struct ParallelBot;

impl ParallelBot {
    /// Evaluates a state snapshot.
    #[must_use]
    pub fn choose(request: &BotRequest) -> BotDecision {
        let state = &request.state;
        let player = state.current().player.id;
        let legal = match state.phase() {
            TurnPhase::AwaitingMove { legal_tokens, .. } => legal_tokens,
            TurnPhase::AwaitingRoll => {
                return BotDecision {
                    revision: state.revision(),
                    player,
                    token: None,
                    simulations: 0,
                };
            }
        };
        let simulations_per_candidate = simulation_count(request);
        let token = legal
            .par_iter()
            .map(|token| (*token, evaluate(state, *token, request), token.index()))
            .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.2.cmp(&left.2)))
            .map(|result| result.0);
        let simulations = u32::try_from(legal.len())
            .unwrap_or(u32::MAX)
            .saturating_mul(simulations_per_candidate);
        BotDecision {
            revision: state.revision(),
            player,
            token,
            simulations,
        }
    }
}

/// Long-lived non-blocking Rayon-backed AI adapter.
#[cfg(feature = "native-worker")]
pub struct BotWorker {
    sender: Sender<BotRequest>,
    receiver: Receiver<BotDecision>,
}

#[cfg(feature = "native-worker")]
impl BotWorker {
    /// Creates a bounded worker. New requests are evaluated on Rayon's global pool.
    #[must_use]
    pub fn new() -> Self {
        let (request_sender, request_receiver) = flume::bounded::<BotRequest>(1);
        let (decision_sender, decision_receiver) = flume::bounded::<BotDecision>(1);
        let stale_decisions = decision_receiver.clone();
        std::thread::Builder::new()
            .name("ludo-ai-dispatch".to_owned())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    // One outer task at a time bounds retained snapshots while
                    // the evaluation itself saturates Rayon's shared pool.
                    let decision = ParallelBot::choose(&request);
                    if let Err(flume::TrySendError::Full(decision)) =
                        decision_sender.try_send(decision)
                    {
                        let _ = stale_decisions.try_recv();
                        let _ = decision_sender.try_send(decision);
                    }
                }
            })
            .ok();
        Self {
            sender: request_sender,
            receiver: decision_receiver,
        }
    }

    /// Queues a state snapshot without blocking the caller.
    ///
    /// # Errors
    ///
    /// Returns the original request when the bounded queue is full or closed.
    pub fn request(&self, request: BotRequest) -> Result<(), Box<BotRequest>> {
        self.sender
            .try_send(request)
            .map_err(|error| Box::new(error.into_inner()))
    }

    /// Returns a completed decision when available.
    #[must_use]
    pub fn try_decision(&self) -> Option<BotDecision> {
        self.receiver.try_recv().ok()
    }
}

#[cfg(feature = "native-worker")]
impl Default for BotWorker {
    fn default() -> Self {
        Self::new()
    }
}

fn evaluate(state: &GameState, token: TokenId, request: &BotRequest) -> i32 {
    let from = state.current().tokens[token.index()].position;
    let mut simulated = state.clone();
    let Ok(events) = simulated.apply(GameCommand::Move(token)) else {
        return i32::MIN;
    };
    let to = simulated.players()[state.current_player_index()].tokens[token.index()].position;
    let player = state.current().player.id;
    let advancement = match (from, to) {
        (TokenPosition::Yard, TokenPosition::Path(0)) => 24,
        (TokenPosition::Path(before), TokenPosition::Path(after)) => i32::from(after - before),
        (_, TokenPosition::Finished) => 120,
        _ => 0,
    };
    let capture = i32::try_from(
        events
            .iter()
            .filter(|event| matches!(event, GameEvent::TokenCaptured { .. }))
            .count(),
    )
    .unwrap_or_default()
        * 70;
    let tactical = advancement + capture;
    match request.difficulty {
        Difficulty::Easy => tactical,
        Difficulty::Medium => {
            tactical
                + safety_score(&simulated, player, token)
                + monte_carlo_score(&simulated, player, token, simulation_count(request))
        }
        Difficulty::Hard => {
            let future: i32 = (1_u8..=6)
                .into_par_iter()
                .map(|roll| future_score(&simulated, roll, player))
                .sum();
            tactical * 6
                + safety_score(&simulated, player, token) * 3
                + exposure_score(&simulated, player, token)
                + blockade_score(&simulated, player, token)
                + opponent_model_score(&simulated, player)
                + future
                + monte_carlo_score(&simulated, player, token, simulation_count(request))
        }
    }
}

fn simulation_count(request: &BotRequest) -> u32 {
    match request.difficulty {
        Difficulty::Easy => 0,
        Difficulty::Medium => 6,
        Difficulty::Hard if request.thinking_time_ms == 0 => 0,
        Difficulty::Hard => u32::try_from(request.thinking_time_ms.saturating_mul(2))
            .unwrap_or(u32::MAX)
            .clamp(24, 512),
    }
}

fn safety_score(state: &GameState, player: PlayerId, token: TokenId) -> i32 {
    match state.global_track_index(player, token) {
        Some(index) if state.is_safe_track_index(index) => 18,
        Some(_) => 0,
        None => 12,
    }
}

fn exposure_score(state: &GameState, player: PlayerId, token: TokenId) -> i32 {
    let Some(target) = state.global_track_index(player, token) else {
        return 8;
    };
    if state.is_safe_track_index(target) {
        return 24;
    }
    state
        .players()
        .iter()
        .filter(|opponent| opponent.player.id != player)
        .flat_map(|opponent| {
            opponent.tokens.iter().filter_map(|opponent_token| {
                state
                    .global_track_index(opponent.player.id, opponent_token.id)
                    .map(|attacker| (target + 52 - attacker) % 52)
            })
        })
        .filter(|distance| (1..=6).contains(distance))
        .map(|distance| -i32::from(7 - distance) * 8)
        .sum()
}

fn blockade_score(state: &GameState, player: PlayerId, token: TokenId) -> i32 {
    let Some(global) = state.global_track_index(player, token) else {
        return 0;
    };
    let allies = state.players()[player.index()]
        .tokens
        .iter()
        .filter(|ally| state.global_track_index(player, ally.id) == Some(global))
        .count();
    if allies >= 2 { 42 } else { 0 }
}

fn opponent_model_score(state: &GameState, player: PlayerId) -> i32 {
    let own = progress_score(&state.players()[player.index()].tokens);
    let strongest_opponent = state
        .players()
        .iter()
        .filter(|opponent| opponent.player.id != player)
        .map(|opponent| progress_score(&opponent.tokens))
        .max()
        .unwrap_or_default();
    (own - strongest_opponent) / 8
}

fn progress_score(tokens: &[ludo_domain::Token; 4]) -> i32 {
    tokens
        .iter()
        .map(|token| match token.position {
            TokenPosition::Yard => 0,
            TokenPosition::Path(progress) => i32::from(progress) + 1,
            TokenPosition::Finished => 64,
        })
        .sum()
}

fn monte_carlo_score(state: &GameState, player: PlayerId, token: TokenId, simulations: u32) -> i32 {
    if simulations == 0 {
        return 0;
    }
    let total: i64 = (0..simulations)
        .into_par_iter()
        .map(|sample| {
            let seed = state
                .revision()
                .wrapping_mul(0x9e37_79b9)
                .wrapping_add(u64::from(sample))
                .wrapping_add(u64::try_from(token.index()).unwrap_or_default() << 32);
            let mut rng = SmallRng::seed_from_u64(seed);
            let mut rollout = state.clone();
            for _ in 0..8 {
                if matches!(rollout.status(), ludo_domain::GameStatus::Finished) {
                    break;
                }
                let command = match rollout.phase() {
                    TurnPhase::AwaitingRoll => {
                        let value = rng.random_range(1..=6);
                        let Some(dice) = ludo_domain::DiceValue::new(value) else {
                            break;
                        };
                        GameCommand::Roll(dice)
                    }
                    TurnPhase::AwaitingMove { legal_tokens, .. } => {
                        let Some(chosen) =
                            legal_tokens.get(rng.random_range(0..legal_tokens.len()))
                        else {
                            break;
                        };
                        GameCommand::Move(*chosen)
                    }
                };
                if rollout.apply(command).is_err() {
                    break;
                }
            }
            i64::from(opponent_model_score(&rollout, player))
        })
        .sum();
    i32::try_from(total / i64::from(simulations)).unwrap_or_default()
}

fn future_score(state: &GameState, roll: u8, evaluated_player: PlayerId) -> i32 {
    let Some(dice) = ludo_domain::DiceValue::new(roll) else {
        return 0;
    };
    let mut future = state.clone();
    if !matches!(future.phase(), TurnPhase::AwaitingRoll) {
        return 0;
    }
    let rolling_player = future.current().player.id;
    if future.apply(GameCommand::Roll(dice)).is_err() {
        return 0;
    }
    let mobility = match future.phase() {
        TurnPhase::AwaitingMove { legal_tokens, .. } => {
            i32::try_from(legal_tokens.len()).unwrap_or_default() * 3
        }
        TurnPhase::AwaitingRoll => 0,
    };
    if rolling_player == evaluated_player {
        mobility
    } else {
        -mobility
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use ludo_domain::{DiceValue, GameState, Rules, standard_players};

    use super::*;

    #[test]
    fn chooses_a_legal_token() {
        let Ok(mut state) = GameState::new(standard_players(), Rules::default()) else {
            return;
        };
        let Some(six) = DiceValue::new(6) else {
            return;
        };
        assert!(state.apply(GameCommand::Roll(six)).is_ok());
        let legal = match state.phase() {
            TurnPhase::AwaitingMove { legal_tokens, .. } => legal_tokens.clone(),
            TurnPhase::AwaitingRoll => Vec::new(),
        };
        let decision = ParallelBot::choose(&BotRequest::new(state, Difficulty::Hard));
        assert!(decision.token.is_some_and(|token| legal.contains(&token)));
        assert!(decision.simulations > 0);
    }

    #[test]
    fn awaiting_roll_returns_a_zero_work_no_move_decision() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let decision = ParallelBot::choose(&BotRequest::new(state, Difficulty::Hard));
        assert_eq!(decision.token, None);
        assert_eq!(decision.simulations, 0);
        assert_eq!(decision.revision, 0);
    }

    #[test]
    fn reported_simulations_match_all_candidate_rollouts() {
        let mut state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        assert!(state.apply(GameCommand::Roll(six)).is_ok());

        let easy = ParallelBot::choose(&BotRequest::new(state.clone(), Difficulty::Easy));
        let medium = ParallelBot::choose(&BotRequest::new(state.clone(), Difficulty::Medium));
        let zero_budget =
            ParallelBot::choose(&BotRequest::new(state, Difficulty::Hard).with_thinking_time_ms(0));
        assert_eq!(easy.simulations, 0);
        assert_eq!(medium.simulations, 24);
        assert_eq!(zero_budget.simulations, 0);
    }

    #[test]
    fn equal_easy_moves_have_a_stable_lowest_token_tie_break() {
        let mut state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        assert!(state.apply(GameCommand::Roll(six)).is_ok());
        let decision = ParallelBot::choose(&BotRequest::new(state, Difficulty::Easy));
        assert_eq!(
            decision
                .token
                .and_then(|token| u8::try_from(token.index()).ok()),
            Some(0)
        );
    }

    #[test]
    fn worker_returns_revision_tagged_decisions_off_thread() {
        let mut state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        assert!(state.apply(GameCommand::Roll(six)).is_ok());
        let revision = state.revision();
        let worker = BotWorker::new();
        assert!(
            worker
                .request(BotRequest::new(state, Difficulty::Easy))
                .is_ok()
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        let decision = loop {
            if let Some(decision) = worker.try_decision() {
                break decision;
            }
            assert!(Instant::now() < deadline, "AI worker timed out");
            std::thread::yield_now();
        };
        assert_eq!(decision.revision, revision);
        assert!(decision.token.is_some());
    }
}
