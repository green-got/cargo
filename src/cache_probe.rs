//! Read-only candidate evaluation for Green-Got's sandbox cache manager.
//! This adapter uses the same fingerprint implementation as the pinned native Cargo.

use crate::compiler::{BuildRunner, UnitInterner, UserIntent};
use crate::ops::{CompileOptions, create_bcx};
use crate::util::CargoResult;
use crate::util::cache_lock::CacheLockMode;
use crate::workspace::Workspace;
use std::path::PathBuf;
use std::time::Instant;

/// Native Cargo revision whose fingerprint implementation this probe uses.
pub const CARGO_COMMIT: &str = "b2e9d5f9db3fb1c454ab84f10c16508984a266e2";

#[derive(Debug, Clone, Copy)]
pub struct CandidateScore {
    pub fresh_units: usize,
    pub total_units: usize,
}

/// Candidates and sources must remain stable until the caller snapshots the winner.
/// The deadline is cooperative; graph resolution and individual filesystem calls can exceed it.
/// Package resolution requires an existing lockfile and cached dependencies.
pub fn probe(
    ws: &Workspace<'_>,
    options: &CompileOptions,
    candidates: &[PathBuf],
    deadline: Instant,
) -> CargoResult<Vec<CandidateScore>> {
    anyhow::ensure!(
        !ws.gctx().lock_update_allowed() && !ws.gctx().network_allowed(),
        "cache probing requires locked, offline resolution"
    );
    anyhow::ensure!(
        ws.target_dir() == ws.build_dir(),
        "separate build directories are unsupported"
    );
    anyhow::ensure!(
        matches!(
            options.build_config.intent,
            UserIntent::Check { .. } | UserIntent::Build
        ),
        "unsupported cache probe command"
    );
    let _read_lock = ws
        .gctx()
        .try_acquire_package_cache_lock(CacheLockMode::Shared)?
        .ok_or_else(|| anyhow::anyhow!("package cache is busy"))?;
    let _lock = ws
        .gctx()
        .try_acquire_package_cache_lock(CacheLockMode::DownloadExclusive)?
        .ok_or_else(|| anyhow::anyhow!("package cache is busy"))?;
    let interner = UnitInterner::new();
    let bcx = create_bcx(ws, options, &interner, None)?;
    anyhow::ensure!(
        !bcx.unit_graph
            .keys()
            .any(|unit| unit.pkg.manifest().metabuild().is_some()),
        "metabuild is unsupported"
    );
    BuildRunner::probe_caches(&bcx, candidates, deadline)
}
