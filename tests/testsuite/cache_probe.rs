use std::time::{Duration, Instant};

use crate::prelude::*;
use crate::utils::cargo_exe;
use cargo::GlobalContext;
use cargo::compiler::UserIntent;
use cargo::ops::CompileOptions;
use cargo::workspace::Workspace;
use cargo_test_support::paths::CargoPathExt;
use cargo_test_support::{paths, project};
use cargo_util_terminal::Shell;

#[cargo_test]
fn reused_metadata_keeps_candidate_scores_stable() {
    let project = project()
        .file("src/lib.rs", "pub fn value() -> u32 { 1 }")
        .build();

    project.cargo("generate-lockfile").run();
    compile_project(&project);

    let empty = project.root().join("empty-cache");
    empty.mkdir_p();
    let fresh = project.build_dir();
    let independent_fresh = probe_scores(&project, std::slice::from_ref(&fresh));
    assert!(independent_fresh[0].total_units > 0);
    assert_eq!(
        independent_fresh[0].fresh_units,
        independent_fresh[0].total_units
    );

    let candidates = [fresh.clone(), empty, fresh.clone()];
    let scores = probe_scores(&project, &candidates);
    assert_eq!(scores.len(), 3);
    assert_eq!(scores[0].fresh_units, independent_fresh[0].fresh_units);
    assert_eq!(scores[0].total_units, independent_fresh[0].total_units);
    assert_eq!(scores[1].fresh_units, 0);
    assert_eq!(scores[1].total_units, independent_fresh[0].total_units);
    assert_eq!(scores[2].fresh_units, independent_fresh[0].fresh_units);
    assert_eq!(scores[2].total_units, independent_fresh[0].total_units);

    project.change_file("src/lib.rs", "pub fn value() -> u32 { 2 }");
    let independent_dirty = probe_scores(&project, std::slice::from_ref(&fresh));
    let scores = probe_scores(&project, &[fresh.clone(), fresh]);
    assert_eq!(independent_dirty[0].fresh_units, 0);
    assert_eq!(scores[0].fresh_units, 0);
    assert_eq!(scores[0].fresh_units, independent_dirty[0].fresh_units);
    assert_eq!(scores[0].total_units, independent_dirty[0].total_units);
    assert_eq!(scores[1].fresh_units, independent_dirty[0].fresh_units);
    assert_eq!(scores[1].total_units, independent_dirty[0].total_units);
}

fn compile_project(project: &cargo_test_support::Project) {
    let gctx = probe_context(project);
    let workspace = Workspace::new(&project.root().join("Cargo.toml"), &gctx).unwrap();
    let options = CompileOptions::new(&gctx, UserIntent::Build).unwrap();
    cargo::ops::compile(&workspace, &options).unwrap();
}

fn probe_scores(
    project: &cargo_test_support::Project,
    candidates: &[std::path::PathBuf],
) -> Vec<cargo::cache_probe::CandidateScore> {
    let gctx = probe_context(project);
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

fn probe_context(project: &cargo_test_support::Project) -> GlobalContext {
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
    gctx
}
