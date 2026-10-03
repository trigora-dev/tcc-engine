// Copyright (c) 2026 Trigora, Inc.
// SPDX-License-Identifier: BUSL-1.1
// See LICENSE for full terms.

use std::collections::HashMap;

/// `TCC_CRASH_AT` match. `hook`, `hook:n` (1-based), or `hook:detail`.
#[derive(Debug, Clone, Default)]
pub struct Crash {
    counts: HashMap<String, u32>,
    pub want: Option<String>,
    /// Process death. Tests leave this false and observe the error after the window.
    pub kill: bool,
}

impl Crash {
    pub fn from_env() -> Self {
        let want = std::env::var("TCC_CRASH_AT")
            .ok()
            .filter(|value| !value.is_empty());
        Self {
            counts: HashMap::new(),
            want,
            kill: true,
        }
    }

    pub fn hit(&mut self, hook: &str, detail: Option<&str>) -> Result<(), CrashHit> {
        let n = self.counts.entry(hook.to_string()).or_insert(0);
        *n += 1;
        let n = *n;
        let Some(want) = self.want.clone() else {
            return Ok(());
        };
        let numbered = format!("{hook}:{n}");
        let detailed = detail.map(|item| format!("{hook}:{item}"));
        let matched = want == hook || want == numbered || detailed.as_ref() == Some(&want);
        if !matched {
            return Ok(());
        }
        if self.kill {
            die();
        }
        Err(CrashHit {
            hook: hook.to_string(),
        })
    }
}

fn die() -> ! {
    #[cfg(unix)]
    unsafe {
        libc::raise(libc::SIGKILL);
    }
    std::process::abort();
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrashHit {
    pub hook: String,
}

impl std::fmt::Display for CrashHit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "crashed at {}", self.hook)
    }
}
