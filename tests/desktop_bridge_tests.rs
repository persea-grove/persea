//! Desktop bridge config tests (S07 wiring): the `[desktop] allow_bridge`
//! flag defaults off, round-trips through the startup mirror, and parses
//! from a TOML config file.

use persea::config::{allow_bridge_enabled, init_allow_bridge, Config, DesktopConfig};

#[test]
fn desktop_config_defaults_bridge_off() {
    assert!(!DesktopConfig::default().allow_bridge);
}

#[test]
fn allow_bridge_startup_mirror_round_trip() {
    init_allow_bridge(true);
    assert!(allow_bridge_enabled());
}

#[test]
fn config_toml_allow_bridge_parses() {
    let tmp = tempfile::tempdir().expect("temp dir");
    let path = tmp.path().join("config.toml");
    std::fs::write(&path, "[desktop]\nallow_bridge = true\n").unwrap();
    let cfg = Config::load(Some(path.to_str().unwrap()));
    assert!(cfg
        .desktop
        .as_ref()
        .map(|d| d.allow_bridge)
        .unwrap_or(false));
}
