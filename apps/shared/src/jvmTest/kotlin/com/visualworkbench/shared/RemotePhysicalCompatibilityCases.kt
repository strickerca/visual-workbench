package com.visualworkbench.shared

import kotlinx.coroutines.runBlocking

/** Python/Kotlin cross-check for owner-physical Paint compatibility rows.
 * tools/remote-edit/test_paint_physical_reports.py builds a complete synthetic
 * multi-block evaluation, runs paint_physical_reports.report() and asserts its
 * documents equal these exact bytes (regenerate with
 * `python test_paint_physical_reports.py --print-kotlin-fixtures`).
 * Synthetic fixture bytes exercise admission only; they are never evidence. */
internal object RemotePhysicalCompatibilityCases {
    private const val BRUSH:String = /*PHYSICAL_BRUSH_FIXTURE*/"""{"schema":1,"kind":"paint-compatibility-observations","identity":{"editor":"Paint","executableName":"mspaint.exe","executableBlake3":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","executableBytes":1000,"fileVersion":null,"packageFullName":"Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe","packageVersion":"11.2605.81.0","shortcutProfileDigest":null},"toolId":"paint.brush","settingsDigest":"dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd","observations":[{"aspect":"Drawing","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"bf68faf60ce9f55a452379fcfb9ad305228bf46c71920b6fb15b2e866fbc0e0f","deviceModel":"SM-S918U"},{"aspect":"Pressure","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"9ff86625672b6cef5800c115b826ac0b272eaba3a18286ee0728114850785f85","deviceModel":"SM-S918U"},{"aspect":"Hover","result":"Inconclusive","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"105b077bde424a7a00b5f1ff6745107c68898c0cdb78a08b86449f95b79ea8c8","deviceModel":"SM-S918U"},{"aspect":"Tilt","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"InvertedEraser","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"BarrelButton","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"e9d2dd75449edf081d7892d4fdf6731e64788a77e40dff580f7f1cd0d3e0e7f9","deviceModel":"SM-S918U"},{"aspect":"Shortcuts","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null}]}"""
    private const val ERASER:String = /*PHYSICAL_ERASER_FIXTURE*/"""{"schema":1,"kind":"paint-compatibility-observations","identity":{"editor":"Paint","executableName":"mspaint.exe","executableBlake3":"cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc","executableBytes":1000,"fileVersion":null,"packageFullName":"Microsoft.Paint_11.2605.81.0_x64__8wekyb3d8bbwe","packageVersion":"11.2605.81.0","shortcutProfileDigest":null},"toolId":"paint.eraser","settingsDigest":"9999999999999999999999999999999999999999999999999999999999999999","observations":[{"aspect":"Drawing","result":"Supported","method":"PhysicalOwnerGesture","inputApi":"WindowsPointerInput","evidenceSha256":"3d77489172b528dcfe09c5993bcdf5d87b1b1fe89dc225938941b80fdc346f9f","deviceModel":"SM-S918U"},{"aspect":"Pressure","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"Hover","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"Tilt","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"InvertedEraser","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"BarrelButton","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null},{"aspect":"Shortcuts","result":"NotTested","method":"NotPerformed","inputApi":"Unknown","evidenceSha256":null,"deviceModel":null}]}"""

    fun run():Int = runBlocking {
        var groups=0
        for((tool,settings,text) in listOf(Triple("paint.brush","d".repeat(64),BRUSH),Triple("paint.eraser","9".repeat(64),ERASER))){
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
        val brush=NativeRemoteCompatibility.decode(target(),BRUSH.toByteArray(Charsets.UTF_8)).records
        check(brush.single{it.aspect==RemoteCompatibilityAspect.Pressure}.result==RemoteCompatibilityResult.Supported)
        check(brush.single{it.aspect==RemoteCompatibilityAspect.Hover}.result==RemoteCompatibilityResult.Inconclusive)
        check(brush.single{it.aspect==RemoteCompatibilityAspect.Tilt}.result==RemoteCompatibilityResult.NotTested)
        val pressureSha=requireNotNull(brush.single{it.aspect==RemoteCompatibilityAspect.Pressure}.evidenceSha256)
        val invalid=listOf(
            // Physical-only aspects require a device model, the physical method and Windows pointer input.
            row(BRUSH,"Pressure"){it.replace("\"deviceModel\":\"SM-S918U\"","\"deviceModel\":null")},
            row(BRUSH,"Pressure"){it.replace("PhysicalOwnerGesture","EditorObserved")},
            row(BRUSH,"Pressure"){it.replace("WindowsPointerInput","Mouse")},
            row(BRUSH,"Pressure"){it.replace("PhysicalOwnerGesture","NotPerformed")},
            row(BRUSH,"Pressure"){it.replace("\"evidenceSha256\":\"$pressureSha\"","\"evidenceSha256\":null")},
            // NotTested carries no evidence or performed method.
            row(BRUSH,"Tilt"){it.replace("\"evidenceSha256\":null","\"evidenceSha256\":\"${"6".repeat(64)}\"")},
            row(BRUSH,"Tilt"){it.replace("NotPerformed","PhysicalOwnerGesture")},
            // A different executable image or a duplicate aspect is never admitted.
            BRUSH.replace("c".repeat(64),"b".repeat(64)),
            row(BRUSH,"Tilt"){it.replace("\"Tilt\"","\"Hover\"")},
        )
        invalid.forEach{raw->check(raw!=BRUSH&&runCatching{NativeRemoteCompatibility.decode(target(),raw.toByteArray(Charsets.UTF_8))}.isFailure)}
        groups+invalid.size
    }
    /** Replaces the one observation object for [aspect] within [document]. */
    private fun row(document:String,aspect:String,transform:(String)->String):String {
        val start=document.indexOf("{\"aspect\":\"$aspect\"");check(start>=0)
        val end=document.indexOf('}',start)+1;check(end>start)
        return document.substring(0,start)+transform(document.substring(start,end))+document.substring(end)
    }
    private fun target():RemoteTargetState {
        val binding=RemoteTargetBinding(1uL,"capture",1uL,"target",1u,"input")
        val identity=RemoteEditorIdentity(RemoteEditorKind.Paint,"mspaint.exe","c".repeat(64),1000uL,null,
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
