package com.visualworkbench.shared

/** Identity is native selected-process evidence, never a title/name guess.
 * Package version and executable fixed version deliberately remain separate. */
public enum class RemoteEditorKind { Krita, Paint, Unknown }
public data class RemoteEditorIdentity(
    public val editor:RemoteEditorKind,
    public val executableName:String,
    public val executableBlake3:String,
    public val executableBytes:ULong,
    public val fileVersion:String?,
    public val packageFullName:String?,
    public val packageVersion:String?,
    public val shortcutProfileDigest:String?,
)
public enum class RemoteEditorAction {
    Undo,Redo,SelectFreehandBrush,ToggleEraserMode,SelectEraser,BrushSmaller,BrushLarger,
    ZoomIn,ZoomOut,FitCanvas,ActualPixels,ResetRotation,PanLeft,PanRight,PanUp,PanDown,
}
public enum class RemoteCompatibilityAspect { Drawing,Pressure,Hover,Tilt,InvertedEraser,BarrelButton,Shortcuts }
public enum class RemoteCompatibilityResult { NotTested,Supported,Unsupported,Inconclusive }
public enum class RemoteCompatibilityMethod { NotPerformed,EditorObserved,PhysicalOwnerGesture }
public enum class RemoteEditorInputApi { WindowsPointerInput,Mouse,Unknown }
public data class RemoteToolCompatibility(
    public val identity:RemoteEditorIdentity,
    public val toolId:String,
    public val settingsDigest:String,
    public val inputApi:RemoteEditorInputApi,
    public val aspect:RemoteCompatibilityAspect,
    public val result:RemoteCompatibilityResult,
    public val method:RemoteCompatibilityMethod,
    public val evidenceSha256:String?,
    public val deviceModel:String?,
)
public enum class RemoteEditorLimit { NoPixelLayerBridge,NoLayeredPsdRoundTrip,NoPaintMultiview,AdditionalViewsDeferred }
private fun String.boundedText(max:Int):Boolean = length in 1..max && none{it.code<32||it.code==127}
private fun String.digest():Boolean = length==64 && all{it in '0'..'9'||it in 'a'..'f'}
private fun String.version():Boolean = boundedText(64) && split('.').let{it.size in 2..4&&it.all{p->p.isNotEmpty()&&p.length<=5&&p.all{c->c in '0'..'9'}&&p.toUIntOrNull()?.let{n->n<=65535u}==true}}
public fun remotePackageVersion(fullName:String):String? {
    if(!fullName.boundedText(2048))return null
    val fields=fullName.split('_');if(fields.size!=5||fields[0].isEmpty()||fields[2].isEmpty()||fields[4].isEmpty())return null
    return fields[1].takeIf{it.split('.').size==4&&it.version()}
}
public fun RemoteEditorIdentity.valid():Boolean =
    executableName.boundedText(128)&&'/' !in executableName&&'\\' !in executableName&&executableBlake3.digest()&&executableBytes in 64uL..(256uL*1024uL*1024uL)&&
    (fileVersion==null||fileVersion.version())&&(packageFullName==null||packageFullName.boundedText(2048))&&(packageVersion==null||packageVersion.version())&&
    (shortcutProfileDigest==null||shortcutProfileDigest.digest())&&
    (if(packageFullName==null)packageVersion==null else remotePackageVersion(packageFullName)==packageVersion)&&
    when(editor){RemoteEditorKind.Krita->executableName.equals("krita.exe",true)&&fileVersion!=null;RemoteEditorKind.Paint->executableName.equals("mspaint.exe",true)&&packageFullName!=null&&packageVersion!=null;RemoteEditorKind.Unknown->true}
public fun RemoteToolCompatibility.validFor(current:RemoteEditorIdentity,tool:String,settings:String):Boolean {
    if(!identity.valid()||identity!=current||toolId!=tool||settingsDigest!=settings||!toolId.boundedText(128)||!settingsDigest.digest())return false
    if(deviceModel!=null&&!deviceModel.boundedText(128))return false
    if(result==RemoteCompatibilityResult.NotTested)return method==RemoteCompatibilityMethod.NotPerformed&&evidenceSha256==null
    if(method==RemoteCompatibilityMethod.NotPerformed||evidenceSha256?.digest()!=true)return false
    if(aspect in setOf(RemoteCompatibilityAspect.Pressure,RemoteCompatibilityAspect.Hover,RemoteCompatibilityAspect.Tilt,RemoteCompatibilityAspect.InvertedEraser,RemoteCompatibilityAspect.BarrelButton) && (inputApi!=RemoteEditorInputApi.WindowsPointerInput||method!=RemoteCompatibilityMethod.PhysicalOwnerGesture||deviceModel==null))return false
    if(aspect==RemoteCompatibilityAspect.Pressure&&identity.editor==RemoteEditorKind.Krita&&result==RemoteCompatibilityResult.Supported&&inputApi!=RemoteEditorInputApi.WindowsPointerInput)return false
    return true
}
public fun remoteEditorLimits(editor:RemoteEditorKind):List<RemoteEditorLimit> = buildList {
    add(RemoteEditorLimit.NoPixelLayerBridge);add(RemoteEditorLimit.NoLayeredPsdRoundTrip)
    if(editor==RemoteEditorKind.Paint)add(RemoteEditorLimit.NoPaintMultiview) else add(RemoteEditorLimit.AdditionalViewsDeferred)
}
public fun remoteEditorLimitText(limit:RemoteEditorLimit):String = when(limit){
    RemoteEditorLimit.NoPixelLayerBridge->"Remote input only; pixel and layer exchange is unavailable."
    RemoteEditorLimit.NoLayeredPsdRoundTrip->"Layered PSD round trips are not verified."
    RemoteEditorLimit.NoPaintMultiview->"Paint has no verified same-document multiview workflow."
    RemoteEditorLimit.AdditionalViewsDeferred->"Additional editor views are outside this milestone."
}
public fun remoteEditorActionLabel(editor:RemoteEditorKind,action:RemoteEditorAction):String = when(action){
    RemoteEditorAction.Undo->"Undo in ${editor.name}";RemoteEditorAction.Redo->"Redo in ${editor.name}"
    RemoteEditorAction.SelectFreehandBrush->if(editor==RemoteEditorKind.Krita)"Krita freehand tool · eraser mode unchanged" else "Paint brush"
    RemoteEditorAction.ToggleEraserMode->"Toggle Krita eraser mode"
    RemoteEditorAction.SelectEraser->"Paint eraser"
    RemoteEditorAction.BrushSmaller->"Smaller brush";RemoteEditorAction.BrushLarger->"Larger brush"
    RemoteEditorAction.ZoomIn->"Zoom editor in";RemoteEditorAction.ZoomOut->"Zoom editor out"
    RemoteEditorAction.FitCanvas->"Fit editor canvas";RemoteEditorAction.ActualPixels->"Editor 100% view"
    RemoteEditorAction.ResetRotation->"Reset editor rotation"
    RemoteEditorAction.PanLeft->"Pan editor left";RemoteEditorAction.PanRight->"Pan editor right"
    RemoteEditorAction.PanUp->"Pan editor up";RemoteEditorAction.PanDown->"Pan editor down"
}
public fun remoteEditorActions(editor:RemoteEditorKind):List<RemoteEditorAction> = when(editor){
    RemoteEditorKind.Unknown->emptyList()
    RemoteEditorKind.Krita->listOf(RemoteEditorAction.Undo,RemoteEditorAction.Redo,RemoteEditorAction.SelectFreehandBrush,RemoteEditorAction.ToggleEraserMode,RemoteEditorAction.BrushSmaller,RemoteEditorAction.BrushLarger,RemoteEditorAction.ZoomIn,RemoteEditorAction.ZoomOut,RemoteEditorAction.FitCanvas,RemoteEditorAction.ActualPixels,RemoteEditorAction.ResetRotation,RemoteEditorAction.PanLeft,RemoteEditorAction.PanRight,RemoteEditorAction.PanUp,RemoteEditorAction.PanDown)
    RemoteEditorKind.Paint->listOf(RemoteEditorAction.Undo,RemoteEditorAction.Redo,RemoteEditorAction.SelectFreehandBrush,RemoteEditorAction.SelectEraser,RemoteEditorAction.BrushSmaller,RemoteEditorAction.BrushLarger,RemoteEditorAction.ZoomIn,RemoteEditorAction.ZoomOut,RemoteEditorAction.FitCanvas,RemoteEditorAction.ActualPixels,RemoteEditorAction.PanLeft,RemoteEditorAction.PanRight,RemoteEditorAction.PanUp,RemoteEditorAction.PanDown)
}
