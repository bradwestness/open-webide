#![cfg(target_arch = "wasm32")]
use openwebide_editor_layout_probe::{FEATURES, FONT, shape};
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;
wasm_bindgen_test_configure!(run_in_browser);
#[wasm_bindgen(inline_js = r#"
export async function installProbeFont(bytes) {
 const font = await new FontFace('LayoutProbe', bytes.buffer).load();
 document.fonts.add(font);
 for (const [name, features] of [['LayoutProbeOn', '"calt" 1, "liga" 1, "ss01" 1, "ss02" 1, "ss03" 1, "ss04" 1, "ss05" 1, "ss06" 1, "ss07" 1, "ss08" 1, "ss09" 1, "ss10" 1'], ['LayoutProbeOff', '"calt" 0, "liga" 0']]) {
  document.fonts.add(await new FontFace(name, bytes.buffer, {featureSettings:features}).load());
 }
}
export function canvasSupport() {
 const context = new OffscreenCanvas(400,40).getContext('2d');
 context.font='13px LayoutProbeOn';
 const metrics=context.measureText('word -> value ===');
 const pixels=[];
 for (const name of ['LayoutProbeOn','LayoutProbeOff']) {
  context.clearRect(0,0,400,40);
  context.font=`13px ${name}`;
  context.fillText('word -> value === != <=>',0,20);
  pixels.push(context.getImageData(0,0,400,40).data);
 }
 return {index_from_offset:typeof metrics.getIndexFromOffset==='function',
         text_clusters:typeof metrics.getTextClusters==='function',
         selection_rects:typeof metrics.getSelectionRects==='function',
         context_font_features:'fontFeatureSettings' in context,
         face_feature_pixels_differ:pixels[0].some((value,index)=>value!==pixels[1][index])};
}
export function canvasWidth(text, on) {
 const context=new OffscreenCanvas(1,1).getContext('2d');
 context.font=`13px ${on ? 'LayoutProbeOn' : 'LayoutProbeOff'}`;
 return context.measureText(text).width;
}
export function domLayout(text, wrap, features) {
 const row=document.createElement('div');
 row.style.cssText=`position:fixed;left:-10000px;top:0;font:13px LayoutProbe;line-height:19.5px;tab-size:4;white-space:${wrap ? 'pre-wrap' : 'pre'};overflow-wrap:anywhere;width:${wrap ? wrap+'px' : 'max-content'};font-feature-settings:${features}`;
 row.textContent=text;
 document.body.append(row);
 const before=performance.now();
 const rect=row.getBoundingClientRect();
 const result={width:rect.width,height:rect.height,ms:performance.now()-before};
 row.remove();
 return result;
}
"#)]
extern "C" {
    #[wasm_bindgen(catch)]
    async fn installProbeFont(bytes: js_sys::Uint8Array) -> Result<(), JsValue>;
    fn canvasSupport() -> JsValue;
    fn canvasWidth(text: &str, on: bool) -> f64;
    fn domLayout(text: &str, wrap: f32, features: &str) -> JsValue;
}
#[wasm_bindgen_test]
async fn compare_geometry() {
    installProbeFont(js_sys::Uint8Array::from(FONT))
        .await
        .unwrap();
    console_log!(
        "{}",
        serde_json::json!({"browser": js_sys::Reflect::get(&js_sys::global(), &"navigator".into()).and_then(|n| js_sys::Reflect::get(&n, &"userAgent".into())).unwrap().as_string().unwrap()})
    );
    console_log!("{}", js_sys::JSON::stringify(&canvasSupport()).unwrap());
    for (name, text) in [
        ("ascii", "fn main() { let value = 123; }".into()),
        ("tab", "a\tb\tc".into()),
        ("unicode", "e\u{301} 文🦀 mixed".into()),
        ("bidi", "LTR אבג 123 مرحبا text".into()),
        ("long", "word -> value === ".repeat(55000)),
    ] {
        for wrap in [None, Some(300.0)] {
            for features in [FEATURES, "\"calt\" 0, \"liga\" 0"] {
                let before = js_sys::Date::now();
                let layout = shape(&text, wrap, features);
                let rust_ms = js_sys::Date::now() - before;
                let dom = domLayout(&text, wrap.unwrap_or(0.0), features);
                let field = |name: &str| {
                    js_sys::Reflect::get(&dom, &name.into())
                        .unwrap()
                        .as_f64()
                        .unwrap()
                };
                let dom_width = field("width");
                let dom_height = field("height");
                console_log!(
                    "{}",
                    serde_json::json!({"case":name,"bytes":text.len(),"wrap":wrap,"features":features==FEATURES,"rust_ms":rust_ms,"rust_width":layout.width(),"rust_height":layout.height(),"rust_lines":layout.lines().len(),"canvas_width":canvasWidth(&text, features==FEATURES),"dom_width":dom_width,"dom_height":dom_height,"dom_ms":field("ms")})
                );
                assert!(dom_width > 0.0 && dom_height > 0.0 && layout.width() > 0.0);
            }
        }
    }
}
