package com.visualworkbench.android.capture

import android.app.Activity
import android.app.AlertDialog
import android.content.Intent
import android.os.Bundle
import android.widget.Button
import android.widget.LinearLayout
import android.widget.TextView
import android.widget.Toast
import com.visualworkbench.android.editor.RasterTransfers
import kotlinx.coroutines.*

/** Recovery files never expire automatically. Saving copies exact bytes using
 * the existing bounded provider writer; only explicit Discard removes a lease. */
class CaptureInboxActivity:Activity() {
    private val scope=CoroutineScope(SupervisorJob()+Dispatchers.Main.immediate)
    private var ticket:String?=null;private var saving=false
    override fun onCreate(state:Bundle?){super.onCreate(state);ticket=state?.getString("capture-ticket");show()}
    override fun onSaveInstanceState(state:Bundle){state.putString("capture-ticket",ticket);super.onSaveInstanceState(state)}
    private fun show(){
        val root=LinearLayout(this).apply{orientation=LinearLayout.VERTICAL;setPadding(24,24,24,24)}
        root.addView(TextView(this).apply{text="These private capture files were retained after an interrupted operation. Save a copy before retrying image import. A file interrupted during encoding may be incomplete."})
        val files=runCatching{CaptureLease.retained(this)}.getOrElse{emptyList()}
        files.forEach{lease->
            root.addView(Button(this).apply{text="Save capture (${lease.png.length()} bytes)";setOnClickListener{
                if(ticket==null&&!saving){ticket=lease.folder.name
                    @Suppress("DEPRECATION") startActivityForResult(Intent(Intent.ACTION_CREATE_DOCUMENT).addCategory(Intent.CATEGORY_OPENABLE).setType("image/png").putExtra(Intent.EXTRA_TITLE,"capture.png"),71)
                }
            }})
            root.addView(Button(this).apply{text="Discard this capture";setOnClickListener{
                if(ticket==null&&!saving)AlertDialog.Builder(this@CaptureInboxActivity).setMessage("Permanently discard this retained capture file?")
                    .setNegativeButton("Keep",null).setPositiveButton("Discard"){_,_->runCatching{lease.release()}.onFailure{notice("The capture file could not be removed safely.")};show()}.show()
            }})
        }
        if(files.isEmpty())root.addView(TextView(this).apply{text="No retained capture files."})
        setContentView(android.widget.ScrollView(this).apply{addView(root)})
    }
    @Deprecated("Platform callback retained for this standalone recovery activity")
    override fun onActivityResult(request:Int,result:Int,data:Intent?){super.onActivityResult(request,result,data)
        if(request!=71)return
        val name=ticket;ticket=null;val uri=data?.data
        if(result!=RESULT_OK||uri==null)return
        if(uri.scheme!="content"){notice("Choose a document provider.");return}
        saving=true
        scope.launch{
            val transfers=RasterTransfers(this@CaptureInboxActivity)
            var started=false
            try{val lease=CaptureLease.open(this@CaptureInboxActivity,checkNotNull(name));started=true;transfers.saveOriginal(lease.png,uri){};notice("Copy saved. The private original is retained until you discard it.")}
            catch(_:CancellationException){throw CancellationException("Capture recovery cancelled")}
            catch(_:Exception){notice("The copy did not finish. The private original is retained; a partial destination may remain.")}
            finally{if(!started)withContext(NonCancellable){runCatching{transfers.discardDestination(uri)}};saving=false;show()}
        }
    }
    private fun notice(value:String){Toast.makeText(this,value,Toast.LENGTH_LONG).show()}
    override fun onDestroy(){scope.cancel();super.onDestroy()}
}
