package io.github.modularchat.pierogiedition;

import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** OS boundary only: Rust owns the vault, its random data key and atomic migration. */
final class NativeVaultKey {
    private static final String ALIAS = "ModularChat.NativeHost.Vault.v1";
    private static final byte[] AAD = ALIAS.getBytes(StandardCharsets.UTF_8);

    private static SecretKey key(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (store.containsAlias(ALIAS)) {
            SecretKey existing = (SecretKey)store.getKey(ALIAS, null);
            if (existing != null) return existing;
        }
        if (!create) throw new GeneralSecurityException("Vault key unavailable.");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }

    static synchronized byte[] protect(byte[] input) throws Exception {
        try {
            if (input == null || input.length != 32) throw new GeneralSecurityException("Invalid vault key.");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, key(true));
            cipher.updateAAD(AAD);
            byte[] iv = cipher.getIV();
            if (iv.length != 12) throw new GeneralSecurityException("Invalid vault nonce.");
            byte[] encrypted = cipher.doFinal(input);
            byte[] result = Arrays.copyOf(iv, iv.length + encrypted.length);
            System.arraycopy(encrypted, 0, result, iv.length, encrypted.length);
            return result;
        } catch (Exception error) {
            android.util.Log.e("NativeVaultKey", "protect failed", error);
            throw error;
        } finally { if (input != null) Arrays.fill(input, (byte)0); }
    }

    static synchronized byte[] unprotect(byte[] input) throws Exception {
        try {
            if (input == null || input.length != 60) throw new GeneralSecurityException("Invalid vault envelope.");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, key(false), new GCMParameterSpec(128, input, 0, 12));
            cipher.updateAAD(AAD);
            return cipher.doFinal(input, 12, input.length - 12);
        } catch (Exception error) {
            android.util.Log.e("NativeVaultKey", "unprotect failed", error);
            throw error;
        } finally { if (input != null) Arrays.fill(input, (byte)0); }
    }
}
