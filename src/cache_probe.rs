//! Read-only candidate evaluation for Green-Got's sandbox cache manager.
//! This adapter uses the same fingerprint implementation as the pinned native Cargo.

use crate::compiler::{BuildRunner, UnitInterner, UserIntent};
use crate::ops::{CompileOptions, create_bcx};
use crate::util::CargoResult;
use crate::util::cache_lock::CacheLockMode;
use crate::workspace::Workspace;
use anyhow::Context as _;
use std::fmt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Native Cargo revision whose fingerprint implementation this probe uses.
pub const CARGO_COMMIT: &str = "7941be6fb416b4cd9666aef7b858dfea25587a8c";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateScore {
    pub fresh_units: usize,
    pub total_units: usize,
}

impl CandidateScore {
    /// A perfect candidate needs no compilation, so scoring stops at the first one.
    pub fn is_perfect(&self) -> bool {
        self.fresh_units == self.total_units
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeCompletion {
    AllCandidates,
    /// The perfect candidate is the last entry of [`ProbeResult::scores`].
    PerfectCandidate,
    /// The completed candidate prefix is returned; the interrupted candidate is omitted.
    Deadline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeResult {
    /// One entry per scored candidate, a prefix of the caller's candidate order.
    pub scores: Vec<CandidateScore>,
    pub completion: ProbeCompletion,
    /// Wall time spent resolving the package and unit graph before any candidate is scored.
    pub resolution_elapsed: Duration,
    /// Wall time spent scoring each candidate, parallel to `scores`.
    pub candidate_elapsed: Vec<Duration>,
}

/// Expected fallback causes, available through `anyhow::Error::downcast_ref`.
/// Resolution retains the underlying Cargo error; unexpected I/O errors remain unclassified.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProbeFailure {
    /// Another Cargo process holds the package cache lock.
    SourceCacheBusy,
    /// The deadline expired before any candidate completed.
    Deadline,
    /// Package or unit graph resolution failed; the underlying error is the source.
    Resolution,
    /// A probe precondition does not hold for this workspace, options, or candidate list.
    Unsupported(&'static str),
}

impl fmt::Display for ProbeFailure {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SourceCacheBusy => f.write_str("package cache is busy"),
            Self::Deadline => f.write_str("cache probe deadline exceeded"),
            Self::Resolution => f.write_str("cache probe resolution failed"),
            Self::Unsupported(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ProbeFailure {}

fn ensure_supported(condition: bool, reason: &'static str) -> CargoResult<()> {
    if condition {
        Ok(())
    } else {
        Err(ProbeFailure::Unsupported(reason).into())
    }
}

/// Candidates and sources must remain stable until the caller snapshots the winner.
/// All candidate paths are validated before any scoring, so an invalid later candidate fails
/// even when an earlier one is perfect.
/// The deadline is cooperative; graph resolution and individual filesystem calls can exceed it.
/// When it expires, the probe returns the fully scored candidate prefix, or an error if none
/// completed.
/// Package resolution requires an existing lockfile and cached dependencies.
pub fn probe(
    ws: &Workspace<'_>,
    options: &CompileOptions,
    candidates: &[PathBuf],
    deadline: Instant,
) -> CargoResult<ProbeResult> {
    ensure_supported(
        !ws.gctx().lock_update_allowed() && !ws.gctx().network_allowed(),
        "cache probing requires locked, offline resolution",
    )?;
    ensure_supported(
        ws.target_dir() == ws.build_dir(),
        "separate build directories are unsupported",
    )?;
    ensure_supported(
        matches!(
            options.build_config.intent,
            UserIntent::Check { .. } | UserIntent::Build
        ),
        "unsupported cache probe command",
    )?;
    ensure_supported(
        candidates.iter().all(|path| path.is_absolute()),
        "invalid cache candidate",
    )?;
    let _read_lock = ws
        .gctx()
        .try_acquire_package_cache_lock(CacheLockMode::Shared)?
        .ok_or(ProbeFailure::SourceCacheBusy)?;
    let _lock = ws
        .gctx()
        .try_acquire_package_cache_lock(CacheLockMode::DownloadExclusive)?
        .ok_or(ProbeFailure::SourceCacheBusy)?;
    let resolution_start = Instant::now();
    let interner = UnitInterner::new();
    let bcx = create_bcx(ws, options, &interner, None).context(ProbeFailure::Resolution)?;
    let resolution_elapsed = resolution_start.elapsed();
    ensure_supported(
        !bcx.unit_graph
            .keys()
            .any(|unit| unit.pkg.manifest().metabuild().is_some()),
        "metabuild is unsupported",
    )?;
    let mut result = BuildRunner::probe_caches(&bcx, candidates, deadline)?;
    result.resolution_elapsed = resolution_elapsed;
    Ok(result)
}
