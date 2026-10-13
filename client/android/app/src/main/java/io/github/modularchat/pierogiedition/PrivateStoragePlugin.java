package io.github.modularchat.pierogiedition;

import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

@CapacitorPlugin(name = "PrivateStorage")
public class PrivateStoragePlugin extends Plugin {
    private static final String ALIAS = "ModularChat.PrivateCache.v1";
    @PluginMethod public void ready(PluginCall call) {
        ((MainActivity) getActivity()).whenUnlocked(() -> call.resolve());
    }
    private SecretKey key() throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore"); store.load(null);
        if (store.containsAlias(ALIAS)) return (SecretKey) store.getKey(ALIAS, null);
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
            .setKeySize(256).setBlockModes(KeyProperties.BLOCK_MODE_GCM)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE).setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }
    @PluginMethod public synchronized void encrypt(PluginCall call) {
        try {
            String text = call.getString("text"); if (text == null) throw new Exception("Texto ausente");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding"); cipher.init(Cipher.ENCRYPT_MODE, key());
            cipher.updateAAD("ModularChat.PrivateCache.v1".getBytes(StandardCharsets.UTF_8));
            byte[] encrypted = cipher.doFinal(text.getBytes(StandardCharsets.UTF_8));
            JSObject result = new JSObject();
            result.put("nonce", Base64.encodeToString(cipher.getIV(), Base64.NO_WRAP));
            result.put("data", Base64.encodeToString(encrypted, Base64.NO_WRAP));call.resolve(result);
        } catch (Exception error) { call.reject("Não foi possível proteger o cache do aparelho.", error); }
    }
    @PluginMethod public synchronized void decrypt(PluginCall call) {
        try {
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, key(), new GCMParameterSpec(128, Base64.decode(call.getString("nonce"), Base64.NO_WRAP)));
            cipher.updateAAD("ModularChat.PrivateCache.v1".getBytes(StandardCharsets.UTF_8));
            String text = new String(cipher.doFinal(Base64.decode(call.getString("data"), Base64.NO_WRAP)), StandardCharsets.UTF_8);
            JSObject result = new JSObject(); result.put("text", text);call.resolve(result);
        } catch (Exception error) { call.reject("O cache criptografado não pôde ser aberto. Preserve o histórico no PC.", error); }
    }
}
