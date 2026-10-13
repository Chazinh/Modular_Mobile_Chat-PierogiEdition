package io.github.modularchat.pierogiedition;

import android.content.ComponentName;
import android.content.Context;
import android.content.SharedPreferences;
import android.content.pm.PackageManager;
import com.getcapacitor.JSObject;
import com.getcapacitor.Plugin;
import com.getcapacitor.PluginCall;
import com.getcapacitor.PluginMethod;
import com.getcapacitor.annotation.CapacitorPlugin;

// Changes the home-screen icon by enabling one activity-alias (AndroidManifest.xml) and disabling the others.
// The change is stored and applied in MainActivity.onStop: touching the aliases while the app is open makes some
// launchers kill the app. Ids match client/src/app-icon.js.
@CapacitorPlugin(name = "AppIcon")
public class AppIconPlugin extends Plugin {
    static final String[][] ICONS = {
        {"pierogi_dark", ".IconPierogiDark"},
        {"pierogi_light", ".IconPierogiLight"},
    };
    private static final String PREFS = "app_icon";
    private static final String PENDING = "pending";

    private static String aliasFor(String id) {
        for (String[] icon : ICONS) if (icon[0].equals(id)) return icon[1];
        return null;
    }

    // Aliases resolve against the code namespace, which can differ from a private build's applicationId.
    private static ComponentName component(Context context, String alias) {
        return new ComponentName(context.getPackageName(), AppIconPlugin.class.getPackage().getName() + alias);
    }

    /** Icon currently enabled in the launcher (the first enabled alias; with none, the manifest default). */
    static String current(Context context) {
        PackageManager pm = context.getPackageManager();
        for (String[] icon : ICONS) {
            int state = pm.getComponentEnabledSetting(component(context, icon[1]));
            boolean enabled = state == PackageManager.COMPONENT_ENABLED_STATE_ENABLED
                || state == PackageManager.COMPONENT_ENABLED_STATE_DEFAULT && icon[0].equals(ICONS[0][0]);
            if (enabled) return icon[0];
        }
        return ICONS[0][0];
    }

    /** Applies the stored change (called when the app goes to the background). Never leaves the app without an icon. */
    static void applyPending(Context context) {
        SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        String id = prefs.getString(PENDING, null);
        if (id == null) return;
        prefs.edit().remove(PENDING).apply();
        if (aliasFor(id) == null || id.equals(current(context))) return;
        PackageManager pm = context.getPackageManager();
        // enable the new one before disabling the others
        pm.setComponentEnabledSetting(component(context, aliasFor(id)), PackageManager.COMPONENT_ENABLED_STATE_ENABLED, PackageManager.DONT_KILL_APP);
        for (String[] icon : ICONS) {
            if (icon[0].equals(id)) continue;
            pm.setComponentEnabledSetting(component(context, icon[1]), PackageManager.COMPONENT_ENABLED_STATE_DISABLED, PackageManager.DONT_KILL_APP);
        }
    }

    @PluginMethod public void get(PluginCall call) {
        Context context = getContext();
        String pending = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).getString(PENDING, null);
        JSObject out = new JSObject();
        out.put("id", pending != null ? pending : current(context));
        out.put("pending", pending != null);
        call.resolve(out);
    }

    @PluginMethod public void set(PluginCall call) {
        String id = call.getString("id");
        if (aliasFor(id) == null) {
            call.reject("Ícone desconhecido: " + id);
            return;
        }
        Context context = getContext();
        SharedPreferences prefs = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE);
        boolean pending = !id.equals(current(context));
        if (pending) prefs.edit().putString(PENDING, id).apply();
        else prefs.edit().remove(PENDING).apply();
        JSObject out = new JSObject();
        out.put("id", id);
        out.put("pending", pending);
        call.resolve(out);
    }
}
