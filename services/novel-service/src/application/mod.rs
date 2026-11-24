pub mod commands;
pub mod handlers;
pub mod source_file_cleanup;
pub mod world_series;

use std::{future::Future, time::Duration};
use tokio::{sync::watch, task::JoinHandle};

/// Bound from the stop signal until periodic workers are aborted.
pub const WORKER_STOP_GRACE: Duration = Duration::from_secs(5);

/// A sent `true` or a dropped sender both stop a periodic worker.
pub(crate) fn stop_requested(stop: &watch::Receiver<bool>) -> bool {
    *stop.borrow() || stop.has_changed().is_err()
}

/// Wait for `wake` or the stop signal without cancelling any pass, then report
/// whether the worker may run its next pass.
pub(crate) async fn wake_unless_stopped<F: Future>(
    stop: &mut watch::Receiver<bool>,
    wake: F,
) -> bool {
    tokio::select! {
        _ = wake => {}
        _ = stop.wait_for(|stopped| *stopped) => {}
    }
    !stop_requested(stop)
}

/// Wait for the stop signal, then join `workers` under one shared deadline and
/// abort any worker that is still running when it expires.
pub async fn supervise_workers(
    mut stop: watch::Receiver<bool>,
    workers: Vec<JoinHandle<()>>,
    grace: Duration,
) {
    let _ = stop.wait_for(|stopped| *stopped).await;
    let deadline = tokio::time::Instant::now() + grace;
    for mut worker in workers {
        if tokio::time::timeout_at(deadline, &mut worker)
            .await
            .is_err()
        {
            tracing::warn!("periodic worker exceeded its stop deadline; aborting");
            worker.abort();
            let _ = worker.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(start_paused = true)]
    async fn supervisor_joins_stopped_workers_and_aborts_one_past_the_deadline() {
        let (stop, stopped) = watch::channel(false);
        let quick = tokio::spawn(async {});
        let stuck = tokio::spawn(std::future::pending::<()>());
        let stuck_abort = stuck.abort_handle();
        let supervisor = tokio::spawn(supervise_workers(
            stopped,
            vec![quick, stuck],
            WORKER_STOP_GRACE,
        ));
        tokio::time::sleep(Duration::from_secs(60)).await;
        assert!(!supervisor.is_finished(), "supervisor waits for the signal");

        let signalled = tokio::time::Instant::now();
        stop.send(true).unwrap();
        supervisor.await.unwrap();

        assert_eq!(tokio::time::Instant::now() - signalled, WORKER_STOP_GRACE);
        assert!(stuck_abort.is_finished());
    }
}
