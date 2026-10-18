package com.visualworkbench.shared

/** JSON remains a closed native transport contract. Selection-only observation
 * cannot be relabeled as keyboard focus by a missing/unknown field. */
internal data class RemoteAuthorityObservation(
    val focus:RemoteCanvasFocusProof?,
    val selected:RemoteSelectedCanvasProof?,
)

internal fun remoteAuthorityObservation(value:Map<String,Any?>,binding:RemoteTargetBinding):RemoteAuthorityObservation {
    require(value.keys==setOf("identity","profile_digest","verified_actions","routes","focus","selected_canvas"))
    val routes=RemoteNativeJson.string(value["routes"])
    require(routes in setOf("legacy","essential"))
    val focused=value["focus"]
    val selected=value["selected_canvas"]
    require((focused==null)!=(selected==null))
    require(selected==null||routes=="essential")
    val canvas=RemoteNativeJson.obj(focused?:selected)
    require(canvas.keys==setOf("runtime_id_hash","process_id","sampled_qpc_100ns","class_name","control_type","canvas_rect","profile_digest"))
    val identity=RemoteNativeJson.obj(value["identity"])
    require(identity.keys==setOf("executableName","executableBlake3","executableBytes","fileVersion","packageFullName","packageVersion"))
    val runtime=RemoteNativeJson.string(canvas["runtime_id_hash"])
    val digest=RemoteNativeJson.string(canvas["profile_digest"])
    fun hash(text:String):Boolean=text.length==64&&text.all{it in '0'..'9'||it in 'a'..'f'}
    require(hash(runtime)&&hash(digest)&&digest==RemoteNativeJson.string(value["profile_digest"]))
    val process=RemoteNativeJson.uint(canvas["process_id"])
    val sampled=RemoteNativeJson.ulong(canvas["sampled_qpc_100ns"])
    val className=RemoteNativeJson.string(canvas["class_name"])
    val kind=RemoteNativeJson.int(canvas["control_type"])
    require(process>0u&&sampled>0uL&&className.length in 1..256&&className.none{it.code<32||it.code==127}&&kind in 50000..50100)
    val rectValue=RemoteNativeJson.obj(canvas["canvas_rect"])
    require(rectValue.keys==setOf("x","y","width","height"))
    val rect=RemoteNativeJson.rect(rectValue)
    require(rect.width in 1u..32768u&&rect.height in 1u..32768u&&
        rect.x.toLong()+rect.width.toLong()<=Int.MAX_VALUE&&rect.y.toLong()+rect.height.toLong()<=Int.MAX_VALUE)
    val actions=RemoteNativeJson.array(value["verified_actions"]).map(RemoteNativeJson::uint)
    require(actions.size<=16&&actions.distinct().size==actions.size&&actions.all{it in 1u..16u})
    require(routes!="essential"||actions.all{it<=10u})
    return if(focused!=null)RemoteAuthorityObservation(
        RemoteCanvasFocusProof("Windows UIA",runtime,process,binding.targetToken,binding.geometryRevision,sampled,className,kind,rect,digest),null)
    else RemoteAuthorityObservation(null,
        RemoteSelectedCanvasProof("Windows UIA",runtime,process,binding.targetToken,binding.geometryRevision,sampled,className,kind,rect,digest))
}
