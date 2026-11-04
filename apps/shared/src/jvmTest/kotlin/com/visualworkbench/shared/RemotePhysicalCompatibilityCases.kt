package com.visualworkbench.shared

import kotlinx.coroutines.runBlocking

/** Python/Kotlin cross-check for owner-physical Paint compatibility rows.
 * tools/remote-edit/test_paint_physical_reports.py regenerates these exact bytes
 * with paint_physical_reports.compatibility_document and compares them here.
 * Synthetic fixture bytes exercise admission only; they are never evidence. */
internal object RemotePhysicalCompatibilityCases {
    private const val BRUSH:String = /*PHYSICAL_BRUSH_FIXTURE*/"""{"schema":1,"kind":"paint-compatibility-observations","identity":{"editor":"Paint","executableName":"mspaint.exe","executableBlake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","executableBytes":1024,"fileVersion":"11.2605.81.0","packageFullName":"Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe","packageVersion":"11.2605.81.0","shortcutProfileDigest":null},"toolId":"paint.brush","settingsDigest":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","observations":[{"aspect":"Drawing","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"1111111111111111111111111111111111111111111111111111111111111111","deviceModel":"SM-S918U"},{"aspect":"Pressure","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"2222222222222222222222222222222222222222222222222222222222222222","deviceModel":"SM-S918U"},{"aspect":"Hover","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"3333333333333333333333333333333333333333333333333333333333333333","deviceModel":"SM-S918U"},{"aspect":"Tilt","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"InvertedEraser","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"BarrelButton","result":"Inconclusive","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"4444444444444444444444444444444444444444444444444444444444444444","deviceModel":"SM-S918U"},{"aspect":"Shortcuts","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null}]}"""
    private const val ERASER:String = /*PHYSICAL_ERASER_FIXTURE*/"""{"schema":1,"kind":"paint-compatibility-observations","identity":{"editor":"Paint","executableName":"mspaint.exe","executableBlake3":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","executableBytes":1024,"fileVersion":"11.2605.81.0","packageFullName":"Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe","packageVersion":"11.2605.81.0","shortcutProfileDigest":null},"toolId":"paint.eraser","settingsDigest":"eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee","observations":[{"aspect":"Drawing","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"5555555555555555555555555555555555555555555555555555555555555555","deviceModel":"SM-S918U"},{"aspect":"Pressure","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"Hover","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"Tilt","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"InvertedEraser","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"BarrelButton","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"Shortcuts","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null}]}"""

    fun run():Int = runBlocking {
        var groups=0
        for((tool,settings,text) in listOf(Triple("paint.brush","d".repeat(64),BRUSH),Triple("paint.eraser","e".repeat(64),ERASER))){
            val target=target();val bytes=text.toByteArray(Charsets.UTF_8)
            val decoded=NativeRemoteCompatibility.decode(target,bytes)
            check(decoded.toolId==tool&&decoded.settingsDigest==settings&&decoded.validFor(target))
            check(decoded.records.map{it.aspect}==RemoteCompatibilityAspect.entries.toList())
            check(decoded.records.all{it.validFor(target.identity,tool,settings)})
            for(row in decoded.records){
                if(row.result==RemoteCompatibilityResult.NotTested)
                    check(row.method==RemoteCompatibilityMethod.NotPerformed&&row.evidenceSha256==null&&row.deviceModel==null)
                else check(row.method==RemoteCompatibilityMethod.PhysicalOwnerGesture&&row.inputApi==RemoteEditorInputApi.WindowsPointerInput&&
                    row.deviceModel=="SM-S918U"&&row.evidenceSha256?.length==64)
            }
            // An owner retaining these exact bytes and records admits them; the
            // admission is scoped to this exact tool and settings digest.
            check(NativeRemoteCompatibility.admit(target,bytes,Owner(target,tool,settings,bytes,decoded.records))!=null)
            check(decoded.records.none{it.validFor(target.identity,tool,"f".repeat(64))||it.validFor(target.identity,"paint.other",settings)})
            // The joined drawing projection remains mouse-only; physical rows never pass it.
            check(runCatching{NativeRemoteCompatibility.admitDrawing(target,NativeOwnedCompatibilityReceipt(target,tool,settings,bytes,decoded.records))}.isFailure)
            groups++
        }
        val pressure="{\"aspect\":\"Pressure\",\"result\":\"Supported\",\"method\":\"PhysicalOwnerGesture\",\"inputApi\":\"WindowsPointerInput\",\"evidenceSha256\":\"${"2".repeat(64)}\",\"deviceModel\":\"SM-S918U\"}"
        check(BRUSH.contains(pressure))
        val tilt="{\"aspect\":\"Tilt\",\"result\":\"NotTested\",\"method\":\"NotPerformed\",\"inputApi\":\"Unknown\",\"evidenceSha256\":null,\"deviceModel\":null}"
        check(BRUSH.contains(tilt))
        val invalid=listOf(
            // Physical-only aspects require a device model, the physical method and Windows pointer input.
            BRUSH.replace(pressure,pressure.replace("\"deviceModel\":\"SM-S918U\"","\"deviceModel\":null")),
            BRUSH.replace(pressure,pressure.replace("PhysicalOwnerGesture","EditorObserved")),
            BRUSH.replace(pressure,pressure.replace("WindowsPointerInput","Mouse")),
            BRUSH.replace(pressure,pressure.replace("PhysicalOwnerGesture","NotPerformed")),
            BRUSH.replace(pressure,pressure.replace("\"evidenceSha256\":\"${"2".repeat(64)}\"","\"evidenceSha256\":null")),
            // NotTested carries no evidence or performed method.
            BRUSH.replace(tilt,tilt.replace("\"evidenceSha256\":null","\"evidenceSha256\":\"${"6".repeat(64)}\"")),
            BRUSH.replace(tilt,tilt.replace("NotPerformed","PhysicalOwnerGesture")),
            // A different executable image or duplicate aspect is never admitted.
            BRUSH.replace("a".repeat(64),"b".repeat(64)),
            BRUSH.replace(tilt,tilt.replace("\"Tilt\"","\"Hover\"")),
        )
        invalid.forEach{raw->check(raw!=BRUSH&&runCatching{NativeRemoteCompatibility.decode(target(),raw.toByteArray(Charsets.UTF_8))}.isFailure)}
        groups+invalid.size
    }
    private fun target():RemoteTargetState {
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,"mspaint.exe","a".repeat(64),1024uL,"11.2605.81.0",
            "Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe","11.2605.81.0",null)
        return RemoteTargetState(binding,identity,setOf(RemoteEditorAction.Undo),
            RemoteCanvasFocusProof("UIA","d".repeat(64),42u,"target",1u,100uL,"canvas",50026,RemotePhysicalRect(0,0,10u,10u),"b".repeat(64)),true,null)
    }
    private class Owner(val target:RemoteTargetState,val tool:String,val settings:String,val bytes:ByteArray,
        val records:List<RemoteToolCompatibility>):NativeCompatibilityObservationAccess {
        override suspend fun withCurrentReceipt(expected:RemoteTargetState,receiptSha256:String,
            body:suspend(NativeOwnedCompatibilityReceipt?)->RemoteCompatibilityAdmission?):RemoteCompatibilityAdmission? =
            body(if(receiptSha256==NativeRemoteCompatibility.sha(bytes))NativeOwnedCompatibilityReceipt(target,tool,settings,bytes,records) else null)
    }
}
public fun main():Unit {println("Physical compatibility cross-check groups passed: ${RemotePhysicalCompatibilityCases.run()}")}
