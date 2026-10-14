use wasm_bindgen::JsCast;

/// User-initiated fallback for HTTP VPN hosts where navigator.clipboard is absent.
pub fn copy_on_http(text: &str) -> bool {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return false;
    };
    let Some(body) = document.body() else {
        return false;
    };
    let Ok(element) = document.create_element("textarea") else {
        return false;
    };
    let Ok(area) = element.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return false;
    };
    area.set_value(text);
    area.set_read_only(true);
    let _ = area.set_attribute(
        "style",
        "position:fixed;top:0;left:0;opacity:0;pointer-events:none;font-size:16px",
    );
    let _ = area.set_attribute("aria-hidden", "true");
    let previous = document.active_element();
    if body.append_child(&area).is_err() {
        return false;
    }
    let _ = area.focus();
    area.select();
    let _ = area.set_selection_range(0, text.encode_utf16().count() as u32);
    let copied = js_sys::Reflect::get(document.as_ref(), &"execCommand".into())
        .ok()
        .and_then(|f| f.dyn_into::<js_sys::Function>().ok())
        .and_then(|f| f.call1(document.as_ref(), &"copy".into()).ok())
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    // The text may be a secret: empty the field before it leaves the DOM.
    area.set_value("");
    area.remove();
    if let Some(previous) = previous.and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok()) {
        let _ = previous.focus();
    }
    copied
}
