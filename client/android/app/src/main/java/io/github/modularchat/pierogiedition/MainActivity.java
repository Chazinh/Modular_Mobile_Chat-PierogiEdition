package io.github.modularchat.pierogiedition;

import com.getcapacitor.BridgeActivity;
import android.os.Bundle;
import android.os.Build;
import android.content.Intent;
import android.content.ComponentName;
import android.content.Context;
import android.content.ServiceConnection;
import android.os.IBinder;
import android.provider.Settings;
import android.view.View;
import android.view.Window;
import android.view.WindowManager;
import android.widget.FrameLayout;
import android.widget.TextView;
import android.view.Gravity;
import androidx.appcompat.app.AlertDialog;
import androidx.biometric.BiometricManager;
import androidx.biometric.BiometricPrompt;
import androidx.core.content.ContextCompat;
import java.util.ArrayList;
import java.util.concurrent.CompletableFuture;

public class MainActivity extends BridgeActivity {
    private View privacyCover;
    /** Measurement copy (build type "perf"): no capture block and no biometric gate. Always false in release. */
    private boolean perf;
    private boolean unlocked = false;
    private boolean prompting = false;
    private boolean resumed = false;
    private boolean transientRetried = false;
    private BiometricPrompt prompt;
    private final ArrayList<Runnable> unlockCallbacks = new ArrayList<>();
    private boolean hostBound;
    private final android.os.Handler hostTimeout = new android.os.Handler(android.os.Looper.getMainLooper());
    private CompletableFuture<Void> hostReady = new CompletableFuture<>();
    private final ServiceConnection hostConnection = new ServiceConnection() {
        @Override public void onServiceConnected(ComponentName name, IBinder service) {
            CompletableFuture<Void> target = hostReady;
            ((LocalHostService.HostBinder)service).ready().whenComplete((ignored, error) -> {
                if (error == null) target.complete(null); else target.completeExceptionally(error);
            });
        }
        @Override public void onServiceDisconnected(ComponentName name) {
            hostBound = false;
            hostReady = new CompletableFuture<>();
        }
        @Override public void onBindingDied(ComponentName name) {
            stopHostBinding();
            hostReady.completeExceptionally(new IllegalStateException("Local service disconnected."));
        }
        @Override public void onNullBinding(ComponentName name) {
            stopHostBinding();
            hostReady.completeExceptionally(new IllegalStateException("Local service unavailable."));
        }
    };
    public void whenUnlocked(Runnable callback) { runOnUiThread(() -> { if (unlocked) callback.run(); else unlockCallbacks.add(callback); }); }

    CompletableFuture<Void> localHostReady() {
        startLocalHostService();
        return hostReady;
    }
    private void startLocalHostService() {
        if (!resumed || isFinishing() || isDestroyed()) return;
        if (hostBound) {
            if (hostReady.isCompletedExceptionally()) {
                stopHostBinding();
                hostReady = new CompletableFuture<>();
            } else {
                return;
            }
        } else if (hostReady.isDone() && hostReady.isCompletedExceptionally()) {
            hostReady = new CompletableFuture<>();
        }
        CompletableFuture<Void> waiting = hostReady;
        Runnable timeout = () -> waiting.completeExceptionally(new IllegalStateException("Local service readiness timed out."));
        hostTimeout.postDelayed(timeout, 50_000);
        waiting.whenComplete((ignored, error) -> hostTimeout.removeCallbacks(timeout));
        Intent intent = new Intent(this, LocalHostService.class);
        try {
            if (Build.VERSION.SDK_INT >= 26) startForegroundService(intent); else startService(intent);
            hostBound = bindService(intent, hostConnection, Context.BIND_AUTO_CREATE);
            if (!hostBound) hostReady.completeExceptionally(new IllegalStateException("Local service binding failed."));
        } catch (RuntimeException error) {
            hostReady.completeExceptionally(new IllegalStateException("Local service start unavailable."));
        }
    }
    private void stopHostBinding() {
        if (!hostBound) return;
        hostBound = false;
        try { unbindService(hostConnection); } catch (IllegalArgumentException ignored) {}
    }
    void stopLocalHostService() {
        stopHostBinding();
        stopService(new Intent(this, LocalHostService.class));
        hostReady.completeExceptionally(new IllegalStateException("Local service stopped."));
        hostReady = new CompletableFuture<>();
    }

    @Override public void onCreate(Bundle state) {
        registerPlugin(PrivateStoragePlugin.class);
        registerPlugin(AppIconPlugin.class);
        registerPlugin(LanguagePlugin.class);
        registerPlugin(LocalHostPlugin.class);
        // Only the separate measurement package can bypass the privacy gate.
        perf = getResources().getBoolean(R.bool.mc_perf) && getPackageName().endsWith(".perf");
        if (!perf) getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        super.onCreate(state);
        try {
            android.webkit.WebView.setWebContentsDebuggingEnabled(
                (getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0);
            android.webkit.WebView webView = getBridge() != null ? getBridge().getWebView() : null;
            if (webView != null) {
                android.webkit.WebSettings settings = webView.getSettings();
                settings.setCacheMode(android.webkit.WebSettings.LOAD_DEFAULT);
                settings.setDomStorageEnabled(true);
                settings.setDatabaseEnabled(true);
            }
        } catch (Exception ignored) {}
        if (perf) {
            unlocked = true;
            return;
        }
        TextView unlockView = new TextView(this);
        unlockView.setText(LanguagePlugin.text(this, "Toque para desbloquear", "Tap to unlock"));
        unlockView.setTextColor(0xffe7e7ea);
        unlockView.setGravity(Gravity.CENTER);
        privacyCover = unlockView;
        privacyCover.setBackgroundColor(0xff1b1b1d);
        privacyCover.setClickable(true);
        privacyCover.setOnClickListener(v -> { if (resumed && !prompting && !unlocked) authenticate(); });
        addContentView(privacyCover, new FrameLayout.LayoutParams(-1, -1));
        prompt = new BiometricPrompt(this, ContextCompat.getMainExecutor(this), new BiometricPrompt.AuthenticationCallback() {
            @Override public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                super.onAuthenticationSucceeded(result);
                prompting = false;
                if (!resumed) return;
                unlocked = true;
                privacyCover.setVisibility(View.GONE);
                for (Runnable callback : new ArrayList<>(unlockCallbacks)) callback.run();
                unlockCallbacks.clear();
            }
            @Override public void onAuthenticationError(int code, CharSequence message) {
                super.onAuthenticationError(code, message);
                prompting = false;
                // A system cancellation during Recents is not the user choosing Exit.
                if (code == BiometricPrompt.ERROR_CANCELED) {
                    if (resumed && !transientRetried) {
                        transientRetried = true;
                        privacyCover.post(() -> { if (resumed && hasWindowFocus() && !unlocked && !prompting && !isFinishing()) authenticate(); });
                    }
                    return;
                }
                if (resumed && (code == BiometricPrompt.ERROR_NEGATIVE_BUTTON || code == BiometricPrompt.ERROR_USER_CANCELED)) finish();
            }
        });
    }
    @Override public void onResume() {
        super.onResume();
        resumed = true;
        transientRetried = false;
        startLocalHostService();
        // Wait for the window to return from Recents before opening the system prompt.
        if (hasWindowFocus() && privacyCover != null && !unlocked && !prompting) authenticate();
    }
    @Override public void onWindowFocusChanged(boolean hasFocus) {
        super.onWindowFocusChanged(hasFocus);
        if (hasFocus && resumed && privacyCover != null && !unlocked && !prompting) authenticate();
    }
    @Override public void onPause() {
        resumed = false;
        super.onPause();
    }
    @Override public void onStop() {
        // icon change chosen in the app: applied only while the app is off screen (see AppIconPlugin)
        if (!isChangingConfigurations()) AppIconPlugin.applyPending(this);
        if (perf) { super.onStop(); return; }
        unlocked = false;
        if (privacyCover != null) privacyCover.setVisibility(View.VISIBLE);
        super.onStop();
    }
    @Override public void onDestroy() {
        stopHostBinding();
        hostReady.completeExceptionally(new IllegalStateException("Local host screen closed."));
        super.onDestroy();
    }
    private void authenticate() {
        prompting = true;
        int result = BiometricManager.from(this).canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG);
        if (result != BiometricManager.BIOMETRIC_SUCCESS) {
            new AlertDialog.Builder(this).setTitle(LanguagePlugin.text(this, "Biometria necessária", "Biometrics required"))
                .setMessage(LanguagePlugin.text(this, "Cadastre uma biometria forte nas configurações do aparelho para acessar suas conversas.", "Set up strong biometrics in your device settings to access your conversations."))
                .setPositiveButton(LanguagePlugin.text(this, "Configurações", "Settings"), (d, w) -> {
                    prompting = false;
                    Intent intent = new Intent(Build.VERSION.SDK_INT >= 30 ? Settings.ACTION_BIOMETRIC_ENROLL : Settings.ACTION_SECURITY_SETTINGS);
                    if (Build.VERSION.SDK_INT >= 30) intent.putExtra(Settings.EXTRA_BIOMETRIC_AUTHENTICATORS_ALLOWED, BiometricManager.Authenticators.BIOMETRIC_STRONG);
                    startActivity(intent);
                }).setNegativeButton(LanguagePlugin.text(this, "Sair", "Exit"), (d, w) -> finish()).setCancelable(false).show();
            return;
        }
        prompt.authenticate(new BiometricPrompt.PromptInfo.Builder().setTitle(getString(R.string.app_name))
            .setConfirmationRequired(false)
            .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
            .setNegativeButtonText(LanguagePlugin.text(this, "Sair", "Exit")).build());
    }
}
