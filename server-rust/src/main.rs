use modularchat_server::{
    Error, Result,
    api::{self, AppState},
    config::Config,
};
use std::{future::IntoFuture, path::PathBuf, time::Duration};

#[cfg(windows)]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[tokio::main]
async fn main() {
    // Without RUST_LOG: our own events at `info`, dependencies only from `warn` up.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                tracing_subscriber::EnvFilter::new("warn,modularchat_server=info")
            }),
        )
        // stdout stays reserved for the banner and the JSON of the snapshot/vault-check commands.
        .with_writer(std::io::stderr)
        .init();
    if let Err(e) = run().await {
        eprintln!("ModularChat: {}", e.message);
        std::process::exit(1);
    }
}
/// Ctrl+C and, on Windows, also closing the console, Ctrl+Break and logoff/shutdown: derived state
/// still in memory must be written to the vault before the process ends (`app.shutdown`).
async fn shutdown_signal() {
    #[cfg(windows)]
    {
        use tokio::signal::windows::{ctrl_break, ctrl_close, ctrl_logoff, ctrl_shutdown};
        if let (Ok(mut brk), Ok(mut close), Ok(mut logoff), Ok(mut shutdown)) =
            (ctrl_break(), ctrl_close(), ctrl_logoff(), ctrl_shutdown())
        {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {},
                _ = brk.recv() => {},
                _ = close.recv() => {},
                _ = logoff.recv() => {},
                _ = shutdown.recv() => {},
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}
/// Short class of a job error: the text before the first `:`, kept only when it looks like a fixed message
/// (short and without quotes or brackets that would signal quoted content); otherwise `outro`. Empty stays empty.
fn error_category(message: &str) -> String {
    let head = message.split(':').next().unwrap_or_default().trim();
    if head.is_empty() {
        return String::new();
    }
    if head.chars().count() > 48 || head.contains(['"', '\'', '«', '“', '‘', '{', '[', '\n']) {
        return "outro".into();
    }
    head.to_owned()
}
async fn run() -> Result<()> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().is_some_and(|a| a == "snapshot") {
        if args.len() != 3 {
            return Err(Error::new(
                400,
                "Uso: modularchat-server snapshot <source.db> <destination.db>",
            ));
        }
        let source = PathBuf::from(&args[1]).canonicalize()?;
        let target = PathBuf::from(&args[2]);
        if !source.is_file() {
            return Err(Error::new(404, "Banco de origem ausente."));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)?;
        }
        if source.to_string_lossy().ends_with(".vault") {
            modularchat_server::privacy::snapshot(&source, &target)?;
        } else {
            modularchat_server::store::transfer::sqlite_snapshot(&source, &target)?;
        }
        println!(
            "Snapshot consistente criado; integrity_check=ok; destino={}",
            target.display()
        );
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "vault-check") && args.len() == 2 {
        let db = modularchat_server::privacy::read_snapshot(std::path::Path::new(&args[1]))?;
        let mut counts = serde_json::Map::new();
        for table in [
            "conversations",
            "interactions",
            "versions",
            "memories",
            "devices",
            "personas",
        ] {
            let n: i64 =
                db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?;
            counts.insert(table.into(), serde_json::json!(n));
        }
        let active: i64 = db.query_row(
            "SELECT COUNT(*) FROM generations WHERE status='running'",
            [],
            |r| r.get(0),
        )?;
        // Memory queue diagnostics: jobs by type/state and only the category of the error of failed ones
        // (the stored message can quote model output, so it is never printed).
        let mut jobs = Vec::new();
        if let Ok(mut st) = db.prepare("SELECT id,conv_id,kind,status,attempts,coalesce(error,'') FROM jobs WHERE status IN ('failed','pending','running','blocked') ORDER BY created_at LIMIT 40")
            && let Ok(rows) = st.query_map([], |r| Ok(serde_json::json!({"id":r.get::<_,String>(0)?,"conv_id":r.get::<_,String>(1)?,"kind":r.get::<_,String>(2)?,"status":r.get::<_,String>(3)?,"attempts":r.get::<_,i64>(4)?,"error":error_category(&r.get::<_,String>(5)?)})))
        {
            jobs.extend(rows.flatten());
        }
        println!(
            "{}",
            serde_json::json!({"integrity":"ok","encrypted":true,"counts":counts,"activeGenerations":active,"jobs":jobs})
        );
        return Ok(());
    }
    if args.first().is_some_and(|a| a == "sync-check") && args.len() == 2 {
        println!("{}", modularchat_server::store::Store::audit_sync_snapshot(std::path::Path::new(&args[1]))?);
        return Ok(());
    }
    if !args.is_empty() {
        return Err(Error::new(400, "Argumento desconhecido."));
    }
    let config = Config::from_env()?;
    let app = AppState::new(config.clone()).await?;
    let pairing = app.auth.start().await;
    let listener = tokio::net::TcpListener::bind(config.address).await?;
    println!(
        "ModularChat Rust {} em http://{}; pareamento={} (15 min); DATA_DIR={}",
        if config.production {
            "production"
        } else {
            "experimental"
        },
        listener.local_addr()?,
        pairing["code"].as_str().unwrap_or_default(),
        config.data_dir.display()
    );
    let server = axum::serve(
        listener,
        api::router(app.clone()).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    );
    // Persistent event streams must not hold shutdown open indefinitely.
    tokio::select! {result=server.into_future()=>result.map_err(|e|Error::new(500,e.to_string()))?,_=shutdown_signal()=>{}}
    tokio::time::timeout(Duration::from_secs(40), app.shutdown())
        .await
        .map_err(|_| {
            Error::new(
                504,
                "Encerramento excedeu o prazo; checkpoints serão recuperados.",
            )
        })?;
    Ok(())
}
