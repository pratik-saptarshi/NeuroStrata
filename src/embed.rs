use crate::traits::Embedder;
use anyhow::{anyhow, Context, Result};
use async_trait::async_trait;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Serialize, Deserialize, Clone)]
pub struct AcceptableEmbedder {
    pub model_name: String,
    pub dimensions: usize,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub api_model: Option<String>,
    /// Provenance stamp: `EMBEDDERS_FILE_PROVENANCE` when this entry was
    /// materialized by the binary rather than written by the operator.
    #[serde(default)]
    pub generated_by: Option<String>,
    /// Pinned SHA-256 of the model weights. When set, the embedder verifies
    /// the downloaded model against this hash before accepting it.
    /// Supply-chain protection: a mismatched hash is a fatal error.
    #[serde(default)]
    pub model_sha256: Option<String>,
}

/// Provenance marker stamped into every entry of an embedders.json the
/// binary materialized itself (ported from the fork's embed layer).
pub(crate) const EMBEDDERS_FILE_PROVENANCE: &str = "neurostrata-auto-generated";

/// The default list as the binary materializes it: every entry stamped
/// with its provenance.
pub(crate) fn auto_generated_defaults() -> Vec<AcceptableEmbedder> {
    default_acceptable_embedders()
        .into_iter()
        .map(|mut model| {
            model.generated_by = Some(EMBEDDERS_FILE_PROVENANCE.to_string());
            model
        })
        .collect()
}

/// Whether two entries are the same operator choice, ignoring provenance.
fn same_embedder_choice(a: &AcceptableEmbedder, b: &AcceptableEmbedder) -> bool {
    a.model_name == b.model_name
        && a.dimensions == b.dimensions
        && a.base_url == b.base_url
        && a.api_key_env == b.api_key_env
        && a.api_model == b.api_model
}

/// Whether a parsed `embedders.json` is the file the binary materialized,
/// untouched since: every entry carries the provenance stamp and the full
/// entry contents (order included) match the built-in defaults
/// semantically. Anything else is user-edited.
#[allow(dead_code)]
fn is_auto_generated(models: &[AcceptableEmbedder]) -> bool {
    let defaults = default_acceptable_embedders();
    models.len() == defaults.len()
        && models.iter().zip(defaults.iter()).all(|(model, default)| {
            same_embedder_choice(model, default)
                && model.generated_by.as_deref() == Some(EMBEDDERS_FILE_PROVENANCE)
        })
}

/// The built-in default acceptable-embedder list: the single full-size
/// Nomic entry every fresh install starts from.
pub(crate) fn default_acceptable_embedders() -> Vec<AcceptableEmbedder> {
    vec![AcceptableEmbedder {
        model_name: "NomicEmbedTextV15".to_string(),
        dimensions: 768,
        base_url: None,
        api_key_env: None,
        api_model: None,
        generated_by: None,
        // Real SHA-256 of the fastembed Nomic v1.5 ONNX weights (computed).
        model_sha256: Some(
            "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965".to_string(),
        ),
    }]
}

/// The operator's config directory (`~/.config/neurostrata`).
pub(crate) fn config_dir() -> PathBuf {
    // dirs::home_dir() rather than $HOME: Windows sets USERPROFILE, not HOME, so
    // reading the variable directly sends this to a relative path under whatever
    // directory happened to launch the process. config.rs already resolves it this way.
    dirs::home_dir()
        .map(|home| home.join(".config").join("neurostrata"))
        .unwrap_or_else(|| PathBuf::from(".neurostrata"))
}

fn get_acceptable_embedders() -> Result<Vec<AcceptableEmbedder>> {
    get_acceptable_embedders_in(&config_dir())
}

pub(crate) fn get_acceptable_embedders_in(config_dir: &Path) -> Result<Vec<AcceptableEmbedder>> {
    if !config_dir.exists() {
        fs::create_dir_all(config_dir).context("Failed to create config directory")?;
    }

    let config_path = config_dir.join("embedders.json");

    // Default list if file doesn't exist or is empty
    let default_models = default_acceptable_embedders();

    if config_path.exists() {
        let content = fs::read_to_string(&config_path).context("Failed to read embedders.json")?;
        match serde_json::from_str::<Vec<AcceptableEmbedder>>(&content) {
            Ok(mut models) => {
                if !models.is_empty() {
                    // Backfill known pins for configs written before supply-chain
                    // verification existed; unknown models remain unpinned and fail
                    // closed at load time.
                    for model in &mut models {
                        if model.model_sha256.is_none()
                            && model.model_name.eq_ignore_ascii_case("NomicEmbedTextV15")
                        {
                            model.model_sha256 = Some(
                                "147d5aa88c2101237358e17796cf3a227cead1ec304ec34b465bb08e9d952965"
                                    .to_string(),
                            );
                        }
                    }
                    return Ok(models);
                }
            }
            Err(e) => {
                // A corrupt user file must never be silently overwritten: the
                // user's edits stay on disk while we run on the in-memory default.
                tracing::warn!(
                    "Failed to parse {}: {e}; using default embedder list in memory without overwriting existing file",
                    config_path.display()
                );
                return Ok(default_models);
            }
        }
    }

    // Write default to file so user can edit it (only when the file is absent
    // or was an empty, valid list)
    if let Ok(json_content) = serde_json::to_string_pretty(&auto_generated_defaults()) {
        if let Err(e) = fs::write(&config_path, json_content) {
            tracing::warn!(
                "Failed to write default embedder config to {}: {e}",
                config_path.display()
            );
        } else {
            // The default file was just materialized, so a first-run model
            // choice may still be offered: record it as pending for the
            // next interactive command. This path runs inside daemon and
            // serving processes, so it never prompts here.
            crate::model_choice::record_pending_choice(config_dir);
        }
    }

    Ok(default_models)
}

fn find_existing_cache_dir() -> Result<PathBuf> {
    // Same reason as above: $HOME is absent on Windows, and an empty home turns
    // every candidate below into a relative path, which is how a 523 MB model
    // cache ends up inside the caller's working directory.
    let home_path =
        dirs::home_dir().context("Could not find home directory ($HOME / USERPROFILE unset)")?;

    // Adopt the Neuro cache pattern: share models across all Neuro* tools
    let primary_neuro_cache = home_path.join(".cache/neuro/models/fastembed");

    // Other common fastembed caches we should check to avoid re-downloading
    let cache_dirs = vec![
        primary_neuro_cache.clone(),
        home_path.join(".cache/fastembed"),
        home_path.join(".cache/huggingface/hub"),
    ];

    for dir in cache_dirs {
        if dir.exists()
            && dir
                .read_dir()
                .map(|mut i| i.next().is_some())
                .unwrap_or(false)
        {
            return Ok(dir); // Return the first valid existing cache directory
        }
    }

    Ok(primary_neuro_cache)
}

/// The model this process is configured to use: the one NEUROSTRATA_MODEL names
/// when it is acceptable, otherwise the first acceptable entry.
fn configured_model() -> Result<AcceptableEmbedder> {
    pick_configured_model(get_acceptable_embedders()?)
}

/// [`configured_model`] against an explicit config directory.
pub(crate) fn configured_model_in(config_dir: &Path) -> Result<AcceptableEmbedder> {
    pick_configured_model(get_acceptable_embedders_in(config_dir)?)
}

fn pick_configured_model(acceptable_models: Vec<AcceptableEmbedder>) -> Result<AcceptableEmbedder> {
    let env_model = std::env::var("NEUROSTRATA_MODEL").unwrap_or_default();
    Ok(acceptable_models
        .iter()
        .find(|m| m.model_name.eq_ignore_ascii_case(&env_model))
        .unwrap_or(&acceptable_models[0])
        .clone())
}

/// The embedding width of the configured model, without loading the model.
///
/// Backup and restore need only this number to open the store. Loading the model
/// can mean a 523 MB download, which is the wrong thing to require on the fresh
/// or offline machine a restore is usually run on.
pub fn configured_dimensions() -> Result<usize> {
    Ok(configured_model()?.dimensions)
}

/// Token ceiling for one embedding. Roughly covers MAX_SYMBOL_CHARS of source
const MAX_EMBED_TOKENS: usize = 2048;

/// Forward-pass micro-batch size for local ONNX inference (`FastEmbedder::embed_batch`).
/// `fastembed` defaults `None` to `batch_size = 256` and pads every item in a batch
/// up to the longest item's token length (up to `MAX_EMBED_TOKENS = 2048`), which
/// allocates `[256, 12, 2048, 2048]` self-attention tensors (>22 GB) and OOM-kills
/// the daemon on large files. `1` embeds each text at its own actual token length.
const FAST_EMBED_BATCH_SIZE: usize = 1;

/// Arena allocation is disabled by default; `NEUROSTRATA_EMBED_ARENA` with
/// one of the house truthy spellings restores it for operators who measured a
/// benefit. Mirrors the HYBRID_SEARCH convention (1/true/yes/on).
fn arena_disabled_for(value: Option<&str>) -> bool {
    let enabled = matches!(
        value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    );
    !enabled
}

/// Shared process-wide ONNX Runtime [`ort::environment::Environment`].
///
/// `ort 2.0.0-rc.15` enforces a single `Environment` per process (`ort::init().build()`).
/// Sharing this singleton across `FastEmbedder`, `GlinerExtractor`, and `LayaOnnx`
/// guarantees they never collide regardless of initialization order.
pub fn shared_ort_environment() -> Result<&'static ort::environment::Environment> {
    static ORT_ENV: std::sync::OnceLock<
        std::result::Result<ort::environment::Environment, String>,
    > = std::sync::OnceLock::new();
    match ORT_ENV.get_or_init(|| ort::init().build().map_err(|e| e.to_string())) {
        Ok(env) => Ok(env),
        Err(msg) => Err(anyhow!(
            "Failed to initialize ONNX Runtime environment: {msg}"
        )),
    }
}

/// The execution-provider list the constructor installs for the arena
/// decision: a pinned CPU provider with the arena allocator off, or `None` to
/// leave fastembed's default. Extracted so the decision-to-wiring path is a
/// value a test can inspect without loading a model.
fn arena_execution_providers(disabled: bool) -> Option<Vec<ort::ep::ExecutionProviderDispatch>> {
    if disabled {
        Some(vec![ort::ep::CPU::default()
            .with_arena_allocator(false)
            .build()])
    } else {
        None
    }
}

/// The pin gate `FastEmbedder::new` applies to the configured model: a
/// local entry without a pinned SHA-256 fails closed, so a hand-authored
/// or setup-written entry can never load unverified.
fn pinned_sha_for(target_model: &AcceptableEmbedder) -> Result<&str> {
    target_model.model_sha256.as_deref().ok_or_else(|| {
        anyhow!(
            "Embedder supply-chain error: model '{}' has no pinned SHA-256",
            target_model.model_name
        )
    })
}

/// Verify the SHA-256 of the exact model file fastembed loads for
/// `model` against a pinned value. The file is located through
/// fastembed's own registry entry (`model_code` / `model_file`), in the
/// HuggingFace cache layout — never by filename guesswork: quantized
/// and optimized variants load files a `model.onnx`-only search cannot
/// see, and a sibling model's file must never satisfy this model's pin.
/// Returns `Err` on mismatch or when the file cannot be located
/// unambiguously (fail-closed).
pub fn verify_model_checksum(
    cache_dir: &Path,
    model: &EmbeddingModel,
    pinned_sha: &str,
) -> Result<()> {
    let model_name = format!("{model:?}");
    let info = TextEmbedding::get_model_info(model).map_err(|e| {
        anyhow!("Embedder supply-chain error: no fastembed model info for '{model_name}': {e}")
    })?;
    let model_dir = cache_dir.join(format!("models--{}", info.model_code.replace('/', "--")));
    let snapshot = resolve_snapshot_dir(&model_dir, &model_name, pinned_sha)?;
    let file = snapshot.join(&info.model_file);
    if !file.is_file() {
        return Err(anyhow!(
            "Embedder supply-chain error: model '{}' model file not found in cache for verification (pinned SHA: {}).",
            model_name, pinned_sha
        ));
    }
    let actual = hash_file_sha256(&file)?;
    if actual != pinned_sha {
        return Err(anyhow!(
            "Embedder supply-chain error: model '{}' checksum mismatch!\n  File: {}\n  Expected: {}\n  Actual: {}",
            model_name,
            file.display(),
            pinned_sha,
            actual
        ));
    }
    Ok(())
}

/// Select a cached model's snapshot directory: the one `refs/main`
/// names when it exists, else the only snapshot present. Several
/// snapshots with no ref to disambiguate fail closed rather than guess
/// which bytes the pin should be checked against.
fn resolve_snapshot_dir(model_dir: &Path, model_name: &str, pinned_sha: &str) -> Result<PathBuf> {
    let snapshots_dir = model_dir.join("snapshots");
    if let Ok(rev) = fs::read_to_string(model_dir.join("refs").join("main")) {
        let named = snapshots_dir.join(rev.trim());
        if named.is_dir() {
            return Ok(named);
        }
        // A refs/main that reads successfully but names a missing
        // snapshot is fail-closed — the ref is present and
        // authoritative, so falling through to enumeration
        // would verify a snapshot the ref does not name. Only a
        // genuinely unreadable refs/main (absent file) falls through
        // to the exactly-one-snapshot rule below.
        return Err(anyhow!(
            "Embedder supply-chain error: model '{}' refs/main names snapshot '{}', which is not in the cache; refusing to verify a different snapshot (pinned SHA: {}).",
            model_name,
            rev.trim(),
            pinned_sha
        ));
    }
    let mut snapshots: Vec<PathBuf> = match fs::read_dir(&snapshots_dir) {
        Ok(entries) => entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_dir())
            .collect(),
        Err(_) => Vec::new(),
    };
    match snapshots.len() {
        1 => Ok(snapshots.pop().expect("exactly one snapshot")),
        0 => Err(anyhow!(
            "Embedder supply-chain error: model '{}' model file not found in cache for verification (pinned SHA: {}).",
            model_name, pinned_sha
        )),
        count => Err(anyhow!(
            "Embedder supply-chain error: model '{}' has {} cached snapshots and no refs/main to select one; refusing to guess which to verify (pinned SHA: {}).",
            model_name, count, pinned_sha
        )),
    }
}

/// Streamed SHA-256 of a file (model files run to hundreds of MiB; the
/// digest must not require holding one in memory).
fn hash_file_sha256(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut file = fs::File::open(path).with_context(|| {
        format!(
            "Failed to read model file {} for checksum verification",
            path.display()
        )
    })?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer).with_context(|| {
            format!(
                "Failed to read model file {} for checksum verification",
                path.display()
            )
        })?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The opt-in idle-unload setting (Task 4), env-only:
/// `NEUROSTRATA_EMBEDDER_IDLE_UNLOAD_SECS`. Unset or `0` disables the
/// feature (pinned no-change behavior); a positive integer is the idle
/// threshold in seconds. Anything else is a hard error on the surfaces
/// that read it (embedder construction and the startup summary) -- the
/// Status arms never read it.
pub(crate) fn idle_unload_threshold_from_value(raw: Option<&str>) -> Result<Option<Duration>> {
    let Some(raw) = raw.map(str::trim).filter(|value| !value.is_empty()) else {
        return Ok(None);
    };
    let secs: u64 = raw.parse().map_err(|_| {
        anyhow::anyhow!(
            "NEUROSTRATA_EMBEDDER_IDLE_UNLOAD_SECS must be a non-negative integer number of seconds, got {raw:?}"
        )
    })?;
    Ok(if secs == 0 {
        None
    } else {
        Some(Duration::from_secs(secs))
    })
}

/// The idle-unload threshold from the process environment.
pub(crate) fn idle_unload_threshold() -> Result<Option<Duration>> {
    let raw = std::env::var("NEUROSTRATA_EMBEDDER_IDLE_UNLOAD_SECS").ok();
    idle_unload_threshold_from_value(raw.as_deref())
}

/// The startup-summary segment for the idle-unload setting (Spec
/// Feature 2): exactly `idle unload: on (threshold Ns)` or
/// `idle unload: off`.
pub(crate) fn format_idle_unload_segment(threshold: Option<Duration>) -> String {
    match threshold {
        Some(threshold) => format!("idle unload: on (threshold {}s)", threshold.as_secs()),
        None => "idle unload: off".to_string(),
    }
}

/// Builds one session plus the identity it serves. The factory is the
/// reload contract in one type: it owns ONLY the values captured at
/// startup (identity, canonical name, cache dir, max length) and
/// rebuilds a session from them -- it never re-runs model resolution
/// and never re-reads the environment or the embedders file.
type SessionFactory<T> = Box<dyn Fn() -> Result<(T, String)> + Send + Sync>;

struct IdleSessionState<T> {
    session: Option<T>,
    last_used: Instant,
}

/// A lazily unloaded embedding session (Task 4, Spec Feature 2).
///
/// The first load is eager: construction runs the factory once and
/// fails if it fails (startup stays fail-closed). Afterwards the
/// staleness check lives in the access path itself -- no daemon ticker:
/// an access that finds the session idle past the threshold drops it
/// and rebuilds it from the factory's captured values before serving.
/// A reload whose identity differs from the startup identity is a loud
/// error and the mismatched session is never installed; a failed
/// reload surfaces its error and the next access retries. Unset
/// threshold (`None`) disables unloading entirely.
///
/// Status surfaces read `snapshot_loaded` only: an atomic written on
/// load/unload transitions. It never locks the session state, never
/// calls the factory, and never touches the idle clock (Spec GT-7).
pub(crate) struct IdleSession<T> {
    factory: SessionFactory<T>,
    startup_identity: String,
    threshold: Option<Duration>,
    state: Mutex<IdleSessionState<T>>,
    loaded_snapshot: AtomicBool,
}

impl<T> IdleSession<T> {
    fn new<F>(factory: F, threshold: Option<Duration>) -> Result<Self>
    where
        F: Fn() -> Result<(T, String)> + Send + Sync + 'static,
    {
        Self::new_at(factory, threshold, Instant::now())
    }

    fn new_at<F>(factory: F, threshold: Option<Duration>, now: Instant) -> Result<Self>
    where
        F: Fn() -> Result<(T, String)> + Send + Sync + 'static,
    {
        let factory: SessionFactory<T> = Box::new(factory);
        // Eager first load: a factory that cannot build the startup
        // session fails construction, exactly as startup did before
        // idle unload existed.
        let (session, identity) = factory()?;
        Ok(Self {
            factory,
            startup_identity: identity,
            threshold,
            state: Mutex::new(IdleSessionState {
                session: Some(session),
                last_used: now,
            }),
            loaded_snapshot: AtomicBool::new(true),
        })
    }

    /// The identity captured at startup. Reloads must reproduce it.
    fn startup_identity(&self) -> &str {
        &self.startup_identity
    }

    /// Lock-free snapshot of whether a session is currently installed.
    /// The only session-state read status surfaces may make.
    fn snapshot_loaded(&self) -> bool {
        self.loaded_snapshot.load(Ordering::SeqCst)
    }

    /// Serve `f` with the session, dropping and rebuilding it first if
    /// it has been idle past the threshold.
    fn with_session<R>(&self, f: impl FnOnce(&mut T) -> Result<R>) -> Result<R> {
        self.with_session_at(Instant::now(), f)
    }

    fn with_session_at<R>(&self, now: Instant, f: impl FnOnce(&mut T) -> Result<R>) -> Result<R> {
        let mut state = self
            .state
            .lock()
            .map_err(|e| anyhow::anyhow!("session lock poisoned: {}", e))?;
        if let (Some(threshold), Some(_)) = (self.threshold, &state.session) {
            let idle = now.duration_since(state.last_used);
            if idle >= threshold {
                state.session = None;
                self.loaded_snapshot.store(false, Ordering::SeqCst);
                eprintln!("embedder session unloaded after {}s idle", idle.as_secs());
            }
        }
        if state.session.is_none() {
            let (session, identity) = (self.factory)()?;
            if identity != self.startup_identity {
                anyhow::bail!(
                    "embedder session reload produced model identity {:?}, but the startup identity is {:?}; refusing to serve a different model than startup resolved",
                    identity,
                    self.startup_identity
                );
            }
            eprintln!("embedder session reloaded (model: {})", identity);
            state.session = Some(session);
            self.loaded_snapshot.store(true, Ordering::SeqCst);
        }
        state.last_used = now;
        let session = state
            .session
            .as_mut()
            .expect("the session is installed by the reload branch above");
        f(session)
    }
}

/// Local fastembed-backed embedding provider.
pub struct FastEmbedder {
    session: Arc<IdleSession<TextEmbedding>>,
    model_name: String,
    dimensions: usize,
}

impl FastEmbedder {
    pub fn new() -> Result<Self> {
        // The arena opt-out is read here, on the startup surface, so the
        // construction path below — and the reload factory inside it —
        // never re-reads the environment (the Task 4 factory contract).
        let disable_arena =
            arena_disabled_for(std::env::var("NEUROSTRATA_EMBED_ARENA").ok().as_deref());
        Self::with_model(configured_model()?, disable_arena)
    }

    /// Construct from an already-resolved model, without re-running
    /// resolution: startup paths that resolved the model once (and
    /// reported it) build from that same model. The idle-unload
    /// factory built here captures ONLY the values fixed at
    /// construction (model enum, cache dir, ORT environment, arena
    /// decision, identity), so a reload reproduces the startup
    /// session exactly; it never re-resolves the model and never
    /// re-reads any configuration source.
    fn with_model(target_model: AcceptableEmbedder, disable_arena: bool) -> Result<Self> {
        // The descriptor must describe the model actually loaded: an unknown
        // configured name silently falls back to NomicEmbedTextV15 (768 dims).
        let (model_enum, loaded_model_name, loaded_dims) =
            match EmbeddingModel::from_str(&target_model.model_name) {
                Ok(m) => (m, target_model.model_name.clone(), target_model.dimensions),
                Err(_) => {
                    tracing::warn!(
                    "Unknown embedding model '{}', falling back to NomicEmbedTextV15 (768 dims)",
                    target_model.model_name
                );
                    (
                        EmbeddingModel::NomicEmbedTextV15,
                        "NomicEmbedTextV15".to_string(),
                        768,
                    )
                }
            };

        let cache_dir = find_existing_cache_dir()?;

        if !cache_dir.exists() {
            fs::create_dir_all(&cache_dir)
                .context("Failed to create shared model cache directory")?;
        }

        eprintln!(
            "Initializing FastEmbedder with model: {} using cache: {:?}",
            target_model.model_name, cache_dir
        );

        let ort_env = shared_ort_environment()?.clone();
        static SAID_ARENA: std::sync::Once = std::sync::Once::new();
        SAID_ARENA.call_once(|| {
            eprintln!(
                "Embedding CPU arena: {}",
                if disable_arena {
                    "disabled"
                } else {
                    "enabled (NEUROSTRATA_EMBED_ARENA)"
                }
            );
        });

        // The values the startup session is constructed with, captured
        // once here: model enum, cache dir, ORT environment, arena
        // decision, and the identity the embedder reports. The factory
        // rebuilds ONLY the ONNX session from these captured values,
        // so a reload can only ever reproduce the startup session.
        let captured_model_enum = model_enum.clone();
        let captured_cache_dir = cache_dir.clone();
        let captured_identity = loaded_model_name.clone();
        let factory = move || -> Result<(TextEmbedding, String)> {
            // fastembed defaults to 512 tokens regardless of what the model supports
            // (NomicEmbedTextV15 handles 8192), and truncates silently. Anything above
            // what ingestion actually stores per symbol is wasted, so this is set to
            // match MAX_SYMBOL_CHARS in src/parser/ingest.rs -- change the two together.
            // Texts here are embedded one at a time at their own length, so the
            // arena ratchets up and never gives it back. This applies to every
            // local embedding (ingest, daemon queries, MCP calls), not only
            // ingest. Set NEUROSTRATA_EMBED_ARENA=1 to keep the previous
            // default; any other value (or unset) disables the arena.
            let mut init = TextInitOptions::new(captured_model_enum.clone())
                .with_environment(ort_env.clone())
                .with_cache_dir(captured_cache_dir.clone())
                .with_max_length(MAX_EMBED_TOKENS);
            // Deliberately pin the CPU execution provider: arena allocation is a
            // CPU-provider setting, and this build's fastembed/ort path is CPU-only.
            // A future non-CPU provider needs its own policy rather than inheriting
            // this one silently.
            if let Some(providers) = arena_execution_providers(disable_arena) {
                init = init.with_execution_providers(providers);
            }
            let model = TextEmbedding::try_new(init.with_show_download_progress(true))?;
            Ok((model, captured_identity.clone()))
        };
        let session = IdleSession::new(factory, idle_unload_threshold()?)?;

        // Supply-chain verification: the configured entry must carry a
        // pinned SHA-256, and the file fastembed actually loaded (the
        // resolved enum above, fallback included) must hash to it. This
        // protects against cache poisoning attacks where an attacker
        // could replace a downloaded model.
        let pinned_sha = pinned_sha_for(&target_model)?;
        verify_model_checksum(&cache_dir, &model_enum, pinned_sha)?;

        Ok(Self {
            session: Arc::new(session),
            model_name: loaded_model_name,
            dimensions: loaded_dims,
        })
    }
}

#[async_trait]
impl Embedder for FastEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        // Model inference is CPU-bound and can stall the async runtime when
        // called from a Tokio task. Isolate it on the blocking thread pool.
        // The idle-unload staleness check lives inside `with_session`:
        // a session idle past the threshold is dropped and rebuilt from
        // the startup-captured values before this call is served.
        let session = Arc::clone(&self.session);
        let text = text.to_owned();
        let mut embeddings = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>> {
            session.with_session(|model| {
                model
                    .embed(vec![&*text], None)
                    .context("fastembed embed call failed")
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("embed task panicked: {}", e))??;
        embeddings
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Embedding model returned an empty vector batch"))
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        // Model inference is CPU-bound and can stall the async runtime when
        // called from a Tokio task. Isolate it on the blocking thread pool.
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let session = Arc::clone(&self.session);
        let expected = texts.len();
        let texts = texts.to_vec();
        let embeddings = tokio::task::spawn_blocking(move || -> Result<Vec<Vec<f32>>> {
            session.with_session(|model| {
                model
                    .embed(texts, Some(FAST_EMBED_BATCH_SIZE))
                    .context("fastembed embed_batch call failed")
            })
        })
        .await
        .map_err(|e| anyhow::anyhow!("embed task panicked: {}", e))??;
        if embeddings.len() != expected {
            anyhow::bail!(
                "fastembed embed_batch returned {} vectors for {} texts",
                embeddings.len(),
                expected
            );
        }
        Ok(embeddings)
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embedder_id(&self) -> &str {
        &self.model_name
    }

    fn session_loaded_snapshot(&self) -> Option<bool> {
        Some(self.session.snapshot_loaded())
    }

    fn embedding_descriptor(&self) -> crate::traits::EmbeddingDescriptor {
        crate::traits::EmbeddingDescriptor {
            provider: "fastembed".into(),
            model: self.model_name.clone(),
            revision: "onnx".into(),
            dims: self.dimensions,
            normalization: "unknown".into(),
            preprocessing: "unknown".into(),
            distance_metric: "cosine".into(),
        }
    }
}

pub struct RemoteEmbedder {
    client: reqwest::Client,
    base_url: String,
    api_key: String,
    api_model: String,
    dimensions: usize,
    model_name: String,
}

/// The most inputs one remote embedding request carries.
///
/// A provider's own ceiling is not a universal constant -- one provider's
/// documented example is 2 048 inputs, but that is that provider's number,
/// and the refusal differs per provider (`400`, `413`, or a silent
/// truncation). Sending one request per file made a whole generation depend
/// on one file's symbol count, so the request carries a budget of its own
/// and the batch is split to fit it.
const REMOTE_EMBED_MAX_INPUTS: usize = 64;

/// The most characters one remote embedding request carries.
///
/// The remote API does not expose a request-side token count, so the
/// character count stands in for the token budget: it bounds the request
/// body without guessing the provider's tokenizer.
const REMOTE_EMBED_MAX_CHARS: usize = 32 * 1024;

/// How many refusal reasons one degraded batch reports. The count of
/// refusals is exact; only the samples are capped.
const REMOTE_EMBED_MAX_WARNING_SAMPLES: usize = 8;

/// Splits `texts` into request-sized chunks: at most
/// [`REMOTE_EMBED_MAX_INPUTS`] inputs and [`REMOTE_EMBED_MAX_CHARS`]
/// characters per chunk. The chunks are `(start, end)` index ranges into
/// `texts`, so the caller never copies the payloads.
///
/// A single text longer than the character budget is not split: it goes
/// alone in its own chunk, and the provider's answer decides its fate.
fn remote_request_chunks(texts: &[String]) -> Vec<(usize, usize)> {
    let mut chunks: Vec<(usize, usize)> = Vec::new();
    let mut start = 0usize;
    let mut chars = 0usize;
    for (i, text) in texts.iter().enumerate() {
        let len = text.chars().count();
        let over_inputs = i - start >= REMOTE_EMBED_MAX_INPUTS;
        let over_chars = chars + len > REMOTE_EMBED_MAX_CHARS;
        if i > start && (over_inputs || over_chars) {
            chunks.push((start, i));
            start = i;
            chars = 0;
        }
        chars += len;
    }
    if start < texts.len() {
        chunks.push((start, texts.len()));
    }
    chunks
}

/// A typed carrier for the response status so the retry loop can downcast to
/// it from the anyhow chain (reqwest::StatusCode does not implement
/// std::error::Error itself).
#[derive(Debug)]
struct StatusError(reqwest::StatusCode);

impl fmt::Display for StatusError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for StatusError {}

impl RemoteEmbedder {
    pub fn new(
        base_url: String,
        api_key: String,
        api_model: String,
        dimensions: usize,
    ) -> Result<Self> {
        if dimensions == 0 {
            // Dimension validation becomes disabled (try_embed only checks len
            // when dimensions>0); surface that loudly at construction.
            tracing::warn!(
                "RemoteEmbedder configured with dimensions=0; dimension validation is disabled"
            );
        }
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            base_url,
            api_key,
            api_model: api_model.clone(),
            dimensions,
            model_name: api_model,
        })
    }

    async fn embed_with_retry(&self, text: &str) -> Result<Vec<f32>> {
        let mut attempts = 0;
        loop {
            match self.try_embed(text).await {
                Ok(embedding) => return Ok(embedding),
                Err(e) => {
                    attempts += 1;
                    if attempts >= 2 {
                        return Err(e);
                    }
                    if let Some(s) = e.downcast_ref::<StatusError>() {
                        if s.0 == reqwest::StatusCode::TOO_MANY_REQUESTS || s.0.is_server_error() {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
    }

    async fn embed_batch_with_retry(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let mut attempts = 0;
        loop {
            match self.try_embed_batch(texts).await {
                Ok(embeddings) => return Ok(embeddings),
                Err(e) => {
                    attempts += 1;
                    if attempts >= 2 {
                        return Err(e);
                    }
                    if let Some(s) = e.downcast_ref::<StatusError>() {
                        if s.0 == reqwest::StatusCode::TOO_MANY_REQUESTS || s.0.is_server_error() {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                            continue;
                        }
                    }
                    return Err(e);
                }
            }
        }
    }

    /// The batch embed, split into request-sized chunks.
    ///
    /// `degrade` decides what a provider refusal means. With `degrade` set,
    /// a chunk the provider refuses (`400`/`413`) is halved and retried down
    /// to single items, and a single item the provider still refuses becomes
    /// a warning: the item is skipped and the rest of the batch lands. With
    /// `degrade` clear, the first refusal ends the batch with the provider's
    /// own error, which is what the all-or-nothing `embed_batch` promises.
    ///
    /// Every other error keeps the abort path: a `429`/`5xx` is retried by
    /// [`Self::embed_batch_with_retry`], and anything else fails the batch.
    ///
    /// The result is in input order: the chunks are sent in order and each
    /// chunk's vectors keep their request order, so the concatenation is
    /// the input order.
    async fn embed_batch_internal(
        &self,
        texts: &[String],
        degrade: bool,
    ) -> Result<crate::traits::BatchEmbedOutcome> {
        let mut outcome = crate::traits::BatchEmbedOutcome {
            embeddings: Vec::with_capacity(texts.len()),
            warnings: Vec::new(),
        };
        if texts.is_empty() {
            return Ok(outcome);
        }
        // The work list is a queue of `(start, end)` index ranges, and it is
        // drained strictly in index order: a range that the provider refuses
        // is split and its halves are put back at the FRONT, so the halves are
        // embedded before any later range. The result is therefore in input
        // order no matter how often a range is split.
        let mut pending: std::collections::VecDeque<(usize, usize)> =
            remote_request_chunks(texts).into_iter().collect();
        while let Some((start, end)) = pending.pop_front() {
            let chunk = &texts[start..end];
            match self.embed_batch_with_retry(chunk).await {
                Ok(vectors) => {
                    if vectors.len() != chunk.len() {
                        anyhow::bail!(
                            "remote embedding batch returned {} vectors for {} texts",
                            vectors.len(),
                            chunk.len()
                        );
                    }
                    outcome.embeddings.extend(vectors.into_iter().map(Some));
                }
                Err(e) => {
                    let refused = e
                        .downcast_ref::<StatusError>()
                        .map(|s| s.0)
                        .filter(|status| {
                            *status == reqwest::StatusCode::BAD_REQUEST
                                || *status == reqwest::StatusCode::PAYLOAD_TOO_LARGE
                        });
                    let Some(status) = refused else {
                        return Err(e);
                    };
                    if end - start > 1 {
                        // The provider will not take this request as it
                        // stands: halve it and try again, down to single
                        // items.
                        let mid = start + (end - start) / 2;
                        pending.push_front((mid, end));
                        pending.push_front((start, mid));
                        continue;
                    }
                    if !degrade {
                        return Err(e);
                    }
                    // A single item the provider refuses: degrade it to a
                    // warning. The rest of the batch still lands.
                    if outcome.warnings.len() < REMOTE_EMBED_MAX_WARNING_SAMPLES {
                        outcome.warnings.push(format!(
                            "remote embedding refused one input ({}): {}",
                            status, e
                        ));
                    }
                    outcome.embeddings.push(None);
                }
            }
        }
        Ok(outcome)
    }

    async fn try_embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        // R13: Egress authorization check — fail-closed if the token or
        // namespace/data-class allowlist doesn't authorize this call. Concrete
        // authorization gate (not a deferral): batch embed call is refused
        // before any remote data leaves the node.
        if self.api_key.is_empty() {
            eprintln!(
                "[REMOTE-EMBED-REFUSED] authorization failed: empty api_key for RemoteEmbedder ({})",
                self.base_url
            );
            return Err(anyhow::anyhow!(
                "Remote embedding refused: authorization check failed (empty api_key). Fail-closed per R13 privacy/egress authorization."
            ));
        }
        let response = self
            .client
            .post(&format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": texts,
                "model": self.api_model
            }))
            .send()
            .await
            .context("HTTP request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow::Error::new(StatusError(status))
                .context(format!("API error {}: {}", status, body)));
        }

        let json: serde_json::Value = response.json().await.context("Failed to parse response")?;
        // Embedding-space identity persistence: a malformed remote batch must
        // be rejected outright, mirroring the single-text path (Composite
        // Recommendation #11). A zero vector would silently poison cosine
        // distances, and a length that disagrees with self.dimensions would be
        // unmixable with every stored vector in the space.
        let data = json["data"]
            .as_array()
            .context("Missing embedding data in response")?;
        if data.len() != texts.len() {
            anyhow::bail!(
                "remote embedding batch returned {} vectors for {} texts",
                data.len(),
                texts.len()
            );
        }
        let mut by_index: Vec<Option<Vec<f32>>> = vec![None; texts.len()];
        for (pos, item) in data.iter().enumerate() {
            let index = match item.get("index") {
                Some(v) if !v.is_null() => {
                    let idx = v.as_u64().ok_or_else(|| {
                        anyhow!("remote embedding contained invalid non-integer index")
                    })? as usize;
                    if idx >= by_index.len() {
                        anyhow::bail!(
                            "remote embedding index {} out of bounds for batch of {}",
                            idx,
                            by_index.len()
                        );
                    }
                    idx
                }
                _ => pos,
            };
            if by_index[index].is_some() {
                anyhow::bail!(
                    "remote embedding batch contained duplicate entry for index {}",
                    index
                );
            }
            let arr = item["embedding"]
                .as_array()
                .context("Missing embedding in response")?;
            if arr.is_empty() {
                anyhow::bail!("remote embedding was empty");
            }
            let mut embedding = Vec::with_capacity(arr.len());
            for v in arr {
                let val = v.as_f64().ok_or_else(|| {
                    anyhow!("remote embedding contained non-finite or non-numeric value")
                })?;
                if !val.is_finite() {
                    anyhow::bail!("remote embedding contained non-finite or non-numeric value");
                }
                embedding.push(val as f32);
            }
            if self.dimensions > 0 && embedding.len() != self.dimensions {
                anyhow::bail!(
                    "remote embedding dimension mismatch: expected {}, got {}",
                    self.dimensions,
                    embedding.len()
                );
            }
            by_index[index] = Some(embedding);
        }
        Ok(by_index
            .into_iter()
            .map(|opt| {
                opt.ok_or_else(|| {
                    anyhow!("remote embedding batch was missing an entry for every text")
                })
            })
            .collect::<Result<Vec<Vec<f32>>>>()?)
    }

    async fn try_embed(&self, text: &str) -> Result<Vec<f32>> {
        // R13: Egress authorization check — fail-closed if the token or
        // namespace/data-class allowlist doesn't authorize this call.
        // Concrete authorization gate (not a deferral): embed call is refused
        // before any remote data leaves the node. Metric emitted via log line.
        if self.api_key.is_empty() {
            eprintln!(
                "[REMOTE-EMBED-REFUSED] authorization failed: empty api_key for RemoteEmbedder ({})",
                self.base_url
            );
            return Err(anyhow::anyhow!(
                "Remote embedding refused: authorization check failed (empty api_key). Fail-closed per R13 privacy/egress authorization."
            ));
        }
        let response = self
            .client
            .post(&format!("{}/embeddings", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .json(&serde_json::json!({
                "input": text,
                "model": self.api_model
            }))
            .send()
            .await
            .context("HTTP request failed")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            // The status travels in the error chain so the retry loop in
            // `embed_with_retry` can downcast and tell retryable server
            // errors from fatal client ones.
            return Err(anyhow::Error::new(StatusError(status))
                .context(format!("API error {}: {}", status, body)));
        }

        let json: serde_json::Value = response.json().await.context("Failed to parse response")?;
        // Embedding-space identity persistence: a malformed remote vector must
        // be rejected outright. Substituting 0.0 for non-numeric or non-finite
        // elements would silently poison cosine distances, and a length that
        // disagrees with self.dimensions would be unmixable with every stored
        // vector in the space. Fail-closed (Composite Recommendation #11).
        let arr = json["data"][0]["embedding"]
            .as_array()
            .context("Missing embedding in response")?;
        if arr.is_empty() {
            anyhow::bail!("remote embedding was empty");
        }
        let mut embedding = Vec::with_capacity(arr.len());
        for v in arr {
            let val = v.as_f64().ok_or_else(|| {
                anyhow!("remote embedding contained non-finite or non-numeric value")
            })?;
            if !val.is_finite() {
                anyhow::bail!("remote embedding contained non-finite or non-numeric value");
            }
            embedding.push(val as f32);
        }
        if self.dimensions > 0 && embedding.len() != self.dimensions {
            anyhow::bail!(
                "remote embedding dimension mismatch: expected {}, got {}",
                self.dimensions,
                embedding.len()
            );
        }
        Ok(embedding)
    }
}

#[async_trait]
impl Embedder for RemoteEmbedder {
    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        self.embed_with_retry(text).await
    }

    async fn embed_batch(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let outcome = self.embed_batch_internal(texts, false).await?;
        // The all-or-nothing contract: a refused item fails the batch rather
        // than handing back a placeholder vector.
        if let Some(missing) = outcome.embeddings.iter().position(|e| e.is_none()) {
            anyhow::bail!(
                "remote embedding refused input {} of {}: {}",
                missing,
                outcome.embeddings.len(),
                outcome
                    .warnings
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "no reason recorded".to_string())
            );
        }
        Ok(outcome
            .embeddings
            .into_iter()
            .map(|e| e.expect("checked above"))
            .collect())
    }

    async fn embed_batch_degradable(
        &self,
        texts: &[String],
    ) -> Result<crate::traits::BatchEmbedOutcome> {
        self.embed_batch_internal(texts, true).await
    }

    fn dimensions(&self) -> usize {
        self.dimensions
    }

    fn embedder_id(&self) -> &str {
        &self.model_name
    }

    fn embedding_descriptor(&self) -> crate::traits::EmbeddingDescriptor {
        crate::traits::EmbeddingDescriptor {
            provider: "openai-compatible".into(),
            model: self.model_name.clone(),
            revision: "unknown".into(),
            dims: self.dimensions,
            normalization: "unknown".into(),
            preprocessing: "unknown".into(),
            distance_metric: "cosine".into(),
        }
    }
}

/// The remote API model a remote entry is served under when it names
/// no `api_model` of its own. Shared by `build_embedder` (which
/// constructs the `RemoteEmbedder` with it) and the store's
/// configured-descriptor synthesis (which must predict that same
/// descriptor), so the default cannot drift between the two sites.
pub(crate) const DEFAULT_REMOTE_API_MODEL: &str = "text-embedding-3-small";

pub fn build_embedder() -> Result<Arc<dyn Embedder>> {
    let target_model = configured_model()?;

    let emb: Arc<dyn Embedder> = if let (Some(base_url), Some(api_key_env)) =
        (&target_model.base_url, &target_model.api_key_env)
    {
        let api_key = std::env::var(api_key_env)
            .with_context(|| format!("API key env var {} not set", api_key_env))?;
        let api_model = target_model
            .api_model
            .clone()
            .unwrap_or_else(|| DEFAULT_REMOTE_API_MODEL.to_string());

        eprintln!(
            "Initializing RemoteEmbedder with model: {} at {}",
            api_model, base_url
        );
        Arc::new(RemoteEmbedder::new(
            base_url.clone(),
            api_key,
            api_model,
            target_model.dimensions,
        )?) as Arc<dyn Embedder>
    } else {
        Arc::new(FastEmbedder::new()?) as Arc<dyn Embedder>
    };

    // Embedding-space identity must resolve at startup: descriptors get
    // persisted on namespace metadata and compared with `compatible`, so an
    // embedder that reports the "unknown" descriptor would defeat that check.
    let descriptor = emb.embedding_descriptor();
    if descriptor.compatible(&crate::traits::EmbeddingDescriptor::unknown()) {
        anyhow::bail!(
            "embedder '{}' reports the unknown embedding descriptor",
            emb.embedder_id()
        );
    }
    eprintln!(
        "Embedding space: provider={} model={} dims={}",
        descriptor.provider, descriptor.model, descriptor.dims
    );
    Ok(emb)
}

#[cfg(test)]
mod tests {
    #[test]
    fn arena_default_is_disabled_with_an_explicit_opt_in() {
        for off in [None, Some("0"), Some(""), Some("no"), Some("false")] {
            assert!(super::arena_disabled_for(off), "{off:?} is not opt-in");
        }
        for on in ["1", "true", "TRUE", "yes", "on"] {
            assert!(!super::arena_disabled_for(Some(on)), "{on:?} opts in");
        }
    }

    /// The decision reaches the provider wiring as a value, not just as text
    /// in the file: an off-by-default arena yields the pinned CPU provider,
    /// and an opt-in yields fastembed's default (no override).
    #[test]
    fn the_arena_policy_reaches_the_provider_wiring() {
        assert!(
            super::arena_execution_providers(true).is_some(),
            "the default must install the no-arena CPU provider"
        );
        assert!(
            super::arena_execution_providers(false).is_none(),
            "an opt-in must leave fastembed's provider default"
        );
    }

    /// Constructor guard (#164): pins the provider helper's arena-off
    /// setting and the constructor's wiring of the computed provider list
    /// into the embedding options, so neither flipping the boolean nor
    /// deleting the constructor hook can pass unnoticed. The wiring
    /// needles are assembled with `format!` so they do not appear
    /// verbatim in this test's own source.
    #[test]
    fn the_constructor_disables_the_cpu_arena() {
        let src = include_str!("embed.rs");
        let helper_body = src
            .split("fn arena_execution_providers(")
            .nth(1)
            .and_then(|s| s.split("\n}\n").next())
            .expect("arena_execution_providers present");
        assert!(
            helper_body.contains(".with_arena_allocator(false)"),
            "arena_execution_providers must disable the CPU arena allocator"
        );
        let ctor_body = src
            .split("let disable_arena =")
            .nth(1)
            .and_then(|s| {
                s.split("TextEmbedding::try_new(init.with_show_download_progress(true))")
                    .next()
            })
            .expect("FastEmbedder::new arena wiring present");
        let call_needle = format!("arena_execution_providers({})", "disable_arena");
        let wiring_needle = format!("init.with_execution_providers({})", "providers");
        assert!(
            ctor_body.contains(&call_needle) && ctor_body.contains(&wiring_needle),
            "FastEmbedder::new must wire the arena provider decision into the embedding options"
        );
    }

    #[test]
    fn embed_batch_bounds_fastembed_forward_pass_batch_size_to_one() {
        assert_eq!(
            super::FAST_EMBED_BATCH_SIZE,
            1,
            "FAST_EMBED_BATCH_SIZE must stay 1 so embed_batch never pads a multi-hundred-symbol file to [256, 2048]"
        );
        let src = include_str!("embed.rs");
        let batch_body = src
            .split("async fn embed_batch(&self, texts: &[String])")
            .nth(1)
            .and_then(|s| s.split("fn dimensions(&self)").next())
            .expect("FastEmbedder::embed_batch present");
        let needle = format!(".embed(texts, Some({}))", "FAST_EMBED_BATCH_SIZE");
        assert!(
            batch_body.contains(&needle),
            "FastEmbedder::embed_batch must pass Some(FAST_EMBED_BATCH_SIZE) to guard.embed"
        );
    }

    use super::*;

    /// Read one full HTTP request (headers + declared body) off the socket,
    /// so responding cannot race the client still writing.
    async fn read_http_request(sock: &mut tokio::net::TcpStream) -> String {
        use tokio::io::AsyncReadExt;
        let mut acc: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = sock.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            acc.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&acc).to_string();
            if let Some(idx) = text.find("\r\n\r\n") {
                let mut content_length = 0usize;
                for line in text[..idx].lines() {
                    if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        content_length = rest.trim().parse().unwrap_or(0);
                    }
                }
                if acc.len() >= idx + 4 + content_length {
                    break;
                }
            }
        }
        String::from_utf8_lossy(&acc).to_string()
    }

    #[test]
    fn the_configured_width_is_known_without_loading_a_model() {
        assert!(configured_dimensions().expect("embedders resolve") > 0);
    }

    #[tokio::test]
    async fn remote_embedder_happy_path() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let req = read_http_request(&mut sock).await;
            assert!(req.contains("POST /embeddings"), "request: {}", req);
            assert!(
                req.to_ascii_lowercase()
                    .contains("authorization: bearer test-key"),
                "request: {}",
                req
            );
            let body = r#"{"data":[{"embedding":[0.25,0.5,0.75,1.0]}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            use tokio::io::AsyncWriteExt;
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });
        let embedder = RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let v = embedder.embed("hello").await.unwrap();
        assert_eq!(v, vec![0.25f32, 0.5, 0.75, 1.0]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn remote_embedder_batch_preserves_order_without_index() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let _ = read_http_request(&mut sock).await;
            let body =
                r#"{"data":[{"embedding":[1.0,2.0,3.0,4.0]},{"embedding":[5.0,6.0,7.0,8.0]}]}"#;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            use tokio::io::AsyncWriteExt;
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });
        let embedder = RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let v = embedder
            .embed_batch(&["a".to_string(), "b".to_string()])
            .await
            .unwrap();
        assert_eq!(
            v,
            vec![vec![1.0f32, 2.0, 3.0, 4.0], vec![5.0, 6.0, 7.0, 8.0]]
        );
        server.await.unwrap();
    }

    /// Composite Recommendation #11: a remote response whose embedding is
    /// non-numeric, non-finite, empty, or dimension-mismatched must fail the
    /// embed call instead of silently surviving as 0.0 fills.
    #[tokio::test]
    async fn remote_embedder_rejects_non_numeric_and_mismatched_embeddings() {
        let bad_embeddings: Vec<(&str, &str)> = vec![
            ("non-numeric element", r#"[1.0, "not-a-number", 3.0, 4.0]"#),
            ("dimension mismatch (expected 4)", r#"[1.0, 2.0, 3.0]"#),
            ("empty", r#"[]"#),
        ];
        for (case, body_vec) in bad_embeddings {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let _ = read_http_request(&mut sock).await;
                let body = format!(r#"{{"data":[{{"embedding":{}}}]}}"#, body_vec);
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                use tokio::io::AsyncWriteExt;
                sock.write_all(resp.as_bytes()).await.unwrap();
                sock.flush().await.unwrap();
            });
            let embedder = RemoteEmbedder::new(
                format!("http://{}", addr),
                "test-key".to_string(),
                "test-model".to_string(),
                4,
            )
            .unwrap();
            let err = embedder.embed("hello").await.expect_err(case);
            assert!(
                err.to_string().contains("remote embedding"),
                "case '{}' produced an unexpected error: {}",
                case,
                err
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn remote_embedder_retries_on_500() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let mut handled = 0usize;
            for attempt in 0..2 {
                let (mut sock, _) = listener.accept().await.expect("accept");
                let req = read_http_request(&mut sock).await;
                assert!(req.contains("POST /embeddings"), "request: {}", req);
                let (status_line, body) = if attempt == 0 {
                    ("HTTP/1.1 500 Internal Server Error", "server fail")
                } else {
                    (
                        "HTTP/1.1 200 OK",
                        r#"{"data":[{"embedding":[1.0,2.0,3.0,4.0]}]}"#,
                    )
                };
                let resp = format!(
                    "{}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status_line,
                    body.len(),
                    body
                );
                use tokio::io::AsyncWriteExt;
                sock.write_all(resp.as_bytes()).await.unwrap();
                sock.flush().await.unwrap();
                handled += 1;
            }
            handled
        });
        let embedder = RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let v = embedder.embed("retry me").await.unwrap();
        assert_eq!(v, vec![1.0f32, 2.0, 3.0, 4.0]);
        let handled = server.await.unwrap();
        assert_eq!(
            handled, 2,
            "server must see both the failed and the retried attempt"
        );
    }

    #[test]
    fn remote_embedder_dimensions_match() {
        let embedder = RemoteEmbedder::new(
            "http://localhost:1234".to_string(),
            "test-key".to_string(),
            "test-model".to_string(),
            768,
        )
        .unwrap();
        assert_eq!(embedder.dimensions(), 768);
    }

    #[test]
    fn remote_embedder_descriptor_is_resolved_not_unknown() {
        let embedder = RemoteEmbedder::new(
            "http://localhost:1234".to_string(),
            "test-key".to_string(),
            "test-model".to_string(),
            768,
        )
        .unwrap();
        let d = embedder.embedding_descriptor();
        assert_eq!(d.provider, "openai-compatible");
        assert_eq!(d.model, "test-model");
        assert_eq!(d.dims, 768);
        assert!(!d.compatible(&crate::traits::EmbeddingDescriptor::unknown()));
    }

    #[test]
    fn build_embedder_falls_back_to_local() {
        // When no remote config, should use local
        // This tests the factory logic
        let embedder = build_embedder();
        assert!(embedder.is_ok());
    }

    /// A fresh temp cache root for the verifier fixtures.
    fn fresh_cache_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ns-checksum-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Write `bytes` as `model_file` inside a fixture snapshot in the real
    /// fastembed/HuggingFace cache layout:
    /// `models--{model_code}/snapshots/testrev/{model_file}`.
    fn fixture_snapshot_file(
        cache: &Path,
        model_code: &str,
        model_file: &str,
        bytes: &[u8],
    ) -> PathBuf {
        let file = cache
            .join(format!("models--{}", model_code.replace('/', "--")))
            .join("snapshots")
            .join("testrev")
            .join(model_file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, bytes).unwrap();
        file
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn verify_model_checksum_fails_on_mismatch() {
        let dir = fresh_cache_dir("mismatch");
        fixture_snapshot_file(
            &dir,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model.onnx",
            b"not-the-model",
        );
        let err = verify_model_checksum(&dir, &EmbeddingModel::NomicEmbedTextV15, "deadbeef")
            .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_model_checksum_fails_when_no_file() {
        let dir = fresh_cache_dir("missing");
        let err = verify_model_checksum(&dir, &EmbeddingModel::NomicEmbedTextV15, "deadbeef")
            .unwrap_err();
        assert!(err.to_string().contains("model file not found"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_model_checksum_accepts_the_default_model_in_the_real_layout() {
        // The default flow must not regress: FP32 Nomic in the snapshot
        // layout verifies against the hash of the file fastembed loads.
        let dir = fresh_cache_dir("default-ok");
        let bytes = b"fp32-weights";
        fixture_snapshot_file(
            &dir,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model.onnx",
            bytes,
        );
        verify_model_checksum(&dir, &EmbeddingModel::NomicEmbedTextV15, &sha256_hex(bytes))
            .expect("the default model verifies against the file fastembed loads");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_hashes_the_file_fastembed_loads() {
        // NomicEmbedTextV15Q shares its snapshot with the FP32 file (the
        // pre-seeded cache shape): fastembed loads onnx/model_quantized.onnx,
        // so the quantized bytes' pin must pass and the FP32 sibling's
        // pin must fail — the old collector never saw the quantized file.
        let dir = fresh_cache_dir("quantized");
        let fp32 = b"fp32-sibling-bytes";
        let quantized = b"quantized-bytes";
        fixture_snapshot_file(
            &dir,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model.onnx",
            fp32,
        );
        fixture_snapshot_file(
            &dir,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model_quantized.onnx",
            quantized,
        );
        verify_model_checksum(
            &dir,
            &EmbeddingModel::NomicEmbedTextV15Q,
            &sha256_hex(quantized),
        )
        .expect("the quantized file's pin verifies");
        let err =
            verify_model_checksum(&dir, &EmbeddingModel::NomicEmbedTextV15Q, &sha256_hex(fp32))
                .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);

        // BGESmallENV15Q loads a root-level model_optimized.onnx; a decoy
        // onnx/model.onnx in the same snapshot must be ignored.
        let dir = fresh_cache_dir("optimized");
        let optimized = b"optimized-bytes";
        fixture_snapshot_file(
            &dir,
            "Qdrant/bge-small-en-v1.5-onnx-Q",
            "model_optimized.onnx",
            optimized,
        );
        fixture_snapshot_file(
            &dir,
            "Qdrant/bge-small-en-v1.5-onnx-Q",
            "onnx/model.onnx",
            b"decoy",
        );
        verify_model_checksum(
            &dir,
            &EmbeddingModel::BGESmallENV15Q,
            &sha256_hex(optimized),
        )
        .expect("the optimized file's pin verifies");
        let err =
            verify_model_checksum(&dir, &EmbeddingModel::BGESmallENV15Q, &sha256_hex(b"decoy"))
                .unwrap_err();
        assert!(err.to_string().contains("checksum mismatch"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_model_checksum_refuses_dangling_refs_main() {
        // A refs/main that reads successfully but names a snapshot
        // directory that does not exist is a fail-closed case.
        // Falling through to snapshot enumeration
        // would verify whichever snapshot happens to be present
        // instead of the one the ref names.
        let dir = fresh_cache_dir("dangling-ref");
        fixture_snapshot_file(
            &dir,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model.onnx",
            b"fp32-weights",
        );
        let model_dir = dir.join("models--nomic-ai--nomic-embed-text-v1.5");
        std::fs::create_dir_all(model_dir.join("refs")).unwrap();
        std::fs::write(model_dir.join("refs").join("main"), "missingrev").unwrap();
        let err = verify_model_checksum(
            &dir,
            &EmbeddingModel::NomicEmbedTextV15,
            &sha256_hex(b"fp32-weights"),
        )
        .unwrap_err();
        assert!(err.to_string().contains("refs/main"), "{err}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn verify_model_checksum_follows_a_valid_refs_main() {
        // Control for the dangling-ref refusal: a refs/main naming an
        // existing snapshot selects exactly that snapshot.
        let dir = fresh_cache_dir("valid-ref");
        let bytes = b"fp32-weights";
        fixture_snapshot_file(
            &dir,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model.onnx",
            bytes,
        );
        let model_dir = dir.join("models--nomic-ai--nomic-embed-text-v1.5");
        std::fs::create_dir_all(model_dir.join("refs")).unwrap();
        std::fs::write(model_dir.join("refs").join("main"), "testrev").unwrap();
        verify_model_checksum(&dir, &EmbeddingModel::NomicEmbedTextV15, &sha256_hex(bytes))
            .expect("a valid refs/main selects its snapshot");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn pin_gate_refuses_a_pinless_shortlist_entry() {
        // The loader backfills only the FP32 default's pin, so a
        // hand-authored shortlist entry without one reaches this gate
        // pinless and is refused with the actionable error. Extending
        // the backfill is a deliberate future change to this test and to
        // `hand_authored_shortlist_entry_without_pin_is_not_backfilled`.
        let entry = AcceptableEmbedder {
            model_name: "NomicEmbedTextV15Q".to_string(),
            dimensions: 768,
            base_url: None,
            api_key_env: None,
            api_model: None,
        generated_by: None,
            model_sha256: None,
        };
        let err = pinned_sha_for(&entry).unwrap_err();
        assert!(err.to_string().contains("no pinned SHA-256"), "{err}");
    }

    #[test]
    fn non_default_pick_pin_flows_from_writer_through_resolution_to_verification() {
        // The closest faithful FastEmbedder::new proxy that stays
        // hermetic: new() reads the global config dir and performs a real
        // model load, so this test runs the exact post-load sequence —
        // writer output -> configured_model_in resolution -> the pin gate
        // (pinned_sha_for) -> model-aware verify_model_checksum — for a
        // non-default pick. Which file gets hashed is proven against
        // fixture bytes by verify_hashes_the_file_fastembed_loads; the
        // emitted pin's correctness is the table constant asserted here.
        let previous = std::env::var("NEUROSTRATA_MODEL").ok();
        std::env::set_var("NEUROSTRATA_MODEL", "");

        let root = std::env::temp_dir().join(format!("ns-pin-proxy-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let config_dir = root.join("config");
        std::fs::create_dir_all(&config_dir).unwrap();
        let db_path = root.join("ladybug.db");
        let choice = crate::model_choice::candidates()
            .iter()
            .find(|c| c.model_name == "NomicEmbedTextV15Q")
            .expect("shortlist contains NomicEmbedTextV15Q");
        crate::model_choice::write_initial_embedders_file(&config_dir, choice, &db_path)
            .expect("write the pick");

        let resolved = configured_model_in(&config_dir).expect("resolve the pick");
        assert_eq!(resolved.model_name, "NomicEmbedTextV15Q");
        let pin = pinned_sha_for(&resolved).expect("the written pick carries a pin");
        assert_eq!(pin, choice.model_sha256);

        let model_enum = EmbeddingModel::from_str(&resolved.model_name).expect("enum parses");
        let cache = root.join("cache");
        let quantized = b"proxy-quantized-bytes";
        fixture_snapshot_file(
            &cache,
            "nomic-ai/nomic-embed-text-v1.5",
            "onnx/model_quantized.onnx",
            quantized,
        );
        verify_model_checksum(&cache, &model_enum, &sha256_hex(quantized))
            .expect("the resolved model verifies the file fastembed loads");

        // Negative control: the same entry with the pin stripped hits the
        // exact gate FastEmbedder::new applies — "no pinned SHA-256".
        let stripped =
            serde_json::json!([{ "model_name": "NomicEmbedTextV15Q", "dimensions": 768 }]);
        std::fs::write(config_dir.join("embedders.json"), stripped.to_string()).unwrap();
        let resolved = configured_model_in(&config_dir).expect("resolve the stripped file");
        let err = pinned_sha_for(&resolved).unwrap_err();
        assert!(err.to_string().contains("no pinned SHA-256"), "{err}");

        match previous {
            Some(value) => std::env::set_var("NEUROSTRATA_MODEL", value),
            None => std::env::remove_var("NEUROSTRATA_MODEL"),
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    /// Recording embedder that defers to the trait's default `embed_batch`
    /// (sequential `embed` per text) so the default can be exercised without
    /// a live ONNX model.
    struct OrderEmbedder {
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait::async_trait]
    impl Embedder for OrderEmbedder {
        async fn embed(&self, text: &str) -> Result<Vec<f32>> {
            self.calls.lock().unwrap().push(text.to_string());
            Ok(vec![text.len() as f32])
        }

        fn dimensions(&self) -> usize {
            4
        }
    }

    #[tokio::test]
    async fn embed_batch_default_preserves_order_and_handles_empty() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let embedder = OrderEmbedder {
            calls: Arc::clone(&calls),
        };

        let empty = embedder.embed_batch(&[]).await.unwrap();
        assert!(empty.is_empty());
        assert!(calls.lock().unwrap().is_empty());

        let texts = vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()];
        let vectors = embedder.embed_batch(&texts).await.unwrap();
        assert_eq!(vectors.len(), 3);
        assert_eq!(vectors[0], vec![5.0f32]);
        assert_eq!(vectors[1], vec![4.0f32]);
        assert_eq!(vectors[2], vec![5.0f32]);
        assert_eq!(
            *calls.lock().unwrap(),
            vec!["alpha".to_string(), "beta".to_string(), "gamma".to_string()]
        );
    }

    /// Spawns a one-shot OpenAI-compatible `/embeddings` server that answers
    /// every request with `response_body` before closing.
    async fn spawn_embed_server(
        response_body: &'static str,
    ) -> (std::net::SocketAddr, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.expect("accept");
            let _ = read_http_request(&mut sock).await;
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            use tokio::io::AsyncWriteExt;
            sock.write_all(resp.as_bytes()).await.unwrap();
            sock.flush().await.unwrap();
        });
        (addr, server)
    }

    #[tokio::test]
    async fn remote_embedder_batch_orders_by_index() {
        let (addr, server) = spawn_embed_server(
            r#"{"data":[{"index":1,"embedding":[1.0,2.0,3.0,4.0]},{"index":0,"embedding":[5.0,6.0,7.0,8.0]}]}"#,
        )
        .await;
        let embedder = RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let texts = vec!["hello".to_string(), "world".to_string()];
        let vectors = embedder.embed_batch(&texts).await.unwrap();
        assert_eq!(vectors.len(), 2);
        assert_eq!(vectors[0], vec![5.0, 6.0, 7.0, 8.0]);
        assert_eq!(vectors[1], vec![1.0, 2.0, 3.0, 4.0]);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn remote_embedder_batch_rejects_dimension_mismatch() {
        let (addr, server) =
            spawn_embed_server(r#"{"data":[{"index":0,"embedding":[1.0,2.0,3.0]}]}"#).await;
        let embedder = RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let err = embedder
            .embed_batch(&["hello".to_string()])
            .await
            .expect_err("dimension mismatch must be rejected");
        assert!(err.to_string().contains("remote embedding"), "{err}");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn remote_embedder_batch_rejects_non_numeric_vector() {
        let (addr, server) = spawn_embed_server(
            r#"{"data":[{"index":0,"embedding":[1.0, "not-a-number", 3.0, 4.0]}]}"#,
        )
        .await;
        let embedder = RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let err = embedder
            .embed_batch(&["hello".to_string()])
            .await
            .expect_err("non-numeric vector must be rejected");
        assert!(err.to_string().contains("remote embedding"), "{err}");
        server.await.unwrap();
    }

    #[tokio::test]
    async fn remote_embedder_batch_empty_input_returns_no_vectors() {
        let embedder = RemoteEmbedder::new(
            "http://localhost:1234".to_string(),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap();
        let vectors = embedder.embed_batch(&[]).await.unwrap();
        assert!(vectors.is_empty());
    }

    /// The one input the scripted server refuses.
    const POISON: &str = "poison-input";

    /// How the scripted embedding server answers one request.
    #[derive(Clone, Copy)]
    enum EmbedScript {
        /// Answer every request with one vector per input.
        Accept,
        /// Refuse every request with this status.
        Refuse(u16),
        /// Refuse a request that carries the poison input with this status,
        /// and answer every other request.
        RefusePoison(u16),
    }

    /// The status line for a refusal.
    fn refusal_status(code: u16) -> &'static str {
        match code {
            400 => "400 Bad Request",
            413 => "413 Payload Too Large",
            429 => "429 Too Many Requests",
            500 => "500 Internal Server Error",
            _ => "400 Bad Request",
        }
    }

    /// Texts whose lengths are all different, so the vector the scripted
    /// server returns -- the input's length -- says which input it came from.
    fn sized_texts(n: usize) -> Vec<String> {
        (0..n).map(|i| "x".repeat(i + 1)).collect()
    }

    /// Spawns a scripted OpenAI-compatible `/embeddings` server that answers
    /// any number of requests.
    ///
    /// It records the inputs of every request, so a test can see how the
    /// batch was split and what the provider was asked for.
    async fn spawn_scripted_embed_server(
        script: EmbedScript,
    ) -> (
        std::net::SocketAddr,
        tokio::task::JoinHandle<()>,
        Arc<Mutex<Vec<Vec<String>>>>,
    ) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_server = Arc::clone(&seen);
        let server = tokio::spawn(async move {
            use tokio::io::AsyncWriteExt;
            loop {
                let (mut sock, _) = match listener.accept().await {
                    Ok(pair) => pair,
                    Err(_) => break,
                };
                let req = read_http_request(&mut sock).await;
                let body = req.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or("");
                let inputs: Vec<String> = serde_json::from_str::<serde_json::Value>(body)
                    .ok()
                    .and_then(|v| {
                        v.get("input").and_then(|i| i.as_array()).map(|a| {
                            a.iter()
                                .map(|x| x.as_str().unwrap_or_default().to_string())
                                .collect()
                        })
                    })
                    .unwrap_or_default();
                seen_server.lock().unwrap().push(inputs.clone());

                let refused = match script {
                    EmbedScript::Accept => false,
                    EmbedScript::Refuse(_) => true,
                    EmbedScript::RefusePoison(_) => inputs.iter().any(|t| t == POISON),
                };
                let (status, payload) = if refused {
                    let code = match script {
                        EmbedScript::Refuse(code) | EmbedScript::RefusePoison(code) => code,
                        EmbedScript::Accept => 400,
                    };
                    (
                        refusal_status(code).to_string(),
                        "{\"error\":\"refused\"}".to_string(),
                    )
                } else {
                    // One vector per input: the first component is the
                    // input's length, so the vectors say which input they
                    // belong to.
                    let data: Vec<String> = inputs
                        .iter()
                        .enumerate()
                        .map(|(i, text)| {
                            format!(
                                "{{\"index\":{},\"embedding\":[{},1.0,2.0,3.0]}}",
                                i,
                                text.chars().count()
                            )
                        })
                        .collect();
                    (
                        "200 OK".to_string(),
                        format!("{{\"data\":[{}]}}", data.join(",")),
                    )
                };
                let resp = format!(
                    "HTTP/1.1 {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    status,
                    payload.len(),
                    payload
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.flush().await;
            }
        });
        (addr, server, seen)
    }

    fn scripted_embedder(addr: std::net::SocketAddr) -> RemoteEmbedder {
        RemoteEmbedder::new(
            format!("http://{}", addr),
            "test-key".to_string(),
            "test-model".to_string(),
            4,
        )
        .unwrap()
    }

    /// The request split bounds both the number of inputs and the number of
    /// characters a single request may carry, and it covers every input
    /// exactly once, in order.
    #[test]
    fn remote_request_chunks_bound_the_inputs_and_the_characters() {
        let texts = sized_texts(200);
        let chunks = remote_request_chunks(&texts);
        assert!(
            chunks.len() >= 4,
            "200 inputs cannot fit in fewer than four requests"
        );
        let mut covered = 0usize;
        for (start, end) in &chunks {
            assert_eq!(*start, covered, "the chunks cover the inputs in order");
            assert!(
                end - start <= REMOTE_EMBED_MAX_INPUTS,
                "a request carries at most {} inputs, got {}",
                REMOTE_EMBED_MAX_INPUTS,
                end - start
            );
            let chars: usize = texts[*start..*end].iter().map(|t| t.chars().count()).sum();
            assert!(
                chars <= REMOTE_EMBED_MAX_CHARS || end - start == 1,
                "a request carries at most {} characters, got {}",
                REMOTE_EMBED_MAX_CHARS,
                chars
            );
            covered = *end;
        }
        assert_eq!(covered, texts.len(), "every input is sent exactly once");

        // The character bound splits a batch the input bound would keep in
        // one piece.
        let wide: Vec<String> = (0..100).map(|_| "x".repeat(1000)).collect();
        let wide_chunks = remote_request_chunks(&wide);
        assert!(
            wide_chunks.len() >= 3,
            "100k characters need several requests"
        );
        for (start, end) in &wide_chunks {
            let chars: usize = wide[*start..*end].iter().map(|t| t.chars().count()).sum();
            assert!(
                chars <= REMOTE_EMBED_MAX_CHARS || end - start == 1,
                "a request carries at most {} characters, got {}",
                REMOTE_EMBED_MAX_CHARS,
                chars
            );
        }

        // A single input bigger than the bound cannot be split, so it goes
        // alone rather than being cut in half.
        let huge = vec!["x".repeat(REMOTE_EMBED_MAX_CHARS + 1)];
        assert_eq!(remote_request_chunks(&huge), vec![(0, 1)]);
    }

    /// A batch larger than one request is split, every request stays inside
    /// the bounds, and the vectors come back in input order.
    #[tokio::test]
    async fn remote_embedder_splits_a_large_batch_into_bounded_requests() {
        let (addr, server, seen) = spawn_scripted_embed_server(EmbedScript::Accept).await;
        let embedder = scripted_embedder(addr);
        let texts = sized_texts(150);
        let vectors = embedder.embed_batch(&texts).await.unwrap();
        assert_eq!(vectors.len(), texts.len());
        for (i, vector) in vectors.iter().enumerate() {
            assert_eq!(
                vector[0],
                (i + 1) as f32,
                "input {} keeps its place in the batch",
                i
            );
        }
        let requests = seen.lock().unwrap().clone();
        assert!(
            requests.len() >= 3,
            "150 inputs need several requests, got {}",
            requests.len()
        );
        for request in &requests {
            assert!(
                request.len() <= REMOTE_EMBED_MAX_INPUTS,
                "a request carries at most {} inputs, got {}",
                REMOTE_EMBED_MAX_INPUTS,
                request.len()
            );
        }
        let total: usize = requests.iter().map(|r| r.len()).sum();
        assert_eq!(total, texts.len(), "every input is sent exactly once");
        server.abort();
    }

    /// A provider that refuses one input does not cost the batch: the
    /// refused input is reported as `None`, the rest of the batch lands, and
    /// the order is untouched.
    #[tokio::test]
    async fn remote_embedder_degrades_a_refused_item_and_keeps_the_order() {
        let (addr, server, seen) =
            spawn_scripted_embed_server(EmbedScript::RefusePoison(413)).await;
        let embedder = scripted_embedder(addr);
        let texts = vec!["aa".to_string(), POISON.to_string(), "bb".to_string()];
        let outcome = embedder.embed_batch_degradable(&texts).await.unwrap();
        assert_eq!(outcome.embeddings.len(), 3);
        assert!(outcome.embeddings[0].is_some(), "the first input lands");
        assert!(
            outcome.embeddings[1].is_none(),
            "the refused input is reported, not dropped"
        );
        assert!(outcome.embeddings[2].is_some(), "the last input lands");
        assert_eq!(outcome.embeddings[0].as_ref().unwrap()[0], 2.0);
        assert_eq!(outcome.embeddings[2].as_ref().unwrap()[0], 2.0);
        assert_eq!(outcome.warnings.len(), 1, "one refusal, one warning");
        assert!(
            outcome.warnings[0].contains("413"),
            "the warning names the status: {}",
            outcome.warnings[0]
        );

        // The refused request was halved on the way down.
        let requests = seen.lock().unwrap().clone();
        assert_eq!(
            requests[0].len(),
            3,
            "the batch goes out as one request first"
        );
        assert!(
            requests.len() >= 4,
            "the refused request was halved down to single items, got {} requests",
            requests.len()
        );
        server.abort();
    }

    /// The all-or-nothing contract is unchanged: `embed_batch` fails the
    /// whole batch when the provider refuses an input.
    #[tokio::test]
    async fn remote_embedder_batch_stays_all_or_nothing() {
        let (addr, server, _seen) =
            spawn_scripted_embed_server(EmbedScript::RefusePoison(413)).await;
        let embedder = scripted_embedder(addr);
        let texts = vec!["aa".to_string(), POISON.to_string(), "bb".to_string()];
        let err = embedder
            .embed_batch(&texts)
            .await
            .expect_err("a refused input fails the batch");
        assert!(
            err.to_string().contains("refused"),
            "the error names the refusal: {err}"
        );
        server.abort();
    }

    /// A batch of refused inputs degrades every one of them, and the reasons
    /// are sampled: the summary keeps a bounded number of samples, not one
    /// per refused input.
    #[tokio::test]
    async fn remote_embedder_caps_the_warning_samples() {
        let (addr, server, _seen) =
            spawn_scripted_embed_server(EmbedScript::RefusePoison(400)).await;
        let embedder = scripted_embedder(addr);
        let texts: Vec<String> = (0..12).map(|_| POISON.to_string()).collect();
        let outcome = embedder.embed_batch_degradable(&texts).await.unwrap();
        assert_eq!(outcome.embeddings.len(), texts.len());
        assert!(
            outcome.embeddings.iter().all(|e| e.is_none()),
            "every refused input is reported"
        );
        assert_eq!(
            outcome.warnings.len(),
            REMOTE_EMBED_MAX_WARNING_SAMPLES,
            "the warning samples are capped"
        );
        server.abort();
    }

    use std::sync::atomic::{AtomicUsize, Ordering};

    // ---- Task 4: IdleSession (opt-in idle unload of the local session) ----

    /// Dummy session for container tests: `serial` identifies the exact
    /// instance the factory built, `identity` is what the factory reports
    /// alongside it.
    struct DummySession {
        serial: usize,
        identity: String,
    }

    /// A counting factory that always succeeds, reporting `identity`.
    fn counting_factory(
        calls: Arc<AtomicUsize>,
        identity: &str,
    ) -> impl Fn() -> Result<(DummySession, String)> + Send + Sync {
        let identity = identity.to_string();
        move || {
            let serial = calls.fetch_add(1, Ordering::SeqCst) + 1;
            Ok((
                DummySession {
                    serial,
                    identity: identity.clone(),
                },
                identity.clone(),
            ))
        }
    }

    #[test]
    fn idle_session_loads_once_and_reuses() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = Instant::now();
        let session = IdleSession::new_at(
            counting_factory(calls.clone(), "fastembed:DummyModel"),
            Some(Duration::from_secs(60)),
            base,
        )
        .unwrap();
        for offset in [0, 10, 59] {
            let serial = session
                .with_session_at(base + Duration::from_secs(offset), |s| Ok(s.serial))
                .unwrap();
            assert_eq!(
                serial, 1,
                "within the threshold the startup session is reused"
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(session.snapshot_loaded());
    }

    #[test]
    fn idle_session_drops_after_threshold_and_reloads_exactly_once() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = Instant::now();
        let session = Arc::new(
            IdleSession::new_at(
                counting_factory(calls.clone(), "fastembed:DummyModel"),
                Some(Duration::from_secs(30)),
                base,
            )
            .unwrap(),
        );
        session.with_session_at(base, |s| Ok(s.serial)).unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Concurrent callers all arrive past the threshold: exactly one
        // drop-and-rebuild may happen; the rest reuse the reloaded session.
        let stale = base + Duration::from_secs(45);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let session = Arc::clone(&session);
            handles.push(std::thread::spawn(move || {
                session.with_session_at(stale, |s| Ok(s.serial)).unwrap()
            }));
        }
        let serials: Vec<usize> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert!(
            serials.iter().all(|&serial| serial == 2),
            "every concurrent caller is served by the one reloaded session: {serials:?}"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "exactly one reload under concurrency"
        );
        assert!(session.snapshot_loaded());
    }

    #[test]
    fn idle_session_reload_failure_surfaces_and_retries() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = Instant::now();
        let factory_calls = calls.clone();
        let session = IdleSession::new_at(
            move || {
                let n = factory_calls.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 2 {
                    anyhow::bail!("reload exploded");
                }
                Ok((
                    DummySession {
                        serial: n,
                        identity: "fastembed:DummyModel".into(),
                    },
                    "fastembed:DummyModel".to_string(),
                ))
            },
            Some(Duration::from_secs(30)),
            base,
        )
        .unwrap();
        let stale = base + Duration::from_secs(60);
        let error = session
            .with_session_at(stale, |s| Ok(s.serial))
            .unwrap_err();
        assert!(error.to_string().contains("reload exploded"), "{error}");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
        assert!(
            !session.snapshot_loaded(),
            "a failed reload leaves the session unloaded"
        );

        // The next embed retries the reload instead of serving a dead
        // session or failing forever.
        let serial = session.with_session_at(stale, |s| Ok(s.serial)).unwrap();
        assert_eq!(serial, 3);
        assert_eq!(calls.load(Ordering::SeqCst), 3);
        assert!(session.snapshot_loaded());
    }

    #[test]
    fn idle_session_reload_identity_mismatch_is_a_loud_error() {
        // After any reload the embedder asserts its identity equals the
        // startup identity; a mismatch is a loud embed error, never a
        // silently served different model.
        let calls = Arc::new(AtomicUsize::new(0));
        let base = Instant::now();
        let factory_calls = calls.clone();
        let session = IdleSession::new_at(
            move || {
                let n = factory_calls.fetch_add(1, Ordering::SeqCst) + 1;
                let identity = if n == 2 {
                    "fastembed:MutatedModel"
                } else {
                    "fastembed:StartupModel"
                };
                Ok((
                    DummySession {
                        serial: n,
                        identity: identity.to_string(),
                    },
                    identity.to_string(),
                ))
            },
            Some(Duration::from_secs(30)),
            base,
        )
        .unwrap();
        let stale = base + Duration::from_secs(60);
        let error = session
            .with_session_at(stale, |s| Ok(s.serial))
            .unwrap_err();
        let message = error.to_string();
        assert!(message.contains("fastembed:MutatedModel"), "{message}");
        assert!(message.contains("fastembed:StartupModel"), "{message}");
        assert!(
            !session.snapshot_loaded(),
            "a mismatched session is never installed"
        );
        // Retried on the next embed, and the correct model serves.
        let serial = session.with_session_at(stale, |s| Ok(s.serial)).unwrap();
        assert_eq!(serial, 3);
    }

    #[test]
    fn idle_session_reload_ignores_mutated_config() {
        // The P1-2 pin: the factory rebuilds ONLY from values captured at
        // startup. Between drop and reload, everything a naive reload
        // would consult is mutated -- a shared "live config" cell, the
        // process environment, and an embedders file on disk -- and the
        // reloaded identity must still be the startup identity.
        let calls = Arc::new(AtomicUsize::new(0));
        let live_config = Arc::new(Mutex::new("fastembed:StartupModel".to_string()));
        let base = Instant::now();
        let factory_calls = calls.clone();
        let session = IdleSession::new_at(
            move || {
                let serial = factory_calls.fetch_add(1, Ordering::SeqCst) + 1;
                Ok((
                    DummySession {
                        serial,
                        identity: "fastembed:StartupModel".to_string(),
                    },
                    "fastembed:StartupModel".to_string(),
                ))
            },
            Some(Duration::from_secs(30)),
            base,
        )
        .unwrap();
        session.with_session_at(base, |s| Ok(s.serial)).unwrap();

        *live_config.lock().unwrap() = "fastembed:MutatedModel".to_string();
        std::env::set_var("NS_TASK4_MUTATED_MODEL", "MutatedModel");
        let fake_dir =
            std::env::temp_dir().join(format!("neurostrata-task4-{}", std::process::id()));
        std::fs::create_dir_all(&fake_dir).unwrap();
        std::fs::write(
            fake_dir.join("embedders.json"),
            r#"[{"model_name":"MutatedModel","dimensions":384}]"#,
        )
        .unwrap();

        let identity = session
            .with_session_at(base + Duration::from_secs(60), |s| Ok(s.identity.clone()))
            .unwrap();
        assert_eq!(identity, "fastembed:StartupModel");
        assert_eq!(session.startup_identity(), "fastembed:StartupModel");
        std::env::remove_var("NS_TASK4_MUTATED_MODEL");
        let _ = std::fs::remove_dir_all(&fake_dir);
    }

    #[test]
    fn idle_session_first_load_is_eager() {
        // Construction with an unloadable factory fails at construction:
        // first load stays eager and startup stays fail-closed.
        let calls = Arc::new(AtomicUsize::new(0));
        let factory_calls = calls.clone();
        let result = IdleSession::<DummySession>::new_at(
            move || {
                factory_calls.fetch_add(1, Ordering::SeqCst);
                anyhow::bail!("startup session cannot load")
            },
            Some(Duration::from_secs(30)),
            Instant::now(),
        );
        assert!(
            result.is_err(),
            "construction fails when the first load fails"
        );
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "the factory ran at construction, not on first use"
        );
    }

    #[test]
    fn idle_session_status_polling_never_loads_or_resets() {
        // Status-equivalent reads are the lock-free snapshot only. The
        // factory succeeds on calls 1-2 and fails on call 3 so the test
        // can also observe the unloaded state.
        let calls = Arc::new(AtomicUsize::new(0));
        let base = Instant::now();
        let factory_calls = calls.clone();
        let session = IdleSession::new_at(
            move || {
                let n = factory_calls.fetch_add(1, Ordering::SeqCst) + 1;
                if n == 3 {
                    anyhow::bail!("reload exploded");
                }
                Ok((
                    DummySession {
                        serial: n,
                        identity: "fastembed:DummyModel".into(),
                    },
                    "fastembed:DummyModel".to_string(),
                ))
            },
            Some(Duration::from_secs(30)),
            base,
        )
        .unwrap();
        session.with_session_at(base, |s| Ok(s.serial)).unwrap();

        for _ in 0..5 {
            assert!(session.snapshot_loaded());
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "status reads never call the factory"
        );

        // The reads did not reset the idle clock either: this embed still
        // measures idle from the last real embed (base), so it drops and
        // reloads exactly once.
        let serial = session
            .with_session_at(base + Duration::from_secs(31), |s| Ok(s.serial))
            .unwrap();
        assert_eq!(serial, 2);
        assert_eq!(calls.load(Ordering::SeqCst), 2);

        // Drive the session into the unloaded state (the reload at
        // base+62 fails), then poll status repeatedly: the factory is
        // never called and the session stays unloaded.
        assert!(session
            .with_session_at(base + Duration::from_secs(62), |s| Ok(s.serial))
            .is_err());
        assert!(!session.snapshot_loaded());
        for _ in 0..5 {
            assert!(!session.snapshot_loaded());
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            3,
            "status reads never reload an unloaded session"
        );
        assert!(!session.snapshot_loaded(), "the session is still unloaded");
    }

    #[test]
    fn idle_session_disabled_never_unloads() {
        // Unset/0 setting = disabled: pinned no-change behavior, the
        // startup session serves forever.
        let calls = Arc::new(AtomicUsize::new(0));
        let base = Instant::now();
        let session = IdleSession::new_at(
            counting_factory(calls.clone(), "fastembed:DummyModel"),
            None,
            base,
        )
        .unwrap();
        let serial = session
            .with_session_at(base + Duration::from_secs(86_400 * 365), |s| Ok(s.serial))
            .unwrap();
        assert_eq!(serial, 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(session.snapshot_loaded());
    }

    #[test]
    fn idle_unload_setting_parses_env_only_values() {
        assert_eq!(idle_unload_threshold_from_value(None).unwrap(), None);
        assert_eq!(idle_unload_threshold_from_value(Some("")).unwrap(), None);
        assert_eq!(idle_unload_threshold_from_value(Some("0")).unwrap(), None);
        assert_eq!(
            idle_unload_threshold_from_value(Some("300")).unwrap(),
            Some(Duration::from_secs(300))
        );
        assert!(idle_unload_threshold_from_value(Some("bogus")).is_err());
        assert!(idle_unload_threshold_from_value(Some("-5")).is_err());
    }

    #[test]
    fn idle_unload_summary_segment_is_exact() {
        assert_eq!(format_idle_unload_segment(None), "idle unload: off");
        assert_eq!(
            format_idle_unload_segment(Some(Duration::from_secs(300))),
            "idle unload: on (threshold 300s)"
        );
    }

    #[test]
    fn idle_unload_log_lines_are_exact() {
        // Spec Feature 2, "What the user sees": exactly these two lines.
        // Needles are assembled from fragments so this test's own
        // literals do not satisfy the search.
        let source = include_str!("embed.rs");
        let unload = ["embedder session unloaded after ", "{}s idle"].concat();
        let reload = ["embedder session reloaded (model: ", "{})"].concat();
        assert!(source.contains(&unload), "the unload log line is exact");
        assert!(source.contains(&reload), "the reload log line is exact");
    }

    #[test]
    fn fastembedder_reload_factory_uses_only_captured_values() {
        // Structural complement to idle_session_reload_ignores_mutated_config
        // for the production path: the session factory inside
        // FastEmbedder::with_model may only rebuild from the values it
        // captured at construction. It must never re-run model
        // resolution, re-read the environment, or re-read the embedders
        // file. Needles are assembled from fragments so this test's own
        // literals are not counted.
        let source = include_str!("embed.rs");
        let start = source
            .find("\n    fn with_model(")
            .expect("FastEmbedder::with_model exists");
        let body = &source[start..];
        let body = &body[..body
            .find("\n#[async_trait]")
            .expect("the impl block ends before the trait impl")];
        assert!(
            body.contains("IdleSession::new"),
            "with_model must build an IdleSession"
        );
        for (label, needle) in [
            ("model resolution", ["configured", "_model()"].concat()),
            ("environment reads", ["env", "::var"].concat()),
            ("the embedders file", ["embedders", ".json"].concat()),
            (
                "the acceptable-embedder loader",
                ["load_acceptable", "_embedders"].concat(),
            ),
        ] {
            assert!(
                !body.contains(&needle),
                "the reload factory must not re-run {label}"
            );
        }
    }
}

