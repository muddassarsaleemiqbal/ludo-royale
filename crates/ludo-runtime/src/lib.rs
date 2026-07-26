//! Platform-neutral game orchestration.

use ludo_ai::{BotDecision, BotRequest, Difficulty};
use std::sync::Arc;

use ludo_application::{ApplicationError, DiceSource, GameSession};
use ludo_domain::{
    Controller, DiceValue, GameCommand, GameState, GameStatus, PlayerId, Rules, TokenId, TurnPhase,
    standard_players,
};
use ludo_presentation::GameViewModel;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// A stable effect identifier used to reject stale asynchronous results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EffectId(pub u64);

/// Input accepted from any user-interface adapter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UiAction {
    /// Start a new standard match.
    NewGame,
    /// Request a human dice roll.
    Roll,
    /// Select one human-controlled token.
    SelectToken(TokenId),
    /// Continue an automatically controlled turn after a presentation delay.
    ContinueBot(EffectId),
    /// Complete a platform-generated dice roll.
    DiceReady {
        /// Effect being completed.
        effect: EffectId,
        /// Generated value.
        value: DiceValue,
    },
    /// Complete background bot evaluation.
    BotReady {
        /// Effect being completed.
        effect: EffectId,
        /// Evaluated decision.
        decision: BotDecision,
    },
}

/// Work that must be executed by the current platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RuntimeEffect {
    /// Wait without blocking the UI thread, then continue the bot.
    DelayBot {
        /// Stable effect identifier.
        effect: EffectId,
        /// Delay in milliseconds.
        milliseconds: u32,
    },
    /// Generate randomness using the best source on the platform.
    GenerateDice {
        /// Stable effect identifier.
        effect: EffectId,
    },
    /// Evaluate a move away from the UI thread where possible.
    EvaluateBot {
        /// Stable effect identifier.
        effect: EffectId,
        /// Immutable evaluation request.
        request: BotRequest,
    },
}

/// Complete result of processing one UI action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeUpdate {
    /// Framework-neutral render state.
    pub model: GameViewModel,
    /// Platform work created by the transition.
    pub effects: Vec<RuntimeEffect>,
}

/// Runtime failures.
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum RuntimeError {
    /// Application use case failed.
    #[error(transparent)]
    Application(#[from] ApplicationError),
    /// An action did not match the current controller or phase.
    #[error("action is not available in the current game state")]
    Unavailable,
    /// An asynchronous completion belongs to an older game revision.
    #[error("stale platform effect")]
    StaleEffect,
}

/// Portable state machine shared by native and WebAssembly interfaces.
pub struct GameRuntime {
    session: GameSession,
    next_effect: u64,
    pending: Option<PendingEffect>,
    last_error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingEffect {
    Delay(EffectId),
    Dice(EffectId),
    Bot(EffectId),
}

impl GameRuntime {
    /// Creates the standard local match.
    #[must_use]
    pub fn standard() -> Self {
        Self::from_state(standard_state())
    }

    /// Creates a runtime from an existing domain state.
    #[must_use]
    pub fn from_state(state: GameState) -> Self {
        Self {
            session: GameSession::new(state, Arc::new(PlatformDice)),
            next_effect: 1,
            pending: None,
            last_error: None,
        }
    }

    /// Current presentation snapshot.
    #[must_use]
    pub fn model(&self) -> GameViewModel {
        GameViewModel::new(
            self.session.state(),
            self.pending.is_some(),
            self.last_error.clone(),
        )
    }

    /// Current immutable domain snapshot for persistence and diagnostics.
    #[must_use]
    pub const fn state(&self) -> &GameState {
        self.session.state()
    }

    /// Processes one input and returns a new snapshot plus platform effects.
    ///
    /// # Errors
    ///
    /// Returns an error when the action is stale or unavailable.
    pub fn dispatch(&mut self, action: UiAction) -> Result<RuntimeUpdate, RuntimeError> {
        let result = self.apply_action(action);
        match result {
            Ok(effects) => {
                self.last_error = None;
                Ok(RuntimeUpdate {
                    model: self.model(),
                    effects,
                })
            }
            Err(RuntimeError::StaleEffect) => Ok(RuntimeUpdate {
                model: self.model(),
                effects: Vec::new(),
            }),
            Err(error) => {
                self.last_error = Some(error.to_string());
                Err(error)
            }
        }
    }

    fn apply_action(&mut self, action: UiAction) -> Result<Vec<RuntimeEffect>, RuntimeError> {
        match action {
            UiAction::NewGame => {
                self.session.restore(standard_state());
                self.pending = None;
                Ok(Vec::new())
            }
            UiAction::Roll => {
                self.require_human_roll()?;
                Ok(vec![self.dice_effect()])
            }
            UiAction::SelectToken(token) => {
                self.require_human_move()?;
                self.session.move_token(token)?;
                Ok(self.schedule_bot())
            }
            UiAction::ContinueBot(effect) => {
                self.take_pending(PendingEffect::Delay(effect))?;
                match self.session.state().phase() {
                    TurnPhase::AwaitingRoll => Ok(vec![self.dice_effect()]),
                    TurnPhase::AwaitingMove { .. } => Ok(vec![self.bot_effect()]),
                }
            }
            UiAction::DiceReady { effect, value } => {
                self.take_pending(PendingEffect::Dice(effect))?;
                self.session.execute(GameCommand::Roll(value))?;
                if self.is_bot_turn() {
                    Ok(self.schedule_bot())
                } else {
                    Ok(Vec::new())
                }
            }
            UiAction::BotReady { effect, decision } => {
                self.take_pending(PendingEffect::Bot(effect))?;
                if decision.revision != self.session.state().revision()
                    || decision.player != self.session.state().current().player.id
                {
                    return Err(RuntimeError::StaleEffect);
                }
                let token = decision.token.ok_or(RuntimeError::Unavailable)?;
                self.session.move_token(token)?;
                Ok(self.schedule_bot())
            }
        }
    }

    fn require_human_roll(&self) -> Result<(), RuntimeError> {
        if self.pending.is_none()
            && self.is_human_turn()
            && matches!(self.session.state().status(), GameStatus::Playing)
            && matches!(self.session.state().phase(), TurnPhase::AwaitingRoll)
        {
            Ok(())
        } else {
            Err(RuntimeError::StaleEffect)
        }
    }

    fn require_human_move(&self) -> Result<(), RuntimeError> {
        if self.pending.is_none()
            && self.is_human_turn()
            && matches!(self.session.state().phase(), TurnPhase::AwaitingMove { .. })
        {
            Ok(())
        } else {
            Err(RuntimeError::Unavailable)
        }
    }

    fn is_human_turn(&self) -> bool {
        matches!(
            self.session.state().current().player.controller,
            Controller::Human
        )
    }

    fn is_bot_turn(&self) -> bool {
        matches!(self.session.state().status(), GameStatus::Playing)
            && matches!(
                self.session.state().current().player.controller,
                Controller::Bot
            )
    }

    fn schedule_bot(&mut self) -> Vec<RuntimeEffect> {
        if !self.is_bot_turn() {
            return Vec::new();
        }
        let effect = self.effect_id();
        self.pending = Some(PendingEffect::Delay(effect));
        vec![RuntimeEffect::DelayBot {
            effect,
            milliseconds: 420,
        }]
    }

    fn dice_effect(&mut self) -> RuntimeEffect {
        let effect = self.effect_id();
        self.pending = Some(PendingEffect::Dice(effect));
        RuntimeEffect::GenerateDice { effect }
    }

    fn bot_effect(&mut self) -> RuntimeEffect {
        let effect = self.effect_id();
        self.pending = Some(PendingEffect::Bot(effect));
        RuntimeEffect::EvaluateBot {
            effect,
            request: BotRequest::new(self.session.state().clone(), Difficulty::Hard),
        }
    }

    fn effect_id(&mut self) -> EffectId {
        let id = EffectId(self.next_effect);
        self.next_effect = self.next_effect.saturating_add(1);
        id
    }

    fn take_pending(&mut self, expected: PendingEffect) -> Result<(), RuntimeError> {
        if self.pending == Some(expected) {
            self.pending = None;
            Ok(())
        } else {
            Err(RuntimeError::Unavailable)
        }
    }
}

struct PlatformDice;

impl DiceSource for PlatformDice {
    fn roll(&self) -> DiceValue {
        DiceValue::new(1).unwrap_or_else(|| std::process::abort())
    }
}

fn standard_state() -> GameState {
    GameState::new(standard_players(), Rules::default()).unwrap_or_else(|_| std::process::abort())
}

/// Converts a raw value supplied by a platform into a valid dice result.
#[must_use]
pub const fn dice_value(value: u8) -> Option<DiceValue> {
    DiceValue::new(value)
}

/// Helper used by effect executors to validate the expected player.
#[must_use]
pub fn current_player(state: &GameState) -> PlayerId {
    state.current().player.id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn human_roll_is_an_effect_then_a_transition() {
        let mut runtime = GameRuntime::standard();
        let update = runtime
            .dispatch(UiAction::Roll)
            .unwrap_or_else(|_| std::process::abort());
        let RuntimeEffect::GenerateDice { effect } = update.effects[0] else {
            std::process::abort();
        };
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        let update = runtime
            .dispatch(UiAction::DiceReady { effect, value: six })
            .unwrap_or_else(|_| std::process::abort());
        assert!(update.model.tokens.iter().any(|token| token.selectable));
    }

    #[test]
    fn stale_effect_is_rejected() {
        let mut runtime = GameRuntime::standard();
        assert!(
            runtime
                .dispatch(UiAction::DiceReady {
                    effect: EffectId(999),
                    value: DiceValue::new(1).unwrap_or_else(|| std::process::abort()),
                })
                .is_err()
        );
    }
}
