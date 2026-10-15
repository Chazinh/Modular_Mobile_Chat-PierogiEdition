package io.github.modularchat.pierogiedition;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.os.Binder;
import android.os.Build;
import android.os.IBinder;
import java.util.concurrent.CompletableFuture;

/** Started only by a resumed Activity. Recents/backgrounding do not stop this service. */
public class LocalHostService extends Service {
    private static final String CHANNEL = "mc_local_host";
    private static final int NOTIFICATION_ID = 3011;
    private CompletableFuture<Void> ready;
    private final HostBinder binder = new HostBinder();

    public final class HostBinder extends Binder {
        public CompletableFuture<Void> ready() { return ready; }
    }

    @Override public void onCreate() {
        super.onCreate();
        NotificationManager manager = getSystemService(NotificationManager.class);
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel channel = new NotificationChannel(CHANNEL,
                LanguagePlugin.text(this, "Host local", "Local host"), NotificationManager.IMPORTANCE_LOW);
            channel.setLockscreenVisibility(Notification.VISIBILITY_SECRET);
            channel.setShowBadge(false);
            manager.createNotificationChannel(channel);
        }
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class),
            PendingIntent.FLAG_UPDATE_CURRENT | PendingIntent.FLAG_IMMUTABLE);
        Notification.Builder notification = Build.VERSION.SDK_INT >= 26 ? new Notification.Builder(this, CHANNEL) : new Notification.Builder(this);
        notification.setSmallIcon(android.R.drawable.stat_notify_sync_noanim)
            .setContentTitle(LanguagePlugin.text(this, "Host local", "Local host"))
            .setContentText(LanguagePlugin.text(this, "Serviço disponível no aparelho.", "Service available on this device."))
            .setVisibility(Notification.VISIBILITY_SECRET).setOngoing(true).setContentIntent(open);
        if (Build.VERSION.SDK_INT >= 34) startForeground(NOTIFICATION_ID, notification.build(), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE);
        else startForeground(NOTIFICATION_ID, notification.build());
        ready = NativeHost.get().start(this);
    }

    @Override public int onStartCommand(Intent intent, int flags, int startId) { return START_NOT_STICKY; }
    @Override public IBinder onBind(Intent intent) { return binder; }
    @Override public void onTaskRemoved(Intent rootIntent) {
        NativeHost.get().stop();
        stopSelf();
        super.onTaskRemoved(rootIntent);
    }
    @Override public void onDestroy() {
        NativeHost.get().stop();
        stopForeground(true);
        super.onDestroy();
    }
}
