#![no_main]

use libfuzzer_sys::fuzz_target;

fuzz_target!(|input: &str| {
    let html = viceden::services::safe_html::render_user_html(input);

    assert!(!html.contains("<script"));
    assert!(!html.contains("<iframe"));
    assert!(!html.contains("<object"));
    assert!(!html.contains("<a href=\"javascript:"));
    assert!(!html.contains("<a href='javascript:"));
});
