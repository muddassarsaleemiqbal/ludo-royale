//! GUI-neutral view models.

use ludo_domain::{
    GameState, GameStatus, PlayerColor, PlayerId, TokenId, TokenPosition, TurnPhase,
};

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
        let legal = match &state.phase {
            TurnPhase::AwaitingMove { legal_tokens, .. } => legal_tokens.as_slice(),
            TurnPhase::AwaitingRoll => &[],
        };
        let dice = match &state.phase {
            TurnPhase::AwaitingMove { dice, .. } => Some(dice.get()),
            TurnPhase::AwaitingRoll => None,
        };
        let players = state
            .players
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
            .players
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
        let winner = match state.status {
            GameStatus::Playing => None,
            GameStatus::Won(player) => Some(player),
        };
        let status = if let Some(player) = winner {
            let name = &state.players[player.index()].player.name;
            format!("{name} wins the match!")
        } else {
            let name = &state.current().player.name;
            match state.phase {
                TurnPhase::AwaitingRoll => format!("{name}'s turn — roll the dice"),
                TurnPhase::AwaitingMove { .. } => format!("{name} — choose a glowing token"),
            }
        };
        Self {
            players,
            tokens,
            status,
            dice,
            can_roll: winner.is_none() && matches!(state.phase, TurnPhase::AwaitingRoll),
            revision: state.revision,
            winner,
        }
    }
}
