//! GUI-neutral view models.

use ludo_domain::{
    GameEvent, GameState, GameStatus, PlayerColor, PlayerId, TokenId, TokenPosition, TurnPhase,
};

/// A framework-neutral visual beat derived from domain facts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationCue {
    /// One face in the dice-roll sequence.
    Dice(u8),
    /// A token's intermediate logical position.
    TokenAt {
        /// Owner.
        player: PlayerId,
        /// Token within the owner.
        token: TokenId,
        /// Position to display.
        position: TokenPosition,
    },
    /// A capture accent for the token being sent home.
    Capture {
        /// Captured player.
        player: PlayerId,
        /// Captured token.
        token: TokenId,
    },
    /// A placement celebration.
    Ranked {
        /// Ranked player.
        player: PlayerId,
        /// One-based place.
        place: u8,
    },
    /// The active-player accent should move.
    Turn(PlayerId),
    /// The match-ending celebration.
    Victory(PlayerId),
}

/// One timed item in an animation sequence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationFrame {
    /// Visual change to apply.
    pub cue: AnimationCue,
    /// Delay before applying the change.
    pub delay_ms: u64,
}

/// Converts domain events into a deterministic animation timeline.
///
/// The domain remains unaware of timing and GUI concepts, while every client
/// can render the same semantic sequence in a platform-appropriate way.
#[must_use]
pub fn animation_frames(events: &[GameEvent]) -> Vec<AnimationFrame> {
    let mut frames = Vec::new();
    for event in events {
        match event {
            GameEvent::DiceRolled { value, .. } => {
                let final_value = value.get();
                for face in [2, 5, 1, 6, 3, final_value] {
                    frames.push(AnimationFrame {
                        cue: AnimationCue::Dice(face),
                        delay_ms: 45,
                    });
                }
            }
            GameEvent::TokenMoved {
                player,
                token,
                from,
                to,
            } => {
                for position in movement_positions(*from, *to) {
                    frames.push(AnimationFrame {
                        cue: AnimationCue::TokenAt {
                            player: *player,
                            token: *token,
                            position,
                        },
                        delay_ms: 72,
                    });
                }
            }
            GameEvent::TokenCaptured { player, token, .. } => {
                frames.push(AnimationFrame {
                    cue: AnimationCue::Capture {
                        player: *player,
                        token: *token,
                    },
                    delay_ms: 150,
                });
            }
            GameEvent::PlayerRanked { player, place } => {
                frames.push(AnimationFrame {
                    cue: AnimationCue::Ranked {
                        player: *player,
                        place: *place,
                    },
                    delay_ms: 180,
                });
            }
            GameEvent::TurnChanged { player } => frames.push(AnimationFrame {
                cue: AnimationCue::Turn(*player),
                delay_ms: 100,
            }),
            GameEvent::GameFinished { rankings } => {
                if let Some(winner) = rankings.first() {
                    frames.push(AnimationFrame {
                        cue: AnimationCue::Victory(*winner),
                        delay_ms: 260,
                    });
                }
            }
        }
    }
    frames
}

fn movement_positions(from: TokenPosition, to: TokenPosition) -> Vec<TokenPosition> {
    match (from, to) {
        (TokenPosition::Yard, TokenPosition::Path(0)) => vec![TokenPosition::Path(0)],
        (TokenPosition::Path(start), TokenPosition::Path(end)) => (start.saturating_add(1)..=end)
            .map(TokenPosition::Path)
            .collect(),
        (TokenPosition::Path(start), TokenPosition::Finished) => {
            let mut positions = (start.saturating_add(1)..=56)
                .map(TokenPosition::Path)
                .collect::<Vec<_>>();
            positions.push(TokenPosition::Finished);
            positions
        }
        (_, destination) => vec![destination],
    }
}

/// One token prepared for rendering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TokenViewModel {
    /// Owner.
    pub player: PlayerId,
    /// Token within owner.
    pub token: TokenId,
    /// Owner color.
    pub color: PlayerColor,
    /// Logical position.
    pub position: TokenPosition,
    /// Whether the token is an available user action.
    pub selectable: bool,
}

/// One player card prepared for rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerViewModel {
    /// ID.
    pub id: PlayerId,
    /// Display name.
    pub name: String,
    /// Board color.
    pub color: PlayerColor,
    /// Whether this is the current turn.
    pub active: bool,
    /// Number of completed tokens.
    pub finished: usize,
}

/// Complete presentation snapshot with no GUI framework types.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameViewModel {
    /// Player cards.
    pub players: Vec<PlayerViewModel>,
    /// All tokens.
    pub tokens: Vec<TokenViewModel>,
    /// Status line.
    pub status: String,
    /// Last/current dice value.
    pub dice: Option<u8>,
    /// Whether the roll action is available.
    pub can_roll: bool,
    /// Current state revision.
    pub revision: u64,
    /// Winning player, when complete.
    pub winner: Option<PlayerId>,
}

impl From<&GameState> for GameViewModel {
    fn from(state: &GameState) -> Self {
        let current = state.current().player.id;
        let legal = match state.phase() {
            TurnPhase::AwaitingMove { legal_tokens, .. } => legal_tokens.as_slice(),
            TurnPhase::AwaitingRoll => &[],
        };
        let dice = match state.phase() {
            TurnPhase::AwaitingMove { dice, .. } => Some(dice.get()),
            TurnPhase::AwaitingRoll => None,
        };
        let players = state
            .players()
            .iter()
            .map(|player| PlayerViewModel {
                id: player.player.id,
                name: player.player.name.clone(),
                color: player.player.color,
                active: player.player.id == current,
                finished: player
                    .tokens
                    .iter()
                    .filter(|token| matches!(token.position, TokenPosition::Finished))
                    .count(),
            })
            .collect();
        let tokens = state
            .players()
            .iter()
            .flat_map(|player| {
                player.tokens.iter().map(|token| TokenViewModel {
                    player: player.player.id,
                    token: token.id,
                    color: player.player.color,
                    position: token.position,
                    selectable: player.player.id == current && legal.contains(&token.id),
                })
            })
            .collect();
        let winner = if matches!(state.status(), GameStatus::Finished) {
            state.rankings().first().copied()
        } else {
            None
        };
        let status = if let Some(player) = winner {
            let name = &state.players()[player.index()].player.name;
            format!("{name} wins the match!")
        } else {
            let name = &state.current().player.name;
            match state.phase() {
                TurnPhase::AwaitingRoll => format!("{name}'s turn — roll the dice"),
                TurnPhase::AwaitingMove { .. } => format!("{name} — choose a glowing token"),
            }
        };
        Self {
            players,
            tokens,
            status,
            dice,
            can_roll: matches!(state.status(), GameStatus::Playing)
                && matches!(state.phase(), TurnPhase::AwaitingRoll),
            revision: state.revision(),
            winner,
        }
    }
}

#[cfg(test)]
mod tests {
    use ludo_domain::{
        DiceValue, GameCommand, GameEvent, GameState, PlayerId, Rules, TokenId, TokenPosition,
        standard_players,
    };

    use super::{AnimationCue, GameViewModel, animation_frames};

    #[test]
    fn dice_animation_always_ends_on_the_actual_roll() {
        let player = PlayerId::new(0).unwrap_or_else(|| std::process::abort());
        let value = DiceValue::new(4).unwrap_or_else(|| std::process::abort());
        let frames = animation_frames(&[GameEvent::DiceRolled { player, value }]);

        assert_eq!(
            frames.last().map(|frame| frame.cue),
            Some(AnimationCue::Dice(4))
        );
    }

    #[test]
    fn path_animation_visits_each_intermediate_position() {
        let player = PlayerId::new(0).unwrap_or_else(|| std::process::abort());
        let token = TokenId::new(0).unwrap_or_else(|| std::process::abort());
        let frames = animation_frames(&[GameEvent::TokenMoved {
            player,
            token,
            from: TokenPosition::Path(3),
            to: TokenPosition::Path(6),
        }]);

        let positions = frames
            .iter()
            .filter_map(|frame| match frame.cue {
                AnimationCue::TokenAt { position, .. } => Some(position),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            positions,
            vec![
                TokenPosition::Path(4),
                TokenPosition::Path(5),
                TokenPosition::Path(6)
            ]
        );
    }

    #[test]
    fn initial_view_model_contains_every_player_and_token() {
        let state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let view = GameViewModel::from(&state);
        assert_eq!(view.players.len(), 4);
        assert_eq!(view.tokens.len(), 16);
        assert!(view.can_roll);
        assert_eq!(view.dice, None);
        assert_eq!(view.winner, None);
        assert!(view.players[0].active);
        assert!(view.tokens.iter().all(|token| !token.selectable));
    }

    #[test]
    fn rolled_six_exposes_only_the_domain_legal_actions() {
        let mut state = GameState::new(standard_players(), Rules::default())
            .unwrap_or_else(|_| std::process::abort());
        let six = DiceValue::new(6).unwrap_or_else(|| std::process::abort());
        assert!(state.apply(GameCommand::Roll(six)).is_ok());
        let view = GameViewModel::from(&state);
        assert!(!view.can_roll);
        assert_eq!(view.dice, Some(6));
        assert_eq!(
            view.tokens.iter().filter(|token| token.selectable).count(),
            4
        );
    }

    #[test]
    fn capture_rank_turn_and_victory_events_keep_their_semantic_order() {
        let player = PlayerId::new(0).unwrap_or_else(|| std::process::abort());
        let opponent = PlayerId::new(1).unwrap_or_else(|| std::process::abort());
        let token = TokenId::new(2).unwrap_or_else(|| std::process::abort());
        let frames = animation_frames(&[
            GameEvent::TokenCaptured {
                by: player,
                player: opponent,
                token,
            },
            GameEvent::PlayerRanked { player, place: 1 },
            GameEvent::TurnChanged { player: opponent },
            GameEvent::GameFinished {
                rankings: vec![player, opponent],
            },
        ]);
        assert_eq!(
            frames.iter().map(|frame| frame.cue).collect::<Vec<_>>(),
            vec![
                AnimationCue::Capture {
                    player: opponent,
                    token,
                },
                AnimationCue::Ranked { player, place: 1 },
                AnimationCue::Turn(opponent),
                AnimationCue::Victory(player),
            ]
        );
    }
}
