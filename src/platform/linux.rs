use std::path::PathBuf;

use super::{ForegroundJob, ForegroundProcess};

/// Collect the foreground terminal job for a given child PID.
pub fn foreground_job(child_pid: u32) -> Option<ForegroundJob> {
    let tpgid = foreground_pgid(child_pid)? as u32;
    let mut processes = Vec::new();

    for entry in std::fs::read_dir("/proc").ok()? {
        let entry = entry.ok()?;
        let file_name = entry.file_name();
        let pid_str = file_name.to_str()?;
        if !pid_str.bytes().all(|b| b.is_ascii_digit()) {
            continue;
        }

        let pid: u32 = match pid_str.parse() {
            Ok(pid) => pid,
            Err(_) => continue,
        };

        let Some((pgrp, name)) = process_pgrp_and_comm(pid) else {
            continue;
        };
        if pgrp as u32 != tpgid {
            continue;
        }

        processes.push(ForegroundProcess {
            pid,
            name,
            argv0: None,
            cmdline: process_cmdline(pid),
        });
    }

    if processes.is_empty() {
        return None;
    }

    Some(ForegroundJob {
        process_group_id: tpgid,
        processes,
    })
}

fn foreground_pgid(child_pid: u32) -> Option<i32> {
    // /proc/<pid>/stat format: "pid (comm) state ppid pgrp session tty_nr tpgid ..."
    // The (comm) field can contain spaces and parens, so we find the last ')' first.
    let stat = std::fs::read_to_string(format!("/proc/{child_pid}/stat")).ok()?;
    let rest = stat.get(stat.rfind(')')? + 2..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    // After (comm): state(0) ppid(1) pgrp(2) session(3) tty_nr(4) tpgid(5)
    let tpgid: i32 = fields.get(5)?.parse().ok()?;
    (tpgid > 0).then_some(tpgid)
}

fn process_pgrp_and_comm(pid: u32) -> Option<(i32, String)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let comm = stat.get(1 + stat.find('(')?..close)?.to_string();
    let rest = stat.get(close + 2..)?;
    let fields: Vec<&str> = rest.split_whitespace().collect();
    let pgrp: i32 = fields.get(2)?.parse().ok()?;
    Some((pgrp, comm))
}

fn process_cmdline(pid: u32) -> Option<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    if bytes.is_empty() {
        return None;
    }
    let parts: Vec<String> = bytes
        .split(|&b| b == 0)
        .filter(|part| !part.is_empty())
        .map(|part| String::from_utf8_lossy(part).into_owned())
        .collect();
    (!parts.is_empty()).then(|| parts.join(" "))
}

/// Get the current working directory of a process.
/// Uses /proc/<pid>/cwd symlink.
pub fn process_cwd(pid: u32) -> Option<PathBuf> {
    if pid == 0 {
        return None;
    }
    std::fs::read_link(format!("/proc/{pid}/cwd")).ok()
}
