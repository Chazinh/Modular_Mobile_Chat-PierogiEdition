package io.github.modularchat.pierogiedition;

import android.content.Context;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;

@CapacitorPlugin(name = "AppLanguage")
public class LanguagePlugin extends Plugin {
    private static final String PREFS = "modularchat_language";

    static String text(Context context, String portuguese, String english) {
        return "en".equals(context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
            .getString("language", "pt-BR")) ? english : portuguese;
    }

    @PluginMethod public void set(PluginCall call) {
        String language = call.getString("language", "pt-BR");
        if (!"pt-BR".equals(language) && !"en".equals(language)) {
            call.reject("Unsupported language");
            return;
        }
        getContext().getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putString("language", language).apply();
        JSObject result = new JSObject();
        result.put("language", language);
        call.resolve(result);
    }
}
