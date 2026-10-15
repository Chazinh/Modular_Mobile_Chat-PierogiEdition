package io.github.modularchat.pierogiedition;

import android.content.Context;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.junit.Test;
import org.junit.runner.RunWith;
import static org.junit.Assert.*;
import java.io.File;
import java.lang.reflect.Method;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.security.MessageDigest;
import java.util.Arrays;
import java.util.UUID;

@RunWith(AndroidJUnit4.class)
public class NativeVaultKeyTest {
    private static final byte[] TOKEN = "fixture_android_0123456789abcdef0123456789abcdef0123456789abcdef".getBytes(StandardCharsets.UTF_8);
    private static int call(String name, Class<?>[] types, Object... args) throws Exception {
        Method method = NativeHost.class.getDeclaredMethod(name, types);
        method.setAccessible(true);
        return (Integer)method.invoke(null, args);
    }
    private static int status() throws Exception { return call("nativeStatus", new Class<?>[0]); }
    private static org.json.JSONObject exportFixture(String json, File keyFile) throws Exception {
        org.json.JSONObject envelope = new org.json.JSONObject(json);
        assertEquals("modularchat-encrypted-export", envelope.getString("format"));
        byte[] wrapped = Files.readAllBytes(keyFile.toPath());
        byte[] key = NativeVaultKey.unprotect(Arrays.copyOfRange(wrapped, 8, wrapped.length));
        byte[] sealed = android.util.Base64.decode(envelope.getString("data"), android.util.Base64.DEFAULT);
        // MCVAULT1 stores nonce|tag|ciphertext; Java Cipher expects ciphertext|tag.
        byte[] encrypted = new byte[sealed.length - 20], clear = null;
        try {
            System.arraycopy(sealed, 36, encrypted, 0, sealed.length - 36);
            System.arraycopy(sealed, 20, encrypted, sealed.length - 36, 16);
            javax.crypto.Cipher cipher = javax.crypto.Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(javax.crypto.Cipher.DECRYPT_MODE, new javax.crypto.spec.SecretKeySpec(key, "AES"),
                new javax.crypto.spec.GCMParameterSpec(128, sealed, 8, 12));
            cipher.updateAAD("ModularChat.encrypted-storage.v1".getBytes(StandardCharsets.UTF_8));
            clear = cipher.doFinal(encrypted);
            return new org.json.JSONObject(new String(clear, StandardCharsets.UTF_8));
        } finally { Arrays.fill(key, (byte)0); Arrays.fill(encrypted, (byte)0); if (clear != null) Arrays.fill(clear, (byte)0); }
    }
    private static void removeFixture(File file, String root) throws Exception {
        assertTrue(file.getCanonicalPath().startsWith(root + File.separator) || file.getCanonicalPath().equals(root));
        if (file.isDirectory()) for (File child : file.listFiles()) removeFixture(child, root);
        assertTrue(file.delete());
    }
    private static void await(int expected) throws Exception {
        long deadline = android.os.SystemClock.elapsedRealtime() + 50000;
        while (android.os.SystemClock.elapsedRealtime() < deadline) {
            int actual = status();
            if (actual == expected) return;
            if (actual == 4 && expected != 4) fail("Native host failed: " + call("nativeLastError", new Class<?>[0]));
            Thread.sleep(30);
        }
        fail("Native state timeout: " + expected);
    }
    private static void start(Context context, File dir) throws Exception {
        assertEquals(0, call("nativeStart", new Class<?>[]{byte[].class, byte[].class, byte[].class},
            dir.getAbsolutePath().getBytes(StandardCharsets.UTF_8),
            context.getApplicationInfo().nativeLibraryDir.getBytes(StandardCharsets.UTF_8), TOKEN));
    }
    private static String request(String method, String path, String body) throws Exception {
        HttpURLConnection connection = (HttpURLConnection)new URL("http://127.0.0.1:3011" + path).openConnection();
        connection.setConnectTimeout(5000); connection.setReadTimeout(5000);
        connection.setRequestMethod(method);
        connection.setRequestProperty("Authorization", "Bearer " + new String(TOKEN, StandardCharsets.UTF_8));
        try {
            if (body != null) {
                connection.setRequestProperty("Content-Type", "application/json"); connection.setDoOutput(true);
                try (java.io.OutputStream output = connection.getOutputStream()) { output.write(body.getBytes(StandardCharsets.UTF_8)); }
            }
            assertTrue("HTTP " + connection.getResponseCode(), connection.getResponseCode() < 300);
            try (java.io.InputStream input = connection.getInputStream()) { return new String(input.readAllBytes(), StandardCharsets.UTF_8); }
        } finally { connection.disconnect(); }
    }

    @Test public void keystoreRoundTripAndTamperRejection() throws Exception {
        byte[] key = new byte[32]; new java.security.SecureRandom().nextBytes(key);
        byte[] input = key.clone(), envelope = NativeVaultKey.protect(input);
        assertArrayEquals(new byte[32], input);
        assertArrayEquals(key, NativeVaultKey.unprotect(envelope.clone()));
        byte[] other = NativeVaultKey.protect(key.clone());
        assertFalse(Arrays.equals(envelope, other));
        envelope[envelope.length - 1] ^= 1;
        try { NativeVaultKey.unprotect(envelope); fail("Tampered key accepted"); }
        catch (java.security.GeneralSecurityException expected) { /* authenticated failure */ }
        assertArrayEquals(new byte[60], envelope);
        Arrays.fill(key, (byte)0);
    }

    @Test public void directJniMigratesLegacyEnvelopeAndPreservesVault() throws Exception {
        Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
        assertTrue("Run this test only in the isolated perf package", context.getPackageName().endsWith(".perf"));
        System.loadLibrary("modularchat_server");
        assertEquals("A host is already running; do not touch it", 0, status());
        File dir = new File(context.getNoBackupFilesDir(), "mc-vault-fixture-ação-" + UUID.randomUUID());
        assertTrue(dir.mkdirs());
        File keyFile = new File(dir, "modularchat.db.sandbox.key");
        File vault = new File(dir, "modularchat.db.vault");
        try {
            assertEquals(-1, call("nativeStart", new Class<?>[]{byte[].class, byte[].class, byte[].class}, null, null, null));
            byte[] path = dir.getAbsolutePath().getBytes(StandardCharsets.UTF_8);
            assertEquals(-1, call("nativeStart", new Class<?>[]{byte[].class, byte[].class, byte[].class}, new byte[]{(byte)0xff}, path, TOKEN));
            assertEquals(-1, call("nativeStart", new Class<?>[]{byte[].class, byte[].class, byte[].class}, new byte[]{0}, path, TOKEN));
            start(context, dir); await(2);
            org.json.JSONObject created = new org.json.JSONObject(request("POST", "/api/chats", "{\"name\":\"JNI fixture\",\"prompt\":\"PRIVATE_KEYSTORE_FIXTURE\",\"persona\":\"Alex\"}"));
            String chatPath = "/api/chats/" + created.getString("id");
            // Import an opening through the public API so the fixture has real narrative history.
            org.json.JSONObject bundle = exportFixture(request("GET", chatPath + "/export", null), keyFile);
            org.json.JSONObject exported = bundle.getJSONArray("conversations").getJSONObject(0);
            String iid = UUID.randomUUID().toString(), pid = UUID.randomUUID().toString(), vid = UUID.randomUUID().toString();
            String time = "2026-10-15T00:00:00.000Z";
            exported.getJSONObject("conversation").put("head_interaction_id", iid);
            exported.put("interactions", new org.json.JSONArray().put(new org.json.JSONObject()
                .put("id", iid).put("conv_id", created.getString("id")).put("parent_id", org.json.JSONObject.NULL)
                .put("seq", 1).put("kind", "opening").put("user_text", org.json.JSONObject.NULL).put("created_at", time)));
            exported.put("parts", new org.json.JSONArray().put(new org.json.JSONObject()
                .put("id", pid).put("interaction_id", iid).put("idx", 0).put("selected_version_id", vid).put("created_at", time)));
            exported.put("versions", new org.json.JSONArray().put(new org.json.JSONObject()
                .put("id", vid).put("part_id", pid).put("content", "Lia: A chave continua guardada na torre. ✨")
                .put("status", "complete").put("origin", "legacy").put("created_at", time).put("finished_at", time)));
            org.json.JSONObject imported = new org.json.JSONObject(request("POST", "/api/import", bundle.toString()));
            chatPath = "/api/chats/" + imported.getJSONArray("imported").getString(0);
            org.json.JSONObject original = new org.json.JSONObject(request("GET", chatPath, null));
            assertEquals(1, original.getJSONArray("history").length());
            call("nativeStop", new Class<?>[0]); await(0);
            byte[] wrapped = Files.readAllBytes(keyFile.toPath());
            assertEquals("MCKEYKS1", new String(wrapped, 0, 8, StandardCharsets.US_ASCII));
            byte[] raw = NativeVaultKey.unprotect(Arrays.copyOfRange(wrapped, 8, wrapped.length));
            byte[] legacy = new byte[72];
            System.arraycopy("MCKEY001".getBytes(StandardCharsets.US_ASCII), 0, legacy, 0, 8);
            System.arraycopy(raw, 0, legacy, 8, 32);
            System.arraycopy(MessageDigest.getInstance("SHA-256").digest(Arrays.copyOf(legacy, 40)), 0, legacy, 40, 32);
            Files.write(keyFile.toPath(), legacy); Arrays.fill(legacy, (byte)0);
            byte[] before = Files.readAllBytes(vault.toPath());
            start(context, dir); await(2);
            // Startup can persist operational metadata with a fresh AES-GCM nonce.
            // The migrated data key and canonical conversation must stay identical.
            org.json.JSONObject reopened = new org.json.JSONObject(request("GET", chatPath, null));
            for (String field : new String[]{"id", "name", "prompt", "persona", "history"}) assertEquals(field, original.get(field).toString(), reopened.get(field).toString());
            assertFalse(new File(dir, "modularchat.db").exists());
            wrapped = Files.readAllBytes(keyFile.toPath());
            assertEquals("MCKEYKS1", new String(wrapped, 0, 8, StandardCharsets.US_ASCII));
            assertArrayEquals(raw, NativeVaultKey.unprotect(Arrays.copyOfRange(wrapped, 8, wrapped.length)));
            Arrays.fill(raw, (byte)0);
            call("nativeStop", new Class<?>[0]); await(0);
            wrapped[wrapped.length - 1] ^= 1; Files.write(keyFile.toPath(), wrapped);
            before = Files.readAllBytes(vault.toPath());
            start(context, dir); await(4);
            assertEquals(409, call("nativeLastError", new Class<?>[0]));
            assertArrayEquals(wrapped, Files.readAllBytes(keyFile.toPath()));
            assertArrayEquals(before, Files.readAllBytes(vault.toPath()));
        } finally {
            call("nativeStop", new Class<?>[0]);
            if (status() != 4) await(0);
            // This UUID directory belongs exclusively to the test; no canonical app data is opened.
            removeFixture(dir, dir.getCanonicalPath());
        }
    }
}
