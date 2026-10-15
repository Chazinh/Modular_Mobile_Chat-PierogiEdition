package io.github.modularchat.pierogiedition;

import android.content.Context;
import android.os.SystemClock;
import java.io.File;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;

/** One serial executor and one native runtime for the app process; no UI-thread JNI/IO. */
final class NativeHost {
    static final String BASE_URL = "http://127.0.0.1:3011";
    private static final long TIMEOUT_MS = 50_000;
    private static final NativeHost INSTANCE = new NativeHost();
    private final ExecutorService executor = Executors.newSingleThreadExecutor(r -> new Thread(r, "mc-host-control"));
    private CompletableFuture<Void> starting, stopping;
    private volatile boolean stopRequested;
    private boolean loaded;

    static NativeHost get() { return INSTANCE; }
    private NativeHost() {}
    private static native int nativeStart(byte[] dataDir, byte[] nativeLibDir, byte[] token);
    private static native int nativeStop();
    private static native int nativeStatus();
    private static native int nativeLastError();

    synchronized CompletableFuture<Void> start(Context source) {
        Context context = source.getApplicationContext();
        if (stopping != null && !stopping.isDone()) return stopping.thenCompose(ignored -> start(context));
        if (starting != null && !starting.isDone()) return starting;
        stopRequested = false;
        starting = CompletableFuture.runAsync(() -> {
            byte[] data = null, nativeDir = null, token = null;
            try {
                if (!loaded) { System.loadLibrary("modularchat_server"); loaded = true; }
                if (stopRequested) throw new IllegalStateException("Local host stopped.");
                if (nativeStatus() == 2) return;
                File sandbox = new File(context.getNoBackupFilesDir(), "autohost");
                if (!sandbox.isDirectory() && !sandbox.mkdirs()) throw new IllegalStateException("Local storage unavailable.");
                data = sandbox.getAbsolutePath().getBytes(StandardCharsets.UTF_8);
                nativeDir = context.getApplicationInfo().nativeLibraryDir.getBytes(StandardCharsets.UTF_8);
                token = NativeHostToken.get(context);
                if (stopRequested) throw new IllegalStateException("Local host stopped.");
                int result = nativeStart(data, nativeDir, token);
                Arrays.fill(token, (byte)0); token = null;
                Arrays.fill(nativeDir, (byte)0); nativeDir = null;
                Arrays.fill(data, (byte)0); data = null;
                if (result < 0) throw new IllegalStateException("Local host start failed (" + result + ", " + nativeLastError() + ").");
                awaitState(true);
            } catch (Exception error) { throw new CompletionException(error); }
            finally {
                if (token != null) Arrays.fill(token, (byte)0);
                if (nativeDir != null) Arrays.fill(nativeDir, (byte)0);
                if (data != null) Arrays.fill(data, (byte)0);
            }
        }, executor);
        return starting;
    }

    synchronized CompletableFuture<Void> stop() {
        stopRequested = true;
        if (stopping != null && !stopping.isDone()) return stopping;
        starting = null;
        stopping = CompletableFuture.runAsync(() -> {
            if (!loaded) return;
            int result = nativeStop();
            if (result < 0) throw new IllegalStateException("Local host stop failed (" + result + ").");
            awaitState(false);
        }, executor);
        return stopping;
    }

    CompletableFuture<int[]> status() {
        return CompletableFuture.supplyAsync(() -> loaded ? new int[]{nativeStatus(), nativeLastError()} : new int[]{0, 0}, executor);
    }

    CompletableFuture<byte[]> token(Context source) {
        Context context = source.getApplicationContext();
        return CompletableFuture.supplyAsync(() -> {
            if (!loaded || nativeStatus() != 2) throw new IllegalStateException("Local host unavailable.");
            try { return NativeHostToken.get(context); }
            catch (Exception error) { throw new CompletionException(error); }
        }, executor);
    }

    private void awaitState(boolean ready) {
        long deadline = SystemClock.elapsedRealtime() + TIMEOUT_MS;
        while (SystemClock.elapsedRealtime() < deadline) {
            int status = nativeStatus();
            if (ready && stopRequested) {
                nativeStop();
                throw new IllegalStateException("Local host stopped.");
            }
            if (ready ? status == 2 : (status == 0 || status == 4)) return;
            if (status == 4 || (ready && status != 1))
                throw new IllegalStateException("Local host failed (" + nativeLastError() + ").");
            SystemClock.sleep(100);
        }
        if (ready) nativeStop();
        throw new IllegalStateException("Local host timed out.");
    }
}
