package io.github.modularchat.pierogiedition;

import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;
import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.Collections;
import java.util.IdentityHashMap;
import java.util.Set;

@CapacitorPlugin(name = "LocalHost")
public class LocalHostPlugin extends Plugin {
    private volatile boolean destroyed;
    private final Set<byte[]> pendingTokens = Collections.newSetFromMap(new IdentityHashMap<>());

    private MainActivity activity(PluginCall call) {
        if (destroyed || !(getActivity() instanceof MainActivity) || getActivity().isFinishing() || getActivity().isDestroyed()) {
            call.reject("O host local não está disponível nesta tela.", "LOCAL_HOST_UNAVAILABLE");
            return null;
        }
        return (MainActivity)getActivity();
    }

    @PluginMethod public void ensureStarted(PluginCall call) {
        MainActivity activity = activity(call);
        if (activity == null) return;
        android.util.Log.i("LocalHostPlugin", "ensureStarted called, waiting for unlock");
        activity.whenUnlocked(() -> {
            if (activity(call) == null) return;
            android.util.Log.i("LocalHostPlugin", "unlocked, calling localHostReady");
            activity.localHostReady().thenCompose(ignored -> NativeHost.get().token(activity))
                .whenComplete((token, error) -> activity.runOnUiThread(() -> {
                    if (error != null) {
                        android.util.Log.e("LocalHostPlugin", "Failed to start local host", error);
                        String detail = error.getCause() != null ? error.getCause().getMessage() : error.getMessage();
                        call.reject(detail != null && !detail.isEmpty() ? detail : "Não foi possível iniciar o host local.", "LOCAL_HOST_START_FAILED");
                        return;
                    }
                    if (activity(call) == null) { Arrays.fill(token, (byte)0); return; }
                    synchronized (pendingTokens) { pendingTokens.add(token); }
                    // A pause/relock while native startup/Keystore IO ran must gate the credential again.
                    activity.whenUnlocked(() -> {
                        try {
                            if (activity(call) == null) return;
                            android.util.Log.i("LocalHostPlugin", "local host ready, resolving plugin call");
                            JSObject result = snapshot(2, 0);
                            result.put("baseUrl", NativeHost.BASE_URL);
                            result.put("token", new String(token, StandardCharsets.UTF_8));
                            call.resolve(result);
                        } finally {
                            Arrays.fill(token, (byte)0);
                            synchronized (pendingTokens) { pendingTokens.remove(token); }
                        }
                    });
                }));
        });
    }

    @PluginMethod public void status(PluginCall call) {
        NativeHost.get().status().whenComplete((status, error) -> {
            if (error != null) call.reject("Não foi possível consultar o host local.", "LOCAL_HOST_STATUS_FAILED");
            else call.resolve(snapshot(status[0], status[1]));
        });
    }

    @PluginMethod public void stop(PluginCall call) {
        MainActivity activity = activity(call);
        if (activity == null) return;
        activity.whenUnlocked(() -> {
            if (activity(call) == null) return;
            activity.stopLocalHostService();
            NativeHost.get().stop().thenCompose(ignored -> NativeHost.get().status()).whenComplete((status, error) -> {
                if (error != null) call.reject("Não foi possível encerrar o host local.", "LOCAL_HOST_STOP_FAILED");
                else call.resolve(snapshot(status[0], status[1]));
            });
        });
    }

    private static JSObject snapshot(int status, int error) {
        String[] states = {"stopped", "initializing", "ready", "stopping", "failed"};
        JSObject result = new JSObject();
        result.put("status", status);
        result.put("state", status >= 0 && status < states.length ? states[status] : "failed");
        result.put("error", error);
        return result;
    }

    @Override protected void handleOnDestroy() {
        destroyed = true;
        synchronized (pendingTokens) {
            for (byte[] token : pendingTokens) Arrays.fill(token, (byte)0);
            pendingTokens.clear();
        }
        super.handleOnDestroy();
    }
}
