//! `FEATURES` is the source of truth and the operator tables are hand-maintained mirrors with no compile-time check of their own.
//! This test is that check.

use std::path::PathBuf;

use xai_grok_shell::agent::config::FEATURES;

#[test]
fn every_registered_feature_reaches_the_operator() {
    let internal = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("docs/internal");
    let enterprise_path = internal.join("25-enterprise.md");
    let env_vars_path = internal.join("22-environment-variables.md");
    // This public tree does not ship the internal operator docs.
    if !enterprise_path.is_file() || !env_vars_path.is_file() {
        return;
    }
    let enterprise = std::fs::read_to_string(enterprise_path).unwrap();
    let env_vars = std::fs::read_to_string(env_vars_path).unwrap();

    for spec in FEATURES {
        assert!(
            enterprise.contains(&format!("`{}`", spec.key)),
            "{} has no row in the 25-enterprise.md pinning table",
            spec.key,
        );
        assert!(
            env_vars.contains(&format!("`{}`", spec.env)),
            "{} is undocumented in 22-environment-variables.md",
            spec.env,
        );
    }
}
