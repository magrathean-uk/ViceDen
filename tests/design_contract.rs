use std::path::PathBuf;

fn asset(name: &str) -> String {
    let asset_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static/assets").join(name);
    std::fs::read_to_string(asset_path).expect("asset should exist")
}

#[test]
fn design_additions_keep_dark_only_and_reduced_motion_contract() {
    let css = asset("viceden-motion.css");

    assert!(css.contains("prefers-reduced-motion"));
    assert!(css.contains(".mention"));
    assert!(css.contains("translate3d"));
    assert!(css.contains("content-visibility: auto"));
    assert!(css.contains("contain-intrinsic-size"));
    assert!(!css.contains("color-scheme: light"));
    assert!(!css.contains("@import"), "avoid remote CSS imports");
    assert!(!css.contains("infinite"), "avoid continuous motion");
}

#[test]
fn static_css_assets_own_shell_visual_contracts() {
    for name in ["viceden-app.css", "viceden-public.css", "viceden-auth.css"] {
        let css = asset(name);
        assert!(css.contains("color-scheme: dark"), "{name} should stay dark-only");
        assert!(css.contains("--radius"), "{name} should expose radius tokens");
        assert!(css.contains("--radius-sm: 3px"), "{name} should use square small radius");
        assert!(css.contains("--radius: 4px"), "{name} should use square default radius");
        assert!(css.contains("--shadow"), "{name} should expose shadow tokens");
        assert!(!css.contains("color-scheme: light"), "{name} must not add light mode");
        assert!(!css.contains("@import"), "{name} must not import remote CSS");
        assert!(!css.contains("letter-spacing: -"), "{name} must not use negative tracking");
        assert!(!css.contains("border-radius: 8px"), "{name} must not use soft 8px corners");
        assert!(!css.contains("border-radius: 10px"), "{name} must not use soft 10px corners");
        assert!(!css.contains("border-radius: 999px"), "{name} must not use pill corners");
    }
}

#[test]
fn shell_templates_link_static_css_without_inline_style_blocks() {
    let template_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
    for (name, css) in [
        ("base.html", "viceden-app.css"),
        ("public-lite.html", "viceden-public.css"),
        ("auth-base.html", "viceden-auth.css"),
    ] {
        let html = std::fs::read_to_string(template_dir.join(name)).expect("template should read");
        assert!(!html.contains("<style"), "{name} must not inline shell CSS");
        assert!(html.contains(&format!("/assets/{css}")), "{name} must link shell CSS");
        assert!(html.contains("/assets/viceden-motion.css"), "{name} must link motion CSS");
    }

    let base_html = std::fs::read_to_string(template_dir.join("base.html")).expect("base template");
    assert!(!base_html.contains("<style"), "density must not reintroduce inline CSS");
    assert!(!base_html.contains("style=\""), "density must stay class/data driven");
}

#[test]
fn thread_actions_stay_compact_and_not_pill_shaped() {
    let css = asset("viceden-app.css");
    let marker = ".thread-actions .action-compact,\n.thread-post-actions .action-compact {";
    let start = css.find(marker).expect("thread action block should exist");
    let block = &css[start..css[start..].find('}').map(|offset| start + offset).unwrap()];

    assert!(block.contains("height: 23px"));
    assert!(block.contains("font-size: 0.71875rem"));
    assert!(block.contains("border-radius: var(--radius-sm)"));
    assert!(!block.contains("border-radius: 999px"));
}

#[test]
fn thread_summary_badges_stay_compact_and_square() {
    let css = asset("viceden-app.css");
    let marker = ".badge-text,\n.chip {";
    let start = css.find(marker).expect("badge block should exist");
    let block = &css[start..css[start..].find('}').map(|offset| start + offset).unwrap()];

    assert!(block.contains("padding: 2px var(--space-3)"));
    assert!(block.contains("border-radius: var(--radius-sm)"));
    assert!(block.contains("text-transform: none"));
    assert!(!block.contains("border-radius: 999px"));
}

#[test]
fn profile_hides_density_settings_for_compact_app_shell() {
    let template_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
    let profile = std::fs::read_to_string(template_dir.join("profile.html")).expect("profile");
    let app_css = asset("viceden-app.css");

    assert!(!profile.contains(">Settings<"));
    assert!(!profile.contains(">Density<"));
    assert!(!profile.contains("id=\"compactness\""));
    assert!(!profile.contains("compactness_label"));
    assert!(!app_css.contains("data-density"));
}

#[test]
fn external_ui_asset_owns_mention_insertion() {
    let asset_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static/assets/viceden-ui.js");
    let js = std::fs::read_to_string(asset_path).expect("ui js should exist");

    assert!(js.contains("mention-insert"));
    assert!(js.contains("local-datetime"));
    assert!(!js.contains("eval("));
    assert!(!js.contains("innerHTML"));
}

#[test]
fn shell_does_not_load_external_assets() {
    for dir in ["templates", "static/assets"] {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(dir);
        for entry in std::fs::read_dir(root).expect("dir should read") {
            let path = entry.expect("entry should read").path();
            if !path.is_file() {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("file should read");
            for pattern in ["src=\"http://", "src=\"https://", "href=\"http://", "href=\"https://"]
            {
                assert!(!text.contains(pattern), "{} must not load remote assets", path.display());
            }
        }
    }
}

#[test]
fn app_shell_keeps_keyboard_and_screen_reader_contracts() {
    let template_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates");
    let base = std::fs::read_to_string(template_dir.join("base.html")).expect("base template");
    let board = std::fs::read_to_string(template_dir.join("board.html")).expect("board template");
    let thread =
        std::fs::read_to_string(template_dir.join("thread.html")).expect("thread template");
    let css = asset("viceden-app.css");
    let js = asset("viceden-ui.js");

    assert!(base.contains("aria-label=\"Primary navigation\""));
    assert!(base.contains("aria-label=\"Forum content\""));
    assert!(base.contains("aria-label=\"Open navigation\""));
    assert!(base.contains("aria-label=\"Notifications\""));
    assert!(base.contains("aria-label=\"Search threads\""));
    assert!(base.contains("data-drawer-toggle"));
    assert!(js.contains("event.key === \"Escape\""));

    assert!(board.contains("aria-label=\"Board sorts\""));
    assert!(thread.contains("aria-label=\"Report thread\""));
    assert!(thread.contains("aria-label=\"Report post\""));
    assert!(thread.contains("aria-pressed="));

    assert!(css.contains(":focus-visible"));
    assert!(css.contains("outline:"));
}

#[test]
fn mobile_shell_contract_keeps_drawer_and_forms_usable() {
    let css = asset("viceden-app.css");
    let public_css = asset("viceden-public.css");
    let auth_css = asset("viceden-auth.css");

    assert!(css.contains("@media (max-width: 720px)"));
    assert!(css.contains(".topbar-hamburger"));
    assert!(css.contains("body[data-drawer-open] .app-sidebar"));
    assert!(css.contains("body[data-drawer-open]::after"));
    assert!(css.contains(".app-inspector"));
    assert!(css.contains("display: none"));

    for (name, text) in [
        ("viceden-app.css", css.as_str()),
        ("viceden-public.css", public_css.as_str()),
        ("viceden-auth.css", auth_css.as_str()),
    ] {
        assert!(
            text.contains("font-size: 16px") || text.contains("font-size: 1rem"),
            "{name} must keep mobile form controls at 16px or larger"
        );
    }
}
