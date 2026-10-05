package com.visualworkbench.shared

/** Bounded native DTO reader. No dynamic class loading or captured text eval. */
internal object RemoteNativeJson {
    private data class Number(val text:String)
    fun read(text:String):Map<String,Any?> = Reader(text).readObject()
    @Suppress("UNCHECKED_CAST")
    fun obj(value:Any?):Map<String,Any?> = value as? Map<String,Any?> ?: error("Remote object absent")
    fun string(value:Any?):String = value as? String ?: error("Remote string absent")
    fun optional(value:Any?):String? = value?.let(::string)
    fun ulong(value:Any?):ULong = (value as? Number)?.text?.toULongOrNull() ?: error("Remote unsigned integer")
    fun uint(value:Any?):UInt = ulong(value).also { require(it<=UInt.MAX_VALUE.toULong()) }.toUInt()
    fun int(value:Any?):Int = (value as? Number)?.text?.toIntOrNull() ?: error("Remote signed integer")
    fun array(value:Any?):List<Any?> = value as? List<*> ?: error("Remote array absent")
    fun bytes(value:Any?):ByteArray = array(value).also{require(it.size<=8192)}.map { uint(it).also{n->require(n<=255u)}.toByte() }.toByteArray()
    private fun validText(value:String):Boolean {
        var at=0
        while(at<value.length){val c=value[at++];if(c.isHighSurrogate()){if(at>=value.length||!value[at++].isLowSurrogate())return false}else if(c.isLowSurrogate())return false}
        return true
    }
    fun quote(value:String):String = buildString {
        require(validText(value))
        append('"');for(c in value) when(c){'"'->append("\\\"");'\\'->append("\\\\");else->if(c.code<32)append("\\u"+c.code.toString(16).padStart(4,'0'))else append(c)};append('"')
    }
    fun scope(s:RemoteVideoScope):String = "{\"connection_epoch\":${s.connectionEpoch},\"capture_session_id\":${quote(s.captureSessionId)},\"source_generation\":${s.sourceGeneration},\"target_token\":${quote(s.targetToken)},\"geometry_revision\":${s.geometryRevision}}"
    fun binding(b:RemoteTargetBinding):String = "{\"scope\":${scope(RemoteVideoScope(b.connectionEpoch,b.captureSessionId,b.sourceGeneration,b.targetToken,b.geometryRevision))},\"input_session_id\":${quote(b.inputSessionId)}}"
    fun scope(v:Map<String,Any?>):RemoteVideoScope = RemoteVideoScope(ulong(v["connection_epoch"]),string(v["capture_session_id"]),ulong(v["source_generation"]),string(v["target_token"]),uint(v["geometry_revision"]))
    fun binding(v:Map<String,Any?>):RemoteTargetBinding = scope(obj(v["scope"])).let { RemoteTargetBinding(it.connectionEpoch,it.captureSessionId,it.sourceGeneration,it.targetToken,it.geometryRevision,string(v["input_session_id"])) }
    fun rect(v:Map<String,Any?>):RemotePhysicalRect = RemotePhysicalRect(int(v["x"]),int(v["y"]),uint(v["width"]),uint(v["height"]))
    fun config(text:String):RemoteVideoConfig {
        val v=read(text);val cw=uint(v["coded_width"]);val ch=uint(v["coded_height"]);val vw=uint(v["visible_width"]);val vh=uint(v["visible_height"])
        require(cw in 2u..4096u&&ch in 2u..4096u&&cw%2u==0u&&ch%2u==0u&&vw in 1u..cw&&vh in 1u..ch&&cw-vw<=1u&&ch-vh<=1u&&ulong(v["generation"])==1uL)
        val vps=bytes(v["vps"]);val sps=bytes(v["sps"]);val pps=bytes(v["pps"])
        require(vps.isNotEmpty()&&sps.isNotEmpty()&&pps.isNotEmpty()&&vps.size+sps.size+pps.size<=8192)
        return RemoteVideoConfig(scope(obj(v["scope"])),1uL,cw.toInt(),ch.toInt(),vw.toInt(),vh.toInt(),vps,sps,pps)
    }
    private class Reader(private val source:String) {
        private var at=0;private var values=0
        init {require(source.length<=48*1024)}
        fun readObject():Map<String,Any?> {val result=obj(value(0));space();require(at==source.length);return result}
        private fun space(){while(at<source.length&&source[at] in " \n\r\t")at++}
        private fun take(c:Char):Boolean {space();if(at<source.length&&source[at]==c){at++;return true};return false}
        private fun value(depth:Int):Any? {
            require(depth<=16&&++values<=12_288);space();require(at<source.length)
            return when(source[at]) {
                '{'->{at++;val map=linkedMapOf<String,Any?>();if(!take('}')){while(true){space();val key=text();require(take(':')&&!map.containsKey(key));map[key]=value(depth+1);if(!take(',')){require(take('}'));break}}};map}
                '['->{at++;val list=mutableListOf<Any?>();if(!take(']')){while(true){list.add(value(depth+1));if(!take(',')){require(take(']'));break}}};list}
                '"'->text()
                'n'->{literal("null");null}
                't'->{literal("true");true}
                'f'->{literal("false");false}
                else->{val start=at;if(source[at]=='-')at++;require(at<source.length);if(source[at]=='0')at++ else {require(source[at] in '1'..'9');while(at<source.length&&source[at] in '0'..'9')at++};if(at<source.length&&source[at]=='.'){at++;val digit=at;while(at<source.length&&source[at] in '0'..'9')at++;require(at>digit)};require(at-start<=32);Number(source.substring(start,at))}
            }
        }
        private fun literal(value:String){require(source.startsWith(value,at));at+=value.length}
        private fun text():String {
            require(at<source.length&&source[at++]=='"');val out=StringBuilder()
            while(at<source.length){val c=source[at++];if(c=='"'){val v=out.toString();require(v.length<=8192&&validText(v));return v};require(c.code>=32)
                if(c!='\\')out.append(c)else{require(at<source.length);when(val e=source[at++]){'"','\\','/'->out.append(e);'b'->out.append('\b');'f'->out.append('\u000c');'n'->out.append('\n');'r'->out.append('\r');'t'->out.append('\t');'u'->{require(at+4<=source.length);out.append(source.substring(at,at+4).toInt(16).toChar());at+=4};else->error("Remote string escape")}}
                require(out.length<=8192)
            };error("Remote string truncated")
        }
    }
}
