//! H3-local adaptation of H1's private capture and conservative reservation path.
//! A consumed directory and synced audit journal are deliberately never resumable.
use super::{budget, Args, Corpus, EvalReport, Mode};
use anyhow::{ensure, Context, Result};
use llm_client::{
    chat_completion_response_metadata, ChatRequest, ChatResponse, HttpResponseEvidence,
    MetricsHandle, RuntimeLlmClient, Usage,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    env,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::time::Instant;

const MIB: usize = 1024 * 1024;
const RAW_RECORD_LIMIT: usize = 4 * MIB + 8192;
const RAW_TOTAL_LIMIT: usize = 161 * MIB;
const REPORT_LIMIT: usize = 256 * 1024;
const MAX_LIFETIME: u64 = 3600;
const SOURCES: [&str; 8] = [
    "tools/h3-eval/src/main.rs",
    "tools/h3-eval/src/budget.rs",
    "tools/h3-eval/src/diagnostic.rs",
    "tools/h3-eval/corpus/v1.json",
    "tools/h3-eval/Cargo.toml",
    "Cargo.toml",
    "Cargo.lock",
    "tools/llm-budget/policy-v2.json",
];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registration {
    schema_version: u8,
    profile: String,
    hypothesis: String,
    git_sha: String,
    executable_sha256: String,
    #[serde(deserialize_with = "unique_sources")]
    source_sha256: BTreeMap<String, String>,
    prompt_sha256: String,
    provider: String,
    api_url: String,
    model: String,
    allowed_response_models: Vec<String>,
    semantic_cases: Vec<String>,
    limits: Limits,
    not_before_unix: u64,
    expires_unix: u64,
    max_lifetime_seconds: u64,
    output_directory: PathBuf,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
struct Limits {
    logical_calls: u64,
    attempts: u64,
    tokens: u64,
    cost_micro_cny: u64,
}

// Struct fields already reject duplicates; enforce the same rule for the sole map field.
fn unique_sources<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> std::result::Result<BTreeMap<String, String>, D::Error> {
    struct Visitor;
    impl<'de> serde::de::Visitor<'de> for Visitor {
        type Value = BTreeMap<String, String>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("unique source path/digest pairs")
        }
        fn visit_map<M: serde::de::MapAccess<'de>>(
            self,
            mut map: M,
        ) -> std::result::Result<Self::Value, M::Error> {
            let mut sources = BTreeMap::new();
            while let Some((path, digest)) = map.next_entry::<String, String>()? {
                if sources.insert(path, digest).is_some() {
                    return Err(serde::de::Error::custom("duplicate source path"));
                }
            }
            Ok(sources)
        }
    }
    d.deserialize_map(Visitor)
}

pub fn git() -> Command {
    let mut command = Command::new("git");
    for (key, _) in env::vars_os() {
        if key.to_string_lossy().starts_with("GIT_") {
            command.env_remove(key);
        }
    }
    // Protected global/system config can also grant wildcard trust (e.g. hosted CI).
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("LC_ALL", "C");
    command
}
pub fn checkout_git() -> Result<Command> {
    let mut command = git();
    // Trust only the invocation checkout, never global Git configuration or a caller override.
    let cwd = env::current_dir()?.canonicalize()?;
    command
        .arg("-c")
        .arg(format!("safe.directory={}", cwd.display()));
    Ok(command)
}
fn now() -> Result<u64> {
    Ok(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs())
}
fn registration_deadline(
    expires_unix: u64,
    monotonic_now: Instant,
    wall_now: SystemTime,
) -> Result<Instant> {
    let expiry = UNIX_EPOCH
        .checked_add(Duration::from_secs(expires_unix))
        .context("Diagnostic expiry overflow")?;
    let remaining = expiry
        .duration_since(wall_now)
        .context("Diagnostic already expired")?;
    ensure!(!remaining.is_zero(), "Diagnostic already expired");
    monotonic_now
        .checked_add(remaining.min(Duration::from_secs(MAX_LIFETIME)))
        .context("Diagnostic deadline overflow")
}
fn hash_file(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 65536];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(hash
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect())
}
fn running_binary_hash() -> Result<String> {
    static DIGEST: OnceLock<String> = OnceLock::new();
    if let Some(digest) = DIGEST.get() {
        return Ok(digest.clone());
    }
    // Linux's proc link identifies the running inode even if its pathname is replaced.
    #[cfg(target_os = "linux")]
    let path = PathBuf::from("/proc/self/exe");
    #[cfg(not(target_os = "linux"))]
    let path = env::current_exe()?;
    let digest = hash_file(&path)?;
    let _ = DIGEST.set(digest.clone());
    Ok(digest)
}
fn hash(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}
fn no_symlinks(path: &Path) -> Result<()> {
    ensure!(path.is_absolute(), "private path must be absolute");
    let mut current = PathBuf::new();
    for part in path.components() {
        ensure!(
            !matches!(part, Component::ParentDir | Component::CurDir),
            "noncanonical private path"
        );
        current.push(part);
        ensure!(
            !fs::symlink_metadata(&current)?.file_type().is_symlink(),
            "private path contains symlink"
        );
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn private_metadata(path: &Path, directory: bool) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    no_symlinks(path)?;
    let metadata = fs::symlink_metadata(path)?;
    let uid = fs::metadata("/proc/self")?.uid();
    ensure!(
        metadata.uid() == uid && metadata.mode() & 0o777 == if directory { 0o700 } else { 0o600 },
        "private owner/mode mismatch"
    );
    ensure!(
        if directory {
            metadata.is_dir()
        } else {
            metadata.is_file()
        },
        "private file type mismatch"
    );
    Ok(())
}
#[cfg(not(target_os = "linux"))]
fn private_metadata(_: &Path, _: bool) -> Result<()> {
    anyhow::bail!("H3 Diagnostic requires the reviewed Linux private-file profile")
}
fn outside_git(path: &Path) -> Result<()> {
    let parent = path.parent().context("private path has no parent")?;
    private_metadata(parent, true)?;
    let output = git()
        .arg("-C")
        .arg(parent.canonicalize()?)
        .args(["rev-parse", "--show-toplevel"])
        .output()?;
    ensure!(
        !output.status.success(),
        "private path is inside a Git checkout"
    );
    ensure!(
        output.status.code() == Some(128)
            && String::from_utf8_lossy(&output.stderr).starts_with("fatal: not a git repository"),
        "cannot establish private path outside Git"
    );
    Ok(())
}
fn fresh(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    Ok(options.open(path)?)
}
struct Bounded(Vec<u8>, usize);
impl Write for Bounded {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.1.saturating_sub(self.0.len()) {
            return Err(std::io::Error::other("evidence bound exceeded"));
        }
        self.0.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub fn bounded_json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>> {
    let mut writer = Bounded(Vec::new(), limit);
    serde_json::to_writer(&mut writer, value)?;
    Ok(writer.0)
}
fn write_json(path: &Path, value: &impl Serialize, limit: usize) -> Result<()> {
    let bytes = bounded_json(value, limit)?;
    let mut file = fresh(path)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    Ok(())
}
fn prompt_hash(corpus: &Corpus) -> Result<String> {
    let messages = corpus
        .semantic_cases
        .iter()
        .map(|case| super::judge_request(case, &corpus.rubric_version).messages)
        .collect::<Vec<_>>();
    Ok(hash(&bounded_json(&messages, 256 * 1024)?))
}
impl Registration {
    fn validate(&self, sha: &str, corpus: &Corpus, root: &Path, time: u64) -> Result<()> {
        ensure!(
            self.schema_version == 1 && self.profile == budget::PROFILE,
            "unsupported Diagnostic registration"
        );
        ensure!(
            !self.hypothesis.trim().is_empty()
                && self.hypothesis.len() <= 2000
                && !self.hypothesis.chars().any(char::is_control),
            "invalid hypothesis"
        );
        ensure!(self.git_sha == sha, "Diagnostic commit drift");
        ensure!(
            self.not_before_unix <= time
                && time < self.expires_unix
                && self.expires_unix - self.not_before_unix <= 86400
                && self.max_lifetime_seconds == MAX_LIFETIME,
            "Diagnostic registration expired or invalid"
        );
        ensure!(
            self.provider == "deepseek"
                && self.api_url == "https://api.deepseek.com"
                && self.model == budget::MODEL
                && self.allowed_response_models == [budget::MODEL],
            "Diagnostic model/origin mismatch"
        );
        ensure!(
            self.limits
                == Limits {
                    logical_calls: 8,
                    attempts: 40,
                    tokens: 20_000_000,
                    cost_micro_cny: 35_000_000
                },
            "Diagnostic limits mismatch"
        );
        ensure!(
            self.semantic_cases
                == corpus
                    .semantic_cases
                    .iter()
                    .map(|case| case.id.clone())
                    .collect::<Vec<_>>()
                && self.semantic_cases.len() == 8,
            "Diagnostic case identity mismatch"
        );
        ensure!(
            self.prompt_sha256 == prompt_hash(corpus)?,
            "Diagnostic prompt drift"
        );
        ensure!(
            self.source_sha256.len() == SOURCES.len(),
            "Diagnostic source identities incomplete"
        );
        for source in SOURCES {
            ensure!(
                self.source_sha256.get(source) == Some(&hash_file(&root.join(source))?),
                "Diagnostic source drift"
            );
        }
        for (name, wanted) in [
            ("H3_EVAL_PROVIDER", self.provider.as_str()),
            ("LLM_API_URL", self.api_url.as_str()),
            ("LLM_MODEL", self.model.as_str()),
        ] {
            ensure!(
                env::var(name).ok().as_deref() == Some(wanted),
                "Diagnostic environment identity mismatch"
            );
        }
        for name in ["LLM_DIAGNOSTIC_BUDGET_ID", "LLM_DIAGNOSTIC_BUDGET_LIMITS"] {
            ensure!(
                env::var_os(name).is_none_or(|v| v.is_empty()),
                "cannot inherit a runtime Diagnostic allowance"
            );
        }
        outside_git(&self.output_directory)?;
        ensure!(
            fs::symlink_metadata(&self.output_directory)
                .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound),
            "Diagnostic namespace already consumed"
        );
        ensure!(
            self.executable_sha256 == running_binary_hash()?,
            "Diagnostic executable drift"
        );
        Ok(())
    }
}

#[derive(Serialize)]
struct ResponseRecord<'a> {
    schema_version: u8,
    sequence: usize,
    case_id: &'a str,
    operation: &'static str,
    logical_attempt: u8,
    http_status: u16,
    complete: bool,
    body: &'a [u8],
}
struct Capture {
    file: File,
    count: usize,
    bytes: usize,
    usages: Vec<Usage>,
    models: BTreeSet<String>,
    failed: bool,
}
pub struct Diagnostic {
    control: budget::Control,
    capture: Mutex<Capture>,
    directory: PathBuf,
    metrics: MetricsHandle,
    deadline: Instant,
    expires_unix: u64,
}
impl Diagnostic {
    fn prepare(path: &Path, sha: &str, corpus: &Corpus) -> Result<Arc<Self>> {
        super::validate_checkout(sha, true)?;
        outside_git(path)?;
        private_metadata(path, false)?;
        let mut bytes = Vec::new();
        File::open(path)?.take(32769).read_to_end(&mut bytes)?;
        ensure!(bytes.len() <= 32768, "registration exceeds 32 KiB");
        let registration: Registration = serde_json::from_slice(&bytes)?;
        let root = checkout_git()?
            .args(["rev-parse", "--show-toplevel"])
            .output()?;
        ensure!(root.status.success(), "cannot resolve checkout");
        let root = PathBuf::from(String::from_utf8(root.stdout)?.trim());
        registration.validate(sha, corpus, &root, now()?)?;
        ensure!(
            now()? < registration.expires_unix,
            "Diagnostic expired during identity validation"
        );
        let directory = registration.output_directory;
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            builder.mode(0o700);
        }
        builder.create(&directory)?;
        File::open(directory.parent().context("namespace parent missing")?)?.sync_all()?;
        write_json(
            &directory.join("started.json"),
            &serde_json::json!({"schema_version":1,"registration_sha256":hash(&bytes),"git_sha":sha,"started_unix":now()?}),
            4096,
        )?;
        File::open(&directory)?.sync_all()?;
        let metrics = llm_client::install_metrics("h3-eval")?;
        let diagnostic = Arc::new(Self {
            control: budget::Control::new(
                metrics.clone(),
                fresh(&directory.join("control.jsonl"))?,
            ),
            capture: Mutex::new(Capture {
                file: fresh(&directory.join("responses.jsonl"))?,
                count: 0,
                bytes: 0,
                usages: Vec::new(),
                models: BTreeSet::new(),
                failed: false,
            }),
            // Sample monotonic time first: sampling delay can shorten, never extend expiry.
            deadline: registration_deadline(
                registration.expires_unix,
                Instant::now(),
                SystemTime::now(),
            )?,
            expires_unix: registration.expires_unix,
            directory,
            metrics,
        });
        File::open(&diagnostic.directory)?.sync_all()?;
        Ok(diagnostic)
    }
    pub fn failed(&self) -> bool {
        self.capture.lock().map_or(true, |capture| capture.failed) || self.control.is_stopped()
    }
    fn stop(&self, code: &'static str) {
        self.control.stop(code);
        if let Ok(mut capture) = self.capture.lock() {
            capture.failed = true;
        }
    }
    fn observe(&self, case: &str, response: HttpResponseEvidence<'_>) -> Result<()> {
        let mut capture = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("Diagnostic capture lock failed"))?;
        let result = (|| -> Result<()> {
            ensure!(
                !capture.failed
                    && case.len() <= 100
                    && capture.count < 40
                    && response.body.len() <= MIB,
                "Diagnostic capture bound/stop"
            );
            let record = ResponseRecord {
                schema_version: 1,
                sequence: capture.count + 1,
                case_id: case,
                operation: "offline_evaluation",
                logical_attempt: 1,
                http_status: response.status,
                complete: response.complete,
                body: response.body,
            };
            let mut bytes = bounded_json(&record, RAW_RECORD_LIMIT - 1)?;
            bytes.push(b'\n');
            ensure!(
                bytes.len() <= RAW_TOTAL_LIMIT.saturating_sub(capture.bytes),
                "Diagnostic total capture bound"
            );
            capture.file.write_all(&bytes)?;
            capture.file.sync_all()?;
            capture.bytes += bytes.len();
            capture.count += 1;
            ensure!(response.complete, "Diagnostic partial response");
            if (200..300).contains(&response.status) {
                let (model, usage) = chat_completion_response_metadata(response.body)?;
                ensure!(
                    model == budget::MODEL,
                    "Diagnostic response model not allowed"
                );
                let usage = usage.context("Diagnostic usage missing")?;
                ensure!(
                    u64::from(usage.input_tokens) <= budget::INPUT_CEILING
                        && usage.output_tokens <= 800
                        && usage
                            .cached_input_tokens
                            .is_none_or(|n| n <= usage.input_tokens),
                    "Diagnostic usage outside allowance"
                );
                capture.models.insert(model);
                capture.usages.push(usage);
            }
            Ok(())
        })();
        if result.is_err() {
            capture.failed = true;
            self.control.stop("diagnostic_response_evidence_failed");
        }
        result
    }
    pub async fn chat(
        self: &Arc<Self>,
        client: &RuntimeLlmClient,
        case: &str,
        request: ChatRequest,
    ) -> Result<ChatResponse> {
        let result = self.chat_inner(client, case, request).await;
        if result.is_err() {
            self.stop("diagnostic_request_failed");
        }
        result
    }
    async fn chat_inner(
        self: &Arc<Self>,
        client: &RuntimeLlmClient,
        case: &str,
        request: ChatRequest,
    ) -> Result<ChatResponse> {
        ensure!(
            !self.failed() && Instant::now() < self.deadline && now()? < self.expires_unix,
            "Diagnostic stopped or expired"
        );
        ensure!(
            request.operation == llm_client::LlmOperation::OfflineEvaluation
                && request.max_tokens == Some(800)
                && request.thinking == Some(false)
                && !request.stream
                && request.runtime_user_id.is_none(),
            "Diagnostic request mismatch"
        );
        // The fixed prompts are tiny; this stricter byte ceiling fits the conservative context quote.
        ensure!(
            bounded_json(&request.messages, 128 * 1024).is_ok(),
            "Diagnostic prompt exceeds bound"
        );
        let usage_start = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("Diagnostic lock failed"))?
            .usages
            .len();
        let ticket = self.control.begin(case, 800).map_err(anyhow::Error::msg)?;
        ensure!(
            Instant::now() < self.deadline && now()? < self.expires_unix,
            "Diagnostic expired before dispatch"
        );
        let this = self.clone();
        let case_id = case.to_owned();
        let request = request.observe_responses(move |response| this.observe(&case_id, response));
        let response = tokio::time::timeout_at(self.deadline, client.chat(request)).await??;
        let usages = self
            .capture
            .lock()
            .map_err(|_| anyhow::anyhow!("Diagnostic lock failed"))?
            .usages[usage_start..]
            .to_vec();
        self.control
            .finish(case, ticket, &usages)
            .map_err(anyhow::Error::msg)?;
        Ok(response)
    }
    fn finish(&self, report: Option<&EvalReport>) -> Result<serde_json::Value> {
        // A poisoned state cannot justify reconstructed usage; retain the journal and mark unknown.
        let (count, models, capture_failed) = match self.capture.lock() {
            Ok(capture) => (Some(capture.count), capture.models.clone(), capture.failed),
            Err(_) => (None, BTreeSet::new(), true),
        };
        let budget = self.control.report().ok();
        let state_failed = capture_failed || budget.is_none() || self.control.is_stopped();
        let details = serde_json::json!({"profile":budget::PROFILE,"evidence_class":"Diagnostic","private_response_count":count,"response_models":models,"budget":budget,"balance_checked":false});
        let mut terminal = serde_json::json!({"schema_version":1,"evaluation_returned":report.is_some(),"quality_passed":report.is_some_and(|r|r.passed),"evidence_failed":state_failed,"diagnostic":details});
        let mut output = serde_json::to_value(report)?;
        if let Some(object) = output.as_object_mut() {
            object.insert("schema_version".into(), 2.into());
            object.insert(
                "passed".into(),
                (report.is_some_and(|r| r.passed) && !state_failed).into(),
            );
            object.insert("mode".into(), "diagnostic".into());
            object.insert("response_models".into(), serde_json::to_value(models)?);
            object.insert("diagnostic".into(), details);
        }
        let artifacts = (|| -> Result<()> {
            let metrics = self.metrics.render();
            ensure!(metrics.len() <= MIB, "Diagnostic metrics bound");
            let mut file = fresh(&self.directory.join("metrics.prom"))?;
            file.write_all(metrics.as_bytes())?;
            file.sync_all()?;
            write_json(&self.directory.join("report.json"), &output, REPORT_LIMIT)
        })();
        if artifacts.is_err() {
            self.stop("diagnostic_artifact_failed");
            terminal["evidence_failed"] = true.into();
            terminal["diagnostic"]["budget"] = serde_json::to_value(self.control.report().ok())?;
            terminal["artifact_failure"] = "diagnostic_artifact_failed".into();
        }
        // Still attempt terminal evidence when another evidence artifact cannot be saved.
        write_json(&self.directory.join("terminal.json"), &terminal, 64 * 1024)?;
        File::open(&self.directory)?.sync_all()?;
        artifacts?;
        Ok(output)
    }
}

pub async fn run(args: Args, corpus: Corpus) -> Result<()> {
    ensure!(
        args.mode == Mode::Live && args.metrics_output.is_none(),
        "Diagnostic owns its private metrics output"
    );
    let diagnostic = Diagnostic::prepare(
        args.diagnostic_registration
            .as_deref()
            .context("registration required")?,
        &args.git_sha,
        &corpus,
    )?;
    // No key read occurs until all identities/paths are checked and Started is synced.
    let result = async {
        ensure!(
            Instant::now() < diagnostic.deadline && now()? < diagnostic.expires_unix,
            "Diagnostic expired before key access"
        );
        let mut config = super::run_config(Mode::Live)?;
        config.diagnostic = Some(diagnostic.clone());
        super::evaluate(&corpus, &config, args.git_sha).await
    };
    let outcome = tokio::select! {
        result=tokio::time::timeout_at(diagnostic.deadline,result)=>match result {Ok(result)=>result,Err(_)=>Err(anyhow::anyhow!("Diagnostic deadline"))},
        _=shutdown()=>Err(anyhow::anyhow!("Diagnostic interrupted")),
    };
    if outcome.is_err() {
        diagnostic.stop("diagnostic_interrupted_or_failed");
    }
    let output = diagnostic.finish(outcome.as_ref().ok())?;
    println!("{}", serde_json::to_string_pretty(&output)?);
    let report = outcome?;
    ensure!(
        report.passed && !diagnostic.failed(),
        "H3 Diagnostic failed; retained evidence is final"
    );
    Ok(())
}
async fn shutdown() {
    #[cfg(unix)]
    {
        if let Ok(mut term) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            tokio::select! {_ = tokio::signal::ctrl_c()=>{}, _=term.recv()=>{}}
        } else {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};

    struct Temp(PathBuf);
    impl Temp {
        fn new() -> Self {
            let path = env::temp_dir().join(format!("h3-control-test-{}", uuid::Uuid::new_v4()));
            fs::create_dir(&path).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
            }
            Self(path)
        }
    }
    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn fixture(directory: &Path, milliseconds: u64) -> Arc<Diagnostic> {
        let metrics = llm_client::install_metrics("h3-eval").unwrap();
        Arc::new(Diagnostic {
            control: budget::Control::new(
                metrics.clone(),
                fresh(&directory.join("control.jsonl")).unwrap(),
            ),
            capture: Mutex::new(Capture {
                file: fresh(&directory.join("responses.jsonl")).unwrap(),
                count: 0,
                bytes: 0,
                usages: Vec::new(),
                models: BTreeSet::new(),
                failed: false,
            }),
            directory: directory.to_owned(),
            metrics,
            deadline: Instant::now() + Duration::from_millis(milliseconds),
            expires_unix: now().unwrap() + 3600,
        })
    }
    fn journal(path: &Path) -> Vec<serde_json::Value> {
        fs::read_to_string(path)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect()
    }
    // Each process owns its global metrics recorder and environment. Never touches a provider.
    #[test]
    fn registered_client_lifecycle() {
        if let Ok(scenario) = env::var("H3_CONTROL_TEST_SCENARIO") {
            tokio::runtime::Runtime::new()
                .unwrap()
                .block_on(client_case(&scenario));
            return;
        }
        for scenario in [
            "success",
            "fallback",
            "wrong_model_empty",
            "missing_usage",
            "invalid_usage_empty",
            "partial",
            "unknown_retry",
            "mixed_five",
            "no_sixth",
            "malformed_judge",
            "cancelled",
            "deadline",
            "reserve_write",
            "reserve_sync",
            "settle_sync",
            "capture_write",
            "exhausted",
            "poison",
            "expired",
            "artifact_write",
            "count_bound",
            "total_bound",
            "raw_bound",
        ] {
            let output = Command::new(env::current_exe().unwrap())
                .args([
                    "--exact",
                    "diagnostic::tests::registered_client_lifecycle",
                    "--nocapture",
                ])
                .env("H3_TEST_EXECUTABLE_SHA", running_binary_hash().unwrap())
                .env("H3_CONTROL_TEST_SCENARIO", scenario)
                .env_remove("LLM_DIAGNOSTIC_BUDGET_ID")
                .env_remove("LLM_DIAGNOSTIC_BUDGET_LIMITS")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{scenario}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
    }
    async fn client_case(scenario: &str) {
        let temp = Temp::new();
        let mut diagnostic = if scenario == "success" {
            let (root, path, value) = registration_fixture(&temp.0);
            env::set_current_dir(&root).unwrap();
            env::set_var("H3_EVAL_PROVIDER", "deepseek");
            env::set_var("LLM_API_URL", "https://api.deepseek.com");
            env::set_var("LLM_MODEL", budget::MODEL);
            let sha = value["git_sha"].as_str().unwrap();
            super::super::validate_checkout(sha, true).unwrap();
            write_json(&path, &value, 32768).unwrap();
            Diagnostic::prepare(&path, sha, &super::super::load_corpus().unwrap()).unwrap()
        } else {
            fixture(&temp.0, if scenario == "expired" { 0 } else { 30_000 })
        };
        if matches!(scenario, "count_bound" | "total_bound" | "raw_bound") {
            let case = "bounded-case";
            let body = if scenario == "raw_bound" {
                vec![255; MIB]
            } else {
                b"{}".to_vec()
            };
            let record = ResponseRecord {
                schema_version: 1,
                sequence: 1,
                case_id: case,
                operation: "offline_evaluation",
                logical_attempt: 1,
                http_status: 429,
                complete: true,
                body: &body,
            };
            let length = bounded_json(&record, RAW_RECORD_LIMIT - 1).unwrap().len() + 1;
            if scenario == "count_bound" {
                diagnostic.capture.lock().unwrap().count = 39;
            }
            if scenario == "total_bound" {
                diagnostic.capture.lock().unwrap().bytes = RAW_TOTAL_LIMIT - length;
            }
            diagnostic
                .observe(
                    case,
                    HttpResponseEvidence {
                        status: 429,
                        body: &body,
                        complete: true,
                    },
                )
                .unwrap();
            if scenario == "count_bound" {
                assert_eq!(diagnostic.capture.lock().unwrap().count, 40);
            }
            if scenario == "total_bound" {
                assert_eq!(diagnostic.capture.lock().unwrap().bytes, RAW_TOTAL_LIMIT);
            }
            let next = if scenario == "raw_bound" {
                vec![255; MIB + 1]
            } else {
                body
            };
            assert!(diagnostic
                .observe(
                    case,
                    HttpResponseEvidence {
                        status: 429,
                        body: &next,
                        complete: true
                    }
                )
                .is_err());
            assert!(diagnostic.failed());
            return;
        }
        if matches!(
            scenario,
            "reserve_write" | "reserve_sync" | "exhausted" | "poison"
        ) {
            diagnostic
                .control
                .test_fault(scenario, &diagnostic.directory.join("control.jsonl"));
        }
        if scenario == "capture_write" {
            diagnostic.capture.lock().unwrap().file =
                File::open(diagnostic.directory.join("responses.jsonl")).unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_server = seen.clone();
        let source = super::super::load_corpus().unwrap();
        let cases = source.semantic_cases;
        let case_scenario = scenario.to_owned();
        if scenario == "deadline" {
            // Fixed subsecond wall-clock fixture; real transport must be cancelled at its deadline.
            Arc::get_mut(&mut diagnostic).unwrap().deadline = registration_deadline(
                101,
                Instant::now(),
                UNIX_EPOCH + Duration::from_millis(100_100),
            )
            .unwrap();
        }
        let owned = diagnostic.clone();
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let mut socket = BufReader::new(socket);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    socket.read_line(&mut line).await.unwrap();
                    if line == "\r\n" {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = value.trim().parse().unwrap();
                    }
                }
                let mut request = vec![0; length];
                socket.read_exact(&mut request).await.unwrap();
                let request: serde_json::Value = serde_json::from_slice(&request).unwrap();
                assert_eq!(request["max_tokens"], 800);
                assert_eq!(request["temperature"], 0.0);
                let input = request["messages"][1]["content"].as_str().unwrap();
                let case = cases
                    .iter()
                    .find(|case| {
                        super::super::judge_request(case, super::super::RUBRIC_VERSION).messages[1]
                            .content
                            == input
                    })
                    .unwrap();
                seen_server.lock().unwrap().push(case.id.clone());
                let index = observed.fetch_add(1, Ordering::SeqCst);
                // Every HTTP dispatch sees the complete reservation journal already written.
                let rows = journal(&owned.directory.join("control.jsonl"));
                assert_eq!(rows.last().unwrap()["event"], "reserve");
                assert!(!rows.last().unwrap()["unreleased_reservation"].is_null());
                if matches!(case_scenario.as_str(), "cancelled" | "deadline") {
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
                if case_scenario == "settle_sync" {
                    owned
                        .control
                        .test_fault("reserve_sync", &owned.directory.join("control.jsonl"));
                }
                let empty = (case_scenario == "fallback" && index == 0)
                    || case_scenario == "wrong_model_empty"
                    || case_scenario == "invalid_usage_empty"
                    || (matches!(case_scenario.as_str(), "mixed_five" | "no_sixth") && index == 1);
                let retry = (case_scenario == "unknown_retry" && index == 0)
                    || (matches!(case_scenario.as_str(), "mixed_five" | "no_sixth")
                        && matches!(index, 0 | 2 | 3))
                    || (case_scenario == "no_sixth" && index == 4);
                let content = if empty {
                    "".to_owned()
                } else if case_scenario == "malformed_judge" {
                    "not JSON".to_owned()
                } else {
                    serde_json::to_string(&case.recorded_judgment).unwrap()
                };
                let mut envelope = serde_json::json!({"model":if case_scenario=="wrong_model_empty" {"unregistered-model"} else {budget::MODEL},"choices":[{"finish_reason":"stop","message":{"content":content}}],"usage":{"prompt_tokens":3,"completion_tokens":if case_scenario=="invalid_usage_empty" {801} else {2}}});
                if case_scenario == "missing_usage" {
                    envelope.as_object_mut().unwrap().remove("usage");
                }
                if retry {
                    envelope = serde_json::json!({"error":"synthetic retry"});
                }
                let body = envelope.to_string();
                let status = if retry { 429 } else { 200 };
                let length = body.len() + if case_scenario == "partial" { 10 } else { 0 };
                socket.write_all(format!("HTTP/1.1 {status} Test\r\nContent-Length: {length}\r\nRetry-After: 0\r\nConnection: close\r\n\r\n{body}").as_bytes()).await.unwrap();
            }
        });
        let corpus = super::super::load_corpus().unwrap();
        let mut config = super::super::run_config(Mode::Recorded).unwrap();
        config.mode = Mode::Live;
        config.provider = "deepseek".into();
        config.model = budget::MODEL.into();
        config.client = Some(RuntimeLlmClient::static_config(
            format!("http://{address}"),
            budget::MODEL.into(),
            "synthetic-key".into(),
            false,
        ));
        config.diagnostic = Some(diagnostic.clone());
        if scenario == "cancelled" {
            // Cancel after actual loopback dispatch, not a host-load-sensitive sleep from setup.
            tokio::select! {
                _ = super::super::evaluate(&corpus, &config, "0".repeat(40)) => panic!("delayed response unexpectedly completed"),
                _ = async { while calls.load(Ordering::SeqCst)==0 {tokio::time::sleep(Duration::from_millis(1)).await;} } => {}
            }
            diagnostic.stop("diagnostic_cancelled_fixture");
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            diagnostic.finish(None).unwrap();
            let terminal = journal(&diagnostic.directory.join("terminal.json"));
            assert!(!terminal[0]["diagnostic"]["budget"]["unreleased_reservation"].is_null());
            assert!(diagnostic.failed());
            server.abort();
            return;
        }
        let report = super::super::evaluate(&corpus, &config, "0".repeat(40))
            .await
            .unwrap();
        server.abort();
        let count = calls.load(Ordering::SeqCst);
        let expected = match scenario {
            "success" | "malformed_judge" | "artifact_write" => 8,
            "fallback" => 9,
            "unknown_retry" => 2,
            "mixed_five" | "no_sixth" => 5,
            "reserve_write" | "reserve_sync" | "exhausted" | "poison" | "expired" => 0,
            _ => 1,
        };
        assert_eq!(count, expected, "{scenario}");
        assert_eq!(
            report.passed,
            matches!(scenario, "success" | "fallback" | "artifact_write")
        );
        if scenario == "artifact_write" {
            fresh(&diagnostic.directory.join("metrics.prom")).unwrap();
        }
        if scenario == "poison" {
            assert!(diagnostic.failed());
            let output = diagnostic.finish(Some(&report)).unwrap();
            assert!(output["diagnostic"]["budget"].is_null());
            assert_eq!(
                journal(&diagnostic.directory.join("terminal.json"))[0]["evidence_failed"],
                true
            );
            return;
        }
        let output = diagnostic.finish(Some(&report));
        if scenario == "artifact_write" {
            assert!(output.is_err());
            assert_eq!(
                journal(&diagnostic.directory.join("terminal.json"))[0]["artifact_failure"],
                "diagnostic_artifact_failed"
            );
            return;
        }
        let output = output.unwrap();
        assert_eq!(output["schema_version"], 2);
        assert_eq!(output["mode"], "diagnostic");
        let accounting = &output["diagnostic"]["budget"];
        if matches!(scenario, "success" | "fallback" | "malformed_judge") {
            assert!(accounting["unreleased_reservation"].is_null());
            assert!(accounting["stopped"].is_null());
            assert_eq!(accounting["charged"]["logical_calls"], 8);
            assert_eq!(accounting["charged"]["attempts"], expected);
            let expected_order = corpus
                .semantic_cases
                .iter()
                .map(|case| case.id.clone())
                .collect::<Vec<_>>();
            let mut actual = seen.lock().unwrap().clone();
            actual.dedup();
            assert_eq!(actual, expected_order);
            let rows = journal(&diagnostic.directory.join("control.jsonl"));
            assert_eq!(rows.len(), 16);
            assert_eq!(rows.last().unwrap()["event"], "settle");
            assert!(rows.last().unwrap()["unreleased_reservation"].is_null());
        } else {
            assert!(diagnostic.failed());
            if count > 0 || matches!(scenario, "reserve_write" | "reserve_sync") {
                assert!(!accounting["unreleased_reservation"].is_null());
            }
            let request =
                super::super::judge_request(&corpus.semantic_cases[0], &corpus.rubric_version);
            assert!(diagnostic
                .chat(
                    config.client.as_ref().unwrap(),
                    &corpus.semantic_cases[0].id,
                    request
                )
                .await
                .is_err());
            assert_eq!(calls.load(Ordering::SeqCst), count);
        }
        let raw = fs::read_to_string(diagnostic.directory.join("responses.jsonl")).unwrap();
        assert!(!raw.contains("synthetic-key"));
        if matches!(scenario, "success" | "fallback") {
            for row in raw.lines() {
                let row: serde_json::Value = serde_json::from_str(row).unwrap();
                let body = row["body"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|v| v.as_u64().unwrap() as u8)
                    .collect::<Vec<_>>();
                let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let content = envelope["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap();
                if !content.is_empty() {
                    super::super::parse_judgment(content, super::super::RUBRIC_VERSION).unwrap();
                }
            }
        }
    }

    fn registration_fixture(parent: &Path) -> (PathBuf, PathBuf, serde_json::Value) {
        let root = parent.join("checkout");
        fs::create_dir(&root).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let actual = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        for path in SOURCES {
            let target = root.join(path);
            fs::create_dir_all(target.parent().unwrap()).unwrap();
            fs::copy(actual.join(path), target).unwrap();
        }
        assert!(git()
            .arg("-C")
            .arg(&root)
            .args(["init", "--quiet"])
            .status()
            .unwrap()
            .success());
        assert!(git()
            .arg("-C")
            .arg(&root)
            .args(["add", "."])
            .status()
            .unwrap()
            .success());
        assert!(git()
            .arg("-C")
            .arg(&root)
            .args([
                "-c",
                "user.name=Synthetic",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "synthetic fixture"
            ])
            .status()
            .unwrap()
            .success());
        let sha = String::from_utf8(
            git()
                .arg("-C")
                .arg(&root)
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_owned();
        let corpus = super::super::load_corpus().unwrap();
        let sources = SOURCES
            .into_iter()
            .map(|p| (p, hash_file(&root.join(p)).unwrap()))
            .collect::<BTreeMap<_, _>>();
        let registration = serde_json::json!({"schema_version":1,"profile":budget::PROFILE,"hypothesis":"Synthetic control fixture; never a provider run","git_sha":sha,"executable_sha256":env::var("H3_TEST_EXECUTABLE_SHA").unwrap_or_else(|_|running_binary_hash().unwrap()),"source_sha256":sources,"prompt_sha256":prompt_hash(&corpus).unwrap(),"provider":"deepseek","api_url":"https://api.deepseek.com","model":budget::MODEL,"allowed_response_models":[budget::MODEL],"semantic_cases":corpus.semantic_cases.iter().map(|c|c.id.clone()).collect::<Vec<_>>(),"limits":{"logical_calls":8,"attempts":40,"tokens":20_000_000,"cost_micro_cny":35_000_000},"not_before_unix":now().unwrap()-1,"expires_unix":now().unwrap()+300,"max_lifetime_seconds":MAX_LIFETIME,"output_directory":parent.join("consumed")});
        (root, parent.join("registration.json"), registration)
    }
    #[test]
    fn registration_paths_and_crash_accounting() {
        if let Ok(scenario) = env::var("H3_REGISTRATION_TEST_SCENARIO") {
            registration_case(&scenario);
            return;
        }
        for scenario in [
            "valid",
            "wrong_sha",
            "wrong_executable",
            "expired",
            "future",
            "source_drift",
            "prompt_drift",
            "wrong_model",
            "allow_alias",
            "runtime_budget",
            "duplicates",
            "duplicate_source",
            "duplicate_limit",
            "unknown_field",
            "inside_git",
            "symlink",
            "existing_empty",
            "bad_mode",
            "reserve_crash",
            "partial_settle_crash",
            "settled_crash",
            "reserve_sync",
            "reserve_write",
        ] {
            let audit = Temp::new();
            let output = Command::new(env::current_exe().unwrap())
                .env("H3_TEST_EXECUTABLE_SHA", running_binary_hash().unwrap())
                .env("H3_TEST_AUDIT_ROOT", &audit.0)
                .args([
                    "--exact",
                    "diagnostic::tests::registration_paths_and_crash_accounting",
                    "--nocapture",
                ])
                .env("H3_REGISTRATION_TEST_SCENARIO", scenario)
                .env_remove("LLM_API_KEY")
                .env_remove("LLM_DIAGNOSTIC_BUDGET_ID")
                .env_remove("LLM_DIAGNOSTIC_BUDGET_LIMITS")
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{scenario}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            if matches!(
                scenario,
                "reserve_crash" | "partial_settle_crash" | "settled_crash"
            ) {
                let text = fs::read_to_string(audit.0.join("consumed/control.jsonl")).unwrap();
                let complete = text
                    .split_inclusive('\n')
                    .filter(|line| line.ends_with('\n'))
                    .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
                    .collect::<Vec<_>>();
                let last = complete.last().unwrap();
                assert_eq!(
                    last["event"],
                    if scenario == "settled_crash" {
                        "settle"
                    } else {
                        "reserve"
                    }
                );
                assert_eq!(
                    last["unreleased_reservation"].is_null(),
                    scenario == "settled_crash"
                );
                assert!(audit.0.join("consumed/started.json").exists());
                assert!(!audit.0.join("consumed/terminal.json").exists());
                assert!(fs::create_dir(audit.0.join("consumed")).is_err());
            }
        }
    }
    fn registration_case(scenario: &str) {
        let temp = Temp(PathBuf::from(env::var_os("H3_TEST_AUDIT_ROOT").unwrap()));
        let (root, path, mut value) = registration_fixture(&temp.0);
        env::set_current_dir(&root).unwrap();
        env::set_var("H3_EVAL_PROVIDER", "deepseek");
        env::set_var("LLM_API_URL", "https://api.deepseek.com");
        env::set_var("LLM_MODEL", budget::MODEL);
        // An inherited override must not redirect either checkout or per-path Git discovery.
        env::set_var("GIT_DIR", root.join(".git"));
        env::set_var("GIT_WORK_TREE", &root);
        let sha = value["git_sha"].as_str().unwrap().to_owned();
        let corpus = super::super::load_corpus().unwrap();
        super::super::validate_checkout(&sha, true).unwrap();
        match scenario {
            "wrong_sha" => value["git_sha"] = "0".repeat(40).into(),
            "wrong_executable" => value["executable_sha256"] = "0".repeat(64).into(),
            "expired" => value["expires_unix"] = (now().unwrap() - 1).into(),
            "future" => value["not_before_unix"] = (now().unwrap() + 30).into(),
            "source_drift" => value["source_sha256"][SOURCES[0]] = "0".repeat(64).into(),
            "prompt_drift" => value["prompt_sha256"] = "0".repeat(64).into(),
            "wrong_model" => value["model"] = "wrong-model".into(),
            "allow_alias" => {
                value["allowed_response_models"] =
                    serde_json::json!([budget::MODEL, "another-model"])
            }
            "runtime_budget" => {
                env::set_var("LLM_DIAGNOSTIC_BUDGET_ID", "synthetic-unrelated-binding")
            }
            "unknown_field" => value["extra"] = true.into(),
            "inside_git" => {
                value["output_directory"] = root
                    .join("private-output")
                    .to_string_lossy()
                    .into_owned()
                    .into()
            }
            "existing_empty" => {
                fs::create_dir(temp.0.join("consumed")).unwrap();
            }
            _ => {}
        }
        write_json(&path, &value, 32768).unwrap();
        if scenario == "duplicates" {
            let bytes = fs::read_to_string(&path).unwrap();
            let duplicate = bytes.replacen(
                "\"schema_version\":1",
                "\"schema_version\":1,\"schema_version\":1",
                1,
            );
            fs::write(&path, duplicate).unwrap();
        }
        if scenario == "duplicate_source" {
            let bytes = fs::read_to_string(&path).unwrap();
            let source = format!(
                "\"{}\":\"{}\"",
                SOURCES[0],
                value["source_sha256"][SOURCES[0]].as_str().unwrap()
            );
            fs::write(
                &path,
                bytes.replacen(&source, &format!("{source},{source}"), 1),
            )
            .unwrap();
        }
        if scenario == "duplicate_limit" {
            let bytes = fs::read_to_string(&path).unwrap();
            fs::write(
                &path,
                bytes.replacen(
                    "\"logical_calls\":8",
                    "\"logical_calls\":8,\"logical_calls\":8",
                    1,
                ),
            )
            .unwrap();
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if scenario == "bad_mode" {
                fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            if scenario == "symlink" {
                fs::rename(&path, temp.0.join("actual-registration")).unwrap();
                std::os::unix::fs::symlink(temp.0.join("actual-registration"), &path).unwrap();
            }
        }
        let diagnostic = Diagnostic::prepare(&path, &sha, &corpus);
        if !matches!(
            scenario,
            "valid"
                | "reserve_crash"
                | "partial_settle_crash"
                | "settled_crash"
                | "reserve_sync"
                | "reserve_write"
        ) {
            assert!(diagnostic.is_err(), "{scenario}");
            assert!(!temp.0.join("consumed/started.json").exists());
            return;
        }
        let diagnostic = diagnostic.unwrap();
        private_metadata(&diagnostic.directory, true).unwrap();
        for name in ["started.json", "control.jsonl", "responses.jsonl"] {
            private_metadata(&diagnostic.directory.join(name), false).unwrap();
        }
        assert!(Diagnostic::prepare(&path, &sha, &corpus).is_err());
        if scenario == "valid" {
            // Reproduce hosted runners' wildcard trust without changing any real Git config.
            let ambient_home = temp.0.join("ambient-home");
            fs::create_dir(&ambient_home).unwrap();
            let wildcard = ambient_home.join(".gitconfig");
            fs::write(&wildcard, "[safe]\n\tdirectory = *\n").unwrap();
            env::set_var("HOME", &ambient_home);
            env::set_var("XDG_CONFIG_HOME", ambient_home.join("xdg"));
            env::set_var("GIT_CONFIG_SYSTEM", &wildcard);
            let ambient_accepted = git()
                .env("GIT_CONFIG_NOSYSTEM", "0")
                .env("GIT_CONFIG_SYSTEM", &wildcard)
                .env("GIT_CONFIG_GLOBAL", &wildcard)
                .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap();
            assert!(ambient_accepted.status.success());
            let denied = git()
                // Substitute only the system file path; NOSYSTEM must still block its wildcard.
                .env("GIT_CONFIG_SYSTEM", &wildcard)
                .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap();
            assert!(!denied.status.success());
            assert!(String::from_utf8_lossy(&denied.stderr).contains("dubious ownership"));
            let accepted = checkout_git()
                .unwrap()
                .env("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1")
                .args(["rev-parse", "HEAD"])
                .output()
                .unwrap();
            assert!(accepted.status.success());
            assert_eq!(String::from_utf8(accepted.stdout).unwrap().trim(), sha);
            // Ordinary mode continues to accept a process-scoped safe.directory override.
            // Remove the synthetic wildcard for this check so only the exact override can pass.
            env::set_var("GIT_CONFIG_NOSYSTEM", "1");
            env::set_var("GIT_CONFIG_GLOBAL", "/dev/null");
            env::set_var("GIT_CONFIG_COUNT", "1");
            env::set_var("GIT_CONFIG_KEY_0", "safe.directory");
            env::set_var("GIT_CONFIG_VALUE_0", &root);
            env::set_var("GIT_TEST_ASSUME_DIFFERENT_OWNER", "1");
            super::super::validate_checkout(&sha, false).unwrap();
            // Real alternate/nested checkout discovery, independent of the candidate checkout.
            assert!(outside_git(&root.join("private.json")).is_err());
            let nested = root.join("nested");
            fs::create_dir(&nested).unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&nested, fs::Permissions::from_mode(0o700)).unwrap();
            }
            assert!(git()
                .arg("-C")
                .arg(&nested)
                .args(["init", "--quiet"])
                .status()
                .unwrap()
                .success());
            assert!(outside_git(&nested.join("private.json")).is_err());
            return;
        }
        if matches!(scenario, "reserve_sync" | "reserve_write") {
            diagnostic
                .control
                .test_fault(scenario, &diagnostic.directory.join("control.jsonl"));
            assert!(diagnostic.control.begin("synthetic-case", 800).is_err());
            assert!(diagnostic.control.is_stopped());
            return;
        }
        diagnostic.control.begin("synthetic-case", 800).unwrap();
        let file = diagnostic.directory.join("control.jsonl");
        if scenario == "partial_settle_crash" {
            let mut file = OpenOptions::new().append(true).open(&file).unwrap();
            file.write_all(b"{\"event\":\"settle\"").unwrap();
            file.sync_all().unwrap();
        }
        if scenario == "settled_crash" {
            diagnostic
                .control
                .test_settle_without_provider("synthetic-case");
        }
        // Abrupt process exit skips destructors/terminal handling; the parent audits only synced records.
        if matches!(
            scenario,
            "reserve_crash" | "partial_settle_crash" | "settled_crash"
        ) {
            std::process::exit(0);
        }
        drop(diagnostic);
        // Audit only complete synced records, never resume execution.
        let text = fs::read_to_string(&file).unwrap();
        let complete = text
            .split_inclusive('\n')
            .filter(|line| line.ends_with('\n'))
            .map(|line| serde_json::from_str::<serde_json::Value>(line).unwrap())
            .collect::<Vec<_>>();
        let last = complete.last().unwrap();
        if scenario == "settled_crash" {
            assert_eq!(last["event"], "settle");
            assert!(last["unreleased_reservation"].is_null());
        } else {
            assert_eq!(last["event"], "reserve");
            assert_eq!(last["charged"]["attempts"], 5);
            assert!(!last["unreleased_reservation"].is_null());
        }
        assert!(Diagnostic::prepare(&path, &sha, &corpus).is_err());
    }

    #[test]
    fn registration_expiry_preserves_subsecond_remaining_time() {
        let monotonic = Instant::now();
        let deadline =
            registration_deadline(101, monotonic, UNIX_EPOCH + Duration::from_millis(100_900))
                .unwrap();
        assert_eq!(deadline - monotonic, Duration::from_millis(100));
        assert!(
            registration_deadline(101, monotonic, UNIX_EPOCH + Duration::from_secs(101)).is_err()
        );
        assert!(
            registration_deadline(101, monotonic, UNIX_EPOCH + Duration::from_millis(101_001))
                .is_err()
        );
        assert!(registration_deadline(u64::MAX, monotonic, UNIX_EPOCH).is_err());
        assert_eq!(
            registration_deadline(10_000, monotonic, UNIX_EPOCH).unwrap() - monotonic,
            Duration::from_secs(MAX_LIFETIME)
        );
    }

    #[test]
    fn original_prompt_bytes_and_request_contract_are_preserved() {
        let corpus = super::super::load_corpus().unwrap();
        // Independently derived from the pre-change b9c8312 system literal and corpus serialization.
        assert_eq!(
            prompt_hash(&corpus).unwrap(),
            "399a073f310abb22fd77761c3ca33e92e72f87e0afcbe57ad10b2ccc5b9311e2"
        );
        for case in &corpus.semantic_cases {
            let request = super::super::judge_request(case, &corpus.rubric_version);
            let original = format!(
                "DIMENSION: {:?}\nEVAL_CASE:\n{}",
                case.dimension,
                serde_json::to_string(&case.input).unwrap()
            );
            assert_eq!(request.messages[1].content.as_bytes(), original.as_bytes());
            assert_eq!(request.max_tokens, Some(800));
            assert_eq!(request.thinking, Some(false));
            assert_eq!(request.temperature, Some(0.0));
            assert!(request.json_mode);
        }
    }

    #[test]
    fn evidence_serialization_bounds() {
        let body = vec![255; MIB];
        let record = ResponseRecord {
            schema_version: 1,
            sequence: 40,
            case_id: &"x".repeat(100),
            operation: "offline_evaluation",
            logical_attempt: 1,
            http_status: 200,
            complete: true,
            body: &body,
        };
        let bytes = bounded_json(&record, RAW_RECORD_LIMIT - 1).unwrap();
        assert!(bytes.len() > 4 * MIB);
        assert!(bytes.len() < RAW_RECORD_LIMIT);
        const {
            assert!(40 * RAW_RECORD_LIMIT < RAW_TOTAL_LIMIT);
        }
        const {
            assert!(
                RAW_TOTAL_LIMIT + MIB + REPORT_LIMIT + 128 * 1024 + 64 * 1024 + 32 * 1024 + 4096
                    < 163 * MIB
            );
        }
        assert!(bounded_json(&record, bytes.len() - 1).is_err());
        assert!(bounded_json(&record, bytes.len()).is_ok());
        for limit in [4096, 32 * 1024, 64 * 1024, 128 * 1024, REPORT_LIMIT, MIB] {
            assert!(bounded_json(&"x".repeat(limit - 2), limit).is_ok());
            assert!(bounded_json(&"x".repeat(limit - 1), limit).is_err());
        }
    }
}
