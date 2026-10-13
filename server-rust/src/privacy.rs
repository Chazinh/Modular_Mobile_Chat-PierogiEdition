//! Native vault compatible with MCVAULT1: DPAPI CurrentUser + AES-256-GCM.
//! Starts no Node/PowerShell and never writes plain-text SQLite.
use crate::{Error, Result};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::{
    aead, hmac,
    rand::{SecureRandom, SystemRandom},
};
use rusqlite::{Connection, MAIN_DB};
use serde_json::{Value, json};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8] = b"MCVAULT1";
const AAD: &[u8] = b"ModularChat.encrypted-storage.v1";
const ENTROPY: &[u8] = b"ModularChat.WindowsVault.v1";
fn invalid() -> Error {
    Error::new(
        409,
        "O cofre/chave está inválido ou não pertence a esta conta do Windows. Preserve ambos os arquivos.",
    )
}

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into().map_err(|_| invalid())?,
        pbData: data.as_ptr().cast_mut(),
    };
    let entropy = CRYPT_INTEGER_BLOB {
        cbData: ENTROPY.len() as u32,
        pbData: ENTROPY.as_ptr().cast_mut(),
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // All structures live until return. Only the buffer allocated by DPAPI is freed.
    let ok = unsafe {
        if protect {
            CryptProtectData(
                &input,
                std::ptr::null(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 || output.pbData.is_null() {
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        return Err(invalid());
    }
    let result = unsafe {
        let bytes = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let result = Zeroizing::new(bytes.to_vec());
        bytes.zeroize();
        LocalFree(output.pbData.cast());
        result
    };
    Ok(result)
}
#[cfg(not(windows))]
fn dpapi(_: &[u8], _: bool) -> Result<Zeroizing<Vec<u8>>> {
    Err(Error::new(400, "O cofre DPAPI requer Windows."))
}

fn key_for(file: &Path) -> Result<Zeroizing<Vec<u8>>> {
    safe_file(file)?;
    let key = dpapi(&fs::read(file)?, false)?;
    if key.len() != 32 {
        return Err(invalid());
    }
    Ok(key)
}
fn cipher(key: &[u8]) -> Result<aead::LessSafeKey> {
    Ok(aead::LessSafeKey::new(
        aead::UnboundKey::new(&aead::AES_256_GCM, key).map_err(|_| invalid())?,
    ))
}
const NONCE_AT: usize = MAGIC.len();
const TAG_AT: usize = NONCE_AT + 12;
const HEADER: usize = TAG_AT + 16; // MAGIC | nonce | tag
/// Builds MAGIC|nonce|tag|ciphertext directly in `out` (reusing its capacity): a single copy of the plain
/// text, encrypted in place, with no `to_vec` or `concat`.
fn seal_into(data: &[u8], key: &[u8], out: &mut Vec<u8>) -> Result<()> {
    let mut nonce = [0; 12];
    SystemRandom::new()
        .fill(&mut nonce)
        .map_err(|_| invalid())?;
    out.clear();
    out.reserve(HEADER + data.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce);
    out.extend_from_slice(&[0; 16]);
    out.extend_from_slice(data);
    let tag = cipher(key)?
        .seal_in_place_separate_tag(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut out[HEADER..],
        )
        .map_err(|_| invalid())?;
    out[TAG_AT..HEADER].copy_from_slice(tag.as_ref());
    Ok(())
}
pub fn seal(data: &[u8], key: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    seal_into(data, key, &mut out)?;
    Ok(out)
}
pub fn unseal(data: &[u8], key: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if data.len() < HEADER || &data[..NONCE_AT] != MAGIC {
        return Err(invalid());
    }
    let nonce: [u8; 12] = data[NONCE_AT..TAG_AT].try_into().map_err(|_| invalid())?;
    let mut encrypted = Zeroizing::new([&data[HEADER..], &data[TAG_AT..HEADER]].concat());
    let len = cipher(key)?
        .open_in_place(
            aead::Nonce::assume_unique_for_key(nonce),
            aead::Aad::from(AAD),
            &mut encrypted,
        )
        .map_err(|_| invalid())?
        .len();
    encrypted.truncate(len);
    Ok(encrypted)
}
pub fn safe_file(file: &Path) -> Result<()> {
    if !file.exists() {
        return Ok(());
    }
    if file.symlink_metadata()?.file_type().is_symlink() {
        return Err(Error::new(
            400,
            "Arquivos do cofre não podem ser links simbólicos.",
        ));
    }
    // Every write is tmp + rename: the vault name points to a new file and any
    // other name (hard link) keeps the old content. An extra link must not prevent saving
    // (it once made a finished reply fail its final persistence); warn and carry on.
    let links = crate::config::hard_links(file)?;
    if links > 1 && !LINK_WARNED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        eprintln!(
            "Aviso: {} tem {links} nomes (hard link), em geral criado por um programa que vigia a pasta do projeto (sincronizador de nuvem, antivírus). A gravação atômica segue segura; este aviso aparece uma vez.",
            file.display()
        );
    }
    Ok(())
}
static LINK_WARNED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
// Windows refuses the rename while an antivirus, indexer or file watcher keeps the target open without
// delete sharing; it usually lasts milliseconds, so retry before failing.
fn rename_with_retry(from: &Path, to: &Path) -> std::io::Result<()> {
    let mut attempt = 0u64;
    loop {
        match fs::rename(from, to) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let busy = e.kind() == std::io::ErrorKind::PermissionDenied
                    || matches!(e.raw_os_error(), Some(5 | 32 | 33));
                attempt += 1;
                if !busy || attempt >= 12 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(10 * attempt));
            }
        }
    }
}
pub fn atomic(file: &Path, bytes: &[u8]) -> Result<()> {
    safe_file(file)?;
    let parent = file.parent().ok_or_else(invalid)?;
    fs::create_dir_all(parent)?;
    let tmp = parent.join(format!(".mc-{}.tmp", crate::id()));
    let result = (|| {
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        rename_with_retry(&tmp, file)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
struct Lock(PathBuf);
impl Lock {
    fn acquire(dir: &Path) -> Result<Self> {
        let file = dir.join("modularchat.db.vault.lock");
        safe_file(&file)?;
        if file.exists() {
            let pid = fs::read_to_string(&file)?.trim().parse::<u32>().map_err(|_| Error::new(409,"Lock do cofre inválido. Preserve os dados e revise o processo proprietário."))?;
            if active(pid) {
                return Err(Error::new(
                    409,
                    "O cofre já está aberto por outro servidor. Encerre a instância atual antes de iniciar.",
                ));
            }
            fs::remove_file(&file)?;
        }
        let mut f = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file)?;
        write!(f, "{}", std::process::id())?;
        f.sync_all()?;
        Ok(Self(file))
    }
}
#[cfg(windows)]
fn active(pid: u32) -> bool {
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, GetLastError},
        System::Threading::{GetExitCodeProcess, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    };
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return GetLastError() != ERROR_INVALID_PARAMETER;
        }
        let mut code = 259;
        let success = GetExitCodeProcess(h, &mut code);
        CloseHandle(h);
        success == 0 || code == 259
    }
}
#[cfg(not(windows))]
fn active(_: u32) -> bool {
    true
}
impl Drop for Lock {
    fn drop(&mut self) {
        if fs::read_to_string(&self.0).is_ok_and(|p| p.trim() == std::process::id().to_string()) {
            let _ = fs::remove_file(&self.0);
        }
    }
}
pub struct Vault {
    pub file: PathBuf,
    key_file: PathBuf,
    key: Zeroizing<Vec<u8>>,
    /// Encryption buffer reused between writes (holds ciphertext only).
    scratch: std::sync::Mutex<Vec<u8>>,
    saves: AtomicU64,
    bytes: AtomicU64,
    micros: AtomicU64,
    max_micros: AtomicU64,
    _lock: Lock,
}
pub fn deserialize(data: &mut [u8]) -> Result<Connection> {
    if data.len() < 100 || &data[..16] != b"SQLite format 3\0" {
        return Err(invalid());
    }
    data[18] = 1;
    data[19] = 1; // An in-memory WAL snapshot has no WAL/SHM files.
    let mut db = Connection::open_in_memory()?;
    db.deserialize_read_exact(MAIN_DB, &data[..], data.len(), false)?;
    let check: String = db.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    if check != "ok" {
        return Err(invalid());
    }
    Ok(db)
}
pub fn read_snapshot(file: &Path) -> Result<Connection> {
    safe_file(file)?;
    let key_file = PathBuf::from(
        file.to_string_lossy()
            .strip_suffix(".vault")
            .ok_or_else(invalid)?
            .to_owned()
            + ".key",
    );
    let key = key_for(&key_file)?;
    let mut snapshot = unseal(&fs::read(file)?, &key)?;
    deserialize(&mut snapshot)
}
pub fn snapshot(source: &Path, target: &Path) -> Result<()> {
    if source == target || target.exists() {
        return Err(Error::new(
            409,
            "Destino do backup deve ser novo e independente.",
        ));
    }
    let _ = read_snapshot(source)?;
    let source_key = PathBuf::from(
        source
            .to_string_lossy()
            .strip_suffix(".vault")
            .ok_or_else(invalid)?
            .to_owned()
            + ".key",
    );
    let target_key = PathBuf::from(
        target
            .to_string_lossy()
            .strip_suffix(".vault")
            .ok_or_else(invalid)?
            .to_owned()
            + ".key",
    );
    if target_key.exists() {
        return Err(Error::new(409, "Chave de destino já existe."));
    }
    // A full read observes a consistent version even during an atomic rename.
    let bytes = fs::read(source)?;
    let key = key_for(&source_key)?;
    let mut clear = unseal(&bytes, &key)?;
    let _ = deserialize(&mut clear)?;
    atomic(&target_key, &fs::read(source_key)?)?;
    atomic(target, &bytes)?;
    let _ = read_snapshot(target)?;
    Ok(())
}
impl Vault {
    pub fn open(dir: &Path) -> Result<(Self, Connection)> {
        fs::create_dir_all(dir)?;
        if dir.join("modularchat.db").exists() {
            return Err(Error::new(
                409,
                "Há um SQLite em texto claro. Preserve-o; a produção Rust requer o cofre, sem recriar ou apagar dados.",
            ));
        }
        let lock = Lock::acquire(dir)?;
        // Interrupted writes leave .mc-*.tmp files; with the vault locked by us, no other instance writes.
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let name = entry.file_name();
                let name = name.to_string_lossy();
                if name.starts_with(".mc-") && name.ends_with(".tmp") {
                    let _ = fs::remove_file(entry.path());
                }
            }
        }
        let file = dir.join("modularchat.db.vault");
        let key_file = dir.join("modularchat.db.key");
        safe_file(&file)?;
        let key = if key_file.exists() {
            key_for(&key_file)?
        } else {
            if file.exists() {
                return Err(invalid());
            }
            let mut key = Zeroizing::new(vec![0; 32]);
            SystemRandom::new().fill(&mut key).map_err(|_| invalid())?;
            atomic(&key_file, &dpapi(&key, true)?)?;
            key
        };
        let db = if file.exists() {
            let mut data = unseal(&fs::read(&file)?, &key)?;
            deserialize(&mut data)?
        } else {
            Connection::open_in_memory()?
        };
        db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA journal_mode=MEMORY;")?;
        Ok((
            Self {
                file,
                key_file,
                key,
                scratch: std::sync::Mutex::default(),
                saves: AtomicU64::new(0),
                bytes: AtomicU64::new(0),
                micros: AtomicU64::new(0),
                max_micros: AtomicU64::new(0),
                _lock: lock,
            },
            db,
        ))
    }
    /// Writes the whole database (tmp + fsync + rename). `serialize` points at SQLite's memory (no copy)
    /// and encryption happens in the reused buffer: no transient database-sized copy besides it.
    pub fn save(&self, db: &Connection) -> Result<()> {
        let started = std::time::Instant::now();
        let data = db.serialize(MAIN_DB)?;
        let mut buf = self.scratch.lock().unwrap_or_else(|e| e.into_inner());
        seal_into(&data, &self.key, &mut buf)?;
        drop(data);
        atomic(&self.file, &buf)?;
        let len = buf.len();
        // Does not keep twice the needed size when the database shrank (cleanup, deletions).
        if buf.capacity() > len.saturating_mul(2).max(1 << 20) {
            buf.shrink_to(len + len / 4);
        }
        let micros = started.elapsed().as_micros() as u64;
        self.saves.fetch_add(1, Ordering::Relaxed);
        self.bytes.fetch_add(len as u64, Ordering::Relaxed);
        self.micros.fetch_add(micros, Ordering::Relaxed);
        self.max_micros.fetch_max(micros, Ordering::Relaxed);
        Ok(())
    }
    /// Write counters since opening (diagnostics and tests).
    pub fn stats(&self) -> Value {
        let saves = self.saves.load(Ordering::Relaxed);
        let micros = self.micros.load(Ordering::Relaxed);
        json!({
            "saves": saves,
            "bytes": self.bytes.load(Ordering::Relaxed),
            "avgMs": if saves == 0 { 0.0 } else { micros as f64 / saves as f64 / 1000.0 },
            "maxMs": self.max_micros.load(Ordering::Relaxed) as f64 / 1000.0,
        })
    }
    pub fn cache_key(&self) -> Value {
        json!({"key":STANDARD.encode(hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256,&self.key),b"ModularChat.browser-cache.v1").as_ref())})
    }
    pub fn encrypt_export(&self, data: &Value) -> Result<Value> {
        Ok(
            json!({"format":"modularchat-encrypted-export","version":1,"data":STANDARD.encode(seal(data.to_string().as_bytes(),&self.key)?)}),
        )
    }
    pub fn decrypt_export(&self, data: &Value) -> Result<Value> {
        let bytes = STANDARD
            .decode(data["data"].as_str().ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        serde_json::from_slice(&unseal(&bytes, &self.key)?).map_err(|_| invalid())
    }
    pub fn write_file(&self, file: &Path, text: &str) -> Result<()> {
        atomic(file, &seal(text.as_bytes(), &self.key)?)
    }
    pub fn read_file(&self, file: &Path) -> Result<Option<String>> {
        if !file.exists() {
            return Ok(None);
        }
        safe_file(file)?;
        if file.metadata()?.len() > 800_037 {
            return Err(Error::new(422, "Ficha protegida grande demais."));
        }
        String::from_utf8(unseal(&fs::read(file)?, &self.key)?.to_vec())
            .map(Some)
            .map_err(|_| invalid())
    }
    pub fn backup(&self, db: &Connection, file: &Path) -> Result<()> {
        let key_file = PathBuf::from(
            file.to_string_lossy()
                .strip_suffix(".vault")
                .ok_or_else(invalid)?
                .to_owned()
                + ".key",
        );
        atomic(&key_file, &fs::read(&self.key_file)?)?;
        atomic(file, &seal(&db.serialize(MAIN_DB)?, &self.key)?)?;
        let _ = read_snapshot(file)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_cipher_rejects_changes_and_wrong_key() {
        let key = [7; 32];
        let clear = b"**segredo de teste**";
        let mut a = seal(clear, &key).unwrap();
        let b = seal(clear, &key).unwrap();
        assert_ne!(a, b);
        assert_eq!(&**unseal(&a, &key).unwrap(), clear);
        assert!(unseal(&a, &[8; 32]).is_err());
        a[37] ^= 1;
        assert!(unseal(&a, &key).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn native_dpapi_reopens_private_sqlite_and_refuses_live_lock() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, db) = Vault::open(dir.path()).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        db.execute_batch(
            "CREATE TABLE secrets(text); INSERT INTO secrets VALUES('MARCADOR_SECRETO');",
        )
        .unwrap();
        vault.save(&db).unwrap();
        assert!(!dir.path().join("modularchat.db").exists());
        assert!(
            !fs::read(&vault.file)
                .unwrap()
                .windows(16)
                .any(|w| w == b"MARCADOR_SECRETO")
        );
        drop(db);
        drop(vault);
        let (vault, db) = Vault::open(dir.path()).unwrap();
        let text: String = db
            .query_row("SELECT text FROM secrets", [], |r| r.get(0))
            .unwrap();
        assert_eq!(text, "MARCADOR_SECRETO");
        vault
            .backup(&db, &dir.path().join("backups/test.db.vault"))
            .unwrap();
        let copied = read_snapshot(&dir.path().join("backups/test.db.vault")).unwrap();
        assert_eq!(
            copied
                .query_row("SELECT text FROM secrets", [], |r| r.get::<_, String>(0))
                .unwrap(),
            text
        );
    }
    #[test]
    fn atomic_save_survives_an_extra_hard_link_and_breaks_it() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("x.vault");
        atomic(&file, b"antigo").unwrap();
        let alias = dir.path().join("alias.vault");
        fs::hard_link(&file, &alias).unwrap();
        assert!(crate::config::hard_links(&file).unwrap() > 1);
        atomic(&file, b"novo").unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"novo");
        assert_eq!(fs::read(&alias).unwrap(), b"antigo");
        assert_eq!(crate::config::hard_links(&file).unwrap(), 1);
    }
}
