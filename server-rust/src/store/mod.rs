//! Single SQLite connection owned by a dedicated thread. No SQLite work on Tokio workers.
mod context;
mod jobs;
mod maintenance;
mod manual;
mod memory;
mod repo;
pub mod transfer;
mod turns;

use crate::{Error, Result, now, rules};
use rusqlite::{
    Connection,
    types::{Value as SqlValue, ValueRef},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::{mpsc, oneshot};

pub const SCHEMA_VERSION: u32 = 4;
pub const IMPORT_LIMIT: usize = 80 * 1024 * 1024;
pub const SCHEMA: &str = include_str!("schema.sql");
pub(super) const PATH_CTE: &str = "WITH RECURSIVE p(id) AS (SELECT head_interaction_id FROM conversations WHERE id=?1 AND head_interaction_id IS NOT NULL UNION SELECT i.parent_id FROM interactions i JOIN p ON i.id=p.id WHERE i.parent_id IS NOT NULL)";

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TurnMode {
    Send,
    Edit,
    Regenerate,
    Continue,
}
impl TurnMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Send => "send",
            Self::Edit => "edit",
            Self::Regenerate => "regenerate",
            Self::Continue => "continue",
        }
    }
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TurnRequest {
    pub conv_id: String,
    pub mode: TurnMode,
    #[serde(default, deserialize_with = "nullable_content")]
    pub content: String,
    #[serde(default)]
    pub op_id: Option<String>,
    #[serde(default)]
    pub target_id: Option<String>,
    #[serde(default)]
    pub route_id: Option<String>,
    #[serde(default)]
    pub preset_id: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(default)]
    pub request: Value,
}
fn nullable_content<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<String, D::Error> {
    Option::<String>::deserialize(d).map(Option::unwrap_or_default)
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FinishRequest {
    pub generation_id: String,
    pub status: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub usage: Value,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct EmbeddingItem {
    pub conv_id: String,
    pub owner_type: String,
    pub owner_id: String,
    pub model: String,
    pub source_text: Option<String>,
    pub vector: Vec<f32>,
}

// Only defined operations may enter the queue. There is no arbitrary SQL/function-name escape hatch.
#[derive(Debug)]
pub enum Request {
    Status,
    CreateConversation(Value),
    ListConversations {
        archived: bool,
        query: String,
    },
    GetConversation(String),
    GetMeta(String),
    ClearAll,
    PatchConversation {
        conv_id: String,
        patch: Value,
        base_character_rev: Option<i64>,
    },
    SetCharacter {
        conv_id: String,
        text: String,
        source: String,
    },
    DuplicateConversation(String),
    DeleteConversation(String),
    RestoreConversation(String),
    PurgeTrash(Value),
    GlobalSettings,
    SetGlobalSettings(Value),
    MaybeQueueCuration(String),
    CurationCandidates(String),
    ApplyCuration(Value),
    ClearHistory(String),
    Trash,
    RestoreBatch {
        conv_id: String,
        batch: String,
    },
    DeleteMessage {
        conv_id: String,
        message_id: String,
    },
    BeginTurn(TurnRequest),
    FinishGeneration(FinishRequest),
    FlushVersion {
        version_id: String,
        text: String,
    },
    RecoverInterrupted,
    GetGeneration(String),
    EditMessage {
        conv_id: String,
        part_id: String,
        content: String,
        op_id: Option<String>,
    },
    Versions {
        conv_id: String,
        part_id: String,
    },
    SelectVersion {
        conv_id: String,
        part_id: String,
        version_id: String,
    },
    Branches(String),
    ActivateBranch {
        conv_id: String,
        interaction_id: String,
    },
    Search {
        query: String,
        conv_id: Option<String>,
        limit: usize,
    },
    Export(Option<String>),
    Import(Value),
    Backup,
    Personas,
    SavePersona(Value),
    DeletePersona(String),
    Cast(String),
    SaveCast {
        conv_id: String,
        value: Value,
    },
    DeleteCast {
        conv_id: String,
        cast_id: String,
    },
    Memories(String),
    AddMemory {
        conv_id: String,
        value: Value,
    },
    PatchMemory {
        conv_id: String,
        memory_id: String,
        value: Value,
    },
    DeleteMemory {
        conv_id: String,
        memory_id: String,
    },
    SaveEmbedding {
        conv_id: String,
        owner_type: String,
        owner_id: String,
        model: String,
        source_text: Option<String>,
        vector: Vec<f32>,
    },
    /// Several vectors in a single transaction (one embeddings cycle); returns how many were accepted.
    SaveEmbeddings(Vec<EmbeddingItem>),
    RecordUsage {
        route_id: String,
        raw: usize,
        measured: usize,
    },
    Calibration(String),
    AddDevice {
        name: String,
        token_hash: String,
    },
    FindDevice(String),
    Devices,
    RevokeDevice(String),
    Context(Value),
    Retrieve(Value),
    EnqueueJob(Value),
    ClaimJob,
    FinishJob {
        id: String,
        status: String,
        error: Option<String>,
    },
    /// Returns a claimed job to the queue without spending an attempt (the local model was unavailable, not the input).
    DeferJob {
        id: String,
        error: String,
    },
    RequeueJobs,
    ReleaseLegacyReview,
    RetryJobs(String),
    JobInput(String),
    ApplyExtraction(Value),
    PendingEmbeddings {
        limit: usize,
        model: String,
    },
    QueueEpisodes(String),
    CreateEpisode(Value),
    AddFlags(Value),
    ResolveFlag {
        conv_id: String,
        flag_id: String,
        action: String,
    },
    MergeCast {
        conv_id: String,
        from_id: String,
        into_id: String,
    },
    ReanalyzeCast(String),
    Source {
        conv_id: String,
        interaction_id: String,
    },
    SetContext {
        generation_id: String,
        value: Value,
    },
    ProviderCheck {
        generation_id: String,
        value: Value,
    },
    GenerationByOp {
        conv_id: String,
        op_id: String,
    },
    ImportLegacy,
    Integrity,
    ImportOne(Value),
    SyncLag(String),
    Summary(String),
    CurrentScene {
        conv_id: String,
        before: i64,
    },
    SceneHistory {
        conv_id: String,
        entity: Option<String>,
        limit: usize,
    },
    ReplaceCast {
        conv_id: String,
        list: Vec<Value>,
        persona: Option<Value>,
    },
    SyncUserCast {
        conv_id: String,
        persona: Value,
    },
    SeedPersonas,
    Busy,
    Prune(Value),
    Purge(Value),
    Compact,
    PrivacyKey,
    ProtectExport(Value),
    DecryptImport(Value),
    WritePrivateCharacter {
        conv_id: String,
        text: String,
    },
    ReadPrivateCharacter(String),
    /// Vault write counters (diagnostics and tests).
    PersistStats,
    Close,
}
/// When a change must reach the vault on disk.
#[derive(Clone, Copy, PartialEq)]
enum Durability {
    /// Write before answering: everything the client sees as confirmed.
    Immediate,
    /// Derived/re-runnable state: mark dirty and write in a batch within `DERIVED_DELAY`.
    Derived,
    /// Partial progress of a reply in progress: batched within `CHECKPOINT_DELAY`.
    Checkpoint,
}
const DERIVED_DELAY: Duration = Duration::from_millis(1000);
const CHECKPOINT_DELAY: Duration = Duration::from_millis(5000);
/// Pruning of operational records runs on open and then every 24 h of process uptime.
const PRUNE_EVERY: Duration = Duration::from_secs(24 * 3600);
impl Request {
    fn durability(&self) -> Durability {
        match self {
            // Derived or rebuildable by jobs: a process killed by force loses at most the window.
            Self::SetContext { .. }
            | Self::RecordUsage { .. }
            | Self::ClaimJob
            | Self::FinishJob { .. }
            | Self::DeferJob { .. }
            | Self::RequeueJobs
            | Self::SaveEmbedding { .. }
            | Self::SaveEmbeddings(_)
            | Self::AddFlags(_)
            | Self::MaybeQueueCuration(_)
            | Self::ApplyExtraction(_)
            | Self::FindDevice(_)
            | Self::Context(_) => Durability::Derived,
            Self::FlushVersion { .. } => Durability::Checkpoint,
            _ => Durability::Immediate,
        }
    }
    /// Operations that expose the database state: the vault on disk is updated first.
    fn flushes_first(&self) -> bool {
        matches!(self, Self::Backup | Self::Export(_))
    }
    fn transactional(&self) -> bool {
        !matches!(
            self,
            Self::Status
                | Self::ListConversations { .. }
                | Self::GetConversation(_)
                | Self::GetGeneration(_)
                | Self::Trash
                | Self::Versions { .. }
                | Self::Branches(_)
                | Self::Search { .. }
                | Self::Export(_)
                | Self::Import(_)
                | Self::ImportLegacy
                | Self::Backup
                | Self::Personas
                | Self::Cast(_)
                | Self::Memories(_)
                | Self::Calibration(_)
                | Self::Devices
                | Self::PersistStats
        )
    }
}
struct Envelope {
    request: Request,
    deadline: Instant,
    reply: oneshot::Sender<Result<Value>>,
}
#[derive(Clone)]
pub struct Store {
    sender: mpsc::Sender<Envelope>,
    timeout: Duration,
    pub encrypted: bool,
}
impl Store {
    pub fn open(dir: &Path) -> Result<Self> {
        Self::open_with_limits(dir, 64, Duration::from_secs(30))
    }
    pub fn open_for_config(config: &crate::config::Config) -> Result<Self> {
        Self::open_internal(
            &config.data_dir,
            64,
            Duration::from_secs(30),
            config.production,
            false,
        )
    }
    pub fn open_with_limits(dir: &Path, capacity: usize, timeout: Duration) -> Result<Self> {
        Self::open_internal(dir, capacity, timeout, false, false)
    }
    /// Isolated directory with the encrypted vault (the same as `MC_ENCRYPTED_STORAGE=1`), independent of the environment.
    pub fn open_encrypted(dir: &Path) -> Result<Self> {
        Self::open_internal(dir, 64, Duration::from_secs(30), false, true)
    }
    fn open_internal(
        dir: &Path,
        capacity: usize,
        timeout: Duration,
        production: bool,
        force_encrypted: bool,
    ) -> Result<Self> {
        if capacity == 0 || capacity > 4096 || timeout.is_zero() {
            return Err(Error::new(400, "Limites do store inválidos."));
        }
        let dir = if production {
            dir.canonicalize()?
        } else {
            crate::config::isolated_dir(dir)?
        };
        let encrypted = production
            || force_encrypted
            || dir.join("modularchat.db.vault").exists()
            || std::env::var("MC_ENCRYPTED_STORAGE").as_deref() == Ok("1");
        let (sender, mut receiver) = mpsc::channel::<Envelope>(capacity);
        let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
        std::thread::Builder::new()
            .name("mc-sqlite".into())
            .spawn(move || {
                // Minimal runtime only to wait on the queue with a deadline (flushing dirty derived state).
                let rt = match tokio::runtime::Builder::new_current_thread()
                    .enable_time()
                    .build()
                {
                    Ok(rt) => rt,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e.into()));
                        return;
                    }
                };
                let mut db = match Database::open(dir, encrypted) {
                    Ok(db) => {
                        let _ = ready_tx.send(Ok(()));
                        db
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                loop {
                    // Under continuous load the queue never empties: the deadline is checked on every loop.
                    if db.flush_due() {
                        db.flush_timer();
                    }
                    let next = match db.deadline {
                        None => rt.block_on(receiver.recv()),
                        Some(at) => match rt.block_on(async {
                            tokio::time::timeout_at(
                                tokio::time::Instant::from_std(at),
                                receiver.recv(),
                            )
                            .await
                        }) {
                            Ok(next) => next,
                            Err(_) => continue,
                        },
                    };
                    // Queue closed without `Close`: Database's Drop still flushes whatever is dirty.
                    let Some(env) = next else { return };
                    if matches!(env.request, Request::Close) {
                        let flushed = db.flush_dirty();
                        drop(db);
                        let _ = env.reply.send(flushed.map(|()| json!(true)));
                        return;
                    }
                    // Cancellation/deadline can drop work ONLY before it starts. A started
                    // transaction commits or rolls back on its own result, never on HTTP disconnect.
                    if env.reply.is_closed() {
                        continue;
                    }
                    if Instant::now() >= env.deadline {
                        let _ = env.reply.send(Err(Error::new(
                            504,
                            "Fila de armazenamento excedeu o prazo.",
                        )
                        .extra("code", "STORE_DEADLINE")));
                        continue;
                    }
                    let result = db.execute(env.request);
                    let _ = env.reply.send(result);
                }
            })?;
        ready_rx
            .recv_timeout(Duration::from_secs(20))
            .map_err(|_| Error::new(503, "SQLite não iniciou no prazo."))??;
        Ok(Self {
            sender,
            timeout,
            encrypted,
        })
    }
    pub async fn call(&self, request: Request) -> Result<Value> {
        let (reply, rx) = oneshot::channel();
        let env = Envelope {
            request,
            deadline: Instant::now() + self.timeout,
            reply,
        };
        self.sender.try_send(env).map_err(|e| match e {
            mpsc::error::TrySendError::Full(_) => {
                Error::new(503, "Fila de armazenamento cheia.").extra("code", "STORE_OVERLOADED")
            }
            mpsc::error::TrySendError::Closed(_) => Error::new(503, "Armazenamento encerrado."),
        })?;
        tokio::time::timeout(self.timeout,rx).await.map_err(|_|Error::new(504,"Armazenamento excedeu o prazo; uma operação iniciada pode ter sido confirmada. Reutilize opId ao repetir.").extra("code","STORE_DEADLINE"))?.map_err(|_|Error::new(503,"Armazenamento encerrado."))?
    }
}

pub(super) struct Database {
    pub db: Connection,
    pub dir: PathBuf,
    /// Timeline views already derived (Arc: returning them does not copy the whole tree on every message).
    /// The third field is the approximate serialized size: the cache is also bounded in bytes.
    pub timeline_cache: std::cell::RefCell<Vec<(String, std::sync::Arc<Value>, usize)>>,
    pub(super) vault: Option<crate::privacy::Vault>,
    /// Deadline to write the derived state still only in memory (`None`: the vault on disk is up to date).
    deadline: Option<Instant>,
    /// Requests that changed rows / the ones that wrote the vault immediately (comparison with the old behavior).
    changing: u64,
    immediate: u64,
    /// Last automatic pruning of operational records (see `auto_prune`).
    pruned_at: Instant,
}
impl Drop for Database {
    fn drop(&mut self) {
        if self.deadline.is_some() {
            let _ = self.flush_dirty();
        }
    }
}
impl Database {
    fn open(dir: PathBuf, encrypted: bool) -> Result<Self> {
        let file = dir.join("modularchat.db");
        for p in [
            &file,
            &dir.join("modularchat.db-wal"),
            &dir.join("modularchat.db-shm"),
        ] {
            if p.exists() && p.symlink_metadata()?.file_type().is_symlink() {
                return Err(Error::new(400, "Banco isolado não pode ser um link."));
            }
            // Refuse NTFS hard links as well: an external path must never alias production DB/WAL.
            if p.exists() && crate::config::hard_links(p)? > 1 {
                return Err(Error::new(400, "Banco não pode possuir hard links."));
            }
        }
        let (vault, db) = if encrypted {
            let (vault, db) = crate::privacy::Vault::open(&dir)?;
            (Some(vault), db)
        } else {
            (None, Connection::open(file)?)
        };
        db.busy_timeout(Duration::from_secs(5))?;
        db.set_prepared_statement_cache_capacity(128);
        let this = Self {
            db,
            dir,
            timeline_cache: std::cell::RefCell::default(),
            vault,
            deadline: None,
            changing: 0,
            immediate: 0,
            pruned_at: Instant::now(),
        };
        let has_meta: i64 = this.db.query_row(
            "SELECT count(*) FROM sqlite_master WHERE name='meta'",
            [],
            |r| r.get(0),
        )?;
        if has_meta > 0 {
            let version = this.one("SELECT value FROM meta WHERE key='schema_version'", &[])?;
            if version["value"]
                .as_str()
                .and_then(|v| v.parse::<u32>().ok())
                .unwrap_or(0)
                > SCHEMA_VERSION
            {
                return Err(Error::new(
                    409,
                    "Banco tem schema mais novo que este núcleo Rust.",
                ));
            }
        }
        this.db.execute_batch(if encrypted {
            "PRAGMA journal_mode=MEMORY;PRAGMA temp_store=MEMORY;PRAGMA foreign_keys=OFF;"
        } else {
            "PRAGMA journal_mode=WAL;PRAGMA foreign_keys=OFF;"
        })?;
        this.tx(|| {
            this.db.execute_batch(SCHEMA)?;
            if !this
                .rows("PRAGMA table_info(memories)", &[])?
                .iter()
                .any(|r| r["name"] == "pinned")
            {
                this.db.execute_batch(
                    "ALTER TABLE memories ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;",
                )?;
            }
            if this
                .one("SELECT value FROM meta WHERE key='chunks_v1'", &[])?
                .is_null()
            {
                for r in this.rows(
                    "SELECT * FROM search_fts WHERE kind IN ('user','asst')",
                    &[],
                )? {
                    this.index(
                        &crate::s(&r, "conv_id"),
                        &crate::s(&r, "kind"),
                        &crate::s(&r, "owner_id"),
                        &crate::s(&r, "text"),
                    )?;
                }
                this.exec("INSERT INTO meta(key,value) VALUES('chunks_v1','1')", &[])?;
            }
            // Additive migration: the old index stored each whole message besides its chunks. Removes only the row
            // that already has equivalent chunks (rebuildable from versions/user_text).
            if this
                .one("SELECT value FROM meta WHERE key='fts_chunks_only_v1'", &[])?
                .is_null()
            {
                this.exec("DELETE FROM search_fts WHERE kind IN ('user','asst') AND EXISTS(SELECT 1 FROM text_chunks t WHERE t.kind=search_fts.kind AND t.owner_id=search_fts.owner_id)",&[])?;
                this.exec(
                    "INSERT INTO meta(key,value) VALUES('fts_chunks_only_v1','1')",
                    &[],
                )?;
            }
            this.auto_prune()?;
            this.exec(
                "INSERT OR REPLACE INTO meta(key,value) VALUES('schema_version','4')",
                &[],
            )?;
            Ok(json!(true))
        })?;
        if let Some(vault) = &this.vault {
            vault.save(&this.db)?;
        }
        Ok(this)
    }
    fn execute(&mut self, r: Request) -> Result<Value> {
        let durability = r.durability();
        if r.flushes_first() {
            self.flush_dirty()?;
        }
        if self.pruned_at.elapsed() >= PRUNE_EVERY {
            self.pruned_at = Instant::now();
            // A pruning failure never blocks the request; vault flushing follows the derived-state rhythm.
            if self.tx(|| self.auto_prune()).is_ok() && self.vault.is_some() {
                self.mark_dirty(CHECKPOINT_DELAY);
            }
        }
        let before = self.db.total_changes();
        let result = if r.transactional() {
            self.tx(|| self.dispatch(r))
        } else {
            self.dispatch(r)
        };
        if self.db.total_changes() != before && self.vault.is_some() {
            self.changing += 1;
            match durability {
                // A vault written now also carries the pending derived state (the file is the whole database).
                Durability::Immediate => {
                    self.immediate += 1;
                    self.save_now()?;
                }
                Durability::Derived => self.mark_dirty(DERIVED_DELAY),
                Durability::Checkpoint => self.mark_dirty(CHECKPOINT_DELAY),
            }
        }
        result
    }
    fn save_now(&mut self) -> Result<()> {
        if let Some(vault) = &self.vault {
            vault.save(&self.db)?;
        }
        self.deadline = None;
        Ok(())
    }
    fn mark_dirty(&mut self, delay: Duration) {
        let at = Instant::now() + delay;
        self.deadline = Some(self.deadline.map_or(at, |d| d.min(at)));
    }
    /// Writes now when there is pending derived state (shutdown, backup, export).
    fn flush_dirty(&mut self) -> Result<()> {
        if self.deadline.is_some() {
            self.save_now()
        } else {
            Ok(())
        }
    }
    fn flush_due(&self) -> bool {
        self.deadline.is_some_and(|at| Instant::now() >= at)
    }
    /// Deadline flush; on failure retries in 1 s (the state stays in memory).
    fn flush_timer(&mut self) {
        // A persistent failure must not become one stderr line per second: warn on the 1st and every 60th attempt.
        static FAILURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        use std::sync::atomic::Ordering::Relaxed;
        match self.save_now() {
            Ok(()) => FAILURES.store(0, Relaxed),
            Err(e) => {
                let count = FAILURES.fetch_add(1, Relaxed);
                if count.is_multiple_of(60) {
                    eprintln!(
                        "Aviso: falha ao gravar o cofre ({}); nova tentativa em 1 s ({} falha(s) seguidas).",
                        e.message,
                        count + 1
                    );
                }
                self.mark_dirty(DERIVED_DELAY);
            }
        }
    }
    fn persist_stats(&self) -> Value {
        let mut out = self
            .vault
            .as_ref()
            .map_or_else(|| json!({"saves":0,"bytes":0}), |v| v.stats());
        out["encrypted"] = json!(self.vault.is_some());
        out["changingRequests"] = json!(self.changing);
        out["immediateSaves"] = json!(self.immediate);
        out["pending"] = json!(self.deadline.is_some());
        out
    }
    fn tx<F: FnOnce() -> Result<Value>>(&self, f: F) -> Result<Value> {
        self.db.execute_batch("BEGIN IMMEDIATE")?;
        match f() {
            Ok(v) => {
                if let Err(e) = self.db.execute_batch("COMMIT") {
                    let _ = self.db.execute_batch("ROLLBACK");
                    return Err(e.into());
                }
                Ok(v)
            }
            Err(e) => {
                let _ = self.db.execute_batch("ROLLBACK");
                Err(e)
            }
        }
    }
    fn dispatch(&self, r: Request) -> Result<Value> {
        match r {
            Request::Status => self.status(),
            Request::CreateConversation(p) => self.create(&p),
            Request::ListConversations { archived, query } => self.list(archived, &query),
            Request::GetConversation(c) => self.conversation(&c, true),
            Request::GetMeta(c) => self.conversation(&c, false),
            Request::ClearAll => {
                let busy = self.rows(
                    "SELECT conv_id id FROM generations WHERE status='running'",
                    &[],
                )?;
                if !busy.is_empty() {
                    return Err(
                        Error::new(409, "Há resposta em andamento.").extra("busy", json!(busy))
                    );
                }
                let ids =
                    self.rows("SELECT id FROM conversations WHERE deleted_at IS NULL", &[])?;
                for c in &ids {
                    self.clear(&crate::s(c, "id"))?;
                }
                Ok(json!({"ok":true,"count":ids.len(),"recoverableDays":30}))
            }
            Request::PatchConversation {
                conv_id,
                patch,
                base_character_rev,
            } => self.patch(&conv_id, &patch, base_character_rev),
            Request::SetCharacter {
                conv_id,
                text,
                source,
            } => {
                self.conv(&conv_id, false)?;
                let out = self.character(&conv_id, &text, &source)?;
                if out["changed"] == true {
                    self.bump(&conv_id)?;
                }
                Ok(out)
            }
            Request::DuplicateConversation(c) => self.duplicate(&c),
            Request::DeleteConversation(c) => self.delete_conv(&c),
            Request::RestoreConversation(c) => self.restore_conv(&c),
            Request::PurgeTrash(p) => self.purge_trash(&p),
            Request::MaybeQueueCuration(c) => self.maybe_queue_curation(&c),
            Request::CurationCandidates(c) => self.curation_candidates(&c),
            Request::ApplyCuration(p) => self.apply_curation(&p),
            Request::GlobalSettings => self.global_settings(),
            Request::SetGlobalSettings(p) => self.set_global_settings(&p),
            Request::ClearHistory(c) => self.clear(&c),
            Request::Trash => self.trash(),
            Request::RestoreBatch { conv_id, batch } => self.restore_batch(&conv_id, &batch),
            Request::DeleteMessage {
                conv_id,
                message_id,
            } => self.delete_message(&conv_id, &message_id),
            Request::BeginTurn(p) => self.begin(&p),
            Request::FinishGeneration(p) => self.finish(&p),
            Request::FlushVersion { version_id, text } => {
                // The same text already stored (waiting for the first token) changes no row.
                if !self
                    .one(
                        "SELECT 1 x FROM versions WHERE id=? AND status='generating' AND content=?",
                        &[json!(version_id), json!(text)],
                    )?
                    .is_null()
                {
                    return Ok(json!(true));
                }
                self.flush(&version_id, &text)
            }
            Request::RecoverInterrupted => self.recover(),
            Request::GetGeneration(g) => self.generation(&g),
            Request::EditMessage {
                conv_id,
                part_id,
                content,
                op_id,
            } => self.edit(&conv_id, &part_id, &content, op_id.as_deref()),
            Request::Versions { conv_id, part_id } => self.versions(&conv_id, &part_id),
            Request::SelectVersion {
                conv_id,
                part_id,
                version_id,
            } => self.select_version(&conv_id, &part_id, &version_id),
            Request::Branches(c) => self.branches(&c),
            Request::ActivateBranch {
                conv_id,
                interaction_id,
            } => self.activate(&conv_id, &interaction_id),
            Request::Search {
                query,
                conv_id,
                limit,
            } => self.search(&query, conv_id.as_deref(), limit),
            Request::Export(c) => self.export(c.as_deref()),
            Request::Import(p) => self.import(&p),
            Request::Backup => self.backup(),
            Request::Personas => self.personas(),
            Request::SavePersona(p) => self.save_persona(&p),
            Request::DeletePersona(p) => self.delete_persona(&p),
            Request::Cast(c) => self.cast(&c),
            Request::SaveCast { conv_id, value } => self.save_cast(&conv_id, &value),
            Request::DeleteCast { conv_id, cast_id } => self.delete_cast(&conv_id, &cast_id),
            Request::Memories(c) => self.memories(&c),
            Request::AddMemory { conv_id, value } => self.add_memory(&conv_id, &value),
            Request::PatchMemory {
                conv_id,
                memory_id,
                value,
            } => self.patch_memory(&conv_id, &memory_id, &value),
            Request::DeleteMemory { conv_id, memory_id } => {
                self.delete_memory(&conv_id, &memory_id)
            }
            Request::SaveEmbedding {
                conv_id,
                owner_type,
                owner_id,
                model,
                source_text,
                vector,
            } => self.save_embedding(
                &conv_id,
                &owner_type,
                &owner_id,
                &model,
                source_text.as_deref(),
                &vector,
            ),
            Request::SaveEmbeddings(items) => {
                let mut saved = 0;
                for i in &items {
                    let source = i.source_text.as_deref();
                    if self.save_embedding(
                        &i.conv_id,
                        &i.owner_type,
                        &i.owner_id,
                        &i.model,
                        source,
                        &i.vector,
                    )? == true
                    {
                        saved += 1;
                    }
                }
                Ok(json!(saved))
            }
            Request::RecordUsage {
                route_id,
                raw,
                measured,
            } => {
                if raw > 0 && measured > 0 {
                    let ratio = (measured as f64 / raw as f64).clamp(0.5, 2.0);
                    let cur = self.calibration(&route_id)?;
                    let next = cur["ratio"]
                        .as_f64()
                        .map(|v| v * 0.8 + ratio * 0.2)
                        .unwrap_or(ratio);
                    self.exec("INSERT OR REPLACE INTO token_calibration(route_id,ratio,samples,updated_at) VALUES(?,?,?,?)",&[json!(route_id),json!(next),json!(cur["samples"].as_i64().unwrap_or(0)+1),json!(now())])?;
                }
                Ok(json!(true))
            }
            Request::Calibration(c) => self.calibration(&c),
            Request::AddDevice { name, token_hash } => self.add_device(&name, &token_hash),
            Request::FindDevice(h) => self.find_device(&h),
            Request::Devices => self.devices(),
            Request::RevokeDevice(d) => {
                if self.exec(
                    "UPDATE devices SET revoked_at=? WHERE id=?",
                    &[json!(now()), json!(d)],
                )? == 0
                {
                    return Err(Error::new(404, "Dispositivo não encontrado."));
                }
                Ok(json!({"ok":true}))
            }
            Request::Context(p) => self.context_data(&p),
            Request::Retrieve(p) => self.retrieve(&p),
            Request::EnqueueJob(p) => self.enqueue_job(&p),
            Request::ClaimJob => self.claim_job(),
            Request::FinishJob { id, status, error } => {
                self.finish_job(&id, &status, error.as_deref())
            }
            Request::DeferJob { id, error } => self.defer_job(&id, &error),
            Request::RequeueJobs => {
                let n = self.exec(
                    "UPDATE jobs SET status='pending',updated_at=? WHERE status='running'",
                    &[json!(now())],
                )?;
                Ok(json!(n))
            }
            Request::ReleaseLegacyReview => {
                let t = now();
                let jobs = self.exec("UPDATE jobs SET status='pending',payload_json='{\"skipReview\":true}',updated_at=? WHERE kind='turn' AND status='blocked'", &[json!(t)])?;
                let memories = self.exec(
                    "UPDATE memories SET review_state='ok' WHERE review_state='pending_review'",
                    &[],
                )?;
                let flags = self.exec(
                    "UPDATE review_flags SET status='dismissed',resolved_at=? WHERE status='open'",
                    &[json!(t)],
                )?;
                Ok(json!({"jobs":jobs,"memories":memories,"flags":flags}))
            }
            Request::RetryJobs(c) => {
                self.conv(&c, false)?;
                Ok(json!(self.exec("UPDATE jobs SET status='pending',attempts=0,error=NULL,updated_at=? WHERE conv_id=? AND status='failed'",&[json!(now()),json!(c)])?))
            }
            Request::JobInput(j) => self.job_input(&j),
            Request::ApplyExtraction(p) => self.apply_extraction(&p),
            Request::PendingEmbeddings { limit, model } => self.pending_embeddings(limit, &model),
            Request::QueueEpisodes(c) => self.queue_episodes(&c),
            Request::CreateEpisode(p) => self.create_episode(&p),
            Request::AddFlags(p) => self.add_flags(&p),
            Request::ResolveFlag {
                conv_id,
                flag_id,
                action,
            } => self.resolve_flag(&conv_id, &flag_id, &action),
            Request::MergeCast {
                conv_id,
                from_id,
                into_id,
            } => self.merge_cast(&conv_id, &from_id, &into_id),
            Request::ReanalyzeCast(c) => self.reanalyze_cast(&c),
            Request::Source {
                conv_id,
                interaction_id,
            } => self.source(&conv_id, &interaction_id),
            Request::SetContext {
                generation_id,
                value,
            } => {
                self.exec(
                    "UPDATE generations SET context_json=? WHERE id=?",
                    &[json!(value.to_string()), json!(generation_id)],
                )?;
                Ok(json!(true))
            }
            Request::ProviderCheck {
                generation_id,
                value,
            } => self.provider_check(&generation_id, &value),
            Request::GenerationByOp { conv_id, op_id } => {
                let r = self.one(
                    "SELECT id FROM generations WHERE conv_id=? AND op_id=?",
                    &[json!(conv_id), json!(op_id)],
                )?;
                self.generation(&crate::s(&r, "id"))
            }
            Request::ImportOne(p) => self.import_one(&p),
            Request::SyncLag(c) => self.sync_lag(&c),
            Request::Summary(c) => self.summary(&c),
            Request::CurrentScene { conv_id, before } => self.scene_at(&conv_id, before),
            Request::SceneHistory {
                conv_id,
                entity,
                limit,
            } => self.scene_history(&conv_id, entity.as_deref(), limit),
            Request::ReplaceCast {
                conv_id,
                list,
                persona,
            } => self.replace_cast(&conv_id, &list, persona.as_ref()),
            Request::SyncUserCast { conv_id, persona } => {
                self.sync_user_cast(
                    &conv_id,
                    &crate::s(&persona, "name"),
                    &crate::s(&persona, "description"),
                )?;
                Ok(json!(true))
            }
            Request::SeedPersonas => self.seed_personas(),
            Request::Busy => Ok(json!(
                self.rows(
                    "SELECT conv_id FROM generations WHERE status='running'",
                    &[]
                )?
                .iter()
                .map(|r| r["conv_id"].clone())
                .collect::<Vec<_>>()
            )),
            Request::Prune(p) => self.prune_logs(&p),
            Request::Purge(p) => self.purge_expired(&p),
            Request::Compact => {
                self.db.execute_batch(
                    "PRAGMA optimize; INSERT INTO search_fts(search_fts) VALUES('optimize');",
                )?;
                Ok(json!(true))
            }
            Request::ImportLegacy => {
                if self.vault.is_some() {
                    Ok(json!({"imported":0,"failed":[],"skipped":true}))
                } else {
                    self.import_legacy()
                }
            }
            Request::Integrity => self.one("PRAGMA integrity_check", &[]),
            Request::PrivacyKey => Ok(self.private_vault()?.cache_key()),
            Request::ProtectExport(value) => self.private_vault()?.encrypt_export(&value),
            Request::DecryptImport(value) => self.private_vault()?.decrypt_export(&value),
            Request::WritePrivateCharacter { conv_id, text } => {
                let file = self.private_character(&conv_id)?;
                self.private_vault()?.write_file(&file, &text)?;
                Ok(json!(true))
            }
            Request::ReadPrivateCharacter(conv_id) => Ok(json!(
                self.private_vault()?
                    .read_file(&self.private_character(&conv_id)?)?
            )),
            Request::PersistStats => Ok(self.persist_stats()),
            Request::Close => Err(Error::new(503, "Armazenamento encerrado.")),
        }
    }
    fn private_vault(&self) -> Result<&crate::privacy::Vault> {
        self.vault
            .as_ref()
            .ok_or_else(|| Error::new(404, "Cofre não habilitado."))
    }
    fn private_character(&self, c: &str) -> Result<PathBuf> {
        if c.len() < 20 || c.len() > 40 || !c.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
            return Err(Error::new(400, "ID de chat inválido."));
        }
        let dir = self.dir.join("chats").join(c);
        std::fs::create_dir_all(&dir)?;
        if !dir.canonicalize()?.starts_with(self.dir.canonicalize()?) {
            return Err(Error::new(400, "Ficha fora do DATA_DIR."));
        }
        Ok(dir.join("character.txt.sealed"))
    }
    pub(super) fn exec(&self, sql: &str, args: &[Value]) -> Result<usize> {
        let vals: Vec<_> = args.iter().map(sql_value).collect();
        Ok(self
            .db
            .prepare_cached(sql)?
            .execute(rusqlite::params_from_iter(vals))?)
    }
    pub(super) fn rows(&self, sql: &str, args: &[Value]) -> Result<Vec<Value>> {
        let mut st = self.db.prepare_cached(sql)?;
        let cols: Vec<String> = st.column_names().iter().map(|x| x.to_string()).collect();
        let vals: Vec<_> = args.iter().map(sql_value).collect();
        let mut rows = st.query(rusqlite::params_from_iter(vals))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next()? {
            let mut obj = serde_json::Map::new();
            for (i, name) in cols.iter().enumerate() {
                let v = match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => json!(n),
                    ValueRef::Real(n) => json!(n),
                    ValueRef::Text(t) => json!(String::from_utf8_lossy(t)),
                    ValueRef::Blob(b) => json!(b),
                };
                obj.insert(name.clone(), v);
            }
            out.push(Value::Object(obj));
        }
        Ok(out)
    }
    pub(super) fn one(&self, sql: &str, args: &[Value]) -> Result<Value> {
        Ok(self
            .rows(sql, args)?
            .into_iter()
            .next()
            .unwrap_or(Value::Null))
    }
    pub(super) fn conv(&self, c: &str, deleted: bool) -> Result<Value> {
        let r = self.one("SELECT * FROM conversations WHERE id=?", &[json!(c)])?;
        if r.is_null() || (!deleted && !r["deleted_at"].is_null()) {
            return Err(Error::new(404, "Chat não encontrado."));
        }
        Ok(r)
    }
    pub(super) fn bump(&self, c: &str) -> Result<i64> {
        self.exec(
            "UPDATE conversations SET revision=revision+1,updated_at=? WHERE id=?",
            &[json!(now()), json!(c)],
        )?;
        Ok(crate::n(&self.conv(c, true)?, "revision"))
    }
    pub(super) fn path(&self, c: &str) -> Result<Vec<Value>> {
        self.rows(&format!("{PATH_CTE} SELECT i.* FROM p JOIN interactions i ON i.id=p.id WHERE i.conv_id=?1 AND i.deleted_at IS NULL ORDER BY i.seq"),&[json!(c)])
    }
    pub(super) fn ensure_idle(&self, c: &str) -> Result<()> {
        let r = self.one(
            "SELECT id FROM generations WHERE conv_id=? AND status='running'",
            &[json!(c)],
        )?;
        if !r.is_null() {
            return Err(Error::new(409, "Já existe uma resposta em andamento.")
                .extra("generationId", r["id"].clone()));
        }
        Ok(())
    }
    pub(super) fn unindex(&self, kind: &str, owner: &str) -> Result<()> {
        if matches!(kind, "user" | "asst") {
            self.exec("DELETE FROM embeddings WHERE owner_type='chunk' AND owner_id IN (SELECT id FROM text_chunks WHERE kind=? AND owner_id=?)",&[json!(kind),json!(owner)])?;
            self.exec("DELETE FROM search_fts WHERE kind='chunk' AND owner_id IN (SELECT id FROM text_chunks WHERE kind=? AND owner_id=?)",&[json!(kind),json!(owner)])?;
            self.exec(
                "DELETE FROM text_chunks WHERE kind=? AND owner_id=?",
                &[json!(kind), json!(owner)],
            )?;
        }
        self.exec(
            "DELETE FROM search_fts WHERE kind=? AND owner_id=?",
            &[json!(kind), json!(owner)],
        )?;
        Ok(())
    }
    pub(super) fn index(&self, c: &str, kind: &str, owner: &str, text: &str) -> Result<()> {
        self.unindex(kind, owner)?;
        if matches!(kind, "user" | "asst") {
            for (start, end, text) in rules::chunks(text, 1600, 160) {
                let chunk = format!("{kind}:{owner}:{start}");
                self.exec("INSERT INTO text_chunks(id,conv_id,kind,owner_id,start_offset,end_offset,text) VALUES(?,?,?,?,?,?,?)",&[json!(chunk),json!(c),json!(kind),json!(owner),json!(start),json!(end),json!(text)])?;
                self.exec(
                    "INSERT INTO search_fts(text,conv_id,kind,owner_id) VALUES(?,?,'chunk',?)",
                    &[json!(text), json!(c), json!(chunk)],
                )?;
            }
        }
        // Messages are indexed only by their chunks (kind='chunk'): the full-text row duplicated every speech
        // in the index (~28% of the database). Memories and episodes are short and keep their own row.
        if !matches!(kind, "user" | "asst") && !text.trim().is_empty() {
            self.exec(
                "INSERT INTO search_fts(text,conv_id,kind,owner_id) VALUES(?,?,?,?)",
                &[json!(text), json!(c), json!(kind), json!(owner)],
            )?;
        }
        Ok(())
    }
    pub(super) fn calibration(&self, route: &str) -> Result<Value> {
        self.one(
            "SELECT ratio,samples FROM token_calibration WHERE route_id=?",
            &[json!(route)],
        )
    }
    fn status(&self) -> Result<Value> {
        let counts = self.one(
            "SELECT count(*) conversations FROM conversations WHERE deleted_at IS NULL",
            &[],
        )?;
        let jobs = self.rows("SELECT status,count(*) n FROM jobs GROUP BY status", &[])?;
        let jobs: serde_json::Map<String, Value> = jobs
            .iter()
            .map(|j| (crate::s(j, "status"), j["n"].clone()))
            .collect();
        Ok(json!({"schema":"4","conversations":counts["conversations"],"jobs":jobs}))
    }
}
fn sql_value(v: &Value) -> SqlValue {
    match v {
        Value::Null => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::Number(n) => n
            .as_i64()
            .map(SqlValue::Integer)
            .unwrap_or_else(|| SqlValue::Real(n.as_f64().unwrap_or(0.0))),
        Value::String(s) => SqlValue::Text(s.clone()),
        _ => SqlValue::Text(v.to_string()),
    }
}
