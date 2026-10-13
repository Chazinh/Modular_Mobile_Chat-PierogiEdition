use crate::{Error, Result};
use std::{net::SocketAddr, path::PathBuf};

#[derive(Clone)]
pub struct Config {
    pub data_dir: PathBuf,
    pub dist: PathBuf,
    pub address: SocketAddr,
    pub require_pairing: bool,
    /// Model providers (OpenAI-compatible APIs); `None` disables generation. See providers.rs.
    pub providers: Option<std::sync::Arc<crate::providers::Providers>>,
    pub max_generations: usize,
    pub production: bool,
    /// Automatic review of replies ("to review" flags). Off by default: the local agent
    /// did not understand the flow and wasted processing/VRAM. MC_REVIEW=1 only for tests/diagnostics.
    pub review: bool,
}
impl Config {
    pub fn from_env() -> Result<Self> {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .to_path_buf();
        let production =
            std::env::var("MC_RUST_MODE").unwrap_or_else(|_| "production".into()) == "production";
        let raw = std::env::var_os("DATA_DIR")
            .or_else(|| production.then(|| root.join("data").into_os_string()))
            .ok_or_else(|| {
                Error::new(400, "Rust experimental exige DATA_DIR explícito e isolado.")
            })?;
        let data_dir = PathBuf::from(raw);
        let data_dir = if production {
            validate_production(&data_dir)?
        } else {
            isolated_dir(&data_dir)?
        };
        let host = std::env::var("HOST")
            .unwrap_or(if production { "0.0.0.0" } else { "127.0.0.1" }.into());
        let port = std::env::var("PORT").unwrap_or(if production { "3000" } else { "3011" }.into());
        let address: SocketAddr = format!("{host}:{port}")
            .parse()
            .map_err(|_| Error::new(400, "HOST/PORT inválidos"))?;
        if !production && [3000, 20481, 20482].contains(&address.port()) {
            return Err(Error::new(
                400,
                "Porta de produção protegida. Use 3011 ou outra porta isolada.",
            ));
        }
        let require_pairing = production || std::env::var("REQUIRE_PAIRING").as_deref() != Ok("0");
        if !address.ip().is_loopback() && !require_pairing {
            return Err(Error::new(400, "Pareamento é obrigatório na LAN."));
        }
        let providers =
            if std::env::var("MC_RUST_UPSTREAM_ENABLED").map_or(production, |v| v == "1") {
                let fallback = production.then_some("http://127.0.0.1:20128/v1");
                Some(std::sync::Arc::new(
                    crate::providers::Providers::load(&data_dir, fallback)?.ok_or_else(|| {
                        Error::new(
                            400,
                            "Configure a provider (providers.json or MC_UPSTREAM_URL) to enable generation.",
                        )
                    })?,
                ))
            } else {
                None
            };
        let dist = std::env::var_os("MC_CLIENT_DIST")
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join("client/dist"));
        let max_generations = std::env::var("MC_MAX_GENERATIONS")
            .ok()
            .and_then(|s| s.parse::<usize>().ok())
            .unwrap_or(4)
            .clamp(1, 32);
        Ok(Self {
            data_dir,
            dist,
            address,
            require_pairing,
            providers,
            max_generations,
            production,
            review: std::env::var("MC_REVIEW").as_deref() == Ok("1"),
        })
    }
}

pub fn hard_links(path: &std::path::Path) -> Result<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        #[repr(C)]
        struct Info {
            attributes: u32,
            creation: [u32; 2],
            access: [u32; 2],
            write: [u32; 2],
            volume: u32,
            size_high: u32,
            size_low: u32,
            links: u32,
            index_high: u32,
            index_low: u32,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn GetFileInformationByHandle(handle: *mut std::ffi::c_void, info: *mut Info) -> i32;
        }
        let file = std::fs::File::open(path)?;
        let mut info = std::mem::MaybeUninit::<Info>::uninit();
        // Win32's stable API is needed; std MetadataExt::number_of_links is unstable.
        if unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } == 0 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(unsafe { info.assume_init() }.links.into())
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(path.metadata()?.nlink())
    }
    #[cfg(not(any(windows, unix)))]
    {
        let _ = path;
        Ok(1)
    }
}

fn validate_production(dir: &std::path::Path) -> Result<PathBuf> {
    if std::env::var("MC_RUST_VALIDATED").as_deref() != Ok("2.7.2") {
        return Err(Error::new(
            409,
            "Use start-server.bat para abrir o cofre com backup validado.",
        ));
    }
    std::fs::create_dir_all(dir)?;
    let dir = dir.canonicalize()?;
    if dir.join("modularchat.db").exists() {
        return Err(Error::new(
            409,
            "SQLite legado em texto claro encontrado. Preserve os dados; a produção exige cofre criptografado.",
        ));
    }
    let source = dir.join("modularchat.db.vault");
    if source.exists() {
        let backup = PathBuf::from(
            std::env::var_os("MC_RUST_BACKUP_FILE")
                .ok_or_else(|| Error::new(409, "Backup criptografado obrigatório."))?,
        )
        .canonicalize()?;
        crate::privacy::safe_file(&source)?;
        crate::privacy::safe_file(&backup)?;
        if source.canonicalize()? == backup || std::fs::read(&source)? != std::fs::read(&backup)? {
            return Err(Error::new(
                409,
                "Cofre mudou após o backup ou destino não é independente. Encerre o servidor e refaça a cópia.",
            ));
        }
        let _ = crate::privacy::read_snapshot(&backup)?;
    }
    Ok(dir)
}

// Resolve existing ancestors before creating directories; resolve again for junctions.
// The CLI and tests use this gate before SQLite opens any file.
pub fn isolated_dir(dir: &std::path::Path) -> Result<PathBuf> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_path_buf();
    let personal = root
        .join("data")
        .canonicalize()
        .unwrap_or_else(|_| root.join("data"));
    let absolute = std::path::absolute(dir)?;
    if absolute
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(Error::new(400, "DATA_DIR não pode conter .."));
    }
    let mut ancestor = absolute.clone();
    let mut tail = Vec::new();
    while !ancestor.exists() {
        tail.push(
            ancestor
                .file_name()
                .ok_or_else(|| Error::new(400, "DATA_DIR inválido"))?
                .to_owned(),
        );
        ancestor.pop();
    }
    let mut resolved = ancestor.canonicalize()?;
    for item in tail.iter().rev() {
        resolved.push(item);
    }
    if resolved.starts_with(&personal) {
        return Err(Error::new(
            400,
            "A pasta data de produção está protegida. Use DATA_DIR externo e isolado.",
        ));
    }
    std::fs::create_dir_all(&resolved)?;
    let resolved = resolved.canonicalize()?;
    if resolved.starts_with(&personal) {
        return Err(Error::new(400, "A pasta de produção está protegida."));
    }
    Ok(resolved)
}
