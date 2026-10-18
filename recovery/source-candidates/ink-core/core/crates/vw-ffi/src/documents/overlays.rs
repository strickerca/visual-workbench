//! Numeric outlines only; no PDF/SVG content is interpreted in the store worker.
use crate::*;
use tiny_skia::{Path, PathBuilder, PathSegment, Stroke};

#[derive(Clone, Debug, uniffi::Record)]
pub struct EditableOverlay { pub page: u32, pub contours: Vec<Vec<Point>>, pub transform: Transform, pub rgba: u32 }
const IDENTITY: Transform = Transform { a: 1.0, b: 0.0, c: 0.0, d: 1.0, e: 0.0, f: 0.0 };
fn coordinate(n: f64) -> Result<f32> { if !n.is_finite() || n.abs() > 1e7 { Err(CoreError::Invalid) } else { Ok(n as f32) } }
fn rectangle(r: QueryRect) -> Result<tiny_skia::Rect> { tiny_skia::Rect::from_xywh(coordinate(r.x)?, coordinate(r.y)?, coordinate(r.width)?, coordinate(r.height)?).ok_or(CoreError::Invalid) }
fn transform(t: Transform) -> Result<tiny_skia::Transform> { Ok(tiny_skia::Transform::from_row(coordinate(t.a)?,coordinate(t.b)?,coordinate(t.c)?,coordinate(t.d)?,coordinate(t.e)?,coordinate(t.f)?)) }
fn polyline(points: &[Point], closed: bool) -> Result<Path> {
    let mut path = PathBuilder::new();
    for (index, p) in points.iter().enumerate() { if index == 0 { path.move_to(coordinate(p.x)?,coordinate(p.y)?); } else { path.line_to(coordinate(p.x)?,coordinate(p.y)?); } }
    if closed { path.close(); } path.finish().ok_or(CoreError::Invalid)
}
fn text(outline: &[Outline], anchor: Point) -> Result<Path> {
    let mut path = PathBuilder::new(); let x = coordinate(anchor.x)?; let y = coordinate(anchor.y)?;
    for command in outline { match *command {
        Outline::Move { x: a, y: b } => path.move_to(a+x,b+y), Outline::Line { x: a, y: b } => path.line_to(a+x,b+y),
        Outline::Quad { x1, y1, x: a, y: b } => path.quad_to(x1+x,y1+y,a+x,b+y),
        Outline::Cubic { x1, y1, x2, y2, x: a, y: b } => path.cubic_to(x1+x,y1+y,x2+x,y2+y,a+x,b+y), Outline::Close => path.close(),
    } }
    path.finish().ok_or(CoreError::Invalid)
}
fn midpoint(a: Point, b: Point) -> Point { Point { x: (a.x+b.x)*0.5, y: (a.y+b.y)*0.5 } }
fn push(points: &mut Vec<Point>, p: Point, count: &mut usize) -> Result<()> {
    *count = count.checked_add(1).ok_or(CoreError::Backpressure)?; if *count > 250_000 { return Err(CoreError::Backpressure); }
    points.try_reserve(1).map_err(|_| CoreError::Backpressure)?; points.push(p); Ok(())
}
fn distance(p: Point, a: Point, b: Point) -> f64 { let dx=b.x-a.x; let dy=b.y-a.y; let length=(dx*dx+dy*dy).sqrt(); if length < 1e-12 { ((p.x-a.x).powi(2)+(p.y-a.y).powi(2)).sqrt() } else { ((p.x-a.x)*dy-(p.y-a.y)*dx).abs()/length } }
fn cubic(a: Point, b: Point, c: Point, d: Point, depth: u8, points: &mut Vec<Point>, count: &mut usize) -> Result<()> {
    if distance(b,a,d).max(distance(c,a,d)) <= 0.0625 { return push(points,d,count); }
    if depth >= 16 { return Err(CoreError::Unsupported); }
    let ab=midpoint(a,b); let bc=midpoint(b,c); let cd=midpoint(c,d); let abc=midpoint(ab,bc); let bcd=midpoint(bc,cd); let middle=midpoint(abc,bcd);
    cubic(a,ab,abc,middle,depth+1,points,count)?; cubic(middle,bcd,cd,d,depth+1,points,count)
}
fn flatten(path: &Path, count: &mut usize) -> Result<Vec<Vec<Point>>> {
    let p=|n: tiny_skia::Point| Point { x: f64::from(n.x), y: f64::from(n.y) };
    let mut contours = Vec::new(); let mut points=Vec::new(); let mut current=Point{x:0.0,y:0.0};
    for segment in path.segments() { match segment {
        PathSegment::MoveTo(to) => { if !points.is_empty() { if points.len()>=3 { contours.push(std::mem::take(&mut points)); } else { points.clear(); } } current=p(to); push(&mut points,current,count)?; }
        PathSegment::LineTo(to) => { current=p(to); push(&mut points,current,count)?; }
        PathSegment::QuadTo(control,to) => { let b=p(control); let d=p(to); let a=current; cubic(a,Point{x:a.x+(b.x-a.x)*2.0/3.0,y:a.y+(b.y-a.y)*2.0/3.0},Point{x:d.x+(b.x-d.x)*2.0/3.0,y:d.y+(b.y-d.y)*2.0/3.0},d,0,&mut points,count)?; current=d; }
        PathSegment::CubicTo(b,c,d) => { let d=p(d); cubic(current,p(b),p(c),d,0,&mut points,count)?; current=d; }
        PathSegment::Close => { if points.len()>=3 { contours.push(std::mem::take(&mut points)); } else { points.clear(); } }
    } if contours.len()>16_384 { return Err(CoreError::Backpressure); } }
    if points.len()>=3 { contours.push(points); } Ok(contours)
}
fn append(result: &mut Vec<EditableOverlay>, path: Path, item: &RenderItem, page: u32, rgba: u32, count: &mut usize) -> Result<()> {
    if rgba & 255 == 0 { return Ok(()); }
    // Flatten after the affine so the error bound is in page D, including scale.
    let path=path.transform(transform(item.transform)?).ok_or(CoreError::Invalid)?;
    let contours=flatten(&path,count)?; if contours.is_empty() { return Ok(()); }
    let alpha=((rgba & 255) as f64 * item.layer_opacity).round().clamp(0.0,255.0) as u32;
    if result.len()>=4096 { return Err(CoreError::Backpressure); }
    result.push(EditableOverlay { page, contours, transform: IDENTITY, rgba: rgba & 0xffff_ff00 | alpha }); Ok(())
}
fn draw(result: &mut Vec<EditableOverlay>, path: Path, item: &RenderItem, page: u32, fill_only: bool, count: &mut usize) -> Result<()> {
    if fill_only { return append(result,path,item,page,item.style.rgba,count); }
    if let Some(fill)=item.style.fill { append(result,path.clone(),item,page,fill,count)?; }
    if item.style.width > 0.0 {
        let stroke=Stroke { width: coordinate(item.style.width)?, line_cap: tiny_skia::LineCap::Round, line_join: tiny_skia::LineJoin::Round, ..Default::default() };
        if item.style.screen_constant_width {
            let path=path.transform(transform(item.transform)?).ok_or(CoreError::Invalid)?;
            let outline=path.stroke(&stroke,1.0).ok_or(CoreError::Unsupported)?;
            let mut translated=item.clone(); translated.transform=IDENTITY;
            append(result,outline,&translated,page,item.style.rgba,count)?;
        } else { append(result,path.stroke(&stroke,1.0).ok_or(CoreError::Unsupported)?,item,page,item.style.rgba,count)?; }
    }
    Ok(())
}
pub(super) fn outlines(items: &[RenderItem], page: u32) -> Result<Vec<EditableOverlay>> {
    let mut result=Vec::new(); let mut count=0;
    for item in items {
        if matches!(item.shape,DrawShape::Guide { .. }) { continue; }
        if item.layer_blend != "normal" { return Err(CoreError::Unsupported); }
        match &item.shape {
            DrawShape::Stroke { .. } => {
                let mut path=PathBuilder::new(); let mut start=0;
                for end in &item.stroke_contours.ends {
                    let end=*end as usize; if end>item.stroke_contours.x.len() || end>item.stroke_contours.y.len() || end<start+3 { return Err(CoreError::Invalid); }
                    for i in start..end { let x=item.stroke_contours.x[i] as f64/256.0; let y=item.stroke_contours.y[i] as f64/256.0; if i==start { path.move_to(coordinate(x)?,coordinate(y)?); } else { path.line_to(coordinate(x)?,coordinate(y)?); } } path.close();start=end;
                }
                if let Some(path)=path.finish() { draw(&mut result,path,item,page,true,&mut count)?; }
            }
            DrawShape::Line { points } | DrawShape::Arrow { points } => {
                let mut stroke=item.clone();stroke.style.fill=None;draw(&mut result,polyline(points,false)?,&stroke,page,false,&mut count)?;
                if matches!(item.shape,DrawShape::Arrow { .. }) {
                    let end=points.last().ok_or(CoreError::Invalid)?; let before=points.iter().rev().skip(1).find(|p| p.x!=end.x || p.y!=end.y).ok_or(CoreError::Invalid)?;
                    let dx=end.x-before.x;let dy=end.y-before.y;let length=(dx*dx+dy*dy).sqrt();let size=(item.style.width*4.0).max(8.0);let ux=dx/length;let uy=dy/length;
                    draw(&mut result,polyline(&[*end,Point{x:end.x-ux*size-uy*size*0.45,y:end.y-uy*size+ux*size*0.45},Point{x:end.x-ux*size+uy*size*0.45,y:end.y-uy*size-ux*size*0.45}],true)?,item,page,true,&mut count)?;
                }
            }
            DrawShape::Rectangle { rectangle: r } => draw(&mut result,PathBuilder::from_rect(rectangle(*r)?),item,page,false,&mut count)?,
            DrawShape::Ellipse { rectangle: r } => { let mut b=PathBuilder::new();b.push_oval(rectangle(*r)?);draw(&mut result,b.finish().ok_or(CoreError::Invalid)?,item,page,false,&mut count)?; }
            DrawShape::Polygon { points, closed } => draw(&mut result,polyline(points,*closed)?,item,page,false,&mut count)?,
            DrawShape::Text { anchor, outline, .. } => { if !outline.is_empty() { draw(&mut result,text(outline,*anchor)?,item,page,true,&mut count)?; } }
            DrawShape::Marker { number, point, rectangle: r } => {
                let radius=(item.style.width*4.0).max(12.0);let mut b=PathBuilder::new();b.push_circle(coordinate(point.x)?,coordinate(point.y)?,coordinate(radius)?);draw(&mut result,b.finish().ok_or(CoreError::Invalid)?,item,page,false,&mut count)?;
                if let Some(r)=r { draw(&mut result,PathBuilder::from_rect(rectangle(*r)?),item,page,false,&mut count)?; }
                // Marker glyph generation uses the same positioned raster API.
                let layout=vw_raster::layout_text(&number.to_string(),vw_raster::FontFamily::Inter,(radius*1.15) as f32)?;
                let mut b=PathBuilder::new();for glyph in layout.glyphs { for command in glyph.outline { use vw_raster::OutlineCommand as O; match command { O::Move(x,y)=>b.move_to(x,y),O::Line(x,y)=>b.line_to(x,y),O::Quad(x1,y1,x,y)=>b.quad_to(x1,y1,x,y),O::Cubic(x1,y1,x2,y2,x,y)=>b.cubic_to(x1,y1,x2,y2,x,y),O::Close=>b.close() } } }
                if let Some(path)=b.finish() { let path=path.transform(tiny_skia::Transform::from_translate(coordinate(point.x)?-layout.width/2.0,coordinate(point.y)?+(radius*1.15) as f32*0.35)).ok_or(CoreError::Invalid)?;draw(&mut result,path,item,page,true,&mut count)?; }
            }
            DrawShape::Guide { .. } => {}, DrawShape::Result { .. } | DrawShape::Adjustment => return Err(CoreError::Unsupported),
        }
    }
    Ok(result)
}
