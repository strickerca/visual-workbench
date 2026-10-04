mod common;
use common::*;
use proptest::prelude::*;
use vw_ai::{config::*, *};
use vw_raster::Pixels;

fn failure(error: impl std::fmt::Display) -> TestCaseError {
    TestCaseError::fail(error.to_string())
}
proptest! {
    #![proptest_config(ProptestConfig::with_cases(48))]

    #[test]
    fn crop_always_contains_every_selected_pixel_and_chooses_valid_size(
        width in 1u32..257,height in 1u32..257,x in any::<u32>(),y in any::<u32>(),wide in 1u32..80,tall in 1u32..80
    ) {
        let (x,y)=(x%width,y%height);let (right,bottom)=((x+wide).min(width),(y+tall).min(height));
        let mask=region(width,height,x,y,right,bottom).map_err(failure)?;
        let caps=provider().capabilities;let plan=geometry::plan(&mask.to_dense().map_err(failure)?,width,height,&caps).map_err(failure)?;
        prop_assert!(plan.crop.x<=x as i32&&plan.crop.y<=y as i32);
        prop_assert!(plan.crop.x+plan.crop.width as i32>=right as i32&&plan.crop.y+plan.crop.height as i32>=bottom as i32);
        prop_assert!(caps.valid_size(plan.model_width,plan.model_height));
        prop_assert!(u64::from(plan.crop.width.max(plan.crop.height))<=u64::from(plan.crop.width.min(plan.crop.height))*3);
    }

    #[test]
    fn quoted_price_equals_independent_integer_rational_charge(
        text in 0u64..1_000_000,image in 0u64..1_000_000,output in 0u64..1_000_000,rate in 1u64..100_000_000
    ) {
        let prices=Prices{text_input_microusd_per_million:rate,image_input_microusd_per_million:rate+1,image_output_microusd_per_million:rate+2};
        let numerator=u128::from(text)*u128::from(rate)+u128::from(image)*u128::from(rate+1)+u128::from(output)*u128::from(rate+2);
        let expected=(numerator/1_000_000+u128::from(numerator%1_000_000!=0))as u64;
        prop_assert_eq!(prices.cost(Tokens{text_input:text,image_input:image,image_output:output}).map_err(failure)?,expected);
    }

    #[test]
    fn zero_feather_composite_is_exact_source_outside_and_full_result_inside(
        x in 0u32..31,y in 0u32..31,red in any::<u8>(),green in any::<u8>(),blue in any::<u8>(),alpha in 1u8..=255
    ) {
        let source=png8(32,32,|xx,yy|[(xx*7)as u8,(yy*7)as u8,110,123]).map_err(failure)?;
        let mask=region(32,32,x,y,x+1,y+1).map_err(failure)?;let mut opt=options(1).map_err(failure)?;opt.feather_px=0;
        let p=Prepared::new(&source,&[mask],opt,&NeverCancel).map_err(failure)?;
        let done=p.finish(p.mock_response([red,green,blue,alpha]).map_err(failure)?,&NeverCancel).map_err(failure)?;
        let (Pixels::Rgba8(a),Pixels::Rgba8(b))=(p.source().pixels(),done.image().pixels())else{return Err(TestCaseError::fail("8-bit required"))};
        for yy in 0..32{for xx in 0..32{let i=(yy*32+xx)as usize*4;if xx!=x||yy!=y{prop_assert_eq!(&a[i..i+4],&b[i..i+4]);}else{
            for (got,want)in b[i..i+4].iter().zip([red,green,blue,alpha]){prop_assert!(got.abs_diff(want)<=1);}
        }}}
        prop_assert_eq!(done.proof().changed_outside,0);
    }

    #[test]
    fn change_union_order_does_not_change_binding_or_payload(x in 0u32..24,y in 0u32..24){
        let source=png8(32,32,|_,_|[100,120,140,255]).map_err(failure)?;
        let a=region(32,32,x,y,x+5,y+5).map_err(failure)?;let b=region(32,32,8,8,24,24).map_err(failure)?;
        let p=Prepared::new(&source,&[a.clone(),b.clone()],options(1).map_err(failure)?,&NeverCancel).map_err(failure)?;
        let q=Prepared::new(&source,&[b,a],options(1).map_err(failure)?,&NeverCancel).map_err(failure)?;
        prop_assert_eq!(p.review().request_id(),q.review().request_id());prop_assert_eq!(p.request_mask_png(),q.request_mask_png());
    }
}
