use crate::guard::models::ValidateVerdict;
use std::process::Stdio;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;
use tracing::{info, warn};

/// Host paths a guarded action's working directory may never sit under:
/// the sandbox mounts the directory read-only at /workspace, so anything
/// here would be exposed to the evaluated payload.
const FORBIDDEN_PREFIXES: &[&str] = &[
    "/etc", "/var/run", "/root", "/sys", "/dev", "/proc", "/boot",
];

fn is_forbidden_cwd(cwd: &str) -> bool {
    cwd == "/"
        || FORBIDDEN_PREFIXES
            .iter()
            .any(|prefix| cwd.starts_with(prefix))
}

/// The container image the sandbox evaluates payloads in.
const SANDBOX_IMAGE: &str = "alpine:latest";

/// Arguments for the ephemeral evaluation container: --rm (ephemeral),
/// --network=none (isolated), the working directory mounted read-only at
/// /workspace, a tmpfs scratch /tmp, payload via `sh -c`.
fn run_args(cwd: &str, payload: &str) -> Vec<String> {
    vec![
        "run".to_string(),
        "--rm".to_string(),
        "--network=none".to_string(),
        format!("-v={}:/workspace:ro", cwd),
        "--tmpfs=/tmp".to_string(),
        "-w=/workspace".to_string(),
        SANDBOX_IMAGE.to_string(),
        "sh".to_string(),
        "-c".to_string(),
        payload.to_string(),
    ]
}

/// The evaluation command: `run_args`, output piped for the verdict.
fn sandbox_command(engine: &str, cwd: &str, payload: &str) -> Command {
    let mut cmd = Command::new(engine);
    cmd.args(run_args(cwd, payload))
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

pub struct SandboxEvaluator {
    engine: String, // "podman" or "docker"
    available: bool,
}

impl SandboxEvaluator {
    pub async fn new() -> Self {
        let (engine, available) = Self::detect_engine().await;
        Self { engine, available }
    }

    async fn detect_engine() -> (String, bool) {
        // Prefer podman (per AGENTS.md: ALWAYS use podman, NEVER use docker)
        if let Ok(output) = Command::new("podman").arg("--version").output().await {
            if output.status.success() {
                return ("podman".to_string(), true);
            }
        }

        // No fallback to docker per project constraints
        ("none".to_string(), false)
    }

    pub async fn evaluate(
        &self,
        action_type: &str,
        payload: &str,
        cwd: &str,
    ) -> anyhow::Result<ValidateVerdict> {
        if !self.available {
            return Ok(ValidateVerdict::DeterministicReject {
                reasons: vec!["No container engine (Podman) available. Sandbox evaluation required. Please install Podman for full security.".to_string()],
                constraints: vec![],
            });
        }

        // Validate and canonicalize cwd to prevent host path exfiltration
        let canonical_cwd =
            std::fs::canonicalize(cwd).map_err(|_| anyhow::anyhow!("Invalid cwd path"))?;
        let cwd_str = canonical_cwd.to_string_lossy().to_string();

        if is_forbidden_cwd(&cwd_str) {
            return Ok(ValidateVerdict::DeterministicReject {
                reasons: vec![format!(
                    "Security violation: Attempted to mount forbidden host path {}",
                    cwd_str
                )],
                constraints: vec![
                    "Never attempt to interact with or mount sensitive host system paths."
                        .to_string(),
                ],
            });
        }

        // We only sandbox bash or known script commands for now
        if action_type != "bash" && action_type != "script" {
            return Ok(ValidateVerdict::DeterministicReject {
                reasons: vec![format!(
                    "Sandbox evaluation not supported for action_type: {}",
                    action_type
                )],
                constraints: vec![],
            });
        }

        info!("Executing sandbox evaluation using engine: {}", self.engine);

        let start = std::time::Instant::now();

        // Command execution inside ephemeral container (see run_args)
        let child = sandbox_command(&self.engine, &cwd_str, payload).spawn()?;

        // Wait with a strict timeout (e.g., 5 seconds for inline guard)
        let execution_result = timeout(Duration::from_secs(5), child.wait_with_output()).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match execution_result {
            Ok(Ok(output)) => {
                let mut stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                let mut stderr = String::from_utf8_lossy(&output.stderr).into_owned();

                // Truncate logs to prevent LLM context overflow which causes death spirals and compaction errors
                if stdout.len() > 500 {
                    stdout = format!(
                        "...[TRUNCATED {} chars]...\n{}",
                        stdout.len() - 500,
                        &stdout[stdout.len() - 500..]
                    );
                }
                if stderr.len() > 500 {
                    stderr = format!(
                        "...[TRUNCATED {} chars]...\n{}",
                        stderr.len() - 500,
                        &stderr[stderr.len() - 500..]
                    );
                }

                if output.status.success() {
                    Ok(ValidateVerdict::SandboxPassLowFidelity {
                        notes: vec![format!("Execution succeeded in {}ms", duration_ms)],
                    })
                } else {
                    // It failed the dry-run
                    Ok(ValidateVerdict::SandboxReject {
                        reasons: vec![format!(
                            "Sandbox returned exit code: {:?}",
                            output.status.code()
                        )],
                        logs: format!("STDOUT:\n{}\nSTDERR:\n{}", stdout, stderr),
                        constraints: vec![], // Injected later by main
                    })
                }
            }
            Ok(Err(e)) => {
                warn!("Sandbox execution failed to run: {}", e);
                Ok(ValidateVerdict::SandboxReject {
                    reasons: vec![format!("Sandbox execution error: {}", e)],
                    logs: String::new(),
                    constraints: vec![],
                })
            }
            Err(_) => {
                // Timeout
                warn!("Sandbox execution timed out after 5 seconds");
                Ok(ValidateVerdict::SandboxReject {
                    reasons: vec!["Sandbox timed out".to_string()],
                    logs: String::new(),
                    constraints: vec![],
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Instant;

    fn args() -> Vec<String> {
        run_args("/work/project", "echo hello")
    }

    #[test]
    fn run_args_pin_image_by_digest() {
        let a = args();
        assert!(
            a.iter().all(|x| !x.contains(":latest")),
            "no floating :latest tag may reach the invocation: {a:?}"
        );
        let image = a
            .iter()
            .find(|x| x.starts_with("alpine@sha256:"))
            .expect("the image is referenced as alpine@sha256:<digest>");
        let digest = image.strip_prefix("alpine@sha256:").unwrap();
        assert_eq!(digest.len(), 64, "a full sha256 hex digest: {image}");
        assert!(
            digest.chars().all(|c| c.is_ascii_hexdigit() && !c.is_uppercase()),
            "lowercase hex digest: {image}"
        );
        assert_eq!(
            image, SANDBOX_IMAGE,
            "the invocation uses the pinned image constant"
        );
    }

    #[test]
    fn run_args_drop_all_capabilities() {
        let a = args();
        assert!(
            a.iter().any(|x| x == "--cap-drop=ALL"),
            "the invocation drops every capability: {a:?}"
        );
    }

    #[test]
    fn run_args_set_pid_and_memory_limits() {
        // DR-3 register values, ridden at their defaults.
        let a = args();
        assert!(
            a.iter().any(|x| x == "--pids-limit=256"),
            "the invocation sets the DR-3 pids limit: {a:?}"
        );
        assert!(
            a.iter().any(|x| x == "--memory=512m"),
            "the invocation sets the DR-3 memory limit: {a:?}"
        );
    }

    #[test]
    fn home_is_a_forbidden_prefix() {
        assert!(is_forbidden_cwd("/home"));
        assert!(is_forbidden_cwd("/home/user/project"));
        // Pins: the pre-existing denylist entries still refuse, and an
        // ordinary project path is not refused.
        assert!(is_forbidden_cwd("/etc"));
        assert!(is_forbidden_cwd("/"));
        assert!(!is_forbidden_cwd("/work/project"));
    }

    #[test]
    fn network_is_none() {
        let a = args();
        assert!(
            a.iter().any(|x| x == "--network=none"),
            "the container is network-isolated: {a:?}"
        );
    }

    #[test]
    fn workspace_mount_is_read_only() {
        let a = args();
        assert!(
            a.iter().any(|x| x == "-v=/work/project:/workspace:ro"),
            "the working directory is mounted read-only: {a:?}"
        );
    }

    #[cfg(unix)]
    fn process_alive(pid: i32) -> bool {
        // kill(pid, 0): existence probe, delivers no signal.
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Behavioral leg of the timeout contract: a child spawned through
    /// `sandbox_command` that outlives the caller's timeout must be
    /// KILLED when the timed-out wait is dropped, not left running —
    /// otherwise the 5s guard timeout abandons a live container. A stub
    /// "engine" script stands in for the container CLI: it records its
    /// pid, then execs a long sleep (a hung evaluation's exact shape).
    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_the_child() {
        let dir = std::env::temp_dir().join(format!(
            "ns-sandbox-timeout-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("child.pid");
        let engine = dir.join("stub-engine");
        {
            let mut f = std::fs::File::create(&engine).unwrap();
            writeln!(f, "#!/bin/sh").unwrap();
            writeln!(f, "echo $$ > {}", pidfile.display()).unwrap();
            writeln!(f, "exec sleep 60").unwrap();
        }
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&engine, std::fs::Permissions::from_mode(0o755)).unwrap();
        }

        let child = sandbox_command(engine.to_str().unwrap(), "/work/project", "ignored")
            .spawn()
            .expect("the stub engine spawns");
        let outcome = timeout(Duration::from_millis(500), child.wait_with_output()).await;
        assert!(outcome.is_err(), "the stub outlives the 500ms timeout");

        // The stub records its pid at startup; read it, then require the
        // process to die promptly once the timed-out wait was dropped.
        let mut pid = None;
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline {
            if let Ok(text) = std::fs::read_to_string(&pidfile) {
                if let Ok(n) = text.trim().parse::<i32>() {
                    pid = Some(n);
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let pid = pid.expect("the stub recorded its pid");
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut alive = true;
        while Instant::now() < deadline {
            alive = process_alive(pid);
            if !alive {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert!(
            !alive,
            "child pid {pid} survived the dropped timeout — the sandbox would leave the container running"
        );
    }
}
