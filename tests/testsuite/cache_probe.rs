use std::time::{Duration, Instant};

use crate::prelude::*;
use crate::utils::cargo_exe;
use cargo::GlobalContext;
use cargo::compiler::UserIntent;
use cargo::ops::CompileOptions;
use cargo::workspace::Workspace;
use cargo_test_support::paths::CargoPathExt;
use cargo_test_support::{paths, project, str};
use cargo_util_terminal::Shell;

#[cargo_test]
fn freshness_matches_build_across_reused_metadata() {
    let project = project()
        .file("src/lib.rs", "pub fn value() -> u32 { 1 }")
        .build();

    project.cargo("build").run();
    project
        .cargo("build")
        .with_stderr_data(str![[r#"
[FINISHED] `dev` profile [unoptimized + debuginfo] target(s) in [ELAPSED]s

"#]])
        .run();

    let empty = project.root().join("empty-cache");
    empty.mkdir_p();
    let candidates = [project.build_dir(), project.build_dir(), empty];
    let scores = probe_scores(&project, &candidates);
    assert_eq!(scores.len(), 3);
    assert_eq!(scores[0].fresh_units, scores[0].total_units);
    assert_eq!(scores[1].fresh_units, scores[0].fresh_units);
    assert_eq!(scores[1].total_units, scores[0].total_units);
    assert_eq!(scores[2].fresh_units, 0);
    assert_eq!(scores[2].total_units, scores[0].total_units);

    project.change_file("src/lib.rs", "pub fn value() -> u32 { 2 }");
    let scores = probe_scores(&project, &candidates[..2]);
    assert_eq!(scores[0].fresh_units, 0);
    assert_eq!(scores[1].fresh_units, scores[0].fresh_units);
    assert_eq!(scores[1].total_units, scores[0].total_units);
    project
        .cargo("build")
        .with_stderr_data(str![[r#"
[COMPILING] foo v0.0.1 ([ROOT]/foo)
[FINISHED] `dev` profile [unoptimized + debuginfo] target(s) in [ELAPSED]s

"#]])
        .run();
}

fn probe_scores(
    project: &cargo_test_support::Project,
    candidates: &[std::path::PathBuf],
) -> Vec<cargo::cache_probe::CandidateScore> {
    let shell = Shell::from_write(Box::new(Vec::new()));
    let mut gctx = GlobalContext::new(shell, project.root(), paths::home());
    gctx.set_cargo_exe(cargo_exe());
    gctx.configure(
        0,
        true,
        Some("never"),
        true,
        true,
        true,
        &Some(project.build_dir()),
        &[],
        &[],
    )
    .unwrap();
    let workspace = Workspace::new(&project.root().join("Cargo.toml"), &gctx).unwrap();
    let options = CompileOptions::new(&gctx, UserIntent::Build).unwrap();
    cargo::cache_probe::probe(
        &workspace,
        &options,
        candidates,
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap()
}
