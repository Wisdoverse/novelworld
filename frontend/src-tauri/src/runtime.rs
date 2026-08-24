use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
use tauri::{AppHandle, Manager};
use uuid::Uuid;

const PG_PORT: u16 = 35_432;
const USER_PORT: u16 = 38_001;
const NOVEL_PORT: u16 = 38_002;
const AGENT_PORT: u16 = 38_003;
const NARRATIVE_PORT: u16 = 38_004;
const GATEWAY_PORT: u16 = 38_080;
const INSTANCE_NAME: &str = "novelworld-desktop";

const INITIAL_SCHEMA: &str = include_str!("../../../infra/postgres/init.sql");
const MIGRATIONS: &[(&str, &str)] = &[
    (
        "0001_runtime_contract.sql",
        include_str!("../../../infra/postgres/migrations/0001_runtime_contract.sql"),
    ),
    (
        "0002_reading_progress_contract.sql",
        include_str!("../../../infra/postgres/migrations/0002_reading_progress_contract.sql"),
    ),
    (
        "0003_chat_turn_contract.sql",
        include_str!("../../../infra/postgres/migrations/0003_chat_turn_contract.sql"),
    ),
    (
        "0004_narrative_choice_contract.sql",
        include_str!("../../../infra/postgres/migrations/0004_narrative_choice_contract.sql"),
    ),
    (
        "0005_remove_default_seed.sql",
        include_str!("../../../infra/postgres/migrations/0005_remove_default_seed.sql"),
    ),
    (
        "0006_runtime_llm_config.sql",
        include_str!("../../../infra/postgres/migrations/0006_runtime_llm_config.sql"),
    ),
    (
        "0007_chapter_lore_search.sql",
        include_str!("../../../infra/postgres/migrations/0007_chapter_lore_search.sql"),
    ),
    (
        "0008_llm_thinking_mode.sql",
        include_str!("../../../infra/postgres/migrations/0008_llm_thinking_mode.sql"),
    ),
    (
        "0009_narrative_inline_anchor.sql",
        include_str!("../../../infra/postgres/migrations/0009_narrative_inline_anchor.sql"),
    ),
    (
        "0010_player_timeline_chapters.sql",
        include_str!("../../../infra/postgres/migrations/0010_player_timeline_chapters.sql"),
    ),
    (
        "0011_canon_story_models.sql",
        include_str!("../../../infra/postgres/migrations/0011_canon_story_models.sql"),
    ),
    (
        "0012_narrative_transitions.sql",
        include_str!("../../../infra/postgres/migrations/0012_narrative_transitions.sql"),
    ),
    (
        "0013_living_world_turns.sql",
        include_str!("../../../infra/postgres/migrations/0013_living_world_turns.sql"),
    ),
    (
        "0014_source_file_storage.sql",
        include_str!("../../../infra/postgres/migrations/0014_source_file_storage.sql"),
    ),
    (
        "0015_durable_import_jobs.sql",
        include_str!("../../../infra/postgres/migrations/0015_durable_import_jobs.sql"),
    ),
    (
        "0016_erasure_records.sql",
        include_str!("../../../infra/postgres/migrations/0016_erasure_records.sql"),
    ),
    (
        "0017_canon_extraction_checkpoints.sql",
        include_str!("../../../infra/postgres/migrations/0017_canon_extraction_checkpoints.sql"),
    ),
    (
        "0018_expand_canon_checkpoint_source.sql",
        include_str!("../../../infra/postgres/migrations/0018_expand_canon_checkpoint_source.sql"),
    ),
    (
        "0019_share_novels_across_user_shelves.sql",
        include_str!(
            "../../../infra/postgres/migrations/0019_share_novels_across_user_shelves.sql"
        ),
    ),
    (
        "0020_advanced_game_rules.sql",
        include_str!("../../../infra/postgres/migrations/0020_advanced_game_rules.sql"),
    ),
    (
        "0021_world_turn_memory_projection.sql",
        include_str!("../../../infra/postgres/migrations/0021_world_turn_memory_projection.sql"),
    ),
];

#[derive(Serialize, Deserialize)]
struct Secrets {
    jwt: String,
    database: String,
    runtime_config: String,
    internal_service: String,
}

impl Secrets {
    fn generate() -> Self {
        Self {
            jwt: random_hex(),
            database: random_hex(),
            runtime_config: random_hex(),
            internal_service: random_hex(),
        }
    }
}

pub struct DesktopRuntime {
    children: Vec<Child>,
    pg0: PathBuf,
}

impl DesktopRuntime {
    pub fn start(app: &AppHandle) -> Result<Self> {
        let app_dir = match std::env::var_os("NOVELWORLD_DESKTOP_DATA_DIR") {
            Some(path) => PathBuf::from(path),
            None => app
                .path()
                .app_data_dir()
                .context("desktop data directory is unavailable")?,
        };
        let runtime_dir = app_dir.join("runtime");
        let log_dir = app_dir.join("logs");
        fs::create_dir_all(&runtime_dir)?;
        fs::create_dir_all(&log_dir)?;

        let secrets = load_or_create_secrets(&app_dir)?;
        let bin_dir = resource_bin_dir(app)?;
        let pg0 = executable(&bin_dir, "pg0")?;
        let _ = command(&pg0)
            .args(["stop", "--name", INSTANCE_NAME])
            .status();
        ensure_ports_available()?;

        let mut runtime = Self {
            children: Vec::new(),
            pg0,
        };
        runtime.start_postgres(&runtime_dir, &secrets)?;
        runtime.apply_schema(&runtime_dir)?;
        runtime.start_services(&bin_dir, &log_dir, &secrets)?;
        wait_for_port(GATEWAY_PORT, Duration::from_secs(30))?;
        Ok(runtime)
    }

    pub fn shutdown(&mut self) {
        while let Some(mut child) = self.children.pop() {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = command(&self.pg0)
            .args(["stop", "--name", INSTANCE_NAME])
            .status();
    }

    fn start_postgres(&self, runtime_dir: &Path, secrets: &Secrets) -> Result<()> {
        let status = command(&self.pg0)
            .arg("start")
            .args(["--name", INSTANCE_NAME])
            .args(["--port", &PG_PORT.to_string()])
            .arg("--data-dir")
            .arg(runtime_dir.join("postgres"))
            .args(["--username", "novel"])
            .args(["--password", &secrets.database])
            .args(["--database", "novel_world"])
            .args(["--config", "listen_addresses=127.0.0.1"])
            // PostgreSQL outlives the pg0 launcher. Capturing these handles would
            // keep Command::output waiting forever for EOF on Windows.
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("start embedded PostgreSQL")?;
        if !status.success() {
            bail!("start embedded PostgreSQL failed with {status}");
        }
        wait_for_port(PG_PORT, Duration::from_secs(30))
    }

    fn apply_schema(&self, runtime_dir: &Path) -> Result<()> {
        let sql_dir = runtime_dir.join("sql");
        let schema_marker = runtime_dir.join("schema-initialized");
        fs::create_dir_all(&sql_dir)?;

        if !schema_marker.exists() {
            let path = sql_dir.join("init.sql");
            fs::write(&path, INITIAL_SCHEMA)?;
            self.psql_file(&path)?;
            fs::write(&schema_marker, b"initialized\n")?;
        }

        for (name, sql) in MIGRATIONS {
            let path = sql_dir.join(name);
            fs::write(&path, sql)?;
            self.psql_file(&path)?;
        }
        Ok(())
    }

    fn psql_file(&self, path: &Path) -> Result<()> {
        run_checked(
            command(&self.pg0)
                .arg("psql")
                .args(["--name", INSTANCE_NAME, "-v", "ON_ERROR_STOP=1", "-f"])
                .arg(path),
            &format!("apply {}", path.display()),
        )
    }

    fn start_services(&mut self, bin_dir: &Path, log_dir: &Path, secrets: &Secrets) -> Result<()> {
        let database_url = format!(
            "postgres://novel:{}@127.0.0.1:{PG_PORT}/novel_world",
            secrets.database
        );
        let common = [
            ("DATABASE_URL", database_url.as_str()),
            ("BIND_ADDR", "127.0.0.1"),
            ("INTERNAL_SERVICE_TOKEN", secrets.internal_service.as_str()),
            ("RUNTIME_CONFIG_KEY", secrets.runtime_config.as_str()),
            ("RUST_LOG", "info"),
        ];

        self.spawn_service(
            bin_dir,
            log_dir,
            "user-service",
            &common,
            &[
                ("PORT", "38001"),
                ("JWT_SECRET", secrets.jwt.as_str()),
                ("AGENT_SERVICE_URL", "http://127.0.0.1:38003"),
            ],
            USER_PORT,
        )?;
        self.spawn_service(
            bin_dir,
            log_dir,
            "novel-service",
            &common,
            &[
                ("PORT", "38002"),
                ("USER_SERVICE_URL", "http://127.0.0.1:38001"),
                ("AGENT_SERVICE_URL", "http://127.0.0.1:38003"),
                ("S3_ENABLED", "false"),
            ],
            NOVEL_PORT,
        )?;
        self.spawn_service(
            bin_dir,
            log_dir,
            "narrative-service",
            &common,
            &[
                ("PORT", "38004"),
                ("USER_SERVICE_URL", "http://127.0.0.1:38001"),
                ("NOVEL_SERVICE_URL", "http://127.0.0.1:38002"),
                ("AGENT_SERVICE_URL", "http://127.0.0.1:38003"),
            ],
            NARRATIVE_PORT,
        )?;
        self.spawn_service(
            bin_dir,
            log_dir,
            "agent-service",
            &common,
            &[
                ("PORT", "38003"),
                ("REDIS_URL", "memory://"),
                ("USER_SERVICE_URL", "http://127.0.0.1:38001"),
                ("NOVEL_SERVICE_URL", "http://127.0.0.1:38002"),
                ("NARRATIVE_SERVICE_URL", "http://127.0.0.1:38004"),
            ],
            AGENT_PORT,
        )?;
        self.spawn_service(
            bin_dir,
            log_dir,
            "gateway",
            &common,
            &[
                ("PORT", "38080"),
                ("JWT_SECRET", secrets.jwt.as_str()),
                ("USER_SERVICE_URL", "http://127.0.0.1:38001"),
                ("NOVEL_SERVICE_URL", "http://127.0.0.1:38002"),
                ("AGENT_SERVICE_URL", "http://127.0.0.1:38003"),
                ("NARRATIVE_SERVICE_URL", "http://127.0.0.1:38004"),
                (
                    "CORS_ORIGINS",
                    "http://localhost:5173,http://tauri.localhost,tauri://localhost",
                ),
            ],
            GATEWAY_PORT,
        )?;
        Ok(())
    }

    fn spawn_service(
        &mut self,
        bin_dir: &Path,
        log_dir: &Path,
        name: &str,
        common: &[(&str, &str)],
        specific: &[(&str, &str)],
        port: u16,
    ) -> Result<()> {
        let executable = executable(bin_dir, name)?;
        let log = File::create(log_dir.join(format!("{name}.log")))?;
        let mut process = command(&executable);
        process
            .envs(common.iter().copied())
            .envs(specific.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone()?))
            .stderr(Stdio::from(log));
        let child = process.spawn().with_context(|| format!("start {name}"))?;
        self.children.push(child);
        wait_for_port(port, Duration::from_secs(20))
            .with_context(|| format!("{name} did not become ready"))
    }
}

impl Drop for DesktopRuntime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn resource_bin_dir(app: &AppHandle) -> Result<PathBuf> {
    let resources = app
        .path()
        .resource_dir()
        .context("resource directory is unavailable")?;
    for candidate in [resources.join("resources/bin"), resources.join("bin")] {
        if candidate.exists() {
            return Ok(candidate);
        }
    }
    bail!(
        "desktop runtime binaries are missing from {}",
        resources.display()
    )
}

fn executable(bin_dir: &Path, name: &str) -> Result<PathBuf> {
    let suffix = if cfg!(windows) { ".exe" } else { "" };
    let path = bin_dir.join(format!("{name}{suffix}"));
    if path.is_file() {
        Ok(path)
    } else {
        bail!(
            "required desktop runtime binary is missing: {}",
            path.display()
        )
    }
}

fn ensure_ports_available() -> Result<()> {
    for port in [
        PG_PORT,
        USER_PORT,
        NOVEL_PORT,
        AGENT_PORT,
        NARRATIVE_PORT,
        GATEWAY_PORT,
    ] {
        TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .with_context(|| format!("local port {port} is already in use"))?;
    }
    Ok(())
}

fn wait_for_port(port: u16, timeout: Duration) -> Result<()> {
    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port);
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_ok() {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(200));
    }
    bail!("timed out waiting for 127.0.0.1:{port}")
}

fn load_or_create_secrets(app_dir: &Path) -> Result<Secrets> {
    let path = app_dir.join("runtime-secrets.json");
    if path.exists() {
        return serde_json::from_slice(&fs::read(&path)?).context("desktop secrets are invalid");
    }
    let secrets = Secrets::generate();
    fs::write(&path, serde_json::to_vec(&secrets)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(secrets)
}

fn random_hex() -> String {
    format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple())
}

fn run_checked(command: &mut Command, action: &str) -> Result<()> {
    let output = command.output().with_context(|| action.to_owned())?;
    if output.status.success() {
        return Ok(());
    }
    bail!(
        "{action} failed: {}",
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn command(program: &Path) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    command
}

#[cfg(test)]
mod tests {
    use super::{random_hex, MIGRATIONS};

    #[test]
    fn generated_secrets_are_64_character_hex_values() {
        let value = random_hex();
        assert_eq!(value.len(), 64);
        assert!(value.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }

    #[test]
    fn embedded_migrations_are_complete_and_ordered() {
        assert_eq!(MIGRATIONS.first().unwrap().0, "0001_runtime_contract.sql");
        assert_eq!(
            MIGRATIONS.last().unwrap().0,
            "0021_world_turn_memory_projection.sql"
        );
        assert!(MIGRATIONS.windows(2).all(|pair| pair[0].0 < pair[1].0));
    }

    #[test]
    fn embedded_world_turn_projection_migration_keeps_its_authority_fences() {
        let migration = MIGRATIONS
            .iter()
            .find(|(name, _)| *name == "0021_world_turn_memory_projection.sql")
            .unwrap()
            .1;
        for required in [
            "IF NOT projection_status_exists THEN",
            "WHEN status = 'completed' THEN 'skipped'",
            "world_turns_memory_projection_status_check",
            "world_turns_memory_projection_state_check",
            "CREATE UNIQUE INDEX idx_world_turns_one_in_progress",
            "OR (status = 'completed' AND memory_projection_status = 'pending')",
        ] {
            assert!(migration.contains(required), "missing: {required}");
        }
    }
}
