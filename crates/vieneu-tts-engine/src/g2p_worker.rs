/****
 * [ATOM]: g2p_worker
 * Chức năng: Quản lý persistent child process để thực thi G2P (Grapheme-to-Phoneme)
 * qua IPC (stdin/stdout). Tự động hồi sinh (Self-Healing Auto-Restart) nếu tiến trình bị đóng pipe.
 ****/
use crate::log_event;
use anyhow::{Result, anyhow};
use serde::{Deserialize, Serialize};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use vieneu_tts_core::events::EventCallback;

#[derive(Serialize)]
struct G2pRequest {
    text: String,
}

#[derive(Deserialize)]
struct G2pResponse {
    success: bool,
    phonemes: Option<String>,
    error: Option<String>,
}

pub struct G2pWorker {
    model_dir: PathBuf,
    bridge_path: Option<PathBuf>,
    python_path: Option<PathBuf>,
    events: Option<EventCallback>,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl G2pWorker {
    fn spawn_process(
        model_dir: &Path,
        bridge_path: Option<&Path>,
        python_path: Option<&Path>,
    ) -> Result<(Child, ChildStdin, BufReader<ChildStdout>)> {
        // Use the explicitly configured interpreter or the system PATH.
        let mut python_cands = Vec::new();
        if let Some(path) = python_path {
            python_cands.push(path.to_path_buf());
        } else {
            python_cands.push(PathBuf::from("python3"));
            python_cands.push(PathBuf::from("python"));
        }

        // Resolve only the configured bridge or files shipped with the model directory.
        let mut bridge_cands = Vec::new();
        if let Some(path) = bridge_path {
            bridge_cands.push(path.to_path_buf());
        }
        if cfg!(windows) {
            bridge_cands.push(model_dir.join("g2p_bridge").join("g2p_bridge.exe"));
            bridge_cands.push(model_dir.join("g2p_bridge.exe"));
        } else {
            bridge_cands.push(model_dir.join("g2p_bridge").join("g2p_bridge"));
            bridge_cands.push(model_dir.join("g2p_bridge"));
        }

        let py_script_cands = [model_dir.join("g2p_bridge.py")];

        for b_path in &bridge_cands {
            if b_path.exists() {
                let mut cmd = Command::new(b_path);
                if let Some(parent) = b_path.parent() {
                    cmd.current_dir(parent);
                }
                if let Ok(mut child) = cmd
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::null())
                    .spawn()
                    && let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take())
                {
                    return Ok((child, stdin, BufReader::new(stdout)));
                }
            }
        }

        for script in &py_script_cands {
            if script.exists() {
                for py in &python_cands {
                    if let Ok(mut child) = Command::new(py)
                        .arg(script)
                        .stdin(Stdio::piped())
                        .stdout(Stdio::piped())
                        .stderr(Stdio::null())
                        .spawn()
                        && let (Some(stdin), Some(stdout)) =
                            (child.stdin.take(), child.stdout.take())
                    {
                        return Ok((child, stdin, BufReader::new(stdout)));
                    }
                }
            }
        }

        Err(anyhow!(
            "Không tìm thấy micro-worker G2P (g2p_bridge hoặc Python script với package vig2p/vieneu_utils)."
        ))
    }

    pub fn new(
        model_dir: &Path,
        bridge_path: Option<&Path>,
        python_path: Option<&Path>,
        events: Option<EventCallback>,
    ) -> Result<Self> {
        let (child, stdin, stdout) = Self::spawn_process(model_dir, bridge_path, python_path)?;
        Ok(Self {
            model_dir: model_dir.to_path_buf(),
            bridge_path: bridge_path.map(Path::to_path_buf),
            python_path: python_path.map(Path::to_path_buf),
            events,
            child,
            stdin,
            stdout,
        })
    }

    fn restart(&mut self) -> Result<()> {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let (child, stdin, stdout) = Self::spawn_process(
            &self.model_dir,
            self.bridge_path.as_deref(),
            self.python_path.as_deref(),
        )?;
        self.child = child;
        self.stdin = stdin;
        self.stdout = stdout;
        Ok(())
    }

    fn try_phonemize_once(&mut self, text: &str) -> Result<String> {
        let req = G2pRequest {
            text: text.to_string(),
        };
        let req_json = serde_json::to_string(&req)?;

        writeln!(self.stdin, "{}", req_json)?;
        self.stdin.flush()?;

        let mut res_str = String::new();
        self.stdout.read_line(&mut res_str)?;

        if res_str.trim().is_empty() {
            return Err(anyhow!("G2P worker đột ngột đóng kết nối hoặc crash"));
        }

        let res: G2pResponse = serde_json::from_str(&res_str)
            .map_err(|e| anyhow!("Lỗi phân tích JSON từ G2P: {} (Raw: {})", e, res_str))?;

        if res.success {
            res.phonemes
                .ok_or_else(|| anyhow!("G2P worker trả về thành công nhưng thiếu phonemes"))
        } else {
            Err(anyhow!(
                "G2P worker báo lỗi: {}",
                res.error
                    .unwrap_or_else(|| "không có thông tin lỗi".to_string())
            ))
        }
    }

    pub fn phonemize(&mut self, text: &str) -> Result<String> {
        for attempt in 0..3 {
            match self.try_phonemize_once(text) {
                Ok(res) => return Ok(res),
                Err(e) => {
                    log_event(
                        &self.events,
                        format!(
                            "G2P lần {} thất bại: {e}; đang khởi động lại worker.",
                            attempt + 1
                        ),
                    );
                    if let Err(restart_err) = self.restart() {
                        log_event(
                            &self.events,
                            format!("Không thể khởi động lại G2P worker: {restart_err}"),
                        );
                    }
                }
            }
        }
        Err(anyhow!("G2P worker thất bại sau 3 lần thử"))
    }
}

impl Drop for G2pWorker {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
