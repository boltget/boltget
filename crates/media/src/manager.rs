//! The sidecar manager: install, verify, activate, update, roll back, prune.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use boltget_engine::{
    AddRequest, Checksum, ConflictPolicy, Destination, Engine, EngineConfig, HashAlgo,
    RequestContext,
};
use sha2::{Digest as _, Sha256};

use crate::manifest::{self, Installed, Manifest};
use crate::runner::Invocation;
use crate::sources::{self, Channel, GithubRelease, Layout, Sources};
use crate::{Error, Program, Result, Tool, extract, pgp};

/// yt-dlp is checked for updates at most this often in normal use.
pub const DAILY: Duration = Duration::from_secs(24 * 3600);

/// After an extraction failure that suggests a broken extractor, yt-dlp is
/// checked again unless a failure already caused a check this recently.
pub const FAILURE_RECHECK: Duration = Duration::from_secs(3600);

/// How long a smoke test may take (yt-dlp's first start can be slow while an
/// antivirus product scans it).
const SMOKE_TIMEOUT: Duration = Duration::from_secs(120);

/// The largest metadata file (release JSON, checksums, signature) accepted.
const MAX_METADATA: u64 = 8 * 1024 * 1024;

/// Directory for downloads in progress, inside the tools directory.
const DOWNLOADS: &str = ".downloads";

/// Progress of an install or update, for a front end to show.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum InstallEvent {
    /// Reading release information.
    Checking {
        /// The tool.
        tool: Tool,
    },
    /// Downloading a file.
    Downloading {
        /// The tool.
        tool: Tool,
        /// The file being downloaded.
        file: String,
        /// Bytes so far.
        done: u64,
        /// Size, if known.
        total: Option<u64>,
    },
    /// Checking signatures and checksums.
    Verifying {
        /// The tool.
        tool: Tool,
    },
    /// Unpacking.
    Unpacking {
        /// The tool.
        tool: Tool,
    },
    /// Running the new version's smoke test.
    SmokeTesting {
        /// The tool.
        tool: Tool,
        /// The version.
        version: String,
    },
    /// The new version is now the active one.
    Activated {
        /// The tool.
        tool: Tool,
        /// The version.
        version: String,
    },
    /// The active version failed its smoke test and the previous one is back.
    RolledBack {
        /// The tool.
        tool: Tool,
        /// The version given up.
        from: String,
        /// The version now active.
        to: String,
    },
}

/// A release that can be installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    /// The tool.
    pub tool: Tool,
    /// Its version (yt-dlp: the release tag).
    pub version: String,
    /// The download's size in bytes (from the release metadata).
    pub size: u64,
    /// The download URL.
    pub url: String,
}

/// What an install, update or check did.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Outcome {
    /// A new version was installed and is active.
    Installed {
        /// The tool.
        tool: Tool,
        /// The new active version.
        version: String,
        /// The version it replaced, kept for rollback.
        previous: Option<String>,
    },
    /// The active version is current.
    UpToDate {
        /// The tool.
        tool: Tool,
        /// The active version.
        version: String,
    },
    /// No check was due.
    NotDue {
        /// The tool.
        tool: Tool,
    },
    /// The tool is not installed, so it was not updated (installs happen on
    /// demand only).
    NotInstalled {
        /// The tool.
        tool: Tool,
    },
}

/// The installed state of one tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolStatus {
    /// The tool.
    pub tool: Tool,
    /// The active version.
    pub active: Option<String>,
    /// The version kept for rollback.
    pub previous: Option<String>,
    /// The release channel (yt-dlp only; FFmpeg follows Boltget's pinned build).
    pub channel: Channel,
    /// When the last update check succeeded.
    pub last_check: Option<SystemTime>,
    /// The active version's programs.
    pub programs: Vec<(Program, PathBuf)>,
    /// Whether a build exists for this platform.
    pub supported: bool,
}

/// The state of every tool.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Status {
    /// The tools directory.
    pub dir: PathBuf,
    /// One entry per tool, in [`Tool::ALL`] order.
    pub tools: Vec<ToolStatus>,
}

/// Manages the sidecars in one tools directory. See the crate documentation.
#[derive(Debug, Clone)]
pub struct Sidecars {
    dir: PathBuf,
    sources: Sources,
    engine: EngineConfig,
    smoke_timeout: Duration,
}

impl Sidecars {
    /// A manager for `dir` with the given sources. Downloads use `engine`
    /// (its proxy and certificates, for example), with the hosts pinned to
    /// [`Sources::allowed_hosts`] and no Mark of the Web (the files are
    /// verified, and are not opened by the user).
    pub fn new(dir: PathBuf, sources: Sources, engine: EngineConfig) -> Self {
        Sidecars {
            dir,
            sources,
            engine,
            smoke_timeout: SMOKE_TIMEOUT,
        }
    }

    /// The default tools directory: `tools` in Boltget's data directory
    /// (`%LOCALAPPDATA%\Boltget\tools` on Windows).
    pub fn default_dir() -> Option<PathBuf> {
        boltget_engine::default_data_dir().map(|d| d.join("tools"))
    }

    /// Sets how long a smoke test may take.
    #[must_use]
    pub fn smoke_timeout(mut self, timeout: Duration) -> Self {
        self.smoke_timeout = timeout;
        self
    }

    /// The tools directory.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// What is installed. Reads only the manifest.
    pub fn status(&self) -> Result<Status> {
        let m = Manifest::load(&self.dir)?;
        let tools = Tool::ALL
            .iter()
            .map(|&tool| {
                let st = m.tool(tool);
                let programs = st
                    .active
                    .as_ref()
                    .and_then(|v| st.versions.get(v).map(|i| (v, i)))
                    .map(|(v, i)| self.program_paths(tool, v, i))
                    .unwrap_or_default();
                ToolStatus {
                    tool,
                    active: st.active.clone(),
                    previous: st.previous.clone(),
                    channel: st.channel,
                    last_check: st.last_check.map(|s| UNIX_EPOCH + Duration::from_secs(s)),
                    programs,
                    supported: self.supported(tool),
                }
            })
            .collect();
        Ok(Status {
            dir: self.dir.clone(),
            tools,
        })
    }

    fn supported(&self, tool: Tool) -> bool {
        match tool {
            Tool::YtDlp => self.sources.yt_dlp_asset.is_some(),
            Tool::Ffmpeg => self.sources.ffmpeg.is_some(),
        }
    }

    fn program_paths(
        &self,
        tool: Tool,
        version: &str,
        inst: &Installed,
    ) -> Vec<(Program, PathBuf)> {
        let base = self.version_dir(tool, version);
        inst.programs
            .iter()
            .filter_map(|(name, rel)| Some((Program::parse(name)?, under(&base, rel))))
            .collect()
    }

    fn tool_dir(&self, tool: Tool) -> PathBuf {
        self.dir.join(tool.id())
    }

    fn version_dir(&self, tool: Tool, version: &str) -> PathBuf {
        self.tool_dir(tool).join(version)
    }

    /// The path of an installed program (the active version's).
    pub fn program(&self, program: Program) -> Result<PathBuf> {
        let tool = program.tool();
        let m = Manifest::load(&self.dir)?;
        let st = m.tool(tool);
        let (version, inst) = st
            .active
            .as_ref()
            .and_then(|v| st.versions.get(v).map(|i| (v, i)))
            .ok_or(Error::NotInstalled(tool))?;
        self.program_paths(tool, version, inst)
            .into_iter()
            .find(|(p, _)| *p == program)
            .map(|(_, path)| path)
            .ok_or(Error::NotInstalled(tool))
    }

    /// Chooses yt-dlp's release channel. The next update check goes to it.
    pub fn set_channel(&self, channel: Channel) -> Result<()> {
        let _lock = manifest::lock(&self.dir)?;
        let mut m = Manifest::load(&self.dir)?;
        let st = m.tool_mut(Tool::YtDlp);
        if st.channel != channel {
            st.channel = channel;
            st.last_check = None;
            m.save(&self.dir)?;
        }
        Ok(())
    }

    /// The release that an install or update would use now. For yt-dlp this
    /// reads the release metadata (not the signature: that is checked before
    /// anything is installed).
    pub async fn latest(&self, tool: Tool) -> Result<Release> {
        match tool {
            Tool::YtDlp => {
                let channel = Manifest::load(&self.dir)?.tool(tool).channel;
                let engine = self.engine()?;
                let result = self.yt_dlp_release(&engine, channel, &|_| {}).await;
                engine.shutdown().await;
                let (release, _) = result?;
                Ok(release.into())
            }
            Tool::Ffmpeg => {
                let b = self
                    .sources
                    .ffmpeg
                    .as_ref()
                    .ok_or(Error::Unsupported(tool))?;
                Ok(Release {
                    tool,
                    version: b.version.clone(),
                    size: b.size,
                    url: b.url.clone(),
                })
            }
        }
    }

    /// Installs `tool` if it is not installed (on demand). An installed tool
    /// is left as it is: see [`Sidecars::update`].
    pub async fn install(
        &self,
        tool: Tool,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Outcome> {
        if let Some(version) = Manifest::load(&self.dir)?.tool(tool).active {
            return Ok(Outcome::UpToDate { tool, version });
        }
        self.update(tool, on_event).await
    }

    /// Installs the latest release of `tool` if it differs from the active
    /// version (or nothing is active). Never uses yt-dlp's own updater.
    pub async fn update(
        &self,
        tool: Tool,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Outcome> {
        if !self.supported(tool) {
            return Err(Error::Unsupported(tool));
        }
        fs::create_dir_all(&self.dir)
            .map_err(|e| Error::io(format!("creating {}", self.dir.display()), e))?;
        let _lock = manifest::lock(&self.dir)?;
        let engine = self.engine()?;
        let result = self.update_locked(&engine, tool, on_event).await;
        engine.shutdown().await;
        let _ = fs::remove_dir_all(self.dir.join(DOWNLOADS));
        result
    }

    async fn update_locked(
        &self,
        engine: &Engine,
        tool: Tool,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Outcome> {
        let state = Manifest::load(&self.dir)?.tool(tool);
        on_event(InstallEvent::Checking { tool });
        let plan = match tool {
            Tool::YtDlp => {
                let (release, asset) = self.yt_dlp_release(engine, state.channel, on_event).await?;
                Plan {
                    version: release.version,
                    url: release.url,
                    size: release.size,
                    sha256: None,
                    layout: asset.layout,
                    programs: asset.programs,
                    sums: Some(Sums {
                        checksums: release.checksums,
                        signature: release.signature,
                        asset: asset.name,
                    }),
                }
            }
            Tool::Ffmpeg => {
                let b = self
                    .sources
                    .ffmpeg
                    .clone()
                    .ok_or(Error::Unsupported(tool))?;
                Plan {
                    version: b.version,
                    url: b.url,
                    size: b.size,
                    sha256: Some(b.sha256),
                    layout: b.layout,
                    programs: b.programs,
                    sums: None,
                }
            }
        };
        let now = unix_now();
        if state.active.as_deref() == Some(plan.version.as_str())
            || state.rejected.contains(&plan.version)
        {
            if state.rejected.contains(&plan.version) {
                tracing::warn!(%tool, version = %plan.version, "latest version was rolled back before; staying");
            }
            let mut m = Manifest::load(&self.dir)?;
            m.tool_mut(tool).last_check = Some(now);
            m.save(&self.dir)?;
            return Ok(match state.active {
                Some(version) => Outcome::UpToDate { tool, version },
                None => Outcome::NotInstalled { tool },
            });
        }
        let outcome = self.install_plan(engine, tool, plan, on_event).await?;
        Ok(outcome)
    }

    /// The latest yt-dlp release on `channel` and this platform's asset.
    async fn yt_dlp_release(
        &self,
        engine: &Engine,
        channel: Channel,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<(ReleaseInfo, sources::AssetSpec)> {
        let tool = Tool::YtDlp;
        let spec = self
            .sources
            .yt_dlp_asset
            .clone()
            .ok_or(Error::Unsupported(tool))?;
        let url = self.sources.release_url(channel).to_owned();
        let json = self
            .fetch_small(engine, tool, &url, "release.json", on_event)
            .await?;
        let release: GithubRelease = serde_json::from_slice(&json)
            .map_err(|e| Error::release(tool, format!("release JSON: {e}")))?;
        if !sources::valid_version(&release.tag_name) {
            return Err(Error::release(
                tool,
                format!("unexpected release tag {:?}", release.tag_name),
            ));
        }
        let asset = release
            .asset(&spec.name)
            .ok_or_else(|| Error::release(tool, format!("the release has no {}", spec.name)))?;
        for needed in ["SHA2-256SUMS", "SHA2-256SUMS.sig"] {
            if release.asset(needed).is_none() {
                return Err(Error::release(tool, format!("the release has no {needed}")));
            }
        }
        self.sources.check_url(tool, &asset.browser_download_url)?;
        let info = ReleaseInfo {
            version: release.tag_name.clone(),
            url: asset.browser_download_url.clone(),
            size: asset.size,
            checksums: release
                .asset("SHA2-256SUMS")
                .map(|a| a.browser_download_url.clone())
                .unwrap_or_default(),
            signature: release
                .asset("SHA2-256SUMS.sig")
                .map(|a| a.browser_download_url.clone())
                .unwrap_or_default(),
        };
        Ok((info, spec))
    }

    async fn install_plan(
        &self,
        engine: &Engine,
        tool: Tool,
        mut plan: Plan,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Outcome> {
        // yt-dlp: the expected SHA-256 comes from the signed checksum file.
        if let Some(sums) = plan.sums.take() {
            let text = self
                .fetch_small(engine, tool, &sums.checksums, "SHA2-256SUMS", on_event)
                .await?;
            let sig = self
                .fetch_small(engine, tool, &sums.signature, "SHA2-256SUMS.sig", on_event)
                .await?;
            pgp::verify_detached(&self.sources.yt_dlp_key, &text, &sig, SystemTime::now())
                .map_err(|detail| Error::Signature { tool, detail })?;
            let text = String::from_utf8(text)
                .map_err(|_| Error::release(tool, "SHA2-256SUMS is not text"))?;
            plan.sha256 = Some(sources::sum_for(&text, &sums.asset).ok_or_else(|| {
                Error::release(
                    tool,
                    format!("SHA2-256SUMS does not list {} once", sums.asset),
                )
            })?);
        }
        let sha256 = plan
            .sha256
            .clone()
            .ok_or_else(|| Error::release(tool, "no checksum"))?;
        let file_name = plan
            .url
            .rsplit('/')
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("download")
            .to_owned();
        let downloads = self.dir.join(DOWNLOADS);
        let download = downloads.join(format!("{}-{}", tool.id(), sources_safe(&file_name)));
        self.fetch(
            engine,
            tool,
            &plan.url,
            &download,
            Some(&sha256),
            &file_name,
            plan.size,
            on_event,
        )
        .await?;
        on_event(InstallEvent::Verifying { tool });
        let actual = sha256_file(&download)?;
        if actual != sha256 {
            return Err(Error::Checksum {
                tool,
                file: file_name,
                expected: sha256,
                actual,
            });
        }

        on_event(InstallEvent::Unpacking { tool });
        let tool_dir = self.tool_dir(tool);
        let staging = tool_dir.join(format!(".staging-{}-{}", plan.version, std::process::id()));
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)
            .map_err(|e| Error::io(format!("creating {}", staging.display()), e))?;
        if let Err(detail) = extract::unpack(&download, &plan.layout, &staging) {
            let _ = fs::remove_dir_all(&staging);
            return Err(Error::Archive {
                tool,
                file: file_name,
                detail,
            });
        }
        let _ = fs::remove_file(&download);
        let final_dir = self.version_dir(tool, &plan.version);
        if final_dir.exists() {
            fs::remove_dir_all(&final_dir)
                .map_err(|e| Error::io(format!("replacing {}", final_dir.display()), e))?;
        }
        fs::rename(&staging, &final_dir).map_err(|e| {
            let _ = fs::remove_dir_all(&staging);
            Error::io(format!("moving into {}", final_dir.display()), e)
        })?;

        let programs: BTreeMap<String, String> = plan
            .programs
            .iter()
            .map(|(p, rel)| (p.name().to_owned(), rel.clone()))
            .collect();
        on_event(InstallEvent::SmokeTesting {
            tool,
            version: plan.version.clone(),
        });
        if let Err(detail) = self
            .smoke_test(tool, &plan.version, &final_dir, &programs)
            .await
        {
            // Never activated: the active version stays as it was.
            let _ = fs::remove_dir_all(&final_dir);
            return Err(Error::SmokeTest {
                tool,
                version: plan.version,
                detail,
            });
        }

        let mut m = Manifest::load(&self.dir)?;
        let st = m.tool_mut(tool);
        let previous = st.active.take();
        st.versions.insert(
            plan.version.clone(),
            Installed {
                url: plan.url,
                sha256,
                size: plan.size,
                added_at: unix_now(),
                programs,
            },
        );
        st.active = Some(plan.version.clone());
        st.previous.clone_from(&previous);
        st.last_check = Some(unix_now());
        m.save(&self.dir)?;
        on_event(InstallEvent::Activated {
            tool,
            version: plan.version.clone(),
        });
        let _ = self.prune_locked(tool);
        Ok(Outcome::Installed {
            tool,
            version: plan.version,
            previous,
        })
    }

    /// Runs the version's programs with `--version` / `-version` and checks
    /// what they print.
    async fn smoke_test(
        &self,
        tool: Tool,
        version: &str,
        dir: &Path,
        programs: &BTreeMap<String, String>,
    ) -> std::result::Result<(), String> {
        for &program in tool.programs() {
            let rel = programs
                .get(program.name())
                .ok_or_else(|| format!("{program} is missing from the install"))?;
            let exe = under(dir, rel);
            if !exe.is_file() {
                return Err(format!("{} is missing", exe.display()));
            }
            let inv = match program {
                Program::YtDlp => Invocation::yt_dlp(&exe, None, &["--version".to_owned()]),
                _ => Invocation::ffmpeg(program, &exe, &["-version".to_owned()]),
            }
            .map_err(|e| e.to_string())?
            .timeout(self.smoke_timeout)
            .max_output(64 * 1024);
            let out = inv.run().await.map_err(|e| e.to_string())?;
            if !out.status.success() {
                return Err(format!(
                    "{program} exited with {}: {}",
                    out.status,
                    out.stderr_text().trim()
                ));
            }
            let text = out.stdout_text();
            let first = text
                .lines()
                .find(|l| !l.trim().is_empty())
                .unwrap_or("")
                .trim();
            let ok = match program {
                Program::YtDlp => first == version,
                _ => {
                    first.starts_with(&format!("{} version ", program.name()))
                        && first.contains(version.trim_start_matches('n'))
                }
            };
            if !ok {
                return Err(format!(
                    "{program} printed {first:?}, expected version {version}"
                ));
            }
        }
        Ok(())
    }

    /// Smoke-tests the active version of `tool`. If it fails and the previous
    /// version passes, the previous one becomes active again and the failed
    /// version is never installed again automatically.
    pub async fn verify_active(
        &self,
        tool: Tool,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<String> {
        let _lock = manifest::lock(&self.dir)?;
        let m = Manifest::load(&self.dir)?;
        let st = m.tool(tool);
        let active = st.active.clone().ok_or(Error::NotInstalled(tool))?;
        let programs = st
            .versions
            .get(&active)
            .map(|i| i.programs.clone())
            .unwrap_or_default();
        let dir = self.version_dir(tool, &active);
        let Err(detail) = self.smoke_test(tool, &active, &dir, &programs).await else {
            return Ok(active);
        };
        tracing::warn!(%tool, version = %active, %detail, "active version failed its smoke test");
        let Some(previous) = st.previous.clone() else {
            return Err(Error::SmokeTest {
                tool,
                version: active,
                detail,
            });
        };
        let prev_programs = st
            .versions
            .get(&previous)
            .map(|i| i.programs.clone())
            .unwrap_or_default();
        let prev_dir = self.version_dir(tool, &previous);
        if let Err(prev_detail) = self
            .smoke_test(tool, &previous, &prev_dir, &prev_programs)
            .await
        {
            return Err(Error::SmokeTest {
                tool,
                version: active,
                detail: format!(
                    "{detail}; the previous version {previous} failed too: {prev_detail}"
                ),
            });
        }
        self.switch_back(tool)?;
        on_event(InstallEvent::RolledBack {
            tool,
            from: active,
            to: previous.clone(),
        });
        Ok(previous)
    }

    /// Makes the previous version active again, by hand. The version given up
    /// is not installed again automatically. Returns the version now active.
    pub fn rollback(&self, tool: Tool) -> Result<String> {
        let _lock = manifest::lock(&self.dir)?;
        self.switch_back(tool)
    }

    fn switch_back(&self, tool: Tool) -> Result<String> {
        let mut m = Manifest::load(&self.dir)?;
        let st = m.tool_mut(tool);
        let previous = st.previous.take().ok_or(Error::NoPrevious(tool))?;
        if let Some(bad) = st.active.replace(previous.clone())
            && !st.rejected.contains(&bad)
        {
            st.rejected.push(bad);
        }
        m.save(&self.dir)?;
        let _ = self.prune_locked(tool);
        Ok(previous)
    }

    /// Removes every version of `tool` except the active and previous ones,
    /// and leftovers of interrupted installs. A directory still in use (a
    /// running sidecar on Windows) is left for the next prune. Returns the
    /// versions removed.
    pub fn prune(&self, tool: Tool) -> Result<Vec<String>> {
        let _lock = manifest::lock(&self.dir)?;
        self.prune_locked(tool)
    }

    fn prune_locked(&self, tool: Tool) -> Result<Vec<String>> {
        let mut m = Manifest::load(&self.dir)?;
        let st = m.tool_mut(tool);
        let keep: Vec<String> = st
            .active
            .iter()
            .chain(st.previous.iter())
            .cloned()
            .collect();
        let mut removed = Vec::new();
        let tool_dir = self.tool_dir(tool);
        let entries = match fs::read_dir(&tool_dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(removed),
            Err(e) => return Err(Error::io(format!("reading {}", tool_dir.display()), e)),
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if keep.contains(&name) {
                continue;
            }
            let path = entry.path();
            let gone = if path.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
            match gone {
                Ok(()) => {
                    if !name.starts_with('.') {
                        removed.push(name);
                    }
                }
                Err(e) => tracing::debug!(path = %path.display(), error = %e, "cannot prune yet"),
            }
        }
        let before = st.versions.len();
        st.versions
            .retain(|v, _| keep.contains(v) || tool_dir.join(v).exists());
        if st.versions.len() != before {
            m.save(&self.dir)?;
        }
        removed.sort();
        Ok(removed)
    }

    /// Updates installed tools whose check is due at `now`: yt-dlp once a day
    /// ([`DAILY`]); FFmpeg when Boltget's pinned build differs from the active
    /// one (no network check: the pin changes with Boltget releases). Tools
    /// that are not installed are left alone.
    pub async fn update_if_due(
        &self,
        now: SystemTime,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Vec<Outcome>> {
        let m = Manifest::load(&self.dir)?;
        let mut out = Vec::new();
        for tool in Tool::ALL {
            let st = m.tool(tool);
            if st.active.is_none() {
                out.push(Outcome::NotInstalled { tool });
                continue;
            }
            let due = match tool {
                Tool::YtDlp => st
                    .last_check
                    .is_none_or(|t| secs(now).saturating_sub(t) >= DAILY.as_secs()),
                Tool::Ffmpeg => self
                    .sources
                    .ffmpeg
                    .as_ref()
                    .is_some_and(|b| st.active.as_deref() != Some(b.version.as_str())),
            };
            out.push(if due {
                self.update(tool, on_event).await?
            } else {
                Outcome::NotDue { tool }
            });
        }
        Ok(out)
    }

    /// Called when an extraction failed in a way that suggests a broken
    /// extractor ("the site changed"): checks for a newer yt-dlp at once,
    /// unless such a failure already caused a check within
    /// [`FAILURE_RECHECK`] of `now`.
    pub async fn on_extractor_failure(
        &self,
        now: SystemTime,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Outcome> {
        let tool = Tool::YtDlp;
        {
            let _lock = manifest::lock(&self.dir)?;
            let mut m = Manifest::load(&self.dir)?;
            let st = m.tool_mut(tool);
            if st.active.is_none() {
                return Ok(Outcome::NotInstalled { tool });
            }
            if st
                .last_failure_check
                .is_some_and(|t| secs(now).saturating_sub(t) < FAILURE_RECHECK.as_secs())
            {
                return Ok(Outcome::NotDue { tool });
            }
            st.last_failure_check = Some(secs(now));
            m.save(&self.dir)?;
        }
        self.update(tool, on_event).await
    }

    fn engine(&self) -> Result<Engine> {
        let mut cfg = self.engine.clone();
        cfg.allowed_hosts = Some(self.sources.allowed_hosts.clone());
        cfg.mark_of_the_web = false;
        Ok(Engine::new(cfg)?)
    }

    /// Downloads a small metadata file and returns its bytes.
    async fn fetch_small(
        &self,
        engine: &Engine,
        tool: Tool,
        url: &str,
        name: &str,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<Vec<u8>> {
        let dest = self
            .dir
            .join(DOWNLOADS)
            .join(format!("{}-{name}", tool.id()));
        self.fetch(engine, tool, url, &dest, None, name, 0, on_event)
            .await?;
        let len = fs::metadata(&dest)
            .map_err(|e| Error::io(format!("reading {}", dest.display()), e))?
            .len();
        if len > MAX_METADATA {
            return Err(Error::release(tool, format!("{name} is too large")));
        }
        let bytes =
            fs::read(&dest).map_err(|e| Error::io(format!("reading {}", dest.display()), e))?;
        let _ = fs::remove_file(&dest);
        Ok(bytes)
    }

    /// Downloads `url` to `dest` with the engine, reporting progress.
    #[expect(
        clippy::too_many_arguments,
        reason = "internal helper; each argument is distinct"
    )]
    async fn fetch(
        &self,
        engine: &Engine,
        tool: Tool,
        url: &str,
        dest: &Path,
        sha256: Option<&str>,
        file: &str,
        size_hint: u64,
        on_event: &(dyn Fn(InstallEvent) + Sync),
    ) -> Result<()> {
        self.sources.check_url(tool, url)?;
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent)
                .map_err(|e| Error::io(format!("creating {}", parent.display()), e))?;
        }
        let ctx = RequestContext::new(url)?;
        let mut req = AddRequest::new(ctx, Destination::File(dest.to_path_buf()))
            .conflict(ConflictPolicy::Overwrite)
            .server_digest(false);
        if let Some(h) = sha256 {
            let c = Checksum::new(HashAlgo::Sha256, h).map_err(|e| Error::release(tool, e))?;
            req = req.checksum(c);
        }
        let handle = engine.add(req);
        let id = handle.id();
        let wait = handle.wait();
        tokio::pin!(wait);
        let mut tick = tokio::time::interval(Duration::from_millis(250));
        let done = loop {
            tokio::select! {
                r = &mut wait => break r,
                _ = tick.tick() => {
                    if let Some(p) = engine.progress(&[id]).into_iter().next() {
                        on_event(InstallEvent::Downloading {
                            tool,
                            file: file.to_owned(),
                            done: p.downloaded,
                            total: p.total.or((size_hint > 0).then_some(size_hint)),
                        });
                    }
                }
            }
        };
        let done = done?;
        on_event(InstallEvent::Downloading {
            tool,
            file: file.to_owned(),
            done: done.bytes,
            total: Some(done.bytes),
        });
        Ok(())
    }
}

/// What the latest yt-dlp release offers.
#[derive(Debug, Clone)]
struct ReleaseInfo {
    version: String,
    url: String,
    size: u64,
    checksums: String,
    signature: String,
}

impl From<ReleaseInfo> for Release {
    fn from(r: ReleaseInfo) -> Self {
        Release {
            tool: Tool::YtDlp,
            version: r.version,
            size: r.size,
            url: r.url,
        }
    }
}

/// Where yt-dlp's signed checksums are, and which line to read.
#[derive(Debug, Clone)]
struct Sums {
    checksums: String,
    signature: String,
    asset: String,
}

/// One install to carry out.
#[derive(Debug)]
struct Plan {
    version: String,
    url: String,
    size: u64,
    sha256: Option<String>,
    layout: Layout,
    programs: Vec<(Program, String)>,
    sums: Option<Sums>,
}

fn sources_safe(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut f =
        fs::File::open(path).map_err(|e| Error::io(format!("reading {}", path.display()), e))?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)
        .map_err(|e| Error::io(format!("reading {}", path.display()), e))?;
    Ok(h.finalize().iter().fold(String::new(), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    }))
}

/// `rel` (with `/` separators, as stored in the manifest) under `base`.
fn under(base: &Path, rel: &str) -> PathBuf {
    rel.split('/')
        .filter(|c| !c.is_empty())
        .fold(base.to_path_buf(), |p, c| p.join(c))
}

fn secs(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

fn unix_now() -> u64 {
    secs(SystemTime::now())
}
