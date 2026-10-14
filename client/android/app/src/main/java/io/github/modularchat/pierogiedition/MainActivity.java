package io.github.modularchat.pierogiedition;

import com.getcapacitor.BridgeActivity;
import android.os.Bundle;
import android.os.Build;
import android.content.Intent;
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
    public void whenUnlocked(Runnable callback) { runOnUiThread(() -> { if (unlocked) callback.run(); else unlockCallbacks.add(callback); }); }

    @Override public void onCreate(Bundle state) {
        registerPlugin(PrivateStoragePlugin.class);
        registerPlugin(AppIconPlugin.class);
        registerPlugin(LanguagePlugin.class);
        super.onCreate(state);
        try {
            android.webkit.WebView webView = getBridge() != null ? getBridge().getWebView() : null;
            if (webView != null) {
                android.webkit.WebSettings settings = webView.getSettings();
                settings.setCacheMode(android.webkit.WebSettings.LOAD_DEFAULT);
                settings.setDomStorageEnabled(true);
                settings.setDatabaseEnabled(true);
            }
        } catch (Exception ignored) {}
        perf = getResources().getBoolean(R.bool.mc_perf);
        if (perf) {
            unlocked = true;
            return;
        }
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
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
