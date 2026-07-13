//! CLI signal handling and operator-cancellation state.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use indicatif::MultiProgress;
use tokio_util::sync::CancellationToken;

pub(crate) fn spawn_signal_handlers(
    mp: &MultiProgress,
    cancel: &CancellationToken,
    cancellation_state: &CliCancellationState,
) {
    let cancel_clone = cancel.clone();
    let state_clone = cancellation_state.clone();
    let mp_clone = mp.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            state_clone.mark_user_requested();
            mp_clone
                .println("Shutting down gracefully... (waiting for in-flight tasks)")
                .ok();
            cancel_clone.cancel();
        }
    });

    // Register before spawning so failures surface here rather than panicking
    // inside a detached task where tokio silently swallows the panic.
    let sig = match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::window_change()) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("Could not register SIGWINCH handler (resize redraw disabled): {e}");
            return;
        }
    };
    let winch_cancel = cancel.clone();
    let winch_mp = mp.clone();
    tokio::spawn(async move {
        let mut sig = sig;
        loop {
            tokio::select! {
                _ = sig.recv() => { winch_mp.println("").ok(); }
                _ = winch_cancel.cancelled() => break,
            }
        }
    });
}

/// Tracks cancellation that came from the operator rather than internal or
/// normal end-of-batch shutdown.
#[derive(Clone, Debug, Default)]
pub(crate) struct CliCancellationState {
    user_requested: Arc<AtomicBool>,
}

impl CliCancellationState {
    pub(crate) fn mark_user_requested(&self) {
        self.user_requested.store(true, Ordering::Release);
    }

    pub(crate) fn user_requested(&self) -> bool {
        self.user_requested.load(Ordering::Acquire)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_cancellation_state_distinguishes_user_and_internal_shutdown() {
        let state = CliCancellationState::default();
        let token = tokio_util::sync::CancellationToken::new();
        assert!(!state.user_requested());

        token.cancel();
        assert!(token.is_cancelled());
        assert!(
            !state.user_requested(),
            "internal cancellation is not Ctrl-C"
        );

        state.mark_user_requested();
        assert!(state.user_requested());
    }

    #[test]
    fn cli_cancellation_state_normal_shutdown_is_not_user_cancelled() {
        let state = CliCancellationState::default();
        let token = tokio_util::sync::CancellationToken::new();
        let user_requested_before_shutdown = state.user_requested();
        token.cancel();
        assert!(!user_requested_before_shutdown);
        assert!(!state.user_requested());
    }
}
