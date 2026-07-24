//! Parallel computer-player evaluation.

use flume::{Receiver, Sender};
use ludo_domain::{GameCommand, GameEvent, GameState, PlayerId, TokenId, TokenPosition, TurnPhase};
use rayon::prelude::*;

/// Difficulty controls the amount of parallel look-ahead work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Difficulty {
    /// Direct tactical heuristic.
    Easy,
    /// Tactical heuristic plus opponent exposure.
    Medium,
    /// Parallel deterministic rollouts over possible next rolls.
    Hard,
}

/// Revision-tagged immutable AI request.
#[derive(Debug, Clone)]
pub struct BotRequest {
    /// State snapshot.
    pub state: GameState,
    /// Difficulty.
    pub difficulty: Difficulty,
}

/// Worker result, safe to discard when its revision is stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BotDecision {
    /// Source state revision.
    pub revision: u64,
    /// Current player.
    pub player: PlayerId,
    /// Selected token, or none if the request was not in move phase.
    pub token: Option<TokenId>,
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
        let legal = match &state.phase {
            TurnPhase::AwaitingMove { legal_tokens, .. } => legal_tokens,
            TurnPhase::AwaitingRoll => {
                return BotDecision {
                    revision: state.revision,
                    player,
                    token: None,
                };
            }
        };
        let token = legal
            .par_iter()
            .map(|token| {
                (
                    *token,
                    evaluate(state, *token, request.difficulty),
                    token.index(),
                )
            })
            .max_by(|left, right| left.1.cmp(&right.1).then_with(|| right.2.cmp(&left.2)))
            .map(|result| result.0);
        BotDecision {
            revision: state.revision,
            player,
            token,
        }
    }
}

/// Long-lived non-blocking Rayon-backed AI adapter.
pub struct BotWorker {
    sender: Sender<BotRequest>,
    receiver: Receiver<BotDecision>,
}

impl BotWorker {
    /// Creates a bounded worker. New requests are evaluated on Rayon's global pool.
    #[must_use]
    pub fn new() -> Self {
        let (request_sender, request_receiver) = flume::bounded::<BotRequest>(4);
        let (decision_sender, decision_receiver) = flume::bounded::<BotDecision>(4);
        std::thread::Builder::new()
            .name("ludo-ai-dispatch".to_owned())
            .spawn(move || {
                while let Ok(request) = request_receiver.recv() {
                    let sender = decision_sender.clone();
                    rayon::spawn(move || {
                        let _ = sender.send(ParallelBot::choose(&request));
                    });
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
    pub fn request(&self, request: BotRequest) -> Result<(), BotRequest> {
        self.sender
            .try_send(request)
            .map_err(flume::TrySendError::into_inner)
    }

    /// Returns a completed decision when available.
    #[must_use]
    pub fn try_decision(&self) -> Option<BotDecision> {
        self.receiver.try_recv().ok()
    }
}

impl Default for BotWorker {
    fn default() -> Self {
        Self::new()
    }
}

fn evaluate(state: &GameState, token: TokenId, difficulty: Difficulty) -> i32 {
    let from = state.current().tokens[token.index()].position;
    let mut simulated = state.clone();
    let Ok(events) = simulated.apply(GameCommand::Move(token)) else {
        return i32::MIN;
    };
    let to = simulated.players[state.current_player].tokens[token.index()].position;
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
    match difficulty {
        Difficulty::Easy => tactical,
        Difficulty::Medium => tactical + safety_score(&simulated, player, token),
        Difficulty::Hard => {
            let future: i32 = (1_u8..=6)
                .into_par_iter()
                .map(|roll| future_score(&simulated, roll))
                .sum();
            tactical * 6 + safety_score(&simulated, player, token) * 3 + future
        }
    }
}

fn safety_score(state: &GameState, player: PlayerId, token: TokenId) -> i32 {
    match state.global_track_index(player, token) {
        Some(index) if [0, 8, 13, 21, 26, 34, 39, 47].contains(&index) => 18,
        Some(_) => 0,
        None => 12,
    }
}

fn future_score(state: &GameState, roll: u8) -> i32 {
    let Some(dice) = ludo_domain::DiceValue::new(roll) else {
        return 0;
    };
    let mut future = state.clone();
    if !matches!(future.phase, TurnPhase::AwaitingRoll) {
        return 0;
    }
    let Ok(events) = future.apply(GameCommand::Roll(dice)) else {
        return 0;
    };
    let mobility = match future.phase {
        TurnPhase::AwaitingMove {
            ref legal_tokens, ..
        } => i32::try_from(legal_tokens.len()).unwrap_or_default() * 3,
        TurnPhase::AwaitingRoll => 0,
    };
    mobility
        + i32::from(
            events
                .iter()
                .any(|event| matches!(event, GameEvent::DiceRolled { .. })),
        )
}

#[cfg(test)]
mod tests {
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
        let legal = match &state.phase {
            TurnPhase::AwaitingMove { legal_tokens, .. } => legal_tokens.clone(),
            TurnPhase::AwaitingRoll => Vec::new(),
        };
        let decision = ParallelBot::choose(&BotRequest {
            state,
            difficulty: Difficulty::Hard,
        });
        assert!(decision.token.is_some_and(|token| legal.contains(&token)));
    }
}
