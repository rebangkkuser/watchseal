// SPDX-License-Identifier: GPL-3.0-or-later
//! Process snapshots taken from /proc.

use std::fs;
use std::io;
use std::os::unix::fs::MetadataExt;

#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: i32,
    pub ppid: i32,
    pub uid: u32,
    /// starttime in clock ticks; tells a reused pid apart from the old process
    pub start: u64,
    pub comm: String,
}

/// Parses /proc/<pid>/stat. The name sits between the first '(' and the LAST
/// ')' because it may itself contain spaces and parentheses.
pub fn parse_stat(pid: i32, buf: &[u8]) -> Option<Proc> {
    let lp = buf.iter().position(|&b| b == b'(')?;
    let rp = buf.iter().rposition(|&b| b == b')')?;
    if rp < lp {
        return None;
    }
    let comm: String = String::from_utf8_lossy(&buf[lp + 1..rp])
        .chars()
        .map(|c| if c.is_control() { '?' } else { c })
        .collect();
    let rest = std::str::from_utf8(&buf[rp + 1..]).ok()?;
    // After ')': state is field 3, ppid field 4, starttime field 22.
    let mut f = rest.split_whitespace();
    let ppid = f.nth(1)?.parse().ok()?;
    let start = f.nth(17)?.parse().ok()?;
    Some(Proc { pid, ppid, uid: u32::MAX, start, comm })
}

/// Ok(None) means the process vanished while we looked (normal). Any other
/// read failure is an error, so a scan is never silently incomplete.
fn read_proc(pid: i32) -> io::Result<Option<Proc>> {
    let buf = match fs::read(format!("/proc/{}/stat", pid)) {
        Ok(b) => b,
        Err(e) => {
            return match e.raw_os_error() {
                Some(libc::ENOENT) | Some(libc::ESRCH) => Ok(None),
                _ => Err(e),
            }
        }
    };
    let Some(mut p) = parse_stat(pid, &buf) else {
        return Ok(None);
    };
    p.uid = fs::metadata(format!("/proc/{}", pid))
        .map(|m| m.uid())
        .unwrap_or(u32::MAX);
    Ok(Some(p))
}

/// Kernel threads (pid 2 and its children) churn constantly and are left out.
fn is_kernel_thread(p: &Proc) -> bool {
    p.pid == 2 || p.ppid == 2
}

/// Sorted-by-pid list of user-space processes. An error means the snapshot
/// is incomplete and must not be compared against the previous one.
pub fn scan() -> io::Result<Vec<Proc>> {
    let mut out = Vec::new();
    for entry in fs::read_dir("/proc")? {
        let entry = entry?;
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        if pid <= 0 {
            continue;
        }
        if let Some(p) = read_proc(pid)? {
            if !is_kernel_thread(&p) {
                out.push(p);
            }
        }
    }
    out.sort_by_key(|p| p.pid);
    Ok(out)
}

#[derive(Debug, PartialEq)]
pub enum Change<'a> {
    New(&'a Proc),
    Gone(&'a Proc),
}

/// Differences between two pid-sorted snapshots.
pub fn diff<'a>(old: &'a [Proc], new: &'a [Proc]) -> Vec<Change<'a>> {
    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    while i < old.len() || j < new.len() {
        if j >= new.len() || (i < old.len() && old[i].pid < new[j].pid) {
            out.push(Change::Gone(&old[i]));
            i += 1;
        } else if i >= old.len() || new[j].pid < old[i].pid {
            out.push(Change::New(&new[j]));
            j += 1;
        } else {
            if old[i].start != new[j].start {
                // same pid, different start time: the pid was reused
                out.push(Change::Gone(&old[i]));
                out.push(Change::New(&new[j]));
            }
            i += 1;
            j += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(pid: i32, start: u64) -> Proc {
        Proc { pid, ppid: 1, uid: 0, start, comm: format!("p{}", pid) }
    }

    #[test]
    fn parses_tricky_names() {
        let line = b"123 (we ird) na)me) S 45 1 1 0 -1 4194560 100 0 0 0 1 2 0 0 20 0 1 0 9999 1000 100 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let p = parse_stat(123, line).unwrap();
        assert_eq!(p.comm, "we ird) na)me");
        assert_eq!(p.ppid, 45);
        assert_eq!(p.start, 9999);
    }

    #[test]
    fn rejects_truncated_stat() {
        assert!(parse_stat(1, b"1 (x) S 0 1").is_none());
        assert!(parse_stat(1, b"garbage").is_none());
    }

    #[test]
    fn diff_new_gone_and_reuse() {
        let old = vec![p(10, 1), p(20, 1), p(30, 1)];
        let new = vec![p(10, 1), p(25, 5), p(30, 9)];
        let d = diff(&old, &new);
        assert_eq!(
            d,
            vec![
                Change::Gone(&old[1]),
                Change::New(&new[1]),
                Change::Gone(&old[2]),
                Change::New(&new[2]),
            ]
        );
    }

    #[test]
    fn scan_sees_ourselves() {
        let list = scan().unwrap();
        let me = std::process::id() as i32;
        assert!(list.iter().any(|p| p.pid == me));
    }
}
