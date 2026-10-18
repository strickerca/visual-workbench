use vw_ffi::*;
fn camera(scale:f64,rotation:f64)->Camera { Camera {center:Point{x:-73.5,y:991.25},scale,rotation,viewport_width:1280.0,viewport_height:900.0} }
fn bits(t:&Transform)->[u64;6] {[t.a.to_bits(),t.b.to_bits(),t.c.to_bits(),t.d.to_bits(),t.e.to_bits(),t.f.to_bits()]}
fn point_close(a:&Point,b:&Point){assert!((a.x-b.x).abs()<=1e-6,"x");assert!((a.y-b.y).abs()<=1e-6,"y");}
fn apply(t:&Transform,p:&Point)->Point {Point{x:t.a*p.x+t.c*p.y+t.e,y:t.b*p.x+t.d*p.y+t.f}}
#[test]fn closing_restores_the_exact_saved_matrix_at_every_scale_and_rotation(){
    for scale in [1.0/64.0,0.75,1.0,13.0,64.0] {for rotation in [-3.1,-0.5,0.0,1.7,3.1]{
        let c=camera(scale,rotation);let lens=create_precision_lens(c.clone()).unwrap();let before=camera_matrix(c,false).unwrap();
        for mag in [2,4,8]{lens.configure(Point{x:20.0,y:30.0},Point{x:500.0,y:250.0},mag).unwrap();lens.close_lens().unwrap();assert_eq!(bits(&before),bits(&lens.mapping().unwrap().lens));}
    }}
}
#[test]fn the_focus_maps_to_inset_center_and_native_samples_return_to_document(){
    let c=camera(1.3,0.74);let lens=create_precision_lens(c.clone()).unwrap();let focus=Point{x:100.5,y:20.25};
    for mag in [2,4,8]{let center=Point{x:450.0,y:340.0};lens.configure(focus.clone(),center.clone(),mag).unwrap();
        point_close(&apply(&lens.mapping().unwrap().lens,&focus),&center);
        lens.down(true).unwrap();let mapped=lens.map_to_base(vec![center],false).unwrap();let doc=camera_map_points(c.clone(),true,mapped).unwrap();point_close(&doc[0],&focus);lens.up().unwrap();
    }
}
#[test]fn two_hundred_midstroke_toggle_traces_have_one_unbroken_mapping(){
    for trace in 0..200 {
        let c=camera(0.25+f64::from(trace%40)/7.0,-3.0+f64::from(trace)/37.0);let lens=create_precision_lens(c.clone()).unwrap();
        lens.configure(Point{x:13.0,y:40.0},Point{x:330.0,y:440.0},4).unwrap();lens.down(true).unwrap();
        let points:Vec<_>=(0..60).map(|i|Point{x:10.0+f64::from(i)*7.0,y:950.0-f64::from(i)*11.0}).collect();
        let expected=lens.map_to_base(points.clone(),true).unwrap();let mut actual=Vec::new();
        for (i,p) in points.into_iter().enumerate(){if i==17{lens.close_lens().unwrap();}if i==29{lens.configure(Point{x:300.0,y:50.0},Point{x:70.0,y:90.0},8).unwrap();}
            actual.extend(lens.map_to_base(vec![p],i%2==0).unwrap());}
        for (a,b) in actual.iter().zip(&expected){point_close(a,b);}
        assert!(lens.mapping().unwrap().latched);lens.up().unwrap();assert!(!lens.mapping().unwrap().latched);assert!(lens.mapping().unwrap().open);
    }
}
#[test]fn outside_down_stays_bit_identical_after_crossing_inset_and_queued_open(){
    let lens=create_precision_lens(camera(1.0,0.0)).unwrap();lens.down(false).unwrap();
    lens.configure(Point{x:5.0,y:6.0},Point{x:500.0,y:600.0},8).unwrap();
    let raw=vec![Point{x:-0.0,y:0.0},Point{x:1e-12,y:-999.125}];
    let got=lens.map_to_base(raw.clone(),true).unwrap();for(a,b)in raw.iter().zip(got){assert_eq!(a.x.to_bits(),b.x.to_bits());assert_eq!(a.y.to_bits(),b.y.to_bits());}
    lens.up().unwrap();assert!(lens.mapping().unwrap().open);
}
#[test]fn invalid_reconfiguration_is_atomic_and_does_not_replace_queued_close(){
    let lens=create_precision_lens(camera(1.0,0.3)).unwrap();lens.configure(Point{x:1.0,y:2.0},Point{x:100.0,y:200.0},4).unwrap();lens.down(true).unwrap();lens.close_lens().unwrap();
    assert!(lens.configure(Point{x:f64::NAN,y:0.0},Point{x:0.0,y:0.0},4).is_err());assert!(lens.configure(Point{x:1.0,y:2.0},Point{x:0.0,y:0.0},3).is_err());
    lens.up().unwrap();assert!(!lens.mapping().unwrap().open);
}
#[test]fn render_footprint_ignores_outside_contact_latch_without_changing_input(){
    let lens=create_precision_lens(camera(1.0,0.0)).unwrap();lens.configure(Point{x:0.0,y:0.0},Point{x:500.0,y:400.0},4).unwrap();lens.down(false).unwrap();
    let input=Point{x:500.0,y:400.0};let raw=lens.map_to_base(vec![input.clone()],true).unwrap();assert_eq!(raw[0].x,input.x);
    let view=lens.view_to_base(vec![input]).unwrap();assert_ne!(view[0].x,raw[0].x);assert!(!lens.mapping().unwrap().lens_input);
}
#[test]fn disposal_and_batch_bounds_refuse_without_changing_geometry(){
    let lens=create_precision_lens(camera(1.0,0.0)).unwrap();let old=bits(&lens.mapping().unwrap().lens);
    assert!(lens.map_to_base(vec![Point{x:0.0,y:0.0};513],false).is_err());assert!(lens.map_to_base(vec![Point{x:f64::INFINITY,y:0.0}],false).is_err());assert_eq!(old,bits(&lens.mapping().unwrap().lens));
    assert!(lens.up().is_err());lens.down(false).unwrap();assert!(lens.down(true).is_err());lens.dispose().unwrap();assert!(matches!(lens.mapping(),Err(CoreError::Closed)));lens.dispose().unwrap();
}
#[test]fn inspection_matrix_never_changes_the_editable_handle(){
    let c=camera(0.875,0.42);let lens=create_precision_lens(c.clone()).unwrap();lens.down(false).unwrap();let before=bits(&lens.mapping().unwrap().lens);
    for i in 0..100{precision_view_transform(c.clone(),Point{x:f64::from(i),y:7.0},Point{x:700.0,y:300.0},4).unwrap();}
    assert_eq!(before,bits(&lens.mapping().unwrap().lens));assert!(!lens.mapping().unwrap().open);
}
