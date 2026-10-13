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
    private BiometricPrompt prompt;
    private final ArrayList<Runnable> unlockCallbacks = new ArrayList<>();
    public void whenUnlocked(Runnable callback) { runOnUiThread(() -> { if (unlocked) callback.run(); else unlockCallbacks.add(callback); }); }

    @Override public void onCreate(Bundle state) {
        registerPlugin(PrivateStoragePlugin.class);
        registerPlugin(AppIconPlugin.class);
        registerPlugin(LanguagePlugin.class);
        super.onCreate(state);
        perf = getResources().getBoolean(R.bool.mc_perf);
        if (perf) {
            unlocked = true;
            return;
        }
        getWindow().addFlags(WindowManager.LayoutParams.FLAG_SECURE);
        privacyCover = new View(this);
        privacyCover.setBackgroundColor(0xff1b1b1d);
        privacyCover.setClickable(true);
        addContentView(privacyCover, new FrameLayout.LayoutParams(-1, -1));
        prompt = new BiometricPrompt(this, ContextCompat.getMainExecutor(this), new BiometricPrompt.AuthenticationCallback() {
            @Override public void onAuthenticationSucceeded(BiometricPrompt.AuthenticationResult result) {
                super.onAuthenticationSucceeded(result);
                prompting = false;
                unlocked = true;
                privacyCover.setVisibility(View.GONE);
                for (Runnable callback : new ArrayList<>(unlockCallbacks)) callback.run();
                unlockCallbacks.clear();
            }
            @Override public void onAuthenticationError(int code, CharSequence message) {
                super.onAuthenticationError(code, message);
                prompting = false;
                if (!isFinishing() && getLifecycle().getCurrentState().isAtLeast(androidx.lifecycle.Lifecycle.State.STARTED)) finish();
            }
        });
    }
    @Override public void onResume() {
        super.onResume();
        if (privacyCover != null && !unlocked && !prompting) authenticate();
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
