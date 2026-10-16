//! Native MCVAULT1 vault: AES-256-GCM, DPAPI CurrentUser on Windows.
//! Starts no Node/PowerShell and never writes plain-text SQLite.
//! Android wraps the data key with Android Keystore through the direct JNI boundary.
//! Other non-Windows platforms use a versioned sandbox key (0700/0600 on Unix).
//! DPAPI and sandbox key files are intentionally distinct and are not cross-platform credentials.
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
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use zeroize::{Zeroize, Zeroizing};

const MAGIC: &[u8] = b"MCVAULT1";
const AAD: &[u8] = b"ModularChat.encrypted-storage.v1";
#[cfg(windows)]
const ENTROPY: &[u8] = b"ModularChat.WindowsVault.v1";
#[cfg(not(windows))]
const KEY_MAGIC: &[u8] = b"MCKEY001";
#[cfg(target_os = "android")]
const ANDROID_KEY_MAGIC: &[u8] = b"MCKEYKS1";
fn invalid() -> Error {
    #[cfg(windows)]
    let message = "O cofre/chave está inválido ou não pertence a esta conta do Windows. Preserve ambos os arquivos.";
    #[cfg(not(windows))]
    let message = "O cofre/chave do sandbox está ausente ou inválido. Preserve ambos os arquivos.";
    Error::new(409, message)
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
fn key_for(file: &Path) -> Result<Zeroizing<Vec<u8>>> {
    let mut f = open_regular(file, true)?;
    if f.metadata()?.len() > 65536 {
        return Err(invalid());
    }
    let mut bytes = Zeroizing::new(Vec::new());
    Read::by_ref(&mut f).take(65537).read_to_end(&mut bytes)?;
    #[cfg(windows)]
    let key = dpapi(&bytes, false)?;
    #[cfg(not(windows))]
    let key = {
        #[cfg(target_os = "android")]
        if bytes.starts_with(ANDROID_KEY_MAGIC) {
            let key = crate::android::jni::vault_key(&bytes[ANDROID_KEY_MAGIC.len()..], false)?;
            if key.len() != 32 { return Err(invalid()); }
            return Ok(key);
        }
        let key_end = KEY_MAGIC.len() + 32;
        if bytes.len() != key_end + 32
            || !bytes.starts_with(KEY_MAGIC)
            || ring::digest::digest(&ring::digest::SHA256, &bytes[..key_end]).as_ref()
                != &bytes[key_end..]
        {
            return Err(invalid());
        }
        Zeroizing::new(bytes[KEY_MAGIC.len()..key_end].to_vec())
    };
    if key.len() != 32 {
        return Err(invalid());
    }
    Ok(key)
}
fn protected_key(key: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if key.len() != 32 {
        return Err(invalid());
    }
    #[cfg(windows)]
    {
        dpapi(key, true)
    }
    #[cfg(target_os = "android")]
    {
        let envelope = crate::android::jni::vault_key(key, true)?;
        // Verify the OS can recover the exact key before publishing or migrating it.
        let verified = crate::android::jni::vault_key(&envelope, false)?;
        if verified.as_slice() != key { return Err(invalid()); }
        let mut bytes = Zeroizing::new(ANDROID_KEY_MAGIC.to_vec());
        bytes.extend_from_slice(&envelope);
        Ok(bytes)
    }
    #[cfg(all(not(windows), not(target_os = "android")))]
    {
        let mut bytes = Zeroizing::new(Vec::with_capacity(KEY_MAGIC.len() + 64));
        bytes.extend_from_slice(KEY_MAGIC);
        bytes.extend_from_slice(key);
        // Detect truncated/corrupt key files even before the first vault save. This checksum
        // is NOT an authentication mechanism; GCM authenticates the database itself.
        let check = ring::digest::digest(&ring::digest::SHA256, &bytes);
        bytes.extend_from_slice(check.as_ref());
        Ok(bytes)
    }
}
fn snapshot_key_path(file: &Path) -> Result<PathBuf> {
    let base = file
        .to_str()
        .and_then(|s| s.strip_suffix(".vault"))
        .ok_or_else(invalid)?;
    #[cfg(windows)]
    let suffix = ".key";
    #[cfg(not(windows))]
    let suffix = ".sandbox.key";
    Ok(PathBuf::from(format!("{base}{suffix}")))
}

/// The leaf directory must be real. Unix directories created for the vault and backups are private.
pub(crate) fn private_dir(dir: &Path) -> Result<()> {
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_dir() => {
            return Err(Error::new(
                400,
                "Diretório do cofre não pode ser um link ou arquivo.",
            ));
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(dir)?;
        }
        Err(e) => return Err(e.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut opts = fs::OpenOptions::new();
        opts.read(true);
        nofollow(&mut opts);
        let handle = opts.open(dir)?;
        if !handle.metadata()?.is_dir() || fs::symlink_metadata(dir)?.file_type().is_symlink() {
            return Err(Error::new(400, "Diretório do cofre inválido."));
        }
        handle.set_permissions(fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn nofollow(options: &mut fs::OpenOptions) {
    // Linux/Android's stable O_NOFOLLOW value; avoids following a link between check and open.
    // Other systems also verify metadata below and never truncate an existing file before validation.
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(0x20000);
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    let _ = options;
}
fn verify_regular(path: &Path, file: &fs::File, private: bool) -> Result<()> {
    let named = fs::symlink_metadata(path)?;
    let opened = file.metadata()?;
    if named.file_type().is_symlink() || !named.is_file() || !opened.is_file() {
        return Err(Error::new(
            400,
            "Arquivo do cofre não pode ser um link ou diretório.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        if named.dev() != opened.dev()
            || named.ino() != opened.ino()
            || private && opened.nlink() != 1
        {
            return Err(Error::new(
                400,
                "Arquivo privado do cofre foi substituído ou possui hard links.",
            ));
        }
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(())
}
fn open_regular(path: &Path, private: bool) -> Result<fs::File> {
    safe_file(path)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    nofollow(&mut options);
    let file = options.open(path)?;
    verify_regular(path, &file, private)?;
    Ok(file)
}
fn read_regular(path: &Path) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    open_regular(path, false)?.read_to_end(&mut bytes)?;
    Ok(bytes)
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
    // A rejected key must never put plaintext into the reusable ciphertext buffer.
    let cipher = cipher(key)?;
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
    let tag = match cipher.seal_in_place_separate_tag(
        aead::Nonce::assume_unique_for_key(nonce),
        aead::Aad::from(AAD),
        &mut out[HEADER..],
    ) {
        Ok(tag) => tag,
        Err(_) => {
            out[HEADER..].zeroize();
            out.clear();
            return Err(invalid());
        }
    };
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
    let meta = match fs::symlink_metadata(file) {
        Ok(meta) => meta,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
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
    atomic_write(file, bytes, true)
}
fn atomic_new(file: &Path, bytes: &[u8]) -> Result<()> {
    atomic_write(file, bytes, false)
}
fn atomic_write(file: &Path, bytes: &[u8], replace: bool) -> Result<()> {
    safe_file(file)?;
    let parent = file.parent().ok_or_else(invalid)?;
    private_dir(parent)?;
    let tmp = parent.join(format!(".mc-{}.tmp", crate::id()));
    let result = (|| {
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut f = options.open(&tmp)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            f.set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        if replace {
            rename_with_retry(&tmp, file)?;
        } else {
            #[cfg(target_os = "android")]
            {
                // Android untrusted_app SELinux blocks linkat (hard link) with EACCES (os error 13).
                // Within the UID-isolated sandbox protected by the vault lock, atomic rename safely publishes.
                if file.exists() {
                    return Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "Arquivo de destino já existe.").into());
                }
                rename_with_retry(&tmp, file)?;
            }
            #[cfg(not(target_os = "android"))]
            {
                // A same-directory hard link publishes the completely flushed key without replacing
                // an existing key, including a dangling symlink. Never truncate/regenerate key material.
                fs::hard_link(&tmp, file)?;
                fs::remove_file(&tmp)?;
            }
        }
        #[cfg(all(unix, not(target_os = "android")))]
        fs::File::open(parent)?.sync_all()?;
        #[cfg(target_os = "android")]
        {
            if let Ok(dir_file) = fs::File::open(parent) {
                let _ = dir_file.sync_all();
            }
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(tmp);
    }
    result
}
struct Lock {
    #[cfg(windows)]
    path: PathBuf,
    #[cfg(not(windows))]
    file: fs::File,
}
impl Lock {
    #[cfg(windows)]
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
        Ok(Self { path: file })
    }
    #[cfg(not(windows))]
    fn acquire(dir: &Path) -> Result<Self> {
        use std::io::{Seek, SeekFrom};
        let path = dir.join("modularchat.db.vault.lock");
        safe_file(&path)?;
        let mut options = fs::OpenOptions::new();
        options.read(true).write(true).create(true); // deliberately no truncation
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        nofollow(&mut options);
        let mut file = options.open(&path)?;
        verify_regular(&path, &file, true)?;
        file.try_lock().map_err(|e| match e {
            fs::TryLockError::WouldBlock => {
                Error::new(409, "O cofre já está aberto por outro servidor.")
            }
            fs::TryLockError::Error(e) => e.into(),
        })?;
        // Keep compatibility with legacy PID-only locks. An unlocked file still belonging to a
        // live legacy writer must not be stolen; a crashed writer's PID can be recovered.
        let mut owner = String::new();
        Read::by_ref(&mut file)
            .take(128)
            .read_to_string(&mut owner)?;
        if !owner.trim().is_empty() {
            let pid = owner.trim().parse::<u32>().ok().filter(|p| *p > 0)
                .ok_or_else(|| Error::new(409, "Lock do cofre inválido. Preserve os dados e revise o processo proprietário."))?;
            if active(pid) {
                return Err(Error::new(
                    409,
                    "O cofre já está aberto por outro servidor.",
                ));
            }
        }
        verify_regular(&path, &file, true)?;
        file.set_len(0)?;
        file.seek(SeekFrom::Start(0))?;
        write!(file, "{}", std::process::id())?;
        file.sync_all()?;
        // The inode persists. Unlinking it would let a second writer lock a different inode.
        Ok(Self { file })
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
#[cfg(all(target_os = "linux", not(target_os = "android")))]
fn active(pid: u32) -> bool {
    // /proc is present on Linux; inaccessible metadata is conservatively treated as live.
    fs::metadata(format!("/proc/{pid}"))
        .map_or_else(|e| e.kind() != std::io::ErrorKind::NotFound, |_| true)
}
#[cfg(target_os = "android")]
fn active(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
    }
    let Ok(pid) = std::ffi::c_int::try_from(pid) else {
        return true;
    };
    // On Android, untrusted_app SELinux blocks /proc/<pid> inspection with EACCES.
    // Within the app sandbox, all processes share the app's Linux UID.
    // POSIX kill(pid, 0) == 0 means a process with our UID exists.
    // ESRCH (3) means no such process exists (it was killed/crashed).
    // EPERM (1) means the PID was recycled by the OS for another UID (not this app).
    unsafe {
        if kill(pid, 0) == 0 {
            true
        } else {
            let err = std::io::Error::last_os_error().raw_os_error();
            err != Some(3) && err != Some(1)
        }
    }
}
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
fn active(pid: u32) -> bool {
    unsafe extern "C" {
        fn kill(pid: std::ffi::c_int, signal: std::ffi::c_int) -> std::ffi::c_int;
    }
    let Ok(pid) = std::ffi::c_int::try_from(pid) else {
        return true;
    };
    // signal 0 only probes existence; ESRCH=3 is the POSIX/BSD "no process" result.
    unsafe { kill(pid, 0) == 0 || std::io::Error::last_os_error().raw_os_error() != Some(3) }
}
#[cfg(not(any(windows, unix)))]
fn active(_: u32) -> bool {
    true
}
impl Drop for Lock {
    fn drop(&mut self) {
        #[cfg(windows)]
        if fs::read_to_string(&self.path).is_ok_and(|p| p.trim() == std::process::id().to_string())
        {
            let _ = fs::remove_file(&self.path);
        }
        #[cfg(not(windows))]
        {
            // Clear our PID while still holding the advisory lock; File's drop releases the lock,
            // also automatically after SIGKILL. Never remove a live instance's lock file.
            let _ = self.file.set_len(0);
            let _ = self.file.sync_all();
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
    let key_file = snapshot_key_path(file)?;
    let key = key_for(&key_file)?;
    let mut snapshot = unseal(&read_regular(file)?, &key)?;
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
    let source_key = snapshot_key_path(source)?;
    let target_key = snapshot_key_path(target)?;
    if target_key.exists() {
        return Err(Error::new(409, "Chave de destino já existe."));
    }
    // A full read observes a consistent version even during an atomic rename.
    let bytes = read_regular(source)?;
    let key = key_for(&source_key)?;
    let mut clear = unseal(&bytes, &key)?;
    let _ = deserialize(&mut clear)?;
    #[cfg(target_os = "android")]
    let key_bytes = protected_key(&key)?;
    #[cfg(not(target_os = "android"))]
    let key_bytes = Zeroizing::new(read_regular(&source_key)?);
    atomic_new(&target_key, &key_bytes)?;
    atomic(target, &bytes)?;
    let _ = read_snapshot(target)?;
    Ok(())
}
impl Vault {
    pub fn open(dir: &Path) -> Result<(Self, Connection)> {
        private_dir(dir)?;
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
        let key_file = snapshot_key_path(&file)?;
        safe_file(&file)?;
        safe_file(&key_file)?;
        #[cfg(windows)]
        let other_key = dir.join("modularchat.db.sandbox.key");
        #[cfg(not(windows))]
        let other_key = dir.join("modularchat.db.key");
        if !key_file.exists() && fs::symlink_metadata(other_key).is_ok() {
            return Err(invalid());
        }
        let key = if key_file.exists() {
            key_for(&key_file)?
        } else {
            if file.exists() {
                return Err(invalid());
            }
            let mut key = Zeroizing::new(vec![0; 32]);
            SystemRandom::new().fill(&mut key).map_err(|_| invalid())?;
            atomic_new(&key_file, &protected_key(&key)?)?;
            key
        };
        let db = if file.exists() {
            let mut data = unseal(&read_regular(&file)?, &key)?;
            deserialize(&mut data)?
        } else {
            Connection::open_in_memory()?
        };
        db.execute_batch("PRAGMA temp_store=MEMORY; PRAGMA journal_mode=MEMORY;")?;
        #[cfg(target_os = "android")]
        if read_regular(&key_file)?.starts_with(KEY_MAGIC) {
            // The old key and database have both been authenticated/deserialized above.
            // Replace only the envelope, atomically; keep the data key and vault bytes.
            atomic(&key_file, &protected_key(&key)?)?;
        }
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
        String::from_utf8(unseal(&read_regular(file)?, &self.key)?.to_vec())
            .map(Some)
            .map_err(|_| invalid())
    }
    pub fn backup(&self, db: &Connection, file: &Path) -> Result<()> {
        let key_file = snapshot_key_path(file)?;
        let key_bytes = Zeroizing::new(read_regular(&self.key_file)?);
        atomic(&key_file, &key_bytes)?;
        atomic(file, &seal(&db.serialize(MAIN_DB)?, &self.key)?)?;
        let _ = read_snapshot(file)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_seal_key_preserves_the_ciphertext_buffer_sentinel() {
        let sentinel = b"CIPHERTEXT_BUFFER_SENTINEL";
        let plaintext = b"PLAINTEXT_MUST_NOT_ENTER_SCRATCH";
        let mut out = Vec::with_capacity(256);
        out.extend_from_slice(sentinel);
        let capacity = out.capacity();
        for key in [&[][..], &[7; 16][..], &[7; 31][..], &[7; 33][..]] {
            assert!(seal_into(plaintext, key, &mut out).is_err());
            assert_eq!(out, sentinel);
            assert_eq!(out.capacity(), capacity);
        }
        seal_into(plaintext, &[7; 32], &mut out).unwrap();
        assert_eq!(out.capacity(), capacity);
        assert_eq!(&out[..NONCE_AT], MAGIC);
        assert_eq!(out.len(), HEADER + plaintext.len());
        assert_eq!(&**unseal(&out, &[7; 32]).unwrap(), plaintext);
        assert!(!out.windows(plaintext.len()).any(|v| v == plaintext));
    }

    #[test]
    fn authenticated_cipher_rejects_changes_and_wrong_key() {
        let key = [7; 32];
        let clear = b"**segredo de teste**";
        let a = seal(clear, &key).unwrap();
        let b = seal(clear, &key).unwrap();
        assert_ne!(a, b);
        assert_eq!(&**unseal(&a, &key).unwrap(), clear);
        assert!(unseal(&a, &[8; 32]).is_err());
        for offset in [0, NONCE_AT, TAG_AT, HEADER] {
            let mut tampered = a.clone();
            tampered[offset] ^= 1;
            assert!(unseal(&tampered, &key).is_err());
        }
    }

    #[test]
    fn native_vault_reopens_private_sqlite_and_refuses_live_lock() {
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
    #[test]
    fn vault_refuses_missing_wrong_and_corrupt_key_or_ciphertext_without_replacing_data() {
        let dir = tempfile::tempdir().unwrap();
        let (vault, db) = Vault::open(dir.path()).unwrap();
        db.execute_batch(
            "CREATE TABLE secrets(text); INSERT INTO secrets VALUES('PRIVATE_RECORD');",
        )
        .unwrap();
        vault.save(&db).unwrap();
        let file = vault.file.clone();
        let key_file = vault.key_file.clone();
        let original_key = Zeroizing::new(fs::read(&key_file).unwrap());
        let original_vault = fs::read(&file).unwrap();
        drop(db);
        drop(vault);

        fs::remove_file(&key_file).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert!(
            !key_file.exists(),
            "a missing key must never be regenerated for an existing vault"
        );
        fs::write(&key_file, &*original_key).unwrap();

        for bad_key in [
            protected_key(&[42; 32]).unwrap(),
            Zeroizing::new(vec![1, 2, 3]),
        ] {
            fs::write(&key_file, &*bad_key).unwrap();
            assert!(Vault::open(dir.path()).is_err());
            assert_eq!(fs::read(&key_file).unwrap(), *bad_key);
            assert_eq!(fs::read(&file).unwrap(), original_vault);
        }
        fs::write(&key_file, &*original_key).unwrap();
        let mut tampered = original_vault.clone();
        *tampered.last_mut().unwrap() ^= 1;
        fs::write(&file, &tampered).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert_eq!(fs::read(&file).unwrap(), tampered);
        fs::write(&file, &original_vault).unwrap();
        assert!(Vault::open(dir.path()).is_ok());
    }
    #[test]
    fn key_publication_never_overwrites_an_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("existing.key");
        atomic_new(&file, b"original").unwrap();
        assert!(atomic_new(&file, b"replacement").is_err());
        assert_eq!(fs::read(&file).unwrap(), b"original");
        assert!(
            !fs::read_dir(dir.path()).unwrap().any(|e| e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".mc-"))
        );
    }
    #[test]
    fn legacy_live_or_invalid_pid_lock_is_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("modularchat.db.vault.lock");
        for owner in [std::process::id().to_string(), "invalid-owner".into()] {
            fs::write(&file, &owner).unwrap();
            assert!(Lock::acquire(dir.path()).is_err());
            assert_eq!(fs::read_to_string(&file).unwrap(), owner);
        }
    }
    #[test]
    #[ignore = "subprocess fixture invoked only by process_lock_recovers_after_kill"]
    fn vault_lock_child_fixture() {
        let Some(dir) = std::env::var_os("MC_PRIVACY_CHILD_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let (vault, db) = Vault::open(&dir).unwrap();
        db.execute_batch(
            "CREATE TABLE secrets(text); INSERT INTO secrets VALUES('ACK_BEFORE_KILL');",
        )
        .unwrap();
        vault.save(&db).unwrap();
        atomic(&dir.join("child-ready"), b"ready").unwrap();
        let _ = std::io::stdin().read(&mut [0u8; 1]);
        drop(db);
        drop(vault);
    }
    #[test]
    fn process_lock_recovers_after_kill_without_stealing_live_owner() {
        use std::process::{Command, Stdio};
        struct ChildGuard(std::process::Child);
        impl Drop for ChildGuard {
            fn drop(&mut self) {
                let _ = self.0.kill();
                let _ = self.0.wait();
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "privacy::tests::vault_lock_child_fixture",
                "--ignored",
                "--test-threads=1",
            ])
            .env("MC_PRIVACY_CHILD_DIR", dir.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = ChildGuard(command.spawn().unwrap());
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !dir.path().join("child-ready").exists() {
            assert!(
                child.0.try_wait().unwrap().is_none(),
                "child exited before acquiring the vault"
            );
            assert!(
                std::time::Instant::now() < deadline,
                "child vault startup timeout"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let lock_file = dir.path().join("modularchat.db.vault.lock");
        let owner = fs::read(&lock_file).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert_eq!(fs::read(&lock_file).unwrap(), owner);
        atomic(
            &dir.path().join(".mc-crashed.tmp"),
            b"interrupted ciphertext",
        )
        .unwrap();
        let pid = child.0.id();
        child.0.kill().unwrap();
        child.0.wait().unwrap();
        assert!(!active(pid));
        let (vault, db) = Vault::open(dir.path()).unwrap();
        assert_eq!(
            db.query_row("SELECT text FROM secrets", [], |r| r.get::<_, String>(0))
                .unwrap(),
            "ACK_BEFORE_KILL"
        );
        assert!(!dir.path().join(".mc-crashed.tmp").exists());
        drop(db);
        drop(vault);
        #[cfg(not(windows))]
        assert!(
            lock_file.is_file(),
            "advisory lock inode must persist across releases"
        );
    }
    #[cfg(unix)]
    #[test]
    fn sandbox_permissions_and_key_format_are_private() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = tempfile::tempdir().unwrap();
        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o755)).unwrap();
        let (vault, db) = Vault::open(dir.path()).unwrap();
        db.execute_batch("CREATE TABLE secrets(text);").unwrap();
        vault.save(&db).unwrap();
        assert_eq!(fs::metadata(dir.path()).unwrap().mode() & 0o777, 0o700);
        for file in [
            &vault.key_file,
            &vault.file,
            &dir.path().join("modularchat.db.vault.lock"),
        ] {
            assert_eq!(fs::metadata(file).unwrap().mode() & 0o777, 0o600);
        }
        let key = Zeroizing::new(fs::read(&vault.key_file).unwrap());
        assert!(key.starts_with(KEY_MAGIC));
        assert_eq!(key.len(), KEY_MAGIC.len() + 64);
        assert!(!dir.path().join("modularchat.db.key").exists());
        let mut corrupted = key.clone();
        corrupted[KEY_MAGIC.len()] ^= 1;
        fs::write(&vault.key_file, &*corrupted).unwrap();
        assert!(
            key_for(&vault.key_file).is_err(),
            "key checksum detects accidental corruption"
        );
        fs::write(&vault.key_file, &*key).unwrap();
        let temporary = dir.path().join(".mc-permission.tmp");
        atomic(&temporary, b"encrypted temporary").unwrap();
        assert_eq!(fs::metadata(&temporary).unwrap().mode() & 0o777, 0o600);
    }
    #[cfg(unix)]
    #[test]
    fn sandbox_rejects_symlinks_and_key_hard_links_without_touching_targets() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing-target");
        let key = dir.path().join("modularchat.db.sandbox.key");
        symlink(&missing, &key).unwrap();
        assert!(Vault::open(dir.path()).is_err());
        assert!(fs::symlink_metadata(&key).unwrap().file_type().is_symlink());
        assert!(!missing.exists());
        fs::remove_file(&key).unwrap();
        let linked_dir = dir.path().join("linked-directory");
        symlink(dir.path(), &linked_dir).unwrap();
        assert!(Vault::open(&linked_dir).is_err());
        let (vault, db) = Vault::open(dir.path()).unwrap();
        let alias = dir.path().join("key-alias");
        fs::hard_link(&vault.key_file, &alias).unwrap();
        assert!(key_for(&vault.key_file).is_err());
        drop(db);
        drop(vault);
    }
}
