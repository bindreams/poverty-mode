//! The `ProxyManager` seam (R15 / spec §5.9): an abstraction over starting and
//! tearing down proxy hops, so a future shared/refcounted manager drops in
//! without changing the orchestrator. v1 impl: `EphemeralManager`.

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;

use crate::config::ResolvedProxy;
use crate::proxy::{ProxyName, Upstream};

use super::teardown::ProxyGroup;
use super::{health_chain_id, proxy_child_args, read_ready_line, ProxyHopSpec};

/// One first-party hop to start, carried as STRUCTURED fields (not a pre-rendered
/// argv). The manager renders the exact argv via the single source of truth
/// `proxy_child_args` once it knows the real `--upstream` for this hop (it wires
/// back-to-front). This means the argv the manager spawns is byte-for-byte what
/// `proxy_child_args` produces and what M6.4's unit tests assert — no
/// strip/re-append dance, no divergence between the tested and shipped artifact.
pub struct HopSpec {
    /// The resolved proxy (name + settings → transform flags via `proxy_child_args`).
    pub proxy: ResolvedProxy,
    /// Per-run ULID identity shared by all hops (R10), stamped as `--run-id`.
    pub run_id: String,
    /// `--log-file` destination for this hop.
    pub log_file: PathBuf,
}

impl HopSpec {
    pub fn name(&self) -> ProxyName {
        self.proxy.name
    }
}

/// A started, healthy hop.
pub struct RunningProxy {
    pub name: ProxyName,
    pub port: u16,
    pub base_url: url::Url,
}

/// The seam the orchestrator builds chains through.
///
/// Hand-rolled async-fn-in-trait (no `async_trait` dep, R2): the methods return a
/// boxed, pinned future so the trait stays object-safe for `&mut dyn ProxyManager`
/// (the orchestrator drives chain build entirely through this trait object, R15).
pub trait ProxyManager {
    /// Start the given hops BACK-TO-FRONT (tail first), wiring each hop's upstream
    /// to the next, performing the READY handshake + `/__pm/health` identity check,
    /// and return them head-first. On any failure, everything already started must
    /// be torn down before returning (fail-closed).
    fn start_hops<'a>(
        &'a mut self,
        hops: &'a [HopSpec],
        tail_upstream: &'a Upstream,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RunningProxy>>> + Send + 'a>>;

    /// Tear down everything this manager started (await real exit).
    fn shutdown<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>>;
}

/// v1 manager: per-session ephemeral first-party proxy children in a
/// kill-on-exit `ProxyGroup` (Unix PDEATHSIG+death-pipe+killpg; Windows job).
pub struct EphemeralManager {
    exe: PathBuf,
    group: ProxyGroup,
}

impl EphemeralManager {
    pub fn new(exe: PathBuf) -> anyhow::Result<EphemeralManager> {
        Ok(EphemeralManager {
            exe,
            group: ProxyGroup::new()?,
        })
    }

    /// Render the EXACT self-spawn argv for a hop with its real upstream, via the
    /// single source of truth `proxy_child_args` (M6.4). The argv spawned here is
    /// identical to what M6.4's unit tests assert.
    fn full_args(spec: &HopSpec, upstream: &str) -> Vec<String> {
        let hop_spec = ProxyHopSpec {
            proxy: &spec.proxy,
            listen: "127.0.0.1:0".to_string(),
            upstream: upstream.to_string(),
            run_id: spec.run_id.clone(),
            log_file: spec.log_file.clone(),
        };
        proxy_child_args(&hop_spec)
    }

    async fn start_hops_inner(
        &mut self,
        hops: &[HopSpec],
        tail_upstream: &Upstream,
    ) -> anyhow::Result<Vec<RunningProxy>> {
        let exe = self.exe.clone();
        let mut next_upstream = tail_upstream.url.to_string();
        // We push tail-first, then reverse to return head-first.
        let mut started_rev: Vec<RunningProxy> = Vec::with_capacity(hops.len());

        for hop in hops.iter().rev() {
            let args = Self::full_args(hop, &next_upstream);
            let spawned = self.group.spawn(&exe, &args).map_err(|e| {
                anyhow::anyhow!("spawning proxy hop '{}': {e}", hop.name().as_str())
            })?;
            let stdout = spawned.stdout.ok_or_else(|| {
                anyhow::anyhow!("proxy hop '{}' had no piped stdout", hop.name().as_str())
            })?;
            let mut reader = tokio::io::BufReader::new(stdout);

            // Blocking READY read = real synchronization (no sleep/poll). Validates
            // ready/name/run_id (R10).
            let ready = read_ready_line(&mut reader, hop.name(), &hop.run_id)
                .await
                .map_err(|e| {
                    anyhow::anyhow!(
                        "READY handshake for hop '{}' failed: {e}",
                        hop.name().as_str()
                    )
                })?;

            // Verify identity/staleness via /__pm/health off the async executor
            // (R5: the blocking GET must not run on the runtime thread).
            let hop_base = url::Url::parse(&format!("http://127.0.0.1:{}", ready.port))?;
            let run_id = hop.run_id.clone();
            let probe_base = hop_base.clone();
            let live_id = tokio::task::spawn_blocking(move || health_chain_id(&probe_base))
                .await
                .map_err(|e| anyhow::anyhow!("health probe task join error: {e}"))?;
            match live_id {
                Some(id) if id == run_id => {}
                Some(id) => anyhow::bail!(
                    "hop '{}' health run_id '{}' != expected '{}'",
                    hop.name().as_str(),
                    id,
                    run_id
                ),
                None => anyhow::bail!(
                    "hop '{}' did not answer /__pm/health on port {}",
                    hop.name().as_str(),
                    ready.port
                ),
            }

            next_upstream = format!("http://127.0.0.1:{}", ready.port);
            started_rev.push(RunningProxy {
                name: hop.name(),
                port: ready.port,
                base_url: hop_base,
            });
        }

        started_rev.reverse(); // head-first
        Ok(started_rev)
    }
}

impl ProxyManager for EphemeralManager {
    fn start_hops<'a>(
        &'a mut self,
        hops: &'a [HopSpec],
        tail_upstream: &'a Upstream,
    ) -> Pin<Box<dyn Future<Output = anyhow::Result<Vec<RunningProxy>>> + Send + 'a>> {
        Box::pin(async move {
            match self.start_hops_inner(hops, tail_upstream).await {
                Ok(v) => Ok(v),
                Err(e) => {
                    // Fail-closed: tear down anything already started before
                    // returning, so no partial chain survives.
                    let _ = self.group.kill_all();
                    let _ = self.group.wait_all_exited().await;
                    Err(e.context("chain readiness failed; torn down all started proxies"))
                }
            }
        })
    }

    fn shutdown<'a>(&'a mut self) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send + 'a>> {
        Box::pin(async move {
            self.group.kill_all()?;
            self.group.wait_all_exited().await
        })
    }
}

#[cfg(test)]
#[path = "manager_tests.rs"]
mod manager_tests;
