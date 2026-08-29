//! Process-out adapters for pi (JSONL RPC) and deepseek-harness.

use aria_router_agent::{parse_decision_json, AgentExtension, RouteTask};
use aria_router_config::ExtensionCfg;
use aria_router_core::{RouteDecision, RouterError};
use async_trait::async_trait;
use std::path::Path;
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::time::{timeout, Duration};

pub struct SubprocessExtension {
    pub cfg: ExtensionCfg,
}

impl SubprocessExtension {
    pub fn ensure_binary(&self) -> Result<(), RouterError> {
        let cmd = self.cfg.command.first().ok_or_else(|| {
            RouterError::Config(format!("extension {} missing command", self.cfg.name))
        })?;
        if Path::new(cmd).is_file() {
            return Ok(());
        }
        // Allow PATH lookup: try `which`-style by spawning `command -v` is messy;
        // if it's a bare name, we still try at route time. For serve-time check,
        // require either absolute path exists or it is on PATH via `which`.
        which(cmd).ok_or_else(|| {
            RouterError::Extension(format!(
                "extension {} binary not found: {cmd}",
                self.cfg.name
            ))
        })?;
        Ok(())
    }
}

fn which(cmd: &str) -> Option<String> {
    if cmd.contains('/') && Path::new(cmd).exists() {
        return Some(cmd.into());
    }
    let path = std::env::var("PATH").ok()?;
    for dir in path.split(':') {
        let p = Path::new(dir).join(cmd);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

#[async_trait]
impl AgentExtension for SubprocessExtension {
    fn name(&self) -> &str {
        &self.cfg.name
    }

    async fn route(&self, task: RouteTask) -> Result<RouteDecision, RouterError> {
        let (prog, args) = self.cfg.command.split_first().ok_or_else(|| {
            RouterError::Config(format!("extension {} missing command", self.cfg.name))
        })?;
        let mut cmd = Command::new(prog);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(wd) = &self.cfg.workdir {
            cmd.current_dir(wd);
        }
        for (k, v) in &self.cfg.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| {
            RouterError::Extension(format!("spawn {}: {e}", self.cfg.name))
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            RouterError::Extension("stdin".into())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            RouterError::Extension("stdout".into())
        })?;

        let payload = match self.cfg.ext_type.as_str() {
            "pi" => {
                serde_json::json!({
                    "type": "prompt",
                    "message": format!(
                        "{}\nEligible: {}\nReply with JSON {{\"model\":\"...\",\"reason\":\"...\",\"confidence\":0.8}} only.",
                        task.prompt,
                        task.eligible.iter().map(|m| m.name.as_str()).collect::<Vec<_>>().join(",")
                    )
                })
            }
            _ => {
                serde_json::json!({
                    "prompt": task.prompt,
                    "eligible": task.eligible.iter().map(|m| &m.name).collect::<Vec<_>>(),
                    "system": task.system
                })
            }
        };
        let line = serde_json::to_string(&payload).unwrap() + "\n";
        let io = async {
            stdin.write_all(line.as_bytes()).await.map_err(|e| {
                RouterError::Extension(e.to_string())
            })?;
            stdin.flush().await.map_err(|e| RouterError::Extension(e.to_string()))?;
            drop(stdin);
            let mut reader = BufReader::new(stdout);
            let mut out = String::new();
            reader.read_line(&mut out).await.map_err(|e| RouterError::Extension(e.to_string()))?;
            Ok::<_, RouterError>(out)
        };
        let out = timeout(Duration::from_millis(task.timeout_ms), io)
            .await
            .map_err(|_| RouterError::Timeout(self.cfg.name.clone()))??;
        let _ = child.kill().await;
        let text = extract_text(&self.cfg.ext_type, out.trim());
        parse_decision_json(&text, &task.eligible)
    }
}

fn extract_text(ext_type: &str, line: &str) -> String {
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(line) {
        if ext_type == "pi" {
            if let Some(s) = v.pointer("/message/content").and_then(|x| x.as_str()) {
                return s.to_string();
            }
            if let Some(s) = v.get("data").and_then(|d| d.as_str()) {
                return s.to_string();
            }
        }
        if let Some(s) = v.get("decision").cloned() {
            return s.to_string();
        }
    }
    line.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use aria_router_core::ModelCard;

    #[tokio::test]
    async fn mock_script_json() {
        let dir = std::env::temp_dir();
        let script = dir.join("aria-router-ext-mock.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nread line\necho '{\"model\":\"local/general\",\"reason\":\"mock\",\"confidence\":0.9}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&script).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&script, p).unwrap();
        }
        let ext = SubprocessExtension {
            cfg: ExtensionCfg {
                name: "mock".into(),
                ext_type: "deepseek-harness".into(),
                command: vec![script.to_string_lossy().into_owned()],
                workdir: None,
                timeout_ms: Some(2000),
                env: Default::default(),
                endpoint: None,
            },
        };
        ext.ensure_binary().unwrap();
        let task = RouteTask {
            prompt: "hi".into(),
            eligible: vec![ModelCard {
                name: "local/general".into(),
                locality: "local".into(),
                modality: "text".into(),
                capabilities: vec![],
                provider_model_id: "x".into(),
            }],
            system: "x".into(),
            timeout_ms: 2000,
            max_turns: 1,
        };
        let d = ext.route(task).await.unwrap();
        assert_eq!(d.model, "local/general");
    }

    #[test]
    fn missing_binary_fails() {
        let ext = SubprocessExtension {
            cfg: ExtensionCfg {
                name: "pi".into(),
                ext_type: "pi".into(),
                command: vec!["definitely-not-installed-pi-bin".into()],
                workdir: None,
                timeout_ms: Some(10),
                env: Default::default(),
                endpoint: None,
            },
        };
        assert!(ext.ensure_binary().is_err());
    }

    #[tokio::test]
    async fn mock_pi_jsonl() {
        let dir = std::env::temp_dir();
        let script = dir.join("aria-router-pi-mock.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nread line\necho '{\"data\":\"{\\\"model\\\":\\\"local/general\\\",\\\"reason\\\":\\\"pi\\\",\\\"confidence\\\":0.7}\"}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut p = std::fs::metadata(&script).unwrap().permissions();
            p.set_mode(0o755);
            std::fs::set_permissions(&script, p).unwrap();
        }
        let ext = SubprocessExtension {
            cfg: ExtensionCfg {
                name: "pi".into(),
                ext_type: "pi".into(),
                command: vec![script.to_string_lossy().into_owned()],
                workdir: None,
                timeout_ms: Some(2000),
                env: Default::default(),
                endpoint: None,
            },
        };
        let task = RouteTask {
            prompt: "hi".into(),
            eligible: vec![ModelCard {
                name: "local/general".into(),
                locality: "local".into(),
                modality: "text".into(),
                capabilities: vec![],
                provider_model_id: "x".into(),
            }],
            system: "x".into(),
            timeout_ms: 2000,
            max_turns: 1,
        };
        let d = ext.route(task).await.unwrap();
        assert_eq!(d.model, "local/general");
    }
}
