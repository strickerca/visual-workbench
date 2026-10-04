package com.visualworkbench.android.capture

import android.Manifest
import android.annotation.SuppressLint
import android.app.Activity
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.provider.Settings
import android.service.quicksettings.TileService
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView

/** Exported shortcut target always requires a visible owner confirmation. An
 * arbitrary external Intent cannot invoke the private service's request path. */
class CaptureEntryActivity:Activity() {
    override fun onCreate(state:Bundle?){super.onCreate(state)
        val root=LinearLayout(this).apply{orientation=LinearLayout.VERTICAL;setPadding(24,24,24,24)}
        root.addView(TextView(this).apply{text="Screen capture is optional. Enable Visual Workbench in Accessibility settings. Only a capture action reads the screenshot and current screen's accessibility information; ordinary events are ignored. Password text is excluded. Secure windows may be black. Close this window before capturing from the tile or notification."})
        root.addView(Button(this).apply{text="Open Accessibility settings";setOnClickListener{startActivity(Intent(Settings.ACTION_ACCESSIBILITY_SETTINGS))}})
        root.addView(Button(this).apply{text="Retained capture files";setOnClickListener{startActivity(Intent(this@CaptureEntryActivity,CaptureInboxActivity::class.java))}})
        root.addView(Button(this).apply{text="Add capture shortcut";setOnClickListener{
            val manager=getSystemService(android.content.pm.ShortcutManager::class.java)
            if(manager.isRequestPinShortcutSupported){
                val shortcut=android.content.pm.ShortcutInfo.Builder(this@CaptureEntryActivity,"capture")
                    .setShortLabel("Workbench capture").setIcon(android.graphics.drawable.Icon.createWithResource(this@CaptureEntryActivity,android.R.drawable.ic_menu_camera))
                    .setIntent(Intent(this@CaptureEntryActivity,CaptureEntryActivity::class.java).setAction("com.visualworkbench.action.CAPTURE")).build()
                manager.requestPinShortcut(shortcut,null)
            }else android.widget.Toast.makeText(this@CaptureEntryActivity,"This launcher does not support pinned shortcuts.",android.widget.Toast.LENGTH_LONG).show()
        }})
        root.addView(Button(this).apply{text="Capture after closing this window";setOnClickListener{val service=CaptureService.instance.get();if(service==null)android.widget.Toast.makeText(this@CaptureEntryActivity,"Enable the service and open Visual Workbench first.",android.widget.Toast.LENGTH_LONG).show() else {finish();service.request()}}})
        if(Build.VERSION.SDK_INT>=33&&checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)
            root.addView(Button(this).apply{text="Allow capture notification";setOnClickListener{requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS),41)}})
        root.addView(Button(this).apply{text="Close";setOnClickListener{finish()}})
        setContentView(root)
    }
}
class CaptureTileService:TileService() {
    override fun onClick(){super.onClick();unlockAndRun{
        val service=CaptureService.instance.get()
        if(service==null){val intent=Intent(this,CaptureEntryActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
            if(Build.VERSION.SDK_INT>=34)startActivityAndCollapse(PendingIntent.getActivity(this,0,intent,PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT))
            else launchLegacyConfirmation(intent)
        }else service.request()
    }}
    // The PendingIntent overload exists only from API34. This exact legacy
    // launch is confined to older devices and still opens owner confirmation.
    @SuppressLint("StartActivityAndCollapseDeprecated")
    @Suppress("DEPRECATION")
    private fun launchLegacyConfirmation(intent:Intent){
        check(Build.VERSION.SDK_INT<34)
        startActivityAndCollapse(intent)
    }
}
internal object CaptureEntrypoints {
    fun notification(context:Context){
        if(Build.VERSION.SDK_INT>=33&&context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS)!=PackageManager.PERMISSION_GRANTED)return
        val manager=context.getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel("capture-actions","Capture actions",NotificationManager.IMPORTANCE_LOW))
        val action=PendingIntent.getActivity(context,17,Intent(context,CaptureEntryActivity::class.java),PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        manager.notify(0x5657,Notification.Builder(context,"capture-actions").setSmallIcon(android.R.drawable.ic_menu_camera)
            .setContentTitle("Visual Workbench capture").setContentText("Choose when to capture the current screen.")
            .setContentIntent(action).setOngoing(true).build())
    }
}
