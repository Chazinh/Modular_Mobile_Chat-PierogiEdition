//! JNI owns no Android lifecycle: bounded UTF-8 copies enter the existing Rust host.
//! The Keystore class is captured on a Java thread, then used by attached store threads.
use super::*;
use ::jni::{
    JNIEnv, JavaVM,
    objects::{GlobalRef, JByteArray, JClass, JValue},
    sys::jint,
};

struct Keystore {
    vm: JavaVM,
    class: GlobalRef,
}
static KEYSTORE: OnceLock<Keystore> = OnceLock::new();

fn init_keystore(env: &mut JNIEnv<'_>) -> ::jni::errors::Result<()> {
    if KEYSTORE.get().is_none() {
        let class = env.find_class("io/github/modularchat/pierogiedition/NativeVaultKey")?;
        let bridge = Keystore {
            vm: env.get_java_vm()?,
            class: env.new_global_ref(class)?,
        };
        let _ = KEYSTORE.set(bridge);
    }
    Ok(())
}

pub(crate) fn vault_key(bytes: &[u8], protect: bool) -> Result<Zeroizing<Vec<u8>>> {
    let unavailable = || {
        Error::new(
            409,
            "Chave do cofre Android indisponível. Preserve os dados e o Android Keystore.",
        )
    };
    let bridge = KEYSTORE.get().ok_or_else(unavailable)?;
    let mut env = bridge
        .vm
        .attach_current_thread()
        .map_err(|_| unavailable())?;
    let result = env.with_local_frame(8, |env| -> ::jni::errors::Result<_> {
        let input = env.byte_array_from_slice(bytes)?;
        let class: &JClass<'_> = bridge.class.as_obj().into();
        let call = env.call_static_method(
            class,
            if protect { "protect" } else { "unprotect" },
            "([B)[B",
            &[JValue::Object(input.as_ref())],
        );
        // Java also wipes this copy in finally, including on cipher failure.
        if env.exception_check()? {
            env.exception_clear()?;
        }
        env.set_byte_array_region(&input, 0, &vec![0; bytes.len()])?;
        let output = JByteArray::from(call?.l()?);
        let copied = Zeroizing::new(env.convert_byte_array(&output)?);
        env.set_byte_array_region(&output, 0, &vec![0; copied.len()])?;
        Ok(copied)
    });
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_clear();
    }
    result.map_err(|_| unavailable())
}

fn input(env: &JNIEnv<'_>, array: &JByteArray<'_>, max: usize) -> Result<Zeroizing<String>> {
    if array.is_null() {
        return Err(Error::new(400, "Entrada local ausente."));
    }
    let len = env
        .get_array_length(array)
        .map_err(|_| Error::new(400, "Entrada local inválida."))?;
    if len <= 0 || len as usize > max {
        return Err(Error::new(400, "Entrada local excedeu o limite."));
    }
    let bytes = Zeroizing::new(
        env.convert_byte_array(array)
            .map_err(|_| Error::new(400, "Entrada local inválida."))?,
    );
    let text =
        std::str::from_utf8(&bytes).map_err(|_| Error::new(400, "Entrada local não é UTF-8."))?;
    if text.contains('\0') {
        return Err(Error::new(400, "Entrada local contém NUL."));
    }
    Ok(Zeroizing::new(text.to_owned()))
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_modularchat_pierogiedition_NativeHost_nativeStart(
    mut env: JNIEnv<'_>,
    _class: JClass<'_>,
    data: JByteArray<'_>,
    native: JByteArray<'_>,
    token: JByteArray<'_>,
) -> jint {
    ffi_result(|| {
        let options = (|| {
            let data = input(&env, &data, 4096)?;
            let native = input(&env, &native, 4096)?;
            let token = input(&env, &token, 256)?;
            init_keystore(&mut env)
                .map_err(|_| Error::new(409, "Android Keystore indisponível."))?;
            StartOptions::new(
                PathBuf::from(data.as_str()),
                PathBuf::from(native.as_str()),
                token,
            )
        })();
        match options {
            Ok(options) => host().start(options),
            Err(_) => INVALID_INPUT,
        }
    })
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_modularchat_pierogiedition_NativeHost_nativeStop(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jint {
    mc_android_stop()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_modularchat_pierogiedition_NativeHost_nativeStatus(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jint {
    mc_android_status()
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_io_github_modularchat_pierogiedition_NativeHost_nativeLastError(
    _env: JNIEnv<'_>,
    _class: JClass<'_>,
) -> jint {
    mc_android_last_error()
}
