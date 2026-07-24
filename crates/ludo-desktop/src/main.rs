use std::{sync::Arc, time::Duration};

use gpui::{
    App, Application, Bounds, Context, Rgba, Task, Timer, Window, WindowBounds, WindowOptions, div,
    prelude::*, px, rgb, size,
};
use gpui_component::{
    ActiveTheme, Disableable, Root, Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use ludo_ai::{BotRequest, BotWorker, Difficulty};
use ludo_application::GameSession;
use ludo_domain::{
    Controller, GameState, GameStatus, PlayerColor, Rules, TokenId, TokenPosition, TurnPhase,
    standard_players,
};
use ludo_infrastructure::RandomDice;
use ludo_presentation::{GameViewModel, PlayerViewModel, TokenViewModel};

const CELL: f32 = 34.0;
const BOARD: f32 = CELL * 15.0;

struct GameView {
    session: GameSession,
    view_model: GameViewModel,
    bot_worker: BotWorker,
    bot_pending: bool,
    last_error: Option<String>,
    bot_task: Option<Task<()>>,
}

impl GameView {
    fn new() -> Self {
        let state = create_game();
        let view_model = GameViewModel::from(&state);
        Self {
            session: GameSession::new(state, Arc::new(RandomDice)),
            view_model,
            bot_worker: BotWorker::new(),
            bot_pending: false,
            last_error: None,
            bot_task: None,
        }
    }

    fn new_game(&mut self, cx: &mut Context<Self>) {
        let state = create_game();
        self.session.restore(state);
        self.bot_pending = false;
        self.last_error = None;
        self.refresh(cx);
    }

    fn roll(&mut self, cx: &mut Context<Self>) {
        if self.bot_pending || !self.view_model.can_roll {
            return;
        }
        match self.session.roll() {
            Ok(_) => self.last_error = None,
            Err(error) => self.last_error = Some(error.to_string()),
        }
        self.refresh(cx);
        self.drive_bot(cx);
    }

    fn move_token(&mut self, token: TokenId, cx: &mut Context<Self>) {
        if self.bot_pending {
            return;
        }
        match self.session.move_token(token) {
            Ok(_) => self.last_error = None,
            Err(error) => self.last_error = Some(error.to_string()),
        }
        self.refresh(cx);
        self.drive_bot(cx);
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        self.view_model = GameViewModel::from(self.session.state());
        cx.notify();
    }

    fn drive_bot(&mut self, cx: &mut Context<Self>) {
        if self.bot_pending
            || !matches!(self.session.state().status, GameStatus::Playing)
            || !matches!(
                self.session.state().current().player.controller,
                Controller::Bot
            )
        {
            return;
        }
        self.bot_pending = true;
        match self.session.state().phase {
            TurnPhase::AwaitingRoll => {
                self.bot_task = Some(cx.spawn(async move |view, cx| {
                    Timer::after(Duration::from_millis(420)).await;
                    let _ = cx.update(|cx| {
                        let Some(view) = view.upgrade() else {
                            return;
                        };
                        view.update(cx, |this, cx| {
                            this.bot_pending = false;
                            match this.session.roll() {
                                Ok(_) => this.last_error = None,
                                Err(error) => this.last_error = Some(error.to_string()),
                            }
                            this.refresh(cx);
                            this.drive_bot(cx);
                        });
                    });
                }));
            }
            TurnPhase::AwaitingMove { .. } => {
                let request = BotRequest {
                    state: self.session.state().clone(),
                    difficulty: Difficulty::Hard,
                };
                if self.bot_worker.request(request).is_err() {
                    self.bot_pending = false;
                    return;
                }
                self.bot_task = Some(cx.spawn(async move |view, cx| {
                    loop {
                        Timer::after(Duration::from_millis(24)).await;
                        let done = cx
                            .update(|cx| {
                                let Some(view) = view.upgrade() else {
                                    return true;
                                };
                                view.update(cx, |this, cx| {
                                    let Some(decision) = this.bot_worker.try_decision() else {
                                        return false;
                                    };
                                    if decision.revision == this.session.state().revision
                                        && decision.player
                                            == this.session.state().current().player.id
                                        && let Some(token) = decision.token
                                    {
                                        match this.session.move_token(token) {
                                            Ok(_) => this.last_error = None,
                                            Err(error) => {
                                                this.last_error = Some(error.to_string());
                                            }
                                        }
                                    }
                                    this.bot_pending = false;
                                    this.refresh(cx);
                                    this.drive_bot(cx);
                                    true
                                })
                            })
                            .unwrap_or(true);
                        if done {
                            break;
                        }
                    }
                }));
            }
        }
    }

    fn render_board(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mut board = div()
            .relative()
            .size(px(BOARD))
            .flex()
            .flex_col()
            .overflow_hidden()
            .rounded_xl()
            .border_2()
            .border_color(rgb(0x001d_2a24))
            .shadow_2xl()
            .bg(rgb(0x00f8_f3e7));

        for row in 0_u8..15 {
            let mut row_element = div().flex().h(px(CELL));
            for column in 0_u8..15 {
                row_element = row_element.child(render_cell(row, column));
            }
            board = board.child(row_element);
        }

        for token in &self.view_model.tokens {
            if let Some((row, column)) = token_coordinate(*token) {
                let player = token.player;
                let token_id = token.token;
                let selectable = token.selectable
                    && matches!(
                        self.session.state().current().player.controller,
                        Controller::Human
                    );
                let token_element = div()
                    .id(("token", player.index() * 4 + token_id.index()))
                    .absolute()
                    .left(px(f32::from(column) * CELL + 5.0))
                    .top(px(f32::from(row) * CELL + 4.0))
                    .size(px(CELL - 9.0))
                    .rounded_full()
                    .border_2()
                    .border_color(rgb(0x00ff_ffff))
                    .bg(color(token.color))
                    .shadow_lg()
                    .flex()
                    .items_center()
                    .justify_center()
                    .text_xs()
                    .font_semibold()
                    .text_color(rgb(0x00ff_ffff))
                    .cursor_pointer()
                    .when(selectable, |element| {
                        element
                            .border_3()
                            .border_color(rgb(0x00ff_ffff))
                            .shadow_xl()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.move_token(token_id, cx);
                            }))
                    })
                    .child((token_id.index() + 1).to_string());
                board = board.child(token_element);
            }
        }
        board
    }

    fn player_card(player: &PlayerViewModel) -> impl IntoElement {
        let tint = color(player.color);
        div()
            .w_full()
            .p_3()
            .rounded_xl()
            .border_1()
            .border_color(if player.active {
                tint
            } else {
                rgb(0x0035_423b)
            })
            .bg(if player.active {
                rgba_from_rgb(tint, 0.20)
            } else {
                rgb(0x0018_211d)
            })
            .shadow_md()
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().size_3().rounded_full().bg(tint))
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .font_semibold()
                            .text_color(rgb(0x00f3_f6f4))
                            .child(player.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(rgb(0x009e_b0a7))
                            .child(format!("{}/4 home", player.finished)),
                    ),
            )
    }

    fn dice_face(&self) -> impl IntoElement {
        let value = self.view_model.dice.unwrap_or(0);
        div()
            .size(px(84.0))
            .rounded_2xl()
            .bg(rgb(0x00f9_f5e9))
            .border_2()
            .border_color(rgb(0x00d8_cda9))
            .shadow_xl()
            .flex()
            .items_center()
            .justify_center()
            .text_3xl()
            .font_bold()
            .text_color(rgb(0x0017_221c))
            .child(if value == 0 {
                "•".to_owned()
            } else {
                value.to_string()
            })
    }
}

impl Render for GameView {
    #[allow(
        clippy::too_many_lines,
        reason = "the declarative GPUI view tree is clearest when its layout remains contiguous"
    )]
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let can_human_roll = self.view_model.can_roll
            && !self.bot_pending
            && matches!(
                self.session.state().current().player.controller,
                Controller::Human
            );
        let status = self
            .last_error
            .clone()
            .unwrap_or_else(|| self.view_model.status.clone());

        div()
            .size_full()
            .min_w(px(900.0))
            .min_h(px(680.0))
            .bg(rgb(0x000d_1712))
            .text_color(cx.theme().foreground)
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(72.0))
                    .px_8()
                    .flex()
                    .items_center()
                    .border_b_1()
                    .border_color(rgb(0x0026_362e))
                    .bg(rgb(0x0011_1d17))
                    .child(
                        div()
                            .flex_1()
                            .child(
                                div()
                                    .text_2xl()
                                    .font_bold()
                                    .text_color(rgb(0x00f4_d67b))
                                    .child("LUDO ROYALE"),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(rgb(0x008e_a096))
                                    .child("Native Rust • GPUI • Parallel AI"),
                            ),
                    )
                    .child(
                        Button::new("new-game")
                            .label("New game")
                            .on_click(cx.listener(|this, _, _, cx| this.new_game(cx))),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .flex()
                    .items_center()
                    .justify_center()
                    .gap_8()
                    .p_6()
                    .child(
                        div()
                            .w(px(230.0))
                            .flex()
                            .flex_col()
                            .gap_3()
                            .child(
                                div()
                                    .mb_2()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(rgb(0x0074_877c))
                                    .child("PLAYERS"),
                            )
                            .children(self.view_model.players.iter().map(Self::player_card)),
                    )
                    .child(
                        div()
                            .p_4()
                            .rounded_2xl()
                            .bg(rgb(0x0075_482c))
                            .border_1()
                            .border_color(rgb(0x009d_6844))
                            .shadow_2xl()
                            .child(self.render_board(cx)),
                    )
                    .child(
                        div()
                            .w(px(220.0))
                            .flex()
                            .flex_col()
                            .items_center()
                            .gap_5()
                            .child(
                                div()
                                    .text_xs()
                                    .font_semibold()
                                    .text_color(rgb(0x0074_877c))
                                    .child("CURRENT ROLL"),
                            )
                            .child(self.dice_face())
                            .child(
                                Button::new("roll-dice")
                                    .primary()
                                    .large()
                                    .label(if self.bot_pending {
                                        "Bot thinking…"
                                    } else {
                                        "Roll dice"
                                    })
                                    .disabled(!can_human_roll)
                                    .on_click(cx.listener(|this, _, _, cx| this.roll(cx))),
                            )
                            .child(
                                div()
                                    .w_full()
                                    .p_4()
                                    .rounded_xl()
                                    .bg(rgb(0x0015_231c))
                                    .border_1()
                                    .border_color(rgb(0x002a_3a32))
                                    .text_sm()
                                    .text_center()
                                    .text_color(rgb(0x00d9_e3dd))
                                    .child(status),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_center()
                                    .text_color(rgb(0x0071_847a))
                                    .child("Roll a six to leave the yard. Safe stars protect your tokens."),
                            ),
                    ),
            )
    }
}

fn create_game() -> GameState {
    match GameState::new(standard_players(), Rules::default()) {
        Ok(state) => state,
        Err(error) => {
            tracing::error!(%error, "standard game configuration is invalid");
            std::process::abort();
        }
    }
}

fn render_cell(row: u8, column: u8) -> impl IntoElement {
    let background = cell_color(row, column);
    let is_track = is_track_cell(row, column);
    let is_safe = matches!(
        (row, column),
        (6, 1 | 12) | (1 | 12, 8) | (8, 13 | 2) | (13 | 2, 6)
    );
    div()
        .size(px(CELL))
        .flex_none()
        .bg(background)
        .border_1()
        .border_color(if is_track {
            rgb(0x00b9_b29e)
        } else {
            background
        })
        .flex()
        .items_center()
        .justify_center()
        .text_sm()
        .font_bold()
        .text_color(rgb(0x00ff_ffff))
        .when(is_safe, |element| element.child("★"))
}

fn cell_color(row: u8, column: u8) -> Rgba {
    if row < 6 && column < 6 {
        return rgb(0x00d9_404b);
    }
    if row < 6 && column > 8 {
        return rgb(0x002b_ab70);
    }
    if row > 8 && column > 8 {
        return rgb(0x00e9_b934);
    }
    if row > 8 && column < 6 {
        return rgb(0x0034_7ed1);
    }
    if (row == 7 && (1..=5).contains(&column)) || (row == 6 && column == 1) {
        return color(PlayerColor::Red);
    }
    if (column == 7 && (1..=5).contains(&row)) || (row == 1 && column == 8) {
        return color(PlayerColor::Green);
    }
    if (row == 7 && (9..=13).contains(&column)) || (row == 8 && column == 13) {
        return color(PlayerColor::Yellow);
    }
    if (column == 7 && (9..=13).contains(&row)) || (row == 13 && column == 6) {
        return color(PlayerColor::Blue);
    }
    if (6..=8).contains(&row) && (6..=8).contains(&column) {
        return match (row, column) {
            (6 | 7, 6) => color(PlayerColor::Red),
            (6, 7 | 8) => color(PlayerColor::Green),
            (7 | 8, 8) => color(PlayerColor::Yellow),
            _ => color(PlayerColor::Blue),
        };
    }
    rgb(0x00fa_f7ed)
}

fn is_track_cell(row: u8, column: u8) -> bool {
    ((6..=8).contains(&row) && !(6..=8).contains(&column))
        || ((6..=8).contains(&column) && !(6..=8).contains(&row))
}

fn token_coordinate(token: TokenViewModel) -> Option<(u8, u8)> {
    match token.position {
        TokenPosition::Yard => Some(yard_coordinate(token.color, token.token)),
        TokenPosition::Path(progress) if progress < 52 => {
            let index = (token.color.start_index() + progress) % 52;
            Some(TRACK[index as usize])
        }
        TokenPosition::Path(progress) => {
            let offset = progress.saturating_sub(52);
            match token.color {
                PlayerColor::Red => Some((7, 1 + offset)),
                PlayerColor::Green => Some((1 + offset, 7)),
                PlayerColor::Yellow => Some((7, 13 - offset)),
                PlayerColor::Blue => Some((13 - offset, 7)),
            }
        }
        TokenPosition::Finished => None,
    }
}

fn yard_coordinate(color: PlayerColor, token: TokenId) -> (u8, u8) {
    let offset = match token.index() {
        0 => (0, 0),
        1 => (0, 2),
        2 => (2, 0),
        _ => (2, 2),
    };
    let base = match color {
        PlayerColor::Red => (2, 2),
        PlayerColor::Green => (2, 10),
        PlayerColor::Yellow => (10, 10),
        PlayerColor::Blue => (10, 2),
    };
    (base.0 + offset.0, base.1 + offset.1)
}

fn color(color: PlayerColor) -> Rgba {
    match color {
        PlayerColor::Red => rgb(0x00df_3545),
        PlayerColor::Green => rgb(0x001f_a966),
        PlayerColor::Yellow => rgb(0x00e2_aa21),
        PlayerColor::Blue => rgb(0x0028_78cf),
    }
}

fn rgba_from_rgb(color: Rgba, alpha: f32) -> Rgba {
    Rgba { a: alpha, ..color }
}

const TRACK: [(u8, u8); 52] = [
    (6, 1),
    (6, 2),
    (6, 3),
    (6, 4),
    (6, 5),
    (5, 6),
    (4, 6),
    (3, 6),
    (2, 6),
    (1, 6),
    (0, 6),
    (0, 7),
    (0, 8),
    (1, 8),
    (2, 8),
    (3, 8),
    (4, 8),
    (5, 8),
    (6, 9),
    (6, 10),
    (6, 11),
    (6, 12),
    (6, 13),
    (6, 14),
    (7, 14),
    (8, 14),
    (8, 13),
    (8, 12),
    (8, 11),
    (8, 10),
    (8, 9),
    (9, 8),
    (10, 8),
    (11, 8),
    (12, 8),
    (13, 8),
    (14, 8),
    (14, 7),
    (14, 6),
    (13, 6),
    (12, 6),
    (11, 6),
    (10, 6),
    (9, 6),
    (8, 5),
    (8, 4),
    (8, 3),
    (8, 2),
    (8, 1),
    (8, 0),
    (7, 0),
    (6, 0),
];

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ludo=info".into()),
        )
        .init();

    Application::new().run(|cx: &mut App| {
        gpui_component::init(cx);
        let bounds = Bounds::centered(None, size(px(1120.0), px(760.0)), cx);
        let result = cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some("Ludo Royale".into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| {
                let game = cx.new(|_| GameView::new());
                cx.new(|cx| Root::new(game, window, cx))
            },
        );
        if let Err(error) = result {
            tracing::error!(%error, "failed to open the Ludo window");
            return;
        }
        cx.activate(true);
    });
}
