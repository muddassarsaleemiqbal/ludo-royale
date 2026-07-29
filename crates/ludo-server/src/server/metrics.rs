//! Low-overhead process metrics in Prometheus text format.

use super::{AppState, AtomicU64, Ordering, State};

#[derive(Default)]
pub(super) struct Metrics {
    pub(super) websocket_connections_total: AtomicU64,
    pub(super) websocket_connections_active: AtomicU64,
    pub(super) commands_total: AtomicU64,
    pub(super) command_errors_total: AtomicU64,
    pub(super) outbound_dropped_total: AtomicU64,
    pub(super) matches_completed_total: AtomicU64,
}

pub(super) async fn metrics(State(state): State<AppState>) -> String {
    let metrics = &state.metrics;
    format!(
        concat!(
            "# TYPE ludo_websocket_connections_total counter\n",
            "ludo_websocket_connections_total {}\n",
            "# TYPE ludo_websocket_connections_active gauge\n",
            "ludo_websocket_connections_active {}\n",
            "# TYPE ludo_commands_total counter\n",
            "ludo_commands_total {}\n",
            "# TYPE ludo_command_errors_total counter\n",
            "ludo_command_errors_total {}\n",
            "# TYPE ludo_outbound_dropped_total counter\n",
            "ludo_outbound_dropped_total {}\n",
            "# TYPE ludo_matches_completed_total counter\n",
            "ludo_matches_completed_total {}\n"
        ),
        metrics.websocket_connections_total.load(Ordering::Relaxed),
        metrics.websocket_connections_active.load(Ordering::Relaxed),
        metrics.commands_total.load(Ordering::Relaxed),
        metrics.command_errors_total.load(Ordering::Relaxed),
        metrics.outbound_dropped_total.load(Ordering::Relaxed),
        metrics.matches_completed_total.load(Ordering::Relaxed),
    )
}
