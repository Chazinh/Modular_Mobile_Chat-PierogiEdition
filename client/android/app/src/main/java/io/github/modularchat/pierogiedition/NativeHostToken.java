package io.github.modularchat.pierogiedition;

import android.content.Context;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.system.Os;
import android.util.AtomicFile;
import android.util.Base64;
import java.io.File;
import java.io.FileInputStream;
import java.io.FileOutputStream;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.GeneralSecurityException;
import java.security.KeyStore;
import java.security.SecureRandom;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** A separate device credential; never shares PrivateStorage keys or desktop pairing state. */
final class NativeHostToken {
    private static final String ALIAS = "ModularChat.NativeHost.Token.v1";
    private static final byte[] AAD = ALIAS.getBytes(StandardCharsets.UTF_8);
    private static final int MAGIC = 0x4d434831;

    private static SecretKey key(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (store.containsAlias(ALIAS)) return (SecretKey)store.getKey(ALIAS, null);
        if (!create) throw new GeneralSecurityException("Local credential key unavailable.");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }

    static synchronized byte[] get(Context context) throws Exception {
        File base = new File(context.getNoBackupFilesDir(), "native-host-token.v1");
        AtomicFile file = new AtomicFile(base);
        if (base.exists() || new File(base.getPath() + ".bak").exists()) {
            byte[] envelope = null, encrypted = null, token = null;
            try (FileInputStream input = file.openRead()) {
                long size = input.getChannel().size();
                if (size < 33 || size > 512) throw new GeneralSecurityException("Invalid local credential.");
                envelope = new byte[(int)size];
                int offset = 0, read;
                while (offset < envelope.length && (read = input.read(envelope, offset, envelope.length - offset)) > 0) offset += read;
                if (offset != envelope.length) throw new GeneralSecurityException("Incomplete local credential.");
                ByteBuffer buffer = ByteBuffer.wrap(envelope);
                if (buffer.getInt() != MAGIC || buffer.get() != 12) throw new GeneralSecurityException("Invalid local credential.");
                byte[] iv = new byte[12];
                buffer.get(iv);
                encrypted = new byte[buffer.remaining()];
                buffer.get(encrypted);
                Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
                cipher.init(Cipher.DECRYPT_MODE, key(false), new GCMParameterSpec(128, iv));
                cipher.updateAAD(AAD);
                token = cipher.doFinal(encrypted);
                if (!valid(token)) throw new GeneralSecurityException("Invalid local credential.");
                byte[] result = token;
                token = null;
                return result;
            } finally {
                if (envelope != null) Arrays.fill(envelope, (byte)0);
                if (encrypted != null) Arrays.fill(encrypted, (byte)0);
                if (token != null) Arrays.fill(token, (byte)0);
            }
        }
        // Missing/corrupt keys or ciphertext above are failures, never a silent credential reset.
        byte[] random = new byte[32], token = null, encrypted = null, envelope = null;
        FileOutputStream output = null;
        boolean committed = false;
        try {
            new SecureRandom().nextBytes(random);
            token = Base64.encode(random, Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING);
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.ENCRYPT_MODE, key(true));
            cipher.updateAAD(AAD);
            encrypted = cipher.doFinal(token);
            byte[] iv = cipher.getIV();
            if (iv.length != 12) throw new GeneralSecurityException("Invalid local credential nonce.");
            envelope = ByteBuffer.allocate(5 + iv.length + encrypted.length)
                .putInt(MAGIC).put((byte)iv.length).put(iv).put(encrypted).array();
            output = file.startWrite();
            output.write(envelope);
            file.finishWrite(output);
            output = null;
            Os.chmod(base.getPath(), 0600);
            committed = true;
            return token;
        } finally {
            if (output != null) file.failWrite(output);
            Arrays.fill(random, (byte)0);
            if (!committed && token != null) Arrays.fill(token, (byte)0);
            if (encrypted != null) Arrays.fill(encrypted, (byte)0);
            if (envelope != null) Arrays.fill(envelope, (byte)0);
        }
    }

    private static boolean valid(byte[] token) {
        if (token.length != 43) return false;
        for (byte value : token) {
            if (!((value >= 'a' && value <= 'z') || (value >= 'A' && value <= 'Z') ||
                (value >= '0' && value <= '9') || value == '-' || value == '_')) return false;
        }
        return true;
    }
}
