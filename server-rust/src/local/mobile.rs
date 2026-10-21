//! Android activation and request ownership. Background tasks keep only Weak owners.
use super::{
    Process,
    model_files::{Cancel, ModelFile, unavailable},
};
use crate::Result;
use serde_json::{Value, json};
use std::{
    sync::{Arc, Mutex as StdMutex},
    time::Duration,
};
use tokio::{
    sync::{Mutex, Notify, OwnedSemaphorePermit, Semaphore},
    task::JoinHandle,
    time::Instant,
};

pub(super) const IDLE: Duration = Duration::from_secs(240);
struct Lifecycle {
    activated: bool,
    stopped: bool,
    active: usize,
    last_use: Instant,
}
struct Provisioning {
    cancel: Cancel,
    task: JoinHandle<()>,
}
pub(super) struct Mobile {
    chat: Arc<Process>,
    embed: Arc<Process>,
    pub chat_file: Arc<ModelFile>,
    pub embed_file: Arc<ModelFile>,
    lifecycle: StdMutex<Lifecycle>,
    gate: Arc<Semaphore>,
    transition: Mutex<()>,
    provision: StdMutex<Option<Provisioning>>,
    worker: StdMutex<Option<JoinHandle<()>>>,
    wake: Arc<Notify>,
    idle: Duration,
    interval: Duration,
}
pub(super) struct Operation {
    mobile: Arc<Mobile>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for Operation {
    fn drop(&mut self) {
        let mut state = self.mobile.lifecycle.lock().unwrap();
        state.active -= 1;
        state.last_use = Instant::now();
        // The permit drops after this method. A standby worker awaits it rather than missing the wake.
        self.mobile.wake.notify_one();
    }
}
impl Mobile {
    pub fn new(
        chat: Arc<Process>,
        embed: Arc<Process>,
        chat_file: Arc<ModelFile>,
        embed_file: Arc<ModelFile>,
    ) -> Arc<Self> {
        Arc::new(Self {
            chat,
            embed,
            chat_file,
            embed_file,
            lifecycle: StdMutex::new(Lifecycle {
                activated: false,
                stopped: false,
                active: 0,
                last_use: Instant::now(),
            }),
            gate: Arc::new(Semaphore::new(1)),
            transition: Mutex::new(()),
            provision: StdMutex::new(None),
            worker: StdMutex::new(None),
            wake: Arc::new(Notify::new()),
            idle: IDLE,
            interval: Duration::from_secs(10),
        })
    }
    pub fn activated(&self) -> bool {
        let state = self.lifecycle.lock().unwrap();
        state.activated && !state.stopped
    }
    #[cfg(test)]
    pub fn test_deadline(owner: &mut Arc<Self>, idle: Duration) {
        let mobile = Arc::get_mut(owner).unwrap();
        mobile.idle = idle;
        mobile.interval = Duration::from_millis(10);
    }
    pub fn status(&self) -> Value {
        let state = self.lifecycle.lock().unwrap();
        json!({"activated":state.activated,"stopped":state.stopped,"activeOperations":state.active,"idleUnloadSeconds":self.idle.as_secs()})
    }
    pub async fn begin(self: &Arc<Self>) -> Result<Operation> {
        let permit = self
            .gate
            .clone()
            .acquire_owned()
            .await
            .map_err(|_| unavailable("Modelos locais encerrados."))?;
        let mut state = self.lifecycle.lock().unwrap();
        if !state.activated || state.stopped {
            return Err(unavailable("Modelos Android em standby."));
        }
        state.active += 1;
        Ok(Operation {
            mobile: self.clone(),
            _permit: permit,
        })
    }
    pub async fn activate(self: &Arc<Self>) -> Result<()> {
        let _transition = self.transition.lock().await;
        {
            let mut state = self.lifecycle.lock().unwrap();
            if state.stopped {
                return Err(unavailable("Modelos locais encerrados."));
            }
            if state.activated {
                return Ok(());
            }
            state.activated = true;
            state.last_use = Instant::now();
        }
        // An explicit standby -> activate may retry once. No timer or request retries downloads.
        self.cancel_provision().await;
        self.start_worker();
        if !self.chat.exe.is_file() {
            self.chat_file.fail("Runtime Android indisponível.");
            self.embed_file.fail("Runtime Android indisponível.");
            return Ok(());
        }
        let cancel = Cancel::new();
        let cancellation = cancel.clone();
        let files = [self.embed_file.clone(), self.chat_file.clone()];
        let weak = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            for file in files {
                if cancellation.is_cancelled() {
                    break;
                }
                // Independent models: an embedding failure need not discard a usable chat GGUF.
                let _ = file.provision(&cancellation).await;
            }
            if let Some(mobile) = weak.upgrade() {
                mobile.wake.notify_one();
            }
        });
        *self.provision.lock().unwrap() = Some(Provisioning { cancel, task });
        Ok(())
    }
    pub async fn download_file(self: &Arc<Self>, kind: &str) -> Result<()> {
        let _transition = self.transition.lock().await;
        if self.lifecycle.lock().unwrap().stopped {
            return Err(unavailable("Modelos locais encerrados."));
        }
        let file = match kind {
            "chat" => self.chat_file.clone(),
            "embed" => self.embed_file.clone(),
            _ => return Err(crate::Error::new(400, "Tipo de modelo desconhecido.")),
        };
        let cancel = Cancel::new();
        let cancellation = cancel.clone();
        let weak = Arc::downgrade(self);
        let task = tokio::spawn(async move {
            let _ = file.provision(&cancellation).await;
            if let Some(mobile) = weak.upgrade() {
                mobile.wake.notify_one();
            }
        });
        *self.provision.lock().unwrap() = Some(Provisioning { cancel, task });
        Ok(())
    }
    pub async fn delete_file(self: &Arc<Self>, kind: &str) -> Result<()> {
        let _transition = self.transition.lock().await;
        match kind {
            "chat" => {
                let _ = self.chat.stop().await;
                self.chat_file.delete().await?;
            }
            "embed" => {
                let _ = self.embed.stop().await;
                self.embed_file.delete().await?;
            }
            _ => return Err(crate::Error::new(400, "Tipo de modelo desconhecido.")),
        };
        self.wake.notify_one();
        Ok(())
    }
    pub async fn standby(&self) {
        let _transition = self.transition.lock().await;
        self.lifecycle.lock().unwrap().activated = false;
        if let Some(job) = self.provision.lock().unwrap().as_ref() {
            job.cancel.cancel();
        }
        self.wake.notify_one();
    }
    fn due(&self) -> bool {
        let state = self.lifecycle.lock().unwrap();
        !state.stopped && (!state.activated || state.last_use.elapsed() >= self.idle)
    }
    fn start_worker(self: &Arc<Self>) {
        let mut worker = self.worker.lock().unwrap();
        if worker.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        let wake = self.wake.clone();
        let interval = self.interval;
        *worker = Some(tokio::spawn(async move {
            loop {
                tokio::select! { _ = wake.notified() => {}, _ = tokio::time::sleep(interval) => {} }
                let Some(mobile) = weak.upgrade() else {
                    break;
                };
                if !mobile.due() {
                    continue;
                }
                let gate = mobile.gate.clone();
                drop(mobile);
                let Ok(_permit) = gate.acquire_owned().await else {
                    break;
                };
                let Some(mobile) = weak.upgrade() else {
                    break;
                };
                // Recheck after the active request ended or an activation overtook standby.
                if mobile.due() {
                    let _ = mobile.chat.stop().await;
                    let _ = mobile.embed.stop().await;
                }
            }
        }));
    }
    async fn cancel_provision(&self) {
        let job = self.provision.lock().unwrap().take();
        if let Some(mut job) = job {
            job.cancel.cancel();
            if tokio::time::timeout(Duration::from_secs(3), &mut job.task)
                .await
                .is_err()
            {
                job.task.abort();
                let _ = job.task.await;
            }
        }
    }
    pub async fn stop(&self) {
        let _transition = self.transition.lock().await;
        {
            let mut state = self.lifecycle.lock().unwrap();
            state.stopped = true;
            state.activated = false;
        }
        self.gate.close();
        let worker = self.worker.lock().unwrap().take();
        if let Some(worker) = worker {
            worker.abort();
            let _ = worker.await;
        }
        self.cancel_provision().await;
    }
}
impl Drop for Mobile {
    fn drop(&mut self) {
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            worker.abort();
        }
        if let Some(job) = self.provision.get_mut().unwrap().take() {
            job.cancel.cancel();
            job.task.abort();
        }
    }
}
