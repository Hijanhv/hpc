//! Execution of daemon-issued commands on the node.
//!
//! Filesystem operations are genuinely destructive and require root, so the
//! executor is deliberately conservative:
//!
//! * When `allow_exec` is **false** (the default) every command is *validated*
//!   and the exact argv that would run is logged and returned — a dry-run. This
//!   makes the agent safe to run anywhere, including CI and demos.
//! * When `allow_exec` is **true** the command is actually spawned and its exit
//!   status / stdout / stderr are captured into the [`CommandOutcome`].
//!
//! Destructive actions (`format`) additionally require `force` to be set,
//! enforced both here and at the daemon's REST boundary.

use hpc_core::types::{now_unix, CommandOutcome, DeploySpec, FsAction, FsSpec};

/// Carries out commands for one node.
#[derive(Debug, Clone)]
pub struct Executor {
    node_id: String,
    allow_exec: bool,
}

impl Executor {
    /// Create an executor for `node_id`. `allow_exec` gates real side effects.
    pub fn new(node_id: impl Into<String>, allow_exec: bool) -> Self {
        Executor {
            node_id: node_id.into(),
            allow_exec,
        }
    }

    fn outcome(&self, id: &str, success: bool, exit_code: i32, message: String) -> CommandOutcome {
        CommandOutcome {
            command_id: id.to_string(),
            node_id: self.node_id.clone(),
            success,
            exit_code,
            message,
            stdout: String::new(),
            stderr: String::new(),
            completed_at_unix: now_unix(),
        }
    }

    /// Execute a deploy command. Deploys are always simulated (there is no
    /// universal package manager to shell out to); the intent is logged and
    /// acknowledged so the control plane's audit trail is complete.
    pub async fn run_deploy(&self, spec: &DeploySpec) -> CommandOutcome {
        tracing::info!(
            deployment_id = %spec.deployment_id,
            action = ?spec.action,
            component = %spec.component,
            version = %spec.version,
            "handling deploy command"
        );
        let message = format!(
            "{:?} {} v{} -> {} (simulated)",
            spec.action,
            spec.component,
            if spec.version.is_empty() {
                "latest"
            } else {
                &spec.version
            },
            if spec.target_path.is_empty() {
                "<default>"
            } else {
                &spec.target_path
            }
        );
        self.outcome(&spec.deployment_id, true, 0, message)
    }

    /// Execute a filesystem command (mount/unmount/remount/check/format).
    pub async fn run_fs(&self, spec: &FsSpec) -> CommandOutcome {
        let argv = match self.build_argv(spec) {
            Ok(argv) => argv,
            Err(message) => {
                tracing::warn!(command_id = %spec.command_id, %message, "invalid fs command");
                return self.outcome(&spec.command_id, false, -1, message);
            }
        };

        if !self.allow_exec {
            let message = format!("dry-run: would execute `{}`", argv.join(" "));
            tracing::info!(command_id = %spec.command_id, %message, "fs command (dry-run)");
            return self.outcome(&spec.command_id, true, 0, message);
        }

        tracing::info!(command_id = %spec.command_id, cmd = %argv.join(" "), "executing fs command");
        match tokio::process::Command::new(&argv[0])
            .args(&argv[1..])
            .output()
            .await
        {
            Ok(output) => {
                let exit_code = output.status.code().unwrap_or(-1);
                let mut outcome = self.outcome(
                    &spec.command_id,
                    output.status.success(),
                    exit_code,
                    format!("`{}` exited with {}", argv.join(" "), exit_code),
                );
                outcome.stdout = String::from_utf8_lossy(&output.stdout).into_owned();
                outcome.stderr = String::from_utf8_lossy(&output.stderr).into_owned();
                outcome
            }
            Err(e) => self.outcome(
                &spec.command_id,
                false,
                -1,
                format!("failed to spawn `{}`: {e}", argv.join(" ")),
            ),
        }
    }

    /// Translate an [`FsSpec`] into a concrete argv, validating required fields.
    fn build_argv(&self, spec: &FsSpec) -> std::result::Result<Vec<String>, String> {
        let opts = spec.mount_options.join(",");
        match spec.action {
            FsAction::Mount => {
                if spec.device.is_empty() || spec.mount_point.is_empty() {
                    return Err("mount requires device and mount_point".into());
                }
                let mut argv = vec!["mount".to_string()];
                if !spec.fs_type.is_empty() {
                    argv.push("-t".into());
                    argv.push(spec.fs_type.clone());
                }
                if !opts.is_empty() {
                    argv.push("-o".into());
                    argv.push(opts);
                }
                argv.push(spec.device.clone());
                argv.push(spec.mount_point.clone());
                Ok(argv)
            }
            FsAction::Unmount => {
                if spec.mount_point.is_empty() {
                    return Err("unmount requires mount_point".into());
                }
                Ok(vec!["umount".into(), spec.mount_point.clone()])
            }
            FsAction::Remount => {
                if spec.mount_point.is_empty() {
                    return Err("remount requires mount_point".into());
                }
                let remount_opts = if opts.is_empty() {
                    "remount".to_string()
                } else {
                    format!("remount,{opts}")
                };
                Ok(vec![
                    "mount".into(),
                    "-o".into(),
                    remount_opts,
                    spec.mount_point.clone(),
                ])
            }
            FsAction::Check => {
                if spec.device.is_empty() {
                    return Err("check requires device".into());
                }
                // `-n` => never modify, a safe consistency check.
                Ok(vec!["fsck".into(), "-n".into(), spec.device.clone()])
            }
            FsAction::Format => {
                if !spec.force {
                    return Err("format is destructive and requires force=true".into());
                }
                if spec.device.is_empty() || spec.fs_type.is_empty() {
                    return Err("format requires device and fs_type".into());
                }
                Ok(vec![format!("mkfs.{}", spec.fs_type), spec.device.clone()])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fs(action: FsAction) -> FsSpec {
        FsSpec {
            command_id: "c1".into(),
            action,
            device: "/dev/sdb1".into(),
            mount_point: "/mnt/data".into(),
            fs_type: "xfs".into(),
            mount_options: vec!["noatime".into()],
            force: false,
        }
    }

    #[tokio::test]
    async fn dry_run_mount_is_success_without_side_effects() {
        let exec = Executor::new("n1", false);
        let outcome = exec.run_fs(&fs(FsAction::Mount)).await;
        assert!(outcome.success);
        assert!(outcome.message.contains("dry-run"));
        assert!(outcome.message.contains("/dev/sdb1"));
    }

    #[tokio::test]
    async fn format_without_force_is_rejected() {
        let exec = Executor::new("n1", true);
        let outcome = exec.run_fs(&fs(FsAction::Format)).await;
        assert!(!outcome.success);
        assert!(outcome.message.contains("force"));
    }
}
