@file:OptIn(ExperimentalUnsignedTypes::class)
package com.visualworkbench.shared

import kotlinx.coroutines.flow.Flow

/** The public editor API is platform neutral; generated JNA types stay in actuals. */
public interface WorkbenchCore {
    public fun newId(unixMs: ULong): String
    public fun newDeviceId(): String
    public suspend fun create(options: CreateProject): WorkbenchProject
    public suspend fun open(path: String): WorkbenchProject
    public suspend fun layoutText(text: String, font: String, size: Float): TextLayout
    public fun cameraMatrix(camera: Camera, inverse: Boolean = false): Transform
    public fun mapPoints(camera: Camera, inverse: Boolean, points: List<Point>): List<Point>
}
public expect fun workbenchCore(): WorkbenchCore
internal expect fun prepareCoreLibrary()

public interface WorkbenchProject {
    public val changes: Flow<ProjectChange>
    public suspend fun info(): ProjectInfo
    public suspend fun document(documentId: String): DocumentSnapshot
    public suspend fun background(documentId: String, memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL, assumeUntaggedSrgb: Boolean = false): BackgroundImage
    public suspend fun beginStroke(options: StrokeOptions): WorkbenchStroke
    public suspend fun edit(options: EditOptions, commands: List<EditCommand>): ProjectInfo
    public suspend fun undoRedo(options: EditOptions, redo: Boolean): ProjectInfo
    public suspend fun render(documentId: String, rectangle: Rect? = null): RenderList
    public suspend fun export(options: ExportOptions): ExportResult
    public suspend fun close()
}
public interface WorkbenchStroke {
    /** Retain and retry this exact sequence only when CoreFailure.Backpressure is reported. */
    public suspend fun append(batch: SampleBatch): InkUpdate
    public suspend fun predict(batch: SampleBatch): InkUpdate
    public suspend fun commit(): ProjectInfo
    public fun cancel()
    /** Await native resource release; an already-started durable commit may finish. */
    public suspend fun dispose()
}
public enum class CoreFailureKind { Invalid, Backpressure, Closed, Cancelled, Storage, Unsupported, Worker, Stroke, Raster }
public class CoreFailure(public val kind: CoreFailureKind, cause: Throwable? = null) : Exception(kind.name, cause)
public data class CreateProject(public val path: String, public val projectId: String, public val documentId: String, public val layerId: String, public val deviceId: String, public val title: String, public val source: ByteArray, public val nowMs: Long)
public data class ProjectInfo(public val projectId: String, public val title: String, public val deviceId: String, public val nextLamport: ULong, public val canUndo: Boolean, public val canRedo: Boolean, public val hostSeq: ULong, public val stateHash: String, public val documentIds: List<String>)
public data class Point(public val x: Double, public val y: Double)
public data class Rect(public val x: Double, public val y: Double, public val width: Double, public val height: Double)
public data class Transform(public val a: Double = 1.0, public val b: Double = 0.0, public val c: Double = 0.0, public val d: Double = 1.0, public val e: Double = 0.0, public val f: Double = 0.0)
public data class Camera(public val center: Point, public val scale: Double, public val rotation: Double, public val viewportWidth: Double, public val viewportHeight: Double)
public data class StrokeOptions(public val gestureId: String, public val transactionId: String, public val objectId: String, public val documentId: String, public val layerId: String, public val deviceId: String, public val lamport: ULong, public val createdAtMs: Long, public val family: String, public val width: Double, public val rgba: UInt, public val stabilization: Float = 0f, public val pressureCurve: List<Double> = emptyList())
/** Immutable owned batch; callers must not mutate arrays after submitting it. */
public data class SampleBatch(public val sequence: ULong, public val x: DoubleArray, public val y: DoubleArray, public val timeMs: UIntArray, public val pressure: FloatArray, public val tilt: FloatArray = floatArrayOf(), public val orientation: FloatArray = floatArrayOf())
/** Compound NONZERO fill in 1/256 D units; draw together once, never alpha-blend each contour. */
public data class Contours(public val x: LongArray, public val y: LongArray, public val ends: UIntArray)
public data class InkUpdate(public val sequence: ULong, public val firstPolygon: ULong, public val sampleCount: ULong, public val contours: Contours)
public data class ObjectStyle(public val rgba: UInt, public val width: Double, public val screenConstantWidth: Boolean = false, public val fill: UInt? = null)
public sealed interface Outline {
    public data class Move(public val x: Float, public val y: Float) : Outline
    public data class Line(public val x: Float, public val y: Float) : Outline
    public data class Quad(public val x1: Float, public val y1: Float, public val x: Float, public val y: Float) : Outline
    public data class Cubic(public val x1: Float, public val y1: Float, public val x2: Float, public val y2: Float, public val x: Float, public val y: Float) : Outline
    public data object Close : Outline
}
public sealed interface Shape {
    public data class Stroke(public val family: String) : Shape
    public data class Line(public val points: List<Point>) : Shape
    public data class Arrow(public val points: List<Point>) : Shape
    public data class Rectangle(public val rectangle: Rect) : Shape
    public data class Ellipse(public val rectangle: Rect) : Shape
    public data class Polygon(public val points: List<Point>, public val closed: Boolean) : Shape
    public data class Text(public val anchor: Point, public val text: String, public val font: String, public val size: Double, public val outline: List<Outline> = emptyList()) : Shape
    public data class Marker(public val number: UInt, public val point: Point, public val rectangle: Rect?) : Shape
    public data class Guide(public val rectangle: Rect) : Shape
    public data class Result(public val assetId: String, public val resultId: String = "") : Shape
    public data object Adjustment : Shape
}
public data class RenderItem(public val objectId: String, public val layerId: String, public val layerOpacity: Double, public val layerBlend: String, public val bounds: Rect, public val contours: Contours, public val transform: Transform, public val style: ObjectStyle, public val shape: Shape, public val locked: Boolean)
public data class RenderList(public val revision: ProjectInfo, public val items: List<RenderItem>)
public data class LayerInfo(public val id: String, public val name: String, public val visible: Boolean, public val locked: Boolean, public val opacity: Double, public val blend: String)
public data class DocumentSnapshot(public val documentId: String, public val title: String, public val width: UInt, public val height: UInt, public val bitDepth: UInt, public val layers: List<LayerInfo>, public val render: RenderList)
/** Display derivative only. Never persist/export these bytes as the original asset. */
public data class BackgroundImage(public val width: UInt, public val height: UInt, public val rgba: ByteArray, public val sourceAssetId: String, public val sourceBitDepth: UByte, public val iccProfile: ByteArray)
public data class EditOptions(public val transactionId: String, public val documentId: String, public val deviceId: String, public val lamport: ULong, public val createdAtMs: Long, public val expectedHostSeq: ULong? = null, public val expectedStateHash: String? = null, public val gestureId: String? = null)
public sealed interface EditCommand {
    public data class Create(public val objectId: String, public val layerId: String, public val shape: Shape, public val style: ObjectStyle, public val transform: Transform = Transform()) : EditCommand
    public data class Delete(public val objectId: String) : EditCommand
    public data class SetTransform(public val objectId: String, public val transform: Transform) : EditCommand
    public data class SetText(public val objectId: String, public val text: String, public val font: String, public val size: Double) : EditCommand
    public data class SetStyle(public val objectId: String, public val style: ObjectStyle) : EditCommand
}
public sealed interface ImageFormat {
    public data object Png8 : ImageFormat
    public data object Png16 : ImageFormat
    public data class Jpeg(public val quality: UByte) : ImageFormat
    public data object WebpLossless : ImageFormat
    public data class WebpLossy(public val quality: UByte) : ImageFormat
}
public data class ExportOptions(public val documentId: String, public val format: ImageFormat = ImageFormat.Png8, public val marked: Boolean = true, public val region: Rect? = null, public val matteRgb: UInt? = null, public val convertToSrgb: Boolean = false, public val assumeUntaggedSrgb: Boolean = false, public val allowDepthReduction: Boolean = false, public val memoryBudgetBytes: ULong = 256uL * 1024uL * 1024uL)
public data class ExportResult(public val bytes: ByteArray, public val blake3: String, public val metadataJson: String, public val revision: ProjectInfo)
public data class GlyphOutline(public val glyphId: UInt, public val cluster: ULong, public val font: String, public val x: Float, public val y: Float, public val advance: Float, public val outline: List<Outline>)
public data class TextLayout(public val algorithmVersion: UInt, public val glyphs: List<GlyphOutline>, public val width: Float, public val height: Float, public val lineHeight: Float)
public enum class ChangeKind { Opened, Committed, ExportStarted, ExportReady, ExportFailed, Closed }
public data class ProjectChange(public val sequence: ULong, public val kind: ChangeKind, public val project: ProjectInfo)
