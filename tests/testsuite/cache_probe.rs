use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::prelude::*;
use crate::utils::cargo_exe;
use cargo::GlobalContext;
use cargo::cache_probe::{CandidateScore, ProbeCompletion, ProbeFailure, ProbeResult};
use cargo::compiler::UserIntent;
use cargo::ops::CompileOptions;
use cargo::util::CargoResult;
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
    assert_eq!(
        independent_fresh.completion,
        ProbeCompletion::PerfectCandidate
    );
    assert!(independent_fresh.resolution_elapsed > Duration::ZERO);
    assert_eq!(independent_fresh.candidate_elapsed.len(), 1);
    assert!(independent_fresh.candidate_elapsed[0] > Duration::ZERO);
    let perfect = independent_fresh.scores[0];
    assert!(perfect.total_units > 0);
    assert!(perfect.is_perfect());

    let result = probe_scores(&project, &[empty.clone(), fresh.clone(), fresh.clone()]);
    assert_eq!(result.completion, ProbeCompletion::PerfectCandidate);
    assert_eq!(result.candidate_elapsed.len(), 2);
    assert_eq!(
        result.scores,
        [
            CandidateScore {
                fresh_units: 0,
                total_units: perfect.total_units,
            },
            perfect,
        ]
    );

    project.change_file("src/lib.rs", "pub fn value() -> u32 { 2 }");
    let independent_dirty = probe_scores(&project, std::slice::from_ref(&fresh));
    assert_eq!(independent_dirty.completion, ProbeCompletion::AllCandidates);
    let dirty = independent_dirty.scores[0];
    assert_eq!(dirty.fresh_units, 0);
    assert_eq!(dirty.total_units, perfect.total_units);
    let result = probe_scores(&project, &[fresh.clone(), empty, fresh]);
    assert_eq!(result.completion, ProbeCompletion::AllCandidates);
    assert_eq!(result.scores, [dirty, dirty, dirty]);
    assert_eq!(result.candidate_elapsed.len(), 3);
}

#[cargo_test]
fn invalid_later_candidate_fails_before_perfect_stop() {
    let project = project()
        .file("src/lib.rs", "pub fn value() -> u32 { 1 }")
        .build();

    project.cargo("generate-lockfile").run();
    compile_project(&project);

    let fresh = project.build_dir();
    let error = try_probe_scores(
        &project,
        &[fresh, PathBuf::from("relative-cache")],
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "invalid cache candidate");
    assert_eq!(
        error.downcast_ref::<ProbeFailure>(),
        Some(&ProbeFailure::Unsupported("invalid cache candidate"))
    );
}

#[cargo_test]
fn resolution_failure_keeps_underlying_error() {
    let project = project()
        .file(
            "Cargo.toml",
            r#"
                [package]
                name = "foo"
                version = "0.0.1"
                edition = "2015"

                [dependencies]
                missing-dependency = "1.0.0"
            "#,
        )
        .file("src/lib.rs", "")
        .build();

    let error = try_probe_scores(
        &project,
        &[project.build_dir()],
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(
        error.downcast_ref::<ProbeFailure>(),
        Some(&ProbeFailure::Resolution)
    );
    assert_eq!(error.to_string(), "cache probe resolution failed");
    assert!(
        error.chain().count() > 1,
        "underlying resolution error missing: {error:#}"
    );
}

#[cargo_test]
fn busy_package_cache_is_reported() {
    let project = project()
        .file("src/lib.rs", "pub fn value() -> u32 { 1 }")
        .build();

    project.cargo("generate-lockfile").run();
    compile_project(&project);

    let holder = probe_context(&project);
    let _exclusive = holder
        .acquire_package_cache_lock(cargo::util::cache_lock::CacheLockMode::DownloadExclusive)
        .unwrap();
    let error = try_probe_scores(
        &project,
        &[project.build_dir()],
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    assert_eq!(error.to_string(), "package cache is busy");
    assert_eq!(
        error.downcast_ref::<ProbeFailure>(),
        Some(&ProbeFailure::SourceCacheBusy)
    );
}

#[cargo_test]
fn forced_rebuild_scores_nothing_fresh() {
    let project = project()
        .file("src/lib.rs", "pub fn value() -> u32 { 1 }")
        .build();

    project.cargo("generate-lockfile").run();
    compile_project(&project);

    let gctx = probe_context(&project);
    let workspace = Workspace::new(&project.root().join("Cargo.toml"), &gctx).unwrap();
    let mut options = CompileOptions::new(&gctx, UserIntent::Build).unwrap();
    options.build_config.force_rebuild = true;
    let result = cargo::cache_probe::probe(
        &workspace,
        &options,
        &[project.build_dir()],
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap();
    assert_eq!(result.completion, ProbeCompletion::AllCandidates);
    assert_eq!(result.scores.len(), 1);
    assert_eq!(result.scores[0].fresh_units, 0);
    assert!(result.scores[0].total_units > 0);
}

#[cargo_test]
fn expired_deadline_without_completed_candidate_fails() {
    let project = project()
        .file("src/lib.rs", "pub fn value() -> u32 { 1 }")
        .build();

    project.cargo("generate-lockfile").run();
    compile_project(&project);

    let error = try_probe_scores(&project, &[project.build_dir()], Instant::now()).unwrap_err();
    assert_eq!(error.to_string(), "cache probe deadline exceeded");
    assert_eq!(
        error.downcast_ref::<ProbeFailure>(),
        Some(&ProbeFailure::Deadline)
    );
}

fn compile_project(project: &cargo_test_support::Project) {
    let gctx = probe_context(project);
    let workspace = Workspace::new(&project.root().join("Cargo.toml"), &gctx).unwrap();
    let options = CompileOptions::new(&gctx, UserIntent::Build).unwrap();
    cargo::ops::compile(&workspace, &options).unwrap();
}

fn probe_scores(project: &cargo_test_support::Project, candidates: &[PathBuf]) -> ProbeResult {
    try_probe_scores(project, candidates, Instant::now() + Duration::from_secs(2)).unwrap()
}

fn try_probe_scores(
    project: &cargo_test_support::Project,
    candidates: &[PathBuf],
    deadline: Instant,
) -> CargoResult<ProbeResult> {
    let gctx = probe_context(project);
    let workspace = Workspace::new(&project.root().join("Cargo.toml"), &gctx).unwrap();
    let options = CompileOptions::new(&gctx, UserIntent::Build).unwrap();
    cargo::cache_probe::probe(&workspace, &options, candidates, deadline)
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
