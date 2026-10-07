use parley::{
    FontContext, FontFamily, FontFeatures, Layout, LayoutContext, LineHeight, OverflowWrap,
    StyleProperty,
};

pub const FONT: &[u8] = include_bytes!(env!("OPENWEBIDE_LAYOUT_FONT"));
pub const FEATURES: &str = "\"calt\" 1, \"liga\" 1, \"ss01\" 1, \"ss02\" 1, \"ss03\" 1, \"ss04\" 1, \"ss05\" 1, \"ss06\" 1, \"ss07\" 1, \"ss08\" 1, \"ss09\" 1, \"ss10\" 1";
pub fn shape(text: &str, wrap: Option<f32>, features: &str) -> Layout<[u8; 4]> {
    let mut fonts = FontContext::new();
    let loaded = fonts.collection.register_fonts(FONT.to_vec().into(), None);
    assert_eq!(loaded.len(), 1);
    let family = fonts
        .collection
        .family_name(loaded[0].0)
        .unwrap()
        .to_string();
    let mut context = LayoutContext::<[u8; 4]>::new();
    let mut builder = context.ranged_builder(&mut fonts, text, 1.0, false);
    builder.push_default(StyleProperty::FontFamily(FontFamily::named(&family)));
    builder.push_default(StyleProperty::FontSize(13.0));
    builder.push_default(StyleProperty::FontFeatures(FontFeatures::from(features)));
    builder.push_default(StyleProperty::LineHeight(LineHeight::Absolute(19.5)));
    builder.push_default(StyleProperty::OverflowWrap(OverflowWrap::Anywhere));
    let mut result = builder.build(text);
    result.break_all_lines(wrap);
    result
}
