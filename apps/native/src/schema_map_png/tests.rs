use super::*;
fn encode(svg: &str, width: u32, height: u32) -> Result<Vec<u8>, PngError> {
    PngPlan::for_size(width, height, svg.len())?
        .with_working_limit(MAX_WORKING_BYTES)?
        .encode(svg.as_bytes().to_vec(), &Cancellation::default())
}
#[test]
fn exact_two_times_dimensions_and_rgba_colours_on_white() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="1"><rect width="4" height="1" fill="white"/><rect width="1" height="1" fill="#ff0000"/><rect x="1" width="1" height="1" fill="#0000ff"/><rect x="2" width="1" height="1" fill="#00ff00"/></svg>"##;
    let bytes = encode(svg, 4, 1).unwrap();
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().unwrap();
    let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
    let info = reader.next_frame(&mut pixels).unwrap();
    assert_eq!((info.width, info.height), (8, 2));
    assert_eq!(info.color_type, png::ColorType::Rgba);
    assert_eq!(info.bit_depth, png::BitDepth::Eight);
    assert_eq!(&pixels[0..4], &[255, 0, 0, 255]);
    assert_eq!(&pixels[8..12], &[0, 0, 255, 255]);
    assert_eq!(&pixels[16..20], &[0, 255, 0, 255]);
    assert_eq!(&pixels[24..28], &[255, 255, 255, 255]);
}
#[test]
fn ordinary_viewport_keeps_two_times_scale_and_refuses_extremes() {
    let plan = PngPlan::for_size(1440, 900, 1024).unwrap();
    assert_eq!(plan.dimensions(), (2880, 1800));
    assert!(plan.required_bytes() < MAX_WORKING_BYTES);
    for (w, h) in [(0, 1), (1, 0), (u32::MAX, 2), (4097, 1), (2000, 2000)] {
        assert!(PngPlan::for_size(w, h, 1).is_err());
    }
    assert!(PngPlan::for_size(1, 1, MAX_BYTES + 1).is_err());
    assert!(PngPlan::for_size(1, 1, MAX_BYTES).is_err());
    assert!(plan.with_working_limit(plan.required_bytes() - 1).is_err());
    assert!(plan.with_working_limit(MAX_WORKING_BYTES + 1).is_err());
}
#[test]
fn clipped_layer_is_measured_before_raster_allocation_and_not_just_canvas() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20"><clipPath id="c"><rect width="1" height="1"/></clipPath><g clip-path="url(#c)"><rect width="100" height="100" fill="red"/></g></svg>"##;
    let tree = tree::parse(svg.as_bytes(), (20, 20), &Cancellation::default()).unwrap();
    let extra = tree::raster_working(&tree, (40, 40)).unwrap();
    assert!(extra >= 9 * 200 * 200);
    let plan = PngPlan::for_size(20, 20, svg.len()).unwrap();
    assert_eq!(
        plan.encode(svg.as_bytes().to_vec(), &Cancellation::default())
            .unwrap_err(),
        PngError::WorkingLimit
    );
    assert!(encode(svg, 20, 20).is_ok());
}
#[test]
fn nested_layers_accumulate_parent_storage_and_reject_unsupported_features() {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><g opacity="0.5"><g opacity="0.5"><rect width="10" height="10"/></g></g></svg>"##;
    let tree = tree::parse(svg.as_bytes(), (10, 10), &Cancellation::default()).unwrap();
    assert!(tree::raster_working(&tree, (20, 20)).unwrap() >= 2 * 4 * 24 * 24);
    let image = r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><image href="file:///private/never-read"/></svg>"#;
    assert_eq!(encode(image, 10, 10).unwrap_err(), PngError::Unsupported);
    let filter =
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10"><filter id="f"/></svg>"#;
    assert_eq!(encode(filter, 10, 10).unwrap_err(), PngError::Unsupported);
}
#[test]
fn missing_bundled_glyph_is_a_visible_refusal() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="30"><text x="0" y="20" font-family="monospace">雪</text></svg>"#;
    assert_eq!(encode(svg, 100, 30).unwrap_err(), PngError::MissingGlyph);
}
#[test]
fn cancellation_and_changed_dimensions_refuse_without_encoding() {
    let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" width="2" height="2"/>"#;
    let token = Cancellation::default();
    token.cancel();
    let plan = PngPlan::for_size(2, 2, svg.len()).unwrap();
    assert_eq!(
        plan.encode(svg.as_bytes().to_vec(), &token).unwrap_err(),
        PngError::Cancelled
    );
    assert_eq!(encode(svg, 3, 2).unwrap_err(), PngError::Dimensions);
}
#[test]
fn output_cap_refuses_whole_chunk_before_appending() {
    let token = Cancellation::default();
    let mut output = Output {
        bytes: vec![0; MAX_BYTES - 1],
        cancel: &token,
        limited: false,
    };
    assert!(output.write_all(&[1, 2]).is_err());
    assert!(output.limited);
    assert_eq!(output.bytes.len(), MAX_BYTES - 1);
}
