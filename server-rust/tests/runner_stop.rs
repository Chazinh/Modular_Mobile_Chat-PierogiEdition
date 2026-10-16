//! Stop must survive the interval between spawning a worker and its watch subscription.
use modularchat_server::{
    api::AppState,
    config::Config,
    jobs::Runner,
    local::LocalModels,
    store::{Request, Store},
};
use std::{path::Path, sync::Arc, time::Duration};
use tokio::sync::{Notify, broadcast};

const STOP_DEADLINE: Duration = Duration::from_secs(1);

fn isolated_config(data: &Path, native: &Path) -> Config {
    // Explicit Android bootstrap ignores desktop environment/providers. Empty native
    // and data directories cannot supply models; no HTTP handlers are invoked.
    let mut config = Config::for_android(data, native).unwrap();
    config.address = "127.0.0.1:0".parse().unwrap();
    config
}

fn runner(config: &Config) -> (Store, Runner) {
    let store = Store::open_for_config(config).unwrap();
    let local = LocalModels::new(config, reqwest::Client::new()).unwrap();
    let (events, _) = broadcast::channel(8);
    let runner = Runner::new(store.clone(), local, Arc::new(Notify::new()), events, false);
    (store, runner)
}

async fn stopped(mut worker: tokio::task::JoinHandle<()>, runner: &Runner, store: &Store) {
    let outcome = tokio::time::timeout(STOP_DEADLINE, &mut worker).await;
    if outcome.is_err() {
        // A failing regression must not leave a detached worker holding the store.
        worker.abort();
        let _ = worker.await;
    }
    store.call(Request::Close).await.unwrap();
    outcome
        .expect("stop must finish before the shutdown fallback deadline")
        .expect("worker must exit cleanly");
    let state = runner.state().await;
    assert_eq!(state["running"], false);
    assert!(state["lastError"].is_null());
}

#[tokio::test(flavor = "current_thread")]
async fn stop_before_start_is_retained_for_the_future_subscriber() {
    let data = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let (store, runner) = runner(&isolated_config(data.path(), native.path()));

    runner.stop();
    let worker = runner.start();

    stopped(worker, &runner, &store).await;
}

#[tokio::test(flavor = "current_thread")]
async fn stop_after_spawn_before_worker_runs_is_retained() {
    let data = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let (store, runner) = runner(&isolated_config(data.path(), native.path()));

    // spawn never polls inline. On this single-thread runtime, no await between
    // start and stop means Runner::start has not yet reached subscribe(). With
    // watch::Sender::send(true), there are no receivers and the stop is lost.
    let worker = runner.start();
    runner.stop();

    stopped(worker, &runner, &store).await;
}

#[tokio::test(flavor = "current_thread")]
async fn app_shutdown_immediately_after_bootstrap_finishes_promptly() {
    let data = tempfile::tempdir().unwrap();
    let native = tempfile::tempdir().unwrap();
    let state = AppState::new(isolated_config(data.path(), native.path()))
        .await
        .unwrap();

    tokio::time::timeout(STOP_DEADLINE, state.shutdown())
        .await
        .expect("immediate shutdown must not wait for the 10-second worker fallback");

    let status = state.runner.state().await;
    assert_eq!(status["running"], false);
    assert!(status["lastError"].is_null());
    assert!(!data.path().join("models").exists());
    assert!(data.path().join("modularchat.db.vault").is_file());
}
